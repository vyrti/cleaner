//! Run marked targets.
//!
//! Deletion goes through the existing parallel [`Deleter`], so byte accounting
//! and error handling match the rest of the tool. Commands are run with captured
//! output - inheriting stdio would scribble over the TUI's alternate screen.
//!
//! Elevated targets are never run here. They are returned to the caller, which
//! owns the terminal and can suspend it. See [`super::elevate`].

use super::glob;
use super::{allowed_roots, is_container_allowed, is_path_allowed, Action, Target, Tier};
use crate::deleter::Deleter;
use crate::pool::SCAN_POOL;
use crate::scanner::ScanResult;
use crate::stats::Stats;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Outcome of a run.
#[derive(Debug, Default)]
pub struct RunReport {
    pub freed: u64,
    /// Labels of targets that completed.
    pub done: Vec<String>,
    /// `(label, reason)` for targets that did not.
    pub failed: Vec<(String, String)>,
    /// Targets that need administrator rights, handed back for the caller to
    /// run after suspending the terminal.
    pub deferred: Vec<Target>,
}

/// Execute `marked`.
///
/// `dry_run` measures without deleting and without running any command.
pub fn run(
    marked: Vec<Target>,
    home: &Path,
    dry_run: bool,
    sink: Arc<Mutex<Vec<String>>>,
) -> RunReport {
    let roots = allowed_roots(home);
    let mut report = RunReport::default();
    let stats = Arc::new(Stats::new());

    for target in marked {
        // Anything needing administrator rights is handed back rather than
        // attempted: this process cannot prompt for a password without the
        // terminal, which the caller owns.
        if target.tier == Tier::NeedsRoot || matches!(target.action, Action::Elevated { .. }) {
            report.deferred.push(target);
            continue;
        }

        match &target.action {
            Action::ReportOnly => report.failed.push((
                target.label.clone(),
                "listed only; this tool never deletes it".to_string(),
            )),
            Action::Elevated { .. } => unreachable!("elevated targets are deferred above"),
            Action::Command { program, args } => {
                if dry_run {
                    report.done.push(target.label.clone());
                    continue;
                }
                match run_command(program, args) {
                    Ok(()) => report.done.push(target.label.clone()),
                    Err(error) => report.failed.push((target.label.clone(), error)),
                }
            }
            Action::Remove(_) | Action::Empty(_) | Action::Glob(_) => {
                match delete(&target, &roots, dry_run, Arc::clone(&stats), &sink) {
                    Ok(()) => report.done.push(target.label.clone()),
                    Err(error) => report.failed.push((target.label.clone(), error)),
                }
            }
        }
    }

    report.freed = stats.bytes();
    report
}

/// Resolve a target's paths and hand them to the parallel deleter.
fn delete(
    target: &Target,
    roots: &[PathBuf],
    dry_run: bool,
    stats: Arc<Stats>,
    sink: &Arc<Mutex<Vec<String>>>,
) -> Result<(), String> {
    let mut items = Vec::new();

    for path in target.action.paths() {
        let resolved = match &target.action {
            Action::Glob(_) => glob::expand(path),
            _ => {
                if path.exists() {
                    vec![path.clone()]
                } else {
                    Vec::new()
                }
            }
        };

        for path in resolved {
            match &target.action {
                // Empty keeps the directory and removes what is inside it, so
                // the directory only has to be a legal *container*.
                Action::Empty(_) => {
                    if !is_container_allowed(&path, roots) {
                        return Err(refusal(&path));
                    }
                    let Ok(entries) = std::fs::read_dir(&path) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        let child = entry.path();
                        // Belt and braces: every child is checked on its own.
                        if !is_path_allowed(&child, roots) {
                            return Err(refusal(&child));
                        }
                        if let Some(item) = validated(&child) {
                            items.push(item);
                        }
                    }
                }
                // A catalog typo must never become `rm -rf ~`.
                _ => {
                    if !is_path_allowed(&path, roots) {
                        return Err(refusal(&path));
                    }
                    if let Some(item) = validated(&path) {
                        items.push(item);
                    }
                }
            }
        }
    }

    if items.is_empty() {
        return Ok(());
    }

    let (tx, rx) = crossbeam_channel::bounded(1024);
    let deleter = Deleter::with_sink(
        stats,
        dry_run,
        false,
        Arc::clone(&SCAN_POOL),
        Arc::clone(sink),
    );
    let worker = std::thread::spawn(move || deleter.process(rx));

    for item in items {
        if tx.send(item).is_err() {
            break;
        }
    }
    drop(tx);

    worker
        .join()
        .map_err(|_| "deletion worker panicked".to_string())
}

fn refusal(path: &Path) -> String {
    format!("refused: {} is outside the allowed roots", path.display())
}

/// Re-check a path immediately before deleting it.
///
/// Mirrors the guard the TUI already applies to manual deletes: a path whose
/// type changed since the scan, or that turned out to be a symlink, is skipped
/// rather than followed.
fn validated(path: &Path) -> Option<ScanResult> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    let is_dir = metadata.is_dir();
    let size = if is_dir { 0 } else { file_size(&metadata) };
    Some(ScanResult {
        path: path.to_path_buf(),
        is_dir,
        size,
    })
}

fn file_size(metadata: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::cmp::min(metadata.len(), metadata.blocks() * 512)
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}

/// Run a tool with its output captured.
fn run_command(program: &str, args: &[String]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("{program}: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr.lines().next().unwrap_or("command failed").trim();
    Err(format!("{program}: {reason}"))
}

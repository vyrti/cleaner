//! Running targets that need administrator rights.
//!
//! These cannot run from a worker thread: they need the terminal, either for a
//! `sudo` password prompt or for a UAC handoff. [`exec::run`] therefore defers
//! them, and whoever owns the terminal calls [`run_elevated`] after suspending
//! the UI.
//!
//! [`exec::run`]: super::exec::run

use super::{Action, Target};
use std::path::PathBuf;
use std::process::Command;

/// Result of an elevated batch.
#[derive(Debug, Default)]
pub struct ElevatedReport {
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
    /// Set on Windows, where the work runs from a generated script.
    pub script: Option<PathBuf>,
}

/// The argv lines a target needs, without any elevation prefix.
///
/// Path deletions become explicit `rm`/`Remove-Item` invocations rather than
/// shell globs: a glob would be expanded by a root shell, and that is not a
/// thing worth being clever about.
pub fn command_lines(target: &Target) -> Vec<Vec<String>> {
    match &target.action {
        Action::Elevated { program, args } | Action::Command { program, args } => {
            let mut line = vec![program.clone()];
            line.extend(args.iter().cloned());
            vec![line]
        }
        Action::Remove(paths) => paths.iter().map(|path| remove_line(path)).collect(),
        Action::Empty(paths) => paths.iter().map(|path| empty_line(path)).collect(),
        Action::Glob(paths) => paths.iter().map(|path| remove_line(path)).collect(),
        Action::ReportOnly => Vec::new(),
    }
}

#[cfg(not(windows))]
fn remove_line(path: &std::path::Path) -> Vec<String> {
    vec![
        "rm".to_string(),
        "-rf".to_string(),
        path.to_string_lossy().into_owned(),
    ]
}

#[cfg(not(windows))]
fn empty_line(path: &std::path::Path) -> Vec<String> {
    // `find -mindepth 1 -delete` empties a directory without the caller having
    // to expand a glob, and leaves the directory itself in place.
    vec![
        "find".to_string(),
        path.to_string_lossy().into_owned(),
        "-mindepth".to_string(),
        "1".to_string(),
        "-delete".to_string(),
    ]
}

#[cfg(windows)]
fn remove_line(path: &std::path::Path) -> Vec<String> {
    vec![
        "Remove-Item".to_string(),
        "-LiteralPath".to_string(),
        format!("\"{}\"", path.display()),
        "-Recurse".to_string(),
        "-Force".to_string(),
        "-ErrorAction".to_string(),
        "SilentlyContinue".to_string(),
    ]
}

#[cfg(windows)]
fn empty_line(path: &std::path::Path) -> Vec<String> {
    vec![
        "Remove-Item".to_string(),
        "-Path".to_string(),
        format!("\"{}\\*\"", path.display()),
        "-Recurse".to_string(),
        "-Force".to_string(),
        "-ErrorAction".to_string(),
        "SilentlyContinue".to_string(),
    ]
}

/// A preview of what elevation will run, for display and for the generated
/// script. Safe to call at any time.
pub fn preview(targets: &[Target]) -> Vec<String> {
    targets
        .iter()
        .flat_map(command_lines)
        .map(|line| {
            if cfg!(windows) {
                line.join(" ")
            } else {
                format!("sudo {}", line.join(" "))
            }
        })
        .collect()
}

/// Run elevated targets.
///
/// **The caller must have already left the alternate screen and disabled raw
/// mode.** This inherits stdio so the password or UAC prompt reaches the user.
pub fn run_elevated(targets: &[Target]) -> ElevatedReport {
    #[cfg(not(windows))]
    {
        run_unix(targets)
    }
    #[cfg(windows)]
    {
        run_windows(targets)
    }
}

#[cfg(not(windows))]
fn run_unix(targets: &[Target]) -> ElevatedReport {
    let mut report = ElevatedReport::default();

    // Prime the credential cache once so a batch costs a single prompt.
    let primed = Command::new("sudo")
        .arg("-v")
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !primed {
        for target in targets {
            report.failed.push((
                target.label.clone(),
                "sudo authentication was cancelled or failed".to_string(),
            ));
        }
        return report;
    }

    for target in targets {
        let mut failure = None;
        for line in command_lines(target) {
            let Some((program, args)) = line.split_first() else {
                continue;
            };
            let status = Command::new("sudo").arg(program).args(args).status();
            match status {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    failure = Some(format!("{program} exited with {status}"));
                    break;
                }
                Err(error) => {
                    failure = Some(format!("{program}: {error}"));
                    break;
                }
            }
        }
        match failure {
            Some(reason) => report.failed.push((target.label.clone(), reason)),
            None => report.done.push(target.label.clone()),
        }
    }

    report
}

/// Windows has no `sudo`. Generate a PowerShell script and hand it to UAC.
///
/// Going through a visible script rather than elevating silently means the user
/// can read exactly what is about to run as Administrator before approving it.
#[cfg(windows)]
fn run_windows(targets: &[Target]) -> ElevatedReport {
    let mut report = ElevatedReport::default();
    if targets.is_empty() {
        return report;
    }

    let script_path =
        std::env::temp_dir().join(format!("cleaner-elevated-{}.ps1", std::process::id()));

    let mut script = String::from(
        "# Generated by cleaner. Review before running.\n$ErrorActionPreference = 'Continue'\n",
    );
    for target in targets {
        script.push_str(&format!(
            "Write-Host '== {}'\n",
            target.label.replace('\'', "''")
        ));
        for line in command_lines(target) {
            script.push_str(&line.join(" "));
            script.push('\n');
        }
    }
    script.push_str("Write-Host ''\nRead-Host 'Done. Press Enter to close'\n");

    if let Err(error) = std::fs::write(&script_path, script) {
        for target in targets {
            report.failed.push((
                target.label.clone(),
                format!("could not write script: {error}"),
            ));
        }
        return report;
    }

    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Start-Process",
            "powershell",
            "-Verb",
            "RunAs",
            "-Wait",
            "-ArgumentList",
        ])
        .arg(format!(
            "'-NoProfile','-ExecutionPolicy','Bypass','-File','{}'",
            script_path.display()
        ))
        .status();

    report.script = Some(script_path);

    match status {
        Ok(status) if status.success() => {
            for target in targets {
                report.done.push(target.label.clone());
            }
        }
        Ok(_) => {
            for target in targets {
                report.failed.push((
                    target.label.clone(),
                    "elevation was declined or the script failed".to_string(),
                ));
            }
        }
        Err(error) => {
            for target in targets {
                report
                    .failed
                    .push((target.label.clone(), format!("could not elevate: {error}")));
            }
        }
    }

    report
}

//! Measure catalog targets.
//!
//! Sizes come from [`crate::fastwalk`], which already reports allocated blocks
//! rather than apparent length on unix. That is what makes a sparse file like
//! Docker's `Docker.raw` report the ~46 GiB it actually occupies instead of the
//! 228 GB it claims.

use super::glob;
use super::{Candidate, Group, Target};
use crate::fastwalk::walk_parallel_mapped;
use crate::pool::SCAN_POOL;
use crate::tree::ScanProgress;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Unrecognised cache directories smaller than this are not worth a row.
const RESIDUAL_MIN_BYTES: u64 = 100 * 1024 * 1024;

/// Phase reported through [`ScanProgress::begin_stage`] while probing.
pub const PHASE_PROBING: u8 = 1;

/// Measure every target, dropping the ones that do not apply to this machine.
///
/// A target is dropped when its required binary is missing, or when nothing it
/// points at exists. Command targets with nothing to measure are kept at size
/// zero - `docker system prune` has real work to do even though there is no
/// single path to point at.
pub fn probe(
    targets: Vec<Target>,
    progress: &ScanProgress,
    cancelled: &Arc<AtomicBool>,
) -> Vec<Candidate> {
    progress.begin_stage(PHASE_PROBING, targets.len());

    let mut candidates = Vec::with_capacity(targets.len());

    for target in targets {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        progress.stage_current.fetch_add(1, Ordering::Relaxed);

        if let Some(binary) = target.requires.as_deref() {
            if !has_binary(binary) {
                continue;
            }
        }

        let resolved: Vec<_> = target.probe.iter().flat_map(|p| glob::expand(p)).collect();

        let size: u64 = resolved
            .iter()
            .map(|path| {
                if cancelled.load(Ordering::Relaxed) {
                    0
                } else {
                    size_of(path, cancelled)
                }
            })
            .sum();

        progress.bytes.fetch_add(size, Ordering::Relaxed);

        // Nothing measurable and nothing to run means this machine does not have
        // whatever the entry describes.
        let present = !resolved.is_empty() || target.probe.is_empty();
        if !present {
            continue;
        }

        // The residual sweep produces a row per unrecognised cache directory;
        // only the large ones earn a place in the list.
        if target.group == Group::Other && size < RESIDUAL_MIN_BYTES {
            continue;
        }

        candidates.push(Candidate {
            target,
            size,
            present,
        });
    }

    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.size));
    candidates
}

/// Allocated size of a file or directory tree.
fn size_of(path: &Path, cancelled: &Arc<AtomicBool>) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };

    // Never follow a symlink into someone else's data, and never count it.
    if metadata.file_type().is_symlink() {
        return 0;
    }

    if metadata.is_file() {
        return file_size(&metadata);
    }
    if !metadata.is_dir() {
        return 0;
    }

    // Reuse the parallel walker, and use its skip hook as the cancellation
    // point so a huge tree can be abandoned mid-flight rather than only between
    // targets.
    let flag = Arc::clone(cancelled);
    let skip = Arc::new(move |_: &Path| flag.load(Ordering::Relaxed));

    let output = walk_parallel_mapped(path.to_path_buf(), &SCAN_POOL, skip, None, &|_,
                                                                                    entries|
     -> u64 {
        entries
            .iter()
            .filter(|entry| !entry.is_dir && !entry.is_symlink)
            .map(|entry| entry.size)
            .sum()
    });

    output.entries.values().sum()
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

/// Whether `name` resolves on `PATH`.
pub fn has_binary(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };

    #[cfg(windows)]
    let extensions: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
        .split(';')
        .map(|e| e.to_lowercase())
        .collect();

    std::env::split_paths(&path).any(|dir| {
        if dir.join(name).is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            extensions
                .iter()
                .any(|ext| dir.join(format!("{name}{ext}")).is_file())
        }
        #[cfg(not(windows))]
        false
    })
}

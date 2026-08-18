use super::entry::DirEntry;
use super::progress::ScanProgress;
use foldhash::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) fn apply_directory_sizes(
    dir: &Path,
    children: &mut HashMap<PathBuf, Arc<Vec<DirEntry>>>,
    progress: &ScanProgress,
    cancelled: &AtomicBool,
) -> u64 {
    struct Frame {
        path: PathBuf,
        entries: Arc<Vec<DirEntry>>,
        next: usize,
        total: u64,
    }

    let Some((path, entries)) = children.remove_entry(dir) else {
        return 0;
    };
    let mut stack = vec![Frame {
        path,
        entries,
        next: 0,
        total: 0,
    }];
    let mut root_total = 0;
    let mut completed = 0usize;

    while !stack.is_empty() {
        if cancelled.load(Ordering::Relaxed) {
            progress.stage_current.store(completed, Ordering::Relaxed);
            for frame in stack.drain(..) {
                children.insert(frame.path, frame.entries);
            }
            return 0;
        }

        let child = {
            let frame = stack.last_mut().expect("size stack is not empty");
            let mut child = None;
            while frame.next < frame.entries.len() {
                let index = frame.next;
                frame.next += 1;
                let entry = &frame.entries[index];
                if entry.is_dir && entry.name != ".." {
                    child = Some((index, frame.path.join(&entry.name)));
                    break;
                }
                frame.total = frame.total.saturating_add(entry.size);
            }
            child
        };

        if let Some((index, child_path)) = child {
            if let Some((path, entries)) = children.remove_entry(&child_path) {
                stack.push(Frame {
                    path,
                    entries,
                    next: 0,
                    total: 0,
                });
            } else if let Some(frame) = stack.last_mut() {
                Arc::make_mut(&mut frame.entries)[index].size = 0;
            }
            continue;
        }

        let frame = stack.pop().expect("completed size frame exists");
        let total = frame.total;
        children.insert(frame.path, frame.entries);
        completed = completed.saturating_add(1);
        if completed.is_multiple_of(1024) {
            progress.stage_current.store(completed, Ordering::Relaxed);
        }
        if let Some(parent) = stack.last_mut() {
            let child_index = parent.next - 1;
            Arc::make_mut(&mut parent.entries)[child_index].size = total;
            parent.total = parent.total.saturating_add(total);
        } else {
            root_total = total;
        }
    }

    progress.stage_current.store(completed, Ordering::Relaxed);

    root_total
}

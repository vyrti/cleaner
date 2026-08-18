//! Directory tree with MAXIMUM PERFORMANCE single-pass scan
//! Single WalkDir, no duplicate syscalls, O(n) everywhere

mod builder;
mod entry;
mod progress;
mod sizing;
mod sort;

#[cfg(test)]
mod tests;

pub use entry::DirEntry;
pub use progress::ScanProgress;
pub use sort::{sort_by_name, sort_by_size};

use foldhash::{HashMap, HashMapExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct DirTree {
    pub children: HashMap<PathBuf, Arc<Vec<DirEntry>>>,
    sort_modes: HashMap<PathBuf, bool>,
}

impl DirTree {
    pub fn from_children(children: HashMap<PathBuf, Vec<DirEntry>>) -> Self {
        let children = children
            .into_iter()
            .map(|(path, entries)| (path, Arc::new(entries)))
            .collect();
        Self {
            children,
            sort_modes: HashMap::new(),
        }
    }

    pub fn from_shared_children(children: HashMap<PathBuf, Arc<Vec<DirEntry>>>) -> Self {
        Self {
            children,
            sort_modes: HashMap::new(),
        }
    }

    pub fn get_children(&mut self, path: &Path, by_name: bool) -> Arc<Vec<DirEntry>> {
        if self.sort_modes.get(path).copied() != Some(by_name) {
            if let Some(entries) = self.children.get_mut(path) {
                let entries = Arc::make_mut(entries);
                if by_name {
                    sort_by_name(entries);
                } else {
                    sort_by_size(entries);
                }
                self.sort_modes.insert(path.to_path_buf(), by_name);
            }
        }
        self.children
            .get(path)
            .cloned()
            .unwrap_or_else(|| Arc::new(Vec::new()))
    }

    /// Remove entry from the tree and update all parent sizes (O(depth))
    pub fn delete_entry(&mut self, path: &PathBuf, is_dir: bool) {
        if let Some(parent) = path.parent() {
            let parent_buf = parent.to_path_buf();

            // 1. Remove from parent's children list
            if let Some(entries) = self.children.get_mut(&parent_buf) {
                let entries = Arc::make_mut(entries);
                if let Some(idx) = entries
                    .iter()
                    .position(|entry| Some(entry.name.as_os_str()) == path.file_name())
                {
                    let removed = entries.remove(idx);
                    let size_removed = removed.size;

                    // Manual deletion is rare relative to tree construction, so
                    // avoid an eager full-path index on every scan.
                    let mut current_parent = parent_buf;
                    while let Some(grandparent) = current_parent.parent() {
                        let grandparent = grandparent.to_path_buf();
                        if let Some(entries) = self.children.get_mut(&grandparent) {
                            if let Some(parent_entry) =
                                Arc::make_mut(entries).iter_mut().find(|entry| {
                                    Some(entry.name.as_os_str()) == current_parent.file_name()
                                })
                            {
                                parent_entry.size = parent_entry.size.saturating_sub(size_removed);
                            }
                        }
                        current_parent = grandparent;
                    }
                }
            }
        }

        // 3. If directory, remove its children entry mapping (optional cleanup)
        if is_dir {
            self.children
                .retain(|candidate, _| !candidate.starts_with(path));
            self.sort_modes
                .retain(|candidate, _| !candidate.starts_with(path));
        }
    }

    pub fn get_temp_stats(&self, dir: &Path) -> (usize, usize, u64) {
        let mut totals = (0usize, 0usize, 0u64);
        let mut stack = vec![dir.to_path_buf()];
        while let Some(path) = stack.pop() {
            if let Some(entries) = self.children.get(&path) {
                for entry in entries.iter().filter(|entry| entry.name != "..") {
                    if entry.is_temp {
                        if entry.is_dir {
                            totals.0 = totals.0.saturating_add(1);
                        } else {
                            totals.1 = totals.1.saturating_add(1);
                        }
                        totals.2 = totals.2.saturating_add(entry.size);
                    } else if entry.is_dir {
                        stack.push(path.join(&entry.name));
                    }
                }
            }
        }
        totals
    }
}

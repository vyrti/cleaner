//! Directory tree with MAXIMUM PERFORMANCE single-pass scan
//! Single WalkDir, no duplicate syscalls, O(n) everywhere

use crate::patterns::PatternMatcher;
use crate::fastwalk;
use crate::pool::SCAN_POOL;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_temp: bool,
}

pub struct ScanProgress {
    pub files: AtomicUsize,
    pub dirs: AtomicUsize,
    pub bytes: AtomicU64,
    pub done: AtomicBool,
    pub phase: AtomicU8,
}

impl ScanProgress {
    pub fn new() -> Self {
        Self {
            files: AtomicUsize::new(0),
            dirs: AtomicUsize::new(0),
            bytes: AtomicU64::new(0),
            done: AtomicBool::new(false),
            phase: AtomicU8::new(0),
        }
    }

    pub fn get_files(&self) -> usize { self.files.load(Ordering::Relaxed) }
    pub fn get_dirs(&self) -> usize { self.dirs.load(Ordering::Relaxed) }
    pub fn get_bytes(&self) -> u64 { self.bytes.load(Ordering::Relaxed) }
    pub fn is_done(&self) -> bool { self.done.load(Ordering::Relaxed) }
    pub fn get_phase(&self) -> u8 { self.phase.load(Ordering::Relaxed) }
}



pub struct DirTree {
    pub children: HashMap<PathBuf, Vec<DirEntry>>,
}

impl DirTree {
    /// Build tree with SINGLE WalkDir pass - maximum performance
    pub fn build_with_progress(
        root: &PathBuf,
        matcher: &PatternMatcher,
        progress: Arc<ScanProgress>,
        cancelled: Arc<AtomicBool>,
    ) -> Self {
        // Build the skip check closure
        #[cfg(target_os = "macos")]
        let docker_path: Option<PathBuf> = {
            if let Some(home) = std::env::var_os("HOME") {
                let docker_container = PathBuf::from(home)
                    .join("Library/Containers/com.docker.docker");
                if docker_container.exists() {
                    Some(docker_container)
                } else {
                    None
                }
            } else {
                None
            }
        };
        #[cfg(not(target_os = "macos"))]
        let docker_path: Option<PathBuf> = None;

        let root_clone = root.clone();
        let skip_check = Arc::new(move |path: &Path| -> bool {
            if let Some(ref docker) = docker_path {
                if path.starts_with(docker) {
                    return true;
                }
            }
            #[cfg(target_os = "macos")]
            {
                if (path.starts_with("/System/Volumes") || path == Path::new("/System/Volumes"))
                    && !root_clone.starts_with("/System/Volumes")
                {
                    return true;
                }
                if (path.starts_with("/Volumes") || path == Path::new("/Volumes"))
                    && !root_clone.starts_with("/Volumes")
                {
                    return true;
                }
            }
            false
        });

        let progress_clone = Arc::clone(&progress);
        let progress_cb = Arc::new(move |is_dir: bool, size: u64| {
            if is_dir {
                progress_clone.dirs.fetch_add(1, Ordering::Relaxed);
            } else {
                progress_clone.files.fetch_add(1, Ordering::Relaxed);
                progress_clone.bytes.fetch_add(size, Ordering::Relaxed);
            }
        });

        // 1. Walk the directory tree in parallel using native platform syscalls
        let raw_tree = fastwalk::walk_parallel(root.clone(), &SCAN_POOL, skip_check, Some(progress_cb));

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Relaxed);
            return Self { children: HashMap::new() };
        }

        // 2. Build the children map using pre-allocated capacity
        let mut children: HashMap<PathBuf, Vec<DirEntry>> = HashMap::with_capacity(raw_tree.len());

        for (dir_path, entries) in &raw_tree {
            let dir_entries: Vec<DirEntry> = entries
                .iter()
                .filter(|e| !e.is_symlink)
                .map(|e| {
                    let full_path = dir_path.join(&e.name);
                    let is_temp = if e.is_dir {
                        matcher.is_temp_directory(&e.name)
                    } else {
                        matcher.is_temp_file(&e.name)
                    };

                    DirEntry {
                        path: full_path,
                        name: e.name.clone(),
                        size: e.size,
                        is_dir: e.is_dir,
                        is_temp,
                    }
                })
                .collect();
            children.insert(dir_path.clone(), dir_entries);
        }

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Relaxed);
            return Self { children };
        }

        progress.phase.store(1, Ordering::Relaxed);

        // 3. Bottom-up recursive sizing
        let mut size_cache: HashMap<PathBuf, u64> = HashMap::with_capacity(children.len());
        compute_dir_size(root, &children, &mut size_cache);

        // 4. Apply computed sizes to directory entries in children map
        for entries in children.values_mut() {
            for entry in entries.iter_mut() {
                if entry.is_dir {
                    entry.size = *size_cache.get(&entry.path).unwrap_or(&0);
                }
            }
        }

        // 5. Sort entries and add ".." navigation
        for (dir_path, entries) in children.iter_mut() {
            entries.sort_unstable_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => b.size.cmp(&a.size),
            });

            if dir_path != root {
                if let Some(parent) = dir_path.parent() {
                    entries.insert(
                        0,
                        DirEntry {
                            path: parent.to_path_buf(),
                            name: "..".to_string(),
                            size: 0,
                            is_dir: true,
                            is_temp: false,
                        },
                    );
                }
            }
        }

        progress.done.store(true, Ordering::Relaxed);
        Self { children }
    }
}

fn compute_dir_size(
    dir: &Path,
    children: &HashMap<PathBuf, Vec<DirEntry>>,
    cache: &mut HashMap<PathBuf, u64>,
) -> u64 {
    if let Some(&s) = cache.get(dir) {
        return s;
    }
    let total = children.get(dir).map_or(0, |entries| {
        entries
            .iter()
            .map(|e| {
                if e.is_dir && e.name != ".." {
                    compute_dir_size(&e.path, children, cache)
                } else {
                    e.size
                }
            })
            .sum()
    });
    cache.insert(dir.to_path_buf(), total);
    total
}

impl DirTree {

    pub fn get_children(&self, path: &PathBuf) -> Vec<DirEntry> {
        self.children.get(path).cloned().unwrap_or_default()
    }

    /// Remove entry from the tree and update all parent sizes (O(depth))
    pub fn delete_entry(&mut self, path: &PathBuf, is_dir: bool) {
        if let Some(parent) = path.parent() {
            let parent_buf = parent.to_path_buf();
            
            // 1. Remove from parent's children list
            if let Some(entries) = self.children.get_mut(&parent_buf) {
                if let Some(idx) = entries.iter().position(|e| &e.path == path) {
                    let removed = entries.remove(idx);
                    let size_removed = removed.size;

                    // 2. Propagate size change up the tree
                    let mut current_parent = parent_buf.clone();
                    loop {
                        // Find this parent in its own parent's listing
                        if let Some(grandparent) = current_parent.parent() {
                            let grandparent_buf = grandparent.to_path_buf();
                             if let Some(siblings) = self.children.get_mut(&grandparent_buf) {
                                if let Some(parent_entry) = siblings.iter_mut().find(|e| e.path == current_parent) {
                                    parent_entry.size = parent_entry.size.saturating_sub(size_removed);
                                }
                             }
                             current_parent = grandparent_buf;
                        } else {
                            break; // Reached root parent (which has no parent)
                        }
                    }
                }
            }
        }

        // 3. If directory, remove its children entry mapping (optional cleanup)
        if is_dir {
            self.children.remove(path);
        }
    }
}

pub fn sort_by_size(entries: &mut [DirEntry]) {
    entries.sort_unstable_by(|a, b| {
        if a.name == ".." { return std::cmp::Ordering::Less; }
        if b.name == ".." { return std::cmp::Ordering::Greater; }
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.size.cmp(&a.size),
        }
    });
}

pub fn sort_by_name(entries: &mut [DirEntry]) {
    entries.sort_unstable_by(|a, b| {
        if a.name == ".." { return std::cmp::Ordering::Less; }
        if b.name == ".." { return std::cmp::Ordering::Greater; }
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });
}

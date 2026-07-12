//! Directory tree with MAXIMUM PERFORMANCE single-pass scan
//! Single WalkDir, no duplicate syscalls, O(n) everywhere

use crate::patterns::PatternMatcher;
use crate::fastwalk;
use crate::pool::SCAN_POOL;
use rayon::prelude::*;
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
        force: bool,
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

        // Protected directories (NEVER auto-clean inside these, but allow scanning and manual TUI deletion)
        let mut protected_paths: Vec<PathBuf> = vec![];

        // Home-relative paths
        if let Some(home) = dirs::home_dir() {
            protected_paths.extend(vec![
                home.join(".cargo"),
                home.join(".rustup"),
                home.join("go"),
                home.join(".go"),
                home.join(".npm"),
                home.join(".nvm"),
                home.join(".pyenv"),
                home.join(".rbenv"),
                home.join(".gradle"),
                home.join(".m2"),
                home.join(".local"),
                home.join(".config"),
                home.join(".ssh"),
                home.join(".gnupg"),
                home.join("Library"),
            ]);
            #[cfg(windows)]
            {
                protected_paths.push(home.join("AppData"));
            }
        }

        // Unix system directories
        #[cfg(unix)]
        {
            protected_paths.extend(vec![
                PathBuf::from("/System"),
                PathBuf::from("/Library"),
                PathBuf::from("/Applications"),
                PathBuf::from("/usr"),
                PathBuf::from("/var"),
                PathBuf::from("/etc"),
                PathBuf::from("/bin"),
                PathBuf::from("/sbin"),
                PathBuf::from("/lib"),
                PathBuf::from("/lib64"),
                PathBuf::from("/boot"),
                PathBuf::from("/opt"),
                PathBuf::from("/private"),
                PathBuf::from("/dev"),
                PathBuf::from("/proc"),
                PathBuf::from("/sys"),
                PathBuf::from("/run"),
            ]);
        }

        // Windows system directories
        #[cfg(windows)]
        {
            if let Some(win_dir) = std::env::var_os("SystemRoot").map(PathBuf::from) {
                protected_paths.push(win_dir);
            } else {
                protected_paths.push(PathBuf::from("C:\\Windows"));
            }
            if let Some(prog_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
                protected_paths.push(prog_files);
            } else {
                protected_paths.push(PathBuf::from("C:\\Program Files"));
            }
            if let Some(prog_files_x86) = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from) {
                protected_paths.push(prog_files_x86);
            } else {
                protected_paths.push(PathBuf::from("C:\\Program Files (x86)"));
            }
            if let Some(prog_data) = std::env::var_os("ProgramData").map(PathBuf::from) {
                protected_paths.push(prog_data);
            } else {
                protected_paths.push(PathBuf::from("C:\\ProgramData"));
            }
            protected_paths.push(PathBuf::from("C:\\System Volume Information"));
        }

        let protected_paths_arc = Arc::new(protected_paths);
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

        // 2. Build the children map using parallel Rayon iteration
        let protected_paths_clone = Arc::clone(&protected_paths_arc);
        let root_clone2 = root.clone();
        let mut children: HashMap<PathBuf, Vec<DirEntry>> = raw_tree
            .into_par_iter()
            .map(|(dir_path, entries)| {
                let protected_paths = Arc::clone(&protected_paths_clone);
                let root_path = root_clone2.clone();
                let dir_entries: Vec<DirEntry> = entries
                    .into_iter()
                    .filter(|e| !e.is_symlink)
                    .map(|e| {
                        let full_path = dir_path.join(&e.name);
                        let in_protected = !force && protected_paths.iter().any(|p| full_path.starts_with(p) && !root_path.starts_with(p));
                        let is_temp = if in_protected {
                            false
                        } else if e.is_dir {
                            matcher.is_temp_directory(&e.name)
                        } else {
                            matcher.is_temp_file(&e.name)
                        };

                        DirEntry {
                            path: full_path,
                            name: e.name,
                            size: e.size,
                            is_dir: e.is_dir,
                            is_temp,
                        }
                    })
                    .collect();
                (dir_path, dir_entries)
            })
            .collect();

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Relaxed);
            return Self { children };
        }

        progress.phase.store(1, Ordering::Relaxed);

        // 3. Bottom-up recursive sizing
        let mut size_cache: HashMap<PathBuf, u64> = HashMap::with_capacity(children.len());
        compute_dir_size(root, &children, &mut size_cache);

        // 4. Apply computed sizes, sort entries, and add ".." navigation in parallel
        let root_clone = root.clone();
        children.par_iter_mut().for_each(|(dir_path, entries)| {
            // Apply sizing to directory entries
            for entry in entries.iter_mut() {
                if entry.is_dir {
                    entry.size = *size_cache.get(&entry.path).unwrap_or(&0);
                }
            }

            // Sort entries
            entries.sort_unstable_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => b.size.cmp(&a.size),
            });

            // Add navigation
            if dir_path != &root_clone {
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
        });

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

    pub fn get_temp_stats(&self, dir: &Path) -> (usize, usize, u64) {
        let mut dirs = 0;
        let mut files = 0;
        let mut bytes = 0;
        
        if let Some(entries) = self.children.get(dir) {
            for entry in entries {
                if entry.name == ".." {
                    continue;
                }
                if entry.is_temp {
                    if entry.is_dir {
                        dirs += 1;
                        bytes += entry.size;
                    } else {
                        files += 1;
                        bytes += entry.size;
                    }
                } else if entry.is_dir {
                    let (sub_dirs, sub_files, sub_bytes) = self.get_temp_stats(&entry.path);
                    dirs += sub_dirs;
                    files += sub_files;
                    bytes += sub_bytes;
                }
            }
        }
        
        (dirs, files, bytes)
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

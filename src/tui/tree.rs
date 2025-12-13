//! Directory tree with MAXIMUM PERFORMANCE single-pass scan
//! Single WalkDir, no duplicate syscalls, O(n) everywhere

use crate::patterns::PatternMatcher;
use jwalk::WalkDir;
use std::collections::HashMap;
use std::path::PathBuf;
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

/// Entry info collected in single pass (no extra syscalls)
struct RawEntry {
    path: PathBuf,
    parent: PathBuf,
    name: String,
    size: u64,
    is_dir: bool,
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
        let mut children: HashMap<PathBuf, Vec<DirEntry>> = HashMap::new();

        // SINGLE PASS: Collect all entries with parallel jwalk
        let mut entries: Vec<RawEntry> = Vec::new();
        let mut dir_sizes: HashMap<PathBuf, u64> = HashMap::new();

        // Use jwalk with parallelism enabled
        for entry in WalkDir::new(root)
            .parallelism(jwalk::Parallelism::RayonNewPool(num_cpus::get()))
            .skip_hidden(false)
            .min_depth(1) {
            if cancelled.load(Ordering::Relaxed) {
                progress.done.store(true, Ordering::Relaxed);
                return Self { children };
            }

            if let Ok(e) = entry {
                let path = e.path();
                let is_dir = e.file_type().is_dir(); // Already cached by jwalk!
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let size = if is_dir {
                    progress.dirs.fetch_add(1, Ordering::Relaxed);
                    0 // Will calculate later
                } else {
                    let s = e.metadata().map(|m| m.len()).unwrap_or(0);
                    progress.files.fetch_add(1, Ordering::Relaxed);
                    progress.bytes.fetch_add(s, Ordering::Relaxed);
                    
                    // Aggregate to parent directories immediately
                    let mut current = path.parent();
                    while let Some(dir) = current {
                        *dir_sizes.entry(dir.to_path_buf()).or_insert(0) += s;
                        if dir == root.as_path() { break; }
                        current = dir.parent();
                    }
                    s
                };

                if let Some(parent) = path.parent() {
                    let parent_buf = parent.to_path_buf();
                    entries.push(RawEntry {
                        path,
                        parent: parent_buf,
                        name,
                        size,
                        is_dir,
                    });
                }
            }
        }

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Relaxed);
            return Self { children };
        }

        progress.phase.store(1, Ordering::Relaxed);

        // Build children map - single pass through collected entries
        for e in entries {
            let size = if e.is_dir {
                *dir_sizes.get(&e.path).unwrap_or(&0)
            } else {
                e.size
            };

            let is_temp = if e.is_dir {
                matcher.is_temp_directory(&e.name)
            } else {
                matcher.is_temp_file(&e.name)
            };

            children.entry(e.parent.clone()).or_default().push(DirEntry {
                path: e.path,
                name: e.name,
                size,
                is_dir: e.is_dir,
                is_temp,
            });
        }

        // Sort and add ".." navigation
        for (dir_path, entries) in children.iter_mut() {
            entries.sort_unstable_by(|a, b| {
                match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => b.size.cmp(&a.size),
                }
            });

            if dir_path != root {
                if let Some(parent) = dir_path.parent() {
                    entries.insert(0, DirEntry {
                        path: parent.to_path_buf(),
                        name: "..".to_string(),
                        size: 0,
                        is_dir: true,
                        is_temp: false,
                    });
                }
            }
        }

        progress.done.store(true, Ordering::Relaxed);
        Self { children }
    }

    pub fn get_children(&self, path: &PathBuf) -> Vec<DirEntry> {
        self.children.get(path).cloned().unwrap_or_default()
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

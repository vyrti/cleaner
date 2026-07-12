//! Directory tree with MAXIMUM PERFORMANCE single-pass scan
//! Single WalkDir, no duplicate syscalls, O(n) everywhere

use crate::fastwalk;
use crate::patterns::PatternMatcher;
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
    pub stage_current: AtomicUsize,
    pub stage_total: AtomicUsize,
}

impl ScanProgress {
    pub fn new() -> Self {
        Self {
            files: AtomicUsize::new(0),
            dirs: AtomicUsize::new(0),
            bytes: AtomicU64::new(0),
            done: AtomicBool::new(false),
            phase: AtomicU8::new(0),
            stage_current: AtomicUsize::new(0),
            stage_total: AtomicUsize::new(0),
        }
    }

    pub fn get_files(&self) -> usize {
        self.files.load(Ordering::Relaxed)
    }
    pub fn get_dirs(&self) -> usize {
        self.dirs.load(Ordering::Relaxed)
    }
    pub fn get_bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
    pub fn get_phase(&self) -> u8 {
        self.phase.load(Ordering::Acquire)
    }

    pub fn get_stage_progress(&self) -> (usize, usize) {
        (
            self.stage_current.load(Ordering::Relaxed),
            self.stage_total.load(Ordering::Relaxed),
        )
    }

    fn begin_stage(&self, phase: u8, total: usize) {
        self.stage_current.store(0, Ordering::Relaxed);
        self.stage_total.store(total, Ordering::Relaxed);
        self.phase.store(phase, Ordering::Release);
    }
}

pub struct DirTree {
    pub children: HashMap<PathBuf, Vec<DirEntry>>,
}

impl DirTree {
    /// Build tree with SINGLE WalkDir pass - maximum performance
    pub fn build_with_progress(
        root: &Path,
        matcher: &PatternMatcher,
        progress: Arc<ScanProgress>,
        cancelled: Arc<AtomicBool>,
        force: bool,
    ) -> Self {
        // Build the skip check closure
        #[cfg(target_os = "macos")]
        let docker_path: Option<PathBuf> = {
            if let Some(home) = std::env::var_os("HOME") {
                let docker_container =
                    PathBuf::from(home).join("Library/Containers/com.docker.docker");
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

        let root_clone = root.to_path_buf();

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

        // Whether the scan root is protected is constant. Remove those paths once
        // instead of repeating the same root.starts_with check for every entry.
        if force {
            protected_paths.clear();
        } else {
            protected_paths.retain(|path| !root.starts_with(path));
        }

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
        let raw_tree = fastwalk::walk_parallel(
            root.to_path_buf(),
            &SCAN_POOL,
            skip_check,
            Some(progress_cb),
        );

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Release);
            return Self {
                children: HashMap::new(),
            };
        }

        // 2. Build the children map using parallel Rayon iteration
        progress.begin_stage(1, raw_tree.len());
        let mut children: HashMap<PathBuf, Vec<DirEntry>> = raw_tree
            .into_par_iter()
            .map(|(dir_path, entries)| {
                let dir_entries: Vec<DirEntry> = entries
                    .into_iter()
                    .filter(|e| !e.is_symlink)
                    .map(|e| {
                        let full_path = dir_path.join(&e.name);
                        let in_protected = protected_paths.iter().any(|p| full_path.starts_with(p));
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
                progress.stage_current.fetch_add(1, Ordering::Relaxed);
                (dir_path, dir_entries)
            })
            .collect();

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Release);
            return Self { children };
        }

        // 3. Compute sizes in place. The old implementation duplicated every
        // directory PathBuf into a second hash map, which became very expensive
        // for multi-million-entry scans.
        progress.begin_stage(2, children.len());
        apply_directory_sizes(root, &mut children, &progress, &cancelled);

        if cancelled.load(Ordering::Relaxed) {
            progress.done.store(true, Ordering::Release);
            return Self { children };
        }

        // 4. Sort entries and add ".." navigation in parallel.
        progress.begin_stage(3, children.len());
        let root_clone = root.to_path_buf();
        children.par_iter_mut().for_each(|(dir_path, entries)| {
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
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
            progress.stage_current.fetch_add(1, Ordering::Relaxed);
        });

        progress.done.store(true, Ordering::Release);
        Self { children }
    }
}

fn apply_directory_sizes(
    dir: &Path,
    children: &mut HashMap<PathBuf, Vec<DirEntry>>,
    progress: &ScanProgress,
    cancelled: &AtomicBool,
) -> u64 {
    if cancelled.load(Ordering::Relaxed) {
        return 0;
    }

    let Some((dir_path, mut entries)) = children.remove_entry(dir) else {
        return 0;
    };

    let mut total = 0u64;
    for entry in &mut entries {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        if entry.is_dir && entry.name != ".." {
            entry.size = apply_directory_sizes(&entry.path, children, progress, cancelled);
        }
        total = total.saturating_add(entry.size);
    }

    children.insert(dir_path, entries);
    progress.stage_current.fetch_add(1, Ordering::Relaxed);
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
                    while let Some(grandparent) = current_parent.parent() {
                        // Find this parent in its own parent's listing
                        let grandparent_buf = grandparent.to_path_buf();
                        if let Some(siblings) = self.children.get_mut(&grandparent_buf) {
                            if let Some(parent_entry) =
                                siblings.iter_mut().find(|e| e.path == current_parent)
                            {
                                parent_entry.size = parent_entry.size.saturating_sub(size_removed);
                            }
                        }
                        current_parent = grandparent_buf;
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
        if a.name == ".." {
            return std::cmp::Ordering::Less;
        }
        if b.name == ".." {
            return std::cmp::Ordering::Greater;
        }
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.size.cmp(&a.size),
        }
    });
}

pub fn sort_by_name(entries: &mut [DirEntry]) {
    entries.sort_unstable_by(|a, b| {
        if a.name == ".." {
            return std::cmp::Ordering::Less;
        }
        if b.name == ".." {
            return std::cmp::Ordering::Greater;
        }
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::test_support::TempDir;

    fn entry(path: PathBuf, name: &str, size: u64, is_dir: bool, is_temp: bool) -> DirEntry {
        DirEntry {
            path,
            name: name.into(),
            size,
            is_dir,
            is_temp,
        }
    }

    fn matcher() -> PatternMatcher {
        PatternMatcher::new(Arc::new(Config {
            directories: vec!["target".into()],
            files: vec![".pyc".into()],
            days: None,
            force: false,
        }))
    }

    #[test]
    fn progress_accessors_reflect_atomic_state() {
        let progress = ScanProgress::new();
        progress.files.store(2, Ordering::Relaxed);
        progress.dirs.store(3, Ordering::Relaxed);
        progress.bytes.store(40, Ordering::Relaxed);
        progress.phase.store(1, Ordering::Relaxed);
        progress.done.store(true, Ordering::Relaxed);
        assert_eq!(
            (
                progress.get_files(),
                progress.get_dirs(),
                progress.get_bytes()
            ),
            (2, 3, 40)
        );
        assert_eq!(progress.get_phase(), 1);
        assert!(progress.is_done());
        progress.begin_stage(3, 12);
        progress.stage_current.store(5, Ordering::Relaxed);
        assert_eq!(progress.get_phase(), 3);
        assert_eq!(progress.get_stage_progress(), (5, 12));
    }

    #[test]
    fn in_place_sizing_updates_nested_entries_and_progress() {
        let root = PathBuf::from("/sizing-root");
        let child = root.join("child");
        let nested = child.join("nested");
        let mut children = HashMap::new();
        children.insert(
            root.clone(),
            vec![
                entry(child.clone(), "child", 0, true, false),
                entry(root.join("root.bin"), "root.bin", 2, false, false),
            ],
        );
        children.insert(
            child.clone(),
            vec![
                entry(nested.clone(), "nested", 0, true, false),
                entry(child.join("child.bin"), "child.bin", 3, false, false),
            ],
        );
        children.insert(
            nested.clone(),
            vec![entry(
                nested.join("nested.bin"),
                "nested.bin",
                5,
                false,
                false,
            )],
        );
        let progress = ScanProgress::new();
        progress.begin_stage(2, children.len());
        let total = apply_directory_sizes(&root, &mut children, &progress, &AtomicBool::new(false));
        assert_eq!(total, 10);
        assert_eq!(children[&root][0].size, 8);
        assert_eq!(children[&child][0].size, 5);
        assert_eq!(progress.get_stage_progress(), (3, 3));
    }

    #[test]
    fn in_place_sizing_handles_a_wide_tree() {
        const DIRECTORY_COUNT: usize = 10_000;
        let root = PathBuf::from("/wide-root");
        let mut root_entries = Vec::with_capacity(DIRECTORY_COUNT);
        let mut children = HashMap::with_capacity(DIRECTORY_COUNT + 1);
        for index in 0..DIRECTORY_COUNT {
            let name = format!("dir-{index}");
            let path = root.join(&name);
            root_entries.push(entry(path.clone(), &name, 0, true, false));
            children.insert(
                path.clone(),
                vec![entry(path.join("file.bin"), "file.bin", 1, false, false)],
            );
        }
        children.insert(root.clone(), root_entries);
        let progress = ScanProgress::new();
        progress.begin_stage(2, children.len());
        assert_eq!(
            apply_directory_sizes(&root, &mut children, &progress, &AtomicBool::new(false),),
            DIRECTORY_COUNT as u64
        );
        assert_eq!(
            progress.get_stage_progress(),
            (DIRECTORY_COUNT + 1, DIRECTORY_COUNT + 1)
        );
        assert_eq!(children[&root][0].size, 1);
    }

    #[test]
    fn build_computes_sizes_temp_flags_navigation_and_progress() {
        let temp = TempDir::new("tree-build");
        temp.write("root.txt", b"12");
        temp.write("src/cache.pyc", b"123");
        temp.write("target/artifact", b"12345");
        let progress = Arc::new(ScanProgress::new());
        let tree = DirTree::build_with_progress(
            temp.path(),
            &matcher(),
            Arc::clone(&progress),
            Arc::new(AtomicBool::new(false)),
            false,
        );
        assert!(progress.is_done());
        assert_eq!(progress.get_files(), 3);
        assert_eq!(progress.get_dirs(), 2);
        assert_eq!(progress.get_bytes(), 10);
        assert_eq!(progress.get_phase(), 3);
        assert_eq!(
            progress.get_stage_progress(),
            (tree.children.len(), tree.children.len())
        );
        let root = tree.get_children(&temp.path().to_path_buf());
        let target = root.iter().find(|e| e.name == "target").unwrap();
        assert!(target.is_temp);
        assert_eq!(target.size, 5);
        let src = tree.get_children(&temp.join("src"));
        assert_eq!(src[0].name, "..");
        assert!(src.iter().find(|e| e.name == "cache.pyc").unwrap().is_temp);
    }

    #[test]
    fn cancelled_build_returns_no_children_and_marks_done() {
        let temp = TempDir::new("tree-cancel");
        temp.write("file", b"data");
        let progress = Arc::new(ScanProgress::new());
        let tree = DirTree::build_with_progress(
            temp.path(),
            &matcher(),
            Arc::clone(&progress),
            Arc::new(AtomicBool::new(true)),
            false,
        );
        assert!(tree.children.is_empty());
        assert!(progress.is_done());
    }

    #[test]
    fn recursive_temp_stats_do_not_double_count_contents_of_temp_dirs() {
        let root = PathBuf::from("/virtual-root");
        let regular = root.join("regular");
        let target = root.join("target");
        let mut children = HashMap::new();
        children.insert(
            root.clone(),
            vec![
                entry(regular.clone(), "regular", 4, true, false),
                entry(target.clone(), "target", 10, true, true),
                entry(root.join("temp.pyc"), "temp.pyc", 2, false, true),
            ],
        );
        children.insert(
            regular.clone(),
            vec![entry(
                regular.join("nested.pyc"),
                "nested.pyc",
                4,
                false,
                true,
            )],
        );
        children.insert(
            target,
            vec![entry(
                root.join("target/inside.pyc"),
                "inside.pyc",
                10,
                false,
                true,
            )],
        );
        let tree = DirTree { children };
        assert_eq!(tree.get_temp_stats(&root), (1, 2, 16));
        assert_eq!(tree.get_temp_stats(Path::new("/missing")), (0, 0, 0));
    }

    #[test]
    fn deleting_entry_updates_ancestors_and_directory_map() {
        let root = PathBuf::from("/root");
        let child = root.join("child");
        let target = child.join("target");
        let mut children = HashMap::new();
        children.insert(
            root.clone(),
            vec![entry(child.clone(), "child", 12, true, false)],
        );
        children.insert(
            child.clone(),
            vec![entry(target.clone(), "target", 7, true, true)],
        );
        children.insert(target.clone(), vec![]);
        let mut tree = DirTree { children };
        tree.delete_entry(&target, true);
        assert!(tree.get_children(&child).is_empty());
        assert_eq!(tree.get_children(&root)[0].size, 5);
        assert!(!tree.children.contains_key(&target));
        tree.delete_entry(&root.join("missing"), false);
    }

    #[test]
    fn sorting_keeps_parent_first_and_directories_before_files() {
        let root = PathBuf::from("/root");
        let mut entries = vec![
            entry(root.join("z.txt"), "z.txt", 100, false, false),
            entry(root.join("b"), "b", 5, true, false),
            entry(root.join("a"), "A", 10, true, false),
            entry(root.parent().unwrap().into(), "..", 0, true, false),
        ];
        sort_by_size(&mut entries);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["..", "A", "b", "z.txt"]
        );
        sort_by_name(&mut entries);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["..", "A", "b", "z.txt"]
        );
    }
}

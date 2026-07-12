//! TUI Application state with threaded deletion and live UI feedback

use super::tree::{self, DirEntry, DirTree};
use crate::deleter::Deleter;
use crate::patterns::PatternMatcher;
use crate::pool::SCAN_POOL;
use crate::scanner::Scanner;
use crate::stats::Stats;
use crossbeam_channel::bounded;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortMode {
    Size,
    Name,
}

/// Deletion state for async deletion
pub struct DeleteState {
    pub handle: JoinHandle<Result<(), String>>,
    pub entry_name: OsString,
    pub entry_path: PathBuf,
    pub is_dir: bool,
    pub entry_size: u64,
}

/// Clean state for async cleaning
pub struct CleanState {
    pub handle: JoinHandle<(usize, usize, u64)>, // (dirs, files, bytes)
    pub cancelled: Arc<AtomicBool>,
}

pub struct RebuildState {
    pub handle: JoinHandle<DirTree>,
    pub completion_message: String,
    pub progress: Arc<tree::ScanProgress>,
    pub cancelled: Arc<AtomicBool>,
    pub restore_path: PathBuf,
    pub restore_name: Option<OsString>,
}

pub struct App {
    pub root: PathBuf,
    pub current_path: PathBuf,
    pub path_stack: Vec<PathBuf>,
    pub entries: Arc<Vec<DirEntry>>,
    pub selected: usize,
    pub sort_mode: SortMode,
    pub confirm_delete: bool,
    pub confirm_clean: bool,
    pub status_message: Option<String>,
    pub status_time: Option<Instant>,
    pub total_size: u64,
    pub disk_total: u64,
    pub disk_free: u64,
    pub force: bool,
    matcher: Arc<PatternMatcher>,
    tree: Option<DirTree>,
    /// Active deletion thread
    delete_state: Option<DeleteState>,
    /// Active clean thread
    clean_state: Option<CleanState>,
    rebuild_state: Option<RebuildState>,
    clean_preview: Option<(usize, usize, u64)>,
}

impl App {
    #[allow(dead_code)]
    pub fn new(root: PathBuf, matcher: Arc<PatternMatcher>, force: bool) -> Self {
        Self {
            current_path: root.clone(),
            root,
            path_stack: Vec::new(),
            entries: Arc::new(Vec::new()),
            selected: 0,
            sort_mode: SortMode::Size,
            confirm_delete: false,
            confirm_clean: false,
            status_message: None,
            status_time: None,
            total_size: 0,
            disk_total: 0,
            disk_free: 0,
            force,
            matcher,
            tree: None,
            delete_state: None,
            clean_state: None,
            rebuild_state: None,
            clean_preview: None,
        }
    }

    pub fn new_with_tree(
        root: PathBuf,
        matcher: Arc<PatternMatcher>,
        tree: DirTree,
        force: bool,
    ) -> Self {
        let mut app = Self {
            current_path: root.clone(),
            root,
            path_stack: Vec::new(),
            entries: Arc::new(Vec::new()),
            selected: 0,
            sort_mode: SortMode::Size,
            confirm_delete: false,
            confirm_clean: false,
            status_message: None,
            status_time: None,
            total_size: 0,
            disk_total: 0,
            disk_free: 0,
            force,
            matcher,
            tree: Some(tree),
            delete_state: None,
            clean_state: None,
            rebuild_state: None,
            clean_preview: None,
        };
        app.load_current_dir();
        app
    }

    /// Check if currently deleting or cleaning
    pub fn is_busy(&self) -> bool {
        self.delete_state.is_some() || self.clean_state.is_some() || self.rebuild_state.is_some()
    }

    /// Check if currently deleting
    pub fn is_deleting(&self) -> bool {
        self.delete_state.is_some()
    }

    /// Check if currently cleaning
    pub fn is_cleaning(&self) -> bool {
        self.clean_state.is_some()
    }

    pub fn rebuild_progress(&self) -> Option<(u8, usize, usize)> {
        self.rebuild_state.as_ref().map(|state| {
            let (current, total) = state.progress.get_stage_progress();
            (state.progress.get_phase(), current, total)
        })
    }

    #[allow(dead_code)]
    pub fn build_tree(&mut self) {
        let progress = Arc::new(tree::ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        self.tree = Some(DirTree::build_with_progress(
            &self.root,
            &self.matcher,
            progress,
            cancelled,
            self.force,
        ));
        self.load_current_dir();
    }

    fn load_current_dir(&mut self) {
        self.load_current_dir_with_selection(None);
    }

    fn load_current_dir_with_selection(&mut self, select_name: Option<&OsStr>) {
        if let Some(ref mut tree) = self.tree {
            self.entries = tree.get_children(&self.current_path, self.sort_mode == SortMode::Name);
            self.total_size = self.entries.iter().map(|e| e.size).sum();
        }

        self.update_disk_usage();

        // Try to find and select the previously entered folder
        if let Some(name) = select_name {
            if let Some(idx) = self.entries.iter().position(|e| e.name == name) {
                self.selected = idx;
            } else {
                self.selected = 0;
            }
        } else {
            self.selected = 0;
        }

        self.confirm_delete = false;
        self.confirm_clean = false;
        self.clean_preview = None;
    }

    fn start_rebuild(&mut self, completion_message: String) {
        let root = self.root.clone();
        let matcher = Arc::clone(&self.matcher);
        let force = self.force;
        let progress = Arc::new(tree::ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_progress = Arc::clone(&progress);
        let worker_cancelled = Arc::clone(&cancelled);
        let restore_path = self.current_path.clone();
        let restore_name = self.selected_entry().map(|entry| entry.name.clone());
        let handle = thread::spawn(move || {
            DirTree::build_with_progress(&root, &matcher, worker_progress, worker_cancelled, force)
        });
        self.rebuild_state = Some(RebuildState {
            handle,
            completion_message,
            progress,
            cancelled,
            restore_path,
            restore_name,
        });
    }

    #[allow(dead_code)]
    pub fn scan_current_dir(&mut self) {
        if self.tree.is_none() {
            self.build_tree();
        } else {
            self.load_current_dir();
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn move_down(&mut self) {
        if self.selected < self.entries.len().saturating_sub(1) {
            self.selected += 1;
        }
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn go_top(&mut self) {
        self.selected = 0;
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn go_bottom(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn enter(&mut self) {
        if self.is_busy() {
            return;
        }
        if let Some(entry) = self.entries.get(self.selected) {
            let is_dir = entry.is_dir;
            let name = entry.name.clone();
            if is_dir {
                if name == ".." {
                    self.go_back();
                } else {
                    self.path_stack.push(self.current_path.clone());
                    self.current_path.push(name);
                    self.load_current_dir();
                }
            }
        }
    }

    pub fn go_back(&mut self) {
        if self.is_busy() {
            return;
        }
        if let Some(prev) = self.path_stack.pop() {
            // Get current folder name to restore cursor position
            let current_name = self.current_path.file_name().map(OsStr::to_os_string);

            self.current_path = prev;
            self.load_current_dir_with_selection(current_name.as_deref());
        }
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn toggle_sort(&mut self) {
        let selected_name = self.selected_entry().map(|entry| entry.name.clone());
        self.sort_mode = match self.sort_mode {
            SortMode::Size => SortMode::Name,
            SortMode::Name => SortMode::Size,
        };
        self.load_current_dir_with_selection(selected_name.as_deref());
    }

    pub fn toggle_delete_confirm(&mut self) {
        if self.is_busy() {
            return;
        }
        if !self.entries.is_empty() {
            let entry = &self.entries[self.selected];
            if entry.name != ".." {
                self.confirm_delete = !self.confirm_delete;
                self.confirm_clean = false;
            }
        }
    }

    pub fn toggle_clean_confirm(&mut self) {
        if self.is_busy() {
            return;
        }
        self.confirm_clean = !self.confirm_clean;
        self.confirm_delete = false;
        self.clean_preview = if self.confirm_clean {
            Some(self.compute_current_temp_stats())
        } else {
            None
        };
    }

    fn set_status(&mut self, msg: String) {
        self.status_message = Some(msg);
        self.status_time = Some(Instant::now());
    }

    /// Check for completed deletion/clean and clear expired status
    pub fn tick(&mut self) {
        // Check if deletion completed
        if let Some(state) = self.delete_state.take() {
            if state.handle.is_finished() {
                // Get the name for cursor restoration
                let deleted_name = state.entry_name.clone();

                match state.handle.join() {
                    Ok(Ok(())) => {
                        self.set_status(format!(
                            "Deleted: {} ({})",
                            state.entry_name.to_string_lossy(),
                            humansize::format_size(state.entry_size, humansize::BINARY)
                        ));

                        // INSTANT UPDATE: Remove from tree in-memory (O(log n))
                        if let Some(ref mut tree) = self.tree {
                            tree.delete_entry(&state.entry_path, state.is_dir);
                        }

                        // Reload and try to keep cursor near deleted item
                        self.load_current_dir_with_selection(Some(deleted_name.as_os_str()));
                    }
                    Ok(Err(e)) => {
                        self.set_status(format!("Error: {}", e));
                    }
                    Err(_) => {
                        self.set_status("Error: deletion thread panicked".to_string());
                    }
                }
            } else {
                self.delete_state = Some(state);
            }
        }

        // Check if clean completed
        if let Some(state) = self.clean_state.take() {
            if state.handle.is_finished() {
                match state.handle.join() {
                    Ok((dirs, files, bytes)) => {
                        let message = format!(
                            "Cleaned: {} dirs, {} files ({})",
                            dirs,
                            files,
                            humansize::format_size(bytes, humansize::BINARY)
                        );
                        self.start_rebuild(message);
                    }
                    Err(_) => {
                        self.set_status("Error: clean thread panicked".to_string());
                    }
                }
            } else {
                self.clean_state = Some(state);
            }
        }

        if let Some(state) = self.rebuild_state.take() {
            if state.handle.is_finished() {
                match state.handle.join() {
                    Ok(tree) => {
                        self.tree = Some(tree);
                        if self
                            .tree
                            .as_ref()
                            .is_some_and(|tree| tree.children.contains_key(&state.restore_path))
                        {
                            self.current_path = state.restore_path;
                        } else {
                            self.current_path = self.root.clone();
                            self.path_stack.clear();
                        }
                        self.load_current_dir_with_selection(state.restore_name.as_deref());
                        self.set_status(state.completion_message);
                    }
                    Err(_) => self.set_status("Error: rebuild thread panicked".to_string()),
                }
            } else {
                self.rebuild_state = Some(state);
            }
        }

        // Clear expired status message
        if let Some(time) = self.status_time {
            if time.elapsed().as_secs() >= 10 {
                self.status_message = None;
                self.status_time = None;
            }
        }
    }

    /// Fast directory deletion using native recursive removal
    fn remove_dir_fast(path: PathBuf) -> Result<(), String> {
        std::fs::remove_dir_all(&path).map_err(|e| e.to_string())
    }

    /// Start async deletion
    pub fn delete_selected(&mut self) {
        if self.is_busy() {
            return;
        }

        if let Some(entry) = self.entries.get(self.selected) {
            if entry.name == ".." {
                self.confirm_delete = false;
                return;
            }

            let entry_name = entry.name.clone();
            let entry_size = entry.size;
            let is_dir = entry.is_dir;
            let path = self.current_path.join(&entry_name);

            if !is_dir {
                match fs::remove_file(&path) {
                    Ok(()) => {
                        self.set_status(format!(
                            "Deleted: {} ({})",
                            entry_name.to_string_lossy(),
                            humansize::format_size(entry_size, humansize::BINARY)
                        ));
                        if let Some(tree) = &mut self.tree {
                            tree.delete_entry(&path, false);
                        }
                        self.load_current_dir_with_selection(Some(entry_name.as_os_str()));
                    }
                    Err(error) => self.set_status(format!("Error: {error}")),
                }
                self.confirm_delete = false;
                return;
            }

            let worker_path = path.clone();
            let handle = thread::spawn(move || Self::remove_dir_fast(worker_path));

            self.delete_state = Some(DeleteState {
                handle,
                entry_name,
                entry_path: path,
                is_dir,
                entry_size,
            });
        }
        self.confirm_delete = false;
    }

    /// Start async clean of current directory (uses main scanner)
    pub fn clean_current(&mut self) {
        if self.is_busy() {
            return;
        }

        let root = self.current_path.clone();
        let config = self.matcher.config();
        let num_threads = SCAN_POOL.current_num_threads();
        let worker_pool = crate::pool::build_worker_pool(num_threads, "cleaner-worker");
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);

        let handle = thread::spawn(move || {
            let stats = Arc::new(Stats::new());

            let (tx, rx) = bounded(1024);
            let scanner = Scanner::with_pool(root, Arc::clone(&worker_pool), config);

            // Scan and delete concurrently while the bounded channel applies
            // backpressure if deletion falls behind discovery.
            let scan_handle =
                thread::spawn(move || scanner.scan_with_cancel(tx, &worker_cancelled));

            let deleter = Deleter::with_pool(Arc::clone(&stats), false, false, worker_pool);
            deleter.process(rx);
            if let Ok(summary) = scan_handle.join() {
                stats.add_errors(summary.errors);
            }

            (stats.directories(), stats.files(), stats.bytes())
        });

        self.clean_state = Some(CleanState { handle, cancelled });
        self.confirm_clean = false;
    }

    pub fn current_temp_stats(&self) -> (usize, usize, u64) {
        if self.confirm_clean {
            self.clean_preview
                .unwrap_or_else(|| self.compute_current_temp_stats())
        } else {
            self.compute_current_temp_stats()
        }
    }

    fn compute_current_temp_stats(&self) -> (usize, usize, u64) {
        if let Some(ref tree) = self.tree {
            tree.get_temp_stats(&self.current_path)
        } else {
            (0, 0, 0)
        }
    }

    pub fn refresh(&mut self) {
        if self.is_busy() {
            return;
        }
        self.start_rebuild("Refreshed".to_string());
    }

    pub fn selected_entry(&self) -> Option<&DirEntry> {
        self.entries.get(self.selected)
    }

    pub fn update_disk_usage(&mut self) {
        if let Some((total, free)) = get_disk_usage(&self.current_path) {
            self.disk_total = total;
            self.disk_free = free;
        } else {
            self.disk_total = 0;
            self.disk_free = 0;
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(state) = self.rebuild_state.take() {
            state
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = state.handle.join();
        }
        if let Some(state) = self.clean_state.take() {
            state
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = state.handle.join();
        }
        if let Some(state) = self.delete_state.take() {
            let _ = state.handle.join();
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn get_disk_usage(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let path_str = path.to_str()?;
    let c_path = CString::new(path_str).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            let block_size = if stat.f_frsize > 0 {
                stat.f_frsize as u64
            } else {
                stat.f_bsize as u64
            };
            let total = block_size * stat.f_blocks as u64;
            let free = block_size * stat.f_bavail as u64;
            Some((total, free))
        } else {
            None
        }
    }
}

#[cfg(target_os = "freebsd")]
fn get_disk_usage(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let path_str = path.to_str()?;
    let c_path = CString::new(path_str).ok()?;
    unsafe {
        let mut stat: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut stat) == 0 {
            let block_size = stat.f_bsize as u64;
            let total = block_size * stat.f_blocks as u64;
            let free = block_size * stat.f_bavail as u64;
            Some((total, free))
        } else {
            None
        }
    }
}

#[cfg(windows)]
fn get_disk_usage(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let mut path_u16: Vec<u16> = OsStr::new(path).encode_wide().collect();
    path_u16.push(0);

    let mut free_bytes = 0u64;
    let mut total_bytes = 0u64;
    let mut total_free = 0u64;

    unsafe {
        if GetDiskFreeSpaceExW(
            path_u16.as_ptr(),
            &mut free_bytes,
            &mut total_bytes,
            &mut total_free,
        ) != 0
        {
            Some((total_bytes, free_bytes))
        } else {
            None
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn get_disk_usage(_path: &std::path::Path) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::test_support::TempDir;
    use foldhash::{HashMap, HashMapExt};
    use std::time::Duration;

    fn matcher() -> Arc<PatternMatcher> {
        Arc::new(PatternMatcher::new(Arc::new(Config {
            directories: vec!["target".into()],
            files: vec![".pyc".into()],
            days: None,
            force: false,
        })))
    }

    fn entry(_path: PathBuf, name: &str, size: u64, is_dir: bool, is_temp: bool) -> DirEntry {
        DirEntry {
            name: name.into(),
            size,
            is_dir,
            is_temp,
        }
    }

    fn app_with_tree(temp: &TempDir) -> App {
        let root = temp.path().to_path_buf();
        let folder = temp.join("folder");
        let mut children = HashMap::new();
        children.insert(
            root.clone(),
            vec![
                entry(folder.clone(), "folder", 8, true, false),
                entry(temp.join("cache.pyc"), "cache.pyc", 3, false, true),
            ],
        );
        children.insert(
            folder.clone(),
            vec![
                entry(root.clone(), "..", 0, true, false),
                entry(folder.join("nested.pyc"), "nested.pyc", 8, false, true),
            ],
        );
        App::new_with_tree(root, matcher(), DirTree::from_children(children), false)
    }

    fn select(app: &mut App, name: &str) {
        app.selected = app.entries.iter().position(|e| e.name == name).unwrap();
    }

    fn wait_until_idle(app: &mut App) {
        for _ in 0..200 {
            app.tick();
            if !app.is_busy() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("background operation did not finish");
    }

    #[test]
    fn navigation_selection_sorting_and_confirmations_work() {
        let temp = TempDir::new("app-navigation");
        let mut app = app_with_tree(&temp);
        assert_eq!(app.total_size, 11);
        assert_eq!(app.selected, 0);
        assert!(app.selected_entry().is_some());

        app.move_down();
        assert_eq!(app.selected, 1);
        app.move_down();
        assert_eq!(app.selected, 1);
        app.move_up();
        app.go_bottom();
        assert_eq!(app.selected, 1);
        app.go_top();
        assert_eq!(app.selected, 0);

        app.toggle_sort();
        assert_eq!(app.sort_mode, SortMode::Name);
        app.toggle_sort();
        assert_eq!(app.sort_mode, SortMode::Size);

        select(&mut app, "cache.pyc");
        app.toggle_delete_confirm();
        assert!(app.confirm_delete);
        app.toggle_clean_confirm();
        assert!(app.confirm_clean);
        assert!(app.clean_preview.is_some());
        assert!(!app.confirm_delete);
        assert_eq!(app.current_temp_stats(), (0, 2, 11));
    }

    #[test]
    fn entering_and_leaving_directory_restores_selection() {
        let temp = TempDir::new("app-enter");
        let mut app = app_with_tree(&temp);
        select(&mut app, "folder");
        app.enter();
        assert_eq!(app.current_path, temp.join("folder"));
        assert_eq!(app.path_stack, [temp.path().to_path_buf()]);
        select(&mut app, "..");
        app.enter();
        assert_eq!(app.current_path, temp.path());
        assert_eq!(app.selected_entry().unwrap().name, "folder");
        app.go_back(); // Already at root: no-op.
        assert_eq!(app.current_path, temp.path());
    }

    #[test]
    fn deleting_selected_file_updates_disk_and_in_memory_tree() {
        let temp = TempDir::new("app-delete");
        temp.mkdir("folder");
        let file = temp.write("cache.pyc", b"123");
        let mut app = app_with_tree(&temp);
        select(&mut app, "cache.pyc");
        app.delete_selected();
        assert!(!app.is_busy());
        assert!(!app.is_deleting());
        assert!(!file.exists());
        assert!(!app.entries.iter().any(|e| e.name == "cache.pyc"));
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .starts_with("Deleted:"));
    }

    #[test]
    fn directory_deletion_helper_and_error_status_are_covered() {
        let temp = TempDir::new("app-remove-dir");
        let directory = temp.mkdir("remove-me/nested");
        assert!(App::remove_dir_fast(temp.join("remove-me")).is_ok());
        assert!(!directory.exists());
        assert!(App::remove_dir_fast(temp.join("missing")).is_err());

        let mut app = app_with_tree(&temp);
        app.delete_state = Some(DeleteState {
            handle: thread::spawn(|| Err("expected failure".into())),
            entry_name: "bad".into(),
            entry_path: temp.join("bad"),
            is_dir: false,
            entry_size: 0,
        });
        wait_until_idle(&mut app);
        assert_eq!(
            app.status_message.as_deref(),
            Some("Error: expected failure")
        );
    }

    #[test]
    fn clean_current_removes_default_patterns_and_rebuilds() {
        let temp = TempDir::new("app-clean");
        temp.write("target/artifact", b"12345");
        temp.write("keep/source.rs", b"keep");
        let mut app = App::new(temp.path().to_path_buf(), matcher(), false);
        app.scan_current_dir();
        assert!(!app.entries.is_empty());
        app.toggle_clean_confirm();
        app.clean_current();
        assert!(app.is_cleaning());
        wait_until_idle(&mut app);
        assert!(!temp.join("target").exists());
        assert!(temp.join("keep/source.rs").exists());
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .starts_with("Cleaned:"));

        app.refresh();
        wait_until_idle(&mut app);
        assert_eq!(app.status_message.as_deref(), Some("Refreshed"));
    }

    #[test]
    fn tick_expires_old_status_and_disk_usage_handles_valid_path() {
        let temp = TempDir::new("app-status");
        let mut app = app_with_tree(&temp);
        app.set_status("old".into());
        app.status_time = Some(Instant::now() - Duration::from_secs(11));
        app.tick();
        assert!(app.status_message.is_none());
        app.update_disk_usage();
        #[cfg(any(unix, windows))]
        assert!(app.disk_total > 0);
    }
}

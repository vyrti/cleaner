//! TUI Application state with threaded deletion and live UI feedback

use super::tree::{self, DirEntry, DirTree};
use crate::patterns::PatternMatcher;
use jwalk::WalkDir;
use rayon::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
    pub entry_name: String,
    pub entry_size: u64,
}

pub struct App {
    pub root: PathBuf,
    pub current_path: PathBuf,
    pub path_stack: Vec<PathBuf>,
    pub entries: Vec<DirEntry>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub sort_mode: SortMode,
    pub confirm_delete: bool,
    pub status_message: Option<String>,
    pub status_time: Option<Instant>,
    pub total_size: u64,
    matcher: Arc<PatternMatcher>,
    tree: Option<DirTree>,
    /// Active deletion thread
    delete_state: Option<DeleteState>,
}

impl App {
    pub fn new(root: PathBuf, matcher: Arc<PatternMatcher>) -> Self {
        Self {
            current_path: root.clone(),
            root,
            path_stack: Vec::new(),
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            sort_mode: SortMode::Size,
            confirm_delete: false,
            status_message: None,
            status_time: None,
            total_size: 0,
            matcher,
            tree: None,
            delete_state: None,
        }
    }

    pub fn new_with_tree(root: PathBuf, matcher: Arc<PatternMatcher>, tree: DirTree) -> Self {
        let mut app = Self {
            current_path: root.clone(),
            root,
            path_stack: Vec::new(),
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            sort_mode: SortMode::Size,
            confirm_delete: false,
            status_message: None,
            status_time: None,
            total_size: 0,
            matcher,
            tree: Some(tree),
            delete_state: None,
        };
        app.load_current_dir();
        app
    }

    /// Check if currently deleting
    pub fn is_deleting(&self) -> bool {
        self.delete_state.is_some()
    }

    pub fn build_tree(&mut self) {
        let progress = Arc::new(tree::ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        self.tree = Some(DirTree::build_with_progress(&self.root, &self.matcher, progress, cancelled));
        self.load_current_dir();
    }

    fn load_current_dir(&mut self) {
        if let Some(ref tree) = self.tree {
            self.entries = tree.get_children(&self.current_path);
            self.apply_sort();
            self.total_size = self.entries.iter().map(|e| e.size).sum();
        }
        self.selected = 0;
        self.scroll_offset = 0;
        self.confirm_delete = false;
    }

    fn rebuild_tree(&mut self) {
        let progress = Arc::new(tree::ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        self.tree = Some(DirTree::build_with_progress(&self.root, &self.matcher, progress, cancelled));
        self.load_current_dir();
    }

    pub fn scan_current_dir(&mut self) {
        if self.tree.is_none() {
            self.build_tree();
        } else {
            self.load_current_dir();
        }
    }

    fn apply_sort(&mut self) {
        match self.sort_mode {
            SortMode::Size => tree::sort_by_size(&mut self.entries),
            SortMode::Name => tree::sort_by_name(&mut self.entries),
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
        self.confirm_delete = false;
    }

    pub fn move_down(&mut self) {
        if self.selected < self.entries.len().saturating_sub(1) {
            self.selected += 1;
        }
        self.confirm_delete = false;
    }

    pub fn go_top(&mut self) {
        self.selected = 0;
        self.scroll_offset = 0;
        self.confirm_delete = false;
    }

    pub fn go_bottom(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
        self.confirm_delete = false;
    }

    pub fn enter(&mut self) {
        if self.is_deleting() { return; }
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            if entry.is_dir {
                if entry.name == ".." {
                    self.go_back();
                } else {
                    self.path_stack.push(self.current_path.clone());
                    self.current_path = entry.path.clone();
                    self.load_current_dir();
                }
            }
        }
    }

    pub fn go_back(&mut self) {
        if self.is_deleting() { return; }
        if let Some(prev) = self.path_stack.pop() {
            self.current_path = prev;
            self.load_current_dir();
        }
        self.confirm_delete = false;
    }

    pub fn toggle_sort(&mut self) {
        self.sort_mode = match self.sort_mode {
            SortMode::Size => SortMode::Name,
            SortMode::Name => SortMode::Size,
        };
        self.apply_sort();
    }

    pub fn toggle_delete_confirm(&mut self) {
        if self.is_deleting() { return; }
        if !self.entries.is_empty() {
            let entry = &self.entries[self.selected];
            if entry.name != ".." {
                self.confirm_delete = !self.confirm_delete;
            }
        }
    }

    fn set_status(&mut self, msg: String) {
        self.status_message = Some(msg);
        self.status_time = Some(Instant::now());
    }

    /// Check for completed deletion and clear expired status
    pub fn tick(&mut self) {
        // Check if deletion completed
        if let Some(state) = self.delete_state.take() {
            if state.handle.is_finished() {
                match state.handle.join() {
                    Ok(Ok(())) => {
                        self.set_status(format!(
                            "Deleted: {} ({})",
                            state.entry_name,
                            humansize::format_size(state.entry_size, humansize::BINARY)
                        ));
                        self.rebuild_tree();
                    }
                    Ok(Err(e)) => {
                        self.set_status(format!("Error: {}", e));
                    }
                    Err(_) => {
                        self.set_status("Error: deletion thread panicked".to_string());
                    }
                }
            } else {
                // Not finished yet, put it back
                self.delete_state = Some(state);
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
    #[cfg(unix)]
    fn remove_dir_fast(path: PathBuf) -> Result<(), String> {
        // Use std::fs::remove_dir_all which uses unlinkat() internally on Unix
        // This is already the fastest Rust-native approach
        std::fs::remove_dir_all(&path).map_err(|e| e.to_string())
    }

    #[cfg(windows)]
    fn remove_dir_fast(path: PathBuf) -> Result<(), String> {
        // Windows: use parallel Rust deletion
        let files: Vec<PathBuf> = WalkDir::new(&path)
            .parallelism(jwalk::Parallelism::RayonNewPool(num_cpus::get()))
            .skip_hidden(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path())
            .collect();

        files.par_iter().for_each(|f| {
            let _ = fs::remove_file(f);
        });

        let mut dirs: Vec<PathBuf> = WalkDir::new(&path)
            .skip_hidden(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_dir())
            .map(|e| e.path())
            .collect();

        dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));

        for dir in dirs {
            let _ = fs::remove_dir(&dir);
        }

        Ok(())
    }

    /// Start async deletion
    pub fn delete_selected(&mut self) {
        if self.is_deleting() { return; }

        if let Some(entry) = self.entries.get(self.selected).cloned() {
            if entry.name == ".." {
                self.confirm_delete = false;
                return;
            }

            let path = entry.path.clone();
            let is_dir = entry.is_dir;

            // Start deletion in background thread
            let handle = thread::spawn(move || {
                if is_dir {
                    Self::remove_dir_fast(path)
                } else {
                    fs::remove_file(&path).map_err(|e| e.to_string())
                }
            });

            self.delete_state = Some(DeleteState {
                handle,
                entry_name: entry.name.clone(),
                entry_size: entry.size,
            });
        }
        self.confirm_delete = false;
    }

    pub fn refresh(&mut self) {
        if self.is_deleting() { return; }
        self.rebuild_tree();
        self.set_status("Refreshed".to_string());
    }

    pub fn selected_entry(&self) -> Option<&DirEntry> {
        self.entries.get(self.selected)
    }
}

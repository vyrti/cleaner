//! TUI Application state with threaded deletion and live UI feedback

use super::tree::{self, DirEntry, DirTree};
use crate::patterns::PatternMatcher;
use crate::scanner::Scanner;
use crate::deleter::Deleter;
use crate::stats::Stats;
use crossbeam_channel::unbounded;
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
    pub entry_name: String,
    pub entry_path: PathBuf,
    pub is_dir: bool,
    pub entry_size: u64,
}

/// Clean state for async cleaning
pub struct CleanState {
    pub handle: JoinHandle<(usize, usize, u64)>, // (dirs, files, bytes)
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
    pub confirm_clean: bool,
    pub status_message: Option<String>,
    pub status_time: Option<Instant>,
    pub total_size: u64,
    matcher: Arc<PatternMatcher>,
    tree: Option<DirTree>,
    /// Active deletion thread
    delete_state: Option<DeleteState>,
    /// Active clean thread
    clean_state: Option<CleanState>,
    /// Last entered folder name (for cursor restoration on go_back)
    last_entered_folder: Option<String>,
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
            confirm_clean: false,
            status_message: None,
            status_time: None,
            total_size: 0,
            matcher,
            tree: None,
            delete_state: None,
            clean_state: None,
            last_entered_folder: None,
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
            confirm_clean: false,
            status_message: None,
            status_time: None,
            total_size: 0,
            matcher,
            tree: Some(tree),
            delete_state: None,
            clean_state: None,
            last_entered_folder: None,
        };
        app.load_current_dir();
        app
    }

    /// Check if currently deleting or cleaning
    pub fn is_busy(&self) -> bool {
        self.delete_state.is_some() || self.clean_state.is_some()
    }

    /// Check if currently deleting
    pub fn is_deleting(&self) -> bool {
        self.delete_state.is_some()
    }

    /// Check if currently cleaning
    pub fn is_cleaning(&self) -> bool {
        self.clean_state.is_some()
    }

    pub fn build_tree(&mut self) {
        let progress = Arc::new(tree::ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        self.tree = Some(DirTree::build_with_progress(&self.root, &self.matcher, progress, cancelled));
        self.load_current_dir();
    }

    fn load_current_dir(&mut self) {
        self.load_current_dir_with_selection(None);
    }

    fn load_current_dir_with_selection(&mut self, select_name: Option<&str>) {
        if let Some(ref tree) = self.tree {
            self.entries = tree.get_children(&self.current_path);
            self.apply_sort();
            self.total_size = self.entries.iter().map(|e| e.size).sum();
        }
        
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
        
        self.scroll_offset = 0;
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    fn rebuild_tree(&mut self) {
        let progress = Arc::new(tree::ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        self.tree = Some(DirTree::build_with_progress(&self.root, &self.matcher, progress, cancelled));
        self.load_current_dir();
    }

    fn rebuild_tree_with_selection(&mut self, select_name: Option<&str>) {
        let progress = Arc::new(tree::ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        self.tree = Some(DirTree::build_with_progress(&self.root, &self.matcher, progress, cancelled));
        self.load_current_dir_with_selection(select_name);
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
        self.scroll_offset = 0;
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn go_bottom(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn enter(&mut self) {
        if self.is_busy() { return; }
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            if entry.is_dir {
                if entry.name == ".." {
                    self.go_back();
                } else {
                    // Remember the folder name we're entering
                    self.last_entered_folder = Some(entry.name.clone());
                    self.path_stack.push(self.current_path.clone());
                    self.current_path = entry.path.clone();
                    self.load_current_dir();
                }
            }
        }
    }

    pub fn go_back(&mut self) {
        if self.is_busy() { return; }
        if let Some(prev) = self.path_stack.pop() {
            // Get current folder name to restore cursor position
            let current_name = self.current_path.file_name()
                .map(|n| n.to_string_lossy().to_string());
            
            self.current_path = prev;
            self.load_current_dir_with_selection(current_name.as_deref());
        }
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn toggle_sort(&mut self) {
        self.sort_mode = match self.sort_mode {
            SortMode::Size => SortMode::Name,
            SortMode::Name => SortMode::Size,
        };
        self.apply_sort();
    }

    pub fn toggle_delete_confirm(&mut self) {
        if self.is_busy() { return; }
        if !self.entries.is_empty() {
            let entry = &self.entries[self.selected];
            if entry.name != ".." {
                self.confirm_delete = !self.confirm_delete;
                self.confirm_clean = false;
            }
        }
    }

    pub fn toggle_clean_confirm(&mut self) {
        if self.is_busy() { return; }
        self.confirm_clean = !self.confirm_clean;
        self.confirm_delete = false;
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
                            state.entry_name,
                            humansize::format_size(state.entry_size, humansize::BINARY)
                        ));
                        
                        // INSTANT UPDATE: Remove from tree in-memory (O(log n))
                        if let Some(ref mut tree) = self.tree {
                            tree.delete_entry(&state.entry_path, state.is_dir);
                        }
                        
                        // Reload and try to keep cursor near deleted item
                        self.load_current_dir_with_selection(Some(&deleted_name));
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
                        self.set_status(format!(
                            "Cleaned: {} dirs, {} files ({})",
                            dirs,
                            files,
                            humansize::format_size(bytes, humansize::BINARY)
                        ));
                        // Full rebuild needed after clean
                        self.rebuild_tree();
                    }
                    Err(_) => {
                        self.set_status("Error: clean thread panicked".to_string());
                    }
                }
            } else {
                self.clean_state = Some(state);
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
        if self.is_busy() { return; }

        if let Some(entry) = self.entries.get(self.selected).cloned() {
            if entry.name == ".." {
                self.confirm_delete = false;
                return;
            }

            let path = entry.path.clone();
            let is_dir = entry.is_dir;

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
                entry_path: entry.path.clone(),
                is_dir: entry.is_dir,
                entry_size: entry.size,
            });
        }
        self.confirm_delete = false;
    }

    /// Start async clean of current directory (uses main scanner)
    pub fn clean_current(&mut self) {
        if self.is_busy() { return; }
        
        let root = self.current_path.clone();
        let matcher = Arc::clone(&self.matcher);
        
        let handle = thread::spawn(move || {
            let stats = Arc::new(Stats::new());
            let config = crate::config::Config::default();
            let config = Arc::new(config);
            
            let (tx, rx) = unbounded();
            let scanner = Scanner::new(root, num_cpus::get(), config);
            
            // Run scanner in this thread
            let _scanned = scanner.scan(tx);
            
            // Process deletions
            let deleter = Deleter::new(Arc::clone(&stats), false, false);
            deleter.process(rx);
            
            (stats.directories(), stats.files(), stats.bytes())
        });

        self.clean_state = Some(CleanState { handle });
        self.confirm_clean = false;
    }

    pub fn refresh(&mut self) {
        if self.is_busy() { return; }
        self.rebuild_tree();
        self.set_status("Refreshed".to_string());
    }

    pub fn selected_entry(&self) -> Option<&DirEntry> {
        self.entries.get(self.selected)
    }
}

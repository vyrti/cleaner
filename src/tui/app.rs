//! TUI Application state with instant navigation

use super::tree::{self, DirEntry, DirTree};
use crate::patterns::PatternMatcher;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortMode {
    Size,
    Name,
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
    pub total_size: u64,
    matcher: Arc<PatternMatcher>,
    /// Pre-built directory tree (instant lookups)
    tree: Option<DirTree>,
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
            total_size: 0,
            matcher,
            tree: None,
        }
    }

    /// Create app with pre-built tree (from threaded scan)
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
            total_size: 0,
            matcher,
            tree: Some(tree),
        };
        app.load_current_dir();
        app
    }

    /// Build the full directory tree (call once at startup)
    pub fn build_tree(&mut self) {
        let progress = std::sync::Arc::new(tree::ScanProgress::new());
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.tree = Some(DirTree::build_with_progress(&self.root, &self.matcher, progress, cancelled));
        self.load_current_dir();
    }

    /// Load entries for current directory (instant from tree)
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

    /// Rebuild tree after deletion
    fn rebuild_tree(&mut self) {
        let progress = std::sync::Arc::new(tree::ScanProgress::new());
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.tree = Some(DirTree::build_with_progress(&self.root, &self.matcher, progress, cancelled));
        self.load_current_dir();
    }

    /// Legacy method for compatibility
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
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            if entry.is_dir {
                self.path_stack.push(self.current_path.clone());
                self.current_path = entry.path.clone();
                self.load_current_dir(); // Instant!
            }
        }
    }

    pub fn go_back(&mut self) {
        if let Some(prev) = self.path_stack.pop() {
            self.current_path = prev;
            self.load_current_dir(); // Instant!
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
        if !self.entries.is_empty() {
            self.confirm_delete = !self.confirm_delete;
        }
    }

    pub fn delete_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            let result = if entry.is_dir {
                fs::remove_dir_all(&entry.path)
            } else {
                fs::remove_file(&entry.path)
            };

            match result {
                Ok(_) => {
                    self.status_message = Some(format!(
                        "Deleted: {} ({})",
                        entry.name,
                        humansize::format_size(entry.size, humansize::BINARY)
                    ));
                    // Rebuild tree after deletion
                    self.rebuild_tree();
                }
                Err(e) => {
                    self.status_message = Some(format!("Error: {}", e));
                }
            }
        }
        self.confirm_delete = false;
    }

    pub fn refresh(&mut self) {
        self.rebuild_tree();
        self.status_message = Some("Refreshed".to_string());
    }

    pub fn selected_entry(&self) -> Option<&DirEntry> {
        self.entries.get(self.selected)
    }
}

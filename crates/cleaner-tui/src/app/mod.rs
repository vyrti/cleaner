//! TUI Application state with threaded deletion and live UI feedback

mod actions;
mod navigation;
mod state;

#[cfg(test)]
mod tests;

pub use state::{CleanState, DeleteState, RebuildState, SortMode};

use cleaner_core::get_disk_usage;
use cleaner_core::patterns::PatternMatcher;
use cleaner_core::tree::{DirEntry, DirTree};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

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
    delete_state: Option<DeleteState>,
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

    pub fn is_rebuilding(&self) -> bool {
        self.rebuild_state.is_some()
    }

    pub fn selected_entry(&self) -> Option<&DirEntry> {
        self.entries.get(self.selected)
    }

    pub fn update_disk_usage(&mut self) {
        if let Some((total, free)) = get_disk_usage(self.current_path.as_path()) {
            self.disk_total = total;
            self.disk_free = free;
        } else {
            self.disk_total = 0;
            self.disk_free = 0;
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
        self.status_time = Some(Instant::now());
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
        self.status_time = None;
    }

    pub fn rebuild_progress(&self) -> Option<(u8, usize, usize)> {
        let state = self.rebuild_state.as_ref()?;
        let (current, total) = state.progress.get_stage_progress();
        Some((state.progress.get_phase(), current, total))
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

use super::state::{CleanState, DeleteState, RebuildState, SortMode};
use super::App;
use cleaner_core::deleter::Deleter;
use cleaner_core::pool::SCAN_POOL;
use cleaner_core::scanner::Scanner;
use cleaner_core::stats::Stats;
use cleaner_core::tree::{self, DirTree};
use crossbeam_channel::bounded;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;

impl App {
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

    pub fn load_current_dir(&mut self) {
        self.load_current_dir_with_selection(None);
    }

    pub(crate) fn load_current_dir_with_selection(&mut self, select_name: Option<&OsStr>) {
        if let Some(ref mut tree) = self.tree {
            let by_name = self.sort_mode == SortMode::Name;
            self.entries = tree.get_children(&self.current_path, by_name);

            // Compute total size for current dir (excluding ".." and parent refs)
            self.total_size = self
                .entries
                .iter()
                .filter(|e| e.name != "..")
                .map(|e| e.size)
                .sum();

            // Try to preserve or find selection
            if let Some(name) = select_name {
                if let Some(idx) = self.entries.iter().position(|e| e.name == name) {
                    self.selected = idx;
                } else {
                    self.selected = self.selected.min(self.entries.len().saturating_sub(1));
                }
            } else {
                self.selected = 0;
            }
        }
        self.update_disk_usage();
    }

    pub fn build_tree(&mut self) {
        let root = self.root.clone();
        let matcher = Arc::clone(&self.matcher);
        let progress = Arc::new(tree::ScanProgress::new());
        let tree = DirTree::build_with_progress(
            &root,
            &matcher,
            progress,
            Arc::new(AtomicBool::new(false)),
            self.force,
        );
        self.tree = Some(tree);
        self.load_current_dir();
    }

    pub(crate) fn start_rebuild(&mut self, completion_message: String) {
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

    pub fn cancel_rebuild(&mut self) {
        if let Some(state) = self.rebuild_state.take() {
            state
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = state.handle.join();
            self.set_status("Rebuild cancelled");
        }
    }

    /// Check for completed deletion/clean and clear expired status
    pub fn tick(&mut self) {
        self.tick_deep();
        // Check if deletion completed
        if let Some(state) = self.delete_state.take() {
            if state.handle.is_finished() {
                let deleted_name = state.entry_name.clone();

                match state.handle.join() {
                    Ok(Ok(())) => {
                        self.set_status(format!(
                            "Deleted: {} ({})",
                            state.entry_name.to_string_lossy(),
                            humansize::format_size(state.entry_size, humansize::BINARY)
                        ));

                        if let Some(ref mut tree) = self.tree {
                            tree.delete_entry(&state.entry_path, state.is_dir);
                        }

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
                            self.selected = 0;
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

    pub(crate) fn remove_dir_fast(path: PathBuf) -> Result<(), String> {
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

            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.set_status(format!("Delete rejected: {error}"));
                    self.confirm_delete = false;
                    return;
                }
            };
            let actual = metadata.file_type();
            if actual.is_symlink() || actual.is_dir() != is_dir || (!is_dir && !actual.is_file()) {
                self.set_status("Delete rejected: path type changed since scan");
                self.confirm_delete = false;
                return;
            }

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
        let worker_pool = cleaner_core::pool::build_worker_pool(num_threads, "cleaner-worker");
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);

        let handle = thread::spawn(move || {
            let stats = Arc::new(Stats::new());

            let (tx, rx) = bounded(1024);
            let scanner = Scanner::with_pool(root, Arc::clone(&worker_pool), config);

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

    pub fn compute_temp_stats_for_offer(&self) -> (usize, usize, u64) {
        self.compute_current_temp_stats()
    }

    pub(crate) fn compute_current_temp_stats(&self) -> (usize, usize, u64) {
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

    #[allow(dead_code)]
    pub fn scan_current_dir(&mut self) {
        if self.tree.is_none() {
            self.build_tree();
        } else {
            self.load_current_dir();
        }
    }
}

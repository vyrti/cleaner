//! Full-screen session lifecycle, background scan handoff, and input mapping.

mod keys;
mod runner;
mod types;

#[cfg(test)]
mod tests;

pub use runner::run;
pub use types::{CleanOffer, Outcome, StartOpts};

use crate::app::App;
use crate::ui::{self, Chrome};
use cleaner_core::config::Config;
use cleaner_core::patterns::PatternMatcher;
use cleaner_core::tree::{DirTree, ScanProgress};
use ratatui::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

pub struct Session {
    phase: types::Phase,
}

impl Session {
    pub fn start(root: PathBuf, config: Arc<Config>, _opts: StartOpts) -> Self {
        let matcher = Arc::new(PatternMatcher::new(Arc::clone(&config)));
        let force = config.force;
        let progress = Arc::new(ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        let progress_clone = Arc::clone(&progress);
        let cancelled_clone = Arc::clone(&cancelled);
        let root_clone = root.clone();
        let matcher_clone = Arc::clone(&matcher);
        let scan_handle = Some(thread::spawn(move || {
            DirTree::build_with_progress(
                &root_clone,
                &matcher_clone,
                progress_clone,
                cancelled_clone,
                force,
            )
        }));

        Self {
            phase: types::Phase::Scanning {
                root,
                progress,
                cancelled,
                scan_handle,
                matcher,
                force,
            },
        }
    }

    pub fn tick(&mut self) {
        match &mut self.phase {
            types::Phase::Scanning {
                progress,
                scan_handle,
                root,
                matcher,
                force,
                ..
            } => {
                if !progress.is_done() {
                    return;
                }
                let Some(handle) = scan_handle.take() else {
                    return;
                };
                let dir_tree = match handle.join() {
                    Ok(tree) => tree,
                    Err(_) => {
                        self.phase = types::Phase::Exited;
                        return;
                    }
                };
                let app = App::new_with_tree(root.clone(), Arc::clone(matcher), dir_tree, *force);
                self.phase = types::Phase::Ready(Box::new(app));
            }
            types::Phase::Ready(app) => app.tick(),
            types::Phase::Exited => {}
        }
    }

    /// Draw with full standalone chrome (digit bar) or content-only for Abyss embed.
    pub fn draw(&self, frame: &mut Frame, area: Rect, chrome: Chrome) {
        match &self.phase {
            types::Phase::Scanning { root, progress, .. } => {
                ui::draw_scan_progress(frame, area, root, progress)
            }
            types::Phase::Ready(app) => ui::render_in(frame, app, area, chrome),
            types::Phase::Exited => {}
        }
    }

    pub fn status_line(&self) -> Option<String> {
        match &self.phase {
            types::Phase::Scanning { root, progress, .. } => {
                let phase = progress.get_phase();
                let name = match phase {
                    0 => "Scanning",
                    1 => "Indexing",
                    2 => "Sizing",
                    _ => "Finalizing",
                };
                Some(format!(
                    "{name} {} — {} folders, {} files",
                    root.display(),
                    progress.get_dirs(),
                    progress.get_files()
                ))
            }
            types::Phase::Ready(app) => ui::status_line(app),
            types::Phase::Exited => None,
        }
    }

    pub fn refresh(&mut self) {
        if let types::Phase::Ready(app) = &mut self.phase {
            app.refresh();
        }
    }

    pub fn toggle_sort(&mut self) {
        if let types::Phase::Ready(app) = &mut self.phase {
            app.toggle_sort();
        }
    }

    pub fn toggle_clean_confirm(&mut self) {
        if let types::Phase::Ready(app) = &mut self.phase {
            app.toggle_clean_confirm();
        }
    }

    /// Preview for an Abyss-hosted Clean confirmation dialog.
    pub fn clean_offer(&self) -> CleanOffer {
        let types::Phase::Ready(app) = &self.phase else {
            return CleanOffer::Unavailable("Analyze is still scanning".into());
        };
        if app.is_busy() {
            return CleanOffer::Unavailable("Cleaner is busy".into());
        }
        let (dirs, files, bytes) = app.compute_temp_stats_for_offer();
        if dirs == 0 && files == 0 {
            return CleanOffer::Empty {
                path: app.current_path.clone(),
            };
        }
        CleanOffer::Ready {
            path: app.current_path.clone(),
            dirs,
            files,
            bytes,
        }
    }

    pub fn run_clean(&mut self) {
        if let types::Phase::Ready(app) = &mut self.phase {
            app.clean_current();
        }
    }

    pub fn toggle_delete_confirm(&mut self) {
        if let types::Phase::Ready(app) = &mut self.phase {
            app.toggle_delete_confirm();
        }
    }

    pub fn show_help_status(&mut self) {
        if let types::Phase::Ready(app) = &mut self.phase {
            app.status_message = Some(
                "Keys: ↑↓/jk nav  Enter open  ← back  4/s sort  5/c clean  6/d delete  7/r refresh  Esc leave  0 quit"
                    .into(),
            );
            app.status_time = Some(Instant::now());
        }
    }

    pub fn is_exited(&self) -> bool {
        matches!(self.phase, types::Phase::Exited)
    }
}

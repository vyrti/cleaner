//! Deep Clean actions.
//!
//! Follows the same shape as the browser's threaded work: spawn, poll from
//! [`App::tick`], join on completion, and cancel in `Drop`.

use super::state::{DeepPhase, DeepState};
use super::App;
use cleaner_core::sysclean::{self, Candidate, Tier};
use cleaner_core::tree::ScanProgress;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// What a user has to type to confirm a destructive row.
pub const DESTRUCTIVE_WORD: &str = "WIPE";

/// Visible rows, after group collapse and the absent-row filter.
///
/// Returned as indices into `items` so marking stays keyed to the real row.
pub fn visible_rows(state: &DeepState) -> Vec<usize> {
    let mut rows = Vec::with_capacity(state.items.len());
    for (index, candidate) in state.items.iter().enumerate() {
        if !state.show_absent && !candidate.present {
            continue;
        }
        if state.collapsed.contains(candidate.section()) {
            continue;
        }
        rows.push(index);
    }
    rows
}

impl App {
    /// Open Deep Clean and start measuring in the background.
    pub fn open_deep(&mut self) {
        if self.is_busy() || self.deep.is_some() {
            return;
        }

        let home = dirs::home_dir().unwrap_or_else(|| self.root.clone());
        let progress = Arc::new(ScanProgress::new());
        let cancelled = Arc::new(AtomicBool::new(false));

        let handle = {
            let progress = Arc::clone(&progress);
            let cancelled = Arc::clone(&cancelled);
            let home = home.clone();
            thread::spawn(move || {
                let targets = sysclean::catalog(&home);
                let candidates = sysclean::probe(targets, &progress, &cancelled);
                progress.done.store(true, Ordering::Release);
                candidates
            })
        };

        self.deep = Some(DeepState::new(home, progress, cancelled, handle));
        self.clear_status();
    }

    /// Leave Deep Clean, cancelling any measurement in flight.
    pub fn close_deep(&mut self) {
        let Some(mut state) = self.deep.take() else {
            return;
        };
        state.cancelled.store(true, Ordering::Relaxed);
        if let Some(handle) = state.probe_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = state.run_handle.take() {
            let _ = handle.join();
        }
    }

    pub fn deep_move(&mut self, delta: isize) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        let rows = visible_rows(state);
        if rows.is_empty() {
            return;
        }
        let current = rows.iter().position(|i| *i == state.cursor).unwrap_or(0);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            (current + delta as usize).min(rows.len() - 1)
        };
        state.cursor = rows[next];
    }

    pub fn deep_go_top(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        if let Some(first) = visible_rows(state).first() {
            state.cursor = *first;
        }
    }

    pub fn deep_go_bottom(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        if let Some(last) = visible_rows(state).last() {
            state.cursor = *last;
        }
    }

    /// Toggle the row under the cursor.
    ///
    /// Report-only rows cannot be marked - there is nothing this tool will do
    /// with them.
    pub fn deep_toggle(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        let Some(candidate) = state.items.get(state.cursor) else {
            return;
        };
        if !candidate.target.selectable() {
            let label = candidate.target.label.clone();
            self.set_status(format!("{label} is listed only and is never deleted"));
            return;
        }
        if let Some(slot) = state.marked.get_mut(state.cursor) {
            *slot = !*slot;
        }
        state.typed.clear();
    }

    /// Mark every pure-cache row, and nothing else.
    pub fn deep_mark_safe(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        for (index, candidate) in state.items.iter().enumerate() {
            let safe = candidate.target.selectable() && candidate.target.tier == Tier::Safe;
            if let Some(slot) = state.marked.get_mut(index) {
                *slot = safe;
            }
        }
        state.typed.clear();
    }

    pub fn deep_unmark_all(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        state.marked.iter_mut().for_each(|slot| *slot = false);
        state.typed.clear();
    }

    /// Collapse or expand the section the cursor is in.
    pub fn deep_toggle_section(&mut self, collapse: bool) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        let Some(section) = state.items.get(state.cursor).map(|c| c.section()) else {
            return;
        };
        if collapse {
            state.collapsed.insert(section.to_string());
            // The cursor cannot stay on a hidden row.
            if let Some(first) = visible_rows(state).first() {
                state.cursor = *first;
            }
        } else {
            state.collapsed.remove(section);
        }
    }

    pub fn deep_toggle_absent(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        state.show_absent = !state.show_absent;
    }

    /// Move from browsing to confirmation.
    pub fn deep_begin_confirm(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        if state.is_busy() || state.marked_count() == 0 {
            return;
        }
        state.typed.clear();
        state.phase = if state.has_destructive_marked() {
            DeepPhase::Typing
        } else {
            DeepPhase::Confirm
        };
    }

    pub fn deep_cancel_confirm(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        state.typed.clear();
        state.phase = DeepPhase::Ready;
    }

    /// Feed a character into the destructive-row confirmation buffer.
    pub fn deep_type(&mut self, ch: char) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        if state.phase != DeepPhase::Typing {
            return;
        }
        state.typed.push(ch);
        if state.typed.len() > DESTRUCTIVE_WORD.len() {
            state.typed.clear();
        }
    }

    pub fn deep_backspace(&mut self) {
        if let Some(state) = self.deep.as_mut() {
            state.typed.pop();
        }
    }

    /// Start deleting the marked rows.
    pub fn deep_execute(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        if state.is_busy() {
            return;
        }
        if state.phase == DeepPhase::Typing && state.typed != DESTRUCTIVE_WORD {
            return;
        }

        let marked: Vec<_> = state
            .marked
            .iter()
            .zip(&state.items)
            .filter(|(marked, _)| **marked)
            .map(|(_, candidate)| candidate.target.clone())
            .collect();

        if marked.is_empty() {
            state.phase = DeepPhase::Ready;
            return;
        }

        let home = state.home.clone();
        let sink = Arc::clone(&state.errors);
        state.phase = DeepPhase::Running;
        state.typed.clear();
        state.run_handle = Some(thread::spawn(move || {
            sysclean::run(marked, &home, false, sink)
        }));
    }

    /// Poll the Deep Clean worker threads. Called from [`App::tick`].
    pub(crate) fn tick_deep(&mut self) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };

        if let Some(handle) = state.probe_handle.take() {
            if handle.is_finished() {
                let mut items: Vec<Candidate> = handle.join().unwrap_or_default();
                // Group sections into contiguous runs so the list can render a
                // heading whenever the section changes, largest row first
                // within each section.
                items.sort_by(|a, b| {
                    a.section_rank()
                        .cmp(&b.section_rank())
                        .then(b.size.cmp(&a.size))
                });
                state.marked = items.iter().map(|c| c.target.default_marked()).collect();
                state.items = items;
                state.cursor = visible_rows(state).first().copied().unwrap_or(0);
                state.phase = DeepPhase::Ready;
            } else {
                state.probe_handle = Some(handle);
            }
        }

        if let Some(handle) = state.run_handle.take() {
            if handle.is_finished() {
                let report = handle.join().unwrap_or_default();
                let freed = humansize::format_size(report.freed, humansize::BINARY);
                let mut summary = format!("Freed {freed} from {} target(s)", report.done.len());
                if !report.failed.is_empty() {
                    summary.push_str(&format!(", {} failed", report.failed.len()));
                }
                if !report.deferred.is_empty() {
                    summary.push_str(&format!(
                        ", {} need administrator rights",
                        report.deferred.len()
                    ));
                }
                state.pending_elevated = report.deferred;
                state.phase = DeepPhase::Done(summary);
                state.marked.iter_mut().for_each(|slot| *slot = false);
                self.update_disk_usage();
            } else {
                state.run_handle = Some(handle);
            }
        }
    }

    /// Hand the administrator-rights targets to whoever owns the terminal.
    ///
    /// Returns them once and clears them, so a caller that ignores this never
    /// runs anything elevated - it just leaves the rows listed.
    pub fn take_elevated(&mut self) -> Vec<cleaner_core::sysclean::Target> {
        let Some(state) = self.deep.as_mut() else {
            return Vec::new();
        };
        if state.is_busy() {
            return Vec::new();
        }
        std::mem::take(&mut state.pending_elevated)
    }

    /// Record what happened after the caller ran the elevated batch.
    pub fn report_elevated(&mut self, done: usize, failed: Vec<(String, String)>) {
        let Some(state) = self.deep.as_mut() else {
            return;
        };
        let mut summary = format!("Administrator: {done} target(s) completed");
        if !failed.is_empty() {
            summary.push_str(&format!(", {} failed", failed.len()));
            if let Some((label, reason)) = failed.first() {
                summary.push_str(&format!(" ({label}: {reason})"));
            }
        }
        state.phase = DeepPhase::Done(summary);
        self.update_disk_usage();
    }
}

use super::types::{Outcome, Phase};
use super::Session;
use crate::app::{App, DeepPhase};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use std::sync::atomic::Ordering;
use std::time::Instant;

impl Session {
    pub fn handle_event(&mut self, ev: Event) -> Outcome {
        let Event::Key(key) = ev else {
            return Outcome::Continue;
        };
        if key.kind != KeyEventKind::Press {
            return Outcome::Continue;
        }

        match &mut self.phase {
            Phase::Scanning { cancelled, .. } => {
                if matches!(
                    key.code,
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('0')
                ) {
                    cancelled.store(true, Ordering::Relaxed);
                    if let Phase::Scanning { scan_handle, .. } = &mut self.phase {
                        if let Some(handle) = scan_handle.take() {
                            let _ = handle.join();
                        }
                    }
                    self.phase = Phase::Exited;
                    return Outcome::Exit;
                }
                Outcome::Continue
            }
            Phase::Ready(app) => {
                if let Some(outcome) = handle_ready_key(app, key.code) {
                    if outcome == Outcome::Exit {
                        self.phase = Phase::Exited;
                    }
                    return outcome;
                }
                Outcome::Continue
            }
            Phase::Exited => Outcome::Exit,
        }
    }
}

pub(crate) fn handle_ready_key(app: &mut App, code: KeyCode) -> Option<Outcome> {
    // Deep Clean owns the keyboard while it is open, so its checkbox keys never
    // fall through to the browser's delete/clean bindings.
    if app.in_deep() {
        return handle_deep_key(app, code);
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('0') => Some(Outcome::Exit),
        KeyCode::Char('1') => {
            app.status_message = Some(
                "Keys: ↑↓/jk nav  Enter/l open  ←/h back  3/s sort  4 deep clean  5/c clean  6/d delete  7/r refresh  0/q quit"
                    .into(),
            );
            app.status_time = Some(Instant::now());
            Some(Outcome::Continue)
        }
        KeyCode::Char('3') | KeyCode::Char('s') => {
            app.toggle_sort();
            Some(Outcome::Continue)
        }
        KeyCode::Char('4') => {
            app.open_deep();
            Some(Outcome::Continue)
        }
        KeyCode::Char('5') | KeyCode::Char('c') => {
            app.toggle_clean_confirm();
            Some(Outcome::Continue)
        }
        KeyCode::Char('6') | KeyCode::Char('d') => {
            app.toggle_delete_confirm();
            Some(Outcome::Continue)
        }
        KeyCode::Char('7') | KeyCode::Char('r') => {
            app.refresh();
            Some(Outcome::Continue)
        }
        KeyCode::Char('2') | KeyCode::Char('8') | KeyCode::Char('9') => Some(Outcome::Continue),
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_up();
            Some(Outcome::Continue)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_down();
            Some(Outcome::Continue)
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            app.enter();
            Some(Outcome::Continue)
        }
        KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => {
            app.go_back();
            Some(Outcome::Continue)
        }
        KeyCode::Char('y') if app.confirm_delete => {
            app.delete_selected();
            Some(Outcome::Continue)
        }
        KeyCode::Char('y') if app.confirm_clean => {
            app.clean_current();
            Some(Outcome::Continue)
        }
        KeyCode::Char('n') if app.confirm_delete => {
            app.confirm_delete = false;
            Some(Outcome::Continue)
        }
        KeyCode::Char('n') if app.confirm_clean => {
            app.confirm_clean = false;
            Some(Outcome::Continue)
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.go_top();
            Some(Outcome::Continue)
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.go_bottom();
            Some(Outcome::Continue)
        }
        _ => None,
    }
}

/// Key handling inside the Deep Clean view.
///
/// Returns `Some(Outcome::Exit)` for nothing: leaving Deep Clean returns to the
/// browser rather than quitting, so the user cannot lose the tree by pressing
/// escape one time too many.
fn handle_deep_key(app: &mut App, code: KeyCode) -> Option<Outcome> {
    let phase = app.deep.as_ref().map(|state| state.phase.clone())?;

    // The confirmation phases capture almost every key, so they come first.
    match phase {
        DeepPhase::Confirm => {
            return match code {
                KeyCode::Char('y') => {
                    app.deep_execute();
                    Some(Outcome::Continue)
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    app.deep_cancel_confirm();
                    Some(Outcome::Continue)
                }
                _ => Some(Outcome::Continue),
            };
        }
        DeepPhase::Typing => {
            return match code {
                KeyCode::Esc => {
                    app.deep_cancel_confirm();
                    Some(Outcome::Continue)
                }
                KeyCode::Enter => {
                    app.deep_execute();
                    Some(Outcome::Continue)
                }
                KeyCode::Backspace => {
                    app.deep_backspace();
                    Some(Outcome::Continue)
                }
                KeyCode::Char(ch) => {
                    app.deep_type(ch);
                    Some(Outcome::Continue)
                }
                _ => Some(Outcome::Continue),
            };
        }
        DeepPhase::Running | DeepPhase::Probing => {
            // Only leaving is allowed while a worker is running.
            return match code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('0') => {
                    app.close_deep();
                    Some(Outcome::Continue)
                }
                _ => Some(Outcome::Continue),
            };
        }
        DeepPhase::Ready | DeepPhase::Done(_) => {}
    }

    match code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('0') | KeyCode::Char('4') => {
            app.close_deep();
            Some(Outcome::Continue)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.deep_move(-1);
            Some(Outcome::Continue)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.deep_move(1);
            Some(Outcome::Continue)
        }
        KeyCode::PageUp => {
            app.deep_move(-10);
            Some(Outcome::Continue)
        }
        KeyCode::PageDown => {
            app.deep_move(10);
            Some(Outcome::Continue)
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.deep_go_top();
            Some(Outcome::Continue)
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.deep_go_bottom();
            Some(Outcome::Continue)
        }
        KeyCode::Char(' ') => {
            app.deep_toggle();
            Some(Outcome::Continue)
        }
        KeyCode::Char('a') => {
            app.deep_mark_safe();
            Some(Outcome::Continue)
        }
        KeyCode::Char('u') => {
            app.deep_unmark_all();
            Some(Outcome::Continue)
        }
        KeyCode::Left => {
            app.deep_toggle_section(true);
            Some(Outcome::Continue)
        }
        KeyCode::Right => {
            app.deep_toggle_section(false);
            Some(Outcome::Continue)
        }
        KeyCode::Char('h') => {
            app.deep_toggle_absent();
            Some(Outcome::Continue)
        }
        KeyCode::Char('r') => {
            app.close_deep();
            app.open_deep();
            Some(Outcome::Continue)
        }
        KeyCode::Enter => {
            if matches!(phase, DeepPhase::Done(_)) {
                app.close_deep();
                app.open_deep();
            } else {
                app.deep_begin_confirm();
            }
            Some(Outcome::Continue)
        }
        KeyCode::Char('1') => {
            app.set_status(
                "Deep: space toggle  a mark safe  u unmark  ←/→ fold section  h show hidden  r re-measure  enter run  esc back",
            );
            Some(Outcome::Continue)
        }
        _ => Some(Outcome::Continue),
    }
}

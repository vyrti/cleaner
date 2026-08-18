use super::types::{Outcome, Phase};
use super::Session;
use crate::app::App;
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
    match code {
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('0') => Some(Outcome::Exit),
        KeyCode::Char('1') => {
            app.status_message = Some(
                "Keys: ↑↓/jk nav  Enter/l open  ←/h back  4/s sort  5/c clean  6/d delete  7/r refresh  0/q quit"
                    .into(),
            );
            app.status_time = Some(Instant::now());
            Some(Outcome::Continue)
        }
        KeyCode::Char('4') | KeyCode::Char('s') => {
            app.toggle_sort();
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
        KeyCode::Char('2') | KeyCode::Char('3') | KeyCode::Char('8') | KeyCode::Char('9') => {
            Some(Outcome::Continue)
        }
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

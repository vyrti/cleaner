//! Embeddable cleaner TUI session (scan progress + browse/clean).

#[cfg(target_os = "macos")]
use crate::index::{self, IndexState};
use crate::ui::{self, Chrome};
use crate::App;
use cleaner_core::config::Config;
use cleaner_core::patterns::PatternMatcher;
use cleaner_core::tree::{DirTree, ScanProgress};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

/// Options for starting an analyze session.
#[derive(Clone, Debug, Default)]
pub struct StartOpts {
    pub index_enabled: bool,
    pub rebuild_index: bool,
}

/// Result of handling an input event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Continue,
    /// User cancelled the scan or quit the browse UI (`q` / Esc / 0).
    Exit,
}

/// Preview for a host-owned Clean confirmation dialog (Abyss Analyze).
#[derive(Clone, Debug)]
pub enum CleanOffer {
    Unavailable(String),
    Empty { path: PathBuf },
    Ready {
        path: PathBuf,
        dirs: usize,
        files: usize,
        bytes: u64,
    },
}

enum Phase {
    Scanning {
        root: PathBuf,
        progress: Arc<ScanProgress>,
        cancelled: Arc<AtomicBool>,
        scan_handle: Option<JoinHandle<DirTree>>,
        matcher: Arc<PatternMatcher>,
        force: bool,
        #[cfg(target_os = "macos")]
        index_service: Option<index::IndexService>,
        #[cfg(target_os = "macos")]
        cached_event_id: Option<u64>,
        #[cfg(target_os = "macos")]
        starting_event_id: u64,
        #[cfg(target_os = "macos")]
        index_fallback: Option<String>,
    },
    Ready(App),
    Exited,
}

/// Full cleaner TUI session shared by the standalone bin and abyss-tui.
pub struct Session {
    phase: Phase,
}

impl Session {
    pub fn start(root: PathBuf, config: Arc<Config>, opts: StartOpts) -> Self {
        let matcher = Arc::new(PatternMatcher::new(Arc::clone(&config)));
        let force = config.force;

        #[cfg(target_os = "macos")]
        let mut index_fallback = None;
        #[cfg(target_os = "macos")]
        let (index_service, cached_tree, cached_event_id, starting_event_id) = if opts.index_enabled
        {
            match index::IndexService::open(&root, Arc::clone(&config), opts.rebuild_index) {
                Ok((
                    service,
                    index::IndexStartup::Cached {
                        tree,
                        last_event_id,
                    },
                )) => (Some(service), Some(tree), Some(last_event_id), 0),
                Ok((service, index::IndexStartup::Exact { reason: _reason })) => {
                    let event_id = index::IndexService::current_event_id();
                    (Some(service), None, None, event_id)
                }
                Err(error) => {
                    index_fallback = Some(error.to_string());
                    (None, None, None, 0)
                }
            }
        } else {
            (None, None, None, 0)
        };
        #[cfg(not(target_os = "macos"))]
        let cached_tree: Option<DirTree> = None;
        let _ = (opts.index_enabled, opts.rebuild_index);

        if let Some(dir_tree) = cached_tree {
            let mut app = App::new_with_tree(root, matcher, dir_tree, force);
            #[cfg(target_os = "macos")]
            {
                if let Some(error) = index_fallback {
                    app.mark_index_fallback(error);
                }
                if let Some(service) = index_service {
                    if let Some(last_event_id) = cached_event_id {
                        service.start_catchup(app.tree_snapshot(), last_event_id);
                        app.attach_index(service, IndexState::CatchingUp);
                    } else {
                        service.persist_exact(app.tree_snapshot(), starting_event_id);
                        app.attach_index(service, IndexState::Persisting);
                    }
                }
            }
            return Self {
                phase: Phase::Ready(app),
            };
        }

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
            phase: Phase::Scanning {
                root,
                progress,
                cancelled,
                scan_handle,
                matcher,
                force,
                #[cfg(target_os = "macos")]
                index_service,
                #[cfg(target_os = "macos")]
                cached_event_id,
                #[cfg(target_os = "macos")]
                starting_event_id,
                #[cfg(target_os = "macos")]
                index_fallback,
            },
        }
    }

    pub fn tick(&mut self) {
        match &mut self.phase {
            Phase::Scanning {
                progress,
                scan_handle,
                root,
                matcher,
                force,
                #[cfg(target_os = "macos")]
                index_service,
                #[cfg(target_os = "macos")]
                cached_event_id,
                #[cfg(target_os = "macos")]
                starting_event_id,
                #[cfg(target_os = "macos")]
                index_fallback,
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
                        self.phase = Phase::Exited;
                        return;
                    }
                };
                let mut app =
                    App::new_with_tree(root.clone(), Arc::clone(matcher), dir_tree, *force);
                #[cfg(target_os = "macos")]
                {
                    if let Some(error) = index_fallback.take() {
                        app.mark_index_fallback(error);
                    }
                    if let Some(service) = index_service.take() {
                        if let Some(last_event_id) = *cached_event_id {
                            service.start_catchup(app.tree_snapshot(), last_event_id);
                            app.attach_index(service, IndexState::CatchingUp);
                        } else {
                            service.persist_exact(app.tree_snapshot(), *starting_event_id);
                            app.attach_index(service, IndexState::Persisting);
                        }
                    }
                }
                self.phase = Phase::Ready(app);
            }
            Phase::Ready(app) => app.tick(),
            Phase::Exited => {}
        }
    }

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

    /// Draw with full standalone chrome (digit bar) or content-only for Abyss embed.
    pub fn draw(&self, frame: &mut Frame, area: Rect, chrome: Chrome) {
        match &self.phase {
            Phase::Scanning {
                root, progress, ..
            } => ui::draw_scan_progress(frame, area, root, progress),
            Phase::Ready(app) => ui::render_in(frame, app, area, chrome),
            Phase::Exited => {}
        }
    }

    pub fn status_line(&self) -> Option<String> {
        match &self.phase {
            Phase::Scanning {
                root, progress, ..
            } => {
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
            Phase::Ready(app) => ui::status_line(app),
            Phase::Exited => None,
        }
    }

    pub fn refresh(&mut self) {
        if let Phase::Ready(app) = &mut self.phase {
            app.refresh();
        }
    }

    pub fn toggle_sort(&mut self) {
        if let Phase::Ready(app) = &mut self.phase {
            app.toggle_sort();
        }
    }

    pub fn toggle_clean_confirm(&mut self) {
        if let Phase::Ready(app) = &mut self.phase {
            app.toggle_clean_confirm();
        }
    }

    /// Preview for an Abyss-hosted Clean confirmation dialog.
    pub fn clean_offer(&self) -> CleanOffer {
        let Phase::Ready(app) = &self.phase else {
            return CleanOffer::Unavailable("Analyze is still scanning".into());
        };
        if app.is_busy() {
            return CleanOffer::Unavailable("Cleaner is busy".into());
        }
        if !app.actions_enabled {
            return CleanOffer::Unavailable("Index is syncing; cleaning is disabled".into());
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
        if let Phase::Ready(app) = &mut self.phase {
            app.clean_current();
        }
    }

    pub fn toggle_delete_confirm(&mut self) {
        if let Phase::Ready(app) = &mut self.phase {
            app.toggle_delete_confirm();
        }
    }

    pub fn show_help_status(&mut self) {
        if let Phase::Ready(app) = &mut self.phase {
            app.status_message = Some(
                "Keys: ↑↓/jk nav  Enter open  ← back  4/s sort  5/c clean  6/d delete  7/r refresh  Esc leave  0 quit"
                    .into(),
            );
            app.status_time = Some(Instant::now());
        }
    }

    pub fn is_exited(&self) -> bool {
        matches!(self.phase, Phase::Exited)
    }
}

fn handle_ready_key(app: &mut App, code: KeyCode) -> Option<Outcome> {
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

/// Run the interactive TUI as a standalone app (owns the terminal).
pub fn run(
    root: PathBuf,
    config: Arc<Config>,
    index_enabled: bool,
    rebuild_index: bool,
) -> std::io::Result<()> {
    use crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use std::io::{self, Write};
    use std::time::Duration;

    fn cleanup_terminal() {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        let _ = io::stdout().flush();
    }

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cleanup_terminal();
        original_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut session = Session::start(
        root,
        config,
        StartOpts {
            index_enabled,
            rebuild_index,
        },
    );

    let result = (|| -> io::Result<()> {
        loop {
            session.tick();
            if session.is_exited() {
                return Ok(());
            }
            terminal.draw(|f| {
                let area = f.area();
                session.draw(f, area, Chrome::Full);
            })?;

            if event::poll(Duration::from_millis(100))? {
                let ev = event::read()?;
                if session.handle_event(ev) == Outcome::Exit {
                    return Ok(());
                }
            }
        }
    })();

    cleanup_terminal();
    result
}

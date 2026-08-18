use super::types::{Outcome, StartOpts};
use super::Session;
use crate::ui::Chrome;
use cleaner_core::config::Config;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Run the interactive TUI as a standalone app (owns the terminal).
pub fn run(
    root: PathBuf,
    config: Arc<Config>,
    index_enabled: bool,
    rebuild_index: bool,
) -> std::io::Result<()> {
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

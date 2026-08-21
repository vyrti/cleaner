use super::types::{Outcome, StartOpts};
use super::Session;
use crate::ui::Chrome;
use cleaner_core::config::Config;
use cleaner_core::sysclean::{elevate, Target};
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

            // Administrator work cannot run from a worker thread: it needs the
            // terminal for a password or UAC prompt. This loop owns the
            // terminal, so it is the only place that can hand it over.
            let elevated = session.take_elevated();
            if !elevated.is_empty() {
                let (done, failed) = run_elevated(&mut terminal, &elevated)?;
                session.report_elevated(done, failed);
                continue;
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

/// Drop out of the TUI, run the elevated batch with inherited stdio, and come
/// back.
///
/// The screen has to be fully released first, otherwise the password prompt is
/// swallowed by the alternate screen and the user sees a frozen UI.
fn run_elevated(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    targets: &[Target],
) -> io::Result<(usize, Vec<(String, String)>)> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;

    println!("\ncleaner needs administrator rights for the following:\n");
    for line in elevate::preview(targets) {
        println!("  {line}");
    }
    println!();
    let _ = io::stdout().flush();

    let report = elevate::run_elevated(targets);

    if let Some(script) = &report.script {
        println!("\nScript written to {}", script.display());
    }
    let _ = io::stdout().flush();

    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    terminal.clear()?;

    Ok((report.done.len(), report.failed))
}

//! Interactive TUI module for ncdu-like disk usage browser

mod app;
mod events;
mod tree;
mod ui;

pub use app::App;

use crate::config::Config;
use crate::patterns::PatternMatcher;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tree::ScanProgress;

/// Cleanup terminal on panic or exit
fn cleanup_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = io::stdout().flush();
}

/// Run the interactive TUI
pub fn run(root: PathBuf, config: Arc<Config>) -> io::Result<()> {
    // Set panic hook to cleanup terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cleanup_terminal();
        original_hook(info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create matcher
    let matcher = Arc::new(PatternMatcher::new(Arc::clone(&config)));

    // Create progress tracker with cancel flag
    let progress = Arc::new(ScanProgress::new());
    let cancelled = Arc::new(AtomicBool::new(false));
    let progress_clone = Arc::clone(&progress);
    let cancelled_clone = Arc::clone(&cancelled);

    // Start scan in background thread
    let root_clone = root.clone();
    let matcher_clone = Arc::clone(&matcher);
    let scan_handle = thread::spawn(move || {
        tree::DirTree::build_with_progress(&root_clone, &matcher_clone, progress_clone, cancelled_clone)
    });

    // Show live progress while scanning with quit support
    let mut user_quit = false;
    while !progress.is_done() && !user_quit {
        terminal.draw(|f| {
            let area = f.area();
            let files = progress.get_files();
            let dirs = progress.get_dirs();
            let bytes = progress.get_bytes();
            let size_str = humansize::format_size(bytes, humansize::BINARY);
            let phase = progress.get_phase();

            let text = format!(
                "\n\n  {} {}...\n\n  📁 {} folders\n  📄 {} files\n  💾 {}\n\n  Press 'q' to cancel",
                if phase == 0 { "⏳ Scanning" } else { "🔄 Building tree from" },
                root.display(),
                dirs,
                files,
                size_str
            );

            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Cleaner - Scanning ");
            let paragraph = Paragraph::new(text).block(block);
            f.render_widget(paragraph, area);
        })?;

        // Non-blocking key check for quit
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                        user_quit = true;
                        cancelled.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    // Cleanup if user quit during scan
    if user_quit {
        cleanup_terminal();
        println!("Scan cancelled.");
        return Ok(());
    }

    // Get the completed tree
    let dir_tree = match scan_handle.join() {
        Ok(tree) => tree,
        Err(_) => {
            cleanup_terminal();
            eprintln!("Scan thread panicked");
            return Ok(());
        }
    };

    // Create app with pre-built tree
    let mut app = App::new_with_tree(root, matcher, dir_tree);

    // Main loop
    let result = run_app(&mut terminal, &mut app);

    // Restore terminal
    cleanup_terminal();

    result
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                    KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app.enter(),
                    KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => app.go_back(),
                    KeyCode::Char('d') => app.toggle_delete_confirm(),
                    KeyCode::Char('y') if app.confirm_delete => app.delete_selected(),
                    KeyCode::Char('n') if app.confirm_delete => app.confirm_delete = false,
                    KeyCode::Char('s') => app.toggle_sort(),
                    KeyCode::Char('r') => app.refresh(),
                    KeyCode::Home | KeyCode::Char('g') => app.go_top(),
                    KeyCode::End | KeyCode::Char('G') => app.go_bottom(),
                    _ => {}
                }
            }
        }
    }
}

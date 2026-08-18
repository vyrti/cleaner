use super::*;
use crate::app::App;
use cleaner_core::config::Config;
use cleaner_core::patterns::PatternMatcher;
use cleaner_core::tree::{DirEntry, DirTree};
use foldhash::{HashMap, HashMapExt};
use ratatui::{backend::TestBackend, Terminal};
use std::path::PathBuf;
use std::sync::Arc;

fn app() -> App {
    let root = PathBuf::from("test-root");
    let matcher = Arc::new(PatternMatcher::new(Arc::new(Config {
        directories: vec!["target".into()],
        files: vec![".pyc".into()],
        days: None,
        force: false,
    })));
    let mut children = HashMap::new();
    children.insert(
        root.clone(),
        vec![
            DirEntry {
                name: "target".into(),
                size: 4096,
                is_dir: true,
                is_temp: true,
            },
            DirEntry {
                name: "main.rs".into(),
                size: 20,
                is_dir: false,
                is_temp: false,
            },
        ],
    );
    App::new_with_tree(root, matcher, DirTree::from_children(children), false)
}

fn screen(app: &App) -> String {
    let backend = TestBackend::new(100, 14);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn renders_header_entries_and_digit_bar() {
    let output = screen(&app());
    assert!(output.contains("test-root"));
    assert!(output.contains("Folder:"));
    assert!(output.contains("Sort: size"));
    assert!(output.contains("Name"));
    assert!(output.contains("target"));
    assert!(output.contains("[TEMP]"));
    assert!(output.contains("Clean"));
    assert!(output.contains("Delete"));
    assert!(output.contains("Quit"));
}

#[test]
fn renders_delete_clean_and_status() {
    let mut app = app();
    app.confirm_delete = true;
    assert!(screen(&app).contains("Delete 'target'?"));
    app.confirm_delete = false;
    app.confirm_clean = true;
    assert!(screen(&app).contains("Clean all temp"));
    app.confirm_clean = false;
    app.status_message = Some("Refreshed".into());
    assert!(screen(&app).contains("Refreshed"));
}

#[test]
fn content_only_omits_digit_labels() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = app();
    terminal
        .draw(|frame| render_in(frame, &app, frame.area(), Chrome::ContentOnly))
        .unwrap();
    let output: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(output.contains("target"));
    assert!(output.contains("test-root"));
    assert!(output.contains("Folder:"));
    assert!(!output.contains("0Quit") && !output.contains("0 Exit"));
}

#[test]
fn renders_disk_information_and_name_sort() {
    let mut app = app();
    app.sort_mode = crate::app::SortMode::Name;
    app.disk_total = 1000;
    app.disk_free = 250;
    let output = screen(&app);
    assert!(output.contains("Sort: name"));
    assert!(output.contains("25.0%"));
}

#[test]
fn renders_only_rows_visible_near_selection() {
    let mut app = app();
    app.entries = Arc::new(
        (0..50)
            .map(|index| DirEntry {
                name: format!("entry-{index:02}").into(),
                size: index,
                is_dir: false,
                is_temp: false,
            })
            .collect(),
    );
    app.selected = 49;
    let output = screen(&app);
    assert!(output.contains("entry-49"));
    assert!(!output.contains("entry-00"));
}

#[test]
fn status_line_and_scan_progress_rendering() {
    let mut app = app();
    assert!(status_line(&app).is_none());

    app.delete_selected();
    let status = status_line(&app);
    assert!(status.is_some());

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let progress = cleaner_core::tree::ScanProgress::new();
    terminal
        .draw(|f| draw_scan_progress(f, f.area(), std::path::Path::new("/test"), &progress))
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("Scanning"));
}

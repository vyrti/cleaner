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

/// Build a Deep Clean state directly so rendering can be tested without
/// touching the real filesystem.
fn deep_app() -> App {
    use crate::app::{DeepPhase, DeepState};
    use cleaner_core::sysclean::{Action, Candidate, Group, Target, Tier};
    use cleaner_core::tree::ScanProgress;
    use std::sync::atomic::AtomicBool;

    fn candidate(id: &str, label: &str, group: Group, tier: Tier, size: u64) -> Candidate {
        Candidate {
            target: Target {
                id: id.into(),
                group,
                label: label.into(),
                detail: "regenerates on demand".into(),
                tier,
                action: Action::Remove(vec![PathBuf::from("/tmp/cleaner-ui-test")]),
                probe: vec![],
                requires: None,
            },
            size,
            present: true,
        }
    }

    let mut app = app();
    let items = vec![
        candidate(
            "a",
            "Xcode DerivedData",
            Group::DevCache,
            Tier::Safe,
            3_865_470_566,
        ),
        candidate(
            "b",
            "Docker · WIPE disk image",
            Group::Container,
            Tier::Destructive,
            49_392_123_904,
        ),
        candidate(
            "c",
            "macOS Install Data",
            Group::SystemJunk,
            Tier::NeedsRoot,
            3_328_599_654,
        ),
    ];
    let marked = items.iter().map(|c| c.target.default_marked()).collect();

    let handle = std::thread::spawn(Vec::new);
    let mut state = DeepState::new(
        PathBuf::from("/tmp"),
        Arc::new(ScanProgress::new()),
        Arc::new(AtomicBool::new(false)),
        handle,
    );
    let _ = state.probe_handle.take().map(|h| h.join());
    state.items = items;
    state.marked = marked;
    state.phase = DeepPhase::Ready;
    app.deep = Some(state);
    app
}

#[test]
fn footer_shows_sort_on_three_and_deep_on_four() {
    let output = screen(&app());
    assert!(
        output.contains("3Sort"),
        "footer should bind Sort to 3: {output}"
    );
    assert!(
        output.contains("4Deep"),
        "footer should bind Deep to 4: {output}"
    );
    assert!(output.contains("5Clean"), "Clean should stay on 5");
    assert!(output.contains("0Quit"), "Quit should stay on 0");
}

#[test]
fn deep_view_renders_sections_checkboxes_and_sizes() {
    let output = screen(&deep_app());

    assert!(output.contains("Deep Clean"), "missing title: {output}");
    assert!(
        output.contains("[x]"),
        "safe rows should be pre-marked: {output}"
    );
    assert!(
        output.contains("[ ]"),
        "unsafe rows should be unmarked: {output}"
    );
    assert!(
        output.contains("Dev caches"),
        "missing group heading: {output}"
    );
    assert!(
        output.contains("Needs admin"),
        "admin rows need their own section: {output}"
    );
    assert!(output.contains("GiB"), "sizes should be rendered: {output}");
}

/// The destructive row must never come pre-ticked.
#[test]
fn deep_view_does_not_premark_destructive_rows() {
    let app = deep_app();
    let state = app.deep.as_ref().unwrap();

    for (marked, candidate) in state.marked.iter().zip(&state.items) {
        if candidate.target.tier != cleaner_core::sysclean::Tier::Safe {
            assert!(
                !marked,
                "{} should not be marked by default",
                candidate.target.id
            );
        }
    }
    assert_eq!(
        state.marked_count(),
        1,
        "only the safe row should be marked"
    );
}

#[test]
fn deep_view_replaces_the_browser_list() {
    let browser = screen(&app());
    let deep = screen(&deep_app());

    assert!(browser.contains("main.rs"), "browser should list files");
    assert!(
        !deep.contains("main.rs"),
        "Deep Clean should take over the content area: {deep}"
    );
}

/// Print the Deep Clean view so the layout can be eyeballed.
///
/// Ignored by default; run with
/// `cargo test -p cleaner-tui -- --ignored --nocapture deep_view_layout`.
#[test]
#[ignore = "manual: prints the rendered view"]
fn deep_view_layout_preview() {
    let app = deep_app();
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        println!("|{row}|");
    }
}

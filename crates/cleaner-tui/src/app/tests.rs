use super::state::{DeleteState, SortMode};
use super::App;
use cleaner_core::config::Config;
use cleaner_core::patterns::PatternMatcher;
use cleaner_core::test_support::TempDir;
use cleaner_core::tree::{DirEntry, DirTree};
use foldhash::{HashMap, HashMapExt};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn matcher() -> Arc<PatternMatcher> {
    Arc::new(PatternMatcher::new(Arc::new(Config {
        directories: vec!["target".into()],
        files: vec![".pyc".into()],
        days: None,
        force: false,
    })))
}

fn entry(_path: PathBuf, name: &str, size: u64, is_dir: bool, is_temp: bool) -> DirEntry {
    DirEntry {
        name: name.into(),
        size,
        is_dir,
        is_temp,
    }
}

fn app_with_tree(temp: &TempDir) -> App {
    let root = temp.path().to_path_buf();
    let folder = temp.join("folder");
    let mut children = HashMap::new();
    children.insert(
        root.clone(),
        vec![
            entry(folder.clone(), "folder", 8, true, false),
            entry(temp.join("cache.pyc"), "cache.pyc", 3, false, true),
        ],
    );
    children.insert(
        folder.clone(),
        vec![
            entry(root.clone(), "..", 0, true, false),
            entry(folder.join("nested.pyc"), "nested.pyc", 8, false, true),
        ],
    );
    App::new_with_tree(root, matcher(), DirTree::from_children(children), false)
}

fn select(app: &mut App, name: &str) {
    app.selected = app.entries.iter().position(|e| e.name == name).unwrap();
}

fn wait_until_idle(app: &mut App) {
    for _ in 0..200 {
        app.tick();
        if !app.is_busy() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("background operation did not finish");
}

#[test]
fn navigation_selection_sorting_and_confirmations_work() {
    let temp = TempDir::new("app-navigation");
    let mut app = app_with_tree(&temp);
    assert_eq!(app.total_size, 11);
    assert_eq!(app.selected, 0);
    assert!(app.selected_entry().is_some());

    app.move_down();
    assert_eq!(app.selected, 1);
    app.move_down();
    assert_eq!(app.selected, 1);
    app.move_up();
    app.go_bottom();
    assert_eq!(app.selected, 1);
    app.go_top();
    assert_eq!(app.selected, 0);

    app.toggle_sort();
    assert_eq!(app.sort_mode, SortMode::Name);
    app.toggle_sort();
    assert_eq!(app.sort_mode, SortMode::Size);

    select(&mut app, "cache.pyc");
    app.toggle_delete_confirm();
    assert!(app.confirm_delete);
    app.toggle_clean_confirm();
    assert!(app.confirm_clean);
    assert!(app.clean_preview.is_some());
    assert!(!app.confirm_delete);
    assert_eq!(app.current_temp_stats(), (0, 2, 11));
}

#[test]
fn entering_and_leaving_directory_restores_selection() {
    let temp = TempDir::new("app-enter");
    let mut app = app_with_tree(&temp);
    select(&mut app, "folder");
    app.enter();
    assert_eq!(app.current_path, temp.join("folder"));
    assert_eq!(app.path_stack, [temp.path().to_path_buf()]);
    select(&mut app, "..");
    app.enter();
    assert_eq!(app.current_path, temp.path());
    assert_eq!(app.selected_entry().unwrap().name, "folder");
    app.go_back(); // Already at root: no-op.
    assert_eq!(app.current_path, temp.path());
}

#[test]
fn deleting_selected_file_updates_disk_and_in_memory_tree() {
    let temp = TempDir::new("app-delete");
    temp.mkdir("folder");
    let file = temp.write("cache.pyc", b"123");
    let mut app = app_with_tree(&temp);
    select(&mut app, "cache.pyc");
    app.delete_selected();
    assert!(!app.is_busy());
    assert!(!app.is_deleting());
    assert!(!file.exists());
    assert!(!app.entries.iter().any(|e| e.name == "cache.pyc"));
    assert!(app
        .status_message
        .as_deref()
        .unwrap()
        .starts_with("Deleted:"));
}

#[test]
fn deleting_selected_file_preserves_cursor_position() {
    let temp = TempDir::new("app-delete-cursor");
    let _file1 = temp.write("a.txt", b"111");
    let file2 = temp.write("b.txt", b"11");
    let _file3 = temp.write("c.txt", b"1");
    let root = temp.path().to_path_buf();
    let mut children = HashMap::new();
    children.insert(
        root.clone(),
        vec![
            entry(temp.join("a.txt"), "a.txt", 3, false, false),
            entry(temp.join("b.txt"), "b.txt", 2, false, false),
            entry(temp.join("c.txt"), "c.txt", 1, false, false),
        ],
    );
    let mut app = App::new_with_tree(root, matcher(), DirTree::from_children(children), false);

    // Since we sort by size descending:
    // Index 0: a.txt (size 3)
    // Index 1: b.txt (size 2)
    // Index 2: c.txt (size 1)

    select(&mut app, "b.txt");
    assert_eq!(app.selected, 1);

    app.delete_selected();

    // b.txt is deleted
    assert!(!file2.exists());
    // Cursor index should stay at 1, which is now c.txt (size 1)
    assert_eq!(app.selected, 1);
    assert_eq!(app.entries[app.selected].name, "c.txt");
}

#[test]
fn manual_deletion_revalidates_missing_type_changes_and_symlinks() {
    let temp = TempDir::new("app-delete-revalidate");
    temp.mkdir("folder");
    let file = temp.write("cache.pyc", b"123");
    let mut app = app_with_tree(&temp);
    select(&mut app, "cache.pyc");
    fs::remove_file(&file).unwrap();
    app.delete_selected();
    assert!(app
        .status_message
        .as_deref()
        .unwrap()
        .starts_with("Delete rejected:"));

    temp.mkdir("cache.pyc");
    let mut app = app_with_tree(&temp);
    select(&mut app, "cache.pyc");
    app.delete_selected();
    assert_eq!(
        app.status_message.as_deref(),
        Some("Delete rejected: path type changed since scan")
    );
    assert!(temp.join("cache.pyc").is_dir());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new("app-delete-symlink");
        temp.mkdir("folder");
        temp.write("target", b"keep");
        symlink(temp.join("target"), temp.join("cache.pyc")).unwrap();
        let mut app = app_with_tree(&temp);
        select(&mut app, "cache.pyc");
        app.delete_selected();
        assert_eq!(
            app.status_message.as_deref(),
            Some("Delete rejected: path type changed since scan")
        );
        assert!(temp.join("target").exists());
    }
}

#[test]
fn directory_deletion_helper_and_error_status_are_covered() {
    let temp = TempDir::new("app-remove-dir");
    let directory = temp.mkdir("remove-me/nested");
    assert!(App::remove_dir_fast(temp.join("remove-me")).is_ok());
    assert!(!directory.exists());
    assert!(App::remove_dir_fast(temp.join("missing")).is_err());

    let mut app = app_with_tree(&temp);
    app.delete_state = Some(DeleteState {
        handle: thread::spawn(|| Err("expected failure".into())),
        entry_name: "bad".into(),
        entry_path: temp.join("bad"),
        is_dir: false,
        entry_size: 0,
    });
    wait_until_idle(&mut app);
    assert_eq!(
        app.status_message.as_deref(),
        Some("Error: expected failure")
    );
}

#[test]
fn clean_current_removes_default_patterns_and_rebuilds() {
    let temp = TempDir::new("app-clean");
    temp.write("target/artifact", b"12345");
    temp.write("keep/source.rs", b"keep");
    let mut app = App::new(temp.path().to_path_buf(), matcher(), false);
    app.scan_current_dir();
    assert!(!app.entries.is_empty());
    app.toggle_clean_confirm();
    app.clean_current();
    assert!(app.is_cleaning());
    wait_until_idle(&mut app);
    assert!(!temp.join("target").exists());
    assert!(temp.join("keep/source.rs").exists());
    assert!(app
        .status_message
        .as_deref()
        .unwrap()
        .starts_with("Cleaned:"));

    app.refresh();
    wait_until_idle(&mut app);
    assert_eq!(app.status_message.as_deref(), Some("Refreshed"));
}

#[test]
fn tick_expires_old_status_and_disk_usage_handles_valid_path() {
    let temp = TempDir::new("app-status");
    let mut app = app_with_tree(&temp);
    app.set_status("old");
    app.status_time = Some(Instant::now() - Duration::from_secs(11));
    app.tick();
    assert!(app.status_message.is_none());
    app.update_disk_usage();
    #[cfg(any(unix, windows))]
    assert!(app.disk_total > 0);
    app.set_status("test");
    app.clear_status();
    assert!(app.status_message.is_none());
}

#[test]
fn rebuild_state_progress_and_cancellation() {
    let temp = TempDir::new("app-rebuild");
    let mut app = app_with_tree(&temp);
    assert!(!app.is_rebuilding());
    assert!(app.rebuild_progress().is_none());

    app.start_rebuild("done".into());
    assert!(app.is_rebuilding());
    assert!(app.rebuild_progress().is_some());

    app.cancel_rebuild();
    assert!(!app.is_rebuilding());
    assert_eq!(app.status_message.as_deref(), Some("Rebuild cancelled"));
}

/// Build a Deep Clean state with known rows, bypassing the real catalog.
fn deep_state_with(
    home: &std::path::Path,
    items: Vec<cleaner_core::sysclean::Candidate>,
) -> crate::app::DeepState {
    use crate::app::DeepPhase;
    use cleaner_core::tree::ScanProgress;
    use std::sync::atomic::AtomicBool;

    let handle = thread::spawn(Vec::new);
    let mut state = crate::app::DeepState::new(
        home.to_path_buf(),
        Arc::new(ScanProgress::new()),
        Arc::new(AtomicBool::new(false)),
        handle,
    );
    let _ = state.probe_handle.take().map(|h| h.join());
    state.marked = items.iter().map(|c| c.target.default_marked()).collect();
    state.items = items;
    state.phase = DeepPhase::Ready;
    state
}

fn candidate(
    id: &str,
    tier: cleaner_core::sysclean::Tier,
    action: cleaner_core::sysclean::Action,
) -> cleaner_core::sysclean::Candidate {
    use cleaner_core::sysclean::{Candidate, Group, Target};
    Candidate {
        target: Target {
            id: id.into(),
            group: Group::DevCache,
            label: id.into(),
            detail: "d".into(),
            tier,
            action,
            probe: vec![],
            requires: None,
        },
        size: 1024,
        present: true,
    }
}

#[test]
fn deep_marking_selects_only_safe_rows_and_survives_navigation() {
    use cleaner_core::sysclean::{Action, Tier};

    let temp = TempDir::new("deep-marking");
    let mut app = app_with_tree(&temp);

    let items = vec![
        candidate("safe", Tier::Safe, Action::Remove(vec![temp.join("a")])),
        candidate(
            "risky",
            Tier::Reclaimable,
            Action::Remove(vec![temp.join("b")]),
        ),
        candidate(
            "nuke",
            Tier::Destructive,
            Action::Remove(vec![temp.join("c")]),
        ),
    ];
    app.deep = Some(deep_state_with(temp.path(), items));

    // Only the pure cache is pre-selected.
    assert_eq!(app.deep.as_ref().unwrap().marked_count(), 1);

    app.deep_unmark_all();
    assert_eq!(app.deep.as_ref().unwrap().marked_count(), 0);

    app.deep_mark_safe();
    assert_eq!(app.deep.as_ref().unwrap().marked_count(), 1);

    // Toggling the row under the cursor flips just that row.
    app.deep_go_top();
    app.deep_toggle();
    assert_eq!(app.deep.as_ref().unwrap().marked_count(), 0);
    app.deep_toggle();
    assert_eq!(app.deep.as_ref().unwrap().marked_count(), 1);

    // Unlike the browser list, moving the cursor must not clear the marks.
    app.deep_move(1);
    app.deep_move(1);
    app.deep_move(-1);
    assert_eq!(
        app.deep.as_ref().unwrap().marked_count(),
        1,
        "navigation must not reset marks"
    );
}

#[test]
fn deep_report_only_rows_cannot_be_marked() {
    use cleaner_core::sysclean::{Action, Tier};

    let temp = TempDir::new("deep-report-only");
    let mut app = app_with_tree(&temp);
    app.deep = Some(deep_state_with(
        temp.path(),
        vec![candidate("vm", Tier::Reclaimable, Action::ReportOnly)],
    ));

    app.deep_go_top();
    app.deep_toggle();

    assert_eq!(
        app.deep.as_ref().unwrap().marked_count(),
        0,
        "report-only rows must never become marked"
    );
}

/// A marked destructive row forces the typed-confirmation phase instead of the
/// plain y/n prompt.
#[test]
fn deep_destructive_rows_require_typing_the_word() {
    use crate::app::{DeepPhase, DESTRUCTIVE_WORD};
    use cleaner_core::sysclean::{Action, Tier};

    let temp = TempDir::new("deep-destructive");
    temp.write("nuke/file.bin", &[1u8; 512]);
    let mut app = app_with_tree(&temp);
    app.deep = Some(deep_state_with(
        temp.path(),
        vec![candidate(
            "nuke",
            Tier::Destructive,
            Action::Remove(vec![temp.join("nuke")]),
        )],
    ));

    app.deep_go_top();
    app.deep_toggle();
    app.deep_begin_confirm();
    assert_eq!(app.deep.as_ref().unwrap().phase, DeepPhase::Typing);

    // The wrong word does nothing at all.
    for ch in "nope".chars() {
        app.deep_type(ch);
    }
    app.deep_execute();
    assert_eq!(
        app.deep.as_ref().unwrap().phase,
        DeepPhase::Typing,
        "a wrong confirmation word must not start the run"
    );
    assert!(temp.join("nuke/file.bin").exists());

    // The right one starts it.
    app.deep_unmark_all();
    app.deep_toggle();
    app.deep_begin_confirm();
    for ch in DESTRUCTIVE_WORD.chars() {
        app.deep_type(ch);
    }
    app.deep_execute();
    wait_until_idle(&mut app);
    assert!(matches!(
        app.deep.as_ref().unwrap().phase,
        DeepPhase::Done(_)
    ));
}

#[test]
fn deep_safe_rows_run_after_a_plain_confirmation() {
    use crate::app::DeepPhase;
    use cleaner_core::sysclean::{Action, Tier};

    let temp = TempDir::new("deep-run-safe");
    temp.write("cache/file.bin", &[1u8; 4096]);
    let mut app = app_with_tree(&temp);
    app.deep = Some(deep_state_with(
        temp.path(),
        vec![candidate(
            "cache",
            Tier::Safe,
            Action::Remove(vec![temp.join("cache")]),
        )],
    ));

    app.deep_begin_confirm();
    assert_eq!(app.deep.as_ref().unwrap().phase, DeepPhase::Confirm);

    app.deep_execute();
    wait_until_idle(&mut app);

    assert!(matches!(
        app.deep.as_ref().unwrap().phase,
        DeepPhase::Done(_)
    ));
    assert!(
        !temp.join("cache").exists(),
        "the marked row should have run"
    );
}

#[test]
fn deep_hides_absent_rows_until_asked() {
    use cleaner_core::sysclean::{Action, Tier};

    let temp = TempDir::new("deep-absent");
    let mut app = app_with_tree(&temp);

    let mut present = candidate("present", Tier::Safe, Action::Remove(vec![temp.join("a")]));
    present.present = true;
    let mut absent = candidate("absent", Tier::Safe, Action::Remove(vec![temp.join("b")]));
    absent.present = false;
    app.deep = Some(deep_state_with(temp.path(), vec![present, absent]));

    assert_eq!(
        crate::app::visible_rows(app.deep.as_ref().unwrap()).len(),
        1
    );
    app.deep_toggle_absent();
    assert_eq!(
        crate::app::visible_rows(app.deep.as_ref().unwrap()).len(),
        2
    );
}

#[test]
fn deep_collapsing_a_section_hides_its_rows_but_keeps_marks() {
    use cleaner_core::sysclean::{Action, Tier};

    let temp = TempDir::new("deep-collapse");
    let mut app = app_with_tree(&temp);
    app.deep = Some(deep_state_with(
        temp.path(),
        vec![
            candidate("one", Tier::Safe, Action::Remove(vec![temp.join("a")])),
            candidate("two", Tier::Safe, Action::Remove(vec![temp.join("b")])),
        ],
    ));

    let before = app.deep.as_ref().unwrap().marked_count();
    app.deep_go_top();
    app.deep_toggle_section(true);

    assert!(crate::app::visible_rows(app.deep.as_ref().unwrap()).is_empty());
    assert_eq!(
        app.deep.as_ref().unwrap().marked_count(),
        before,
        "collapsing must not change what is marked"
    );

    app.deep_toggle_section(false);
    assert_eq!(
        crate::app::visible_rows(app.deep.as_ref().unwrap()).len(),
        2
    );
}

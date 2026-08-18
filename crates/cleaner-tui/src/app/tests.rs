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

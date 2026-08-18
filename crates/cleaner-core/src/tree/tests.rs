use super::entry::DirEntry;
use super::progress::ScanProgress;
use super::sizing::apply_directory_sizes;
use super::sort::{sort_by_name, sort_by_size};
use super::DirTree;
use crate::config::Config;
use crate::patterns::PatternMatcher;
use crate::test_support::TempDir;
use foldhash::{HashMap, HashMapExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn entry(_path: PathBuf, name: &str, size: u64, is_dir: bool, is_temp: bool) -> DirEntry {
    DirEntry {
        name: name.into(),
        size,
        is_dir,
        is_temp,
    }
}

#[test]
fn dir_entry_new_constructs_entry() {
    let e = DirEntry::new("test", 100, true, false);
    assert_eq!(e.name, "test");
    assert_eq!(e.size, 100);
    assert!(e.is_dir);
    assert!(!e.is_temp);
}

fn matcher() -> PatternMatcher {
    PatternMatcher::new(Arc::new(Config {
        directories: vec!["target".into()],
        files: vec![".pyc".into()],
        days: None,
        force: false,
    }))
}

#[test]
fn progress_accessors_reflect_atomic_state() {
    let progress = ScanProgress::new();
    progress.files.store(2, Ordering::Relaxed);
    progress.dirs.store(3, Ordering::Relaxed);
    progress.bytes.store(40, Ordering::Relaxed);
    progress.phase.store(1, Ordering::Relaxed);
    progress.done.store(true, Ordering::Relaxed);
    assert_eq!(
        (
            progress.get_files(),
            progress.get_dirs(),
            progress.get_bytes()
        ),
        (2, 3, 40)
    );
    assert_eq!(progress.get_phase(), 1);
    assert!(progress.is_done());
    progress.begin_stage(3, 12);
    progress.stage_current.store(5, Ordering::Relaxed);
    assert_eq!(progress.get_phase(), 3);
    assert_eq!(progress.get_stage_progress(), (5, 12));
}

#[test]
fn in_place_sizing_updates_nested_entries_and_progress() {
    let root = PathBuf::from("/sizing-root");
    let child = root.join("child");
    let nested = child.join("nested");
    let mut children = HashMap::new();
    children.insert(
        root.clone(),
        vec![
            entry(child.clone(), "child", 0, true, false),
            entry(root.join("root.bin"), "root.bin", 2, false, false),
        ],
    );
    children.insert(
        child.clone(),
        vec![
            entry(nested.clone(), "nested", 0, true, false),
            entry(child.join("child.bin"), "child.bin", 3, false, false),
        ],
    );
    children.insert(
        nested.clone(),
        vec![entry(
            nested.join("nested.bin"),
            "nested.bin",
            5,
            false,
            false,
        )],
    );
    let mut children: HashMap<PathBuf, Arc<Vec<DirEntry>>> = children
        .into_iter()
        .map(|(path, entries)| (path, Arc::new(entries)))
        .collect();
    let progress = ScanProgress::new();
    progress.begin_stage(2, children.len());
    let total = apply_directory_sizes(&root, &mut children, &progress, &AtomicBool::new(false));
    assert_eq!(total, 10);
    assert_eq!(children[&root][0].size, 8);
    assert_eq!(children[&child][0].size, 5);
    assert_eq!(progress.get_stage_progress(), (3, 3));
}

#[test]
fn in_place_sizing_handles_a_wide_tree() {
    const DIRECTORY_COUNT: usize = 10_000;
    let root = PathBuf::from("/wide-root");
    let mut root_entries = Vec::with_capacity(DIRECTORY_COUNT);
    let mut children = HashMap::with_capacity(DIRECTORY_COUNT + 1);
    for index in 0..DIRECTORY_COUNT {
        let name = format!("dir-{index}");
        let path = root.join(&name);
        root_entries.push(entry(path.clone(), &name, 0, true, false));
        children.insert(
            path.clone(),
            vec![entry(path.join("file.bin"), "file.bin", 1, false, false)],
        );
    }
    children.insert(root.clone(), root_entries);
    let mut children: HashMap<PathBuf, Arc<Vec<DirEntry>>> = children
        .into_iter()
        .map(|(path, entries)| (path, Arc::new(entries)))
        .collect();
    let progress = ScanProgress::new();
    progress.begin_stage(2, children.len());
    assert_eq!(
        apply_directory_sizes(&root, &mut children, &progress, &AtomicBool::new(false),),
        DIRECTORY_COUNT as u64
    );
    assert_eq!(
        progress.get_stage_progress(),
        (DIRECTORY_COUNT + 1, DIRECTORY_COUNT + 1)
    );
    assert_eq!(children[&root][0].size, 1);
}

#[test]
fn build_computes_sizes_temp_flags_navigation_and_progress() {
    let temp = TempDir::new("tree-build");
    temp.write("root.txt", b"12");
    temp.write("src/cache.pyc", b"123");
    temp.write("target/artifact", b"12345");
    let progress = Arc::new(ScanProgress::new());
    let mut tree = DirTree::build_with_progress(
        temp.path(),
        &matcher(),
        Arc::clone(&progress),
        Arc::new(AtomicBool::new(false)),
        false,
    );
    assert!(progress.is_done());
    assert_eq!(progress.get_files(), 3);
    assert_eq!(progress.get_dirs(), 2);
    assert_eq!(progress.get_bytes(), 10);
    assert_eq!(progress.get_phase(), 3);
    assert_eq!(
        progress.get_stage_progress(),
        (tree.children.len(), tree.children.len())
    );
    let root = tree.get_children(temp.path(), false);
    let target = root.iter().find(|e| e.name == "target").unwrap();
    assert!(target.is_temp);
    assert_eq!(target.size, 5);
    let src = tree.get_children(&temp.join("src"), false);
    assert_eq!(src[0].name, "..");
    assert!(src.iter().find(|e| e.name == "cache.pyc").unwrap().is_temp);
}

#[test]
fn cancelled_build_returns_no_children_and_marks_done() {
    let temp = TempDir::new("tree-cancel");
    temp.write("file", b"data");
    let progress = Arc::new(ScanProgress::new());
    let tree = DirTree::build_with_progress(
        temp.path(),
        &matcher(),
        Arc::clone(&progress),
        Arc::new(AtomicBool::new(true)),
        false,
    );
    assert!(tree.children.is_empty());
    assert!(progress.is_done());
}

#[test]
fn recursive_temp_stats_do_not_double_count_contents_of_temp_dirs() {
    let root = PathBuf::from("/virtual-root");
    let regular = root.join("regular");
    let target = root.join("target");
    let mut children = HashMap::new();
    children.insert(
        root.clone(),
        vec![
            entry(regular.clone(), "regular", 4, true, false),
            entry(target.clone(), "target", 10, true, true),
            entry(root.join("temp.pyc"), "temp.pyc", 2, false, true),
        ],
    );
    children.insert(
        regular.clone(),
        vec![entry(
            regular.join("nested.pyc"),
            "nested.pyc",
            4,
            false,
            true,
        )],
    );
    children.insert(
        target,
        vec![entry(
            root.join("target/inside.pyc"),
            "inside.pyc",
            10,
            false,
            true,
        )],
    );
    let tree = DirTree::from_children(children);
    assert_eq!(tree.get_temp_stats(&root), (1, 2, 16));
    assert_eq!(tree.get_temp_stats(Path::new("/missing")), (0, 0, 0));
}

#[test]
fn deleting_entry_updates_ancestors_and_directory_map() {
    let root = PathBuf::from("/root");
    let child = root.join("child");
    let target = child.join("target");
    let mut children = HashMap::new();
    children.insert(
        root.clone(),
        vec![entry(child.clone(), "child", 12, true, false)],
    );
    children.insert(
        child.clone(),
        vec![entry(target.clone(), "target", 7, true, true)],
    );
    children.insert(target.clone(), vec![]);
    let mut tree = DirTree::from_children(children);
    tree.delete_entry(&target, true);
    assert!(tree.get_children(&child, false).is_empty());
    assert_eq!(tree.get_children(&root, false)[0].size, 5);
    assert!(!tree.children.contains_key(&target));
    tree.delete_entry(&root.join("missing"), false);
}

#[test]
fn sorting_keeps_parent_first_and_directories_before_files() {
    let root = PathBuf::from("/root");
    let mut entries = vec![
        entry(root.join("z.txt"), "z.txt", 100, false, false),
        entry(root.join("b"), "b", 5, true, false),
        entry(root.join("a"), "A", 10, true, false),
        entry(root.parent().unwrap().into(), "..", 0, true, false),
    ];
    sort_by_size(&mut entries);
    assert_eq!(
        entries
            .iter()
            .map(|e| e.name.to_string_lossy())
            .collect::<Vec<_>>(),
        ["..", "A", "b", "z.txt"]
    );
    sort_by_name(&mut entries);
    assert_eq!(
        entries
            .iter()
            .map(|e| e.name.to_string_lossy())
            .collect::<Vec<_>>(),
        ["..", "A", "b", "z.txt"]
    );
}

#[test]
#[ignore = "manual release microbenchmark"]
fn manual_profile_path_hashers() {
    use std::collections::HashMap as StdHashMap;
    use std::hint::black_box;
    use std::time::Instant;

    let paths: Vec<_> = (0..100_000)
        .map(|index| PathBuf::from(format!("/fixture/dir-{index:08}/child")))
        .collect();
    let start = Instant::now();
    let std_map: StdHashMap<_, _> = paths.iter().cloned().zip(0usize..).collect();
    let std_insert = start.elapsed();
    let start = Instant::now();
    for path in &paths {
        black_box(std_map.get(path));
    }
    let std_lookup = start.elapsed();

    let start = Instant::now();
    let fold_map: HashMap<_, _> = paths.iter().cloned().zip(0usize..).collect();
    let fold_insert = start.elapsed();
    let start = Instant::now();
    for path in &paths {
        black_box(fold_map.get(path));
    }
    let fold_lookup = start.elapsed();
    println!(
        "path hashing: std insert={std_insert:?} lookup={std_lookup:?}; foldhash insert={fold_insert:?} lookup={fold_lookup:?}"
    );
}

#[test]
#[ignore = "manual release profile using CLEANER_PROFILE_ROOT"]
fn manual_profile_tui_tree_from_env() {
    use std::time::Instant;

    let Some(root) = std::env::var_os("CLEANER_PROFILE_ROOT").map(PathBuf::from) else {
        return;
    };
    if let Some(threads) = std::env::var("CLEANER_PROFILE_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        crate::pool::configure_scan_pool(threads);
    }
    let matcher = PatternMatcher::new(Arc::new(Config::default()));
    let progress = Arc::new(ScanProgress::new());
    let start = Instant::now();
    let tree = DirTree::build_with_progress(
        &root,
        &matcher,
        Arc::clone(&progress),
        Arc::new(AtomicBool::new(false)),
        false,
    );
    println!(
        "tui tree: elapsed={:?} directories={} files={} bytes={} errors={} retained_directories={}",
        start.elapsed(),
        progress.get_dirs(),
        progress.get_files(),
        progress.get_bytes(),
        progress.get_errors(),
        tree.children.len()
    );
}

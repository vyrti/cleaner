use super::*;
use crate::test_support::TempDir;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[test]
fn reads_file_directory_and_size() {
    let temp = TempDir::new("fastwalk-read");
    temp.mkdir("child");
    temp.write("data.bin", b"12345");
    let entries = read_dir_fast(temp.path()).unwrap();
    let file = entries.iter().find(|e| e.name == "data.bin").unwrap();
    let directory = entries.iter().find(|e| e.name == "child").unwrap();
    assert_eq!(file.size, 5);
    assert!(!file.is_dir);
    assert!(directory.is_dir);
    assert_eq!(directory.size, 0);
    assert!(read_dir_fast(&temp.join("missing")).is_err());

    let _type_only = read_dir_types(temp.path()).unwrap();
    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        _type_only
            .iter()
            .find(|entry| entry.name == "data.bin")
            .unwrap()
            .size,
        0
    );
}

#[cfg(unix)]
#[test]
fn reads_sparse_file_size() {
    let temp = TempDir::new("fastwalk-sparse");
    let path = temp.join("sparse.bin");
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(10 * 1024 * 1024).unwrap(); // 10 MiB
    drop(f);

    let entries = read_dir_fast(temp.path()).unwrap();
    let file = entries.iter().find(|e| e.name == "sparse.bin").unwrap();
    assert!(file.size < 10 * 1024 * 1024);
}

#[cfg(unix)]
#[test]
fn identifies_symlinks_without_following_them() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new("fastwalk-link");
    temp.write("target", b"payload");
    symlink(temp.join("target"), temp.join("link")).unwrap();
    let entries = read_dir_fast(temp.path()).unwrap();
    let link = entries.iter().find(|e| e.name == "link").unwrap();
    assert!(link.is_symlink);
    assert_eq!(link.size, 0);
}

#[cfg(target_os = "macos")]
#[test]
fn reads_child_directory_relative_to_parent_descriptor() {
    let temp = TempDir::new("fastwalk-openat");
    temp.write("child/data.bin", b"1234");
    let parent = mac::open_directory(temp.path()).unwrap();
    let child = mac::open_child_directory(&parent, std::ffi::OsStr::new("child")).unwrap();
    let entries = mac::read_open_directory(&child, MetadataMode::WithSizes).unwrap();
    let file = entries
        .iter()
        .find(|entry| entry.name == "data.bin")
        .unwrap();
    assert_eq!(file.size, 4);
    assert!(!file.is_dir);
}

#[cfg(unix)]
#[test]
fn preserves_non_utf8_names_and_paths() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let temp = TempDir::new("fastwalk-non-utf8");
    let directory_name = OsString::from_vec(b"dir-\xff".to_vec());
    let file_name = OsString::from_vec(b"file-\xfe".to_vec());
    let directory = temp.path().join(&directory_name);
    if std::fs::create_dir(&directory).is_err() {
        return;
    }
    if std::fs::write(directory.join(&file_name), b"data").is_err() {
        return;
    }

    let root_entries = read_dir_fast(temp.path()).unwrap();
    assert!(root_entries
        .iter()
        .any(|entry| entry.name.as_os_str().as_bytes() == b"dir-\xff"));
    let child_entries = read_dir_fast(&directory).unwrap();
    assert!(child_entries
        .iter()
        .any(|entry| entry.name.as_os_str().as_bytes() == b"file-\xfe"));
}

#[test]
fn parallel_walk_honors_skip_and_reports_progress() {
    let temp = TempDir::new("fastwalk-parallel");
    temp.write("root.txt", b"abc");
    temp.write("keep/inside.txt", b"12345");
    temp.write("skip/hidden.txt", b"1234567");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    let skip = temp.join("skip");
    let files = Arc::new(AtomicUsize::new(0));
    let dirs = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let callback = {
        let files = Arc::clone(&files);
        let dirs = Arc::clone(&dirs);
        let bytes = Arc::clone(&bytes);
        Arc::new(move |dir_count, file_count, byte_count| {
            dirs.fetch_add(dir_count, Ordering::Relaxed);
            files.fetch_add(file_count, Ordering::Relaxed);
            bytes.fetch_add(byte_count, Ordering::Relaxed);
        })
    };
    let tree = walk_parallel(
        temp.path().to_path_buf(),
        &pool,
        Arc::new(move |path| path == skip),
        Some(callback),
    );
    assert!(tree.contains_key(temp.path()));
    assert!(tree.contains_key(&temp.join("keep")));
    assert!(!tree.contains_key(&temp.join("skip")));
    assert_eq!(dirs.load(Ordering::Relaxed), 2);
    assert_eq!(files.load(Ordering::Relaxed), 2);
    assert_eq!(bytes.load(Ordering::Relaxed), 8);
}

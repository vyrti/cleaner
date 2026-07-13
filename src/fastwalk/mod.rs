use crossbeam_channel::Sender;
use foldhash::{HashMap, HashMapExt};
use rayon::ThreadPool;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// Eight covers the common small-directory case without reserving a large block
// for every empty/near-empty directory; Vec grows geometrically for wide ones.
pub(super) const INITIAL_DIRECTORY_CAPACITY: usize = 8;
type ProgressCallback = Arc<dyn Fn(usize, usize, u64) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataMode {
    TypesOnly,
    WithSizes,
}

pub struct WalkOutput<V> {
    pub entries: HashMap<PathBuf, V>,
    pub errors: usize,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;
#[cfg(target_os = "macos")]
mod mac;

#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: OsString,
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
}

pub fn read_dir_fast(path: &Path) -> std::io::Result<Vec<RawEntry>> {
    read_dir(path, MetadataMode::WithSizes)
}

pub fn read_dir_types(path: &Path) -> std::io::Result<Vec<RawEntry>> {
    read_dir(path, MetadataMode::TypesOnly)
}

fn read_dir(path: &Path, metadata_mode: MetadataMode) -> std::io::Result<Vec<RawEntry>> {
    #[cfg(target_os = "macos")]
    {
        mac::read_dir_bulk(path, metadata_mode)
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        linux::read_dir_fstatat(path, metadata_mode)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
    {
        let read_dir = std::fs::read_dir(path)?;
        let mut result = Vec::with_capacity(INITIAL_DIRECTORY_CAPACITY);
        for entry in read_dir {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let size = if file_type.is_file() && metadata_mode == MetadataMode::WithSizes {
                entry.metadata()?.len()
            } else {
                0
            };
            result.push(RawEntry {
                name: entry.file_name(),
                size,
                is_dir: file_type.is_dir(),
                is_symlink: file_type.is_symlink(),
            });
        }
        Ok(result)
    }
}

#[cfg(test)]
pub fn walk_parallel(
    root: PathBuf,
    pool: &ThreadPool,
    skip_check: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
    progress_callback: Option<ProgressCallback>,
) -> HashMap<PathBuf, Vec<RawEntry>> {
    walk_parallel_mapped(root, pool, skip_check, progress_callback, &|_, entries| {
        entries
    })
    .entries
}

pub fn walk_parallel_mapped<V, F>(
    root: PathBuf,
    pool: &ThreadPool,
    skip_check: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
    progress_callback: Option<ProgressCallback>,
    mapper: &F,
) -> WalkOutput<V>
where
    V: Send + 'static,
    F: Fn(&Path, Vec<RawEntry>) -> V + Sync,
{
    let (results_tx, results_rx) = crossbeam_channel::bounded(1024);
    let errors = AtomicUsize::new(0);
    let collector = std::thread::spawn(move || {
        let mut results = HashMap::with_capacity(16_384);
        for (path, entries) in results_rx {
            results.insert(path, entries);
        }
        results
    });

    #[cfg(target_os = "macos")]
    let mac_context = MacWalkContext {
        results: &results_tx,
        skip_check: skip_check.as_ref(),
        progress_callback: progress_callback.as_deref(),
        mapper,
        errors: &errors,
    };

    pool.scope(|scope| {
        #[cfg(target_os = "macos")]
        if root == Path::new("/") {
            match mac::open_directory(&root) {
                Ok(directory) => walk_recursive_macos(scope, root, directory, &mac_context),
                Err(_) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        } else {
            walk_recursive(
                scope,
                root,
                &results_tx,
                skip_check.as_ref(),
                progress_callback.as_deref(),
                mapper,
                &errors,
            );
        }
        #[cfg(not(target_os = "macos"))]
        walk_recursive(
            scope,
            root,
            &results_tx,
            skip_check.as_ref(),
            progress_callback.as_deref(),
            mapper,
            &errors,
        );
    });
    drop(results_tx);

    WalkOutput {
        entries: collector.join().expect("directory collector panicked"),
        errors: errors.into_inner(),
    }
}

fn walk_recursive<'scope, V, F>(
    scope: &rayon::Scope<'scope>,
    dir: PathBuf,
    results: &'scope Sender<(PathBuf, V)>,
    skip_check: &'scope (dyn Fn(&Path) -> bool + Send + Sync),
    progress_callback: Option<&'scope (dyn Fn(usize, usize, u64) + Send + Sync)>,
    mapper: &'scope F,
    errors: &'scope AtomicUsize,
) where
    V: Send + 'static,
    F: Fn(&Path, Vec<RawEntry>) -> V + Sync,
{
    let entries = match read_dir_fast(&dir) {
        Ok(e) => e,
        Err(_) => {
            errors.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    if let Some(ref cb) = progress_callback {
        let (dirs, files, bytes) = entries.iter().filter(|entry| !entry.is_symlink).fold(
            (0usize, 0usize, 0u64),
            |(dirs, files, bytes), entry| {
                if entry.is_dir {
                    (dirs + 1, files, bytes)
                } else {
                    (dirs, files + 1, bytes.saturating_add(entry.size))
                }
            },
        );
        cb(dirs, files, bytes);
    }

    let subdirs: Vec<PathBuf> = entries
        .iter()
        .filter(|e| e.is_dir && !e.is_symlink)
        .map(|e| dir.join(&e.name))
        .filter(|p| !skip_check(p))
        .collect();

    let mapped_entries = mapper(&dir, entries);
    if results.send((dir, mapped_entries)).is_err() {
        return;
    }

    for subdir in subdirs {
        scope.spawn(move |s| {
            walk_recursive(
                s,
                subdir,
                results,
                skip_check,
                progress_callback,
                mapper,
                errors,
            );
        });
    }
}

#[cfg(target_os = "macos")]
struct MacWalkContext<'scope, V, F> {
    results: &'scope Sender<(PathBuf, V)>,
    skip_check: &'scope (dyn Fn(&Path) -> bool + Send + Sync),
    progress_callback: Option<&'scope (dyn Fn(usize, usize, u64) + Send + Sync)>,
    mapper: &'scope F,
    errors: &'scope AtomicUsize,
}

#[cfg(target_os = "macos")]
fn walk_recursive_macos<'scope, V, F>(
    scope: &rayon::Scope<'scope>,
    dir: PathBuf,
    directory: Arc<mac::Directory>,
    context: &'scope MacWalkContext<'scope, V, F>,
) where
    V: Send + 'static,
    F: Fn(&Path, Vec<RawEntry>) -> V + Sync,
{
    let entries = match mac::read_open_directory(&directory, MetadataMode::WithSizes) {
        Ok(entries) => entries,
        Err(_) => {
            context.errors.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    if let Some(callback) = context.progress_callback {
        let (dirs, files, bytes) = entries.iter().filter(|entry| !entry.is_symlink).fold(
            (0usize, 0usize, 0u64),
            |(dirs, files, bytes), entry| {
                if entry.is_dir {
                    (dirs + 1, files, bytes)
                } else {
                    (dirs, files + 1, bytes.saturating_add(entry.size))
                }
            },
        );
        callback(dirs, files, bytes);
    }

    let subdirs: Vec<_> = entries
        .iter()
        .filter(|entry| entry.is_dir && !entry.is_symlink)
        .filter_map(|entry| {
            let path = dir.join(&entry.name);
            (!(context.skip_check)(&path)).then(|| (path, entry.name.clone()))
        })
        .collect();

    let mapped_entries = (context.mapper)(&dir, entries);
    if context.results.send((dir, mapped_entries)).is_err() {
        return;
    }

    for (subdir, name) in subdirs {
        let parent = Arc::clone(&directory);
        scope.spawn(
            move |scope| match mac::open_child_directory(&parent, &name) {
                Ok(child) => walk_recursive_macos(scope, subdir, child, context),
                Err(_) => {
                    context.errors.fetch_add(1, Ordering::Relaxed);
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
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
        if let Err(error) = std::fs::create_dir(&directory) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("failed to create non-UTF-8 test directory: {error}");
        }
        std::fs::write(directory.join(&file_name), b"data").unwrap();

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
        // The skipped directory itself is observed in the root; its contents are not.
        assert_eq!(dirs.load(Ordering::Relaxed), 2);
        assert_eq!(files.load(Ordering::Relaxed), 2);
        assert_eq!(bytes.load(Ordering::Relaxed), 8);
    }
}

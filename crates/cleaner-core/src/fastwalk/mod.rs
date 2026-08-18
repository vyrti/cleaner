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
                let metadata = entry.metadata()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    std::cmp::min(metadata.len(), metadata.blocks() * 512)
                }
                #[cfg(not(unix))]
                metadata.len()
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
mod tests;

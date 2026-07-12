use rayon::ThreadPool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;
#[cfg(target_os = "macos")]
mod mac;

#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
}

pub fn read_dir_fast(path: &Path) -> std::io::Result<Vec<RawEntry>> {
    #[cfg(target_os = "macos")]
    {
        mac::read_dir_bulk(path)
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        linux::read_dir_fstatat(path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
    {
        let read_dir = std::fs::read_dir(path)?;
        let mut result = Vec::with_capacity(256);
        for entry in read_dir {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let size = if file_type.is_file() {
                entry.metadata()?.len()
            } else {
                0
            };
            result.push(RawEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                size,
                is_dir: file_type.is_dir(),
                is_symlink: file_type.is_symlink(),
            });
        }
        Ok(result)
    }
}

pub fn walk_parallel(
    root: PathBuf,
    pool: &ThreadPool,
    skip_check: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
    progress_callback: Option<Arc<dyn Fn(bool, u64) + Send + Sync>>,
) -> HashMap<PathBuf, Vec<RawEntry>> {
    let results: Arc<Mutex<HashMap<PathBuf, Vec<RawEntry>>>> =
        Arc::new(Mutex::new(HashMap::with_capacity(16384)));

    {
        let results = Arc::clone(&results);
        let skip_check = Arc::clone(&skip_check);
        let progress_callback = progress_callback.clone();
        pool.scope(|s| {
            walk_recursive(s, root, results, skip_check, progress_callback);
        });
    }

    Arc::try_unwrap(results).unwrap().into_inner().unwrap()
}

fn walk_recursive(
    scope: &rayon::Scope<'_>,
    dir: PathBuf,
    results: Arc<Mutex<HashMap<PathBuf, Vec<RawEntry>>>>,
    skip_check: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
    progress_callback: Option<Arc<dyn Fn(bool, u64) + Send + Sync>>,
) {
    let entries = match read_dir_fast(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    if let Some(ref cb) = progress_callback {
        for e in &entries {
            if !e.is_symlink {
                cb(e.is_dir, e.size);
            }
        }
    }

    let subdirs: Vec<PathBuf> = entries
        .iter()
        .filter(|e| e.is_dir && !e.is_symlink)
        .map(|e| dir.join(&e.name))
        .filter(|p| !skip_check(p))
        .collect();

    results.lock().unwrap().insert(dir, entries);

    for subdir in subdirs {
        let results = Arc::clone(&results);
        let skip_check = Arc::clone(&skip_check);
        let progress_callback = progress_callback.clone();
        scope.spawn(move |s| {
            walk_recursive(s, subdir, results, skip_check, progress_callback);
        });
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
            Arc::new(move |is_dir, size| {
                if is_dir {
                    dirs.fetch_add(1, Ordering::Relaxed);
                } else {
                    files.fetch_add(1, Ordering::Relaxed);
                    bytes.fetch_add(size, Ordering::Relaxed);
                }
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

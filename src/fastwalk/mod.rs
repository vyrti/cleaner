use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use rayon::ThreadPool;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;

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

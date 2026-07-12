//! Parallel directory scanner using jwalk
//! Configured for maximum performance with rayon thread pool

use crate::config::Config;
use crate::fastwalk;
use crate::patterns::PatternMatcher;
#[cfg(test)]
use crate::pool::build_worker_pool;
use crossbeam_channel::Sender;
use rayon::ThreadPool;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn passes_age_filter(path: &Path, days: Option<u64>) -> bool {
    let Some(days) = days else { return true };
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed.as_secs() > days.saturating_mul(24 * 60 * 60))
}

fn matched_file_size(path: &Path, days: Option<u64>) -> Option<u64> {
    let metadata = std::fs::metadata(path);
    let Some(days) = days else {
        return Some(metadata.map(|value| value.len()).unwrap_or(0));
    };

    let metadata = metadata.ok()?;
    let modified = metadata.modified().ok()?;
    let elapsed = modified.elapsed().ok()?;
    (elapsed.as_secs() > days.saturating_mul(24 * 60 * 60)).then_some(metadata.len())
}

/// Result of scanning - a path to delete and whether it's a directory
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanSummary {
    pub entries: usize,
    pub errors: usize,
    pub receiver_closed: bool,
}

/// Parallel directory scanner
pub struct Scanner {
    matcher: PatternMatcher,
    config: Arc<Config>,
    root: PathBuf,
    pool: Arc<ThreadPool>,
}

impl Scanner {
    #[cfg(test)]
    pub fn new(root: PathBuf, num_threads: usize, config: Arc<Config>) -> Self {
        Self::with_pool(
            root,
            build_worker_pool(num_threads, "cleaner-worker"),
            config,
        )
    }

    pub fn with_pool(root: PathBuf, pool: Arc<ThreadPool>, config: Arc<Config>) -> Self {
        Self {
            matcher: PatternMatcher::new(Arc::clone(&config)),
            config,
            root,
            pool,
        }
    }

    /// Scan directory and send matching paths to channel
    /// Returns total number of entries scanned
    pub fn scan(&self, tx: Sender<ScanResult>) -> ScanSummary {
        self.scan_with_cancel(tx, &AtomicBool::new(false))
    }

    pub fn scan_with_cancel(&self, tx: Sender<ScanResult>, cancelled: &AtomicBool) -> ScanSummary {
        let scanned = std::sync::atomic::AtomicUsize::new(0);
        let errors = std::sync::atomic::AtomicUsize::new(0);
        let receiver_closed = AtomicBool::new(false);

        // macOS Docker exclusion: sparse disk image reports wrong sizes
        #[cfg(target_os = "macos")]
        let docker_path: Option<PathBuf> = {
            if let Some(home) = std::env::var_os("HOME") {
                let docker_container =
                    PathBuf::from(home).join("Library/Containers/com.docker.docker");
                if docker_container.exists() {
                    Some(docker_container)
                } else {
                    None
                }
            } else {
                None
            }
        };
        #[cfg(not(target_os = "macos"))]
        let docker_path: Option<PathBuf> = None;

        // Protected directories (NEVER auto-clean inside these, but allow scanning)
        let mut protected_paths: Vec<PathBuf> = vec![];

        // Home-relative paths
        if let Some(home) = dirs::home_dir() {
            protected_paths.extend(vec![
                home.join(".cargo"),
                home.join(".rustup"),
                home.join("go"),
                home.join(".go"),
                home.join(".npm"),
                home.join(".nvm"),
                home.join(".pyenv"),
                home.join(".rbenv"),
                home.join(".gradle"),
                home.join(".m2"),
                home.join(".local"),
                home.join(".config"),
                home.join(".ssh"),
                home.join(".gnupg"),
                home.join("Library"),
            ]);
            #[cfg(windows)]
            {
                protected_paths.push(home.join("AppData"));
            }
        }

        // Unix system directories
        #[cfg(unix)]
        {
            protected_paths.extend(vec![
                PathBuf::from("/System"),
                PathBuf::from("/Library"),
                PathBuf::from("/Applications"),
                PathBuf::from("/usr"),
                PathBuf::from("/var"),
                PathBuf::from("/etc"),
                PathBuf::from("/bin"),
                PathBuf::from("/sbin"),
                PathBuf::from("/lib"),
                PathBuf::from("/lib64"),
                PathBuf::from("/boot"),
                PathBuf::from("/opt"),
                PathBuf::from("/private"),
                PathBuf::from("/dev"),
                PathBuf::from("/proc"),
                PathBuf::from("/sys"),
                PathBuf::from("/run"),
            ]);
        }

        // Windows system directories
        #[cfg(windows)]
        {
            if let Some(win_dir) = std::env::var_os("SystemRoot").map(PathBuf::from) {
                protected_paths.push(win_dir);
            } else {
                protected_paths.push(PathBuf::from("C:\\Windows"));
            }
            if let Some(prog_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
                protected_paths.push(prog_files);
            } else {
                protected_paths.push(PathBuf::from("C:\\Program Files"));
            }
            if let Some(prog_files_x86) = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from) {
                protected_paths.push(prog_files_x86);
            } else {
                protected_paths.push(PathBuf::from("C:\\Program Files (x86)"));
            }
            if let Some(prog_data) = std::env::var_os("ProgramData").map(PathBuf::from) {
                protected_paths.push(prog_data);
            } else {
                protected_paths.push(PathBuf::from("C:\\ProgramData"));
            }
            protected_paths.push(PathBuf::from("C:\\System Volume Information"));
        }

        if self.config.force {
            protected_paths.clear();
        } else {
            protected_paths.retain(|path| !self.root.starts_with(path));
        }

        let context = ScanContext {
            matcher: &self.matcher,
            config: &self.config,
            tx: &tx,
            scanned: &scanned,
            docker_path: &docker_path,
            protected_paths: &protected_paths,
            root: &self.root,
            cancelled,
            errors: &errors,
            receiver_closed: &receiver_closed,
        };

        self.pool.scope(|s| {
            walk_scanner(s, self.root.clone(), false, &context);
        });

        ScanSummary {
            entries: scanned.into_inner(),
            errors: errors.into_inner(),
            receiver_closed: receiver_closed.into_inner(),
        }
    }
}

struct ScanContext<'a> {
    matcher: &'a PatternMatcher,
    config: &'a Config,
    tx: &'a Sender<ScanResult>,
    scanned: &'a std::sync::atomic::AtomicUsize,
    docker_path: &'a Option<PathBuf>,
    protected_paths: &'a [PathBuf],
    root: &'a Path,
    cancelled: &'a AtomicBool,
    errors: &'a std::sync::atomic::AtomicUsize,
    receiver_closed: &'a AtomicBool,
}

fn walk_scanner<'scope>(
    scope: &rayon::Scope<'scope>,
    dir: PathBuf,
    in_protected_dir: bool,
    context: &'scope ScanContext<'scope>,
) {
    if context.cancelled.load(Ordering::Relaxed) {
        return;
    }
    let entries = match fastwalk::read_dir_types(&dir) {
        Ok(e) => e,
        Err(_) => {
            context.errors.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    context.scanned.fetch_add(entries.len(), Ordering::Relaxed);

    let mut subdirs = Vec::with_capacity(8);

    for e in entries {
        if context.cancelled.load(Ordering::Relaxed) {
            return;
        }
        if e.is_dir {
            let path = dir.join(&e.name);
            if context
                .docker_path
                .as_ref()
                .is_some_and(|docker| path.starts_with(docker))
            {
                continue;
            }

            #[cfg(target_os = "macos")]
            {
                if path.starts_with("/System/Volumes")
                    && !context.root.starts_with("/System/Volumes")
                {
                    continue;
                }
                if path.starts_with("/Volumes") && !context.root.starts_with("/Volumes") {
                    continue;
                }
            }

            let in_protected = in_protected_dir
                || context
                    .protected_paths
                    .iter()
                    .any(|protected| path.starts_with(protected));
            if !in_protected && context.matcher.is_temp_directory(&e.name) {
                let should_delete = passes_age_filter(&path, context.config.days);

                if should_delete {
                    if context
                        .tx
                        .send(ScanResult {
                            path,
                            is_dir: true,
                            size: 0,
                        })
                        .is_err()
                    {
                        context.receiver_closed.store(true, Ordering::Relaxed);
                        context.cancelled.store(true, Ordering::Relaxed);
                        return;
                    }
                    continue;
                }
            }

            if !e.is_symlink {
                subdirs.push((path, in_protected));
            }
        } else if !in_protected_dir && context.matcher.is_temp_file(&e.name) {
            let path = dir.join(&e.name);
            if let Some(size) = matched_file_size(&path, context.config.days) {
                if context
                    .tx
                    .send(ScanResult {
                        path,
                        is_dir: false,
                        size,
                    })
                    .is_err()
                {
                    context.receiver_closed.store(true, Ordering::Relaxed);
                    context.cancelled.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    }

    // Spawn sub-tasks in parallel using rayon work-stealing
    for (subdir, protected) in subdirs {
        scope.spawn(move |s| {
            walk_scanner(s, subdir, protected, context);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use crossbeam_channel::unbounded;

    fn config(days: Option<u64>) -> Arc<Config> {
        Arc::new(Config {
            directories: vec!["target".into()],
            files: vec![".pyc".into()],
            days,
            force: false,
        })
    }

    #[test]
    fn scanner_finds_files_and_prunes_matched_directories() {
        let temp = TempDir::new("scanner-match");
        temp.write("module.pyc", b"1234");
        temp.write("source.rs", b"keep");
        temp.write("nested/other.pyc", b"12");
        temp.write("target/inside.pyc", b"not scanned after directory match");
        let (tx, rx) = unbounded();
        let scanned = Scanner::new(temp.path().to_path_buf(), 2, config(None)).scan(tx);
        let mut results: Vec<_> = rx.iter().collect();
        results.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(results.len(), 3);
        assert!(results
            .iter()
            .any(|r| r.path == temp.join("module.pyc") && !r.is_dir && r.size == 4));
        assert!(results
            .iter()
            .any(|r| r.path == temp.join("nested/other.pyc") && !r.is_dir));
        assert!(results
            .iter()
            .any(|r| r.path == temp.join("target") && r.is_dir));
        assert_eq!(scanned.entries, 5);
        assert_eq!(scanned.errors, 0);
    }

    #[test]
    fn scanner_uses_requested_thread_count() {
        let temp = TempDir::new("scanner-threads");
        let scanner = Scanner::new(temp.path().to_path_buf(), 3, config(None));
        assert_eq!(scanner.pool.current_num_threads(), 3);
    }

    #[test]
    fn scanner_reports_read_and_receiver_errors() {
        let temp = TempDir::new("scanner-errors");
        let (tx, rx) = unbounded();
        drop(rx);
        temp.write("match.pyc", b"data");
        let summary = Scanner::new(temp.path().to_path_buf(), 1, config(None)).scan(tx);
        assert!(summary.receiver_closed);

        let (tx, _rx) = unbounded();
        let summary = Scanner::new(temp.join("missing"), 1, config(None)).scan(tx);
        assert_eq!(summary.errors, 1);
    }

    #[cfg(unix)]
    #[test]
    fn scanner_traverses_non_utf8_directories() {
        use std::os::unix::ffi::OsStringExt;
        let temp = TempDir::new("scanner-non-utf8");
        let directory = temp
            .path()
            .join(std::ffi::OsString::from_vec(b"dir-\xff".to_vec()));
        if let Err(error) = std::fs::create_dir(&directory) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("failed to create non-UTF-8 test directory: {error}");
        }
        std::fs::write(directory.join("inside.pyc"), b"123").unwrap();
        let (tx, rx) = unbounded();
        let summary = Scanner::new(temp.path().to_path_buf(), 1, config(None)).scan(tx);
        assert_eq!(summary.errors, 0);
        assert!(rx.iter().any(|result| result.path.ends_with("inside.pyc")));
    }

    #[test]
    fn age_filter_keeps_recent_items_and_handles_huge_values() {
        let temp = TempDir::new("scanner-age");
        let file = temp.write("recent.pyc", b"data");
        assert!(passes_age_filter(&file, None));
        assert!(!passes_age_filter(&file, Some(u64::MAX)));
        assert!(!passes_age_filter(&temp.join("missing"), Some(1)));

        let (tx, rx) = unbounded();
        Scanner::new(temp.path().to_path_buf(), 1, config(Some(u64::MAX))).scan(tx);
        assert!(rx.iter().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn scanner_does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new("scanner-link");
        temp.write("outside/file.pyc", b"data");
        symlink(temp.join("outside"), temp.join("linked-dir")).unwrap();
        let (tx, rx) = unbounded();
        Scanner::new(temp.path().to_path_buf(), 1, config(None)).scan(tx);
        let results: Vec<_> = rx.iter().collect();
        assert_eq!(
            results
                .iter()
                .filter(|r| r.path.ends_with("file.pyc"))
                .count(),
            1
        );
    }
}

//! Parallel directory scanner using jwalk
//! Configured for maximum performance with rayon thread pool

use crate::config::Config;
use crate::fastwalk;
use crate::patterns::PatternMatcher;
use crate::pool::SCAN_POOL;
use crossbeam_channel::Sender;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
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

/// Result of scanning - a path to delete and whether it's a directory
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

/// Parallel directory scanner
pub struct Scanner {
    matcher: Arc<PatternMatcher>,
    config: Arc<Config>,
    root: PathBuf,
    #[allow(dead_code)]
    num_threads: usize,
}

impl Scanner {
    pub fn new(root: PathBuf, num_threads: usize, config: Arc<Config>) -> Self {
        Self {
            matcher: Arc::new(PatternMatcher::new(Arc::clone(&config))),
            config,
            root,
            num_threads,
        }
    }

    /// Scan directory and send matching paths to channel
    /// Returns total number of entries scanned
    pub fn scan(&self, tx: Sender<ScanResult>) -> usize {
        let matcher = Arc::clone(&self.matcher);
        let config_clone = Arc::clone(&self.config);
        let scanned = Arc::new(std::sync::atomic::AtomicUsize::new(0));

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

        let root_clone = self.root.clone();
        let scanned_clone = Arc::clone(&scanned);
        let docker_path_arc = Arc::new(docker_path);
        let protected_paths_arc = Arc::new(protected_paths);
        let root_arc = Arc::new(self.root.clone());

        SCAN_POOL.scope(|s| {
            walk_scanner(
                s,
                root_clone,
                matcher,
                config_clone,
                tx,
                scanned_clone,
                docker_path_arc,
                protected_paths_arc,
                root_arc,
            );
        });

        Arc::try_unwrap(scanned).unwrap().into_inner()
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_scanner(
    scope: &rayon::Scope<'_>,
    dir: PathBuf,
    matcher: Arc<PatternMatcher>,
    config: Arc<Config>,
    tx: Sender<ScanResult>,
    scanned: Arc<std::sync::atomic::AtomicUsize>,
    docker_path: Arc<Option<PathBuf>>,
    protected_paths: Arc<Vec<PathBuf>>,
    root: Arc<PathBuf>,
) {
    let entries = match fastwalk::read_dir_fast(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut subdirs = Vec::with_capacity(8);

    for e in entries {
        scanned.fetch_add(1, Ordering::Relaxed);

        let path = dir.join(&e.name);

        // 1. Skip Docker container on macOS
        if let Some(ref docker) = *docker_path {
            if path.starts_with(docker) {
                continue;
            }
        }

        // Calculate if path is in a protected system directory (where we won't auto-delete)
        let in_protected = !config.force
            && protected_paths
                .iter()
                .any(|p| path.starts_with(p) && !root.starts_with(p));

        // 3. Skip macOS OS mounts to prevent duplicate scans
        #[cfg(target_os = "macos")]
        {
            if (path.starts_with("/System/Volumes") || path == Path::new("/System/Volumes"))
                && !root.starts_with("/System/Volumes")
            {
                continue;
            }
            if (path.starts_with("/Volumes") || path == Path::new("/Volumes"))
                && !root.starts_with("/Volumes")
            {
                continue;
            }
        }

        if e.is_dir {
            if !in_protected && matcher.is_temp_directory(&e.name) {
                let should_delete = passes_age_filter(&path, config.days);

                if should_delete {
                    let _ = tx.send(ScanResult {
                        path,
                        is_dir: true,
                        size: 0,
                    });
                    continue;
                }
            }

            if !e.is_symlink {
                subdirs.push(path);
            }
        } else {
            if !in_protected && matcher.is_temp_file(&e.name) {
                let should_delete = passes_age_filter(&path, config.days);

                if should_delete {
                    let _ = tx.send(ScanResult {
                        path,
                        is_dir: false,
                        size: e.size,
                    });
                }
            }
        }
    }

    // Spawn sub-tasks in parallel using rayon work-stealing
    for subdir in subdirs {
        let matcher = Arc::clone(&matcher);
        let config = Arc::clone(&config);
        let tx = tx.clone();
        let scanned = Arc::clone(&scanned);
        let docker_path = Arc::clone(&docker_path);
        let protected_paths = Arc::clone(&protected_paths);
        let root = Arc::clone(&root);
        scope.spawn(move |s| {
            walk_scanner(
                s,
                subdir,
                matcher,
                config,
                tx,
                scanned,
                docker_path,
                protected_paths,
                root,
            );
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
        assert_eq!(scanned, 5);
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

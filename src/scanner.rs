//! Parallel directory scanner using jwalk
//! Configured for maximum performance with rayon thread pool

use crate::config::Config;
use crate::patterns::PatternMatcher;
use crossbeam_channel::Sender;
use crate::fastwalk;
use crate::pool::SCAN_POOL;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

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
                let docker_container = PathBuf::from(home)
                    .join("Library/Containers/com.docker.docker");
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

        // Protected toolchain/package manager directories (NEVER clean inside these)
        let protected_paths: Vec<PathBuf> = if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            vec![
                home.join(".cargo"),      // Rust toolchain & crates
                home.join(".rustup"),     // Rust toolchains
                home.join("go"),          // Go packages
                home.join(".go"),         // Go alternative
                home.join(".npm"),        // NPM cache
                home.join(".nvm"),        // Node version manager
                home.join(".pyenv"),      // Python version manager
                home.join(".rbenv"),      // Ruby version manager
                home.join(".gradle"),     // Gradle home
                home.join(".m2"),         // Maven repository
                home.join(".local"),      // User local bin/lib
                home.join(".config"),     // User config files
                home.join(".ssh"),        // SSH keys
                home.join(".gnupg"),      // GPG keys
                home.join("Library"),     // macOS Library (contains app data)
            ]
        } else {
            vec![]
        };

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

        // 2. Skip protected paths
        if protected_paths.iter().any(|p| path.starts_with(p)) {
            continue;
        }

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
            if matcher.is_temp_directory(&e.name) {
                let should_delete = if let Some(days) = config.days {
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(elapsed) = modified.elapsed() {
                                elapsed.as_secs() > days * 24 * 60 * 60
                            } else { false }
                        } else { false }
                    } else { false }
                } else {
                    true
                };

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
            if matcher.is_temp_file(&e.name) {
                let should_delete = if let Some(days) = config.days {
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(elapsed) = modified.elapsed() {
                                elapsed.as_secs() > days * 24 * 60 * 60
                            } else { false }
                        } else { false }
                    } else { false }
                } else {
                    true
                };

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

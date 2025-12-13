//! Parallel directory scanner using jwalk
//! Configured for maximum performance with rayon thread pool

use crate::config::Config;
use crate::patterns::PatternMatcher;
use crossbeam_channel::Sender;
use jwalk::{Parallelism, WalkDir};
use std::path::PathBuf;
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
        let mut scanned = 0;

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

        let docker_skip = Arc::new(docker_path);

        // Configure jwalk for maximum parallelism
        let docker_skip_clone = Arc::clone(&docker_skip);
        let walker = WalkDir::new(&self.root)
            .parallelism(Parallelism::RayonNewPool(self.num_threads))
            .skip_hidden(false)
            .follow_links(false)
            .process_read_dir(move |_depth, _path, _state, children| {
                // Skip Docker container on macOS
                if let Some(ref docker) = *docker_skip_clone {
                    children.retain(|entry| {
                        if let Ok(ref e) = entry {
                            !e.path().starts_with(docker)
                        } else {
                            true
                        }
                    });
                }
                
                // Mark directories for skip if they match our patterns
                // This prevents descending into directories we're going to delete
                let matcher_clone = Arc::clone(&matcher);
                let days_opt = config_clone.days;
                
                children.iter_mut().for_each(|entry| {
                    if let Ok(ref e) = entry {
                        if e.file_type().is_dir() {
                            if let Some(name) = e.file_name().to_str() {
                                if matcher_clone.is_temp_directory(name) {
                                    // CHECK TIME: If too new, don't delete AND don't skip descending 
                                    // (treat as normal dir to find potential nested heavy items? 
                                    // Actually, if we say "don't delete target because recent", 
                                    // we likely don't want to delete ANYTHING inside it either)
                                    let should_delete = if let Some(days) = days_opt {
                                        if let Ok(metadata) = e.metadata() {
                                            if let Ok(modified) = metadata.modified() {
                                                if let Ok(elapsed) = modified.elapsed() {
                                                     elapsed.as_secs() > days * 24 * 60 * 60
                                                } else { false } // systematic clock issues -> safe default
                                            } else { false } // no mod time -> safe default
                                        } else { false } // no metadata -> safe default
                                    } else {
                                        true
                                    };

                                    if should_delete {
                                        // We'll handle this directory, skip its contents
                                        let _ = entry.as_mut().map(|e| e.read_children_path = None);
                                    }
                                }
                            }
                        }
                    }
                });
            });

        let matcher = Arc::clone(&self.matcher);

        for entry in walker {
            scanned += 1;

            if let Ok(entry) = entry {
                let path = entry.path();
                let is_dir = entry.file_type().is_dir();

                if matcher.matches(&path, is_dir) {
                    // Check modification time if configured
                    let should_delete = if let Some(days) = self.config.days {
                        if let Ok(metadata) = entry.metadata() {
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
                        // Calculate size for directories (estimate) or files
                        let size = if is_dir {
                            // For directories marked for deletion, we'll calculate size during deletion
                            0
                        } else {
                            entry.metadata().map(|m| m.len()).unwrap_or(0)
                        };

                        let result = ScanResult {
                            path: path.to_path_buf(),
                            is_dir,
                            size,
                        };

                        // Send to deletion channel - ignore send errors (receiver dropped)
                        let _ = tx.send(result);
                    }
                }
            }
        }

        scanned
    }
}

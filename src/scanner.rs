//! Parallel directory scanner using jwalk
//! Configured for maximum performance with rayon thread pool

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
    root: PathBuf,
    num_threads: usize,
}

impl Scanner {
    pub fn new(root: PathBuf, num_threads: usize) -> Self {
        Self {
            matcher: Arc::new(PatternMatcher::new()),
            root,
            num_threads,
        }
    }

    /// Scan directory and send matching paths to channel
    /// Returns total number of entries scanned
    pub fn scan(&self, tx: Sender<ScanResult>) -> usize {
        let matcher = Arc::clone(&self.matcher);
        let mut scanned = 0;

        // Configure jwalk for maximum parallelism
        let walker = WalkDir::new(&self.root)
            .parallelism(Parallelism::RayonNewPool(self.num_threads))
            .skip_hidden(false)
            .follow_links(false)
            .process_read_dir(move |_depth, _path, _state, children| {
                // Mark directories for skip if they match our patterns
                // This prevents descending into directories we're going to delete
                let matcher_clone = Arc::clone(&matcher);
                children.iter_mut().for_each(|entry| {
                    if let Ok(ref e) = entry {
                        if e.file_type().is_dir() {
                            if let Some(name) = e.file_name().to_str() {
                                if matcher_clone.is_temp_directory(name) {
                                    // We'll handle this directory, skip its contents
                                    let _ = entry.as_mut().map(|e| e.read_children_path = None);
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

        scanned
    }
}

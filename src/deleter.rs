//! Parallel deletion engine
//! Uses rayon for parallel file/directory removal with streaming processing

use crate::scanner::ScanResult;
use crate::stats::Stats;
use crossbeam_channel::Receiver;
use rayon::prelude::*;
use std::fs;
use std::sync::Arc;

/// Parallel deletion worker
pub struct Deleter {
    stats: Arc<Stats>,
    dry_run: bool,
    verbose: bool,
}

impl Deleter {
    pub fn new(stats: Arc<Stats>, dry_run: bool, verbose: bool) -> Self {
        Self {
            stats,
            dry_run,
            verbose,
        }
    }

    /// Process items as they arrive - streaming parallel deletion
    /// Uses a batching approach for efficient parallelism
    pub fn process(&self, rx: Receiver<ScanResult>) {
        const BATCH_SIZE: usize = 64;
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        for item in rx {
            batch.push(item);
            
            // Process batch when full
            if batch.len() >= BATCH_SIZE {
                self.process_batch(&batch);
                batch.clear();
            }
        }

        // Process remaining items
        if !batch.is_empty() {
            self.process_batch(&batch);
        }
    }

    /// Process a batch of items in parallel
    #[inline]
    fn process_batch(&self, batch: &[ScanResult]) {
        batch.par_iter().for_each(|item| {
            self.delete_item(item);
        });
    }

    /// Delete a single item - size is calculated only in verbose mode
    fn delete_item(&self, item: &ScanResult) {
        // Only calculate size if verbose (skip expensive recursive walk otherwise)
        let size = if self.verbose {
            if item.is_dir {
                Self::dir_size_fast(&item.path)
            } else {
                item.size
            }
        } else {
            0
        };

        if self.verbose {
            let type_str = if item.is_dir { "DIR " } else { "FILE" };
            let size_str = humansize::format_size(size, humansize::BINARY);
            println!("[{}] {} ({})", type_str, item.path.display(), size_str);
        }

        if self.dry_run {
            if item.is_dir {
                self.stats.add_directory();
            } else {
                self.stats.add_file();
            }
            self.stats.add_bytes(size);
            return;
        }

        // Actually delete
        let result = if item.is_dir {
            fs::remove_dir_all(&item.path)
        } else {
            fs::remove_file(&item.path)
        };

        match result {
            Ok(_) => {
                if item.is_dir {
                    self.stats.add_directory();
                } else {
                    self.stats.add_file();
                }
                self.stats.add_bytes(size);
            }
            Err(e) => {
                self.stats.add_error();
                eprintln!("Error deleting {}: {}", item.path.display(), e);
            }
        }
    }

    /// Fast directory size estimation using parallel walk
    fn dir_size_fast(path: &std::path::Path) -> u64 {
        use jwalk::WalkDir;
        
        WalkDir::new(path)
            .skip_hidden(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .sum()
    }
}

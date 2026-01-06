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

    /// Delete a single item - counts files and bytes for directories
    fn delete_item(&self, item: &ScanResult) {
        // For directories, count files inside and calculate size before deletion
        let (file_count, size) = if item.is_dir {
            Self::count_dir_contents(&item.path)
        } else {
            (0, item.size)
        };

        if self.verbose {
            let type_str = if item.is_dir { "DIR " } else { "FILE" };
            let size_str = humansize::format_size(size, humansize::BINARY);
            println!("[{}] {} ({})", type_str, item.path.display(), size_str);
        }

        if self.dry_run {
            if item.is_dir {
                self.stats.add_directory();
                // Add the files that would be deleted inside this directory
                for _ in 0..file_count {
                    self.stats.add_file();
                }
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
                    // Add the files deleted inside this directory
                    for _ in 0..file_count {
                        self.stats.add_file();
                    }
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

    /// Count files and total size inside a directory using parallel walk
    fn count_dir_contents(path: &std::path::Path) -> (usize, u64) {
        use jwalk::WalkDir;
        
        let mut file_count = 0usize;
        let mut total_size = 0u64;
        
        for entry in WalkDir::new(path)
            .skip_hidden(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            file_count += 1;
            total_size += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
        
        (file_count, total_size)
    }
}

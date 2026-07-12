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

    /// Count files and total size inside a directory using fastwalk
    fn count_dir_contents(path: &std::path::Path) -> (usize, u64) {
        let mut file_count = 0usize;
        let mut total_size = 0u64;
        let mut stack = vec![path.to_path_buf()];

        while let Some(current_path) = stack.pop() {
            if let Ok(entries) = crate::fastwalk::read_dir_fast(&current_path) {
                for e in entries {
                    if e.is_dir {
                        if !e.is_symlink {
                            stack.push(current_path.join(&e.name));
                        }
                    } else {
                        file_count += 1;
                        total_size += e.size;
                    }
                }
            }
        }

        (file_count, total_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use crossbeam_channel::unbounded;

    #[test]
    fn dry_run_counts_directory_contents_without_deleting() {
        let temp = TempDir::new("deleter-dry");
        let directory = temp.mkdir("target");
        temp.write("target/a.bin", b"123");
        temp.write("target/nested/b.bin", b"12345");
        let stats = Arc::new(Stats::new());
        let (tx, rx) = unbounded();
        tx.send(ScanResult {
            path: directory.clone(),
            is_dir: true,
            size: 0,
        })
        .unwrap();
        drop(tx);
        Deleter::new(Arc::clone(&stats), true, false).process(rx);
        assert!(directory.exists());
        assert_eq!(
            (
                stats.directories(),
                stats.files(),
                stats.bytes(),
                stats.error_count()
            ),
            (1, 2, 8, 0)
        );
    }

    #[test]
    fn live_run_deletes_files_and_directories() {
        let temp = TempDir::new("deleter-live");
        let file = temp.write("cache.pyc", b"1234");
        let directory = temp.mkdir("target");
        temp.write("target/content", b"123456");
        let stats = Arc::new(Stats::new());
        let (tx, rx) = unbounded();
        tx.send(ScanResult {
            path: file.clone(),
            is_dir: false,
            size: 4,
        })
        .unwrap();
        tx.send(ScanResult {
            path: directory.clone(),
            is_dir: true,
            size: 0,
        })
        .unwrap();
        drop(tx);
        Deleter::new(Arc::clone(&stats), false, false).process(rx);
        assert!(!file.exists());
        assert!(!directory.exists());
        assert_eq!(
            (stats.directories(), stats.files(), stats.bytes()),
            (1, 2, 10)
        );
    }

    #[test]
    fn failed_deletion_increments_errors_only() {
        let temp = TempDir::new("deleter-error");
        let stats = Arc::new(Stats::new());
        let (tx, rx) = unbounded();
        tx.send(ScanResult {
            path: temp.join("missing"),
            is_dir: false,
            size: 99,
        })
        .unwrap();
        drop(tx);
        Deleter::new(Arc::clone(&stats), false, false).process(rx);
        assert_eq!(stats.error_count(), 1);
        assert_eq!(stats.files(), 0);
        assert_eq!(stats.bytes(), 0);
    }

    #[test]
    fn processing_spans_multiple_batches() {
        let temp = TempDir::new("deleter-batches");
        let stats = Arc::new(Stats::new());
        let (tx, rx) = unbounded();
        for i in 0..70 {
            tx.send(ScanResult {
                path: temp.join(format!("{i}.tmp")),
                is_dir: false,
                size: 1,
            })
            .unwrap();
        }
        drop(tx);
        Deleter::new(Arc::clone(&stats), true, false).process(rx);
        assert_eq!(stats.files(), 70);
        assert_eq!(stats.bytes(), 70);
    }
}

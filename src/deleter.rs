//! Parallel deletion engine
//! Uses rayon for parallel file/directory removal with streaming processing

#[cfg(test)]
use crate::pool::build_worker_pool;
use crate::scanner::ScanResult;
use crate::stats::Stats;
use crossbeam_channel::Receiver;
use rayon::prelude::*;
use rayon::ThreadPool;
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[derive(Default)]
struct DeleteOutcome {
    directories: usize,
    files: usize,
    bytes: u64,
    errors: Vec<String>,
    verbose: Option<String>,
}

impl DeleteOutcome {
    fn merge(&mut self, other: Self) {
        self.directories = self.directories.saturating_add(other.directories);
        self.files = self.files.saturating_add(other.files);
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.errors.extend(other.errors);
        if let Some(line) = other.verbose {
            println!("{line}");
        }
    }
}

/// Parallel deletion worker
pub struct Deleter {
    stats: Arc<Stats>,
    dry_run: bool,
    verbose: bool,
    pool: Arc<ThreadPool>,
    batch_size: usize,
}

impl Deleter {
    #[cfg(test)]
    pub fn with_threads(
        stats: Arc<Stats>,
        dry_run: bool,
        verbose: bool,
        num_threads: usize,
    ) -> Self {
        Self::with_pool(
            stats,
            dry_run,
            verbose,
            build_worker_pool(num_threads, "cleaner-worker"),
        )
    }

    pub fn with_pool(
        stats: Arc<Stats>,
        dry_run: bool,
        verbose: bool,
        pool: Arc<ThreadPool>,
    ) -> Self {
        let batch_size = pool
            .current_num_threads()
            .saturating_mul(32)
            .clamp(64, 1024);
        Self {
            stats,
            dry_run,
            verbose,
            pool,
            batch_size,
        }
    }

    /// Process items as they arrive - streaming parallel deletion
    /// Uses a batching approach for efficient parallelism
    pub fn process(&self, rx: Receiver<ScanResult>) {
        let mut batch = Vec::with_capacity(self.batch_size);

        for item in rx {
            batch.push(item);

            // Process batch when full
            if batch.len() >= self.batch_size {
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
        let outcomes: Vec<_> = self.pool.install(|| {
            batch
                .par_iter()
                .map(|item| self.delete_item(item))
                .collect()
        });
        let mut batch_outcome = DeleteOutcome::default();
        for outcome in outcomes {
            batch_outcome.merge(outcome);
        }
        for error in &batch_outcome.errors {
            eprintln!("{error}");
        }
        self.stats.add_batch(
            batch_outcome.directories,
            batch_outcome.files,
            batch_outcome.bytes,
            batch_outcome.errors.len(),
        );
    }

    fn delete_item(&self, item: &ScanResult) -> DeleteOutcome {
        let mut outcome = if self.dry_run && item.is_dir {
            Self::count_dir_contents(&item.path)
        } else if self.dry_run {
            DeleteOutcome {
                files: 1,
                bytes: item.size,
                ..DeleteOutcome::default()
            }
        } else if item.is_dir {
            Self::remove_dir_counted(&item.path)
        } else {
            match fs::remove_file(&item.path) {
                Ok(()) => DeleteOutcome {
                    files: 1,
                    bytes: item.size,
                    ..DeleteOutcome::default()
                },
                Err(error) => DeleteOutcome {
                    errors: vec![format!("Error deleting {}: {error}", item.path.display())],
                    ..DeleteOutcome::default()
                },
            }
        };

        if self.verbose {
            let type_str = if item.is_dir { "DIR " } else { "FILE" };
            let size_str = humansize::format_size(outcome.bytes, humansize::BINARY);
            outcome.verbose = Some(format!("[{type_str}] {} ({size_str})", item.path.display()));
        }

        if item.is_dir && self.dry_run {
            outcome.directories = 1;
        }
        outcome
    }

    fn remove_dir_counted(root: &Path) -> DeleteOutcome {
        let mut outcome = DeleteOutcome::default();
        let mut stack = vec![(root.to_path_buf(), false, true)];
        while let Some((path, visited, is_root)) = stack.pop() {
            if visited {
                match fs::remove_dir(&path) {
                    Ok(()) if is_root => outcome.directories = 1,
                    Ok(()) => {}
                    Err(error) => outcome
                        .errors
                        .push(format!("Error deleting {}: {error}", path.display())),
                }
                continue;
            }

            stack.push((path.clone(), true, is_root));
            match crate::fastwalk::read_dir_fast(&path) {
                Ok(entries) => {
                    for entry in entries {
                        let child = path.join(&entry.name);
                        if entry.is_dir && !entry.is_symlink {
                            stack.push((child, false, false));
                        } else {
                            match fs::remove_file(&child) {
                                Ok(()) => {
                                    outcome.files = outcome.files.saturating_add(1);
                                    outcome.bytes = outcome.bytes.saturating_add(entry.size);
                                }
                                Err(error) => outcome
                                    .errors
                                    .push(format!("Error deleting {}: {error}", child.display())),
                            }
                        }
                    }
                }
                Err(error) => outcome
                    .errors
                    .push(format!("Error reading {}: {error}", path.display())),
            }
        }
        outcome
    }

    fn count_dir_contents(path: &Path) -> DeleteOutcome {
        let mut outcome = DeleteOutcome::default();
        let mut stack = vec![path.to_path_buf()];

        while let Some(current_path) = stack.pop() {
            match crate::fastwalk::read_dir_fast(&current_path) {
                Ok(entries) => {
                    for entry in entries {
                        if entry.is_dir && !entry.is_symlink {
                            stack.push(current_path.join(&entry.name));
                        } else {
                            outcome.files = outcome.files.saturating_add(1);
                            outcome.bytes = outcome.bytes.saturating_add(entry.size);
                        }
                    }
                }
                Err(error) => outcome
                    .errors
                    .push(format!("Error reading {}: {error}", current_path.display())),
            }
        }

        outcome
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
        Deleter::with_threads(Arc::clone(&stats), true, false, 2).process(rx);
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
        Deleter::with_threads(Arc::clone(&stats), false, false, 2).process(rx);
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
        Deleter::with_threads(Arc::clone(&stats), false, false, 2).process(rx);
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
        Deleter::with_threads(Arc::clone(&stats), true, false, 2).process(rx);
        assert_eq!(stats.files(), 70);
        assert_eq!(stats.bytes(), 70);
    }

    #[test]
    fn deleter_uses_requested_thread_count() {
        let deleter = Deleter::with_threads(Arc::new(Stats::new()), true, false, 3);
        assert_eq!(deleter.pool.current_num_threads(), 3);
    }
}

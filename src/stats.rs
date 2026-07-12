//! Atomic statistics tracking for multi-threaded operations

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Thread-safe statistics using atomic operations
/// Uses Relaxed ordering for maximum performance since we only need eventual consistency
#[derive(Debug, Default)]
pub struct Stats {
    pub directories_deleted: AtomicUsize,
    pub files_deleted: AtomicUsize,
    pub bytes_freed: AtomicU64,
    pub errors: AtomicUsize,
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn add_directory(&self) {
        self.directories_deleted.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_file(&self) {
        self.files_deleted.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_bytes(&self, bytes: u64) {
        self.bytes_freed.fetch_add(bytes, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn directories(&self) -> usize {
        self.directories_deleted.load(Ordering::Relaxed)
    }

    pub fn files(&self) -> usize {
        self.files_deleted.load(Ordering::Relaxed)
    }

    pub fn bytes(&self) -> u64 {
        self.bytes_freed.load(Ordering::Relaxed)
    }

    pub fn error_count(&self) -> usize {
        self.errors.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn counters_start_at_zero_and_accumulate() {
        let stats = Stats::new();
        assert_eq!(
            (
                stats.directories(),
                stats.files(),
                stats.bytes(),
                stats.error_count()
            ),
            (0, 0, 0, 0)
        );
        stats.add_directory();
        stats.add_file();
        stats.add_bytes(42);
        stats.add_bytes(8);
        stats.add_error();
        assert_eq!(
            (
                stats.directories(),
                stats.files(),
                stats.bytes(),
                stats.error_count()
            ),
            (1, 1, 50, 1)
        );
    }

    #[test]
    fn counters_are_thread_safe() {
        let stats = Arc::new(Stats::new());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let stats = Arc::clone(&stats);
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        stats.add_file();
                        stats.add_bytes(2);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(stats.files(), 800);
        assert_eq!(stats.bytes(), 1600);
    }
}

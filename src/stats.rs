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

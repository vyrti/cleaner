use rayon::ThreadPool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

static CONFIGURED_SCAN_THREADS: AtomicUsize = AtomicUsize::new(0);
pub const MAX_WORKER_THREADS: usize = 256;

pub fn default_thread_count() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
}

pub fn normalize_thread_count(num_threads: usize) -> usize {
    num_threads.clamp(1, MAX_WORKER_THREADS)
}

pub fn build_worker_pool(num_threads: usize, name: &str) -> Arc<ThreadPool> {
    let prefix = name.to_string();
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(normalize_thread_count(num_threads))
            .thread_name(move |index| format!("{prefix}-{index}"))
            .build()
            .expect("build worker thread pool"),
    )
}

pub fn configure_scan_pool(num_threads: usize) {
    CONFIGURED_SCAN_THREADS.store(normalize_thread_count(num_threads), Ordering::Relaxed);
}

pub static SCAN_POOL: LazyLock<Arc<ThreadPool>> = LazyLock::new(|| {
    let num_threads = match CONFIGURED_SCAN_THREADS.load(Ordering::Relaxed) {
        0 => default_thread_count(),
        configured => configured,
    };
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|i| format!("scan-{}", i))
            .build()
            .unwrap(),
    )
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_pool_is_initialized_with_workers() {
        assert!(SCAN_POOL.current_num_threads() >= 1);
    }
}

use rayon::ThreadPool;
use std::sync::{Arc, LazyLock};

pub static SCAN_POOL: LazyLock<Arc<ThreadPool>> = LazyLock::new(|| {
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_cpus::get())
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

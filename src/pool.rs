use std::sync::{Arc, LazyLock};
use rayon::ThreadPool;

pub static SCAN_POOL: LazyLock<Arc<ThreadPool>> = LazyLock::new(|| {
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_cpus::get())
            .thread_name(|i| format!("scan-{}", i))
            .build()
            .unwrap()
    )
});

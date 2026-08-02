//! Cleaner core library: scan, clean, and disk-usage analyze engine.

pub mod config;
pub mod deleter;
pub mod disk_usage;
pub mod fastwalk;
pub mod patterns;
pub mod pool;
pub mod protected;
pub mod scanner;
pub mod stats;
pub mod test_support;
pub mod tree;

pub use config::Config;
pub use deleter::Deleter;
pub use disk_usage::get_disk_usage;
pub use patterns::PatternMatcher;
pub use protected::{is_protected_for_root, protected_paths_for_root};
pub use scanner::{ScanResult, ScanSummary, Scanner};
pub use stats::Stats;
pub use tree::{DirEntry, DirTree, ScanProgress};

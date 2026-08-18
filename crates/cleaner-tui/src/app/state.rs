use cleaner_core::tree::{self, DirTree};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Size,
    Name,
}

/// Deletion state for async deletion
pub struct DeleteState {
    pub handle: JoinHandle<Result<(), String>>,
    pub entry_name: OsString,
    pub entry_path: PathBuf,
    pub is_dir: bool,
    pub entry_size: u64,
}

/// Clean state for async cleaning
pub struct CleanState {
    pub handle: JoinHandle<(usize, usize, u64)>, // (dirs, files, bytes)
    pub cancelled: Arc<AtomicBool>,
}

pub struct RebuildState {
    pub handle: JoinHandle<DirTree>,
    pub completion_message: String,
    pub progress: Arc<tree::ScanProgress>,
    pub cancelled: Arc<AtomicBool>,
    pub restore_path: PathBuf,
    pub restore_name: Option<OsString>,
}

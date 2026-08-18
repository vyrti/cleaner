use crate::app::App;
use cleaner_core::patterns::PatternMatcher;
use cleaner_core::tree::{DirTree, ScanProgress};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::JoinHandle;

/// Options for starting a cleaner session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StartOpts {
    pub index_enabled: bool,
    pub rebuild_index: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Continue,
    Exit,
}

/// Host-friendly clean offer preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanOffer {
    Ready {
        path: PathBuf,
        dirs: usize,
        files: usize,
        bytes: u64,
    },
    Empty {
        path: PathBuf,
    },
    Unavailable(String),
}

pub(crate) enum Phase {
    Scanning {
        root: PathBuf,
        progress: Arc<ScanProgress>,
        cancelled: Arc<AtomicBool>,
        scan_handle: Option<JoinHandle<DirTree>>,
        matcher: Arc<PatternMatcher>,
        force: bool,
    },
    Ready(Box<App>),
    Exited,
}

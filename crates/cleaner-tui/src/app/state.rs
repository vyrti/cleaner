use cleaner_core::sysclean::{Candidate, RunReport, Target};
use cleaner_core::tree::{self, DirTree};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
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

/// Where the Deep Clean view is in its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepPhase {
    /// Measuring the catalog.
    Probing,
    /// Waiting for the user to mark rows.
    Ready,
    /// "Run N targets? (y/n)".
    Confirm,
    /// A destructive row is marked, so the user has to type the word out.
    Typing,
    /// Deleting.
    Running,
    /// Finished; showing what happened.
    Done(String),
}

/// State for the Deep Clean view.
///
/// Mirrors the shape of [`RebuildState`]: a worker thread, a cancel flag, and
/// atomics the render thread can read without locking.
pub struct DeepState {
    /// The home directory this view was built for.
    ///
    /// Held rather than re-resolved so the catalog, the measurement and the
    /// deletion allowlist can never disagree about which home they mean.
    pub home: PathBuf,
    pub items: Vec<Candidate>,
    /// Parallel to `items`.
    pub marked: Vec<bool>,
    pub cursor: usize,
    /// Deep Clean keeps a real scroll offset; the browser list recomputes its
    /// window from the cursor on every frame instead.
    pub offset: usize,
    pub collapsed: HashSet<String>,
    /// Rows whose paths do not exist are hidden until this is on.
    pub show_absent: bool,
    pub phase: DeepPhase,
    /// Buffer for the destructive-row confirmation.
    pub typed: String,
    pub progress: Arc<tree::ScanProgress>,
    pub cancelled: Arc<AtomicBool>,
    pub probe_handle: Option<JoinHandle<Vec<Candidate>>>,
    pub run_handle: Option<JoinHandle<RunReport>>,
    /// Collected deleter output. Never printed - that would corrupt the screen.
    pub errors: Arc<Mutex<Vec<String>>>,
    /// Targets that need administrator rights, waiting for the runner to
    /// suspend the terminal and run them.
    pub pending_elevated: Vec<Target>,
}

impl DeepState {
    pub fn new(
        home: PathBuf,
        progress: Arc<tree::ScanProgress>,
        cancelled: Arc<AtomicBool>,
        probe_handle: JoinHandle<Vec<Candidate>>,
    ) -> Self {
        Self {
            home,
            items: Vec::new(),
            marked: Vec::new(),
            cursor: 0,
            offset: 0,
            collapsed: HashSet::new(),
            show_absent: false,
            phase: DeepPhase::Probing,
            typed: String::new(),
            progress,
            cancelled,
            probe_handle: Some(probe_handle),
            run_handle: None,
            errors: Arc::new(Mutex::new(Vec::new())),
            pending_elevated: Vec::new(),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.probe_handle.is_some() || self.run_handle.is_some()
    }

    /// Total size of everything currently marked.
    pub fn marked_bytes(&self) -> u64 {
        self.marked
            .iter()
            .zip(&self.items)
            .filter(|(marked, _)| **marked)
            .map(|(_, candidate)| candidate.size)
            .sum()
    }

    pub fn marked_count(&self) -> usize {
        self.marked.iter().filter(|m| **m).count()
    }

    /// True when a marked row needs the user to type a word before it runs.
    pub fn has_destructive_marked(&self) -> bool {
        self.marked
            .iter()
            .zip(&self.items)
            .any(|(marked, candidate)| *marked && candidate.target.needs_typed_confirmation())
    }
}

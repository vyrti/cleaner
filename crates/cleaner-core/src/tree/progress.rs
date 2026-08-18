use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};

#[derive(Default)]
pub struct ScanProgress {
    pub files: AtomicUsize,
    pub dirs: AtomicUsize,
    pub bytes: AtomicU64,
    pub errors: AtomicUsize,
    pub done: AtomicBool,
    pub phase: AtomicU8,
    pub stage_current: AtomicUsize,
    pub stage_total: AtomicUsize,
}

impl ScanProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_files(&self) -> usize {
        self.files.load(Ordering::Relaxed)
    }

    pub fn get_dirs(&self) -> usize {
        self.dirs.load(Ordering::Relaxed)
    }

    pub fn get_bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn get_errors(&self) -> usize {
        self.errors.load(Ordering::Relaxed)
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    pub fn get_phase(&self) -> u8 {
        self.phase.load(Ordering::Acquire)
    }

    pub fn get_stage_progress(&self) -> (usize, usize) {
        (
            self.stage_current.load(Ordering::Relaxed),
            self.stage_total.load(Ordering::Relaxed),
        )
    }

    pub fn begin_stage(&self, phase: u8, total: usize) {
        self.stage_current.store(0, Ordering::Relaxed);
        self.stage_total.store(total, Ordering::Relaxed);
        self.phase.store(phase, Ordering::Release);
    }
}

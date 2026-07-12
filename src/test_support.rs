use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

/// Small dependency-free temporary directory helper for unit tests.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(name: &str) -> Self {
        let id = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cleaner-{name}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temporary test directory");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path.join(path)
    }

    pub fn mkdir(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = self.join(path);
        std::fs::create_dir_all(&path).expect("create test directory");
        path
    }

    pub fn write(&self, path: impl AsRef<Path>, contents: &[u8]) -> PathBuf {
        let path = self.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create test file parent");
        }
        std::fs::write(&path, contents).expect("write test file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

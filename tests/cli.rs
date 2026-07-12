use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let id = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("cleaner-cli-{name}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.0.join(path)
    }

    fn write(&self, path: impl AsRef<Path>, contents: &[u8]) -> PathBuf {
        let path = self.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cleaner(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cleaner"))
        .args(args)
        .env_remove("CLEANER_DIRS")
        .env_remove("CLEANER_FILES")
        .env_remove("CLEANER_DAYS")
        .output()
        .unwrap()
}

#[test]
fn help_and_version_are_available() {
    let help = cleaner(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Fastest disk scanner and cleaner"));
    let version = cleaner(&["--version"]);
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn json_errors_have_nonzero_exit_status() {
    let temp = TempDir::new("errors");
    let missing = temp.join("missing");
    let output = cleaner(&["--json", missing.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Folder does not exist"));

    let file = temp.write("plain-file", b"data");
    let output = cleaner(&["--json", file.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Path is not a directory"));
}

#[test]
fn json_dry_run_reports_matches_without_deleting() {
    let temp = TempDir::new("dry-run");
    temp.write("target/artifact", b"1234");
    let output = cleaner(&["--json", "--threads", "1", temp.path().to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"success\":true"));
    assert!(stdout.contains("\"mode\":\"dry-run\""));
    assert!(stdout.contains("\"deleted_directories\":1"));
    assert!(stdout.contains("\"deleted_files\":1"));
    assert!(temp.join("target/artifact").exists());
}

#[test]
fn confirmed_json_run_deletes_matches_and_preserves_other_files() {
    let temp = TempDir::new("live");
    temp.write("target/artifact", b"1234");
    temp.write("src/main.rs", b"keep");
    let output = cleaner(&["--json", "--confirm", temp.path().to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"mode\":\"live\""));
    assert!(!temp.join("target").exists());
    assert!(temp.join("src/main.rs").exists());
}

#[test]
fn human_readable_live_run_reports_verbose_results() {
    let temp = TempDir::new("human-output");
    temp.write("cache.pyc", b"1234");
    let output = cleaner(&[
        "--confirm",
        "--verbose",
        "--threads",
        "1",
        temp.path().to_str().unwrap(),
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("LIVE (files will be permanently deleted!)"));
    assert!(stdout.contains("[FILE]"));
    assert!(stdout.contains("Results:"));
    assert!(stdout.contains("Deleted:"));
    assert!(!temp.join("cache.pyc").exists());
}

#[test]
fn human_readable_missing_folder_error_goes_to_stderr() {
    let temp = TempDir::new("human-error");
    let missing = temp.join("missing");
    let output = cleaner(&["--confirm", missing.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Folder does not exist"));
}

//! Protected path helpers shared by scanner and analyze tree.

use std::path::{Path, PathBuf};

/// Build the protected directory list for a scan rooted at `root`.
///
/// When `force` is true the list is empty. Paths that contain `root` are
/// dropped so a scan that starts inside a protected tree is still useful.
pub fn protected_paths_for_root(root: &Path, force: bool) -> Vec<PathBuf> {
    let mut protected_paths: Vec<PathBuf> = Vec::new();

    if let Some(home) = dirs::home_dir() {
        protected_paths.extend([
            home.join(".cargo"),
            home.join(".rustup"),
            home.join("go"),
            home.join(".go"),
            home.join(".npm"),
            home.join(".nvm"),
            home.join(".pyenv"),
            home.join(".rbenv"),
            home.join(".gradle"),
            home.join(".m2"),
            home.join(".local"),
            home.join(".config"),
            home.join(".ssh"),
            home.join(".gnupg"),
            home.join("Library"),
        ]);
        #[cfg(windows)]
        {
            protected_paths.push(home.join("AppData"));
        }
    }

    #[cfg(unix)]
    {
        protected_paths.extend([
            PathBuf::from("/System"),
            PathBuf::from("/Library"),
            PathBuf::from("/Applications"),
            PathBuf::from("/usr"),
            PathBuf::from("/var"),
            PathBuf::from("/etc"),
            PathBuf::from("/bin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/lib"),
            PathBuf::from("/lib64"),
            PathBuf::from("/boot"),
            PathBuf::from("/opt"),
            PathBuf::from("/private"),
            PathBuf::from("/dev"),
            PathBuf::from("/proc"),
            PathBuf::from("/sys"),
            PathBuf::from("/run"),
        ]);
    }

    #[cfg(windows)]
    {
        if let Some(win_dir) = std::env::var_os("SystemRoot").map(PathBuf::from) {
            protected_paths.push(win_dir);
        } else {
            protected_paths.push(PathBuf::from("C:\\Windows"));
        }
        if let Some(prog_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
            protected_paths.push(prog_files);
        } else {
            protected_paths.push(PathBuf::from("C:\\Program Files"));
        }
        if let Some(prog_files_x86) = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from) {
            protected_paths.push(prog_files_x86);
        } else {
            protected_paths.push(PathBuf::from("C:\\Program Files (x86)"));
        }
        if let Some(prog_data) = std::env::var_os("ProgramData").map(PathBuf::from) {
            protected_paths.push(prog_data);
        } else {
            protected_paths.push(PathBuf::from("C:\\ProgramData"));
        }
        protected_paths.push(PathBuf::from("C:\\System Volume Information"));
    }

    if force {
        protected_paths.clear();
    } else {
        protected_paths.retain(|path| !root.starts_with(path));
    }

    protected_paths
}

/// Return true when `path` sits under a protected directory that does not
/// contain `root` (used by macOS index catch-up).
pub fn is_protected_for_root(root: &Path, path: &Path) -> bool {
    protected_paths_for_root(root, false)
        .iter()
        .any(|protected| path.starts_with(protected))
}

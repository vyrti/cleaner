//! The curated target tables, one module per platform.
//!
//! Entries are declared through [`Cat`], which exists purely to keep the
//! platform tables readable: every helper takes home-relative path fragments and
//! fills in the boilerplate.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use super::{Action, Group, Target, Tier};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Accumulator for a platform table.
pub(crate) struct Cat {
    home: PathBuf,
    targets: Vec<Target>,
}

// Which builders a platform table reaches for varies, so on any given target
// some of these are genuinely unused.
#[allow(dead_code)]
impl Cat {
    fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            targets: Vec::with_capacity(160),
        }
    }

    fn abs(&self, rel: &str) -> PathBuf {
        self.home.join(rel)
    }

    fn paths(&self, rel: &[&str]) -> Vec<PathBuf> {
        rel.iter().map(|r| self.abs(r)).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        id: &str,
        group: Group,
        tier: Tier,
        label: &str,
        detail: &str,
        action: Action,
        probe: Vec<PathBuf>,
        requires: Option<&str>,
    ) {
        self.targets.push(Target {
            id: id.to_string(),
            group,
            label: label.to_string(),
            detail: detail.to_string(),
            tier,
            action,
            probe,
            requires: requires.map(str::to_string),
        });
    }

    /// Remove home-relative paths outright.
    fn rm(&mut self, id: &str, group: Group, tier: Tier, label: &str, detail: &str, rel: &[&str]) {
        let paths = self.paths(rel);
        self.push(
            id,
            group,
            tier,
            label,
            detail,
            Action::Remove(paths.clone()),
            paths,
            None,
        );
    }

    /// Empty home-relative directories, keeping the directories themselves.
    fn empty(
        &mut self,
        id: &str,
        group: Group,
        tier: Tier,
        label: &str,
        detail: &str,
        rel: &[&str],
    ) {
        let paths = self.paths(rel);
        self.push(
            id,
            group,
            tier,
            label,
            detail,
            Action::Empty(paths.clone()),
            paths,
            None,
        );
    }

    /// Remove everything matching home-relative glob patterns.
    fn glob(
        &mut self,
        id: &str,
        group: Group,
        tier: Tier,
        label: &str,
        detail: &str,
        rel: &[&str],
    ) {
        let paths = self.paths(rel);
        self.push(
            id,
            group,
            tier,
            label,
            detail,
            Action::Glob(paths.clone()),
            paths,
            None,
        );
    }

    /// Run a tool. `probe_rel` is what gets measured, which is usually the cache
    /// the tool will clear.
    #[allow(clippy::too_many_arguments)]
    fn cmd(
        &mut self,
        id: &str,
        group: Group,
        tier: Tier,
        label: &str,
        detail: &str,
        program: &str,
        args: &[&str],
        probe_rel: &[&str],
    ) {
        let probe = self.paths(probe_rel);
        self.push(
            id,
            group,
            tier,
            label,
            detail,
            Action::Command {
                program: program.to_string(),
                args: args.iter().map(|a| a.to_string()).collect(),
            },
            probe,
            Some(program),
        );
    }

    /// Absolute-path removal, for system locations outside the home directory.
    fn abs_rm(
        &mut self,
        id: &str,
        group: Group,
        tier: Tier,
        label: &str,
        detail: &str,
        paths: &[&str],
    ) {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        self.push(
            id,
            group,
            tier,
            label,
            detail,
            Action::Remove(paths.clone()),
            paths,
            None,
        );
    }

    /// Absolute-path emptying that requires administrator rights.
    fn root_empty(&mut self, id: &str, label: &str, detail: &str, paths: &[&str]) {
        let owned: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        self.push(
            id,
            Group::SystemJunk,
            Tier::NeedsRoot,
            label,
            detail,
            Action::Empty(owned.clone()),
            owned,
            None,
        );
    }

    /// Absolute-path removal that requires administrator rights.
    fn root_rm(&mut self, id: &str, tier: Tier, label: &str, detail: &str, paths: &[&str]) {
        let owned: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        self.push(
            id,
            Group::SystemJunk,
            tier,
            label,
            detail,
            Action::Remove(owned.clone()),
            owned,
            None,
        );
    }

    /// A command that requires administrator rights.
    #[allow(clippy::too_many_arguments)]
    fn root_cmd(
        &mut self,
        id: &str,
        label: &str,
        detail: &str,
        program: &str,
        args: &[&str],
        probe: &[&str],
    ) {
        let probe: Vec<PathBuf> = probe.iter().map(PathBuf::from).collect();
        self.push(
            id,
            Group::SystemJunk,
            Tier::NeedsRoot,
            label,
            detail,
            Action::Elevated {
                program: program.to_string(),
                args: args.iter().map(|a| a.to_string()).collect(),
            },
            probe,
            None,
        );
    }

    /// Absolute-path glob removal.
    fn abs_glob(
        &mut self,
        id: &str,
        group: Group,
        tier: Tier,
        label: &str,
        detail: &str,
        paths: &[&str],
    ) {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        self.push(
            id,
            group,
            tier,
            label,
            detail,
            Action::Glob(paths.clone()),
            paths,
            None,
        );
    }

    /// Absolute-path row that is listed and measured but never deletable.
    fn abs_report(&mut self, id: &str, group: Group, label: &str, detail: &str, paths: &[&str]) {
        let probe: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        self.push(
            id,
            group,
            Tier::Reclaimable,
            label,
            detail,
            Action::ReportOnly,
            probe,
            None,
        );
    }

    /// Listed and measured, never deletable.
    fn report(&mut self, id: &str, group: Group, label: &str, detail: &str, rel: &[&str]) {
        let probe = self.paths(rel);
        self.push(
            id,
            group,
            Tier::Reclaimable,
            label,
            detail,
            Action::ReportOnly,
            probe,
            None,
        );
    }
}

/// Build the catalog for the current platform.
pub fn catalog(home: &Path) -> Vec<Target> {
    let mut cat = Cat::new(home);

    #[cfg(target_os = "macos")]
    macos::fill(&mut cat);
    #[cfg(target_os = "windows")]
    windows::fill(&mut cat);
    #[cfg(target_os = "linux")]
    linux::fill(&mut cat);

    toolchains(&mut cat);
    residual_caches(&mut cat);

    cat.targets
}

/// One row per installed-but-not-default toolchain version.
///
/// These are enumerated from disk rather than by shelling out to `rustup` /
/// `nvm`, so the catalog stays cheap to build and works when the manager is
/// installed but not on `PATH`.
fn toolchains(cat: &mut Cat) {
    let rustup = cat.abs(".rustup/toolchains");
    if let Some(names) = subdirectories(&rustup) {
        let default = default_rust_toolchain(&cat.home);
        for name in names {
            if Some(name.as_str()) == default.as_deref() {
                continue;
            }
            let id = format!("rust-toolchain-{name}");
            cat.push(
                &id,
                Group::Toolchain,
                Tier::Reclaimable,
                &format!("Rust toolchain {name}"),
                "not the default toolchain; reinstall with rustup if needed",
                Action::Command {
                    program: "rustup".to_string(),
                    args: vec![
                        "toolchain".to_string(),
                        "uninstall".to_string(),
                        name.clone(),
                    ],
                },
                vec![rustup.join(&name)],
                Some("rustup"),
            );
        }
    }

    for (dir, label, note) in [
        (".nvm/versions/node", "Node", "old nvm-managed Node version"),
        (
            ".pyenv/versions",
            "Python",
            "old pyenv-managed Python version",
        ),
    ] {
        let base = cat.abs(dir);
        let Some(names) = subdirectories(&base) else {
            continue;
        };
        for name in names {
            let id = format!("{}-{name}", label.to_lowercase());
            cat.push(
                &id,
                Group::Toolchain,
                Tier::Reclaimable,
                &format!("{label} {name}"),
                note,
                Action::Remove(vec![base.join(&name)]),
                vec![base.join(&name)],
                None,
            );
        }
    }
}

/// Read `~/.rustup/settings.toml` for the default toolchain name.
///
/// Deliberately a substring scan rather than a TOML parse: this only decides
/// which row to *omit*, and being wrong costs the user one extra unchecked row.
fn default_rust_toolchain(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join(".rustup/settings.toml")).ok()?;
    text.lines()
        .find_map(|line| line.trim().strip_prefix("default_toolchain = "))
        .map(|value| value.trim().trim_matches('"').to_string())
}

/// One row per unrecognised cache directory over the size threshold.
///
/// This is what covers applications the catalog has never heard of, so the
/// feature stays useful as new tools appear. Rows land in [`Group::Other`] and
/// are filtered by size after probing.
fn residual_caches(cat: &mut Cat) {
    let covered: HashSet<PathBuf> = cat
        .targets
        .iter()
        .flat_map(|t| t.probe.iter())
        .cloned()
        .collect();

    for root in cache_roots(&cat.home) {
        let Some(names) = subdirectories(&root) else {
            continue;
        };
        for name in names {
            let path = root.join(&name);
            // Skip anything an explicit entry already claims, at any depth.
            if covered
                .iter()
                .any(|c| c.starts_with(&path) || path.starts_with(c))
            {
                continue;
            }
            let id = format!("residual-{}", name.to_lowercase().replace(' ', "-"));
            if cat.targets.iter().any(|t| t.id == id) {
                continue;
            }
            cat.push(
                &id,
                Group::Other,
                Tier::Reclaimable,
                &name,
                "unrecognised cache directory",
                Action::Remove(vec![path.clone()]),
                vec![path],
                None,
            );
        }
    }
}

fn cache_roots(home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![home.join("Library/Caches")]
    }
    #[cfg(target_os = "windows")]
    {
        let mut roots = Vec::new();
        if let Some(local) = dirs::data_local_dir() {
            roots.push(local);
        }
        let _ = home;
        roots
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        vec![home.join(".cache")]
    }
}

/// Immediate subdirectory names of `path`, or `None` when it is not a readable
/// directory.
fn subdirectories(path: &Path) -> Option<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    Some(names)
}

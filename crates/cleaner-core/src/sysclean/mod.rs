//! Deep-clean catalog: curated, per-platform cleanup targets.
//!
//! The rest of the crate finds junk by *matching patterns* during a walk, and
//! [`crate::protected`] keeps that matching away from system directories. This
//! module works the other way round: every target is written out by hand, so it
//! is allowed to reach into locations the pattern matcher must never touch
//! (`~/Library`, `~/.cargo`, `/private/var`, ...).
//!
//! That inversion is the whole safety model, so it is enforced rather than
//! assumed: [`exec::run`] refuses any path that is not strictly inside one of
//! [`allowed_roots`], and the catalog tests assert the same property over every
//! entry.

pub mod catalog;
pub mod elevate;
pub mod exec;
pub mod glob;
pub mod probe;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub use catalog::catalog;
pub use elevate::{run_elevated, ElevatedReport};
pub use exec::{run, RunReport};
pub use probe::probe;

/// How much a user stands to lose by running a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Pure cache. Regenerates on demand, nothing is lost. Marked by default.
    Safe,
    /// Real work is discarded or has to be re-downloaded. Never marked by default.
    Reclaimable,
    /// Irreversible and expensive: VM disks, OS rollback data.
    Destructive,
    /// Requires administrator rights, so it cannot run in-process.
    NeedsRoot,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Reclaimable => "reclaimable",
            Self::Destructive => "destructive",
            Self::NeedsRoot => "needs admin",
        }
    }
}

/// What kind of application a target belongs to. Drives grouping in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    DevCache,
    Toolchain,
    Container,
    Mobile,
    Editor,
    Browser,
    Chat,
    Media,
    Games,
    SystemJunk,
    Downloads,
    Other,
}

/// Display order, top to bottom.
pub const GROUP_ORDER: [Group; 12] = [
    Group::DevCache,
    Group::Toolchain,
    Group::Container,
    Group::Mobile,
    Group::Editor,
    Group::Browser,
    Group::Chat,
    Group::Media,
    Group::Games,
    Group::SystemJunk,
    Group::Downloads,
    Group::Other,
];

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Self::DevCache => "Dev caches",
            Self::Toolchain => "Toolchains",
            Self::Container => "Containers & VMs",
            Self::Mobile => "Xcode & mobile",
            Self::Editor => "Editors",
            Self::Browser => "Browsers",
            Self::Chat => "Chat & meetings",
            Self::Media => "Media & creative",
            Self::Games => "Games",
            Self::SystemJunk => "System junk",
            Self::Downloads => "Downloads",
            Self::Other => "Other caches",
        }
    }
}

/// What running a target actually does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Remove these paths outright.
    Remove(Vec<PathBuf>),
    /// Remove the contents of these directories, keeping the directories.
    Empty(Vec<PathBuf>),
    /// Remove every path matching these patterns. Only `*` is supported, and
    /// only within a single path component (`User Data/*/Cache`).
    Glob(Vec<PathBuf>),
    /// Run a tool. Output is captured, never inherited - inherited stdio would
    /// scribble over the alternate screen.
    Command { program: String, args: Vec<String> },
    /// Needs administrator rights. Never executed in-process; handed back to
    /// the caller, which owns the terminal. See [`elevate`].
    Elevated { program: String, args: Vec<String> },
    /// Sized and listed, never deleted. For things a user should see but that
    /// this tool has no business removing (guest VM disks, WSL distros).
    ReportOnly,
}

impl Action {
    /// Paths this action would delete, if any.
    pub fn paths(&self) -> &[PathBuf] {
        match self {
            Self::Remove(paths) | Self::Empty(paths) | Self::Glob(paths) => paths,
            Self::Command { .. } | Self::Elevated { .. } | Self::ReportOnly => &[],
        }
    }

    /// Human-readable one-liner, shown when the cursor rests on a row.
    pub fn describe(&self) -> String {
        match self {
            Self::Remove(paths) => format!("remove {} path(s)", paths.len()),
            Self::Empty(paths) => format!("empty {} directory(ies)", paths.len()),
            Self::Glob(paths) => format!("remove matches of {} pattern(s)", paths.len()),
            Self::Command { program, args } | Self::Elevated { program, args } => {
                format!("{program} {}", args.join(" "))
            }
            Self::ReportOnly => "listed only, never deleted".to_string(),
        }
    }
}

/// One catalog entry.
#[derive(Debug, Clone)]
pub struct Target {
    /// Stable identifier. Unique across the whole catalog (asserted in tests).
    pub id: String,
    pub group: Group,
    pub label: String,
    /// One line: what is lost, or what regenerates.
    pub detail: String,
    pub tier: Tier,
    pub action: Action,
    /// What to measure. Often the same as the action paths, but not always -
    /// `docker system prune` deletes nothing on disk that we can point at, and
    /// `Empty` measures the directory while deleting only its contents.
    pub probe: Vec<PathBuf>,
    /// Binary that must be on `PATH` for this target to be offered at all.
    pub requires: Option<String>,
}

impl Target {
    /// Only pure caches are pre-selected.
    pub fn default_marked(&self) -> bool {
        self.tier == Tier::Safe
    }

    /// Report-only rows can never be marked or run.
    pub fn selectable(&self) -> bool {
        !matches!(self.action, Action::ReportOnly)
    }

    /// Destructive rows demand the user type a word before they run.
    pub fn needs_typed_confirmation(&self) -> bool {
        self.tier == Tier::Destructive
    }
}

/// A target plus its measured size.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub target: Target,
    pub size: u64,
    /// False when nothing the target points at exists on this machine.
    pub present: bool,
}

impl Candidate {
    /// Section heading this row is filed under. Anything needing admin rights is
    /// pulled out of its normal group so the elevation boundary is obvious.
    pub fn section(&self) -> &'static str {
        if self.target.tier == Tier::NeedsRoot {
            "Needs admin"
        } else {
            self.target.group.label()
        }
    }

    /// Display order of this row's section. Admin rows sort last.
    pub fn section_rank(&self) -> usize {
        if self.target.tier == Tier::NeedsRoot {
            return GROUP_ORDER.len();
        }
        GROUP_ORDER
            .iter()
            .position(|group| *group == self.target.group)
            .unwrap_or(GROUP_ORDER.len())
    }
}

/// Roots that catalog paths are permitted to live under.
///
/// Anything outside this set is a bug in the catalog, and [`exec::run`] treats
/// it as one. Note the home directory itself is a root but is not a *valid
/// target* - see [`is_path_allowed`].
pub fn allowed_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![home.to_path_buf()];

    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Library/Caches"));
        roots.push(PathBuf::from("/Library/Logs"));
        roots.push(PathBuf::from("/private/var/db"));
        roots.push(PathBuf::from("/private/var/folders"));
        roots.push(PathBuf::from("/System/Volumes/Data/macOS Install Data"));
    }

    #[cfg(target_os = "windows")]
    {
        for key in ["SystemRoot", "ProgramData", "SystemDrive"] {
            if let Some(value) = std::env::var_os(key) {
                roots.push(PathBuf::from(value));
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/var/cache"));
        roots.push(PathBuf::from("/var/log"));
        roots.push(PathBuf::from("/var/tmp"));
    }

    roots
}

/// True when `path` may itself be deleted.
///
/// Strictly *inside* a root - equal to a root is rejected, which is what stops a
/// catalog typo from turning into `rm -rf ~`. Parent traversal is rejected
/// outright rather than normalised, because normalising a path that does not
/// exist yet is not reliable.
pub fn is_path_allowed(path: &Path, roots: &[PathBuf]) -> bool {
    if has_traversal(path) {
        return false;
    }
    roots
        .iter()
        .any(|root| path.starts_with(root) && path != root.as_path())
}

/// True when `path` may have its *contents* deleted.
///
/// Looser than [`is_path_allowed`] by exactly one case: a root may be emptied
/// even though it may not itself be removed. That is what lets a target clear
/// `/Library/Caches` without `/Library/Caches` ever being a deletable path.
/// Every child still has to pass [`is_path_allowed`], which it does by
/// construction.
pub fn is_container_allowed(path: &Path, roots: &[PathBuf]) -> bool {
    if has_traversal(path) {
        return false;
    }
    roots.iter().any(|root| path.starts_with(root))
}

fn has_traversal(path: &Path) -> bool {
    path.components()
        .any(|component| component == std::path::Component::ParentDir)
}

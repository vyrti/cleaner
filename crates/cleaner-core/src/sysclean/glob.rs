//! Minimal glob expansion for catalog paths.
//!
//! Only `*` is supported, and only inside a single path component - enough for
//! `User Data/*/Cache` and `thumbcache_*.db`, and small enough to reason about.
//! Anything fancier would widen what a catalog entry can reach, which is exactly
//! what the safety model is trying to bound.

use std::path::{Path, PathBuf};

/// True when `pattern` needs expanding.
pub fn is_pattern(path: &Path) -> bool {
    path.to_string_lossy().contains('*')
}

/// Expand a pattern into the paths that currently exist.
///
/// Returns the input unchanged when it contains no `*`, so callers can pass
/// every path through without checking first. Never returns paths that do not
/// exist.
pub fn expand(pattern: &Path) -> Vec<PathBuf> {
    if !is_pattern(pattern) {
        return if pattern.exists() {
            vec![pattern.to_path_buf()]
        } else {
            Vec::new()
        };
    }

    let mut bases: Vec<PathBuf> = Vec::new();
    let mut started = false;

    for component in pattern.components() {
        let part = component.as_os_str().to_string_lossy().into_owned();

        if !started {
            // Seed from the first component so absolute and relative patterns
            // both work.
            bases.push(PathBuf::from(component.as_os_str()));
            started = true;
            continue;
        }

        if !part.contains('*') {
            for base in &mut bases {
                base.push(&part);
            }
            continue;
        }

        // A wildcard component: replace every base with its matching children.
        let mut next = Vec::new();
        for base in &bases {
            let Ok(entries) = std::fs::read_dir(base) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if matches(&part, &name) {
                    next.push(base.join(name.as_ref()));
                }
            }
        }
        bases = next;
        if bases.is_empty() {
            return Vec::new();
        }
    }

    bases.retain(|path| path.exists());
    bases.sort();
    bases.dedup();
    bases
}

/// Wildcard match for a single path component. `*` matches any run of
/// characters, including none.
fn matches(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }

    let mut rest = name;

    // The first and last segments are anchored; everything between just has to
    // appear in order.
    if let Some(first) = parts.first() {
        let Some(stripped) = rest.strip_prefix(first) else {
            return false;
        };
        rest = stripped;
    }
    if let Some(last) = parts.last() {
        if parts.len() > 1 {
            if rest.len() < last.len() || !rest.ends_with(last) {
                return false;
            }
            rest = &rest[..rest.len() - last.len()];
        }
    }
    for middle in &parts[1..parts.len().saturating_sub(1)] {
        if middle.is_empty() {
            continue;
        }
        let Some(index) = rest.find(middle) else {
            return false;
        };
        rest = &rest[index + middle.len()..];
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn wildcard_matching_is_anchored_at_both_ends() {
        assert!(matches("*", "anything"));
        assert!(matches("thumbcache_*.db", "thumbcache_32.db"));
        assert!(!matches("thumbcache_*.db", "thumbcache_32.dbx"));
        assert!(!matches("thumbcache_*.db", "other_32.db"));
        assert!(matches("AndroidStudio*", "AndroidStudio2024.1"));
        assert!(!matches("AndroidStudio*", "IntelliJ2024.1"));
        assert!(matches("exact", "exact"));
        assert!(!matches("exact", "exacts"));
    }

    #[test]
    fn expansion_only_returns_existing_matches() {
        let temp = TempDir::new("glob");
        temp.mkdir("User Data/Default/Cache");
        temp.mkdir("User Data/Profile 1/Cache");
        temp.mkdir("User Data/Profile 2");

        let mut found = expand(&temp.join("User Data/*/Cache"));
        found.sort();

        assert_eq!(
            found,
            vec![
                temp.join("User Data/Default/Cache"),
                temp.join("User Data/Profile 1/Cache"),
            ]
        );
    }

    #[test]
    fn plain_paths_pass_through_when_they_exist() {
        let temp = TempDir::new("glob-plain");
        let real = temp.mkdir("real");
        assert_eq!(expand(&real), vec![real.clone()]);
        assert!(expand(&temp.join("missing")).is_empty());
    }
}

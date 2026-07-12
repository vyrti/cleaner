//! Pattern matching for temporary files and folders

use crate::config::Config;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

/// Pattern matcher with configurable patterns
pub struct PatternMatcher {
    config: Arc<Config>,
    directories: Vec<CompiledPattern>,
    files: Vec<usize>,
}

enum CompiledPattern {
    Exact(usize),
    Suffix(usize),
}

impl PatternMatcher {
    pub fn new(config: Arc<Config>) -> Self {
        let directories = config
            .directories
            .iter()
            .enumerate()
            .map(|(index, pattern)| {
                if pattern.starts_with('*') {
                    CompiledPattern::Suffix(index)
                } else {
                    CompiledPattern::Exact(index)
                }
            })
            .collect();
        let files = (0..config.files.len()).collect();
        Self {
            config,
            directories,
            files,
        }
    }

    /// Check if a directory name matches any temp directory pattern
    #[inline]
    pub fn is_temp_directory(&self, name: impl AsRef<OsStr>) -> bool {
        let Some(name) = name.as_ref().to_str() else {
            return false;
        };
        for pattern in &self.directories {
            match *pattern {
                CompiledPattern::Exact(index) if name == self.config.directories[index] => {
                    return true;
                }
                CompiledPattern::Suffix(index)
                    if name.ends_with(&self.config.directories[index][1..]) =>
                {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// Check if a file name matches any temp file pattern
    #[inline]
    pub fn is_temp_file(&self, name: impl AsRef<OsStr>) -> bool {
        let Some(name) = name.as_ref().to_str() else {
            return false;
        };
        for &index in &self.files {
            if name.ends_with(self.config.files[index].as_str()) {
                return true;
            }
        }
        false
    }

    /// Check if path component matches any temp pattern
    #[inline]
    #[allow(dead_code)]
    pub fn matches(&self, path: &Path, is_dir: bool) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if is_dir {
                self.is_temp_directory(OsStr::new(name))
            } else {
                self.is_temp_file(OsStr::new(name))
            }
        } else {
            false
        }
    }

    /// Get directory patterns for display
    #[allow(dead_code)]
    pub fn directory_patterns(&self) -> &[String] {
        &self.config.directories
    }

    /// Get file patterns for display
    #[allow(dead_code)]
    pub fn file_patterns(&self) -> &[String] {
        &self.config.files
    }

    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Arc<Config> {
        Arc::new(Config {
            directories: vec![
                ".terraform".to_string(),
                "target".to_string(),
                "node_modules".to_string(),
                "__pycache__".to_string(),
                "*.egg-info".to_string(),
            ],
            files: vec![".DS_Store".to_string(), ".pyc".to_string(), "~".to_string()],
            days: None,
            force: false,
        })
    }

    #[test]
    fn test_temp_directories() {
        let matcher = PatternMatcher::new(test_config());
        assert!(matcher.is_temp_directory(".terraform"));
        assert!(matcher.is_temp_directory("target"));
        assert!(matcher.is_temp_directory("node_modules"));
        assert!(matcher.is_temp_directory("__pycache__"));
        assert!(!matcher.is_temp_directory("src"));
        assert!(!matcher.is_temp_directory("lib"));
    }

    #[test]
    fn test_temp_files() {
        let matcher = PatternMatcher::new(test_config());
        assert!(matcher.is_temp_file(".DS_Store"));
        assert!(matcher.is_temp_file("test.pyc"));
        assert!(matcher.is_temp_file("backup~"));
        assert!(!matcher.is_temp_file("main.rs"));
    }

    #[test]
    fn test_egg_info() {
        let matcher = PatternMatcher::new(test_config());
        assert!(matcher.is_temp_directory("mypackage.egg-info"));
        assert!(!matcher.is_temp_directory("egg-info.mypackage"));
    }

    #[test]
    fn matches_paths_and_exposes_configured_patterns() {
        let matcher = PatternMatcher::new(test_config());
        assert!(matcher.matches(Path::new("some/target"), true));
        assert!(matcher.matches(Path::new("some/module.pyc"), false));
        assert!(!matcher.matches(Path::new("some/main.rs"), false));
        assert_eq!(matcher.directory_patterns()[0], ".terraform");
        assert_eq!(matcher.file_patterns()[0], ".DS_Store");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_does_not_match() {
        use std::os::unix::ffi::OsStrExt;
        let matcher = PatternMatcher::new(test_config());
        let path = Path::new(std::ffi::OsStr::from_bytes(b"\xff"));
        assert!(!matcher.matches(path, false));
    }

    #[test]
    #[ignore = "manual release microbenchmark"]
    fn manual_profile_pattern_lookup() {
        use foldhash::HashSet;
        use std::collections::HashSet as StdHashSet;
        use std::hint::black_box;
        use std::time::Instant;

        let config = test_config();
        let matcher = PatternMatcher::new(Arc::clone(&config));
        let names = ["src", "target", "module.pyc", "package.egg-info"];
        let iterations = 1_000_000;

        let start = Instant::now();
        for index in 0..iterations {
            black_box(matcher.is_temp_directory(names[index % names.len()]));
        }
        let linear = start.elapsed();

        let std_exact: StdHashSet<&str> = config
            .directories
            .iter()
            .filter(|pattern| !pattern.starts_with('*'))
            .map(String::as_str)
            .collect();
        let fold_exact: HashSet<&str> = config
            .directories
            .iter()
            .filter(|pattern| !pattern.starts_with('*'))
            .map(String::as_str)
            .collect();
        let suffixes: Vec<_> = config
            .directories
            .iter()
            .filter_map(|pattern| pattern.strip_prefix('*'))
            .collect();

        let start = Instant::now();
        for index in 0..iterations {
            let name = names[index % names.len()];
            black_box(
                std_exact.contains(name) || suffixes.iter().any(|suffix| name.ends_with(suffix)),
            );
        }
        let std_hash = start.elapsed();

        let start = Instant::now();
        for index in 0..iterations {
            let name = names[index % names.len()];
            black_box(
                fold_exact.contains(name) || suffixes.iter().any(|suffix| name.ends_with(suffix)),
            );
        }
        let fold_hash = start.elapsed();

        println!("pattern lookup: linear={linear:?} std_hash={std_hash:?} foldhash={fold_hash:?}");
    }
}

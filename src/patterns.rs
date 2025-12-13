//! Pattern matching for temporary files and folders

use crate::config::Config;
use std::path::Path;
use std::sync::Arc;

/// Pattern matcher with configurable patterns
pub struct PatternMatcher {
    directories: Vec<String>,
    files: Vec<String>,
}

impl PatternMatcher {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            directories: config.directories.clone(),
            files: config.files.clone(),
        }
    }

    /// Check if a directory name matches any temp directory pattern
    #[inline]
    pub fn is_temp_directory(&self, name: &str) -> bool {
        for pattern in &self.directories {
            if name == pattern {
                return true;
            }
            // Handle wildcard patterns like "*.egg-info"
            if pattern.starts_with('*') {
                let suffix = &pattern[1..];
                if name.ends_with(suffix) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a file name matches any temp file pattern
    #[inline]
    pub fn is_temp_file(&self, name: &str) -> bool {
        for pattern in &self.files {
            if name == pattern {
                return true;
            }
            // Extension/suffix matches
            if pattern.starts_with('.') && name.ends_with(pattern.as_str()) {
                return true;
            }
            // Ends with pattern (like ~ for backup files)
            if name.ends_with(pattern.as_str()) {
                return true;
            }
        }
        false
    }

    /// Check if path component matches any temp pattern
    #[inline]
    pub fn matches(&self, path: &Path, is_dir: bool) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if is_dir {
                self.is_temp_directory(name)
            } else {
                self.is_temp_file(name)
            }
        } else {
            false
        }
    }

    /// Get directory patterns for display
    pub fn directory_patterns(&self) -> &[String] {
        &self.directories
    }

    /// Get file patterns for display
    pub fn file_patterns(&self) -> &[String] {
        &self.files
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
            files: vec![
                ".DS_Store".to_string(),
                ".pyc".to_string(),
                "~".to_string(),
            ],
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
    }
}

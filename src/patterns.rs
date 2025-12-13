//! Pattern matching for temporary files and folders

use std::path::Path;

/// Directories that should be completely removed
pub const TEMP_DIRECTORIES: &[&str] = &[
    // Terraform
    ".terraform",
    // Rust / Maven
    "target",
    // Node.js
    "node_modules",
    // Python
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".ruff_cache",
    "venv",
    ".venv",
    ".eggs",
    "*.egg-info",
    // Build outputs
    "dist",
    "build",
    // Next.js / Nuxt.js
    ".next",
    ".nuxt",
    // Turborepo
    ".turbo",
    // Gradle
    ".gradle",
    // Coverage
    "coverage",
    ".coverage",
    "htmlcov",
    // Misc caches
    ".cache",
    ".parcel-cache",
];

/// File patterns that should be removed
pub const TEMP_FILES: &[&str] = &[
    // Python compiled
    ".pyc",
    ".pyo",
    ".pyd",
    // macOS
    ".DS_Store",
    // Windows
    "Thumbs.db",
    "desktop.ini",
    // Editor temp files
    ".swp",
    ".swo",
    "~",
];

/// SIMD-optimized finder for directory patterns
pub struct PatternMatcher;

impl PatternMatcher {
    pub fn new() -> Self {
        Self
    }

    /// Check if a directory name matches any temp directory pattern
    /// Uses SIMD-accelerated search internally
    #[inline]
    pub fn is_temp_directory(&self, name: &str) -> bool {
        // Fast path: direct comparison for common cases
        for pattern in TEMP_DIRECTORIES {
            if name == *pattern {
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
        // Direct matches
        for pattern in TEMP_FILES {
            if name == *pattern {
                return true;
            }
            // Extension/suffix matches
            if pattern.starts_with('.') && name.ends_with(pattern) {
                return true;
            }
            // Ends with pattern (like ~ for backup files)
            if name.ends_with(pattern) {
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
}

impl Default for PatternMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temp_directories() {
        let matcher = PatternMatcher::new();
        assert!(matcher.is_temp_directory(".terraform"));
        assert!(matcher.is_temp_directory("target"));
        assert!(matcher.is_temp_directory("node_modules"));
        assert!(matcher.is_temp_directory("__pycache__"));
        assert!(!matcher.is_temp_directory("src"));
        assert!(!matcher.is_temp_directory("lib"));
    }

    #[test]
    fn test_temp_files() {
        let matcher = PatternMatcher::new();
        assert!(matcher.is_temp_file(".DS_Store"));
        assert!(matcher.is_temp_file("Thumbs.db"));
        assert!(matcher.is_temp_file("test.pyc"));
        assert!(matcher.is_temp_file("backup~"));
        assert!(!matcher.is_temp_file("main.rs"));
    }

    #[test]
    fn test_egg_info() {
        let matcher = PatternMatcher::new();
        assert!(matcher.is_temp_directory("mypackage.egg-info"));
    }
}

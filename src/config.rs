//! Configuration loading with priority: env vars > config file > defaults

use serde::Deserialize;
use std::path::Path;

/// Default directories to delete
pub const DEFAULT_DIRECTORIES: &[&str] = &[
    ".terraform",
    "target",
    "node_modules",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".ruff_cache",
    "venv",
    ".venv",
    ".eggs",
    "*.egg-info",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".turbo",
    ".gradle",
    "coverage",
    ".coverage",
    "htmlcov",
    ".cache",
    ".parcel-cache",
];

/// Default file patterns to delete
pub const DEFAULT_FILES: &[&str] = &[
    ".pyc",
    ".pyo",
    ".pyd",
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    ".swp",
    ".swo",
    "~",
];

/// Configuration file structure
#[derive(Debug, Deserialize, Default)]
pub struct ConfigFile {
    #[serde(default)]
    pub patterns: PatternsConfig,
    pub days: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PatternsConfig {
    #[serde(default)]
    pub directories: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

/// Runtime configuration
#[derive(Debug, Clone)]
pub struct Config {
    pub directories: Vec<String>,
    pub files: Vec<String>,
    pub days: Option<u64>,
    pub force: bool,
}

impl Config {
    pub fn try_load(config_path: Option<&Path>) -> Result<Self, String> {
        if let Some(path) = config_path {
            let content = std::fs::read_to_string(path)
                .map_err(|error| format!("Cannot read config {}: {error}", path.display()))?;
            toml::from_str::<ConfigFile>(&content)
                .map_err(|error| format!("Invalid config {}: {error}", path.display()))?;
        }
        Ok(Self::load(config_path))
    }

    /// Load configuration with priority: env vars > config file > defaults
    pub fn load(config_path: Option<&Path>) -> Self {
        Self::load_with_env(config_path, |name| std::env::var(name).ok())
    }

    fn load_with_env<F>(config_path: Option<&Path>, mut env: F) -> Self
    where
        F: FnMut(&str) -> Option<String>,
    {
        // Start with defaults
        let mut directories: Vec<String> =
            DEFAULT_DIRECTORIES.iter().map(|s| s.to_string()).collect();
        let mut files: Vec<String> = DEFAULT_FILES.iter().map(|s| s.to_string()).collect();
        let mut days = None;

        // Override with config file if provided
        if let Some(path) = config_path {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(config) = toml::from_str::<ConfigFile>(&content) {
                    if !config.patterns.directories.is_empty() {
                        directories = config.patterns.directories;
                    }
                    if !config.patterns.files.is_empty() {
                        files = config.patterns.files;
                    }
                    if config.days.is_some() {
                        days = config.days;
                    }
                }
            }
        }

        // Override with environment variables (highest priority)
        if let Some(env_dirs) = env("CLEANER_DIRS") {
            directories = env_dirs.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Some(env_files) = env("CLEANER_FILES") {
            files = env_files.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Some(env_days) = env("CLEANER_DAYS") {
            if let Ok(d) = env_days.parse() {
                days = Some(d);
            }
        }

        Self {
            directories,
            files,
            days,
            force: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::load(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_contain_expected_patterns() {
        let config = Config::load_with_env(None, no_env);
        assert_eq!(config.directories.len(), DEFAULT_DIRECTORIES.len());
        assert_eq!(config.files.len(), DEFAULT_FILES.len());
        assert!(config.directories.iter().any(|pattern| pattern == "target"));
        assert!(config.files.iter().any(|pattern| pattern == ".pyc"));
        assert_eq!(config.days, None);
        assert!(!config.force);
    }

    #[test]
    fn valid_file_overrides_nonempty_values() {
        let temp = TempDir::new("config-valid");
        let path = temp.write(
            "cleaner.toml",
            b"days = 7\n[patterns]\ndirectories = ['cache-dir']\nfiles = ['.tmp']\n",
        );
        let config = Config::load_with_env(Some(&path), no_env);
        assert_eq!(config.directories, ["cache-dir"]);
        assert_eq!(config.files, [".tmp"]);
        assert_eq!(config.days, Some(7));
    }

    #[test]
    fn empty_or_invalid_file_values_fall_back_to_defaults() {
        let temp = TempDir::new("config-fallback");
        let empty = temp.write("empty.toml", b"[patterns]\ndirectories = []\nfiles = []\n");
        let config = Config::load_with_env(Some(&empty), no_env);
        assert_eq!(config.directories.len(), DEFAULT_DIRECTORIES.len());

        let invalid = temp.write("invalid.toml", b"this is not = valid toml [");
        let config = Config::load_with_env(Some(&invalid), no_env);
        assert_eq!(config.files.len(), DEFAULT_FILES.len());
    }

    #[test]
    fn environment_has_highest_priority_and_trims_lists() {
        let temp = TempDir::new("config-env");
        let path = temp.write(
            "cleaner.toml",
            b"days = 7\n[patterns]\ndirectories = ['from-file']\nfiles = ['.file']\n",
        );
        let config = Config::load_with_env(Some(&path), |name| match name {
            "CLEANER_DIRS" => Some(" one, two ".into()),
            "CLEANER_FILES" => Some(".log, ~".into()),
            "CLEANER_DAYS" => Some("30".into()),
            _ => None,
        });
        assert_eq!(config.directories, ["one", "two"]);
        assert_eq!(config.files, [".log", "~"]);
        assert_eq!(config.days, Some(30));
    }

    #[test]
    fn invalid_environment_days_does_not_replace_file_value() {
        let temp = TempDir::new("config-env-invalid");
        let path = temp.write("cleaner.toml", b"days = 4\n");
        let config = Config::load_with_env(Some(&path), |name| {
            (name == "CLEANER_DAYS").then(|| "not-a-number".into())
        });
        assert_eq!(config.days, Some(4));
    }

    #[test]
    fn try_load_reports_missing_and_invalid_files() {
        let temp = TempDir::new("config-errors");
        assert!(Config::try_load(Some(&temp.join("missing.toml")))
            .unwrap_err()
            .contains("Cannot read config"));
        let invalid = temp.write("invalid.toml", b"not valid = [");
        assert!(Config::try_load(Some(&invalid))
            .unwrap_err()
            .contains("Invalid config"));
    }
}

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
}

impl Config {
    /// Load configuration with priority: env vars > config file > defaults
    pub fn load(config_path: Option<&Path>) -> Self {
        // Start with defaults
        let mut directories: Vec<String> = DEFAULT_DIRECTORIES.iter().map(|s| s.to_string()).collect();
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
        if let Ok(env_dirs) = std::env::var("CLEANER_DIRS") {
            directories = env_dirs.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(env_files) = std::env::var("CLEANER_FILES") {
            files = env_files.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(env_days) = std::env::var("CLEANER_DAYS") {
            if let Ok(d) = env_days.parse() {
                days = Some(d);
            }
        }

        Self {
            directories,
            files,
            days,
        }
    }

    /// Get directories as slice of str references
    pub fn directories(&self) -> Vec<&str> {
        self.directories.iter().map(|s| s.as_str()).collect()
    }

    /// Get files as slice of str references
    pub fn files(&self) -> Vec<&str> {
        self.files.iter().map(|s| s.as_str()).collect()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::load(None)
    }
}

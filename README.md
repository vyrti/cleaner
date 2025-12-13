# Cleaner

[![Build](https://github.com/vyrti/cleaner/actions/workflows/build.yml/badge.svg)](https://github.com/vyrti/cleaner/actions/workflows/build.yml)
[![Release](https://github.com/vyrti/cleaner/actions/workflows/release.yml/badge.svg)](https://github.com/vyrti/cleaner/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

High-performance CLI tool for cleaning development temp files. Scans directories in parallel and removes `.terraform`, `target`, `node_modules`, `__pycache__`, and other common development artifacts.

## Features

- **Fast** - Multi-threaded scanning with [jwalk](https://crates.io/crates/jwalk) and parallel deletion with [rayon](https://crates.io/crates/rayon)
- **Interactive** - ncdu-style TUI for browsing and deleting temp folders (`-i` flag)
- **Configurable** - TOML config file + environment variable overrides
- **Safe** - Dry-run mode, time-based filtering (--days)
- **Cross-platform** - Windows, Linux, macOS, FreeBSD | ARM and x64

## Installation

### From Releases

Download the latest binary from [Releases](https://github.com/vyrti/cleaner/releases).

### From Source

```bash
cargo install --git https://github.com/vyrti/cleaner
```

### Docker

```bash
docker pull ghcr.io/vyrti/cleaner:latest
```

## Usage

```bash
# Interactive TUI mode (ncdu-like)
cleaner -f ~/Projects -i

# Preview what would be deleted (safe)
cleaner -f ~/Projects --dry-run

# Delete temp files
cleaner -f ~/Projects

# Filter by age (safe mode for active projects)
cleaner -f ~/Projects --days 7
```

### Options

| Flag | Description |
|------|-------------|
| `-f, --folder` | Target folder to scan (required) |
| `-i, --interactive` | Interactive TUI mode |
| `-d, --dry-run` | Preview without deleting |
| `-v, --verbose` | Show all matched paths |
| `-c, --config` | Path to TOML config file |
| `-j, --threads` | Number of threads (default: CPU cores) |
| `--days` | Only delete items older than N days |

## Configuration

### Config File

Create `cleaner.toml`:

```toml
[patterns]
directories = [
    ".terraform",
    "target",
    "node_modules",
    "__pycache__",
]

files = [
    ".DS_Store",
    "*.pyc",
]

# Optional: Only delete items older than 30 days
days = 30
```

See [cleaner.toml.example](cleaner.toml.example) for all defaults.

### Environment Variables

Override config with environment variables:

```bash
CLEANER_DIRS=".terraform,target" cleaner -f ~/Projects
CLEANER_FILES=".DS_Store,*.pyc" cleaner -f ~/Projects
```

**Priority:** Environment variables > Config file > Defaults

## Default Patterns

### Directories
`.terraform`, `target`, `node_modules`, `__pycache__`, `.pytest_cache`, `.mypy_cache`, `.tox`, `.ruff_cache`, `venv`, `.venv`, `.eggs`, `*.egg-info`, `dist`, `build`, `.next`, `.nuxt`, `.turbo`, `.gradle`, `coverage`, `.coverage`, `htmlcov`, `.cache`, `.parcel-cache`

### Files
`.pyc`, `.pyo`, `.pyd`, `.DS_Store`, `Thumbs.db`, `desktop.ini`, `.swp`, `.swo`, `~`

## Docker Usage

```bash
# Mount directory and run
docker run -v /path/to/scan:/data ghcr.io/vyrti/cleaner -f /data --dry-run

# With env vars
docker run -e CLEANER_DIRS=".terraform,target" -v /path:/data ghcr.io/vyrti/cleaner -f /data
```

## License

[Apache 2.0](LICENSE)

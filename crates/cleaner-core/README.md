# cleaner-core

[![Crates.io](https://img.shields.io/crates/v/cleaner-core.svg)](https://crates.io/crates/cleaner-core)
[![Documentation](https://docs.rs/cleaner-core/badge.svg)](https://docs.rs/cleaner-core)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

Reusable, high-performance disk scanning, directory tree sizing, and temporary artifact cleaning engine for Rust.

## Features

- **Blazing Fast Scanning**: Highly parallel filesystem traversal using Rayon worker pools.
- **Platform-Native Traversal**: Optimized batched syscalls (such as `getattrlistbulk` on macOS).
- **Single-Pass Sizing & Indexing**: Calculates recursive directory sizes in-place with minimal heap allocations.
- **Configurable Pattern Matching**: Fast precompiled matching for development artifacts (`node_modules`, `target`, `__pycache__`, etc.).
- **System Protection**: Built-in protection boundaries that prevent destructive actions on critical system folders and toolchains.
- **Cross-Platform**: Full support for Linux, macOS, Windows, and FreeBSD.

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
cleaner-core = "0.1"
```

### Basic Example

```rust,no_run
use cleaner_core::config::Config;
use cleaner_core::patterns::PatternMatcher;
use cleaner_core::tree::{DirTree, ScanProgress};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() {
    let root = Path::new("/path/to/scan");
    let config = Arc::new(Config::default());
    let matcher = PatternMatcher::new(Arc::clone(&config));
    let progress = Arc::new(ScanProgress::new());
    let cancelled = Arc::new(AtomicBool::new(false));

    // Build the directory tree with sizes and pattern flags in a single pass
    let mut tree = DirTree::build_with_progress(
        root,
        &matcher,
        progress,
        cancelled,
        false, // force
    );

    // Fetch sorted entries for the root directory
    let entries = tree.get_children(root, false); // false = sort by size
    for entry in entries.iter() {
        println!("{}: {} bytes (temp: {})", entry.name.to_string_lossy(), entry.size, entry.is_temp);
    }
}
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).

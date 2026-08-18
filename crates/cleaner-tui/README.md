# cleaner-tui

[![Crates.io](https://img.shields.io/crates/v/cleaner-tui.svg)](https://crates.io/crates/cleaner-tui)
[![Documentation](https://docs.rs/cleaner-tui/badge.svg)](https://docs.rs/cleaner-tui)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

Fastest disk scanner and temporary development file cleaner with an interactive Norton/ncdu-style TUI.

Provides both the standalone `cleaner` CLI/TUI application and an embeddable `cleaner_tui` library interface for terminal applications (such as [Abyss](https://github.com/vyrti/abyss)).

## Installation

### Cargo

```bash
cargo install cleaner-tui --locked
```

## CLI Usage

```bash
# Launch interactive TUI mode starting in the home directory
cleaner

# Launch interactive TUI mode starting in a specific folder
cleaner ~/Projects

# Run non-interactive CLI dry-run mode (JSON output)
cleaner ~/Projects --json

# Run non-interactive live deletion (permanently removes detected dev artifacts)
cleaner ~/Projects --confirm
```

## Library Usage

To embed the interactive disk analyzer inside your Ratatui application:

```rust,no_run
use cleaner_tui::{Chrome, CleanOffer, Outcome, Session, StartOpts};
use cleaner_core::config::Config;
use crossterm::event::Event;
use ratatui::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let root = PathBuf::from(".");
    let config = Arc::new(Config::default());
    let mut session = Session::start(root, config, StartOpts::default());

    // In your main event loop:
    session.tick();

    // Render into a ratatui frame:
    // session.draw(frame, area, Chrome::Full);

    // Forward terminal events:
    // if let Outcome::Exit = session.handle_event(event) { ... }
}
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).

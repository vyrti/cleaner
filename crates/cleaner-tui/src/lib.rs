//! Interactive TUI interface and CLI binary for Cleaner.
//!
//! Provides the standalone `cleaner` CLI application and embeddable `Session`
//! component for host applications (such as Abyss).

pub mod app;
pub mod cli;
mod events;
pub mod session;
pub mod ui;

pub use app::App;
pub use cleaner_core::config::Config;
pub use session::{run, CleanOffer, Outcome, Session, StartOpts};
pub use ui::Chrome;

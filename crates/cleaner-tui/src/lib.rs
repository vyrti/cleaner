//! Cleaner interactive TUI library (embeddable Session + standalone run).

mod app;
mod events;
#[cfg(target_os = "macos")]
mod index;
mod session;
mod ui;

pub use app::App;
pub use cleaner_core::config::Config;
pub use session::{run, CleanOffer, Outcome, Session, StartOpts};
pub use ui::Chrome;

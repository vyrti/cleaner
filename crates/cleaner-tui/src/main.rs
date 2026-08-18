//! High-performance folder cleaner binary entry point

#[cfg(all(
    feature = "mimalloc-allocator",
    not(feature = "system-allocator"),
    not(target_os = "macos")
))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    cleaner_tui::cli::run();
}

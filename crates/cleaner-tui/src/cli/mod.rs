//! Command-line interface orchestration for batch scanning, reporting, and TUI launch.

mod args;
mod json;
mod reporter;

#[cfg(test)]
mod tests;

pub use args::{parse_thread_count, resolve_folder, Args};
pub use json::{json_escape_path, output_json_error, output_json_results, JsonResults};
pub use reporter::run_cli_scan;

use clap::Parser;
use cleaner_core::config::Config;
use cleaner_core::pool;
use colored::Colorize;
use std::sync::Arc;

pub fn run() {
    let args = Args::parse();
    let is_interactive = !args.json && !args.confirm;

    // Resolve folder: positional > --folder > home directory
    let folder = resolve_folder(&args);

    // Validate folder exists
    if !folder.exists() {
        if args.json {
            output_json_error(&format!("Folder does not exist: {}", folder.display()));
        } else {
            eprintln!(
                "{} Folder does not exist: {}",
                "Error:".red().bold(),
                folder.display()
            );
        }
        std::process::exit(1);
    }

    if !folder.is_dir() {
        if args.json {
            output_json_error(&format!("Path is not a directory: {}", folder.display()));
        } else {
            eprintln!(
                "{} Path is not a directory: {}",
                "Error:".red().bold(),
                folder.display()
            );
        }
        std::process::exit(1);
    }

    // Get absolute path
    let folder = folder.canonicalize().unwrap_or(folder);

    // Load configuration (priority: env vars > config file > defaults)
    let mut config = match Config::try_load(args.config.as_deref()) {
        Ok(config) => config,
        Err(error) => {
            if args.json {
                output_json_error(&error);
            } else {
                eprintln!("{} {}", "Error:".red().bold(), error);
            }
            std::process::exit(1);
        }
    };

    // CLI args override config
    if let Some(days) = args.days {
        config.days = Some(days);
    }
    config.force = args.force;

    let config = Arc::new(config);

    // Determine and configure worker count before any lazy global pool starts.
    let num_threads = args.threads.unwrap_or_else(pool::default_thread_count);
    pool::configure_scan_pool(num_threads);

    // Interactive TUI mode by default when run without folder/path arguments
    if is_interactive {
        if let Err(e) = crate::session::run(folder, config, args.index, args.rebuild_index) {
            eprintln!("{} TUI error: {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
        return;
    }

    run_cli_scan(&args, &folder, config, num_threads);
}

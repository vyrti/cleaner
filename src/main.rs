//! High-performance folder cleaner for development temp files
//!
//! Scans directories using multi-threaded jwalk and removes common
//! development temporary files and folders like .terraform, target,
//! node_modules, __pycache__, etc.

mod config;
mod deleter;
mod fastwalk;
mod patterns;
mod pool;
mod scanner;
mod stats;
#[cfg(test)]
mod test_support;
mod tui;

#[cfg(all(
    feature = "mimalloc-allocator",
    not(feature = "system-allocator"),
    not(target_os = "macos")
))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use clap::Parser;
use colored::Colorize;
use config::Config;
use crossbeam_channel::bounded;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

fn parse_thread_count(value: &str) -> Result<usize, String> {
    let count = value
        .parse::<usize>()
        .map_err(|_| "threads must be a positive integer".to_string())?;
    if !(1..=pool::MAX_WORKER_THREADS).contains(&count) {
        return Err(format!(
            "threads must be between 1 and {}",
            pool::MAX_WORKER_THREADS
        ));
    }
    Ok(count)
}

/// High-performance folder cleaner for development temp files
#[derive(Parser, Debug)]
#[command(name = "cleaner")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Target folder to scan (positional or use -f/--folder)
    #[arg(index = 1)]
    path: Option<PathBuf>,

    /// Target folder to scan (alternative to positional)
    #[arg(short = 'f', long = "folder")]
    folder: Option<PathBuf>,

    /// Path to TOML config file
    #[arg(short = 'c', long = "config")]
    config: Option<PathBuf>,

    /// Confirm deletion (live run) - actually delete files instead of dry-run
    #[arg(short = 'y', long = "confirm", default_value = "false")]
    confirm: bool,

    /// Verbose output - show all matched paths
    #[arg(short = 'v', long = "verbose", default_value = "false")]
    verbose: bool,

    /// Number of threads for scanning and deletion (default: number of CPU cores)
    #[arg(short = 'j', long = "threads", value_parser = parse_thread_count)]
    threads: Option<usize>,

    /// Filter by modification time (only delete items older than N days)
    #[arg(long = "days")]
    days: Option<u64>,

    /// Output results in JSON format (scripting/devops mode)
    #[arg(long = "json", default_value = "false")]
    json: bool,

    /// Force deletion inside protected system directories
    #[arg(long = "force", default_value = "false")]
    force: bool,

    /// Use the persistent macOS TUI filesystem index
    #[arg(long = "index", default_value = "false")]
    index: bool,

    /// Rebuild the persistent macOS TUI filesystem index
    #[arg(long = "rebuild-index", default_value = "false")]
    rebuild_index: bool,
}

fn resolve_folder(args: &Args) -> PathBuf {
    args.path
        .clone()
        .or_else(|| args.folder.clone())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn json_escape_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn main() {
    let args = Args::parse();

    let is_interactive = !args.json && !args.confirm;
    let index_requested = args.index || args.rebuild_index;

    if index_requested && !is_interactive {
        eprintln!(
            "{} --index is only valid in interactive TUI mode",
            "Error:".red().bold()
        );
        std::process::exit(2);
    }
    #[cfg(not(target_os = "macos"))]
    if index_requested {
        eprintln!(
            "{} --index is only available on macOS",
            "Error:".red().bold()
        );
        std::process::exit(2);
    }

    // Resolve folder: positional > --folder > home directory
    let folder = resolve_folder(&args);

    // Validate folder exists
    if !folder.exists() {
        if args.json {
            println!(
                "{{\"success\":false,\"error\":\"Folder does not exist: {}\"}}",
                json_escape_path(&folder)
            );
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
            println!(
                "{{\"success\":false,\"error\":\"Path is not a directory: {}\"}}",
                json_escape_path(&folder)
            );
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
                println!(
                    "{{\"success\":false,\"error\":\"{}\"}}",
                    error.replace('\\', "\\\\").replace('"', "\\\"")
                );
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
        if let Err(e) = tui::run(folder, config, index_requested, args.rebuild_index) {
            eprintln!("{} TUI error: {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
        return;
    }

    if !args.json {
        // Print header
        println!();
        println!(
            "{}",
            "╔══════════════════════════════════════════════════════════════╗"
                .bright_cyan()
                .bold()
        );
        println!(
            "{}",
            "║                    FOLDER CLEANER v0.1.0                     ║"
                .bright_cyan()
                .bold()
        );
        println!(
            "{}",
            "╚══════════════════════════════════════════════════════════════╝"
                .bright_cyan()
                .bold()
        );
        println!();

        if !args.confirm {
            println!(
                "  {} {}",
                "Mode:".bright_yellow().bold(),
                "DRY RUN (no files will be deleted)".yellow()
            );
        } else {
            println!(
                "  {} {}",
                "Mode:".bright_red().bold(),
                "LIVE (files will be permanently deleted!)".red()
            );
        }

        println!("  {} {}", "Target:".bright_white().bold(), folder.display());

        if let Some(ref config_path) = args.config {
            println!(
                "  {} {}",
                "Config:".bright_white().bold(),
                config_path.display()
            );
        }

        println!("  {} {}", "Threads:".bright_white().bold(), num_threads);

        if let Some(days) = config.days {
            println!(
                "  {} {} days (items modified within this time are safe)",
                "Filter:".bright_white().bold(),
                days
            );
        }

        println!();

        // Show patterns being matched
        println!("  {} ", "Patterns:".bright_white().bold());
        println!(
            "    {} {}",
            "Directories:".dimmed(),
            config.directories.join(", ").dimmed()
        );
        println!(
            "    {} {}",
            "Files:".dimmed(),
            config.files.join(", ").dimmed()
        );
        println!();
    }

    // Create shared stats
    let stats = Arc::new(stats::Stats::new());

    // Create channel for scan results
    let (tx, rx) = bounded(1024);

    // Start timer
    let start = Instant::now();

    // Create progress bar
    let pb = if !args.json {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message("Scanning directories...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(pb)
    } else {
        None
    };

    // Start scanner in separate thread
    let worker_pool = pool::build_worker_pool(num_threads, "cleaner-worker");
    let scanner = scanner::Scanner::with_pool(
        folder.clone(),
        Arc::clone(&worker_pool),
        Arc::clone(&config),
    );
    let scan_handle = thread::spawn(move || scanner.scan(tx));

    // Create deleter
    let deleter = deleter::Deleter::with_pool(
        Arc::clone(&stats),
        !args.confirm,
        args.verbose && !args.json,
        worker_pool,
    );

    // Process deletions (this blocks until scanner finishes and channel closes)
    deleter.process(rx);

    // Wait for scanner to complete
    let scan_summary = scan_handle.join().unwrap();
    stats.add_errors(scan_summary.errors);
    let scanned_count = scan_summary.entries;

    // Stop progress bar
    if let Some(ref p) = pb {
        p.finish_and_clear();
    }

    let elapsed = start.elapsed();

    if args.json {
        let mode = if !args.confirm { "dry-run" } else { "live" };
        println!(
            "{{\"success\":true,\"mode\":\"{}\",\"target\":\"{}\",\"scanned_entries\":{},\"time_ms\":{},\"deleted_directories\":{},\"deleted_files\":{},\"bytes_freed\":{},\"errors\":{}}}",
            mode,
            json_escape_path(&folder),
            scanned_count,
            elapsed.as_millis(),
            stats.directories(),
            stats.files(),
            stats.bytes(),
            stats.error_count()
        );
        return;
    }

    // Print results
    println!();
    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════".bright_cyan()
    );
    println!("  {}", "Results:".bright_green().bold());
    println!();

    if !args.confirm {
        println!(
            "    {} {} directories",
            "Would delete:".yellow(),
            stats.directories()
        );
        println!("    {} {} files", "Would delete:".yellow(), stats.files());
        println!(
            "    {} {}",
            "Would free:".yellow(),
            humansize::format_size(stats.bytes(), humansize::BINARY)
        );
    } else {
        println!(
            "    {} {} directories",
            "Deleted:".green(),
            stats.directories()
        );
        println!("    {} {} files", "Deleted:".green(), stats.files());
        println!(
            "    {} {}",
            "Freed:".green(),
            humansize::format_size(stats.bytes(), humansize::BINARY)
        );
    }

    if stats.error_count() > 0 {
        println!(
            "    {} {} (permission denied or in use)",
            "Errors:".red(),
            stats.error_count()
        );
    }

    println!();
    println!(
        "    {} {} entries in {:.2?}",
        "Scanned:".dimmed(),
        scanned_count,
        elapsed
    );
    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════".bright_cyan()
    );
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_options() {
        let args = Args::try_parse_from([
            "cleaner",
            "somewhere",
            "--folder",
            "fallback",
            "--config",
            "config.toml",
            "--confirm",
            "--verbose",
            "--threads",
            "4",
            "--days",
            "30",
            "--json",
            "--force",
        ])
        .unwrap();
        assert_eq!(args.path, Some(PathBuf::from("somewhere")));
        assert_eq!(args.folder, Some(PathBuf::from("fallback")));
        assert_eq!(args.config, Some(PathBuf::from("config.toml")));
        assert!(args.confirm && args.verbose && args.json && args.force);
        assert_eq!(args.threads, Some(4));
        assert_eq!(args.days, Some(30));
    }

    #[test]
    fn positional_folder_takes_priority() {
        let args = Args::try_parse_from(["cleaner", "positional", "--folder", "option"]).unwrap();
        assert_eq!(resolve_folder(&args), PathBuf::from("positional"));
        let args = Args::try_parse_from(["cleaner", "--folder", "option"]).unwrap();
        assert_eq!(resolve_folder(&args), PathBuf::from("option"));
    }

    #[test]
    fn escapes_paths_for_json_strings() {
        assert_eq!(
            json_escape_path(std::path::Path::new("a\\b\"c")),
            "a\\\\b\\\"c"
        );
    }

    #[test]
    fn rejects_zero_and_excessive_thread_counts() {
        assert!(Args::try_parse_from(["cleaner", "--threads", "0"]).is_err());
        assert!(Args::try_parse_from(["cleaner", "--threads", "257"]).is_err());
    }
}

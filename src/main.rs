//! High-performance folder cleaner for development temp files
//!
//! Scans directories using multi-threaded jwalk and removes common
//! development temporary files and folders like .terraform, target,
//! node_modules, __pycache__, etc.

mod config;
mod deleter;
mod patterns;
mod scanner;
mod stats;
mod tui;

use clap::Parser;
use colored::Colorize;
use config::Config;
use crossbeam_channel::unbounded;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

/// High-performance folder cleaner for development temp files
#[derive(Parser, Debug)]
#[command(name = "cleaner")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Target folder to scan
    #[arg(short = 'f', long = "folder", required = true)]
    folder: PathBuf,

    /// Path to TOML config file
    #[arg(short = 'c', long = "config")]
    config: Option<PathBuf>,

    /// Dry run - show what would be deleted without actually deleting
    #[arg(short = 'd', long = "dry-run", default_value = "false")]
    dry_run: bool,

    /// Verbose output - show all matched paths
    #[arg(short = 'v', long = "verbose", default_value = "false")]
    verbose: bool,

    /// Number of threads for scanning and deletion (default: number of CPU cores)
    #[arg(short = 'j', long = "threads")]
    threads: Option<usize>,

    /// Filter by modification time (only delete items older than N days)
    #[arg(long = "days")]
    days: Option<u64>,

    /// Interactive TUI mode (ncdu-like)
    #[arg(short = 'i', long = "interactive")]
    interactive: bool,
}

fn main() {
    let args = Args::parse();

    // Validate folder exists
    if !args.folder.exists() {
        eprintln!(
            "{} Folder does not exist: {}",
            "Error:".red().bold(),
            args.folder.display()
        );
        std::process::exit(1);
    }

    if !args.folder.is_dir() {
        eprintln!(
            "{} Path is not a directory: {}",
            "Error:".red().bold(),
            args.folder.display()
        );
        std::process::exit(1);
    }

    // Get absolute path
    let folder = args.folder.canonicalize().unwrap_or(args.folder);

    // Load configuration (priority: env vars > config file > defaults)
    let mut config = Config::load(args.config.as_deref());
    
    // CLI args override config
    if let Some(days) = args.days {
        config.days = Some(days);
    }
    
    let config = Arc::new(config);

    // Interactive TUI mode
    if args.interactive {
        if let Err(e) = tui::run(folder, config) {
            eprintln!("{} TUI error: {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
        return;
    }

    // Determine thread count
    let num_threads = args.threads.unwrap_or_else(num_cpus::get);

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

    if args.dry_run {
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

    println!(
        "  {} {}",
        "Target:".bright_white().bold(),
        folder.display()
    );

    if let Some(ref config_path) = args.config {
        println!(
            "  {} {}",
            "Config:".bright_white().bold(),
            config_path.display()
        );
    }

    println!(
        "  {} {}",
        "Threads:".bright_white().bold(),
        num_threads
    );

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

    // Create shared stats
    let stats = Arc::new(stats::Stats::new());

    // Create channel for scan results
    let (tx, rx) = unbounded();

    // Start timer
    let start = Instant::now();

    // Create progress bar
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    pb.set_message("Scanning directories...");
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    // Start scanner in separate thread
    let scanner = scanner::Scanner::new(folder.clone(), num_threads, Arc::clone(&config));
    let scan_handle = thread::spawn(move || {
        let count = scanner.scan(tx);
        count
    });

    // Create deleter
    let deleter = deleter::Deleter::new(Arc::clone(&stats), args.dry_run, args.verbose);

    // Process deletions (this blocks until scanner finishes and channel closes)
    deleter.process(rx);

    // Wait for scanner to complete
    let scanned_count = scan_handle.join().unwrap();

    // Stop progress bar
    pb.finish_and_clear();

    let elapsed = start.elapsed();

    // Print results
    println!();
    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════"
            .bright_cyan()
    );
    println!("  {}", "Results:".bright_green().bold());
    println!();

    if args.dry_run {
        println!(
            "    {} {} directories",
            "Would delete:".yellow(),
            stats.directories()
        );
        println!(
            "    {} {} files",
            "Would delete:".yellow(),
            stats.files()
        );
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
        "═══════════════════════════════════════════════════════════════"
            .bright_cyan()
    );
    println!();
}

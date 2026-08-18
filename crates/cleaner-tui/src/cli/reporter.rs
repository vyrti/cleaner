use super::args::Args;
use super::json::output_json_results;
use cleaner_core::config::Config;
use cleaner_core::deleter::Deleter;
use cleaner_core::pool;
use cleaner_core::scanner::Scanner;
use cleaner_core::stats::Stats;
use colored::Colorize;
use crossbeam_channel::bounded;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

pub fn print_banner(args: &Args, folder: &Path, config: &Config, num_threads: usize) {
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

pub fn run_cli_scan(args: &Args, folder: &Path, config: Arc<Config>, num_threads: usize) {
    if !args.json {
        print_banner(args, folder, &config, num_threads);
    }

    let stats = Arc::new(Stats::new());
    let (tx, rx) = bounded(1024);
    let start = Instant::now();

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

    let worker_pool = pool::build_worker_pool(num_threads, "cleaner-worker");
    let scanner = Scanner::with_pool(
        folder.to_path_buf(),
        Arc::clone(&worker_pool),
        Arc::clone(&config),
    );
    let scan_handle = thread::spawn(move || scanner.scan(tx));

    let deleter = Deleter::with_pool(
        Arc::clone(&stats),
        !args.confirm,
        args.verbose && !args.json,
        worker_pool,
    );

    deleter.process(rx);

    let scan_summary = scan_handle.join().unwrap();
    stats.add_errors(scan_summary.errors);
    let scanned_count = scan_summary.entries;

    if let Some(ref p) = pb {
        p.finish_and_clear();
    }

    let elapsed = start.elapsed();

    if args.json {
        output_json_results(&super::json::JsonResults {
            confirm: args.confirm,
            folder,
            scanned_count,
            elapsed_ms: elapsed.as_millis(),
            directories: stats.directories(),
            files: stats.files(),
            bytes: stats.bytes(),
            errors: stats.error_count(),
        });
        return;
    }

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

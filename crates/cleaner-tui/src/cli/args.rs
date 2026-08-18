use clap::Parser;
use cleaner_core::pool;
use std::path::PathBuf;

pub fn parse_thread_count(value: &str) -> Result<usize, String> {
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
#[command(author, version, about = "Fastest disk scanner and cleaner", long_about = None)]
pub struct Args {
    /// Target folder to scan (positional or use -f/--folder)
    #[arg(index = 1)]
    pub path: Option<PathBuf>,

    /// Target folder to scan (alternative to positional)
    #[arg(short = 'f', long = "folder")]
    pub folder: Option<PathBuf>,

    /// Path to TOML config file
    #[arg(short = 'c', long = "config")]
    pub config: Option<PathBuf>,

    /// Confirm deletion (live run) - actually delete files instead of dry-run
    #[arg(short = 'y', long = "confirm", default_value = "false")]
    pub confirm: bool,

    /// Verbose output - show all matched paths
    #[arg(short = 'v', long = "verbose", default_value = "false")]
    pub verbose: bool,

    /// Number of threads for scanning and deletion (default: number of CPU cores)
    #[arg(short = 'j', long = "threads", value_parser = parse_thread_count)]
    pub threads: Option<usize>,

    /// Filter by modification time (only delete items older than N days)
    #[arg(long = "days")]
    pub days: Option<u64>,

    /// Output results in JSON format (scripting/devops mode)
    #[arg(long = "json", default_value = "false")]
    pub json: bool,

    /// Force deletion inside protected system directories
    #[arg(long = "force", default_value = "false")]
    pub force: bool,

    /// Optional legacy index flag (ignored)
    #[arg(long = "index", default_value = "false")]
    pub index: bool,

    /// Optional legacy rebuild-index flag (ignored)
    #[arg(long = "rebuild-index", default_value = "false")]
    pub rebuild_index: bool,
}

pub fn resolve_folder(args: &Args) -> PathBuf {
    args.path
        .clone()
        .or_else(|| args.folder.clone())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

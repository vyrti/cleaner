use super::args::{parse_thread_count, resolve_folder, Args};
use super::json::json_escape_path;
use clap::Parser;
use std::path::{Path, PathBuf};

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
    assert_eq!(json_escape_path(Path::new("a\\b\"c")), "a\\\\b\\\"c");
}

#[test]
fn rejects_zero_and_excessive_thread_counts() {
    assert!(Args::try_parse_from(["cleaner", "--threads", "0"]).is_err());
    assert!(Args::try_parse_from(["cleaner", "--threads", "257"]).is_err());
    assert_eq!(parse_thread_count("8").unwrap(), 8);
    assert!(parse_thread_count("invalid").is_err());
}

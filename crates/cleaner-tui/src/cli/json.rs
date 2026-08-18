use std::path::Path;

pub fn json_escape_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

pub fn output_json_error(message: &str) {
    println!(
        "{{\"success\":false,\"error\":\"{}\"}}",
        message.replace('\\', "\\\\").replace('"', "\\\"")
    );
}

pub struct JsonResults<'a> {
    pub confirm: bool,
    pub folder: &'a Path,
    pub scanned_count: usize,
    pub elapsed_ms: u128,
    pub directories: usize,
    pub files: usize,
    pub bytes: u64,
    pub errors: usize,
}

pub fn output_json_results(results: &JsonResults) {
    let mode = if !results.confirm { "dry-run" } else { "live" };
    println!(
        "{{\"success\":true,\"mode\":\"{}\",\"target\":\"{}\",\"scanned_entries\":{},\"time_ms\":{},\"deleted_directories\":{},\"deleted_files\":{},\"bytes_freed\":{},\"errors\":{}}}",
        mode,
        json_escape_path(results.folder),
        results.scanned_count,
        results.elapsed_ms,
        results.directories,
        results.files,
        results.bytes,
        results.errors
    );
}

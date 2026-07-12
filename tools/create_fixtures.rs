//! Deterministic, dependency-free filesystem fixtures for manual profiling.
//! Compile with: rustc -O tools/create_fixtures.rs -o target/create-fixtures

use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn write_file(path: &Path, bytes: usize) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    let block = [b'x'; 4096];
    let mut remaining = bytes;
    while remaining > 0 {
        let count = remaining.min(block.len());
        file.write_all(&block[..count])?;
        remaining -= count;
    }
    Ok(())
}

fn create(root: &Path, scale: usize) -> io::Result<()> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    fs::create_dir_all(root)?;

    for index in 0..scale {
        write_file(
            &root.join("mostly-files").join(format!("file-{index:08}.bin")),
            64,
        )?;
        fs::create_dir_all(
            root.join("mostly-directories")
                .join(format!("dir-{index:08}")),
        )?;
        fs::create_dir_all(root.join("empty").join(format!("empty-{index:08}")))?;
    }

    let width = scale.min(20_000);
    for index in 0..width {
        write_file(
            &root
                .join("wide")
                .join(format!("dir-{index:08}"))
                .join("data.bin"),
            128,
        )?;
    }

    // Keep below common Windows path limits while still exercising iterative traversal.
    let depth = scale.min(96);
    let mut deep = root.join("deep");
    for index in 0..depth {
        deep.push(format!("d{index:02}"));
        fs::create_dir_all(&deep)?;
        write_file(&deep.join("data.bin"), 32)?;
    }

    let matched = root.join("large-matched").join("target");
    for index in 0..scale {
        write_file(&matched.join(format!("artifact-{index:08}.bin")), 256)?;
    }
    write_file(&root.join("small/source.rs"), 128)?;
    write_file(&root.join("small/cache.pyc"), 64)?;
    Ok(())
}

fn main() -> io::Result<()> {
    let mut args = env::args_os().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/profile-fixtures"));
    let scale = args
        .next()
        .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
        .unwrap_or(10_000);
    create(&root, scale)?;
    println!("created {} at {}", scale, root.display());
    Ok(())
}

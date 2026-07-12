use super::RawEntry;
use rustix::fd::AsFd;
use rustix::fs::{AtFlags, Dir, Mode, OFlags};
use std::path::Path;

pub fn read_dir_fstatat(path: &Path) -> std::io::Result<Vec<RawEntry>> {
    let dir_fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;

    let mut dir = Dir::read_from(&dir_fd)
        .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;

    let mut result = Vec::with_capacity(256);

    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;
        let name_cstr = entry.file_name();
        let name_bytes = name_cstr.to_bytes();

        // Skip "." and ".."
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }

        let is_dir = entry.file_type() == rustix::fs::FileType::Directory;
        let is_symlink = entry.file_type() == rustix::fs::FileType::Symlink;

        let size = if !is_dir && !is_symlink {
            match rustix::fs::statat(
                dir_fd.as_fd(),
                name_cstr,
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(stat) => stat.st_size as u64,
                Err(_) => 0,
            }
        } else {
            0
        };

        let name = name_cstr.to_string_lossy().into_owned();
        result.push(RawEntry {
            name,
            size,
            is_dir,
            is_symlink,
        });
    }

    Ok(result)
}

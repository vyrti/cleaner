use super::{MetadataMode, RawEntry, INITIAL_DIRECTORY_CAPACITY};
use rustix::fd::AsFd;
use rustix::fs::{AtFlags, Dir, Mode, OFlags};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

pub fn read_dir_fstatat(
    path: &Path,
    metadata_mode: MetadataMode,
) -> std::io::Result<Vec<RawEntry>> {
    let dir_fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;

    let mut dir =
        Dir::read_from(&dir_fd).map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;

    let mut result = Vec::with_capacity(INITIAL_DIRECTORY_CAPACITY);

    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;
        let name_cstr = entry.file_name();
        let name_bytes = name_cstr.to_bytes();

        // Skip "." and ".."
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }

        let mut file_type = entry.file_type();
        let needs_type = file_type == rustix::fs::FileType::Unknown;
        let needs_size = metadata_mode == MetadataMode::WithSizes
            && file_type != rustix::fs::FileType::Directory
            && file_type != rustix::fs::FileType::Symlink;
        let stat = if needs_type || needs_size {
            rustix::fs::statat(dir_fd.as_fd(), name_cstr, AtFlags::SYMLINK_NOFOLLOW).ok()
        } else {
            None
        };
        if needs_type {
            if let Some(stat) = &stat {
                file_type = rustix::fs::FileType::from_raw_mode(stat.st_mode);
            }
        }
        let is_dir = file_type == rustix::fs::FileType::Directory;
        let is_symlink = file_type == rustix::fs::FileType::Symlink;
        let size = stat
            .filter(|_| !is_dir && !is_symlink && metadata_mode == MetadataMode::WithSizes)
            .map_or(0, |stat| stat.st_size as u64);

        let name = std::ffi::OsString::from_vec(name_bytes.to_vec());
        result.push(RawEntry {
            name,
            size,
            is_dir,
            is_symlink,
        });
    }

    Ok(result)
}

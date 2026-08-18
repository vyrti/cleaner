//! Platform disk free/total helpers.

use std::path::Path;

/// Returns `(total_bytes, free_bytes)` for the filesystem containing `path`.
pub fn get_disk_usage(path: &Path) -> Option<(u64, u64)> {
    get_disk_usage_inner(path)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn get_disk_usage_inner(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let path_str = path.to_str()?;
    let c_path = CString::new(path_str).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            let block_size = if stat.f_frsize > 0 {
                stat.f_frsize as u64
            } else {
                stat.f_bsize as u64
            };
            let total = block_size * stat.f_blocks as u64;
            let free = block_size * stat.f_bavail as u64;
            Some((total, free))
        } else {
            None
        }
    }
}

#[cfg(target_os = "freebsd")]
fn get_disk_usage_inner(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let path_str = path.to_str()?;
    let c_path = CString::new(path_str).ok()?;
    unsafe {
        let mut stat: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut stat) == 0 {
            let block_size = stat.f_bsize as u64;
            let total = block_size * stat.f_blocks as u64;
            let free = block_size * stat.f_bavail as u64;
            Some((total, free))
        } else {
            None
        }
    }
}

#[cfg(windows)]
fn get_disk_usage_inner(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let mut path_u16: Vec<u16> = OsStr::new(path).encode_wide().collect();
    path_u16.push(0);

    let mut free_bytes = 0u64;
    let mut total_bytes = 0u64;
    let mut total_free = 0u64;

    unsafe {
        if GetDiskFreeSpaceExW(
            path_u16.as_ptr(),
            &mut free_bytes,
            &mut total_bytes,
            &mut total_free,
        ) != 0
        {
            Some((total_bytes, free_bytes))
        } else {
            None
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn get_disk_usage_inner(_path: &Path) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_usage_returns_valid_stats_for_existing_directory() {
        let temp = std::env::temp_dir();
        if let Some((total, free)) = get_disk_usage(&temp) {
            assert!(total > 0);
            assert!(free <= total);
        }
    }

    #[test]
    fn disk_usage_handles_nonexistent_path() {
        let missing = Path::new("/path/that/does/not/exist/ever/12345");
        let _ = get_disk_usage(missing);
    }
}

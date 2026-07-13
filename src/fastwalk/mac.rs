// Portions of this file are adapted from getattrlistbulk-rs
// Copyright (c) 2023 quivent
// Licensed under MIT or Apache-2.0

use super::{MetadataMode, RawEntry, INITIAL_DIRECTORY_CAPACITY};
use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::sync::LazyLock;

const ATTR_BIT_MAP_COUNT: u16 = 5;
const ATTR_CMN_NAME: u32 = 0x00000001;
const ATTR_CMN_OBJTYPE: u32 = 0x00000008;
const ATTR_CMN_RETURNED_ATTRS: u32 = 0x80000000;
const ATTR_FILE_DATALENGTH: u32 = 0x00000200;

#[allow(dead_code)]
const VREG: u32 = 1;
const VDIR: u32 = 2;
const VLNK: u32 = 5;

static BULK_BUFFER_SIZE: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("CLEANER_MACOS_BULK_BUFFER_KIB")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(64)
        .clamp(16, 1024)
        * 1024
});

thread_local! {
    static BULK_BUFFER: RefCell<Vec<u8>> = RefCell::new(vec![0u8; *BULK_BUFFER_SIZE]);
}

struct FileDescriptor(c_int);

impl Drop for FileDescriptor {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct attribute_set_t {
    pub commonattr: u32,
    pub volattr: u32,
    pub dirattr: u32,
    pub fileattr: u32,
    pub forkattr: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct attrreference_t {
    pub attr_dataoffset: i32,
    pub attr_length: u32,
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
struct EntryPrefix {
    length: u32,
    returned: attribute_set_t,
    name_info: attrreference_t,
    obj_type: u32,
}

extern "C" {
    fn getattrlistbulk(
        fd: c_int,
        attrList: *mut libc::attrlist,
        attrBuf: *mut c_void,
        attrBufSize: libc::size_t,
        options: u64,
    ) -> c_int;
}

pub fn read_dir_bulk(path: &Path, metadata_mode: MetadataMode) -> std::io::Result<Vec<RawEntry>> {
    let path_cstr = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let fd = unsafe {
        libc::open(
            path_cstr.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd = FileDescriptor(fd);
    let mut attr_list = libc::attrlist {
        bitmapcount: ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_OBJTYPE,
        volattr: 0,
        dirattr: 0,
        fileattr: if metadata_mode == MetadataMode::WithSizes {
            ATTR_FILE_DATALENGTH
        } else {
            0
        },
        forkattr: 0,
    };

    BULK_BUFFER.with(|buffer| {
        let mut buffer = buffer.borrow_mut();
        let mut result_entries = Vec::with_capacity(INITIAL_DIRECTORY_CAPACITY);

        loop {
            let result = unsafe {
                getattrlistbulk(
                    fd.0,
                    &mut attr_list as *mut libc::attrlist,
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len(),
                    0,
                )
            };

            if result < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if result == 0 {
                break;
            }

            let mut ptr = buffer.as_ptr();
            for _ in 0..result {
                let record_offset = unsafe { ptr.offset_from(buffer.as_ptr()) as usize };
                let remaining = buffer.len().saturating_sub(record_offset);
                if remaining < std::mem::size_of::<EntryPrefix>() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "truncated getattrlistbulk record",
                    ));
                }
                let header = unsafe { std::ptr::read_unaligned(ptr.cast::<EntryPrefix>()) };
                let record_len = header.length as usize;
                if record_len < std::mem::size_of::<EntryPrefix>() || record_len > remaining {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid getattrlistbulk record length",
                    ));
                }

                let name_reference_offset = std::mem::offset_of!(EntryPrefix, name_info);
                let name_start = record_offset as isize
                    + name_reference_offset as isize
                    + header.name_info.attr_dataoffset as isize;
                let name_len = header.name_info.attr_length as usize;
                let record_end = record_offset + record_len;
                if name_start < record_offset as isize
                    || name_start as usize > record_end
                    || name_len > record_end.saturating_sub(name_start as usize)
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid getattrlistbulk filename reference",
                    ));
                }
                let name_bytes = &buffer[name_start as usize..name_start as usize + name_len];
                let len = name_bytes
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(name_bytes.len());
                let name = std::ffi::OsString::from_vec(name_bytes[..len].to_vec());

                if name != "." && name != ".." {
                    let is_dir = header.obj_type == VDIR;
                    let is_symlink = header.obj_type == VLNK;
                    let size = if !is_dir
                        && !is_symlink
                        && (header.returned.fileattr & ATTR_FILE_DATALENGTH) != 0
                    {
                        let data_offset = std::mem::size_of::<EntryPrefix>();
                        if record_len < data_offset + std::mem::size_of::<u64>() {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "truncated getattrlistbulk file length",
                            ));
                        }
                        unsafe { std::ptr::read_unaligned(ptr.add(data_offset).cast::<u64>()) }
                    } else {
                        0
                    };
                    result_entries.push(RawEntry {
                        name,
                        size,
                        is_dir,
                        is_symlink,
                    });
                }

                ptr = unsafe { ptr.add(record_len) };
            }
        }

        Ok(result_entries)
    })
}

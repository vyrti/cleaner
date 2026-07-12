// Portions of this file are adapted from getattrlistbulk-rs
// Copyright (c) 2023 quivent
// Licensed under MIT or Apache-2.0

use super::RawEntry;
use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::path::Path;

const ATTR_BIT_MAP_COUNT: u16 = 5;
const ATTR_CMN_NAME: u32 = 0x00000001;
const ATTR_CMN_OBJTYPE: u32 = 0x00000008;
const ATTR_CMN_RETURNED_ATTRS: u32 = 0x80000000;
const ATTR_FILE_DATALENGTH: u32 = 0x00000200;

#[allow(dead_code)]
const VREG: u32 = 1;
const VDIR: u32 = 2;
const VLNK: u32 = 5;

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
struct EntryHeader {
    length: u32,
    returned: attribute_set_t,
    name_info: attrreference_t,
    obj_type: u32,
    data_length: u64,
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

pub fn read_dir_bulk(path: &Path) -> std::io::Result<Vec<RawEntry>> {
    let path_str = path.to_str().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid UTF-8 path")
    })?;
    let path_cstr = CString::new(path_str)
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

    let mut attr_list = libc::attrlist {
        bitmapcount: ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_OBJTYPE,
        volattr: 0,
        dirattr: 0,
        fileattr: ATTR_FILE_DATALENGTH,
        forkattr: 0,
    };

    let mut result_entries = Vec::with_capacity(256);
    let mut buffer = vec![0u8; 256 * 1024]; // 256KB buffer

    loop {
        let result = unsafe {
            getattrlistbulk(
                fd,
                &mut attr_list as *mut libc::attrlist,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len(),
                0,
            )
        };

        if result < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(err);
        }

        if result == 0 {
            break;
        }

        let mut ptr = buffer.as_ptr();
        for _ in 0..result {
            let header = unsafe { &*(ptr as *const EntryHeader) };

            // Extract the filename using the attrreference_t offset
            let name_info_ptr = unsafe { ptr.offset(24) };
            let name_ptr =
                unsafe { name_info_ptr.offset(header.name_info.attr_dataoffset as isize) };
            let name_bytes = unsafe {
                std::slice::from_raw_parts(name_ptr, header.name_info.attr_length as usize)
            };

            // attr_length includes a trailing null byte or padding
            let len = name_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(name_bytes.len());
            let name = std::str::from_utf8(&name_bytes[..len])
                .unwrap_or("")
                .to_string();

            // Skip "." and ".."
            if name == "." || name == ".." {
                ptr = unsafe { ptr.offset(header.length as isize) };
                continue;
            }

            let is_dir = header.obj_type == VDIR;
            let is_symlink = header.obj_type == VLNK;

            // Extract file size if it's a regular file
            let size =
                if !is_dir && !is_symlink && (header.returned.fileattr & ATTR_FILE_DATALENGTH) != 0
                {
                    header.data_length
                } else {
                    0
                };

            result_entries.push(RawEntry {
                name,
                size,
                is_dir,
                is_symlink,
            });

            // Advance pointer by the record length
            ptr = unsafe { ptr.offset(header.length as isize) };
        }
    }

    unsafe { libc::close(fd) };
    Ok(result_entries)
}

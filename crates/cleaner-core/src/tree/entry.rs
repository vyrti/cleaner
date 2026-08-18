use std::ffi::OsString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: OsString,
    pub size: u64,
    pub is_dir: bool,
    pub is_temp: bool,
}

impl DirEntry {
    pub fn new(name: impl Into<OsString>, size: u64, is_dir: bool, is_temp: bool) -> Self {
        Self {
            name: name.into(),
            size,
            is_dir,
            is_temp,
        }
    }
}

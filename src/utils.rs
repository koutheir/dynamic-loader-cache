// Copyright 2024 Koutheir Attouchi.
// See the "LICENSE.txt" file at the top-level directory of this distribution.
//
// Licensed under the MIT license. This file may not be copied, modified,
// or distributed except according to those terms.

use std::borrow::Cow;
use std::ffi::CStr;
#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(not(unix))]
use std::ffi::OsString;
use std::fs::File;
use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;

use memmap2::{Mmap, MmapOptions};

use crate::errors::Error;
use crate::Result;

#[cfg(unix)]
pub(crate) fn os_str_from_cstr(cstr: &CStr) -> Result<&OsStr> {
    use std::os::unix::ffi::OsStrExt;

    Ok(OsStr::from_bytes(cstr.to_bytes()))
}

#[cfg(windows)]
pub(crate) fn os_string_from_cstr(cstr: &CStr) -> Result<OsString> {
    use std::os::windows::ffi::OsStringExt;

    let wstr: Vec<_> = cstr.to_str()?.encode_utf16().collect();
    Ok(OsString::from_wide(&wstr))
}

#[cfg(unix)]
pub(crate) fn path_from_cstr(cstr: &CStr) -> Result<&Path> {
    os_str_from_cstr(cstr).map(Path::new)
}

#[cfg(not(unix))]
pub(crate) fn path_buf_from_cstr(cstr: &CStr) -> Result<PathBuf> {
    os_string_from_cstr(cstr).map(PathBuf::from)
}

#[cfg(unix)]
pub(crate) fn path_from_bytes(bytes: &[u8]) -> Result<Cow<Path>> {
    use std::os::unix::ffi::OsStrExt;

    Ok(Cow::Borrowed(Path::new(OsStr::from_bytes(bytes))))
}

#[cfg(windows)]
pub(crate) fn path_from_bytes(bytes: &[u8]) -> Result<Cow<Path>> {
    use std::os::windows::ffi::OsStringExt;

    let wstr: Vec<_> = std::str::from_utf8(bytes)?.encode_utf16().collect();
    Ok(Cow::Owned(PathBuf::from(OsString::from_wide(&wstr))))
}

#[cfg(unix)]
pub(crate) fn cstr_entry_to_crate_entry<'cache>(
    key: &'cache CStr,
    value: &'cache CStr,
) -> Result<crate::Entry<'cache>> {
    let file_name = os_str_from_cstr(key).map(Cow::Borrowed)?;
    let full_path = path_from_cstr(value).map(Cow::Borrowed)?;

    Ok(crate::Entry {
        file_name,
        full_path,
    })
}

#[cfg(not(unix))]
pub(crate) fn cstr_entry_to_crate_entry<'cache>(
    key: &'cache CStr,
    value: &'cache CStr,
) -> Result<crate::Entry<'cache>> {
    let file_name = os_string_from_cstr(key).map(Cow::Owned)?;
    let full_path = path_buf_from_cstr(value).map(Cow::Owned)?;

    Ok(crate::Entry {
        file_name,
        full_path,
    })
}

pub(crate) fn map_file(path: &Path) -> Result<Mmap> {
    let file = File::open(path).map_err(|source| Error::Open {
        source,
        path: path.into(),
    })?;

    let md = file.metadata().map_err(|source| Error::ReadMetaData {
        source,
        path: path.into(),
    })?;

    let size = usize::try_from(md.len()).unwrap_or(usize::MAX);
    if size == 0 {
        return Err(Error::FileIsEmpty { path: path.into() });
    }

    unsafe { MmapOptions::default().len(size).map(&file) }.map_err(|source| Error::MapFile {
        source,
        path: path.into(),
    })
}

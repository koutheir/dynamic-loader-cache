// Copyright 2024 Koutheir Attouchi.
// See the "LICENSE.txt" file at the top-level directory of this distribution.
//
// Licensed under the MIT license. This file may not be copied, modified,
// or distributed except according to those terms.

//! Cache of the GNU/Linux old dynamic loader.

use core::ffi::{c_uint, CStr};
use core::mem::size_of;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use memoffset::offset_of;
use nom::bytes::complete::{tag as nom_tag, take as nom_take};
use nom::combinator::peek as nom_peek;
use nom::number::complete::u32 as nom_u32;
use nom::number::Endianness;
use nom::sequence::{preceded as nom_preceded, tuple as nom_tuple};
use nom::IResult;
use static_assertions::assert_eq_size;

use crate::utils::{cstr_entry_to_crate_entry, map_file};
use crate::{CacheProvider, Error, Result};

static CACHE_FILE_PATH: &str = "/etc/ld.so.cache";

static MAGIC: &[u8] = b"ld.so-1.7.0";

#[repr(C)]
struct Header {
    magic: [u8; 11],
    padding: [u8; 1],
    lib_count: c_uint,
}

#[repr(C)]
struct Entry {
    flags: u32,
    key: u32,
    value: u32,
}

const MAX_LIB_COUNT: u32 = u32::MAX
    .saturating_sub(size_of::<Header>() as u32)
    .saturating_div(size_of::<Entry>() as u32);

/// Cache of the GNU/Linux old dynamic loader.
///
/// This loads a dynamic loader cache file (*e.g.*, `/etc/ld.so.cache`),
/// in the old `ld.so-1.7.0` format, for either 32-bits or 64-bits architectures,
/// in either little-endian or big-endian byte order.
#[derive(Debug)]
pub struct Cache {
    path: PathBuf,
    map: Mmap,
    lib_count: u32,
}

impl Cache {
    /// Create a cache that loads the file `/etc/ld.so.cache`.
    pub fn load_default() -> Result<Self> {
        Self::load(CACHE_FILE_PATH)
    }

    /// Create a cache that loads the specified cache file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let map = map_file(path)?;
        let (_, lib_count) =
            Self::parse_header(&map).map_err(|r| Error::from_nom_parse(r, &map, path))?;

        Ok(Self {
            path: path.into(),
            map,
            lib_count,
        })
    }

    fn parse_header(bytes: &[u8]) -> IResult<&[u8], u32> {
        assert_eq_size!(u32, c_uint);

        let (input, lib_count) = nom_preceded(
            nom_preceded(
                nom_tag(MAGIC),
                nom_take(offset_of!(Header, lib_count) - MAGIC.len()),
            ),
            nom_u32(Endianness::Native),
        )(bytes)?;

        if lib_count > MAX_LIB_COUNT {
            return Err(nom::Err::Error(nom::error::make_error(
                bytes,
                nom::error::ErrorKind::TooLarge,
            )));
        }

        let min_size = size_of::<Header>()
            .saturating_add(size_of::<Entry>().saturating_mul(lib_count as usize));
        nom_peek(nom_take(min_size))(bytes)?;

        Ok((input, lib_count))
    }

    /// Return an iterator that returns cache entries.
    pub fn iter(&self) -> Result<impl Iterator<Item = Result<crate::Entry<'_>>> + '_> {
        let entries_end = size_of::<Header>()
            .saturating_add(size_of::<Entry>().saturating_mul(self.lib_count as usize));
        let entries_bytes = &self.map[size_of::<Header>()..entries_end];

        Ok(Iter {
            path: &self.path,
            entries_bytes,
            string_table: &self.map[entries_end..],
        })
    }
}

impl CacheProvider for Cache {
    fn entries_iter<'cache>(
        &'cache self,
    ) -> Result<Box<dyn Iterator<Item = Result<crate::Entry<'cache>>> + 'cache>> {
        let iter = self.iter()?;
        Ok(Box::new(iter))
    }
}

#[derive(Debug)]
struct Iter<'cache> {
    path: &'cache Path,
    entries_bytes: &'cache [u8],
    string_table: &'cache [u8],
}

impl<'cache> Iter<'cache> {
    fn next_fallible(&mut self) -> Result<crate::Entry<'cache>> {
        let (input, (key, value)) = nom_tuple((
            nom_preceded(
                nom_take(offset_of!(Entry, key)),
                nom_u32(Endianness::Native),
            ),
            nom_u32(Endianness::Native),
        ))(self.entries_bytes)
        .map_err(|r| Error::from_nom_parse(r, self.entries_bytes, self.path))?;

        self.entries_bytes = input;

        let key = self
            .string_table
            .get((key as usize)..)
            .ok_or(Error::OffsetIsInvalid {
                path: self.path.into(),
            })?;
        let key = CStr::from_bytes_until_nul(key)?;

        let value = self
            .string_table
            .get((value as usize)..)
            .ok_or(Error::OffsetIsInvalid {
                path: self.path.into(),
            })?;
        let value = CStr::from_bytes_until_nul(value)?;

        cstr_entry_to_crate_entry(key, value)
    }
}

impl<'cache> Iterator for Iter<'cache> {
    type Item = Result<crate::Entry<'cache>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.entries_bytes.len() < size_of::<Entry>() {
            None
        } else {
            Some(self.next_fallible())
        }
    }
}

#[cfg(test)]
fn print_cache(cache: &Cache) {
    for e in cache.iter().unwrap() {
        let e = e.unwrap();
        eprintln!(
            "{} => {}",
            e.file_name.to_string_lossy(),
            e.full_path.display()
        );
    }
}

#[test]
fn test1() {
    let cache = Cache::load("tests/ld.so-1.7.0/ld.so.cache").unwrap();
    print_cache(&cache);
}

#[test]
fn test2() {
    let cache = Cache::load("tests/ld.so-1.7.0/ld.so.cache.compat").unwrap();
    print_cache(&cache);
}

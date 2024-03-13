// Copyright 2024 Koutheir Attouchi.
// See the "LICENSE.txt" file at the top-level directory of this distribution.
//
// Licensed under the MIT license. This file may not be copied, modified,
// or distributed except according to those terms.

//! Cache of the GNU/Linux dynamic loader.

use core::ffi::CStr;
use core::iter::FusedIterator;
use core::mem::size_of;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use memoffset::offset_of;
use nom::bytes::complete::{tag as nom_tag, take as nom_take};
use nom::combinator::peek as nom_peek;
use nom::number::complete::{u32 as nom_u32, u8 as nom_u8};
use nom::number::Endianness;
use nom::sequence::{preceded as nom_preceded, terminated as nom_terminated, tuple as nom_tuple};
use nom::IResult;

use crate::utils::{cstr_entry_to_crate_entry, map_file};
use crate::{CacheProvider, Error, Result};

static CACHE_FILE_PATH: &str = "/etc/ld.so.cache";

static MAGIC: &[u8] = b"glibc-ld.so.cache1.1";

#[repr(C)]
struct Header {
    magic: [u8; 20],
    lib_count: u32,
    string_table_size: u32,
    flags: u8,
    flags_padding: [u8; 3],
    extension_offset: u32,
    unused: [u32; 3],
}

#[repr(C)]
struct Entry {
    flags: u32,
    key: u32,
    value: u32,
    os_version: u32,
    hw_cap: u64,
}

/// Cache of the GNU/Linux dynamic loader.
///
/// This loads a dynamic loader cache file (*e.g.*, `/etc/ld.so.cache`),
/// in the `glibc-ld.so.cache1.1` format, for either 32-bits or 64-bits architectures,
/// in either little-endian or big-endian byte order.
#[derive(Debug)]
pub struct Cache {
    path: PathBuf,
    map: Mmap,
    byte_order: Endianness,
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
        let (_, byte_order) =
            Self::parse_byte_order(&map).map_err(|r| Error::from_nom_parse(r, &map, path))?;
        let (_, lib_count) = Self::parse_header(&map, byte_order)
            .map_err(|r| Error::from_nom_parse(r, &map, path))?;

        Ok(Self {
            path: path.into(),
            map,
            byte_order,
            lib_count,
        })
    }

    fn parse_byte_order(bytes: &[u8]) -> IResult<&[u8], Endianness> {
        let (input, flags) = nom_preceded(nom_take(offset_of!(Header, flags)), nom_u8)(bytes)?;

        match flags & 0b11 {
            0 => Ok((input, Endianness::Native)),
            1 => Err(nom::Err::Error(nom::error::make_error(
                bytes,
                nom::error::ErrorKind::IsA,
            ))),
            2 => Ok((input, Endianness::Little)),
            3 => Ok((input, Endianness::Big)),
            _ => unreachable!(),
        }
    }

    fn parse_header(bytes: &[u8], byte_order: Endianness) -> IResult<&[u8], u32> {
        let (input, (lib_count, string_table_size)) = nom_tuple((
            nom_preceded(nom_tag(MAGIC), nom_u32(byte_order)),
            nom_terminated(
                nom_u32(byte_order),
                nom_take(size_of::<Header>() - offset_of!(Header, flags)),
            ),
        ))(bytes)?;

        let max_lib_count = bytes
            .len()
            .saturating_sub(size_of::<Header>())
            .saturating_div(size_of::<Entry>()) as u32;

        if lib_count > max_lib_count {
            return Err(nom::Err::Error(nom::error::make_error(
                bytes,
                nom::error::ErrorKind::TooLarge,
            )));
        }

        let max_string_table_size = bytes
            .len()
            .saturating_sub(size_of::<Header>())
            .saturating_sub((lib_count as usize).saturating_mul(size_of::<Entry>()))
            as u32;

        if string_table_size > max_string_table_size {
            return Err(nom::Err::Error(nom::error::make_error(
                bytes,
                nom::error::ErrorKind::TooLarge,
            )));
        }

        let min_size = size_of::<Header>()
            .saturating_add(size_of::<Entry>().saturating_mul(lib_count as usize))
            .saturating_add(string_table_size as usize);

        nom_peek(nom_take(min_size))(bytes)?;

        Ok((input, lib_count))
    }

    /// Return an iterator that returns cache entries.
    pub fn iter(&self) -> Result<impl FusedIterator<Item = Result<crate::Entry<'_>>> + '_> {
        let entries_end = size_of::<Header>()
            .saturating_add(size_of::<Entry>().saturating_mul(self.lib_count as usize));
        let entries_bytes = &self.map[size_of::<Header>()..entries_end];

        Ok(Iter {
            path: &self.path,
            entries_bytes,
            bytes: &self.map,
            byte_order: self.byte_order,
        })
    }
}

impl CacheProvider for Cache {
    fn entries_iter<'cache>(
        &'cache self,
    ) -> Result<Box<dyn FusedIterator<Item = Result<crate::Entry<'cache>>> + 'cache>> {
        let iter = self.iter()?;
        Ok(Box::new(iter))
    }
}

#[derive(Debug)]
struct Iter<'cache> {
    path: &'cache Path,
    entries_bytes: &'cache [u8],
    bytes: &'cache [u8],
    byte_order: Endianness,
}

impl<'cache> Iter<'cache> {
    fn next_fallible(&mut self) -> Result<crate::Entry<'cache>> {
        let (input, (key, value)) = nom_tuple((
            nom_preceded(nom_take(offset_of!(Entry, key)), nom_u32(self.byte_order)),
            nom_terminated(
                nom_u32(self.byte_order),
                nom_take(size_of::<Entry>() - offset_of!(Entry, os_version)),
            ),
        ))(self.entries_bytes)
        .map_err(|r| Error::from_nom_parse(r, self.entries_bytes, self.path))?;

        self.entries_bytes = input;

        let key = self
            .bytes
            .get((key as usize)..)
            .ok_or(Error::OffsetIsInvalid {
                path: self.path.into(),
            })?;
        let key = CStr::from_bytes_until_nul(key)?;

        let value = self
            .bytes
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

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.entries_bytes.len() / size_of::<Entry>();
        (remaining, Some(remaining))
    }
}

impl<'cache> FusedIterator for Iter<'cache> {}

impl<'cache> ExactSizeIterator for Iter<'cache> {}

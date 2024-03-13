// Copyright 2024 Koutheir Attouchi.
// See the "LICENSE.txt" file at the top-level directory of this distribution.
//
// Licensed under the MIT license. This file may not be copied, modified,
// or distributed except according to those terms.

//! Cache of the FreeBSD dynamic loader.

use core::iter::FusedIterator;
use core::mem::size_of;
use std::borrow::Cow;
use std::fs::read_dir;
use std::path::Path;
use std::rc::Rc;

use memmap2::Mmap;
use memoffset::offset_of;
use nom::bytes::complete::{tag as nom_tag, take as nom_take};
use nom::combinator::peek as nom_peek;
use nom::number::complete::u32 as nom_u32;
use nom::number::Endianness;
use nom::sequence::{preceded as nom_preceded, terminated as nom_terminated, tuple as nom_tuple};
use nom::IResult;

use crate::utils::{map_file, path_from_bytes};
use crate::{CacheProvider, Error, Result};

pub(crate) static CACHE_FILE_PATHS: &[&str] =
    &["/var/run/ld-elf.so.hints", "/var/run/ld-elf32.so.hints"];

const MAGIC: u32 = 0x74_6e_68_45;
const MAGIC_LE32: [u8; 4] = MAGIC.to_le_bytes();
const MAGIC_BE32: [u8; 4] = MAGIC.to_be_bytes();

const VERSION: u32 = 1_u32;

#[repr(C)]
struct Header {
    /// Magic number.
    magic: u32,
    /// File version (1).
    version: u32,
    /// Offset of string table in file.
    string_table_offset: u32,
    /// Size of string table.
    string_table_size: u32,
    /// Offset of directory list in string table.
    dir_list_offset: u32,
    /// strlen(dir_list).
    dir_list_size: u32,
    /// Room for expansion.
    spare: [u32; 26],
}

/// Cache of the FreeBSD dynamic loader.
///
/// This loads a dynamic loader cache file
/// (*e.g.*, `/var/run/ld-elf.so.hints`, `/var/run/ld-elf32.so.hints`),
/// for either 32-bits or 64-bits architectures, in either little-endian or big-endian byte order.
#[derive(Debug)]
pub struct Cache {
    map: Mmap,
    dir_list_offset: u32,
    dir_list_size: u32,
}

impl Cache {
    /// Create a cache that loads the specified cache file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let map = map_file(path)?;
        let (_, byte_order) =
            Self::parse_byte_order(&map).map_err(|r| Error::from_nom_parse(r, &map, path))?;
        let (_, (string_table_offset, dir_list_offset, dir_list_size)) =
            Self::parse_header(&map, byte_order)
                .map_err(|r| Error::from_nom_parse(r, &map, path))?;

        Ok(Self {
            map,
            dir_list_offset: string_table_offset.saturating_add(dir_list_offset),
            dir_list_size,
        })
    }

    fn parse_byte_order(bytes: &[u8]) -> IResult<&[u8], Endianness> {
        nom_tag::<&[u8], &[u8], nom::error::Error<&[u8]>>(&MAGIC_LE32)(bytes)
            .map(|(input, _)| (input, Endianness::Little))
            .or_else(|_| {
                nom_tag::<&[u8], &[u8], nom::error::Error<&[u8]>>(&MAGIC_BE32)(bytes)
                    .map(|(input, _)| (input, Endianness::Big))
            })
    }

    fn parse_header(bytes: &[u8], byte_order: Endianness) -> IResult<&[u8], (u32, u32, u32)> {
        let version_bytes = match byte_order {
            Endianness::Big => VERSION.to_be_bytes(),
            Endianness::Little => VERSION.to_le_bytes(),
            Endianness::Native => VERSION.to_ne_bytes(),
        };

        let (input, (string_table_offset, string_table_size, dir_list_offset, dir_list_size)) =
            nom_tuple((
                nom_preceded(
                    nom_preceded(
                        nom_take(offset_of!(Header, version)),
                        nom_tag(version_bytes),
                    ),
                    nom_u32(byte_order),
                ),
                nom_u32(byte_order),
                nom_u32(byte_order),
                nom_terminated(
                    nom_u32(byte_order),
                    nom_take(size_of::<Header>() - offset_of!(Header, spare)),
                ),
            ))(bytes)?;

        if string_table_size > u32::MAX.saturating_sub(string_table_offset) {
            return Err(nom::Err::Error(nom::error::make_error(
                bytes,
                nom::error::ErrorKind::TooLarge,
            )));
        }

        if dir_list_offset > u32::MAX.saturating_sub(string_table_offset) {
            return Err(nom::Err::Error(nom::error::make_error(
                bytes,
                nom::error::ErrorKind::TooLarge,
            )));
        }

        let max_dir_list_size = u32::MAX
            .saturating_sub(string_table_offset)
            .saturating_sub(dir_list_offset)
            .saturating_sub(1);
        if dir_list_size > max_dir_list_size {
            return Err(nom::Err::Error(nom::error::make_error(
                bytes,
                nom::error::ErrorKind::TooLarge,
            )));
        }

        let min_size = u32::max(
            string_table_offset.saturating_add(string_table_size),
            string_table_offset
                .saturating_add(dir_list_offset)
                .saturating_add(dir_list_size)
                .saturating_add(1),
        );
        nom_peek(nom_take(min_size))(bytes)?;

        Ok((input, (string_table_offset, dir_list_offset, dir_list_size)))
    }

    /// Return an iterator that returns cache entries.
    pub fn iter(&self) -> Result<impl FusedIterator<Item = Result<crate::Entry<'_>>> + '_> {
        let start = self.dir_list_offset as usize;
        let bytes = &self.map[start..start.saturating_add(self.dir_list_size as usize)];

        let iter = bytes
            .split(|&b| b == b':')
            .map(path_from_bytes)
            .filter_map(Result::ok)
            .map(Rc::new)
            .filter_map(|path| {
                read_dir(path.as_ref().as_ref())
                    .ok()
                    .map(move |dirs| dirs.map(move |entries| (Rc::clone(&path), entries)))
            })
            .flatten()
            .map(|(path, entry)| match entry {
                Ok(entry) => Ok(crate::Entry {
                    file_name: Cow::Owned(entry.file_name()),
                    full_path: Cow::Owned(entry.path()),
                }),

                Err(source) => {
                    let path = path.as_ref().as_ref().into();
                    Err(Error::ReadDir { path, source })
                }
            });

        Ok(iter)
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
    let cache = Cache::load("tests/ld-elf.so.hints/ld-elf.so.hints").unwrap();
    print_cache(&cache);
}

#[test]
fn test2() {
    let cache = Cache::load("tests/ld-elf.so.hints/ld-elf32.so.hints").unwrap();
    print_cache(&cache);
}

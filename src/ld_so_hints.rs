// Copyright 2024 Koutheir Attouchi.
// See the "LICENSE.txt" file at the top-level directory of this distribution.
//
// Licensed under the MIT license. This file may not be copied, modified,
// or distributed except according to those terms.

//! Cache of the OpenBSD or NetBSD dynamic loader.

use core::ffi::{c_int, CStr};
use core::iter::FusedIterator;
use core::mem::size_of;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use memoffset::offset_of;
use nom::bytes::complete::{tag as nom_tag, take as nom_take};
use nom::combinator::peek as nom_peek;
use nom::number::complete::{u32 as nom_u32, u64 as nom_u64};
use nom::number::Endianness;
use nom::sequence::{preceded as nom_preceded, terminated as nom_terminated, tuple as nom_tuple};
use nom::IResult;
use static_assertions::assert_eq_size;

use crate::utils::{cstr_entry_to_crate_entry, map_file};
use crate::{CacheProvider, DataModel, Error, Result};

static CACHE_FILE_PATH: &str = "/var/run/ld.so.hints";

const MAGIC: u32 = 0x4c_44_48_69_u32;
const MAGIC_LE32: [u8; 4] = MAGIC.to_le_bytes();
const MAGIC_BE32: [u8; 4] = MAGIC.to_be_bytes();
const MAGIC_LE64: [u8; 8] = (MAGIC as u64).to_le_bytes();
const MAGIC_BE64: [u8; 8] = (MAGIC as u64).to_le_bytes();

//const VERSION_1: u32 = 1; // We do not support this ancient version.

const VERSION_2: u32 = 2;
const VERSION_2_LE32: [u8; 4] = VERSION_2.to_le_bytes();
const VERSION_2_BE32: [u8; 4] = VERSION_2.to_be_bytes();
const VERSION_2_LE64: [u8; 8] = (VERSION_2 as u64).to_le_bytes();
const VERSION_2_BE64: [u8; 8] = (VERSION_2 as u64).to_le_bytes();

/// Maximum number of recognized shared object version numbers.
const MAX_DEWEY: usize = 8;

/*
/// Header of the hints file.
#[repr(C)]
struct Header {
    magic: c_long,
    /// Interface version number.
    version: c_long,
    /// Location of hash table.
    hash_table: c_long,
    /// Number of buckets in hash_table.
    bucket_count: c_long,
    /// Location of strings.
    string_table: c_long,
    /// Size of strings.
    string_table_size: c_long,
    /// End of hints (max offset in file).
    end_of_hints: c_long,
    /// Colon-separated list of search dirs.
    dir_list: c_long,
}
*/

/// Hash table element in hints file.
#[repr(C)]
struct Bucket {
    /// Index of the library name into the string table.
    name_index: c_int,
    /// Index of the full path into the string table.
    path_index: c_int,
    /// The versions.
    dewey: [c_int; MAX_DEWEY],
    /// Number of version numbers.
    dewey_count: c_int,
    /// Next in this bucket.
    next: c_int,
}

type ParseHeaderImplData = (usize, usize, usize, usize, usize, usize);

/// Cache of the OpenBSD or NetBSD dynamic loader.
///
/// This loads a dynamic loader cache file (*e.g.*, `/var/run/ld.so.hints`),
/// for either 32-bits or 64-bits architectures, in either little-endian or big-endian byte order.
#[derive(Debug)]
pub struct Cache {
    path: PathBuf,
    map: Mmap,
    byte_order: Endianness,
    hash_table: usize,
    bucket_count: usize,
    string_table: usize,
    string_table_size: usize,
}

impl Cache {
    /// Create a cache that loads the file `/var/run/ld.so.hints`.
    pub fn load_default() -> Result<Self> {
        Self::load(CACHE_FILE_PATH)
    }

    /// Create a cache that loads the specified cache file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let map = map_file(path)?;
        let (_, (data_model, byte_order)) =
            Self::parse_byte_order(&map).map_err(|r| Error::from_nom_parse(r, &map, path))?;
        let (_, (hash_table, bucket_count, string_table, string_table_size)) =
            Self::parse_header(&map, data_model, byte_order)
                .map_err(|r| Error::from_nom_parse(r, &map, path))?;

        Ok(Self {
            path: path.into(),
            map,
            byte_order,
            hash_table,
            bucket_count,
            string_table,
            string_table_size,
        })
    }

    fn parse_byte_order(bytes: &[u8]) -> IResult<&[u8], (DataModel, Endianness)> {
        let nom_tag_long = nom_tag::<&[u8], &[u8], nom::error::Error<&[u8]>>;

        let mut nom_64le = nom_terminated(nom_tag_long(&MAGIC_LE64), nom_tag_long(&VERSION_2_LE64));
        let mut nom_64be = nom_terminated(nom_tag_long(&MAGIC_BE64), nom_tag_long(&VERSION_2_BE64));
        let mut nom_32le = nom_terminated(nom_tag_long(&MAGIC_LE32), nom_tag_long(&VERSION_2_LE32));
        let mut nom_32be = nom_terminated(nom_tag_long(&MAGIC_BE32), nom_tag_long(&VERSION_2_BE32));

        nom_64le(bytes)
            .map(|(input, _)| (input, (DataModel::LP64, Endianness::Little)))
            .or_else(|_| {
                nom_64be(bytes).map(|(input, _)| (input, (DataModel::LP64, Endianness::Big)))
            })
            .or_else(|_| {
                nom_32le(bytes).map(|(input, _)| (input, (DataModel::ILP32, Endianness::Little)))
            })
            .or_else(|_| {
                nom_32be(bytes).map(|(input, _)| (input, (DataModel::ILP32, Endianness::Big)))
            })
    }

    fn parse_header(
        bytes: &[u8],
        data_model: DataModel,
        byte_order: Endianness,
    ) -> IResult<&[u8], (usize, usize, usize, usize)> {
        assert_eq_size!(u32, c_int);

        let (
            input,
            (hash_table, bucket_count, string_table, string_table_size, end_of_hints, _dir_list),
        ) = match data_model {
            DataModel::ILP32 => Self::parse_header_impl(bytes, nom_u32(byte_order)),
            DataModel::LP64 => Self::parse_header_impl(bytes, nom_u64(byte_order)),
        }?;

        let hash_table_end =
            hash_table.saturating_add(bucket_count.saturating_mul(size_of::<Bucket>()));
        let string_table_end = string_table.saturating_add(string_table_size);
        let min_size = usize::max(usize::max(hash_table_end, string_table_end), end_of_hints);
        nom_peek(nom_take(min_size))(bytes)?;

        Ok((
            input,
            (hash_table, bucket_count, string_table, string_table_size),
        ))
    }

    fn parse_header_impl<'bytes, ULong, NomULong>(
        bytes: &'bytes [u8],
        nom_ulong: NomULong,
    ) -> IResult<&'bytes [u8], ParseHeaderImplData>
    where
        ULong: Sized,
        usize: TryFrom<ULong>,
        NomULong: Fn(&'bytes [u8]) -> IResult<&'bytes [u8], ULong, nom::error::Error<&'bytes [u8]>>,
    {
        let (
            input,
            (hash_table, bucket_count, string_table, string_table_size, end_of_hints, dir_list),
        ) = nom_tuple((
            nom_preceded(nom_take(size_of::<ULong>().saturating_mul(2)), &nom_ulong),
            &nom_ulong,
            &nom_ulong,
            &nom_ulong,
            &nom_ulong,
            &nom_ulong,
        ))(bytes)?;

        let into_usize = |n: ULong| {
            n.try_into().map_err(|_| {
                let err = nom::error::make_error(bytes, nom::error::ErrorKind::TooLarge);
                nom::Err::Error(err)
            })
        };

        Ok((
            input,
            (
                into_usize(hash_table)?,
                into_usize(bucket_count)?,
                into_usize(string_table)?,
                into_usize(string_table_size)?,
                into_usize(end_of_hints)?,
                into_usize(dir_list)?,
            ),
        ))
    }

    /// Return an iterator that returns cache entries.
    pub fn iter(&self) -> Result<impl FusedIterator<Item = Result<crate::Entry<'_>>> + '_> {
        let hash_table_end = self
            .hash_table
            .saturating_add(self.bucket_count.saturating_mul(size_of::<Bucket>()));
        let hash_table = &self.map[self.hash_table..hash_table_end];

        let string_table_end = self.string_table.saturating_add(self.string_table_size);
        let string_table = &self.map[self.string_table..string_table_end];

        Ok(Iter {
            path: &self.path,
            hash_table,
            string_table,
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
    hash_table: &'cache [u8],
    string_table: &'cache [u8],
    byte_order: Endianness,
}

impl<'cache> Iter<'cache> {
    fn next_fallible(&mut self) -> Result<crate::Entry<'cache>> {
        let (input, (key, value)) = nom_tuple((
            nom_u32(self.byte_order),
            nom_terminated(
                nom_u32(self.byte_order),
                nom_take(size_of::<Bucket>() - offset_of!(Bucket, dewey)),
            ),
        ))(self.hash_table)
        .map_err(|r| Error::from_nom_parse(r, self.hash_table, self.path))?;

        self.hash_table = input;

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
        if self.hash_table.len() < size_of::<Bucket>() {
            None
        } else {
            Some(self.next_fallible())
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.hash_table.len() / size_of::<Bucket>();
        (remaining, Some(remaining))
    }
}

impl<'cache> FusedIterator for Iter<'cache> {}

impl<'cache> ExactSizeIterator for Iter<'cache> {}

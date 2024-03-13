use core::mem::size_of;
use std::io::{Cursor, Write};

use assert_matches::assert_matches;
use memoffset::offset_of;
use nom::number::Endianness;
use proptest::prelude::*;

use super::{Cache, Entry, Header, MAGIC};

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
fn load() {
    let cache = Cache::load("tests/glibc-ld.so.cache1.1/ld.so.cache").unwrap();
    print_cache(&cache);
}

#[test]
fn parse_byte_order_empty() {
    Cache::parse_byte_order(&[]).unwrap_err();
}

prop_compose! {
    /// Generate a random `size`, then random `flags` and `lib_count`,
    /// in a fashion that makes sense to the parser.
    ///
    /// `lib_count` is a random value that depends on the random `size`.
    ///
    /// `flags` is composed of 6 higher bits and two lower `byte_order` bits.
    fn header_components0()
        (size in proptest::num::u16::ANY)
        (
            size in Just(size),
            byte_order in 0_u8..=2,
            flags in 0_u8..=0b0011_1111,
            lib_count in 0_u32..=(size as u32 / size_of::<Entry>() as u32)
        )
        -> (u16, u8, u32)
    {
        let flags = (flags << 2_u8) | if byte_order == 0 { 0 } else { 1 + byte_order };
        (size, flags, lib_count)
    }
}

prop_compose! {
    /// Generate random `flags`, `lib_count`, `string_table_size` and `bytes`,
    /// in a fashion that makes sense to the parser.
    ///
    /// `string_table_size` is a random value that depends on the random `size`
    /// and `lib_count`.
    ///
    /// `bytes` is a random value that depends on the random `size`.
    fn header_components()
        ((size, flags, lib_count) in header_components0())
        (
            flags in Just(flags),
            lib_count in Just(lib_count),
            string_table_size in 0_u32..=(size as u32 - lib_count * size_of::<Entry>() as u32),
            bytes in prop::collection::vec(0_u8.., 0..=(2 * size as usize))
        )
        -> (u8, u32, u32, Vec<u8>)
    {
        (flags, lib_count, string_table_size, bytes)
    }
}

proptest! {
    #[test]
    fn load_random(
        (flags, lib_count, string_table_size, bytes) in header_components(),
        random_u8 in proptest::num::u8::ANY,
        random_u32 in proptest::num::u32::ANY,
        unused in proptest::array::uniform::<_, 19>(0_u8..),
    ) {
        load_random0(flags, lib_count, string_table_size, &bytes, unused)?;
        load_random0(flags, lib_count, random_u32, &bytes, unused)?;
        load_random0(flags, random_u32, string_table_size, &bytes, unused)?;
        load_random0(random_u8, lib_count, string_table_size, &bytes, unused)?;
    }
}

fn load_random0(
    flags: u8,
    lib_count: u32,
    string_table_size: u32,
    bytes: &[u8],
    unused: [u8; 19],
) -> Result<(), TestCaseError> {
    let (byte_order, u32_bytes): (Endianness, fn(u32) -> [u8; 4]) = match flags & 0b11 {
        0 => (Endianness::Native, u32::to_ne_bytes),
        2 => (Endianness::Little, u32::to_le_bytes),
        3 => (Endianness::Big, u32::to_be_bytes),
        _ => (Endianness::Native, u32::to_ne_bytes),
    };

    let mut cursor = Cursor::new(Vec::<u8>::with_capacity(size_of::<Header>() + bytes.len()));
    cursor.write_all(MAGIC)?;
    cursor.write_all(&u32_bytes(lib_count))?;
    cursor.write_all(&u32_bytes(string_table_size))?;
    cursor.write_all(&[flags])?;
    cursor.write_all(&unused)?;
    cursor.write_all(bytes)?;
    let bytes = cursor.into_inner();

    let r = Cache::parse_byte_order(&bytes);
    if (flags & 0b11) == 1 {
        assert_matches!(r, Err(_));
        return Ok(());
    }

    assert_matches!(r, Ok((_, bo)) if bo == byte_order);

    let Ok((_, lib_count)) = Cache::parse_header(&bytes, byte_order) else {
        return Ok(());
    };

    let lib_count_bytes = bytes
        .get(offset_of!(Header, lib_count)..(offset_of!(Header, lib_count) + 4))
        .unwrap_or(&[]);
    prop_assert_eq!(lib_count_bytes, u32_bytes(lib_count));

    // TODO(KAT): Test iterators.

    Ok(())
}

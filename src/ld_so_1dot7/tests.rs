use core::mem::size_of;
use std::io::{Cursor, Write};

use memoffset::offset_of;
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
    let cache = Cache::load("tests/ld.so-1.7.0/ld.so.cache").unwrap();
    print_cache(&cache);
}

#[test]
fn load_compat() {
    let cache = Cache::load("tests/ld.so-1.7.0/ld.so.cache.compat").unwrap();
    print_cache(&cache);
}

#[test]
fn parse_byte_order_empty() {
    Cache::parse_header(&[]).unwrap_err();
}

prop_compose! {
    /// Generate a random `size`, then random `lib_count` and `bytes`,
    /// in a fashion that makes sense to the parser.
    ///
    /// `lib_count` and `bytes` are random values that depend on the random `size`.
    fn header_components()
        (size in proptest::num::u16::ANY)
        (
            lib_count in 0_u32..=(size as u32 / size_of::<Entry>() as u32),
            bytes in prop::collection::vec(0_u8.., 0..=(2 * size as usize)),
        )
        -> (u32, Vec<u8>)
    {
        (lib_count, bytes)
    }
}

proptest! {
    #[test]
    fn load_random(
        (lib_count, bytes) in header_components(),
        random_u8 in proptest::num::u8::ANY,
        random_u32 in proptest::num::u32::ANY,
    ) {
        load_random0([random_u8], lib_count, &bytes)?;
        load_random0([random_u8], random_u32, &bytes)?;
    }
}

fn load_random0(padding: [u8; 1], lib_count: u32, bytes: &[u8]) -> Result<(), TestCaseError> {
    let mut cursor = Cursor::new(Vec::<u8>::with_capacity(size_of::<Header>() + bytes.len()));
    cursor.write_all(MAGIC)?;
    cursor.write_all(&padding)?;
    cursor.write_all(&lib_count.to_ne_bytes())?;
    cursor.write_all(bytes)?;
    let bytes = cursor.into_inner();

    let Ok((_, lib_count)) = Cache::parse_header(&bytes) else {
        return Ok(());
    };

    let lib_count_bytes = bytes
        .get(offset_of!(Header, lib_count)..(offset_of!(Header, lib_count) + 4))
        .unwrap_or(&[]);
    prop_assert_eq!(lib_count_bytes, lib_count.to_ne_bytes());

    // TODO(KAT): Test iterators.

    Ok(())
}

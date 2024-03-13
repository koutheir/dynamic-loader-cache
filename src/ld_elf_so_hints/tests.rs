use super::Cache;

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
fn load_64bits() {
    let cache = Cache::load("tests/ld-elf.so.hints/ld-elf.so.hints").unwrap();
    print_cache(&cache);
}

#[test]
fn load_32bits() {
    let cache = Cache::load("tests/ld-elf.so.hints/ld-elf32.so.hints").unwrap();
    print_cache(&cache);
}

#[test]
fn parse_byte_order_empty() {
    Cache::parse_byte_order(&[]).unwrap_err();
}

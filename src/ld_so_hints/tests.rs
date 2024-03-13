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
fn load() {
    let cache = Cache::load("tests/ld.so.hints/ld.so.hints").unwrap();
    print_cache(&cache);
}

#[test]
fn parse_byte_order_empty() {
    Cache::parse_byte_order(&[]).unwrap_err();
}

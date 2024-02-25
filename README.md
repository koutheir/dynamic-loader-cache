[![crates.io](https://img.shields.io/crates/v/dynamic-loader-cache.svg)](https://crates.io/crates/dynamic-loader-cache)
[![docs.rs](https://docs.rs/dynamic-loader-cache/badge.svg)](https://docs.rs/dynamic-loader-cache)
[![license](https://img.shields.io/github/license/koutheir/dynamic-loader-cache?color=black)](https://raw.githubusercontent.com/koutheir/dynamic-loader-cache/master/LICENSE.txt)

# Reader of the dynamic loader shared libraries cache

On certain operating systems, the component that loads an executable and prepares it for execution
needs to resolve the shared libraries (also known as dynamically-linked libraries) that
the executable depends on.
These dependencies are usually only-partly specified in the executable file.
The mapping from the partially-specified library to the full path of the library can be an expensive
operation, so caches are usually maintained in order to speed up the mapping.

This crate gives read-only access to these caches, allowing resolution of a partially-specified
**library name** to a **full path** of the library file.
For example, querying the library name `libc.so.6` may return a list of likely library paths:
- `/usr/lib/x86_64-linux-gnu/libc.so.6`
- `/usr/lib/i386-linux-gnu/libc.so.6`

```rust
use dynamic_loader_cache::{Cache, Result};

fn main() -> Result<()> {
    let cache = Cache::load()?;
    let libc_iter = cache
        .iter()?
        // Ignore entries with errors.
        .filter_map(Result::ok)
        // Select entries for "libc.so.6".
        .filter_map(|entry| (*entry.file_name == *"libc.so.6").then_some(entry.full_path));

    for full_path in libc_iter {
        println!("{}", full_path.display());
    }
    Ok(())
}
```

The crate presents a simple interface, but extracts and **aggregates** information from all the caches
supported and present on the system. Here is an example that uses this interface:

```rust
use dynamic_loader_cache::{Cache, Result};

fn main() -> Result<()> {
    let cache = Cache::load()?;

    for entry in cache.iter()? {
        let entry = entry?;
        println!("{} => {}", entry.file_name.to_str().unwrap(), entry.full_path.display());
    }
    Ok(())
}
```

This crate also allows loading of a specific dynamic loader cache, instead of automatic discovery
and aggregation of all supported and present caches.
In order to do that, checkout the following structures: [`glibc_ld_so_cache_1dot1::Cache`],
[`ld_elf_so_hints::Cache`], [`ld_so_1dot7::Cache`], [`ld_so_hints::Cache`].

## Supported operating systems

The following operating systems are currently supported:

- **FreeBSD**: dynamic loader cache files `/var/run/ld-elf.so.hints` and `/var/run/ld-elf32.so.hints`.
- **GNU/Linux**: dynamic loader cache file `/etc/ld.so.cache`, in `ld.so-1.7.0` or
  `glibc-ld.so.cache1.1` formats, in little-endian or big-endian byte orders.
- **OpenBSD/NetBSD**: dynamic loader cache file `/var/run/ld.so.hints`.

## Versioning

This project adheres to [Semantic Versioning].
The `CHANGELOG.md` file details notable changes over time.

[Semantic Versioning]: https://semver.org/spec/v2.0.0.html

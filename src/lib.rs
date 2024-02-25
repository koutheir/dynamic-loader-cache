// Copyright 2024 Koutheir Attouchi.
// See the "LICENSE.txt" file at the top-level directory of this distribution.
//
// Licensed under the MIT license. This file may not be copied, modified,
// or distributed except according to those terms.

// TODO(KAT): Allow access to specific cache files.

#![doc = include_str!("../README.md")]
#![doc(html_root_url = "https://docs.rs/dynamic-loader-cache/0.1.0")]
#![warn(
    unsafe_op_in_unsafe_fn,
    missing_docs,
    keyword_idents,
    macro_use_extern_crate,
    missing_debug_implementations,
    non_ascii_idents,
    trivial_casts,
    trivial_numeric_casts,
    unstable_features,
    unused_extern_crates,
    unused_import_braces,
    unused_labels,
    variant_size_differences,
    unused_qualifications,
    clippy::must_use_candidate,
    clippy::default_numeric_fallback,
    clippy::single_char_lifetime_names
)]
// Activate these lints to clean up the code and hopefully detect some issues.
/*
#![warn(clippy::all, clippy::pedantic, clippy::restriction)]
#![allow(
    clippy::doc_markdown,
    clippy::exhaustive_structs,
    clippy::missing_inline_in_public_items,
    clippy::implicit_return,
    clippy::missing_docs_in_private_items,
    clippy::missing_errors_doc,
    clippy::question_mark_used,
    clippy::unnecessary_wraps,
    clippy::single_call_fn,
    clippy::undocumented_unsafe_blocks,
    clippy::shadow_reuse,
    clippy::shadow_unrelated,
    clippy::separated_literal_suffix,
    clippy::expect_used,
    clippy::unused_self,
    clippy::mod_module_files,
    clippy::pub_use,
    clippy::module_name_repetitions,
    clippy::indexing_slicing,
    clippy::absolute_paths,
    clippy::min_ident_chars,
    clippy::impl_trait_in_params
)]
*/

mod errors;
pub mod glibc_ld_so_cache_1dot1;
pub mod ld_elf_so_hints;
pub mod ld_so_1dot7;
pub mod ld_so_hints;
mod utils;

use core::mem::size_of;
use std::borrow::Cow;
use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

use arrayvec::ArrayVec;
use static_assertions::const_assert;

pub use crate::errors::Error;

const CACHE_IMPL_COUNT: usize = 5;

/// Result of a fallible operation.
pub type Result<T> = core::result::Result<T, Error>;

/// Supported data models.
/// See: https://en.wikipedia.org/wiki/64-bit_computing#64-bit_data_models
#[derive(Debug, Clone, Copy)]
enum DataModel {
    /// c_int=i32 c_long=i32
    ILP32,
    /// c_int=i32 c_long=i64
    LP64,
}

/// Cache entry.
#[derive(Debug)]
#[non_exhaustive]
pub struct Entry<'cache> {
    /// File name of the shared library.
    pub file_name: Cow<'cache, OsStr>,
    /// Absolute path of the shared library.
    pub full_path: Cow<'cache, Path>,
}

trait CacheProvider: fmt::Debug + Sync + Send {
    fn entries_iter<'cache>(
        &'cache self,
    ) -> Result<Box<dyn Iterator<Item = Result<Entry<'cache>>> + 'cache>>;
}

#[derive(Debug)]
enum CacheImpl {
    LdSO1dot7(ld_so_1dot7::Cache),
    GLibCLdSOCache1dot1(glibc_ld_so_cache_1dot1::Cache),
    LdELFSOHints(ld_elf_so_hints::Cache),
    LdSOHints(ld_so_hints::Cache),
}

impl AsRef<dyn CacheProvider> for CacheImpl {
    fn as_ref(&self) -> &(dyn CacheProvider + 'static) {
        match self {
            Self::LdSO1dot7(cache) => cache,
            Self::GLibCLdSOCache1dot1(cache) => cache,
            Self::LdELFSOHints(cache) => cache,
            Self::LdSOHints(cache) => cache,
        }
    }
}

/// Reader of the dynamic loader shared libraries cache.
#[derive(Debug)]
pub struct Cache {
    caches: ArrayVec<CacheImpl, CACHE_IMPL_COUNT>,
}

impl Cache {
    /// Load all dynamic loader caches supported and present on the system.
    pub fn load() -> Result<Self> {
        const_assert!(size_of::<u32>() <= size_of::<usize>());

        let mut caches = ArrayVec::<CacheImpl, CACHE_IMPL_COUNT>::default();

        if cfg!(target_os = "freebsd") {
            Self::try_loading_ld_elf_so_hints(&mut caches)?;
            Self::try_loading_ld_so_hints(&mut caches)?;
            Self::try_loading_ld_so_1dot7(&mut caches)?;
            Self::try_loading_glibc_ld_so_cache_1dot1(&mut caches)?;
        } else if cfg!(any(target_os = "openbsd", target_os = "netbsd")) {
            Self::try_loading_ld_so_hints(&mut caches)?;
            Self::try_loading_ld_elf_so_hints(&mut caches)?;
            Self::try_loading_ld_so_1dot7(&mut caches)?;
            Self::try_loading_glibc_ld_so_cache_1dot1(&mut caches)?;
        } else {
            Self::try_loading_glibc_ld_so_cache_1dot1(&mut caches)?;
            Self::try_loading_ld_elf_so_hints(&mut caches)?;
            Self::try_loading_ld_so_hints(&mut caches)?;
            Self::try_loading_ld_so_1dot7(&mut caches)?;
        }

        Ok(Self { caches })
    }

    fn try_loading_glibc_ld_so_cache_1dot1(
        caches: &mut ArrayVec<CacheImpl, CACHE_IMPL_COUNT>,
    ) -> Result<()> {
        if let Ok(cache) = glibc_ld_so_cache_1dot1::Cache::load_default() {
            caches.push(CacheImpl::GLibCLdSOCache1dot1(cache));
        }
        Ok(())
    }

    fn try_loading_ld_elf_so_hints(
        caches: &mut ArrayVec<CacheImpl, CACHE_IMPL_COUNT>,
    ) -> Result<()> {
        for path in ld_elf_so_hints::CACHE_FILE_PATHS.iter().map(Path::new) {
            if let Ok(cache) = ld_elf_so_hints::Cache::load(path) {
                caches.push(CacheImpl::LdELFSOHints(cache));
            }
        }
        Ok(())
    }

    fn try_loading_ld_so_hints(caches: &mut ArrayVec<CacheImpl, CACHE_IMPL_COUNT>) -> Result<()> {
        if let Ok(cache) = ld_so_hints::Cache::load_default() {
            caches.push(CacheImpl::LdSOHints(cache));
        }
        Ok(())
    }

    fn try_loading_ld_so_1dot7(caches: &mut ArrayVec<CacheImpl, CACHE_IMPL_COUNT>) -> Result<()> {
        if let Ok(cache) = ld_so_1dot7::Cache::load_default() {
            caches.push(CacheImpl::LdSO1dot7(cache));
        }
        Ok(())
    }

    /// Returns an iterator that returns the cache entries.
    ///
    /// The entries are aggregated from all dynamic loader caches that have been previously loaded.
    pub fn iter(&self) -> Result<impl Iterator<Item = Result<Entry<'_>>> + '_> {
        Ok(self
            .caches
            .iter()
            .map(AsRef::as_ref)
            .map(CacheProvider::entries_iter)
            .collect::<Result<ArrayVec<_, CACHE_IMPL_COUNT>>>()?
            .into_iter()
            .flatten())
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
    let cache = Cache::load().unwrap();
    print_cache(&cache);
}

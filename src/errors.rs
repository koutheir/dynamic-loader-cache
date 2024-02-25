// Copyright 2024 Koutheir Attouchi.
// See the "LICENSE.txt" file at the top-level directory of this distribution.
//
// Licensed under the MIT license. This file may not be copied, modified,
// or distributed except according to those terms.

use std::path::PathBuf;

/// Information about a failure of an operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum Error {
    #[error("failed to read directory")]
    ReadDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to open file. Path: {path}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to map file. Path: {path}")]
    MapFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read metadata. Path: {path}")]
    ReadMetaData {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("file is empty. Path: {path}")]
    FileIsEmpty { path: PathBuf },

    #[error("parsing failed. Path: {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: nom::Err<nom::error::Error<usize>>,
    },

    #[error("offset is invalid. Path: {path}")]
    OffsetIsInvalid { path: PathBuf },

    #[error(transparent)]
    FromBytesWithNul(#[from] core::ffi::FromBytesWithNulError),

    #[error(transparent)]
    FromBytesUntilNul(#[from] core::ffi::FromBytesUntilNulError),

    #[error(transparent)]
    Utf8(#[from] core::str::Utf8Error),
}

impl Error {
    pub(crate) fn from_nom_parse(
        source: nom::Err<nom::error::Error<&[u8]>>,
        bytes: &[u8],
        path: impl Into<PathBuf>,
    ) -> Self {
        Self::Parse {
            path: path.into(),
            source: source.map(|r| nom::error::Error {
                input: bytes.len().saturating_sub(r.input.len()),
                code: r.code,
            }),
        }
    }
}

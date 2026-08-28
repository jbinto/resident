#![allow(unsafe_code)]

use std::fs::File;
use std::ops::Deref;
use std::path::Path;

use memmap2::{Mmap, MmapOptions};

use crate::{Error, Result};

pub(crate) struct MappedFile {
    _file: File,
    map: Mmap,
}

impl MappedFile {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|source| Error::io(path, source))?;
        // SAFETY: generation shard files are immutable after publication. The open file is
        // retained for the map's lifetime, and writers always publish a different path.
        let map =
            unsafe { MmapOptions::new().map(&file) }.map_err(|source| Error::io(path, source))?;
        Ok(Self { _file: file, map })
    }
}

impl Deref for MappedFile {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

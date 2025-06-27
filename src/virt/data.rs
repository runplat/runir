use crate::{Data, Record, archive::JournalEntry};
use memmap2::Mmap;
use sha2::{Digest, Sha256};
use std::{ops::Deref, sync::Arc};

/// Virtual reference to journaled data
///
/// Uses a mmap'ed file to provide access to journaled data
#[derive(Debug, Clone)]
pub struct VirtualData {
    /// If packed, the mmap will not include the zero-bytes
    is_packed: bool,
    /// Journal entry for this virtual reference
    journaled: JournalEntry,
    /// Memory-map handle to data
    mmap: Arc<Mmap>,
}

/// Slim virtual data only stores offset/len and the backing data
///
/// Can only be constructed from VirtualData which does the validation
#[derive(Debug, Clone)]
pub struct VirtualDataSlim {
    /// Offset into the mmap
    offset: usize,
    /// Len of data
    len: usize,
    /// Memory-map handle to backing data
    mmap: Arc<Mmap>,
}

impl VirtualData {
    /// Returns a new virtual ref, if the provided arguments are valid
    ///
    /// Returns an error if the source/content digests could not be verified
    #[inline]
    pub fn new(journaled: JournalEntry, mmap: Arc<Mmap>) -> std::io::Result<Self> {
        let virt_ref = Self {
            is_packed: false,
            journaled,
            mmap,
        };
        if virt_ref.is_valid() {
            Ok(virt_ref)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Provided data did not match source/content digest constraints",
            ))
        }
    }

    /// Returns a new virtual ref, if the provided arguments are valid
    ///
    /// Returns an error if the source/content digests could not be verified
    ///
    /// Sets the packed flag so that the source confirmation can account for zero bytes
    #[inline]
    pub fn new_packed(journaled: JournalEntry, mmap: Arc<Mmap>) -> std::io::Result<Self> {
        let virt_ref = Self {
            is_packed: true,
            journaled,
            mmap,
        };
        if virt_ref.is_valid() {
            Ok(virt_ref)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Provided packed data did not match source/content digest constraints",
            ))
        }
    }

    /// Returns true if the source/content digests match the current settings
    #[inline]
    pub fn is_valid(&self) -> bool {
        let source_matches = if !self.is_packed {
            Sha256::digest(&self.mmap[..]).as_slice() == self.journaled.source()
        } else {
            let mut digest = Sha256::new();
            digest.update(&self.mmap[..]);
            let zero_bytes = [0; 512];
            digest.update(&zero_bytes);
            digest.update(&zero_bytes);
            digest.finalize().as_slice() == self.journaled.source()
        };
        source_matches && Sha256::digest(&self).as_slice() == self.journaled.content()
    }

    /// Returns the length in bytes
    #[inline]
    pub fn len(&self) -> usize {
        self.as_ref().len()
    }

    /// Returns true if the backing buffer is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.as_ref().is_empty()
    }

    /// Reverses back into a record if journaled data is a record extent
    #[inline]
    pub fn materialize(&self) -> Option<Record> {
        match &self.journaled {
            JournalEntry::Record(record_extent) => {
                record_extent.materialize(&Data::Virtual(self.to_slim()))
            }
            _ => None,
        }
    }

    /// Converts this reference into "slim" mode which removes the journal entry
    /// metadata used for validation
    #[inline]
    pub fn to_slim(&self) -> VirtualDataSlim {
        let (offset, len) = self.journaled.extent();
        VirtualDataSlim {
            offset: offset as usize,
            len: len as usize,
            mmap: self.mmap.clone(),
        }
    }
}

impl AsRef<[u8]> for VirtualData {
    fn as_ref(&self) -> &[u8] {
        &self
    }
}

impl Deref for VirtualData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        let (offset, len) = self.journaled.extent();
        &self.mmap[offset as usize..(offset + len as u64) as usize]
    }
}

impl AsRef<[u8]> for VirtualDataSlim {
    fn as_ref(&self) -> &[u8] {
        &self
    }
}

impl Deref for VirtualDataSlim {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.mmap[self.offset as usize..(self.offset + self.len)]
    }
}

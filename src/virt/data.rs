use std::{ops::Deref, sync::Arc};
use memmap2::Mmap;
use sha2::{Digest, Sha256};
use crate::archive::JournalEntry;

/// Virtual reference to journaled data
/// 
/// Uses a mmap'ed file to provide access to journaled data
#[derive(Debug, Clone)]
pub struct VirtualData {
    /// Journal entry for this virtual reference
    journaled: JournalEntry,
    /// Memory-map handle to data
    mmap: Arc<Mmap>,
}

impl VirtualData {
    /// Returns a new virtual ref, if the provided arguments are valid
    ///
    /// Returns an error if the source/content digetsts could not be verified
    #[inline]
    pub fn new(journaled: JournalEntry, mmap: Arc<Mmap>) -> std::io::Result<Self> {
        let virt_ref = Self { journaled, mmap };
        if virt_ref.is_valid() {
            Ok(virt_ref)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Provided data did not match source/content digest constraints",
            ))
        }
    }

    /// Returns true if the source/content digests match the current settings
    #[inline]
    pub fn is_valid(&self) -> bool {
        Sha256::digest(&self.mmap[..]).as_slice() == self.journaled.source()
            && Sha256::digest(&self).as_slice() == self.journaled.content()
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

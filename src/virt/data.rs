use crate::{Data, Record, archive::JournalEntry};
use bytes::Bytes;
use memmap2::Mmap;
use sha2::{Digest, Sha256};
use std::{fmt::Debug, ops::Deref, sync::Arc};
use tracing::trace;

use super::vol::FrozenMmap;

/// Virtual reference to journaled data
///
/// Uses a mmap'ed file to provide access to journaled data
#[derive(Debug, Clone)]
pub struct VirtualData {
    /// Journal entry for this virtual reference
    journaled: JournalEntry,
    /// Inner data pointer
    inner: BackingData,
}

/// Slim virtual data only stores offset/len and the backing data
///
/// Can only be constructed from VirtualData which does the validation
#[derive(Debug, Clone)]
pub struct VirtualDataSlim {
    /// Offset into the mmap
    pub(crate) offset: usize,
    /// Len of data
    pub(crate) len: usize,
    /// Inner data pointer to backing data
    pub(crate) inner: BackingData,
}

impl VirtualData {
    /// Returns a new virtual ref, if the provided arguments are valid
    ///
    /// Returns an error if the source/content digests could not be verified
    #[inline]
    pub fn new(journaled: JournalEntry, data: impl Into<BackingData>) -> std::io::Result<Self> {
        let virt_ref = Self {
            journaled,
            inner: data.into(),
        };
        if virt_ref.is_valid() {
            Ok(virt_ref)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Provided data did not match source/content digest constraints \n  Expected Source: {}\n  Actual Source: {}\n",
                    hex::encode(virt_ref.journaled.source()),
                    hex::encode(virt_ref.compute_source_digest())
                ),
            ))
        }
    }

    /// Returns true if the source/content digests match the current settings
    #[inline]
    pub fn is_valid(&self) -> bool {
        let source_matches = self.compute_source_digest().as_slice() == self.journaled.source();
        let content_matches = Sha256::digest(&self).as_slice() == self.journaled.content();

        trace!(
            source_matches,
            content_matches,
            joff = self.journaled.extent().0,
            jlen = self.journaled.extent().1
        );
        source_matches && content_matches
    }

    fn compute_source_digest(&self) -> [u8; 32] {
        Sha256::digest(&self.inner[..]).into()
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
            inner: self.inner.clone(),
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
        &self.inner[offset as usize..(offset + len as u64) as usize]
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
        &self.inner[self.offset as usize..(self.offset + self.len)]
    }
}

/// Backing-data is a normalized binary type
#[derive(Clone)]
pub struct BackingData(pub(crate) Arc<dyn Deref<Target = [u8]> + Sync + Send + 'static>);

impl BackingData {
    /// Finds data from backing data
    /// 
    /// Returns Data::Virtual if found
    #[inline]
    pub fn find_data(&self, data: &[u8]) -> Option<Data> {
        self.windows(data.len())
            .enumerate()
            .find(|(_, d)| *d == data)
            .map(|(offset, _)| {
                Data::Virtual(VirtualDataSlim {
                    offset,
                    len: data.len(),
                    inner: self.clone(),
                })
            })
    }
}

impl Debug for BackingData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("InnerData").finish()
    }
}

impl Deref for BackingData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl From<Arc<Mmap>> for BackingData {
    fn from(value: Arc<Mmap>) -> Self {
        Self(value)
    }
}

impl From<Bytes> for BackingData {
    fn from(value: Bytes) -> Self {
        Self(Arc::new(value))
    }
}

impl From<FrozenMmap> for BackingData {
    fn from(value: FrozenMmap) -> Self {
        Self(Arc::new(value))
    }
}

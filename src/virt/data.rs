use crate::{Data, Record, archive::JournalEntry};
use sha2::{Digest, Sha256};
use std::{fmt::Debug, ops::Deref};
use tracing::trace;

/// Virtual reference to journaled data
#[derive(Debug, Clone)]
pub struct Virtual {
    /// Journal entry for this virtual reference
    journaled: JournalEntry,
    /// Inner data pointer
    inner: Data,
}

impl Virtual {
    /// Returns a new virtual ref, if the provided arguments are valid
    ///
    /// Returns an error if the source/content digests could not be verified
    #[inline]
    pub fn new(journaled: JournalEntry, data: impl Into<Data>) -> std::io::Result<Self> {
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

    /// Reverses back into a record if journaled data is a record extent
    #[inline]
    pub fn materialize(&self) -> Option<Record> {
        match &self.journaled {
            JournalEntry::Record(record_extent) => {
                record_extent.materialize(&Data::from(self.clone()))
            }
            _ => None,
        }
    }

    #[inline]
    fn compute_source_digest(&self) -> [u8; 32] {
        self.inner.digest().finalize().into()
    }
}

impl AsRef<[u8]> for Virtual {
    fn as_ref(&self) -> &[u8] {
        &self
    }
}

impl Deref for Virtual {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        let (offset, len) = self.journaled.extent();
        &self.inner[offset as usize..(offset + len as u64) as usize]
    }
}

impl From<Virtual> for Data {
    fn from(value: Virtual) -> Self {
        let (offset, len) = value.journaled.extent();
        value.inner.view(offset as usize, len as usize)
    }
}

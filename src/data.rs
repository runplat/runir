use bytes::Bytes;
use sha2::{digest::Update, Sha256};
use crate::virt::VirtualDataSlim;

/// Enumeration of different data implementations
#[derive(Default, Debug, Clone)]
pub enum Data {
    /// Data has not been set
    #[default]
    Empty,
    /// Data is loaded into memory
    Bytes(Bytes),
    /// Data is stored virtually w/ a reference to a journal entry and mmap
    Virtual(VirtualDataSlim),
}

impl Data {
    /// Returns true if data is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes().is_empty()
    }

    /// Returns the len in bytes of data
    #[inline]
    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    /// Returns a slice of the bytes in data
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Data::Empty => &[],
            Data::Bytes(bytes) => &bytes,
            Data::Virtual(virt_ref) => &virt_ref
        }
    }

    /// Returns a SHA256 digest of this data
    #[inline]
    pub fn digest(&self) -> Sha256 {
        let mut digest = Sha256::default();
        digest.update(self.bytes());
        digest
    }
}

impl From<Bytes> for Data {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}
use crate::virt::VirtualDataSlim;
use bytes::Bytes;
use sha2::{Sha256, digest::Update};

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
            Data::Virtual(virt_ref) => &virt_ref,
        }
    }

    /// Returns a SHA256 digest of this data
    #[inline]
    pub fn digest(&self) -> Sha256 {
        let mut digest = Sha256::default();
        digest.update(self.bytes());
        digest
    }

    /// Returns an inner view of this data, returning a new instance of Data
    ///
    /// Note: offset is treated as relative to the current view of data
    #[inline]
    pub fn view(&self, offset: usize, len: usize) -> Data {
        match self {
            Data::Empty => Data::Empty,
            Data::Bytes(bytes) => Data::Bytes(bytes.slice(offset..offset + len)),
            Data::Virtual(virtual_data_slim) => Data::Virtual(VirtualDataSlim {
                offset: virtual_data_slim.offset + offset,
                len,
                inner: virtual_data_slim.inner.clone(),
            }),
        }
    }

    /// Finds a view within Data and returns a new Data
    #[inline]
    pub fn find_view(&self, view: &[u8]) -> Option<Data> {
        match self {
            Data::Empty => None,
            _ => self
                .bytes()
                .windows(view.len())
                .enumerate()
                .find(|(_, v)| *v == view)
                .map(|(offset, _)| self.view(offset, view.len())),
        }
    }
}

impl From<Bytes> for Data {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}

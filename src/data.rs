use std::ops::Deref;

use bytes::Bytes;
use sha2::{Sha256, digest::Update};

/// Wraps Bytes struct to offer additional functions
#[derive(Default, Debug, Clone)]
pub struct Data {
    data: Bytes,
}

impl Data {
    /// Returns a SHA256 digest of this data
    #[inline]
    pub fn digest(&self) -> Sha256 {
        let mut digest = Sha256::default();
        digest.update(self);
        digest
    }

    /// Returns an inner view of this data, returning a new instance of Data
    ///
    /// Note: offset is treated as relative to the current view of data
    #[inline]
    pub fn view(&self, offset: usize, len: usize) -> Data {
        Self {
            data: self.data.slice(offset..offset + len),
        }
    }

    /// Finds a view within Data and returns a new Data
    #[inline]
    pub fn find_view(&self, view: &[u8]) -> Option<Data> {
        self.windows(view.len())
            .enumerate()
            .find(|(_, v)| *v == view)
            .map(|(offset, _)| self.view(offset, view.len()))
    }

    /// Returns a reference to the inner Bytes
    #[inline]
    pub fn as_bytes(&self) -> &Bytes {
        &self.data
    }
}

impl From<Bytes> for Data {
    fn from(value: Bytes) -> Self {
        Self { data: value }
    }
}

impl AsRef<[u8]> for Data {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Deref for Data {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

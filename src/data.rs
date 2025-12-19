use std::ops::Deref;

use bytes::Bytes;
use rayon::iter::{ParallelBridge, ParallelIterator};
use sha2::{Sha256, digest::Update};

use crate::Namespace;

type ShaDigest<D> = sha2::digest::Output<D>;

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
    pub fn find_view(&self, view: &[u8]) -> Option<(u64, Data)> {
        self.windows(view.len())
            .enumerate()
            .find(|(_, v)| *v == view)
            .map(|(offset, _)| (offset as u64, self.view(offset, view.len())))
    }

    /// Finds a view based on a descriptor
    #[inline]
    pub fn find_cas_view_offset<D: sha2::Digest>(&self, len: usize, digest: ShaDigest<D>) -> Option<(u64, Data)> {
        // TODO: This is probably really slow tbh because a digest needs to get computed per window..
        self.windows(len)
            .enumerate()
            .find(|(_, v)| {
                D::digest(v) == digest
            })
            .map(|(offset, _)| (offset as u64, self.view(offset, len)))
    }

    /// (Parallel) Finds a view based on a length/digest
    #[inline]
    pub fn par_find_cas_view_offset<D: sha2::Digest>(&self, len: usize, digest: ShaDigest<D>) -> Option<(u64, Data)> {
        // TODO: This is probably really slow tbh because a digest needs to get computed per window..
        self.windows(len)
            .enumerate()
            .find(|(_, v)| {
                D::digest(v) == digest
            })
            .map(|(offset, _)| (offset as u64, self.view(offset, len)))
    }

    /// Finds a view based on a len/namespace and digest checksum
    #[inline]
    pub fn find_ns_view_offset<D: sha2::Digest>(&self, len: usize, ns: &Namespace, dchk: u64) -> Option<(u64, Data, ShaDigest<D>)> {
        // TODO: This is probably really slow tbh because a digest needs to get computed per window..
        self.windows(len)
            .enumerate()
            .find_map(|(offset, v)| {
                let digest = D::digest(v);
                if ns.key(digest.as_slice()) == dchk {
                    Some((offset, digest))
                } else {
                    None
                }
            })
            .map(|(offset, digest)| (offset as u64, self.view(offset, len), digest))
    }

    /// (Parallel) Finds a view based on a len/namespace and digest checksum
    #[inline]
    pub fn par_find_ns_view_offset<D: sha2::Digest>(&self, len: usize, ns: &Namespace, dchk: u64) -> Option<(u64, Data, ShaDigest<D>)> {
        /*
            This will search for windows
        */
        self.windows(len)
            .enumerate()
            .par_bridge()
            .find_map_any(|(offset, v)| {
                let digest = D::digest(v);
                if ns.key(digest.as_slice()) == dchk {
                    Some((offset, v, digest))
                } else {
                    None
                }
            })
            .map(|(offset, _, digest)| (offset as u64, self.view(offset, len), digest))
    }

    /// Returns a reference to the inner Bytes
    #[inline]
    pub fn as_bytes(&self) -> &Bytes {
        &self.data
    }
}

impl From<&[u8]> for Data {
    fn from(value: &[u8]) -> Self {
        Bytes::copy_from_slice(value).into()
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

#[cfg(test)]
mod test {
    use crate::{util::PeekExtensions, Namespace};

    #[test]
    fn test_data_util_peek_ext() {
        let ns = Namespace::ephemeral();

        let rec = ns.store("test", &toml::toml! {
            name = "test"
        });

        assert_eq!("test", rec.data.at("name").str().unwrap());
    }
}
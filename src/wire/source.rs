use std::fmt::Display;
use crate::{Data, Symbol};

/// Byte-length of a SHA256 Digest
const SHA256_SIZE: usize = 32;

/// Defines a "content" address which can be used for indexed content lookup
pub struct ContentAddress<'lookup, const SHA_SIZE: usize = SHA256_SIZE> {
    pub len: u64,
    pub digest: &'lookup [u8; SHA_SIZE],
}

/// Trait for sources to provide Data from a content address
pub trait Source {
    /// Returns a Data source provided a content address
    fn source(&self, address: &ContentAddress) -> Option<Data>;
}

impl<'l, const SHA_SIZE: usize> Symbol for ContentAddress<'l, SHA_SIZE> {
    fn symbol(&self) -> impl std::hash::Hash {
        self.digest.as_slice()
    }
}

impl<'l, const SHA_SIZE: usize> Display for ContentAddress<'l, SHA_SIZE> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.len, hex::encode(self.digest))
    }
}
use bytes::Bytes;
use sha2::{Digest, Sha256};

use super::{header::EMPTY_HEADER, DigestBuffer, Header};

/// Enumeration of archive entry types
pub enum Entry {
    /// Regular file entry
    Regular(FileEntry),
    /// Regular file entry reference
    RegularReference(FileReference),
    /// Other entry type
    Other(Header),
    /// Pending an actual entry
    /// 
    /// Returned by the decoder to indicate that it is currently processing an actual entry
    Pending,
    /// 512-byte zero block entry
    Zeros,
}

/// Archived file entry reference
/// 
/// Instead of storing the bytes of the actual file, this
/// returns an offset into the source buffer for the start of the file content,
/// and the expected digest of the stored content.
pub struct FileReference {
    /// Archive header
    pub header: Header,
    /// Digest buffer of the stored content
    pub digest: DigestBuffer,
    /// Offset into the source for the start of the file entry content
    pub offset: u64
}

/// Archived file entry
pub struct FileEntry {
    /// Archive header
    pub header: Header,
    /// Data for this entry
    pub data: Bytes,
    /// Calculated sha2 digest buffer
    pub digest: DigestBuffer,
}

impl Entry {
    /// Returns a new archive regular file entry
    pub fn regular(header: Header, data: Bytes) -> Self {
        let digest = Sha256::digest(&data);
        Self::Regular(FileEntry {
            header,
            data,
            digest: digest.into(),
        })
    }

    /// Returns a new archive entry
    pub fn other(header: Header) -> Self {
        Self::Other(header)
    }

    /// Returns the parsed entry header
    pub fn header(&self) -> &Header {
        match self {
            Entry::Regular(reg) => &reg.header,
            Entry::RegularReference(reg) => &reg.header,
            Entry::Other(h) => &h,
            Entry::Zeros => &EMPTY_HEADER,
            Entry::Pending => &EMPTY_HEADER,
        }
    }

    /// Returns data if this entry contains data w/ a digest string
    pub fn data(&self) -> Option<(Bytes, [u8; 32])> {
        match self {
            Entry::Regular(reg) => Some((
                reg.data.clone(),
                reg.digest.clone(),
            )),
            _ => None,
        }
    }

    /// Returns true if this is a zeroes/padding entry
    pub fn is_zeroes(&self) -> bool {
        match self {
            Entry::Zeros => true,
            _ => false
        }
    }
}

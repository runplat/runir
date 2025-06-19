use bytes::Bytes;

use super::{header::EMPTY_HEADER, DigestBuffer, Header};

/// Archive entry
pub enum Entry {
    /// Regular file entry
    Regular(FileEntry),
    /// Other entry type
    Other(Header),
    /// EOA entry
    EOA,
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
    pub fn regular(header: Header, data: Bytes, digest: DigestBuffer) -> Self {
        Self::Regular(FileEntry {
            header,
            data,
            digest,
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
            Entry::Other(h) => &h,
            Entry::EOA => &EMPTY_HEADER,
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

    /// Returns true if this is an EOA entry
    pub fn is_eoa(&self) -> bool {
        match self {
            Entry::EOA => true,
            _ => false
        }
    }
}

use bytes::Bytes;
use sha2::{Digest, Sha256};
use crate::{Opts, Record};
use super::{Sha256Digest, Header, header::EMPTY_HEADER};

/// Enumeration of archive entry types
pub enum Entry {
    /// Record file entry
    Record(Record),
    /// Regular file entry
    Regular(FileEntry),
    /// Other entry type
    Other(Header),
    /// Pending an actual entry
    ///
    /// Returned by the decoder to indicate that it is currently processing an actual entry
    Pending,
    /// 512-byte zero block entry
    Zeros,
}

/// Archived file entry
pub struct FileEntry {
    /// Archive header
    pub header: Header,
    /// Data for this entry
    pub data: Bytes,
    /// Calculated sha2 digest buffer
    pub digest: Sha256Digest,
}

impl Entry {
    /// Returns a new archive regular file entry
    #[inline]
    pub fn regular(header: Header, data: Bytes) -> Self {
        let digest = Sha256::digest(&data);
        Self::Regular(FileEntry {
            header,
            data,
            digest: digest.into(),
        })
    }

    /// Returns a header-only archive entry
    #[inline]
    pub fn other(header: Header) -> Self {
        Self::Other(header)
    }

    /// Returns the parsed entry header if available
    #[inline]
    pub fn header(&self) -> Header {
        match self {
            Entry::Record(r) => r.make_archive_header().expect("must be able to return archive header"),
            Entry::Regular(reg) => reg.header.clone(),
            Entry::Other(h) => h.clone(),
            Entry::Zeros => EMPTY_HEADER.clone(),
            Entry::Pending => EMPTY_HEADER.clone(),
        }
    }

    /// Returns data if this entry contains data w/ a digest string
    #[inline]
    pub fn data(&self) -> Option<(Bytes, [u8; 32])> {
        match self {
            Entry::Regular(reg) => Some((reg.data.clone(), reg.digest.clone())),
            Entry::Record(rec) => rec.archive().ok().and_then(|e| match e {
                Entry::Regular(reg) => Some((reg.data.clone(), reg.digest.clone())),
                _ => None,
            }),
            _ => None,
        }
    }

    /// Returns the opts if the entry is a record
    #[inline]
    pub fn opts(&self) -> Option<Opts> {
        match self {
            Entry::Record(record) => Some(*record.opts()),
            _ => None,
        }
    }

    /// Returns the crc checksum if the entry is a record
    #[inline]
    pub fn crc(&self) -> Option<u64> {
        match self {
            Entry::Record(record) => Some(record.uuid().as_u64_pair().1),
            _ => None,
        }
    }

    /// Returns true if this is a zeroes/padding entry
    #[inline]
    pub fn is_zeroes(&self) -> bool {
        matches!(self, Entry::Zeros)
    }
}

impl From<Record> for Entry {
    fn from(value: Record) -> Self {
        Entry::Record(value)
    }
}

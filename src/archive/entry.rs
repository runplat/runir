use super::{Header, JournalEntry, Sha256Digest, header::EMPTY_HEADER};
use crate::{Data, IRecord, Opts, Record};
use bytes::Bytes;
use sha2::{Digest, Sha256};

/// Enumeration of archive entry types
pub enum Entry {
    /// Record file entry
    ///
    /// This will lazily call Record::archive(..) and allows for a record extent to be journaled
    Record(Record),
    /// Regular file entry
    Regular(FileEntry),
    /// Entry can be found on disk, and has been journaled
    Journaled {
        header: Header,
        data: Data,
    },
    /// Reference entry
    Reference(FileEntryReference),
    /// Other entry type
    Other(Header),
    /// Pending an actual entry
    ///
    /// Returned by the decoder to indicate that it is currently processing an actual entry
    Pending,
    /// During encoding, this variant indicates to the encoder the end of the encoding,
    /// the encoder can then append 2 x 512 blocks
    /// 
    /// During decoding, this variant represents a single 512-block, since we decode frame by frame
    Zeros,
}

/// Archived file reference entry
pub struct FileEntryReference {
    /// Archive header
    pub(crate) header: Header,
    /// Digest of archive entry's data
    pub(crate) digest: Sha256,
    /// Offset into source archive to the start of entry data
    pub(crate) offset: usize,
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
    /// Returns a new journaled entry from journaled data
    #[inline]
    pub fn from_journaled(header: Header, data: Data) -> Self {
        Self::Journaled { header, data }
    }

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
            Entry::Record(r) => r
                .make_archive_header()
                .expect("must be able to return archive header"),
            Entry::Regular(reg) => reg.header.clone(),
            Entry::Reference(reference) => reference.header.clone(),
            Entry::Journaled { header, .. } => header.clone(),
            Entry::Other(h) => h.clone(),
            Entry::Zeros => EMPTY_HEADER.clone(),
            Entry::Pending => EMPTY_HEADER.clone(),
        }
    }

    /// Returns data if this entry contains data w/ a digest string
    #[inline]
    pub fn data(&self) -> Option<(Data, [u8; 32])> {
        match self {
            Entry::Regular(reg) => Some((Data::from(reg.data.clone()), reg.digest.clone())),
            Entry::Journaled { data, .. } => {
                let data = Data::from(data.clone());
                let digest = data.digest().finalize().into();
                Some((data, digest))
            }
            Entry::Record(rec) => rec.archive().ok().and_then(|e| match e {
                Entry::Regular(reg) => Some((Data::from(reg.data), reg.digest.clone())),
                Entry::Journaled { data, .. } => {
                    let data = Data::from(data.clone());
                    let digest = data.digest().finalize().into();
                    Some((data, digest))
                }
                _ => None,
            }),
            Entry::Reference(FileEntryReference { digest, .. }) => Some((Data::default(), digest.clone().finalize().into())),
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

    /// Creates a journal entry from the current entry at offset
    pub fn create_journal_entry(&self, offset: usize) -> Option<JournalEntry> {
        match self {
            Entry::Record(record) => {
                let (id, data, ns_chk, ts, opts) = record.clone().into_parts();
                let (key, crc) = id.as_u64_pair();
                Some(JournalEntry::Record(crate::RecordExtent {
                    source: [0; 32],
                    content: data.digest().finalize().into(),
                    offset: offset as u64,
                    len: record.bytes().len() as u32,
                    key,
                    crc,
                    ts,
                    ns_chk,
                    opts: opts.encode(),
                }))
            }
            Entry::Regular(file_entry) => Some(JournalEntry::Extent {
                source: [0; 32],
                content: file_entry.digest.clone(),
                offset: offset as u64,
                len: file_entry.data.len() as u32,
            }),
            Entry::Journaled { header, data } => {
                Some(JournalEntry::Extent {
                    source: [0; 32],
                    content: Data::from(data.clone()).digest().finalize().into(),
                    offset: offset as u64,
                    len: header.size() as u32,
                })
            }
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

use super::{Entry, Manifest, Sha256Digest};
use crate::{Namespace, RecordableExtensions, virt::RecordExtent};
use bytes::{BufMut, Bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Enumeration of encoded entry metadata collected by the tape encoder
#[derive(Serialize, Deserialize, Clone)]
pub enum JournalEntry {
    Extent {
        source: Sha256Digest,
        content: Sha256Digest,
        offset: u64,
        len: u32,
    },
    Record(RecordExtent),
}

impl JournalEntry {
    /// If this journal entry is a record extent, returns the archive name of this journal entry
    #[inline]
    pub fn archve_name(&self) -> Option<String> {
        match self {
            JournalEntry::Record(record_extent) => Some(record_extent.format_archve_name()),
            _ => None,
        }
    }

    /// Returns the content digest of the journaled entry's data
    #[inline]
    pub fn source(&self) -> &Sha256Digest {
        match self {
            JournalEntry::Extent { source, .. } => &source,
            JournalEntry::Record(record_extent) => &record_extent.source,
        }
    }

    /// Returns the content digest of the journaled entry's data
    #[inline]
    pub fn content(&self) -> &Sha256Digest {
        match self {
            JournalEntry::Extent { content, .. } => &content,
            JournalEntry::Record(record_extent) => &record_extent.content,
        }
    }

    /// Returns an offset, len tuple
    #[inline]
    pub fn extent(&self) -> (u64, u32) {
        match self {
            JournalEntry::Extent { offset, len, .. } => (*offset, *len),
            JournalEntry::Record(record_extent) => (record_extent.offset, record_extent.len),
        }
    }
}

/// Simple tape archive encoder,
///
/// **NOTE** Does not try to generate a header, just re-packs an unpacked archive entry.
///
#[derive(Default)]
pub struct TapeEncoder {
    /// Journal of encoded entries,
    journal: Vec<JournalEntry>,
    /// Digest data being encoded
    digest: Sha256,
}

impl TapeEncoder {
    /// Ensures a fresh state for the next stamp
    #[inline]
    pub fn next_stamp(&mut self) {
        self.journal.clear();
        self.digest = Sha256::new();
    }

    /// Stamps a manifest for the current state and clears the journal
    #[inline]
    pub fn stamp_manifest(&mut self) -> Manifest {
        let source_digest = self.digest.clone().finalize();
        for enc in self.journal.iter_mut() {
            match enc {
                JournalEntry::Extent { source, .. } => {
                    source.copy_from_slice(source_digest.as_slice());
                }
                JournalEntry::Record(record_extent) => {
                    record_extent.source = source_digest.into();
                }
            }
        }

        let manifest_name = format!("MANIFEST_{:x}", self.digest.clone().finalize());
        let mut record =
            Namespace::new("__runir_store").store(manifest_name.as_str(), self.journal.indexable());
        record.opts_mut().set_manifest_spec(true);

        self.journal.clear();
        self.digest = Sha256::new();
        Manifest { record }
    }

    /// Encodes an entry to a destination buffer
    #[inline]
    pub fn encode_to(
        &mut self,
        item: Entry,
        dst: &mut bytes::BytesMut,
    ) -> Result<(), std::io::Error> {
        if item.is_zeroes() {
            dst.reserve(512 * 2);
            let zero_block = zero_block();
            self.put_update(dst, &zero_block);
            self.put_update(dst, &zero_block);
            Ok(())
        } else {
            dst.reserve(item.header().size());
            let header_bytes = item.header();
            self.put_update(dst, header_bytes.as_ref());
            if let Some((data, _)) = item.data() {
                let offset = dst.len();
                let len = data.len();
                dst.reserve(len);
                let padding = len % 512;
                self.put_update(dst, &data.bytes());
                self.put_update(dst, &vec![0; 512 - padding]);
                if let Some(journal_entry) = item.create_journal_entry(offset) {
                    self.journal.push(journal_entry);
                }
            }
            Ok(())
        }
    }

    /// Puts an update into a dst buffer and updates an internal digester
    #[inline]
    fn put_update(&mut self, dst: &mut bytes::BytesMut, update: &[u8]) {
        dst.put(update);
        self.digest.update(update);
    }
}

/// Returns a 512-byte zero-block which is used to end entries, and files
///
fn zero_block() -> Bytes {
    Bytes::from_iter(std::iter::repeat('\0' as u8).take(512))
}

impl asynchronous_codec::Encoder for TapeEncoder {
    type Item<'a> = Entry;

    type Error = std::io::Error;

    fn encode(&mut self, item: Self::Item<'_>, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        self.encode_to(item, dst)
    }
}

impl std::fmt::Debug for JournalEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Extent {
                source,
                content,
                offset,
                len,
            } => f
                .debug_struct("Extent")
                .field("source", &hex::encode(source))
                .field("content", &hex::encode(content))
                .field("offset", offset)
                .field("len", len)
                .finish(),
            Self::Record(arg0) => f.debug_tuple("Record").field(arg0).finish(),
        }
    }
}

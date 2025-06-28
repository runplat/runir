use super::TextMetadata;
use crate::{archive::{JournalEntry, Sha256Digest}, virt::RecordExtent, IRecord, Namespace, Record};
use flexbuffers::{MapReader, Reader};
use std::{collections::BTreeMap, fmt::Debug};
use tracing::{debug, warn};
use uuid::Uuid;

/// Indexer is an additional module that can index flexbuffer based records
///
/// When invoked, it can scan the record and index fields for later search/querying purposes
#[derive(Debug, Default)]
pub struct Indexer {
    /// Values indexed by this indexer
    ///
    /// The keys are the record's (uuid-hi ^ ns_chk) value and a hash of name of the field for this record as the uuid-lo
    values: BTreeMap<uuid::Uuid, Index>,
    /// Index of journal entries
    journaled: Vec<JournalEntry>,
}

impl Indexer {
    /// Scans a record and updates the indexer's state
    #[inline]
    pub fn scan_update(&mut self, record: &Record) {
        if record.is_valid() && record.opts().is_indexable() {
            let (hi, _) = record.uuid().as_u64_pair();
            let uuid_hi = hi ^ record.ns_chk();

            if let Some(reader) = flexbuffers::Reader::get_root(record.bytes()).ok() {
                if reader.flexbuffer_type().is_map() {
                    self.index_flexbuffer_map(uuid_hi, reader);
                } else if reader.flexbuffer_type().is_vector() {
                    let vec = reader.as_vector();
                    for v in vec.iter() {
                        if v.flexbuffer_type().is_map() {
                            self.index_flexbuffer_map(uuid_hi, v);
                        }
                    }
                }
            }
        }
    }

    /// Returns an iterator over all record-ids w/ matching text fields containing search query
    #[inline]
    pub fn contains_text(&self, field: &str, text: &str) -> impl Iterator<Item = u64> {
        let ns = Namespace::from("___runir__INDEXER");
        let field_key = ns.key(field);

        self.values.iter().filter_map(move |(k, v)| {
            let (rec_id, field_id) = k.as_u64_pair();
            if field_id == field_key {
                match v {
                    Index::Text(text_meta) => {
                        if text_meta.contains(text) {
                            Some(rec_id)
                        } else {
                            None
                        }
                    }
                }
            } else {
                None
            }
        })
    }

    /// Returns true if the indexer contains a journal entry for content
    #[inline]
    pub fn contains_content(&self, content: Sha256Digest) -> bool {
        self.journaled.iter().find(|k| k.content().cmp(&content).is_eq()).is_some()
    }

    /// Returns an iterator over indexed journaled record extents
    #[inline]
    pub fn record_extents(&self) -> impl Iterator<Item = &RecordExtent> {
        self.journaled.iter().filter_map(|e| match e {
            JournalEntry::Record(record_extent) => Some(record_extent),
            _ => None
        })
    }

    /// Returns an iterator over journaled entries
    #[inline]
    pub fn journaled(&self) -> impl Iterator<Item = &JournalEntry> {
        self.journaled.iter()
    }

    /// Absorbs other index data into this indexer
    ///
    /// In cases of collisions, incoming will always replace the existing value
    #[inline]
    pub fn absorb_indexer(&mut self, incoming: Indexer) {
        for (k, v) in incoming.values {
            let collision = self.values.insert(k, v);
            if let Some(_) = collision {
                // This is allowed, and could be intentional
                // ex. updating the state of Indexer in place
                // However, log that it happened in case it wasn't intentional
                debug!("Replacing index at {k}");
            }
        }
    }

    /// Indexes a flexbuffer map
    fn index_flexbuffer_map(&mut self, uuid_hi: u64, reader: Reader<&[u8]>) {
        let ns = Namespace::from("___runir__INDEXER");
        let map = reader.as_map();
        for k in map.iter_keys() {
            let val = map.idx(k);
            if val.flexbuffer_type().is_map() {
                if k == "Record" {
                    self.index_journal_record_extent(val.as_map());
                } else if k == "Extent" {
                    self.index_extent(val.as_map());
                 } else {
                    self.index_flexbuffer_map(uuid_hi, val);
                }
            } else if val.flexbuffer_type().is_string() {
                let uuid_lo = ns.key(k);
                self.values
                    .entry(Uuid::from_u64_pair(uuid_hi, uuid_lo))
                    .or_insert_with(|| TextMetadata::from(val.as_str()).into());
            } else {
                debug!("{k}: {:?}", val.flexbuffer_type());
            }
        }
    }

    /// Indexes a journal record extent
    fn index_journal_record_extent(&mut self, map: MapReader<&[u8]>) {
        if let Some(keys) =
            map_all(&map, &["source", "content", "offset", "len", "key", "ns_chk", "ts", "crc", "opts"])
        {
            match &keys[..] {
                [source, content, offset, len, key, ns_chk, ts, crc, opts, ..] => {
                    let extent = RecordExtent {
                        source: read_sha256_digest(source),
                        content: read_sha256_digest(content),
                        offset: offset.as_u64(),
                        len: len.as_u32(),
                        key: key.as_u64(),
                        ns_chk: ns_chk.as_u64(),
                        ts: ts.as_u64(),
                        crc: crc.as_u64(),
                        opts: opts.as_u64(),
                    };
                    self.journaled.push(JournalEntry::Record(extent));
                }
                _ => unreachable!()
            }
        } else {
            warn!("Could not find all keys for record struct");
        }
    }

    /// Indexes a journal extent
    fn index_extent(&mut self, map: MapReader<&[u8]>) {
        if let Some(keys) =
            map_all(&map, &["source", "content", "offset", "len"])
        {
            match &keys[..] {
                [source, content, offset, len, ..] => {
                    let extent = JournalEntry::Extent {
                        source: read_sha256_digest(source),
                        content: read_sha256_digest(content),
                        offset: offset.as_u64(),
                        len: len.as_u32(),
                    };
                    self.journaled.push(extent);
                }
                _ => unreachable!()
            }
        } else {
            warn!("Could not find all keys for record struct");
        }
    }
}

/// Enumeration of index types indexer can produce
pub enum Index {
    /// Indexed text metadata
    Text(TextMetadata),
}

impl From<TextMetadata> for Index {
    fn from(value: TextMetadata) -> Self {
        Index::Text(value)
    }
}

impl Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(_) => f.debug_tuple("Text").finish(),
        }
    }
}

fn read_sha256_digest(reader: &Reader<&[u8]>) -> Sha256Digest {
    let source_vec = reader.as_vector().iter().map(|a| a.as_u8()).take(32);
    let mut source = [0; 32];
    source.copy_from_slice(&source_vec.collect::<Vec<_>>());
    source
}

fn map_all<'a: 'b, 'b>(map: &'a MapReader<&[u8]>, keys: &[&str]) -> Option<Vec<Reader<&'b [u8]>>> {
    let mut _keys = vec![];
    for k in keys {
        if let Some(k) = map.index_key(k) {
            _keys.push(map.idx(k));
        }
    }
    Some(_keys).filter(|k| k.len() == keys.len())
}

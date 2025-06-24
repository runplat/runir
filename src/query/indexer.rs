use super::TextMetadata;
use crate::{Record, record::Namespace};
use std::{collections::BTreeMap, fmt::Debug};
use tracing::debug;
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
}

impl Indexer {
    /// Scans a record and updates the indexer's state
    #[inline]
    pub fn scan_update(&mut self, record: &Record) {
        let (hi, _) = record.uuid().as_u64_pair();
        let uuid_hi = hi ^ record.ns_chk();

        let ns = Namespace::from("___runir__INDEXER");
        if let Some(reader) = flexbuffers::Reader::get_root(record.data().bytes()).ok() {
            if reader.flexbuffer_type().is_map() {
                let map = reader.as_map();
                for k in map.iter_keys() {
                    let val = map.idx(k);
                    if val.flexbuffer_type().is_string() {
                        let uuid_lo = ns.key(k);
                        self.values
                            .entry(Uuid::from_u64_pair(uuid_hi, uuid_lo))
                            .or_insert_with(|| TextMetadata::from(val.as_str()).into());
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

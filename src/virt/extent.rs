use serde::{Deserialize, Serialize};
use sha2::Digest;
use tracing::error;
use uuid::Uuid;

use crate::{Data, Opts, Record, archive::Sha256Digest};

/// Provides offset/len information of stored record data in addition
/// to source and storage metadata
#[derive(Serialize, Deserialize, Clone)]
pub struct RecordExtent {
    /// Digest of record source
    pub(crate) source: Sha256Digest,
    /// Digest of record content
    pub(crate) content: Sha256Digest,
    /// Offset into file storing record data
    pub(crate) offset: u64,
    /// Length of record data
    pub(crate) len: u32,
    /// Record key
    pub(crate) key: u64,
    /// CRC-checksum of record data
    pub(crate) crc: u64,
    /// Timestamp of when the record was created
    pub(crate) ts: u64,
    /// Record ns_chk vlaue
    pub(crate) ns_chk: u64,
    /// Encoded record opts
    pub(crate) opts: u64,
}

impl RecordExtent {
    /// Formats the archive name used for this record
    #[inline]
    pub fn format_archive_name(&self) -> String {
        format!(
            "{:x}_{}_{:x}",
            self.ns_chk,
            uuid::Uuid::from_u64_pair(self.key, self.crc),
            self.opts
        )
    }

    /// Materializes a record from extent w/ data
    ///
    /// Returns None if the materialized record is not valid
    #[inline]
    pub fn materialize(&self, data: &Data) -> Option<Record> {
        let rec = Record::from_parts((
            Uuid::from_u64_pair(self.key, self.crc),
            data.clone(),
            self.ns_chk,
            self.ts,
            Opts::decode(self.opts),
        ));

        let rec_is_valid = rec.is_valid();
        let digest: [u8; 32] = data.digest().finalize().into();
        if rec_is_valid && digest == self.content {
            Some(rec)
        } else {
            error!(
                is_valid = rec_is_valid,
                data=hex::encode(digest),
                expected=hex::encode(self.content),
                "Could not materialize record extent from provided data"
            );
            None
        }
    }
}

impl std::fmt::Debug for RecordExtent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordExtent")
            .field("source", &hex::encode(self.source))
            .field("content", &hex::encode(self.content))
            .field("offset", &self.offset)
            .field("len", &self.len)
            .field(
                "ts",
                &time::UtcDateTime::from_unix_timestamp(self.ts as i64)
                    .map(|ts| format!("{ts:?}"))
                    .unwrap_or_else(|_| String::default()),
            )
            .field("crc", &self.crc)
            .field("opts", &self.opts)
            .finish()
    }
}

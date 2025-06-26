use serde::{Deserialize, Serialize};

use crate::archive::Sha256Digest;

/// Provides offset/len information of stored record data in addition
/// to source and storage metadata
#[derive(Serialize, Deserialize, Clone)]
pub struct RecordExtent {
    /// Digest of recourd source
    pub(crate) source: Sha256Digest,
    /// Digest of record content
    pub(crate) content: Sha256Digest,
    /// Offset into file storing record data
    pub(crate) offset: u64,
    /// Length of record data
    pub(crate) len: u32,
    /// Timestamp of when the record was created
    pub(crate) ts: u64,
    /// CRC-checksum of record data
    pub(crate) crc: u64,
    /// Encoded record opts
    pub(crate) opts: u64,
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

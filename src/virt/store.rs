use crate::archive::{JournalEntry, Sha256Digest};
use ahash::HashMap;
use memmap2::{Mmap, MmapOptions};
use std::sync::Arc;
use tracing::error;

use super::VirtualRef;

/// Virtual store maps source digests to a mmap
pub struct VirtualStore {
    /// File handle providing the Mmaps for this store
    _file: std::fs::File,
    /// Map of Mmap sources
    mmaps: HashMap<Sha256Digest, Arc<Mmap>>,
}

impl VirtualStore {
    /// Constructs a virtual store from a list journaled entries
    #[inline]
    pub fn new(file: std::fs::File, journaled: Vec<JournalEntry>) -> std::io::Result<Self> {
        let mut maps = HashMap::default();
        for entry in journaled.iter() {
            let (offset, len) = entry.extent();
            let mmap = unsafe {
                MmapOptions::new()
                    .offset(offset)
                    .len(len as usize)
                    .map(&file)?
            };
            maps.insert(*entry.content(), Arc::new(mmap));
        }

        Ok(Self { _file: file, mmaps: maps })
    }

    /// Retrieves a virtual ref for a journaled entry
    /// 
    /// Returns None if the source for the journaled entry is not available in the store
    #[inline]
    pub fn get_virtual_ref(&self, journaled: JournalEntry) -> Option<VirtualRef> {
        self.mmaps.get(journaled.source()).and_then(|source| {
            VirtualRef::new(journaled, source.clone())
                .inspect_err(|e| {
                    error!("Could not create virtual ref {e}");
                })
                .ok()
        })
    }
}

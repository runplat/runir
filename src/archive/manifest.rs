use std::io::Error;

use crate::IRecord;

use super::JournalEntry;

/// Manifest is a footer entry of all archives that includes metadata, provenance information, etc.
pub struct Manifest {
    pub(crate) record: crate::Record,
}

impl Manifest {
    /// Returns true if the manifest is balid
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.record.is_valid()
    }

    /// Loads the journal entries from the backing record
    /// 
    /// Returns an error if the data stored in the record is not valid
    #[inline]
    pub fn journal_entries(&self) -> std::io::Result<Vec<JournalEntry>> {
        if let Some(journal_entries) = self.record.load::<Vec<JournalEntry>>() {
            Ok(journal_entries)
        } else {
            Err(Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid manifest record",
            ))
        }
    }
}

impl std::fmt::Debug for Manifest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manifest")
            .field("record", &self.record.uuid())
            .finish()
    }
}

use std::io::Error;

use super::JournalEntry;


/// Manifest is a footer entry of all archives that includes metadata, provenance information, etc.
pub struct Manifest {
    pub(crate) record: crate::Record
}

impl Manifest {
    /// Returns true if the manifest is balid
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.record.is_valid()
    }

    pub fn journal_entries(&self) -> std::io::Result<Vec<JournalEntry>> {
        if let Some(journal_entries) = self.record.load::<Vec<JournalEntry>>() {
            Ok(journal_entries)
        } else {
            Err(Error::new(std::io::ErrorKind::InvalidData, "Invalid manifest record"))
        }
    }
}
use crate::{archive::JournalEntry, Namespace, Indexer, Record, RecordableExtensions};
use ahash::HashMap;
use std::time::Duration;

/// Contains an index of records
///
/// Finding records by ns/label should be O(1)
#[derive(Default, Debug)]
pub struct Index {
    /// Map of records in this index
    records: HashMap<u64, Record>,
    /// Maintains an index of the stored records that are indexable
    indexer: Indexer,
}

impl Index {
    /// Inserts a record into the index
    ///
    /// Note: Keys for inserted records are namespace aware
    #[inline]
    pub fn index(&mut self, record: &Record) {
        if record.is_valid() {
            let (hi, _) = record.uuid().as_u64_pair();
            let index_key = hi ^ record.ns_chk();
            let existing = self.records.insert(index_key, record.clone());

            // Handle known merge cases
            if let Some(replaced) = existing {
                // Merge manifests under a single manifest inside of index
                if replaced.opts().is_manifest() && record.opts().is_manifest() {
                    if let Some((mut a, b)) = replaced
                        .load::<Vec<JournalEntry>>()
                        .zip(record.load::<Vec<JournalEntry>>())
                    {
                        let merged = a.extend(b);
                        let mut merged_record =  merged.indexable();
                        merged_record.opts_mut().set_manifest_spec(true);
                        let merged =
                            Namespace::from("__ARCHIVE_INTERNALS").store("MANIFEST", merged_record);

                        // Remove the existing one so this doesn't end up in an infinite loop
                        self.records.remove(&index_key);
                        self.index(&merged);
                        return;
                    }
                }
            }

            if record.opts().is_indexable() {
                self.indexer.scan_update(record);
            }
        }
    }

    /// Finds a record w/ a matching label
    #[inline]
    pub fn find(&self, label: &str, ns: impl Into<Namespace>) -> Option<&Record> {
        let ns = ns.into();
        let key = ns.key(label) ^ ns.chk();
        self.records.get(&key)
    }

    /// Finds records older than age
    #[inline]
    pub fn find_older_than(&self, age: Duration) -> impl Iterator<Item = &Record> {
        self.records
            .iter()
            .filter(move |(_, v)| v.age() > age)
            .map(|(_, v)| v)
    }

    /// Searches records for fields that contain text
    #[inline]
    pub fn search_text(&self, field: &str, text: &str) -> impl Iterator<Item = &Record> {
        self.indexer
            .contains_text(field, text)
            .filter_map(|k| self.records.get(&k))
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use bytes::Bytes;

    use crate::{RecordableExtensions, Worker, Namespace};

    use super::Index;

    #[test]
    fn test_index_find() {
        let mut index = Index::default();

        let ns = Namespace::ephemeral();
        let record = ns.record("some / record").commit(Bytes::new());
        index.index(&record);

        let ns2 = Namespace::ephemeral();
        let record2 = ns2
            .record("some / record")
            .commit(Bytes::from_static(b"hello world"));
        index.index(&record2);

        let record = index.find("some / record", ns).expect("should exist");
        assert!(record.data().is_empty());

        let record = index.find("some / record", ns2).expect("should exist");
        assert_eq!(&b"hello world"[..], record.data().bytes());

        assert_eq!(2, index.find_older_than(Duration::from_nanos(1)).count());
        assert_eq!(0, index.find_older_than(Duration::from_secs(10)).count());
    }

    #[test]
    fn test_index_query() {
        use toml::toml;

        let mut worker = Worker::from("test_index_query");

        assert!(
            worker.store(
                "__record_1",
                toml! {
                    value = "hello world"
                }
                .indexable()
            )
        );

        assert!(worker.store(
            "__record_2",
            &toml! {
                value = "good dream world"
            }
        ));

        assert!(
            worker.store(
                "__record_3",
                toml! {
                    value = "do electric worlds dream of sheep, or say hello"
                }
                .indexable()
            )
        );

        let index = worker.to_index();

        assert_eq!(2, index.search_text("value", "dream hello").count())
    }
}

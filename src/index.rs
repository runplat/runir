use crate::{Indexer, Record, record::Namespace};
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
            self.records.insert(hi ^ record.ns_chk(), record.clone());

            if record.enabled(crate::RecordOpts::Indexing) {
                self.indexer.scan_update(record);
            }
        }
    }

    /// Finds a record w/ a matching label
    #[inline]
    pub fn find(&self, label: &str, ns: &Namespace) -> Option<&Record> {
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

    use crate::{record::Namespace, RecordableExtensions, Worker};

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

        let record = index.find("some / record", &ns).expect("should exist");
        assert!(record.data().is_empty());

        let record = index.find("some / record", &ns2).expect("should exist");
        assert_eq!(&b"hello world"[..], record.data().bytes());

        assert_eq!(2, index.find_older_than(Duration::from_nanos(1)).count());
        assert_eq!(0, index.find_older_than(Duration::from_secs(10)).count());
    }

    #[test]
    fn test_index_query() {
        use toml::toml;

        let mut worker = Worker::from("test_index_query");

        assert!(
            worker.save(
                "__record_1",
                toml! {
                    value = "hello world"
                }.indexable()
            )
        );

        assert!(worker.save(
            "__record_2",
            &toml! {
                value = "good dream world"
            }
        ));

        assert!(
            worker.save(
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

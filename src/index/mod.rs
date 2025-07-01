mod storage;
pub use storage::Storage;
use tracing::{debug, trace};

pub mod search;

/// Index backed by HashMap Storage
pub type HashIndex<R> = Index<R, HashMapStorage<R>>;

/// Index backed by Vec Storage
pub type VecIndex<R> = Index<R, VecStorage<R>>;

/// Type-alias for HashMap implementing Storage trait
pub type HashMapStorage<R> = ahash::HashMap<u64, R>;

/// Type-alias for Vec implementing Storage trait
pub type VecStorage<R> = Vec<R>;

/// Contains an index of records
///
/// Finding records by ns/label should be O(1)
#[derive(Debug)]
pub struct Index<R, S>
where
    R: crate::IRecord,
    S: Storage<Record = R>,
{
    /// Record storage implementation
    storage: S,

    /// Reverse lookup hash map
    ///
    /// Always maps IRecord::index_key() -> PutResult.key
    reverse: ahash::HashMap<u64, u64>,
}

impl<R: crate::IRecord, S: Storage<Record = R>> Index<R, S> {
    /// Inserts an IRecord based record into the index
    ///
    /// Returns the key that can be used to lookup the record
    #[inline]
    pub fn index(&mut self, record: R) -> u64 {
        let index_key = record.index_key();
        let result = self.storage.put(record);
        self.reverse.insert(index_key, result.key);
        result.key
    }

    /// Get a record from the index w/ a key returned from Index::index
    ///
    /// Returns None if a record w/ this key could not be found
    #[inline]
    pub fn get(&self, key: u64) -> Option<&R> {
        self.storage.record(key)
    }

    /// Lookup a record by a ns / label pair
    ///
    /// If a record was stored with a non-default namespace, that EXACT same
    /// namespace MUST be used in this function in order for the lookup to succeed
    ///
    /// Example:
    ///
    /// ```rs
    /// let mut index = HashMapIndex::default();
    /// let rec = Namespace::from("hello").store("world", ..);
    /// index.index(rec);
    /// assert!(index.lookup("hello", "world").is_some()) // This is Okay
    ///
    /// let ns = Namespace::from("hello")... // You set some flags on the namespace
    /// let rec = ns.store("world", ..);
    /// index.index(rec);
    /// assert!(!index.lookup("hello", "world").is_some())
    /// // This will not find anything,
    /// // because by changing the options on the namespace,
    /// // makes it a different namespace
    /// ```
    #[inline]
    pub fn lookup(&self, ns: impl Into<crate::Namespace>, label: &str) -> Option<&R> {
        let ns = ns.into();
        let index_key = ns.key(label) ^ ns.chk();
        self.reverse.get(&index_key).and_then(|k| self.get(*k))
    }

    /// Refreshes stored records from another index
    ///
    /// The other index is consdered newer, so the records in other will always
    /// take precedence over the currently stored record
    #[inline]
    pub fn refresh(&mut self, other: &Self)
    where
        R: Clone,
    {
        for r in other.storage.iter_records() {
            let ik = r.index_key();
            if let Some(k) = self.reverse.get(&ik) {
                debug!("existing key found, replacing");
                // If the reverse lookup see that we already have the record stored, we need to replace it at that key
                let replaced = self.storage.replace(*k, r.clone());
                trace!(replaced, ik, at = *k, "refresh");
            } else {
                debug!("existing key is not found, indexing");
                // TODO: Need index_with capabilities here
                self.index(r.clone());
            }
        }
    }

    /// Returns a reference to inner storage
    #[inline]
    pub fn storage(&self) -> &S {
        &self.storage
    }
}

impl<R: crate::IRecord, S: Storage<Record = R>> AsRef<S> for Index<R, S> {
    fn as_ref(&self) -> &S {
        &self.storage
    }
}

impl<R: crate::IRecord, S: Storage<Record = R>> Default for Index<R, S> {
    fn default() -> Self {
        Self {
            storage: Default::default(),
            reverse: ahash::HashMap::default(),
        }
    }
}

impl<R: crate::IRecord, S: Storage<Record = R>> From<Vec<R>> for Index<R, S> {
    fn from(mut value: Vec<R>) -> Self {
        let mut reverse = ahash::HashMap::default();
        Self {
            storage: value.drain(..).fold(S::default(), |mut map, r| {
                let index_key = r.index_key();
                let result = map.put(r);
                reverse.insert(index_key, result.key);
                map
            }),
            reverse,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{
        HashMapStorage, IRecord, Record, RecordableExtensions, Worker, field, filter, namespace,
        query::QueryBuilder,
    };

    #[tokio::test]
    async fn test_index_query() {
        use super::search::iter::Search;
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
                other = "hello dream world"
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

        let index = worker.to_index::<HashMapStorage<Record>>();

        // assert_eq!(2, index.search_text("value", "dream hello").count());

        let query = field("value");
        let results = index.search(query).collect::<Vec<_>>();
        assert_eq!(3, results.len());

        let results = index
            .search(field("value").contains("electric"))
            .collect::<Vec<_>>();
        assert_eq!(1, results.len());

        let results = index
            .search(namespace("test_index_query").and(field("value").contains("dream of sheep")))
            .count();
        assert_eq!(1, results);

        let results = index
            .search(
                namespace("test_index_query")
                    .and(field("value"))
                    .label("__record_2")
                    .and(field("other").contains("hello dream world")),
            )
            .count();
        assert_eq!(1, results);

        let results = index
            .search(
                namespace("test_index_query")
                    .and(field("value"))
                    .label("__record_2")
                    .and(field("other").contains("goodbye dream world")),
            )
            .count();
        assert_eq!(0, results);

        let results = index
            .search(filter::<Record>(|r| r.opts().is_indexable()))
            .count();
        assert_eq!(2, results);
    }
}

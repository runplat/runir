pub mod search;
mod storage;

use crate::{IRecord, opts::Branch, util::Container};
use anyhow::anyhow;
use std::u64;
pub use storage::Storage;
use tracing::{debug, error};

/// Index backed by HashMap Storage
pub type HashIndex<R> = Index<R, HashMapStorage<R>>;

/// Index backed by Vec Storage
pub type VecIndex<R> = Index<R, VecStorage<R>>;

/// Index backed by dashmap::DashMap based Storage
pub type ConcurrentIndex<R> = Index<R, ConcurrentStorage<R>>;

/// Type-alias for HashMap implementing Storage trait
pub type HashMapStorage<R> = ahash::HashMap<u64, R>;

/// Type-alias for Vec implementing Storage trait
pub type VecStorage<R> = Vec<R>;

/// Type-alias for dashmap::DashMap based Storage
pub type ConcurrentStorage<R> = dashmap::DashMap<u64, R>;

pub enum IndexResult<R> {
    /// Index inserted the record
    Inserted(u64),
    /// Index deleted a record
    Deleted(u64),
    /// Index promoted the inserting record automatically
    ///
    /// If the previous record was removed due to the promotion, then;
    ///
    /// a) If "soft" delete is enabled, the previous record is returned
    /// b) Otherwise; the previous record is dropped
    Promoted(u64, Option<R>),
    /// Index did not insert the record because it already exists
    Exists(u64, R),
    /// Failed to promote the record, because the record was not mutable
    CannotPromote(R),
    /// Cannot insert a deleted record
    CannotInsertDeletedRecord(R),
}

impl<R> IndexResult<R> {
    /// Returns the key of the indexed record
    #[inline]
    pub fn key(&self) -> Option<u64> {
        match self {
            IndexResult::Inserted(key)
            | IndexResult::Deleted(key)
            | IndexResult::Exists(key, _)
            | IndexResult::Promoted(key, _) => Some(*key),
            IndexResult::CannotPromote(_) | IndexResult::CannotInsertDeletedRecord(_) => None,
        }
    }

    /// Converts the IndexResult into a crate::Result to enable bubbling up
    #[inline]
    pub fn result(self) -> crate::Result<u64>
    where
        R: IRecord,
    {
        match self {
            IndexResult::Inserted(k)
            | IndexResult::Deleted(k)
            | IndexResult::Exists(k, _)
            | IndexResult::Promoted(k, _) => Ok(k),
            IndexResult::CannotPromote(r) => {
                Err(anyhow!("Failed to promote record {}", r.uuid()).into())
            }
            IndexResult::CannotInsertDeletedRecord(r) => Err(anyhow!(
                "A deleted record cannot be inserted into an index {}",
                r.uuid()
            )
            .into()),
        }
    }
}

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
    pub fn index(&mut self, record: R) -> IndexResult<R> {
        if record.opts().is_deleted() {
            if let Some(k) = self.reverse_lookup(&record) {
                self.delete(k);
                return IndexResult::Deleted(k);
            } else {
                // If the incoming record is in "deleted" mode, do not allow it to be inserted
                return IndexResult::CannotInsertDeletedRecord(record);
            }
        }

        if record.opts().is_staging() {
            // If the incoming record is staging, try to auto promote
            return self.try_auto_promote(record);
        }

        self.index_unchecked(record)
    }

    /// Reverse lookup a record and return it's key within the index
    #[inline]
    pub fn reverse_lookup(&self, record: &R) -> Option<u64> {
        self.reverse.get(&record.index_key()).copied()
    }

    /// Index the record without checking lifecycle state
    #[inline]
    pub fn index_unchecked(&mut self, record: R) -> IndexResult<R> {
        if let Some(key) = self.reverse_lookup(&record) {
            IndexResult::Exists(key, record)
        } else {
            let ik = record.index_key();
            let result = self.storage.put(record);
            self.reverse.insert(ik, result.key);
            IndexResult::Inserted(result.key)
        }
    }

    /// Get a record from the index w/ a key returned from Index::index
    ///
    /// Returns None if a record w/ this key could not be found or if the record was deleted
    #[inline]
    pub fn get(&self, key: u64) -> Option<S::Borrow<'_>> {
        self.storage.record(key).filter(|r| !r.opts().is_deleted())
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
    pub fn lookup(&self, ns: impl Into<crate::Namespace>, label: &str) -> Option<S::Borrow<'_>> {
        let ns = ns.into();
        let index_key = ns.key(label) ^ ns.chk();
        self.reverse.get(&index_key).and_then(|k| self.get(*k))
    }

    /// Marks a record for deletion
    ///
    /// Returns true if the index was able to mark the record for deletion
    #[inline]
    pub fn delete(&mut self, key: u64) -> bool {
        if let Some(mut rec) = self.storage.record_mut(key) {
            if let Some(opts) = rec.opts_mut() {
                opts.enable_branch(Branch::Deleted);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Returns a reference to inner storage
    #[inline]
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Tries to auto-promote a record in "staging" mode
    fn try_auto_promote(&mut self, mut record: R) -> IndexResult<R> {
        if let Some(key) = self.reverse.get(&record.index_key()) {
            // Record already exists, check if auto-promote conditions are available
            if let Some(current) = self.storage.record(*key) {
                if current.opts().is_deleted() {
                    debug!("Existing record is marked for deletetion");
                    if let Some(opts_mut) = record.opts_mut() {
                        opts_mut.promote();
                    } else {
                        debug!(
                            "Implementation tried to perform a mutating action on an IRecord that does not support it"
                        );
                        return IndexResult::CannotPromote(record);
                    }
                } else if !current.opts().is_multi() && record.opts().is_multi() {
                    match Self::try_auto_promote_to_container(&current, &mut record) {
                        Err(err) => {
                            error!("{err}");
                            return IndexResult::CannotPromote(record);
                        },
                        _ => {}
                    }
                } else {
                    return IndexResult::CannotPromote(record);
                }
            } else {
                unreachable!("Reverse-index mapping must never be modified")
            }

            if let Some(replaced) = self.storage.replace(*key, record) {
                if replaced.opts().is_soft_deleted() {
                    debug!(
                        "Previous record is in soft-deletion mode, including it in index result"
                    );
                    return IndexResult::Promoted(*key, Some(replaced));
                } else {
                    debug!(
                        "Previous record was in staging mode, and was also deleted, dropping completely"
                    );
                    return IndexResult::Promoted(*key, None);
                }
            } else {
                unreachable!("We check if there is a record to replace before we tried to replace")
            }
        } else {
            // Record doesn't exist, we are safe to promote this record
            if let Some(opts_mut) = record.opts_mut() {
                opts_mut.promote();
            } else {
                // This is a safeguard, so by default do not try to panic since at this level of the library not being able to promote
                // an immutable IRecord should be obvious.
                // However, IndexResult can be converted to a crate::Result<..> if strictness is desired
                debug!(
                    "Implementation tried to perform a mutating action on an IRecord that does not support it"
                );
                return IndexResult::CannotPromote(record);
            }

            self.index(record)
        }
    }

    /// Tries to auto promote a container, if the existing record is not a multi-root, and the next
    /// record is a multi-root in staging mode, and the root content digest matches the xisting
    /// record, auto-promotes the container
    #[inline]
    fn try_auto_promote_to_container(current: &R, next: &mut R) -> crate::Result<()> {
        debug!(
            "Detected upgrade from record to container, checking if auto-promotion condition is met"
        );

        let record = next.to_record();
        match Container::read(&record) {
            Ok(container) => {
                if container.content() == current.content() {
                    debug!(
                        "Current container content digest matches container root's content digest, auto-promote conditions have been met"
                    );

                    if let Some(opts) = next.opts_mut() {
                        opts.promote();

                        return Ok(());
                    }
                }

                Err(anyhow!("Could not auto-promote container, root digest does not match current root digest").into())
            }
            Err(err) => Err(anyhow!(
                "Could not read as container, auto-promotion could not proceed {err}"
            )
            .into()),
        }
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

impl<R: crate::IRecord, S: Storage<Record = R> + Clone> Clone for Index<R, S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            reverse: self.reverse.clone(),
        }
    }
}

#[cfg(test)]
mod test {
    use bytes::Bytes;
    use rayon::iter::ParallelIterator;

    use crate::{
        field, filter, namespace, query::QueryBuilder, util::{Container, PeekExtensions}, HashMapStorage, IRecord, Namespace, Record, RecordableExtensions, ToNamespace, Worker
    };

    use super::{ConcurrentStorage, Index, VecIndex};

    #[tokio::test]
    async fn test_index_query() {
        use super::search::iter::Search;
        use toml::toml;

        let mut worker = Worker::default();
        let ns = "test_index_query".to_namespace();
        assert!(
            worker.push(
                ns.store(
                    "__record_1",
                    toml! {
                        value = "hello world"
                    }
                    .indexable()
                )
            )
        );

        assert!(worker.push(ns.store(
            "__record_2",
            &toml! {
                value = "good dream world"
                other = "hello dream world"
            }
        )));

        assert!(
            worker.push(
                ns.store(
                    "__record_3",
                    toml! {
                        value = "do electric worlds dream of sheep, or say hello"
                    }
                    .indexable()
                )
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
            .search(filter::<&Record>(|r| r.opts().is_indexable()))
            .count();
        assert_eq!(2, results);
    }

    #[test]
    #[tracing_test::traced_test]
    fn test_index_auto_promote() {
        let ns = Namespace::ephemeral();

        let rec = ns.commit("test", b"hello world");

        let mut index = VecIndex::default();
        let key = index.index(rec.clone()).result().unwrap();
        index
            .index(rec.clone())
            .result()
            .expect("should just skip if record is already indexed");

        let staged = rec.stage(Bytes::from_static(b"hello world 2")).unwrap();
        assert!(
            index.index(staged.clone()).result().is_err(),
            "should not be able to auto-promote a record if the previous was not deleted"
        );
        assert!(index.delete(key), "should be able to delete full records");

        let key = index.index(staged).result().unwrap();
        assert_eq!(b"hello world 2", index.get(key).unwrap().bytes());

        // Since freshly-built containers are returned in staging mode, test that the index accepts it
        // When no existing record exists
        let rec = ns.commit("test2", b"hello world");
        let mut container = Container::build(rec);
        container
            .push_object(&toml::toml! {
                name = "test container"
            })
            .unwrap();
        let built = container.to_read_only().unwrap();

        let key = index.index(built.to_record()).result().unwrap();

        let rec = index.get(key).map(Container::read).unwrap().unwrap();
        assert_eq!("test container", rec.object(2).at("name").str().unwrap());

        // Test auto-promoting a container
        let rec = ns.commit("test3", b"hello world");
        let mut container = Container::build(rec.clone());
        container
            .push_object(&toml::toml! {
                name = "test container"
            })
            .unwrap();
        let built = container.to_read_only().unwrap();

        let key = index.index(rec).result().unwrap();
        let key2 = index.index(built.to_record()).result().unwrap();
        assert_eq!(key, key2);
        let rec = index.get(key2).map(Container::read).unwrap().unwrap();
        assert_eq!("test container", rec.object(2).at("name").str().unwrap())
    }

    #[test]
    fn test_index_search_par() {
        use super::search::par::Search;
        let mut index = Index::<Record, ConcurrentStorage<Record>>::default();

        index.index(Namespace::ephemeral().store("test", &toml::toml! {
            test = "hello"
        }));

        index.index(Namespace::ephemeral().store("test1", &toml::toml! {
            test = "hello world"
        }));


        index.index(Namespace::ephemeral().store("test2", &toml::toml! {
            not_test = "hello world 2"
        }));

        let results = index.search(field("test"));

        assert_eq!(2, results.count());
    }
}

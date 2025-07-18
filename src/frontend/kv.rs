//! # KV Frontend
//!
//! The `kv` frontend provides a familiar key-value store interface on top of `runir` primitives (Namespace, Record)
//!
//! ## First-time Use
//! To open or create a store, simply call `runir::kv::open()` or `runir::kv::new()`
//!
//! This store is *ephemeral* until `save()` or `save_as()` is called.
//!
//! This allows fast, in-memory operation by default, without requiring setup.
//!
//! ## Canonical KV Store API Example
//! ```rs
//! let mut kv = runir::kv::new();
//!
//! kv.put("value", b"hello world")?;
//!
//! // The value is always available immediately
//! assert_eq!(b"hello world", kv.get("value").unwrap());
//!
//! // Once save(..) is called, the same data will be persisted to disk and available on process restart
//! kv.save().await?;
//! ```
//!
//! Calling `save()` will flush all stored values from any kv-store handles associated to the originally opened handle.
//!
//! Opening multiple KV frontends to the same save path (i.e. calling kv::open(..) twice from the same process) concurrently may result in overwrite conflicts. See below section for "Concurrency and Multi-writer Safety" for details.
//!
//! ## KV Store Namespaces
//! ```rs
//! let kv = runir::kv::open().await?;
//!
//! // Instead of baking namespaces into the key itself, you can namespace the entire store
//! let ns_kv = kv.ns("my_namespace");
//! ```
//!
//! ## KV Store "serde" API
//! ```rs
//! let kv = runir::kv::open().await?;
//!
//! let kv_serde = kv.serde();
//!
//! kv_serde.put("value", &toml::toml! {
//!     my_interesting_value = "hello"
//! });
//!
//! let obj = kv_serde.load::<toml::Value>("value").unwrap();
//!
//! let reader = obj.peek().unwrap();
//! assert_eq!("hello", reader.as_map().idx("my_interesting_value").as_str())
//! ```
//!
//! ## KV Multi-Threaded Scenarios
//!
//! Multi-threaded scenarios are supported by calling `refresh()` when necessary.
//!
//! This allows threads to update their indices in order to view records stored by different threads.
//!
//! ```rs
//! let kv = runir::kv::open().await?;
//!
//! // In two different threads:
//!
//! thread::spawn(move || {
//!     let ns = kv.ns("my_ns");
//!     ns.put("val1", b"hello").unwrap();
//! });
//!
//! thread::spawn(move || {
//!     let ns = kv.ns("my_ns");
//!     ns.refresh();
//!     assert_eq!(b"hello", ns.get("val1").unwrap());
//! });
//! ```
//!
//! ## Concurrency and Multi-writer Safety
//!
//! - Cloning a `kv` store (e.g., via `.ns(..)` or manually sharing a `SharedState`) is **thread-safe**.
//!   - All clones share a common index and record buffer.
//!   - Use `refresh()` periodically to sync changes across threads.
//!
//! - **Opening multiple `kv` frontends independently** (e.g., via repeated `kv::open()` or `kv::new()`) is **not safe for concurrent writing to the same archive path**.
//!   - These instances maintain separate snapshots and may overwrite each other's data when calling `save()`.
//!   - In this case, `runir` does not coordinate or lock access to the backing file.
//!
//! **Guideline:** Share `SharedState` if you need multiple writers. Avoid multiple independent frontends writing to the same save path unless you're managing access externally.
//!
//! # Search Support
//!
//! ```rs
//! use runir::query::*;
//!
//! let kv = runir::kv::open().await?;
//!
//! for result in kv.search(field("value").contains("hello")) {
//!     // work with `&Record`
//! }
//! ```
//!
//! //! ## Zero-Copy Field Access with `peek()`
//!
//! Access individual fields in stored structured data without full deserialization:
//!
//! ```rs
//! let kv = runir::kv::open().await?;
//! let serde_kv = kv.serde();
//!
//! serde_kv.put("config", &toml::toml! {
//!     auth = { token = "abc123" }
//! })?;
//!
//! // Later...
//!
//! let token = kv.serde()
//!     .peek("config")
//!     .in_ref()["auth"]["token"]
//!     .str()
//!     .unwrap();
//!
//! assert_eq!("abc123", token);
//! ```
//!
//! This is ideal for quick field access from nested structures (like TOML/JSON),
//! and avoids panics, allocations, or full deserialization unless requested.

/// Opens a new or existing key-value store
///
/// Note: This store is *ephemeral* until `save()` or `save_as()` is called.
///
/// Sugar for `crate::frontend::open::<KeyValue>()`
pub async fn open() -> std::io::Result<KeyValue> {
    super::open::<KeyValue>().await
}

/// Opens a key-value store from dir
///
/// Note: This store is *ephemeral* until `save()` or `save_as()` is called.
///
/// Sugar for `crate::frontend::open_dir::<KeyValue>(..)`
pub async fn open_dir(dir: impl Into<PathBuf>) -> std::io::Result<KeyValue> {
    super::open_dir::<KeyValue>(dir).await
}

use super::{Frontend, state::SharedState};
use crate::{
    IRecord, Namespace, Query, Record, SharedWorker, ToNamespace, VecIndex, search::iter::Search,
    util::PeekExtensions,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Error,
    ops::{Deref, DerefMut},
    path::PathBuf,
};
use tracing::{debug, error};

/// Provides a put(..) function that takes a serializable object as the value
pub trait Put<V> {
    /// Puts a value in the store to key
    ///
    /// Returns an Error if stored data could not be validated
    ///
    /// Note: All stored-values are expected to be idempotent by default, unless a merge-policy is set
    /// on the Namespace. This means that by default if no merge-policy is set, this function will always
    /// replace any existing values, and not return an error
    fn put(&mut self, key: &str, value: &V) -> std::io::Result<()>;

    /// Put many key_values in the store at once
    ///
    /// Returns an Error if stored data could not be validated
    ///
    /// Note: All stored-values are expected to be idempotent by default, unless a merge-policy is set
    /// on the Namespace. This means that by default if no merge-policy is set, this function will always
    /// replace any existing values, and not return an error
    fn put_many(&mut self, key_values: &[(&str, &V)]) -> std::io::Result<()>;
}

/// Provides a get(..) function that returns a value by key
pub trait Get<'get, V: 'get> {
    /// Gets a value from the store by key
    ///
    /// Returns an error if the value could not be found or if unsuccessful
    fn get<'from: 'get>(&'from self, key: &str) -> std::io::Result<V>;
}

/// Provides a Key Value Store frontend
///
/// Get started using `kv.put(..)` with raw bytes
///
/// Upgrade to `kv.serde().put(..)` for any Serialize/Deserialize type
#[derive(Clone)]
pub struct KeyValue {
    /// Namespace this key-value is under, defaults to ""
    ns: Namespace,
    /// Shared worker which handles sending work to the store
    worker: SharedWorker,
    /// State that is shared across all workers
    shared: SharedState,
}

impl Frontend for KeyValue {
    const NAME: &str = "kv";

    fn from_shared(shared: SharedState) -> Self {
        let worker = shared.store().worker();
        Self {
            ns: ().to_namespace(),
            worker: SharedWorker::from(worker),
            shared,
        }
    }
}

impl KeyValue {
    /// Saves the current state of the store to a well-known directory
    ///
    /// Note: This store is *ephemeral* until `save()` or `save_as()` is called.
    ///
    /// Calling `put()` or `get()` will work without error, but state will not persist
    /// between program runs unless explicitly saved.
    ///
    /// This allows fast, in-memory operation by default, without requiring setup.
    ///
    /// # Save Procedure
    ///
    /// If this is the first time the store has saved, this function will ensure the store's working directory
    /// is available and configured.
    ///
    /// WARN: If the current process does not have permissions for any of the above procedures, an error will be returned.
    ///
    /// Sugar for `crate::frontend::save::<KeyValue>(..)`
    pub async fn save(&self) -> std::io::Result<()> {
        super::save(self).await
    }

    /// Saves the current state of the store to a user-specified directory
    ///
    /// Note: This store is *ephemeral* until `save()` or `save_as()` is called.
    ///
    /// Calling `put()` or `get()` will work without error, but state will not persist
    /// between program runs unless explicitly saved.
    ///
    /// This allows fast, in-memory operation by default, without requiring setup.
    ///
    /// (See KeyValue::save for details on operational behavior)
    ///
    /// Sugar for `crate::frontend::save_as::<KeyValue>(..)`
    #[inline]
    pub async fn save_as(&self, to: impl Into<PathBuf>) -> std::io::Result<()> {
        super::save_as(self, to).await
    }

    /// Returns a new key-value store scoped to a namespace
    #[inline]
    pub fn ns(&self, ns: impl Into<Namespace>) -> KeyValue {
        let ns = ns.into();

        Self {
            ns,
            worker: self.worker.clone(),
            shared: self.shared.clone(),
        }
    }

    /// Returns a key-value store interface for putting values that implement serde::Serialize
    #[inline]
    pub fn serde(&self) -> KeySerdeValue {
        KeySerdeValue {
            kv: KeyValue {
                ns: self.ns.clone(),
                worker: self.worker.clone(),
                shared: self.shared.clone(),
            },
        }
    }

    /// Refreshes the key value store's indexes
    #[inline]
    pub async fn refresh(&self) -> std::io::Result<()> {
        // Sync any work pending from workers
        self.worker.sync()?.await?;

        // Flushes all work to disk-backed stores and updates indicies
        self.shared.state.reduce()?;
        Ok(())
    }

    /// Searches over stored records using a query.
    ///
    /// This function will invoke `force_sync()` to ensure the search operates on the latest data.
    ///
    /// If performance is critical (e.g., within a tight loop), consider using [`KeyValue::take_snapshot()`],
    /// which returns a stable, read-only view of the current data and exposes the same `search(..)` API.
    #[inline]
    pub fn search<'q>(
        &'q self,
        query: impl Into<Query<'q, &'q Record>>,
    ) -> impl Iterator<Item = &'q Record> {
        let q = query.into();

        self.force_sync();

        self.shared.snapshot().search(q)
    }

    /// Puts a record into the kv store
    #[inline]
    pub fn put_raw(&self, record: Record) -> std::io::Result<()> {
        if self.worker.push(record.clone()) {
            Ok(())
        } else {
            Err(Error::new(
                std::io::ErrorKind::InvalidInput,
                "Record could not be stored",
            ))
        }
    }

    /// Get a record from the kv store
    #[inline]
    pub fn get_raw(&self, key: &str) -> Option<&Record> {
        if self.shared.snapshot().storage().is_empty() {
            debug!("Snapshot is empty trying to load");
            self.force_sync();
        }
        self.shared
            .snapshot()
            .lookup(self.ns.clone(), key)
            .or_else(|| {
                self.force_sync();
                self.shared.snapshot().lookup(self.ns.clone(), key)
            })
    }

    /// Returns a new snapshot of the kv-store
    #[inline]
    pub fn take_snapshot(&self) -> VecIndex<Record> {
        self.shared.snapshot().clone()
    }

    /// Forces a sync of the underlying store.
    ///
    /// This is a blocking operation and is only invoked automatically under specific conditions:
    /// - If the current snapshot is empty during a `lookup`
    /// - If a `lookup(..)` fails (returns `None`) — in this case, `force_sync()` is retried automatically
    /// - If [`KeyValue::search()`] is used
    ///
    /// It ensures the freshest possible view of the data without introducing additional latency
    /// during normal reads.
    ///
    /// ⚠️ **Note:** If the `tokio` feature is enabled, this must be called within a multithreaded tokio runtime,
    /// as it internally invokes `spawn_blocking(..)` and awaits the result.
    ///
    /// This function is not intended for frequent use in performance-critical paths. Prefer `refresh()`
    /// in async contexts or use `take_snapshot()` when manual control is desired.
    #[inline]
    fn force_sync(&self) {
        debug!("Force sync invoked, blocking for a refresh");
        if let Err(err) =
            // NOTE: If tokio is enabled, this MUST run on a multi-threaded tokio-runtime
            futures::executor::block_on(crate::util::spawn_blocking(|| self.refresh()))
        {
            error!("Could not refresh {err}");
        } else {
            self.shared.update_snapshot();
        }
    }
}

/// Wraps a key-value store and provides a put/get interface that is aware of serde-values
#[derive(Clone)]
pub struct KeySerdeValue {
    /// Inner KV Store
    kv: KeyValue,
}

impl KeySerdeValue {
    /// Peek at the stored object w/ PeekExtentions
    ///
    /// Allows getting data from the object without deserializing it into a full type
    #[inline]
    pub fn peek(&self, key: &str) -> impl PeekExtensions {
        self.get_raw(key).and_then(|r| r.peek().val())
    }

    /// Loads an object from the store
    #[inline]
    pub fn load<'de, T: Deserialize<'de> + 'de>(&'de self, key: &str) -> Option<T> {
        Get::<T>::get(self, key).ok()
    }
}

impl Deref for KeySerdeValue {
    type Target = KeyValue;

    fn deref(&self) -> &Self::Target {
        &self.kv
    }
}

impl DerefMut for KeySerdeValue {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.kv
    }
}

impl<V: Serialize> Put<V> for KeySerdeValue {
    fn put(&mut self, key: &str, value: &V) -> std::io::Result<()> {
        self.put_raw(self.ns.store(key, value))
    }

    fn put_many(&mut self, key_values: &[(&str, &V)]) -> std::io::Result<()> {
        for (key, v) in key_values {
            self.put_raw(self.ns.store(key, v))?;
        }

        Ok(())
    }
}

impl<V: AsRef<[u8]>> Put<V> for KeyValue {
    fn put(&mut self, key: &str, value: &V) -> std::io::Result<()> {
        self.put_raw(self.ns.commit(key, value.as_ref()))
    }

    fn put_many(&mut self, key_values: &[(&str, &V)]) -> std::io::Result<()> {
        for (key, v) in key_values {
            self.put_raw(self.ns.commit(key, v.as_ref()))?;
        }

        Ok(())
    }
}

impl<'get, V: Deserialize<'get> + 'get> Get<'get, V> for KeySerdeValue {
    fn get<'from: 'get>(&'from self, key: &str) -> std::io::Result<V> {
        match self.get_raw(key).and_then(|r| r.load::<V>()) {
            Some(r) => Ok(r),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Could not find {key}"),
            )),
        }
    }
}

impl<'get> Get<'get, &'get [u8]> for KeyValue {
    fn get<'from: 'get>(&'from self, key: &str) -> std::io::Result<&'get [u8]> {
        match self.get_raw(key).map(|r| r.bytes()) {
            Some(r) => Ok(r),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Could not find {key}"),
            )),
        }
    }
}

impl AsRef<SharedState> for KeyValue {
    fn as_ref(&self) -> &SharedState {
        &self.shared
    }
}

#[cfg(test)]
mod test {
    use super::{Get, KeyValue, Put};
    use crate::{
        field, filter, frontend::Frontend, namespace, util::{PeekExtensions, PeekRefExtensions}, IRecord, QueryBuilder, Record
    };
    use std::path::PathBuf;

    #[test]
    #[tracing_test::traced_test]
    fn test_kv_put_get() {
        let mut kv = KeyValue::new();

        kv.put("hello", b"hello").unwrap();

        assert_eq!(b"hello", kv.get("hello").unwrap());
        assert_eq!(b"hello", kv.get("hello").unwrap());
    }

    #[test]
    #[tracing_test::traced_test]
    fn test_kv_put_get_serde() {
        let mut kv = KeyValue::new().serde();

        kv.put(
            "hello",
            &toml::toml! {
                value = "really important value"

                [other.values]
                also_important = "hello"
            },
        )
        .unwrap();

        assert_eq!(
            "hello",
            kv.load::<toml::Value>("hello").unwrap()["other"]["values"]["also_important"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "really important value",
            kv.peek("hello").at("value").str().unwrap()
        );

        assert_eq!(1, kv.search(field("other")).count());

        assert_eq!(
            1,
            kv.search(filter::<&Record>(|r| { r.matches_label("hello", "") }))
                .count()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[tracing_test::traced_test]
    async fn test_kv_bg_sync() {
        let mut kv = KeyValue::new().serde();

        kv.put(
            "hello",
            &toml::toml! {
                value = "really important value"

                [other.values]
                also_important = "hello"
                int = 0
            },
        )
        .unwrap();

        let mut parallel_ns = kv.ns(()).serde();
        parallel_ns
            .put(
                "hello2",
                &toml::toml! {
                    value = "another really important value"

                    [other.values]
                    also_important = "another hello"
                },
            )
            .unwrap();

        assert_eq!(
            "hello",
            kv.load::<toml::Value>("hello").unwrap()["other"]["values"]["also_important"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "really important value",
            kv.peek("hello").at("value").str().unwrap_or_default()
        );

        assert_eq!(2, kv.search(field("other")).count());

        assert_eq!(
            1,
            kv.search(filter::<&Record>(|r| { r.matches_label("hello", "") }))
                .count()
        );

        assert!(
            kv.shared
                .snapshot()
                .lookup("", "hello")
                .unwrap()
                .is_virtual(),
            "Worker should now be using virtual data, and not the original buffer"
        );

        assert_eq!(
            "hello",
            kv.load::<toml::Value>("hello").unwrap()["other"]["values"]["also_important"]
                .as_str()
                .unwrap()
        );

        // Test that after refresh is called, we have access to records created in different stores
        assert_eq!(
            "another hello",
            kv.load::<toml::Value>("hello2").unwrap()["other"]["values"]["also_important"]
                .as_str()
                .unwrap()
        );

        let _ = std::fs::create_dir(".test");
        kv.save().await.unwrap();
        kv.save_as(".test").await.unwrap();

        let kv = super::open().await.unwrap();
        let count = kv
            .search(
                namespace(())
                    .label("hello2")
                    .and(field("value").contains("another")),
            )
            .count();
        assert_eq!(1, count);

        kv.shared.state.import(".test/kv.tar").await.unwrap();
        let imported = kv
            .shared
            .state
            .snapshot(crate::frontend::state::Snapshot::Imported(PathBuf::from(
                ".test/kv.tar",
            )));

        let hello2 = imported.lookup("", "hello2").unwrap();
        assert_eq!(
            "another hello",
            hello2.load::<toml::Value>().unwrap()["other"]["values"]["also_important"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "another hello",
            hello2
                .peek()
                .at_path(["other", "values", "also_important"])
                .str()
                .unwrap()
        );

        assert_eq!(
            Some("another hello"),
            hello2.peek().in_ref()["other"]["values"]["also_important"].str()
        );

        assert_eq!(
            None,
            hello2.peek().in_ref()["other"]["values"]["doesn't exist"].str()
        );

        let other_values = hello2.peek().in_ref()["other"]["values"].clone();
        assert_eq!(
            Some("another hello"),
            other_values.clone()["also_important"].str()
        );
        assert_eq!(
            Some("another hello"),
            other_values.clone()["also_important"].str()
        );
        assert_eq!(None, other_values.clone()["doesn't exist"].str());

        assert_eq!(
            Some("another hello"),
            kv.serde().peek("hello2").in_ref()["other"]["values"]["also_important"].str()
        );

        assert_eq!(
            Some(0),
            kv.serde()
                .peek("hello")
                .at("other")
                .at("values")
                .at("int")
                .int()
        );

        let path = crate::util::PeekPath::default();
        let other_values = &path["other"]["values"];

        assert_eq!(
            Some("another hello"),
            other_values["also_important"]
                .lookup(kv.serde().peek("hello2"))
                .str()
        );
        assert_eq!(
            None,
            other_values["doesn't exist"]
                .lookup(kv.serde().peek("hello2"))
                .str()
        );
        assert_eq!(
            Some("another hello"),
            other_values["also_important"]
                .lookup(kv.serde().peek("hello2"))
                .str()
        );

        assert_eq!(
            Some("another hello"),
            kv.serde().peek("hello2").at_dot("other.values.also_important").str()
        );

        assert_eq!(
            Some("another hello"),
            kv.get_raw("hello2").field("other.values.also_important").str()
        );
        
        assert_eq!(
            None,
            kv.get_raw("hello2").field("other.values.doesn't exist").str()
        );

        assert_eq!(
            None,
            kv.get_raw("doesn't exist").field("other.values.also_important").str()
        );
    }
}

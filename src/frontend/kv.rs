use crate::{
    IRecord, Namespace, Query, Queue, Record, Store, Worker, search::iter::Search,
    store::StoreArchive, worker::BackgroundSync,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Error,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};

/// Provides a put(..) function that takes a serializable object as the value
pub trait Put<V> {
    /// Puts a value in the store to key
    ///
    /// Returns a BackgroundSync, which is a future that returns after the sync data has been queued
    /// for packing. Reading a value from the thread it was written on is always immediately available.
    ///
    /// Note: If waiting for the result of the background sync isn't desired `.forget(..)` can be called.
    ///
    /// Returns an Error if stored data could not be validated
    ///
    /// Note: All stored-values are expected to be idempotent by default, unless a merge-policy is set
    /// on the Namespace. This means that by default if no merge-policy is set, this function will always
    /// replace any existing values, and not return an error
    fn put(&mut self, key: &str, value: &V) -> std::io::Result<BackgroundSync>;

    /// Put many key_values in the store at once
    ///
    /// This is more effecient then calling put(..) multiple times if all key_values are known ahead of time.
    ///
    /// Returns a BackgroundSync, which is a future that returns after the sync data has been queued
    /// for packing. Reading a value from the thread it was written on is always immediately available.
    ///
    /// Note: If waiting for the result of the background sync isn't desired `.forget(..)` can be called.
    ///
    /// Returns an Error if stored data could not be validated
    ///
    /// Note: All stored-values are expected to be idempotent by default, unless a merge-policy is set
    /// on the Namespace. This means that by default if no merge-policy is set, this function will always
    /// replace any existing values, and not return an error
    fn put_many(&mut self, key_values: &[(&str, &V)]) -> std::io::Result<BackgroundSync>;
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
pub struct KeyValue {
    /// Namespace all keys will be indexed under
    worker: Worker,
    state: SharedState,
}

impl KeyValue {
    /// Opens a new key-value store
    ///
    /// Note: This store is *ephemeral* until `save()` or `save_as()` is called.
    ///
    /// Calling `put()` or `get()` will work without error, but state will not persist
    /// between program runs unless explicitly saved.
    ///
    /// This allows fast, in-memory operation by default, without requiring setup.
    ///
    /// # KV Store Opening Procedure
    ///
    /// If the RUNIR_KV_HOME env variable is not set, the key-value store will default to a temp-directory
    /// for all file-system operations
    ///
    /// When KeyValue::save() is called, the kv_store will attempt to use "std::env::current_dir()/.runir/<CARGO_PKG>",
    /// as the root directory of the store. If successful, the value of RUNIR_KV_HOME will be set via a .env_runir file written to
    /// "std::env::current_dir()".
    ///
    /// At the start of the application, dotenvy will be used to re-hydrate env variables set in .env_runir.
    ///
    /// From that point on, the data stored in RUNIR_KV_HOME will continue to be used cleaned up, or deleted.
    ///
    /// If instead KeyValue::save_as(..) is used, this will follow the above env setup as above but with the user provided directory.
    ///
    /// WARN: If the current process does not have permissions for any of the above procedures, an error will be returned.
    pub async fn open() -> std::io::Result<KeyValue> {
        todo!()
    }

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
    /// If successful, this function will output a .env_runir file if it has not been set, (See KeyValue::open(..) for more details)
    ///
    /// WARN: If the current process does not have permissions for any of the above procedures, an error will be returned.
    pub async fn save(&self) -> std::io::Result<()> {
        todo!()
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
    pub async fn save_as(&self, _to: impl Into<PathBuf>) -> std::io::Result<()> {
        todo!()
    }

    /// Returns a new key-value store scoped to a namespace
    pub fn ns(&self, ns: impl Into<Namespace>) -> KeyValue {
        let ns = ns.into();

        Self {
            worker: self.state.store().namespace(ns),
            state: self.state.clone(),
        }
    }

    /// Returns a key-value store interface for putting values that implement serde::Serialize
    #[inline]
    pub fn serde(&self) -> KeySerdeValue {
        KeySerdeValue {
            kv: KeyValue {
                worker: self
                    .state
                    .store()
                    .namespace(self.worker.namespace().clone()),
                state: self.state.clone(),
            },
        }
    }

    /// Searches over stored records w/ a query
    #[inline]
    pub fn search<'q>(
        &'q self,
        query: impl Into<Query<'q, Record>>,
    ) -> impl Iterator<Item = &'q Record> {
        self.worker.cache().search(query)

        // TODO: For now this searches the local worker cache
        // Will add the search functionality over the entire StoreArchive once the sync/flush code is finished
    }
}

/// Wraps a key-value store and provides a put/get interface that is aware of serde-values
pub struct KeySerdeValue {
    /// Inner KV Store
    kv: KeyValue,
}

impl KeySerdeValue {
    /// Peek at the stored object w/ a flexbuffer Reader
    ///
    /// Allows getting data from the object without deserializing it into a full type
    #[inline]
    pub fn peek(&self, key: &str) -> Option<flexbuffers::Reader<&[u8]>> {
        self.kv
            .get(key)
            .ok()
            .and_then(|r| flexbuffers::Reader::get_root(r).ok())
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
    fn put(&mut self, key: &str, value: &V) -> std::io::Result<BackgroundSync> {
        if self.worker.store(key, value) {
            Ok(self
                .worker
                .sync()
                .expect("should only return None if worker was created outside the store"))
        } else {
            Err(Error::new(
                std::io::ErrorKind::InvalidInput,
                "Value could not be stored",
            ))
        }
    }

    fn put_many(&mut self, key_values: &[(&str, &V)]) -> std::io::Result<BackgroundSync> {
        for (key, v) in key_values {
            if !self.worker.store(key, v) {
                return Err(Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Value could not be stored",
                ));
            }
        }

        Ok(self
            .worker
            .sync()
            .expect("should only return None if worker was created outside the store"))
    }
}

impl<V: AsRef<[u8]>> Put<V> for KeyValue {
    fn put(&mut self, key: &str, value: &V) -> std::io::Result<BackgroundSync> {
        if self.worker.commit(key, value.as_ref()) {
            Ok(self
                .worker
                .sync()
                .expect("should only return None if worker was created outside the store"))
        } else {
            Err(Error::new(
                std::io::ErrorKind::InvalidInput,
                "Value could not be stored",
            ))
        }
    }

    fn put_many(&mut self, key_values: &[(&str, &V)]) -> std::io::Result<BackgroundSync> {
        for (key, v) in key_values {
            if !self.worker.commit(key, v.as_ref()) {
                return Err(Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Value could not be stored",
                ));
            }
        }

        Ok(self
            .worker
            .sync()
            .expect("should only return None if worker was created outside the store"))
    }
}

impl<'get, V: Deserialize<'get> + 'get> Get<'get, V> for KeySerdeValue {
    fn get<'from: 'get>(&'from self, key: &str) -> std::io::Result<V> {
        match self
            .worker
            .cache()
            .lookup(self.worker.namespace().clone(), key)
            .and_then(|r| r.load())
        {
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
        match self
            .worker
            .cache()
            .lookup(self.worker.namespace().clone(), key)
            .map(|r| r.bytes())
        {
            Some(r) => Ok(r),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Could not find {key}"),
            )),
        }
    }
}

struct Shared {
    /// Immediate Store, for immediate put/get requests
    store: Store,
    /// Last snapshot of records
    pending: Queue<StoreArchive>,
}

struct SharedState(Arc<Shared>);

impl SharedState {
    /// Returns a the current store
    fn store(&self) -> &Store {
        &self.0.store
    }
}

impl Clone for SharedState {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::{Record, Store, field, filter};

    use super::{Get, KeyValue, Put, SharedState};

    #[test]
    fn test_kv_put_get() {
        let state = SharedState(Arc::new(super::Shared {
            store: Store::default(),
            pending: Default::default(),
        }));

        let worker = state.store().namespace("default");
        let mut kv = KeyValue { worker, state };

        kv.put("hello", b"hello").unwrap().forget();

        assert_eq!(b"hello", kv.get("hello").unwrap());
    }

    #[test]
    fn test_kv_put_get_serde() {
        let state = SharedState(Arc::new(super::Shared {
            store: Store::default(),
            pending: Default::default(),
        }));

        let worker = state.store().namespace("default");
        let mut kv = KeyValue { worker, state }.serde();

        kv.put(
            "hello",
            &toml::toml! {
                value = "really important value"

                [other.values]
                also_important = "hello"
            },
        )
        .unwrap()
        .forget();

        assert_eq!(
            "hello",
            kv.load::<toml::Value>("hello").unwrap()["other"]["values"]["also_important"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "really important value",
            kv.peek("hello").unwrap().as_map().idx("value").as_str()
        );

        assert_eq!(1, kv.search(field("other")).count());

        assert_eq!(
            1,
            kv.search(filter::<Record>(|r| {
                r.matches_label("hello", "default")
            }))
            .count()
        );
    }

    #[tokio::test]
    async fn test_kv_bg_sync() {
        let state = SharedState(Arc::new(super::Shared {
            store: Store::default(),
            pending: Default::default(),
        }));

        let worker = state.store().namespace("default");
        let mut kv = KeyValue { worker, state }.serde();

        kv.put(
            "hello",
            &toml::toml! {
                value = "really important value"

                [other.values]
                also_important = "hello"
            },
        )
        .unwrap()
        .await
        .unwrap();

        assert_eq!(
            "hello",
            kv.load::<toml::Value>("hello").unwrap()["other"]["values"]["also_important"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "really important value",
            kv.peek("hello").unwrap().as_map().idx("value").as_str()
        );

        assert_eq!(1, kv.search(field("other")).count());

        assert_eq!(
            1,
            kv.search(filter::<Record>(|r| {
                r.matches_label("hello", "default")
            }))
            .count()
        );

        assert_eq!(
            1,
            kv.state
                .store()
                .packer
                .flush()
                .inspect(|p| {
                    eprintln!("{p:?}");
                })
                .count()
        )
    }
}

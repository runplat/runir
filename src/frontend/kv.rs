use crate::{
    IRecord, Namespace, Query, Record, Worker, search::iter::Search, worker::BackgroundSync,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Error,
    ops::{Deref, DerefMut},
    path::PathBuf,
};
use tracing::{debug, trace};

use super::state::{SharedState, State};

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
    /// State that is shared across all workers
    shared: SharedState,
}

pub fn default_save_dir() -> std::io::Result<PathBuf> {
    let dot_folder = format!(".{}", env!("CARGO_PKG_NAME"));
    let dir = std::env::var("RUNIR_WORK_DIR")
        .map(|w| PathBuf::from(w))
        .ok()
        .unwrap_or(std::env::current_dir()?.join(&dot_folder));

    if !dir.exists() && dir.ends_with(dot_folder) {
        debug!("Creating .runir folder {dir:?}");
        std::fs::create_dir(&dir)?;
    } else if !dir.exists() {
        // Since this is passed into the process, do not attempt to create directory unless opt-in
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Directory set in RUNIR_WORK_DIR must exist",
        ));
    }

    Ok(dir)
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
        let save_dir = default_save_dir()?;
        let state = State::load(save_dir).await?;

        let mut shared = SharedState::from(state);
        shared.update_snapshot();
        Ok(KeyValue {
            worker: shared.store().namespace("default"),
            shared,
        })
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
        let save_dir = default_save_dir()?;
        self.shared.state.save(save_dir).await?;
        Ok(())
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
    #[inline]
    pub async fn save_as(&self, to: impl Into<PathBuf>) -> std::io::Result<()> {
        self.shared.state.save(to.into()).await
    }

    /// Returns a new key-value store scoped to a namespace
    #[inline]
    pub fn ns(&self, ns: impl Into<Namespace>) -> KeyValue {
        let ns = ns.into();

        Self {
            worker: self.shared.store().namespace(ns),
            shared: self.shared.clone(),
        }
    }

    /// Returns a key-value store interface for putting values that implement serde::Serialize
    #[inline]
    pub fn serde(&self) -> KeySerdeValue {
        KeySerdeValue {
            kv: KeyValue {
                worker: self
                    .shared
                    .store()
                    .namespace(self.worker.namespace().clone()),
                shared: self.shared.clone(),
            },
        }
    }

    /// Refreshes the key value store's indexes
    #[inline]
    pub fn refresh(&mut self) {
        if let Some(member_path) = self.worker.archive_member_path() {
            let updated = self
                .shared
                .state
                .refresh_index(&member_path, self.worker.cache_mut());
            trace!(
                updated,
                member = member_path.to_string_lossy().to_string(),
                "kv_refresh"
            );
        }
        self.shared.update_snapshot();
    }

    /// Searches over stored records w/ a query
    #[inline]
    pub fn search<'q>(
        &'q self,
        query: impl Into<Query<'q, Record>>,
    ) -> impl Iterator<Item = &'q Record> {
        let q = query.into();

        if let Some(store) = self.shared.snapshot() {
            store.search(q)
        } else {
            self.worker.cache().search(q)
        }
    }

    /// Lookup a record in the current namespace
    #[inline]
    fn lookup(&self, label: &str) -> Option<&Record> {
        self.worker
            .cache()
            .lookup(self.worker.namespace().clone(), label)
            .or_else(|| {
                self.shared
                    .snapshot()
                    .and_then(|s| s.lookup(self.worker.namespace().clone(), label))
            })
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
            .or_else(|| {
                self.shared
                    .snapshot()
                    .and_then(|s| s.lookup(self.worker.namespace().clone(), key))
                    .and_then(|r| flexbuffers::Reader::get_root(r.bytes()).ok())
            })
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
        match self.lookup(key).and_then(|r| r.load::<V>()) {
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
        match self.lookup(key).map(|r| r.bytes()) {
            Some(r) => Ok(r),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Could not find {key}"),
            )),
        }
    }
}

#[cfg(test)]
mod test {
    use super::{Get, KeyValue, Put};
    use crate::frontend::state::SharedState;
    use crate::{QueryBuilder, Record, field, filter, namespace};

    #[test]
    fn test_kv_put_get() {
        let state = SharedState::default();

        let worker = state.store().namespace("default");
        let mut kv = KeyValue {
            worker,
            shared: state,
        };

        kv.put("hello", b"hello").unwrap().forget();

        assert_eq!(b"hello", kv.get("hello").unwrap());
    }

    #[test]
    fn test_kv_put_get_serde() {
        let state = SharedState::default();

        let worker = state.store().namespace("default");
        let mut kv = KeyValue {
            worker,
            shared: state,
        }
        .serde();

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
    #[tracing_test::traced_test]
    async fn test_kv_bg_sync() {
        let shared = SharedState::default();

        let worker = shared.store().namespace("default");
        let mut kv = KeyValue { worker, shared }.serde();

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

        let mut parallel_ns = kv.ns("default").serde();
        parallel_ns
            .put(
                "hello2",
                &toml::toml! {
                    value = "another really important value"

                    [other.values]
                    also_important = "another hello"
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

        kv.shared.state.flush().await.unwrap();
        kv.refresh();
        assert!(
            kv.worker
                .cache()
                .lookup("default", "hello")
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

        let kv = KeyValue::open().await.unwrap();
        let count = kv
            .search(
                namespace("default")
                    .label("hello2")
                    .and(field("value").contains("another")),
            )
            .count();
        assert_eq!(1, count);
    }
}

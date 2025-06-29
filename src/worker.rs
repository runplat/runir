use std::path::PathBuf;

use crate::{
    IRecord, Index, Namespace, RawRecordable, Record, Storage, VecIndex,
    archive::{self, Entry, archive_to},
    store::{ArchiveMember, StoreSettings},
};
use bytes::Bytes;
use crossbeam::utils::Backoff;
use futures::{StreamExt, future::RemoteHandle};
use serde::Serialize;
use tokio::io::AsyncRead;

pub type BackgroundSync = RemoteHandle<std::io::Result<()>>;

/// A worker is an intermediary which handles a collection of records for a namespace
pub struct Worker {
    /// Namespace this worker belongs to
    namespace: Namespace,
    /// Records being written by this worker
    records: Vec<Record>,
    /// Store this worker is associated to
    store: Option<StoreSettings>,
    /// Record Cache
    ///
    /// Empty unless flush(..) is called
    cache: VecIndex<Record>,
}

impl Worker {
    /// Returns a reference to the worker's read cache
    ///
    /// Empty until flush(..) is called
    #[inline]
    pub fn cache(&self) -> &VecIndex<Record> {
        &self.cache
    }

    /// Returns a mutable reference to the workers cache
    #[inline]
    pub fn cache_mut(&mut self) -> &mut VecIndex<Record> {
        &mut self.cache
    }

    /// Sets a store pusher on this worker
    pub fn with_store(mut self, store: StoreSettings) -> Self {
        self.store.replace(store);
        self
    }

    /// Returns a reference to the namespace of this worker
    #[inline]
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    /// Stores an object into the current worker with name
    ///
    /// Returns true if the object was successfully stored, otherwise returns false
    #[inline]
    pub fn commit(&mut self, name: &str, obj: &[u8]) -> bool {
        let record = self
            .namespace
            .record(name)
            .commit(Bytes::copy_from_slice(obj));
        if record.is_valid() {
            self.records.push(record);
            true
        } else {
            false
        }
    }

    /// Stores an object into the current worker with name
    ///
    /// Returns true if the object was successfully stored, otherwise returns false
    #[inline]
    pub fn store<'a, T: Serialize + 'a>(
        &mut self,
        name: &str,
        obj: impl Into<RawRecordable<'a, T>>,
    ) -> bool {
        let record = self.namespace.store(name, obj);
        if record.is_valid() {
            self.records.push(record);
            true
        } else {
            false
        }
    }

    /// Authors a flexbuffer root that will become the committed value of the record
    #[inline]
    pub fn author(
        &mut self,
        name: &str,
        author: impl Fn(flexbuffers::Builder) -> flexbuffers::Builder,
    ) -> bool {
        let record = self.namespace.author(name, author);
        if record.is_valid() {
            self.records.push(record);
            true
        } else {
            false
        }
    }

    /// Pushes a record onto this worker
    ///
    /// Returns true if the record was pushed into state, false if the record's ns_chk did not match
    /// the current worker's ns_chk
    #[inline]
    pub fn push(&mut self, record: Record) -> bool {
        if record.ns_chk() == self.namespace.chk() {
            self.records.push(record);
            true
        } else {
            false
        }
    }

    /// Consumes worker state and returns an index
    #[inline]
    pub fn to_index<S: Storage<Record = Record>>(mut self) -> Index<Record, S> {
        let mut index = Index::<Record, S>::default();
        for r in self.records.drain(..) {
            index.index(r);
        }
        index
    }

    /// Restores the worker state from an input stream
    #[inline]
    pub async fn restore_from(
        &mut self,
        input: impl AsyncRead + Send + Unpin + 'static,
    ) -> std::io::Result<()> {
        let decoder = archive::TapeDecoder::default();
        let mut reader = tokio_util::codec::FramedRead::new(input, decoder);

        while let Some(entry) = reader.next().await {
            if matches!(
                entry,
                Ok(Entry::Other(..)) | Ok(Entry::Zeros) | Ok(Entry::Pending)
            ) {
                continue;
            }

            let record = entry.and_then(|e| Record::restore(e))?;
            self.records.push(record);
        }

        Ok(())
    }

    /// Drains all records and caches them into the worker-cache,
    /// and maps each record to an archive entry
    pub fn flush(&mut self) -> impl Iterator<Item = Entry> {
        self.records
            .drain(..)
            .inspect(|r| {
                // TODO: use index_with here later
                self.cache.index(r.clone());
            })
            .filter(|f| f.opts().is_archivable())
            .map(|f| Entry::Record(f))
    }

    /// Begins synchronizing data with the store in the background
    ///
    /// Returns None if store settings are not configured for this worker, otherwise
    /// returns a BackgroundSync future
    pub fn sync(&mut self) -> std::io::Result<BackgroundSync> {
        if let Some(settings) = self.store.clone() {
            let output_path = settings.archive_path(&self.namespace);
            let entries = self.flush().map(|e| Ok(e)).collect::<Vec<_>>();
            let packer = settings.packer().clone();

            let handle = crate::util::spawn(async move {
                // TODO: Make this output stream modular, good enough for now
                let output = tokio::fs::File::create_new(&output_path).await?;
                let stream = futures::stream::iter(entries);
                let manifest = archive_to(stream, output).await?;

                let backoff = Backoff::new();
                let mut member = ArchiveMember::Unpacked {
                    path: output_path,
                    manifest,
                };
                while let Some(retry) = packer.push(member) {
                    member = retry;
                    backoff.spin();
                }

                Ok::<_, std::io::Error>(())
            })?;

            Ok(handle)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Worker does not have any store settings",
            ))
        }
    }

    /// Returns the archive member path used by this worker
    #[inline]
    pub fn archive_member_path(&self) -> Option<PathBuf> {
        self.store.as_ref().map(|s| s.archive_path(&self.namespace))
    }
}

impl<T: Into<Namespace>> From<T> for Worker {
    fn from(value: T) -> Self {
        Worker {
            namespace: value.into(),
            records: vec![],
            store: None,
            cache: VecIndex::default(),
        }
    }
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("namespace", &self.namespace.ns_uuid())
            .field("records", &self.records)
            .finish()
    }
}

#[cfg(test)]
mod test {
    use crate::{
        RecordableExtensions, ToNamespace, Worker,
        archive::{Entry, FileEntryReference, archive_to},
    };
    use sha2::Digest;
    use toml::toml;

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_worker_archive_to() {
        let mut worker = Worker::from("test".to_namespace());

        assert!(
            worker.store(
                "record_one",
                toml! {
                    value = "hello world"
                }
                .indexable(),
            )
        );

        assert!(worker.store(
            "record_two",
            &toml! {
                value = "goodbye world"
            },
        ));

        assert!(
            worker.store(
                "record_three",
                toml! {
                    value = "goodbye world"
                }
                .no_archive(),
            )
        );

        assert!(worker.author("record_four", |mut b| {
            let mut map = b.start_map();
            map.push("value", "hello hello");
            map.end_map();
            b
        },));

        std::fs::remove_file("test.tar").ok();
        let archive_file = tokio::fs::File::create_new("test.tar").await.unwrap();
        let entries = futures::stream::iter(worker.flush().map(|e| Ok(e)));
        let manifest = archive_to(entries, archive_file).await.unwrap();
        assert!(manifest.is_valid());

        let mut restoring = Worker::from("test");
        let archive_file = tokio::fs::File::open("test.tar").await.unwrap();
        restoring.restore_from(archive_file).await.unwrap();

        let encoded = manifest.journal_entries().unwrap();
        assert_eq!(3, encoded.len());
        eprintln!("{encoded:#x?}");

        let archive_file = tokio::fs::File::open("test.tar").await.unwrap();
        let references = crate::archive::scan_for_references(archive_file)
            .await
            .unwrap();
        for reference in references {
            if let Entry::Reference(FileEntryReference {
                header,
                digest,
                offset,
            }) = reference
            {
                eprintln!("offset: {offset}, digest: {:x}", digest.finalize());
                eprintln!("{header}");
            }
        }
    }
}

use std::sync::Arc;
use crate::{
    IRecord, Index, Record, Storage,
    archive::{self, Entry, archive_to},
    store::{ArchiveMember, StoreSettings},
};
use crossbeam::utils::Backoff;
use futures::{AsyncRead, StreamExt, future::RemoteHandle};
use tracing::debug;

/// Type-alias for the type returned by Worker::sync(..)
pub type BackgroundSync = RemoteHandle<std::io::Result<()>>;

#[derive(Clone)]
pub struct SharedWorker {
    worker: Arc<parking_lot::RwLock<Worker>>,
}

impl SharedWorker {
    /// Pushes a record to the inner worker
    #[inline]
    pub fn push(&self, rec: impl Into<Record>) -> bool {
        self.worker.write().push(rec.into())
    }

    /// Calls sync on the underlying worker
    ///
    /// This will:
    /// 1) Flush any pending records
    /// 2) Push those records to the store for processing
    /// 3) Index all records directly to the worker
    #[inline]
    pub fn sync(&self) -> std::io::Result<BackgroundSync> {
        self.worker.write().sync()
    }
}

/// A worker is an intermediary which handles a collection of records for a namespace
#[derive(Default)]
pub struct Worker {
    /// Records being written by this worker
    records: Vec<Record>,
    /// Store this worker is associated to
    store: Option<StoreSettings>,
}

impl Worker {
    /// Sets a store pusher on this worker
    #[inline]
    pub fn with_store(mut self, store: StoreSettings) -> Self {
        self.store.replace(store);
        self
    }

    /// Pushes a record onto this worker
    ///
    /// Returns true if the record was pushed into state, false if the record's ns_chk did not match
    /// the current worker's ns_chk
    #[inline]
    pub fn push(&mut self, record: Record) -> bool {
        if record.is_valid() {
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
        let mut reader = asynchronous_codec::FramedRead::new(input, decoder);

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

    /// Begins synchronizing data with the store in the background
    ///
    /// Returns None if store settings are not configured for this worker, otherwise
    /// returns a BackgroundSync future
    pub fn sync(&mut self) -> std::io::Result<BackgroundSync> {
        if let Some(settings) = self.store.clone() {
            let output_path = settings.archive_path();
            let mut total_size = 0;
            let entries = self.flush().inspect(|r| {
                total_size += r.header().size();
                total_size += 512;
                total_size += 512 - (total_size % 512);
            }).map(|e| Ok(e)).collect::<Vec<_>>();
            let packer = settings.packer().clone();

            let handle = crate::util::spawn(async move {
                // TODO: Make this output stream modular, good enough for now
                /*
                16MiB = 32678 512-blocks,
                    1 record = min 2 blocks
                    16384 records max per 16 MiB
                 */
                debug!(total_size, "Creating new archive_member at {output_path:?}");
                let output = crate::util::fs::create_new(&output_path).await?;
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

    /// Drains all records and caches them into the worker-cache,
    /// and maps each record to an archive entry
    fn flush(&mut self) -> impl Iterator<Item = Entry> + '_ {
        self.records
            .drain(..)
            .filter(|f| f.opts().is_archivable())
            .map(|f| Entry::Record(f))
    }
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            // .field("namespace", &self.namespace.ns_uuid())
            .field("records", &self.records)
            .finish()
    }
}

impl From<Worker> for SharedWorker {
    fn from(value: Worker) -> Self {
        Self {
            worker: Arc::new(parking_lot::RwLock::new(value)),
        }
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
        let mut worker = Worker::default(); // ("test".to_namespace());
        let ns = "test".to_namespace();
        assert!(
            worker.push(
                ns.store(
                    "record_one",
                    toml! {
                        value = "hello world"
                    }
                    .indexable(),
                )
            )
        );

        assert!(worker.push(ns.store(
            "record_two",
            &toml! {
                value = "goodbye world"
            },
        )));

        assert!(
            worker.push(
                ns.store(
                    "record_three",
                    toml! {
                        value = "goodbye world"
                    }
                    .no_archive(),
                )
            )
        );

        assert!(worker.push(ns.author("record_four", |mut b| {
            let mut map = b.start_map();
            map.push("value", "hello hello");
            map.end_map();
            b
        })));

        std::fs::remove_file("test.tar").ok();
        let archive_file = crate::util::fs::create_new("test.tar").await.unwrap();
        let entries = futures::stream::iter(worker.flush().map(|e| Ok(e)));
        let manifest = archive_to(entries, archive_file).await.unwrap();
        assert!(manifest.is_valid());

        let mut restoring = Worker::default();
        let archive_file = crate::util::fs::open("test.tar").await.unwrap();
        restoring.restore_from(archive_file).await.unwrap();

        let encoded = manifest.journal_entries().unwrap();
        assert_eq!(3, encoded.len());
        eprintln!("{encoded:#x?}");

        let archive_file = crate::util::fs::open("test.tar").await.unwrap();
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

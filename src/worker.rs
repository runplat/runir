use crate::{
    archive::{self, Entry, Manifest}, record::Namespace, Index, RawRecordable, Record
};
use futures::StreamExt;
use serde::Serialize;
use std::io::Error;
use tokio::io::{AsyncRead, AsyncWrite};

/// A worker is an intermediary which handles a collection of records for a namespace
#[derive(Clone)]
pub struct Worker {
    /// Namespace this worker belongs to
    namespace: Namespace,
    /// Records being written by this worker
    records: Vec<Record>,
}

impl Worker {
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
    pub fn to_index(mut self) -> Index {
        let mut index = Index::default();
        for r in self.records.drain(..) {
            index.index(&r);
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

    /// Archives the worker state to an output stream
    /// 
    /// Returns a record containing a manifest of the contents written to the output stream
    #[inline]
    pub async fn archive_to(
        &self,
        output: impl AsyncWrite + Send + Unpin + 'static,
    ) -> std::io::Result<Manifest> {
        use futures::sink::SinkExt;
        let encoder = archive::TapeEncoder::default();

        let mut writer = tokio_util::codec::FramedWrite::new(output, encoder);

        // Creates archive entries of all archivable records and encodes to the output stream
        let mut stream = futures::stream::iter(
            self.records
                .iter()
                .filter(|f| f.opts().is_archivable())
                .map(|f| Ok(Entry::Record(f.clone()))),
        );

        // Send a stream of entries
        writer
            .send_all(&mut stream)
            .await
            .map_err(|e| Error::new(std::io::ErrorKind::Interrupted, e))?;

        // Need to send this last to indicate the end of the archive
        writer
            .send(archive::Entry::Zeros)
            .await
            .map_err(|e| Error::new(std::io::ErrorKind::Interrupted, e))?;

        // Applies the digest of the current state of the archive to all journal entries
        writer.encoder_mut().stamp_source_digest();

        // Close the writer
        writer
            .close()
            .await
            .map_err(|e| Error::new(std::io::ErrorKind::Interrupted, e))?;

        let manifest = writer.encoder().create_manifest_record();
        Ok(manifest)
    }
}

impl<T: Into<Namespace>> From<T> for Worker {
    fn from(value: T) -> Self {
        Worker {
            namespace: value.into(),
            records: vec![],
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{archive::Entry, RecordableExtensions, Worker};
    use sha2::Digest;
    use toml::toml;

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_worker_archive_to() {
        let mut worker = Worker::from("test");

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

        std::fs::remove_file("test.tar").ok();
        let archive_file = tokio::fs::File::create_new("test.tar").await.unwrap();
        let manifest = worker.archive_to(archive_file).await.unwrap();
        assert!(manifest.is_valid());

        let mut restoring = Worker::from("test");
        let archive_file = tokio::fs::File::open("test.tar").await.unwrap();
        restoring.restore_from(archive_file).await.unwrap();

        let index = restoring.to_index();
        let value = index.find("record_one", "test");
        let toml = value.unwrap().load::<toml::Value>().unwrap();
        assert_eq!("hello world", toml["value"].as_str().unwrap());
        assert!(index.find("record_three", "test").is_none());

        let encoded = manifest.journal_entries().unwrap();
        assert_eq!(2, encoded.len());
        eprintln!("{encoded:#x?}");

        let archive_file = tokio::fs::File::open("test.tar").await.unwrap();
        let references = crate::archive::scan_for_references(archive_file).await.unwrap();
        for reference in references {
            if let Entry::Reference { header, digest, offset } = reference {
                eprintln!("offset: {offset}, digest: {:x}", digest.finalize());
                eprintln!("{header}");
            }
        }
    }
}

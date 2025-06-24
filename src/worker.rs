use crate::{
    archive::{self, Entry}, record::Namespace, Index, RawRecordable, Record
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

            let is_manifest = entry
                .as_ref()
                .map(|e| e.header().name() == "MANIFEST")
                .unwrap_or_default();
            if is_manifest {
                // TODO: Use this list to cross-verify records that have been restored
                continue;
            }

            let record = entry.and_then(|e| Record::restore(e))?;
            self.records.push(record);
        }

        Ok(())
    }

    /// Archives the worker state to an output stream
    #[inline]
    pub async fn archive_to(
        &self,
        output: impl AsyncWrite + Send + Unpin + 'static,
    ) -> std::io::Result<()> {
        use futures::sink::SinkExt;
        let encoder = archive::TapeEncoder::default();

        let mut writer = tokio_util::codec::FramedWrite::new(output, encoder);

        // Creates archive entries of all archivable records and encodes to the output stream
        let mut stream = futures::stream::iter(
            self.records
                .iter()
                .filter(|f| f.opts().is_archivable())
                .map(|f| f.archive()),
        );

        writer
            .send_all(&mut stream)
            .await
            .map_err(|e| Error::new(std::io::ErrorKind::Interrupted, e))?;

        // Stores a manifest of the archived entries from the TapeEncoder
        let manifest = writer.encoder().create_manifest()?;
        writer
            .send(manifest)
            .await
            .map_err(|e| Error::new(std::io::ErrorKind::Interrupted, e))?;

        // Need to send this last to indicate the end of the archive
        writer
            .send(archive::Entry::Zeros)
            .await
            .map_err(|e| Error::new(std::io::ErrorKind::Interrupted, e))?;

        // Close the writer
        writer
            .close()
            .await
            .map_err(|e| Error::new(std::io::ErrorKind::Interrupted, e))?;
        Ok(())
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
    use crate::{RecordableExtensions, Worker};
    use toml::toml;

    #[tokio::test]
    async fn test_worker_archive_to() {
        let mut worker = Worker::from("test");

        assert!(worker.store(
            "record_one",
            toml! {
                value = "hello world"
            }
            .indexable(),
        ));

        assert!(worker.store(
            "record_two",
            &toml! {
                value = "goodbye world"
            },
        ));

        assert!(worker.store(
            "record_three",
            toml! {
                value = "goodbye world"
            }.no_archive(),
        ));

        std::fs::remove_file("test.tar").ok();
        let archive_file = tokio::fs::File::create_new("test.tar").await.unwrap();
        worker.archive_to(archive_file).await.unwrap();

        let mut restoring = Worker::from("test");
        let archive_file = tokio::fs::File::open("test.tar").await.unwrap();
        restoring.restore_from(archive_file).await.unwrap();

        let ns = restoring.namespace.clone();
        let index = restoring.to_index();
        let value = index.find("record_one", &ns);
        let toml = value.unwrap().load::<toml::Value>().unwrap();
        assert_eq!("hello world", toml["value"].as_str().unwrap());
        assert!(index.find("record_three", &ns).is_none());
    }
}

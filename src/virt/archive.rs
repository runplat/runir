use asynchronous_codec::{FramedWrite, FramedWriteParts};
use futures::SinkExt;
use crate::{archive::{Entry, TapeEncoder}, store::ArchiveMember};
use super::{vol::{Volume, VolumeTarget}, Virtual};

impl<T: VolumeTarget> Volume<T, TapeEncoder> {
    /// Creates a new volume w/ target for archiving
    #[inline]
    pub fn new_archive(target: T) -> Self {
        Self::from_parts((target, TapeEncoder::default()))
    }

    /// Encodes a series of Records using [TapeEncoder] and writes them sequentially into the underlying target
    #[inline]
    pub async fn archive_batch(self, batch: Vec<Entry>) -> std::io::Result<Self> {
        let (target, encoder) = self.into_parts();
        let mut writer = FramedWrite::new(target, encoder);

        for r in batch {
            writer.feed(r).await?;
        }

        writer.close().await?;

        let FramedWriteParts {
            io,
            encoder,
            buffer,
            ..
        } = writer.into_parts();

        if !buffer.is_empty() {
            unreachable!("This means that close() did not finish flushing")
        }
        Ok(Self::from_parts((
            io,
            encoder,
        )))
    }

    /// Swaps the inner target and snapshots the state into an index
    #[inline]
    pub fn swap_and_archive(&mut self, next: T) -> std::io::Result<ArchiveMember> {
        let (target, encoder) = self.parts_mut();
        
        let target = std::mem::replace(target, next);
        let encoder = std::mem::replace(encoder, TapeEncoder::default());

        let to_archive = Self::from_parts((target, encoder));
        to_archive.to_archive()
    }

    /// Consumes the volume and returns an archive member
    #[inline]
    pub fn to_archive(self) -> std::io::Result<ArchiveMember> {
        let (target, mut encoder) = self.into_parts();
        let path = target.path().as_ref().to_path_buf();

        let target = target.freeze()?;
        let manifest = encoder.stamp_manifest();

        let mut records = vec![];
        let entries = manifest.journal_entries()?;
        let count = entries.len();
        for e in entries {
            let data = Virtual::new(e, target.clone())?;
            if let Some(record) = data.materialize() {
                records.push(record);
            }
        }

        if count != records.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Would have returned partial data",
            ));
        }
        Ok(ArchiveMember::Volume {
            path,
            manifest,
            records,
        })
    }
}

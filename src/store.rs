use crate::{
    Namespace, Record, VirtualData, Worker,
    archive::{Entry, FileEntryReference, Manifest},
};
use ahash::HashSet;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use tracing::error;

/// Store coordinates multiple workers and namespaces
#[derive(Default, Clone)]
pub struct Store {
    /// Workers owned by this store
    workers: Vec<Worker>,
}

impl Store {
    /// Returns a mutable reference for a worker to a specific namespace
    #[inline]
    pub fn namespace(&mut self, ns: impl Into<Namespace>) -> &mut Worker {
        self.workers.push(Worker::from(ns));
        self.workers.last_mut().expect("should exist")
    }

    /// Builds and archive of the current state of the store
    ///
    /// Writes all intermediate files to tempdir
    pub async fn archive(&self) -> std::io::Result<StoreArchive> {
        let tmp = std::env::temp_dir();

        let build_id = Namespace::ephemeral();
        let output_dir = tmp.join(format!("{:x}", build_id.chk()));

        let mut archived = vec![];
        std::fs::create_dir(&output_dir)?;

        for (idx, worker) in self.workers.iter().enumerate() {
            let dest = output_dir.join(format!("{:x}_{:x}.tar", worker.namespace().chk(), idx));
            let file = tokio::fs::File::create_new(&dest).await?;
            let manifest = worker.archive_to(file).await?;
            archived.push(ArchiveMember::Unpacked(dest, manifest));
        }

        Ok(StoreArchive {
            archived,
            output_dir,
        })
    }
}

/// Store archive is the intermediate build output of the archival pipeline
#[derive(Debug)]
pub struct StoreArchive {
    /// List of worker archives in the output dir
    archived: Vec<ArchiveMember>,
    /// Output directory of store files
    output_dir: PathBuf,
}

/// Enumeration of archive member state
#[derive(Debug)]
pub enum ArchiveMember {
    /// Archive member is in an intermediate state on disk
    Unpacked(PathBuf, Manifest),
    /// Archive member is already packed
    Packed {
        /// Path to the source of this packed archive member
        path: PathBuf,
        /// Offset into the pack where the member's source begins
        offset: u64,
        /// Length of the source
        len: u32,
        /// Manifest listing packed records
        manifest: Manifest,
    },
}

impl ArchiveMember {
    /// Returns records stored in archive member
    ///
    /// Note: These records will always be virtual based records, this gurantees that
    /// records returned from an archive member were available on disk when the records
    /// were returned
    pub async fn get_records(&self) -> std::io::Result<Vec<Record>> {
        match self {
            ArchiveMember::Unpacked(path_buf, manifest) => {
                let source = tokio::fs::File::open(path_buf).await?;
                let mmap = unsafe { memmap2::Mmap::map(&source)? };
                let mmap = Arc::new(mmap);

                let mut records = vec![];
                for entry in manifest.journal_entries()?.iter() {
                    let data = VirtualData::new(entry.clone(), mmap.clone())?;
                    if let Some(rec) = data.materialize() {
                        records.push(rec);
                    }
                }
                Ok(records)
            }
            ArchiveMember::Packed {
                offset,
                len,
                manifest,
                path,
            } => {
                let source = tokio::fs::File::open(path).await?;
                let mmap = unsafe {
                    memmap2::MmapOptions::new()
                        .offset(*offset)
                        .len(*len as usize)
                        .map(&source)?
                };
                let mmap = Arc::new(mmap);

                let mut records = vec![];
                for entry in manifest.journal_entries()?.iter() {
                    // Since we're packed in a single file, the zero-byte paddings aren't going
                    // to be present, new_packed ensures the constructor is aware of this when validating
                    // the data before returning the record
                    let data = VirtualData::new_packed(entry.clone(), mmap.clone())?;
                    if let Some(rec) = data.materialize() {
                        records.push(rec);
                    }
                }
                Ok(records)
            }
        }
    }

    /// Returns the manifest of the archive member
    #[inline]
    pub fn manifest(&self) -> &Manifest {
        match self {
            ArchiveMember::Unpacked(.., manifest) => manifest,
            ArchiveMember::Packed { manifest, .. } => manifest,
        }
    }
}

impl StoreArchive {
    /// "Unpacks" a store.tar and returns a StoreArchive
    ///
    /// The .tar is not actually unpacked as in it's files are not written to disk, instead
    /// the content boundaries are found in order to create references into the file
    ///
    /// This allows records to be materialized from the store.tar without needing to expand.
    #[inline]
    pub async fn unpack(path: PathBuf) -> std::io::Result<Self> {
        if !path.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Path must be to an existing file",
            ));
        }
        let output_dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(PathBuf::from("/"));
        let references =
            crate::archive::scan_for_references(tokio::fs::File::open(&path).await.unwrap())
                .await
                .unwrap();

        let source = tokio::fs::File::open(&path).await?;
        let mmap = unsafe { memmap2::MmapOptions::new().map(&source)? };
        let mmap = Arc::new(mmap);

        let source = Sha256::digest(&mmap[..]);

        let mut members = vec![];

        let mut cursor = 0;
        let mut records = HashSet::default();
        for r in references {
            match r {
                Entry::Reference(FileEntryReference {
                    header,
                    digest,
                    offset,
                }) => {
                    let digest: [u8; 32] = digest.finalize().into();
                    if let Some((ns_chk, uuid, opts)) = header.split_name_for_record() {
                        if opts.is_manifest() {
                            let (key, crc) = uuid.as_u64_pair();
                            // The entries that preceded this manifest must match the entries contained in this manifest
                            // To unpack, first materialize the manifest in order to get the manifest back which is able to list the entries
                            // Next, find the offset/len of the sector of the pack belonging to the the member
                            // Finally, create a Packed archive entry

                            let virt = VirtualData::new(
                                crate::archive::JournalEntry::Record(crate::RecordExtent {
                                    source: source.into(),
                                    content: digest,
                                    offset: offset as u64,
                                    len: header.size() as u32,
                                    key,
                                    crc,
                                    ts: header.last_modified(),
                                    ns_chk,
                                    opts: opts.encode(),
                                }),
                                mmap.clone(),
                            )?;
                            if let Some(record) = virt.materialize() {
                                let manifest = Manifest { record };

                                for entry in manifest.journal_entries()?.iter() {
                                    if !records.remove(entry.content()) {
                                        return Err(std::io::Error::new(
                                            std::io::ErrorKind::InvalidData,
                                            "Pack is incomplete",
                                        ));
                                    }
                                }

                                if !records.is_empty() {
                                    error!("Found orphaned records {:#?}", records);
                                    return Err(std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "Packed manifest is corrupted",
                                    ));
                                }

                                members.push(ArchiveMember::Packed {
                                    path: path.clone(),
                                    offset: cursor,
                                    // Len of source is the current offset, minus the header bytes, minus the cursor
                                    len: (offset.saturating_sub(512).saturating_sub(cursor as usize) as u32),
                                    manifest,
                                });
                            }

                            // Cursor is the current offset of the file header, plus the size of the manifest, plus padding to the nearest 512 block
                            cursor = offset as u64 + header.size() as u64;
                            let padding = cursor % 512;
                            cursor += 512 - padding;
                        } else {
                            records.insert(digest);
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(Self {
            archived: members,
            output_dir,
        })
    }

    /// Packs worker archives from the output directory into a single store archive
    pub async fn pack(&self) -> std::io::Result<Manifest> {
        use futures::sink::SinkExt;
        let encoder = crate::archive::TapeEncoder::default();
        let dest = self.output_dir.join("PACKING");
        let output = tokio::fs::File::create_new(&dest).await.inspect_err(|_| {
            error!("Previous packing attempt did not succeed");
        })?;

        let mut writer = tokio_util::codec::FramedWrite::new(output, encoder);

        for member in self.archived.iter() {
            let records = member.get_records().await?;
            let manifest = member.manifest();
            let to_enc =
                futures::stream::iter(records.iter().map(|r| Ok(Entry::Record(r.clone())))).chain(
                    futures::stream::once(async { Ok(Entry::Record(manifest.record.clone())) }),
                );
            tokio::pin!(to_enc);
            writer.send_all(&mut to_enc).await?;
        }

        writer.close().await?;
        writer.encoder_mut().stamp_source_digest();

        let manifest = writer.encoder_mut().create_manifest();

        let completed_dest = self.store_tar_path();
        std::fs::rename(dest, &completed_dest)?;
        Ok(manifest)
    }

    /// Returns the output path of the store.tar
    #[inline]
    pub fn store_tar_path(&self) -> PathBuf {
        self.output_dir.join("store.tar")
    }

    /// Returns members in the store archive
    #[inline]
    pub fn members(&self) -> impl Iterator<Item = &ArchiveMember> {
        self.archived.iter()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_store_build_archive() {
        let mut store = Store::default();
        {
            let worker = store.namespace("test_ns_1");
            assert!(worker.author("record_1", |mut b| {
                b.start_map().push("value", "hello world");
                b
            }));

            assert!(worker.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world");
                b
            }));
        }

        {
            let worker = store.namespace("test_ns_2");
            assert!(worker.author("record_1", |mut b| {
                b.start_map().push("value", "hello world 2");
                b
            }));

            assert!(worker.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world 2");
                b
            }));
        }

        {
            let worker = store.namespace("test_ns_1");
            assert!(worker.author("record_1", |mut b| {
                b.start_map().push("value", "hello world 2");
                b
            }));

            assert!(worker.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world");
                b
            }));
        }

        let store_archive = store.archive().await.unwrap();
        eprintln!("{:#?}", store_archive);

        let packed = store_archive.pack().await.unwrap();
        eprintln!("{:#?}", packed.journal_entries().unwrap());

        let store = store_archive.store_tar_path();
        let store_archive = StoreArchive::unpack(store).await.unwrap();
        eprintln!("{store_archive:#?}");
        for member in store_archive.members() {
            let records = member.get_records().await.unwrap();
            for r in records {
                assert!(r.is_valid());
                eprintln!("{} is valid!", r.uuid());
            }
        }
    }
}

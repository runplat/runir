use crate::{
    archive::{Entry, FileEntryReference, Manifest}, queue::Pusher, util::Intern, IRecord, Index, Namespace, Queue, Record, Storage, VecIndex, Virtual, Worker
};
use ahash::{HashSet, HashSetExt};
use anyhow::anyhow;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{io::Error, path::{Path, PathBuf}
};
use tracing::{debug, error};

pub trait ToArchiveMember {
    fn to_archive_member(self, path: impl Into<PathBuf>) -> ArchiveMember;
}

impl<R: IRecord, S: Storage<Record = R>> ToArchiveMember for Index<R, S> {
    fn to_archive_member(self, path: impl Into<PathBuf>) -> ArchiveMember {
        let mut index = VecIndex::default();

        for r in self.storage().iter_records() {
            index.index(r.to_record());
        }
        ArchiveMember::Index { path: path.into(), index }
    }
}

/// Store settings contains options for workers to use during
/// their operations
#[derive(Clone)]
pub struct StoreSettings {
    /// Name of the archive
    archive: &'static str,
    /// Ephemeral namespace
    session_ns: Namespace,
    /// Main working directory for storing unpacked archive-members
    work_dir: PathBuf,
    /// Queue for pushing ArchiveMembers for packing
    packer: Pusher<ArchiveMember>,
}

impl StoreSettings {
    /// Returns the path a member should use for their archive data
    #[inline]
    pub fn archive_path(&self) -> PathBuf {
        let archive_out = format!(
            "{}_{:x}_{:x}",
            self.archive,
            self.session_ns.chk(),
            Namespace::ephemeral().chk()
        );
        self.work_dir.join(archive_out)
    }

    /// Returns an interface to push work to the store for packing
    #[inline]
    pub fn packer(&self) -> &Pusher<ArchiveMember> {
        &self.packer
    }
}

/// Store centralizes record archival by distributing a queue to workers
/// which ingest incoming data
///
/// When Store::archive(..) is called, the queue will be flushed and all records will
/// be written to an intermediate archive member
///
/// If data must be persisted long-term or for transport, a collection of members can be packed
/// into a single archive file, at this point any left-over member artifacts can be purged
pub struct Store {
    /// Queue for receiving archive members for packing
    pub(crate) packer: Queue<ArchiveMember>,
    /// Working directory for this store
    work_dir: PathBuf,
    /// Name of the archive
    archive: &'static str,
}

impl Store {
    /// Sets the archive name
    #[inline]
    pub fn set_archive(&mut self, archive: &'static str) {
        self.archive = archive
    }

    /// Returns a store w/ work_dir set
    #[inline]
    pub fn work_dir(path: impl Into<PathBuf>) -> Self {
        Self {
            packer: Default::default(),
            work_dir: path.into(),
            archive: "store",
        }
    }

    /// Returns a mutable reference for a worker to a specific namespace
    #[inline]
    pub fn worker(&self) -> Worker {
        Worker::default().with_store(StoreSettings {
            archive: self.archive,
            session_ns: Namespace::ephemeral(),
            work_dir: self.work_dir.clone(),
            packer: self.packer.pusher().unwrap(),
        })
    }

    /// Pushes an archive member to the store
    #[inline]
    pub fn pack(&self, member: ArchiveMember) -> crate::Result<()> {
        match self.packer.pusher() {
            Some(push) => {
                push.ensure_push(member);
                Ok(())
            },
            None => Err(anyhow!("Store is closed").into()),
        }
    }

    /// Flushes all archive members and returns a StoreArchive
    #[inline]
    pub fn archive(&self) -> StoreArchive {
        let mut archived = vec![];

        for member in self.packer.flush() {
            archived.push(member);
        }

        StoreArchive {
            members: archived,
            output_dir: self.work_dir.clone(),
            archive: self.archive,
        }
    }
}

/// Store archive is the intermediate build output of the archival pipeline
#[derive(Debug)]
pub struct StoreArchive {
    /// List of worker archives in the output dir
    pub(crate) members: Vec<ArchiveMember>,
    /// Output directory of store files
    pub(crate) output_dir: PathBuf,
    /// Name of the archive
    pub(crate) archive: &'static str,
}

/// Enumeration of archive member state
#[derive(Debug, Clone)]
pub enum ArchiveMember {
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
    Volume {
        /// "Symbolic" volume path
        path: PathBuf,
        /// Manifest
        manifest: Manifest,
        /// Records stored in this volume
        records: Vec<Record>,
    },
    Index {
        /// "Symbolic" index path
        path: PathBuf,
        /// Indexed Records
        index: VecIndex<Record>,
    },
}

impl ArchiveMember {
    /// Returns the path of this archive member
    #[inline]
    pub fn path(&self) -> &PathBuf {
        match self {
            ArchiveMember::Packed { path, .. } => path,
            ArchiveMember::Volume { path, .. } => path,
            ArchiveMember::Index { path, .. } => path,
        }
    }

    /// Returns records stored in archive member
    ///
    /// Note: These records will always be virtual based records, this gurantees that
    /// records returned from an archive member were available on disk when the records
    /// were returned
    pub fn get_records(&self) -> std::io::Result<Vec<Record>> {
        match self {
            ArchiveMember::Packed {
                offset,
                len,
                manifest,
                path,
            } => {
                let source = std::fs::File::open(path)?;
                let mmap = unsafe {
                    memmap2::MmapOptions::new()
                        .offset(*offset)
                        .len(*len as usize)
                        .map(&source)?
                };
                let mmap = Bytes::from_owner(mmap);

                let mut records = vec![];
                let entries = manifest.journal_entries()?;
                // debug!("Mapping journal entries {entries:#?}");
                for entry in entries.iter() {
                    // Since we're packed in a single file, the zero-byte paddings aren't going
                    // to be present, new_packed ensures the constructor is aware of this when validating
                    // the data before returning the record
                    let data = Virtual::new(entry.clone(), mmap.clone())?;
                    if let Some(rec) = data.materialize() {
                        records.push(rec);
                    }
                }
                Ok(records)
            }
            ArchiveMember::Volume {
                records, manifest, ..
            } => {
                let entries = manifest.journal_entries()?;
                let mut entries =
                    entries
                        .iter()
                        .enumerate()
                        .fold(ahash::HashSet::new(), |mut s, (i, r)| {
                            s.insert((i, r.uuid(), *r.content()));
                            s
                        });
                let mut validated = vec![];
                for (i, r) in records.iter().enumerate().filter(|r| r.1.is_valid()) {
                    if entries.remove(&(i, r.uuid(), r.content())) {
                        validated.push(r.clone());
                    }
                }

                if !entries.is_empty() {
                    return Err(Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Records in archive member do not match received manifest",
                    ));
                }

                debug!(
                    count = validated.len(),
                    "Validated records from volume member"
                );
                Ok(validated)
            }
            ArchiveMember::Index { index, .. } => Ok(index
                .storage()
                .iter()
                .cloned()
                .collect::<Vec<_>>()),
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
    pub async fn unpack(path: impl AsRef<Path>) -> std::io::Result<Self> {
        if !path.as_ref().is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Path must be to an existing file",
            ));
        }

        let archive = path
            .as_ref()
            .file_name()
            .expect("should have a name")
            .to_string_lossy()
            .trim_end_matches(".tar")
            .to_string();

        let output_dir = path
            .as_ref()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(PathBuf::from("/"));
        let references =
            crate::archive::scan_for_references(crate::util::fs::open(path.as_ref()).await?)
                .await?;

        let source = std::fs::File::open(path.as_ref())?;
        let mmap = unsafe { memmap2::MmapOptions::new().map(&source)? };
        let mmap = Bytes::from_owner(mmap);

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

                            let virt = Virtual::new(
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
                                    if !records.remove(&entry.uuid()) {
                                        error!("Did not remove {}: {}", entry.uuid(), hex::encode(entry.content()));
                                        return Err(std::io::Error::new(
                                            std::io::ErrorKind::InvalidData,
                                            "Pack is incomplete",
                                        ));
                                    }
                                }

                                if !records.is_empty() {
                                    error!(
                                        "Found orphaned records {:#?}",
                                        records.iter().map(|r| hex::encode(r)).collect::<Vec<_>>()
                                    );
                                    error!("Manifest has: {:#?}", manifest.journal_entries()?);
                                    error!(
                                        "Manifest digest: {}",
                                        hex::encode(manifest.record.data.digest().finalize())
                                    );
                                    return Err(std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "Packed manifest is corrupted",
                                    ));
                                }

                                members.push(ArchiveMember::Packed {
                                    path: path.as_ref().to_path_buf(),
                                    offset: cursor,
                                    // Len of source is the current offset, minus the header bytes, minus the cursor
                                    len: (offset.saturating_sub(512).saturating_sub(cursor as usize)
                                        as u32),
                                    manifest,
                                });
                            }

                            // Cursor is the current offset of the file header, plus the size of the manifest, plus padding to the nearest 512 block
                            cursor = offset as u64 + header.size() as u64;
                            let padding = cursor % 512;
                            cursor += 512 - padding;
                        } else {
                            if let Some((_, uuid, _)) = header.split_name_for_record() {
                                records.insert(uuid);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(Self {
            members,
            output_dir,
            archive: archive.intern(),
        })
    }

    /// Packs worker archives from the output directory into a single store archive
    pub async fn pack(&self) -> std::io::Result<()> {
        use futures::sink::SinkExt;
        let completed_dest = self.store_tar_path();

        let dest = self.output_dir.join(format!("{}.PACKING", self.archive));

        let file = crate::util::fs::create_new(&dest).await.inspect_err(|_| {
            error!("Previous packing attempt did not succeed");
        })?;

        let encoder = crate::archive::TapeEncoder::default();

        let mut writer = asynchronous_codec::FramedWrite::new(file, encoder);

        let mut cleanup = vec![];

        let mut dedupe = HashSet::new();

        for member in self.members.iter() {
            dedupe.clear(); // Dedupe at the member level

            // Prepares a clean slate for the next "stamp"
            writer.encoder_mut().next_stamp();
            let records = member.get_records()?;
            let including = records
                .iter()
                .filter(|r| {
                    dedupe.insert((r.uuid(), r.opts().encode()))
                })
                .map(|r| Entry::Record(r.clone()));

            let mut count = 0;
            for r in including {
                writer.feed(r).await?;
                count += 1;
            }

            writer.flush().await?;

            // If no records were appended, do not include this stamp
            if count > 0 {
                let manifest = writer.encoder_mut().stamp_manifest();
                writer.send(Entry::Record(manifest.record)).await?;

                debug!("Packed {count} records from {:?}", member.path());
            }

            // Note the member's "path"
            cleanup.push(member.path());
        }

        // Complete the archive
        writer.feed(Entry::Zeros).await?;
        writer.close().await?;

        std::fs::rename(&dest, &completed_dest)?;
        debug!("Packed {dest:?} -> {completed_dest:?}");
        Ok(())
    }

    /// Returns the output path of the store.tar
    #[inline]
    pub fn store_tar_path(&self) -> PathBuf {
        self.output_dir.join(format!("{}.tar", self.archive))
    }

    /// Returns members in the store archive
    #[inline]
    pub fn members(&self) -> impl Iterator<Item = &ArchiveMember> {
        self.members.iter()
    }
}

impl Default for Store {
    fn default() -> Self {
        Self {
            archive: "store",
            packer: Default::default(),
            work_dir: std::env::temp_dir(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{IRecord, ToNamespace};

    use super::*;

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_store_build_archive() {
        let store = Store::default();
        let mut worker = store.worker();

        {
            let ns = "test_ns_1".to_namespace();
            assert!(worker.push(ns.author("record_1", |mut b| {
                b.start_map().push("value", "hello world");
                b
            })));

            assert!(worker.push(ns.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world");
                b
            })));
        }

        {
            let ns = "test_ns_2".to_namespace();
            assert!(worker.push(ns.author("record_1", |mut b| {
                b.start_map().push("value", "hello world 2");
                b
            })));

            assert!(worker.push(ns.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world 2");
                b
            })));
        }

        {
            let ns = "test_ns_1".to_namespace();
            assert!(worker.push(ns.author("record_1", |mut b| {
                b.start_map().push("value", "hello world 2");
                b
            })));

            assert!(worker.push(ns.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world");
                b
            })));
        }

        worker.sync().unwrap().await.unwrap();
        let store_archive = store.archive();
        eprintln!("{:#?}", store_archive);

        store_archive.pack().await.unwrap();

        let store = store_archive.store_tar_path();
        let store_archive = StoreArchive::unpack(&store).await.unwrap();
        eprintln!("{store_archive:#?}");
        for member in store_archive.members() {
            let records = member.get_records().unwrap();
            for r in records {
                assert!(r.is_valid());
                eprintln!("{} is valid!", r.uuid());
            }
        }

        let store = Store::default();
        {
            let mut worker = store.worker();
            let ns = "test_ns_12".to_namespace();
            assert!(worker.push(ns.author("record_1", |mut b| {
                b.start_map().push("value", "hello world");
                b
            })));

            assert!(worker.push(ns.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world");
                b
            })));
            worker.sync().unwrap().await.unwrap();
        }

        {
            let mut worker = store.worker();
            let ns = "test_ns_22".to_namespace();
            assert!(worker.push(ns.author("record_1", |mut b| {
                b.start_map().push("value", "hello world 2");
                b
            })));

            assert!(worker.push(ns.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world 2");
                b
            })));
            worker.sync().unwrap().await.unwrap();
        }

        {
            let mut worker = store.worker();
            let ns = "test_ns_12".to_namespace();
            assert!(worker.push(ns.author("record_1", |mut b| {
                b.start_map().push("value", "hello world 2");
                b
            })));

            assert!(worker.push(ns.author("record_2", |mut b| {
                b.start_map().push("value", "goodbye world");
                b
            })));
            worker.sync().unwrap().await.unwrap();
        }

        let store_archive = store.archive();
        eprintln!("{:#?}", store_archive);
        let _ = store_archive.pack().await.unwrap();
    }
}

use crate::{
    IRecord, Record, Storage, Store, VecIndex,
    store::{ArchiveMember, StoreArchive},
};
use parking_lot::RwLock;
use std::{
    ops::Deref,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tracing::debug;

use super::Frontend;

type RecordSnapshot = Arc<VecIndex<Record>>;

/// Type-alias over a snapshot cell
///
/// The Pin<Box<..>> allows dereferencing the underlying snapshot, to allow for regular borrow-semantics,
/// and also to allow for atomically-replacing the snapshot when needed.
type SnapshotCell = Arc<RwLock<Pin<Box<RecordSnapshot>>>>;

/// Wrapper over State to allow cloning/sharing
pub struct SharedState {
    pub(crate) state: Arc<State>,
    snapshot: SnapshotCell,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            state: Default::default(),
            snapshot: Arc::new(RwLock::new(Box::pin(RecordSnapshot::default()))),
        }
    }
}

/// Common frontend state, can be used by one or more frontend implementations.
///
/// On it's own State is mostly thread-safe, however sharing between threads should use the "SharedState" wrapper
///
/// - Manages record lifecycle (staging, promoting, deletion, indexing, saving, loading, etc)
/// - Manages record archive members produced by store/worker system
/// - Manages record "indexes" and "snapshots"
///     - Each archive member produces an index
///     - A snapshot is a merged view of all indexes
pub struct State {
    /// Store only holds a reference to a queue, and a work_dir
    ///
    /// Most of the "store" logic happens in Worker and Record respectively
    store: Store,
    /// Map of stored snapshots
    snapshots: dashmap::DashMap<Snapshot, RecordSnapshot>,
    /// Name of the state frontend
    frontend: &'static str,
    /// Instance ID (WIP)
    instance: usize,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Snapshot {
    /// Default from work_dir
    Default,
    /// Staged records
    Staging,
    /// Soft deleted records
    SoftDeleted,
    /// Imported from a pack
    Imported(PathBuf),
}

impl State {
    /// Sets the "identity" parameters for this state, which are the main frontend name and instance no
    #[inline]
    pub fn set_identity<F: Frontend>(&mut self, instance: usize) {
        self.frontend = F::NAME;
        self.instance = instance;
        self.store.set_archive(F::NAME);
    }

    /// Reduces all records from all archive members found in Store into a single snapshot
    ///
    /// Archive Member -> Index => Snapshot
    pub fn reduce(&self) -> std::io::Result<()> {
        // Store::archive() will flush all pending archive members into a StoreArchive
        let store_archive = self.store.archive();

        if store_archive.members.is_empty() {
            debug!("No new data to reduce");
            return Ok(());
        }

        let mut indexes = vec![];
        // Process each member to produce a new "index" for each record
        for member in store_archive.members() {
            let path = member.path().to_path_buf();
            debug!("Packing member from {path:?}");
            let records = member.get_records()?;

            if records.is_empty() {
                debug!("skipping {path:?}, no records were found");
                continue;
            }

            let mut index = VecIndex::default();
            for r in records {
                index.index_unchecked(r.clone());
            }
            indexes.push((path.clone(), index));
        }

        let mut next_snapshot = self.update_or_create_snapshot(Snapshot::Default);
        let mut staging_index = self.update_or_create_snapshot(Snapshot::Staging);
        let mut soft_deleted = self.update_or_create_snapshot(Snapshot::SoftDeleted);

        for (idx, (_, i)) in indexes.iter().enumerate() {
            debug!(
                "====== Processing member {idx}, count: {} ======",
                i.storage().len()
            );
            for r in i.storage().iter_records() {
                match next_snapshot.index(r.clone()) {
                    crate::index::IndexResult::Inserted(k) => {
                        debug!(
                            "Inserted {}/{:x}/{:x} -> {k}",
                            r.uuid(),
                            r.ns_chk(),
                            r.index_key(),
                        );
                    }
                    crate::index::IndexResult::Promoted(k, previous) => {
                        debug!("Promoted {} -> {k}", r.uuid());
                        if let Some(previous) = previous {
                            soft_deleted.index_unchecked(previous);
                        }
                    }
                    crate::index::IndexResult::Exists(k, skipped) => {
                        debug!(
                            "Exists {} @ {k} staging: {}",
                            skipped.uuid(),
                            skipped.opts().is_staging()
                        );
                    }
                    crate::index::IndexResult::CannotPromote(staging) => {
                        let uuid = staging.uuid();
                        let key = next_snapshot
                            .reverse_lookup(&staging)
                            .expect("should return a key since cannot promote was returned");
                        let existing = next_snapshot
                            .get(key)
                            .expect("should return since cannot promoted returned");

                        if existing.content() == staging.content() {
                            debug!(
                                "Attempted to insert duplicate content {uuid} @ {key}, skipping"
                            );
                        } else {
                            match staging_index.index_unchecked(staging) {
                                crate::index::IndexResult::Inserted(k) => {
                                    debug!("Staging {uuid} @ {k}")
                                }
                                crate::index::IndexResult::Exists(k, skipping) => {
                                    debug!(
                                        "Staging already occupied @ {k}, skipping {}",
                                        skipping.uuid()
                                    )
                                }
                                _ => {}
                            }
                        }
                    }
                    crate::index::IndexResult::CannotInsertDeletedRecord(deleting) => {
                        debug!(
                            soft_delete = deleting.opts().is_soft_deleted(),
                            "Skipping Deleted Record {} in {}",
                            deleting.uuid(),
                            idx
                        );
                        if deleting.opts().is_soft_deleted() {
                            soft_deleted.index_unchecked(deleting);
                        }
                    }
                    crate::index::IndexResult::Deleted(deleted) => {
                        debug!("Deleted record @ {deleted} in {idx}");
                    }
                }
            }
        }

        // Only store the next snapshot if it isn't empty
        if !next_snapshot.storage().is_empty() {
            self.snapshots
                .insert(Snapshot::Default, Arc::new(next_snapshot));
        }

        if !staging_index.storage().is_empty() {
            self.snapshots
                .insert(Snapshot::Staging, Arc::new(staging_index));
        }

        if !soft_deleted.storage().is_empty() {
            self.snapshots
                .insert(Snapshot::SoftDeleted, Arc::new(soft_deleted));
        }
        Ok(())
    }

    /// Returns a stored snapshot
    #[inline]
    pub fn snapshot(&self, snapshot: Snapshot) -> RecordSnapshot {
        self.snapshots.entry(snapshot).or_default().clone()
    }

    /// Returns a clone of an existing snapshot for updates or creates a new snapshot
    #[inline]
    fn update_or_create_snapshot(&self, snapshot: Snapshot) -> VecIndex<Record> {
        self.snapshots
            .get(&snapshot)
            .map(|v| v.deref().deref().clone())
            .unwrap_or_default()
    }

    /// Saves state to output_directory
    #[inline]
    pub async fn save(&self, output_dir: impl Into<PathBuf>) -> std::io::Result<()> {
        let archive = StoreArchive {
            // members: archived,
            members: vec![
                ArchiveMember::Index {
                    path: PathBuf::from("<softdelete>"),
                    index: self.snapshot(Snapshot::SoftDeleted).deref().clone(),
                },
                ArchiveMember::Index {
                    path: PathBuf::from("<default>"),
                    index: self.snapshot(Snapshot::Default).deref().clone(),
                },
                ArchiveMember::Index {
                    path: PathBuf::from("<staging>"),
                    index: self.snapshot(Snapshot::Staging).deref().clone(),
                },
            ],
            output_dir: output_dir.into(),
            archive: self.frontend,
        };

        archive.pack().await?;
        debug!(
            frontend = self.frontend,
            instance = self.instance,
            "saved store archive to dir {:?}",
            archive.output_dir
        );
        Ok(())
    }

    /// Imports a store_archive into state
    #[inline]
    pub async fn import(&self, store_tar: impl AsRef<Path>) -> std::io::Result<()> {
        let restored = StoreArchive::unpack(store_tar.as_ref()).await?;
        let mut imported = VecIndex::default();
        for member in restored.members() {
            let records = member.get_records()?;

            for r in records {
                imported.index(r.clone());
            }
        }

        imported.set_name(
            store_tar
                .as_ref()
                .file_stem()
                .and_then(|f| f.to_str())
                .unwrap_or_default(),
        );

        self.snapshots.insert(
            Snapshot::Imported(store_tar.as_ref().to_path_buf()),
            imported.into(),
        );
        debug!(
            frontend = self.frontend,
            instance = self.instance,
            "imported {:?} to state",
            store_tar.as_ref()
        );
        Ok(())
    }

    /// Loads state from a {WORK_DIR}/store.tar file
    #[inline]
    pub async fn load<F: Frontend>(work_dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let work_dir = work_dir.into();
        let frontend_tar = work_dir.join(F::archive_name());
        let restored = StoreArchive::unpack(&frontend_tar).await?;

        let mut state = Self::default();
        state.set_identity::<F>(F::next_instance_id());
        state.store = Store::work_dir(&work_dir);
        let pusher = state.store.packer.pusher().expect("should not be closed");

        for mem in restored.members {
            pusher.ensure_push(mem);
        }

        state.reduce()?;

        debug!(
            frontend = F::NAME,
            instance = state.instance,
            "loaded state from {:?}",
            frontend_tar
        );
        Ok(state)
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            frontend: "store",
            instance: 0,
            store: Default::default(),
            snapshots: dashmap::DashMap::new(),
        }
    }
}

impl SharedState {
    /// Returns a reference to State
    #[inline]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Returns a reference to the store
    #[inline]
    pub fn store(&self) -> &Store {
        &self.state.store
    }

    /// Returns the frontend id
    #[inline]
    pub fn frontend_id(&self) -> (&'static str, usize) {
        (self.state.frontend, self.state.instance)
    }

    /// Returns a reference to the latest snapshot
    ///
    /// Note: update_snapshot() must be called in order to update this value
    #[inline]
    pub fn snapshot(&self) -> &VecIndex<Record> {
        let resource = self.snapshot.read();
        let resource = resource.as_ref();
        unsafe {
            let inner = Pin::into_inner_unchecked(resource);
            let cast = cast_ref(inner);
            let cast = cast.as_ref();
            cast.expect("should never be a null pointer")
        }
    }

    /// Borrows a mutable reference to the inner runtime
    #[inline]
    pub fn update_snapshot(&self) {
        let mut resource = self.snapshot.write();
        let mut resource = resource.as_mut();
        *resource = self.state.snapshot(Snapshot::Default);
    }
}

/// Casts a mutable reference to a raw mutable pointer
fn cast_ref<T: ?Sized>(r: &T) -> *const T {
    r
}

impl Clone for SharedState {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            snapshot: self.snapshot.clone(),
        }
    }
}

impl From<State> for SharedState {
    fn from(value: State) -> Self {
        Self {
            state: value.into(),
            snapshot: Arc::new(RwLock::new(Box::pin(RecordSnapshot::default()))),
        }
    }
}

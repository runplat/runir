use parking_lot::RwLock;
use tracing::debug;

use crate::{
    store::{ArchiveMember, StoreArchive}, Record, Storage, Store, VecIndex
};
use std::{
    ops::Deref,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use super::Frontend;

type RecordSnapshot = Arc<VecIndex<Record>>;

/// Type-alias over a snapshot cell
/// 
/// The Pin<Box<..>> allows dereferencing the underlying snapshot, to allow for regular borrow-semantics,
/// and also to allow for atomically-replacing the snapshot when needed.
type SnapshotCell = Arc<RwLock<Pin<Box<RecordSnapshot>>>>;

/// Wrapper over State to allow cloning
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

/// Common state used w/ all frontends
pub struct State {
    /// Name of the state frontend
    frontend: &'static str,
    /// Instance ID (WIP)
    instance: usize,
    /// Store only holds a reference to a queue, and a work_dir
    ///
    /// Most of the "store" logic happens in Worker and Record respectively
    store: Store,
    /// Map of indexes
    indexes: dashmap::DashMap<PathBuf, VecIndex<Record>>,
    /// Active archive members
    members: dashmap::DashMap<PathBuf, ArchiveMember>,
    /// Map of stored snapshots
    snapshots: dashmap::DashMap<Snapshot, RecordSnapshot>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Snapshot {
    /// Default from work_dir
    Default,
    /// Staged records
    Staging,
    /// Imported from a pack
    Imported(PathBuf),
}

impl State {
    /// Set the frontend name for this state
    #[inline]
    pub fn set_frontend<F: Frontend>(&mut self, instance: usize) {
        self.frontend = F::NAME;
        self.instance = instance;
        self.store.set_archive(F::NAME);
    }

    /// Asynchronously flushes all pending archive-members in state
    #[inline]
    pub fn flush(&self) -> std::io::Result<()> {
        let store_archive = self.store.archive();

        for member in store_archive.members() {
            let path = member.path().to_path_buf();
            debug!("packing member from {path:?}");
            let records = member.get_records()?;

            if records.is_empty() {
                debug!("skipping {path:?}, no records were found");
                continue;
            }

            let mut index = self.indexes.entry(path.clone()).or_default();
            for r in records {
                index.index(r.clone());
            }

            // Ensure empty indexes are never stored
            self.members.insert(path, member.clone());
        }

        let mut snapshot = VecIndex::default();

        for i in self.indexes.iter() {
            for r in i.storage().iter_records() {
                snapshot.index(r.clone());
            }
        }

        // TODO: Need to add this if we add a .gc() function to State
        // // In the case members have been cleaned up, ensure we don't lose data
        // for i in self.snapshots.iter() {
        //     for r in i.storage().iter_records() {
        //         snapshot.index(r.clone());
        //     }
        // }

        // Only store the new snapshot if it isn't empty
        if !snapshot.storage().is_empty() {
            self.snapshots.insert(Snapshot::Default, Arc::new(snapshot));
        }
        Ok(())
    }

    /// Refreshes the indes of a target index from the data stored by an archive_member
    ///
    /// Returns true if data was found from the archive_member
    #[inline]
    pub fn refresh_index(&self, target: &mut VecIndex<Record>) {
        for member_index in self.indexes.iter() {
            debug!("Updating index from member {:?}", member_index.key());
            target.refresh(&member_index);
        }
    }

    /// Returns a stored snapshot
    #[inline]
    pub fn snapshot(&self, snapshot: Snapshot) -> RecordSnapshot {
        self.snapshots.entry(snapshot).or_default().clone()
    }

    /// Saves state to output_directory
    #[inline]
    pub async fn save(&self, output_dir: impl Into<PathBuf>) -> std::io::Result<()> {
        let archived = self
            .members
            .iter()
            .map(|kv| kv.deref().clone())
            .collect::<Vec<_>>();

        let archive = StoreArchive {
            archived,
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
        state.set_frontend::<F>(F::next_instance_id());
        state.store = Store::work_dir(&work_dir);

        let mut snapshot = VecIndex::<Record>::default();
        for mem in restored.members() {
            for r in mem.get_records()? {
                snapshot.index(r);
            }
        }
        state
            .snapshots
            .insert(Snapshot::Default, Arc::new(snapshot));
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
            indexes: dashmap::DashMap::new(),
            members: dashmap::DashMap::new(),
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

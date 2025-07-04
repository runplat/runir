//! # bg module
//!
//! The goal of this module is to act as a drop in replacement for some common ::fs api's
//! in order to normalize them for use w/ a store system.
//!
//! It shall provide a placement for,
//!
//! - PathBuf -> bg::Address
//! - ::fs::open(..) -> bg::open(..) -> Result<bg::Handle>;
//! - ::fs::File::create_new(..) -> bg::create_new(..) -> Result<bg::Handle>;
//!

mod error;
use error::Error;

mod handle;
pub use handle::Handle;

mod file;

use std::{fmt::Display, ops::Deref, str::FromStr, sync::OnceLock};
use crate::Opts;
use dashmap::{DashMap, DashSet};
use uuid::Uuid;

/// Opens an address for reading and returns a Handle
#[inline]
pub(super) fn open(address: impl AsRef<Address>) -> Result<Handle> {
    if let Some(open_fn) = interner().open.get(address.as_ref().scheme) {
        open_fn(address.as_ref())
    } else {
        Err(anyhow::anyhow!("Scheme is unregistered, {}", address.as_ref().scheme).into())
    }
}

/// Opens an address for reading and writing and returns a Handle
#[inline]
pub(super) fn open_rw(address: impl AsRef<Address>) -> Result<Handle> {
    if let Some(rw_open) = interner().rw_open.get(address.as_ref().scheme) {
        rw_open(address.as_ref())
    } else {
        Err(anyhow::anyhow!("Scheme is unregistered, {}", address.as_ref().scheme).into())
    }
}

/// Creates a new location for an Object at Address
/// 
/// Returns an error if the Object could not be created
#[inline]
pub(super) fn create_new(address: impl AsRef<Address>) -> Result<Handle> {
    if let Some(create_new_fn) = interner().create_new.get(address.as_ref().scheme) {
        create_new_fn(address.as_ref())
    } else {
        Err(anyhow::anyhow!("Scheme is unregistered, {}", address.as_ref().scheme).into())
    }
}

/// Moves an object into a container
#[inline]
pub fn move_obj(address: impl AsRef<Address>, container: Container) -> Result<()> {
    if let Some(move_obj_fn) = interner().move_obj.get(address.as_ref().scheme) {
        move_obj_fn(address.as_ref(), container)
    } else {
        Err(anyhow::anyhow!("Scheme is unregistered, {}", address.as_ref().scheme).into())
    }
}

/// Function that given an address, opens the space associated to that address for reading, and returns a handle to access the object
pub type OpenFn = fn(&Address) -> Result<Handle>;

/// Function that given an address, creates new space for that object, and returns a handle to access the object
pub type CreateNewFn = fn(&Address) -> Result<Handle>;

/// Function that given an address, opens the space associated to that address for readding and writing, and returns a handle to access the object
/// 
/// Note: This does not imply exclusive access to the object
pub type ReadWriteOpenFn = fn(&Address) -> Result<Handle>;

/// Function that given a handle, and a target container
/// 
/// Moves the data from the address to the target container.
/// 
/// Must return an error if the object could not be moved 
pub type MoveObjFn = fn(&Address, Container) -> Result<()>;

pub type Result<T> = std::result::Result<T, Error>;

/// Registers a scheme
#[inline]
pub fn register_scheme<S: Scheme>() {
    let map = interner();
    map.create_new.insert(S::PREFIX, S::create_new);
    map.open.insert(S::PREFIX, S::open);
    map.rw_open.insert(S::PREFIX, S::rw_open);
    map.move_obj.insert(S::PREFIX, S::move_obj);
}

/// Scheme represents a concrete fs-like protocol
pub trait Scheme {
    /// Prefix symbol used to identify the scheme
    const PREFIX: &'static str;

    /// Opens a read-only handle to an Object
    fn open(address: &Address) -> Result<Handle>;

    /// Creates a new location for an Object and returns a Handle
    fn create_new(address: &Address) -> Result<Handle>;

    /// Opens a rw handle to an Object
    fn rw_open(address: &Address) -> Result<Handle>;

    /// Moves an object from an address to a different container
    fn move_obj(address: &Address, to: Container) -> Result<()>;

    /// Create the specified container
    fn create(container: Container) -> Result<()>;
}

/// An address is an identifier that can be used to locate an "Object"
#[derive(Debug, Clone)]
pub struct Address {
    /// Address scheme
    scheme: &'static str,
    /// Container storing the data for the object
    container: Container,
    /// UUID of the object
    object: Object,
}

impl Address {
    /// Returns true if the address is for Container::Staging
    #[inline]
    pub fn is_staging(&self) -> bool {
        matches!(self.container, Container::Staging)
    }

    /// Returns true if the address is for Container::Work
    #[inline]
    pub fn is_work(&self) -> bool {
        matches!(self.container, Container::Work)
    }

    /// Returns true if the address is for Container::Shared
    #[inline]
    pub fn is_shared(&self) -> bool {
        matches!(self.container, Container::Shared)
    }

    /// Returns true if the address is for Object::ArchiveMember
    #[inline]
    pub fn is_archive_member(&self) -> bool {
        matches!(self.object, Object::ArchiveMember { .. })
    }

    /// Returns the name of the inner object
    #[inline]
    pub fn name(&self) -> String {
        self.object.to_string()
    }

    pub fn to_staging(self) -> Self {
        Self { scheme: self.scheme, container: Container::Staging, object: self.object }
    }

    pub fn to_work(self) -> Self {
        Self { scheme: self.scheme, container: Container::Work, object: self.object }
    }

    pub fn to_shared(self) -> Self {
        Self { scheme: self.scheme, container: Container::Shared, object: self.object }
    }
}

/// Enumeration of data container types
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Container {
    /// Staging container stores data temporarily and offers the highest level
    /// of isolation and modification, should be used for staging data and moved to a higher level container
    Staging,
    /// Work container is for data that must be persisted. It has less isolation then Temp, but
    /// as long as a single process is operating on it, it should be fine
    Work,
    /// Shared container is for data that must be shared w/ other processes. As the name suggests,
    /// it has no isolation
    Shared,
    /// An unknown container is a container that has no semantic value to the store. We can make no assumptions
    /// or gurantees safely of an unknown container
    Unknown,
}

/// Enumeration of different resources consumed by the store
#[derive(Debug, Clone)]
pub enum Object {
    /// Object has not been resolved
    Query(String),
    /// Object is an archive entry
    ///
    /// An archive entry is the archived representation of a Record
    ///
    /// Archive entry names are always formatted as,
    ///
    /// {ns_chk:x}_{uuid:simple}_{opts:x}
    ArchiveEntry {
        /// Namespace chk value
        ns_chk: u64,
        /// UUID of the record stored by this entry
        uuid: Uuid,
        /// Opts set on the record at the time of archival
        opts: Opts,
    },
    /// Object is an unpacked archive member
    ///
    /// Archive members are stand-alone encoded archives
    ///
    /// Archive member names are always formatted as,
    ///
    /// {session:x}_{ns_chk:x}_{archive}
    ArchiveMember {
        /// Session is an ns_chk of an Ephemeral namespace assigned
        /// to this archive member
        session: u64,
        /// Namespace chk value of this archive member
        ns_chk: u64,
        /// Archive this member is trying to join
        archive: &'static str,
    },
    /// Object is an archive
    ///
    /// An archive is a packed archive consisting of Archive Members and their manifests
    ///
    /// Archives are always formatted as,
    ///
    /// {archive}.tar
    Archive(&'static str),
    /// Miscellaneous object
    ///
    /// Miscellaneous names are always formatted as a normal file pattern (w/ w/o) an extension.
    ///
    /// The &'static str implies that Misc objects are known ahead of time
    Misc(&'static str),
}

impl Object {
    /// Resolves the object
    ///
    /// Returns None if the Object is in an unknown representation
    #[inline]
    pub fn resolve(self) -> Option<Object> {
        match self {
            Object::Query(q) => {
                if let Some(misc) = misc_symbol(&q) {
                    return Some(Object::Misc(misc));
                }

                if let Some(archive) = archive_symbol(&q) {
                    return Some(Object::Archive(archive));
                }

                let parts = q
                    .split_once('_')
                    .and_then(|(c1, c2c3)| c2c3.split_once('_').map(|(c2, c3)| (c1, c2, c3)));

                if let Some((c1, c2, c3)) = parts {
                    if let Some((archive, (session, ns_chk))) = archive_symbol(c3).zip(
                        u64::from_str_radix(c1, 16)
                            .ok()
                            .zip(u64::from_str_radix(c2, 16).ok()),
                    ) {
                        return Some(Object::ArchiveMember {
                            session,
                            ns_chk,
                            archive,
                        });
                    }

                    // Check if archive member
                    if let Some(uuid) = Uuid::from_str(c2).ok() {
                        if let Some((ns_chk, opts)) = u64::from_str_radix(c1, 16)
                            .ok()
                            .zip(u64::from_str_radix(c3, 16).ok())
                        {
                            return Some(Object::ArchiveEntry {
                                ns_chk,
                                uuid,
                                opts: Opts::decode(opts),
                            });
                        } else {
                            return None;
                        }
                    }
                }
                None
            }
            ok => Some(ok),
        }
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let display = if matches!(self, Object::Query(..)) {
            if let Some(resolved) = self.clone().resolve(){
                resolved
            } else {
                return Ok(());
            }
        } else {
            self.clone()
        };

        match display {
            Object::ArchiveEntry { ns_chk, uuid, opts } => {
                write!(f, "{ns_chk:x}_{}_{}", uuid.as_simple(), opts.encode())
            },
            Object::ArchiveMember { session, ns_chk, archive } => {
                write!(f, "{session:x}_{ns_chk:x}_{archive}")
            },
            Object::Archive(arch) => {
                write!(f, "{arch}")
            },
            Object::Misc(misc) => {
                write!(f, "{misc}")
            },
            _ => Ok(())
        }
    }
}

impl AsRef<Address> for Address {
    fn as_ref(&self) -> &Address {
        self
    }
}

fn archive_symbol(test: &str) -> Option<&'static str> {
    OBJECT_INTERNER
        .get()
        .and_then(|i| i.archive.get(test))
        .map(|i| *i.deref())
}

fn misc_symbol(test: &str) -> Option<&'static str> {
    OBJECT_INTERNER
        .get()
        .and_then(|i| i.misc.get(test))
        .map(|i| *i.deref())
}

fn interner<'a>() -> &'a Interner {
    OBJECT_INTERNER.get_or_init(Interner::default)
}

type ArtifactSymbol = &'static str;
type MiscSymbol = &'static str;
type SchemeSymbol = &'static str;

#[derive(Default)]
struct Interner {
    archive: DashSet<ArtifactSymbol>,
    misc: DashSet<MiscSymbol>,
    create_new: DashMap<SchemeSymbol, CreateNewFn>,
    open: DashMap<SchemeSymbol, OpenFn>,
    rw_open: DashMap<SchemeSymbol, ReadWriteOpenFn>,
    move_obj: DashMap<SchemeSymbol, MoveObjFn>,
}

static OBJECT_INTERNER: OnceLock<Interner> = OnceLock::new();

pub fn intern_archive(interning: &'static str) {
    let map = interner();
    map.archive.insert(interning);
}

pub fn intern_misc(interning: &'static str) {
    let map = interner();
    map.misc.insert(interning);
}

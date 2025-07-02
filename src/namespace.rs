use ahash::RandomState;
use bytes::Bytes;
use serde::Serialize;

use crate::Opts;
use crate::RawRecordable;
use crate::Record;
use std::hash::Hash;

/// Convenience function to explicitly convert to a namespace
/// Enables fluent api for configuring the namespace
pub trait ToNamespace: Into<Namespace> {
    /// Converts reference to a namespace
    #[inline]
    fn to_namespace(self) -> Namespace {
        self.into()
    }
}

impl<T: Into<Namespace>> ToNamespace for T {}

/// Namespace provides a hasher for the record
///
/// If created via str, the namespace will be deterministic, and records saved via
/// this namespace may be archived/restored with deterministic symbols
///
/// Otherwise, the namespace is treated as ephemeral and will only be valid during the lifetime of
/// the process
#[derive(Clone)]
pub struct Namespace {
    k1: u64,
    k2: u64,
    k3: u64,
    k4: u64,
    /// Default record options
    opts: Opts,
}

impl Hash for Namespace {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.chk().hash(state);
    }
}

impl PartialEq for Namespace {
    fn eq(&self, other: &Self) -> bool {
        self.chk() == other.chk()
    }
}

impl Namespace {
    /// Derives a new namespace from a namespace label
    ///
    /// Note: This function re-uses ahash in order to have persistent keys,
    /// however these hashes are not intended to be DOS-resistant. That layer
    /// of hashing is handled in the Index code which should be servicing the majority
    /// of hash-based lookups
    #[inline]
    pub fn new(namespace: &str) -> Namespace {
        let init_hash = ahash::RandomState::with_seeds(1, 0, 0, 0);
        let k1 = init_hash.hash_one(namespace);

        let init_hash = ahash::RandomState::with_seeds(0, 1, 0, 0);
        let k2 = init_hash.hash_one(namespace);

        let init_hash = ahash::RandomState::with_seeds(0, 0, 1, 0);
        let k3 = init_hash.hash_one(namespace);

        let init_hash = ahash::RandomState::with_seeds(0, 0, 0, 1);
        let k4 = init_hash.hash_one(namespace);

        Namespace {
            k1,
            k2,
            k3,
            k4,
            opts: Opts::default(),
        }
    }

    /// Returns an ephemeral namespace
    #[inline]
    pub fn ephemeral() -> Namespace {
        Namespace {
            k1: 0,
            k2: 0,
            k3: 0,
            k4: 0,
            opts: Opts::ephemeral(),
        }
    }

    /// Returns a new empty record under this namespace
    #[inline]
    pub fn record(&self, label: &str) -> Record {
        Record::create(label, self.clone()).with_opts(self.opts)
    }

    /// Returns the key value for a label under this namespace
    #[inline]
    pub fn key(&self, label: &str) -> u64 {
        self.hash_state().hash_one(label)
    }

    /// Returns the checksum value for the namespace
    #[inline]
    pub fn chk(&self) -> u64 {
        self.hash_state().hash_one(self.opts)
    }

    /// Returns a uuid representing this namespace
    #[inline]
    pub fn ns_uuid(&self) -> uuid::Uuid {
        uuid::Uuid::from_u64_pair(self.chk(), self.opts.encode())
    }

    /// Returns an encoding for this namespace
    #[inline]
    pub fn encode(&self) -> [uuid::Uuid; 3] {
        [
            uuid::Uuid::from_u64_pair(self.k1, self.k2),
            uuid::Uuid::from_u64_pair(self.k3, self.k4),
            self.ns_uuid()
        ]
    }

    /// Decodes the namespace
    /// 
    /// Returns None if the namespace chk value does not match the encoded chk value
    #[inline]
    pub fn decode(encoded: [uuid::Uuid; 3]) -> Option<Self> {
        let [(k1, k2), (k3, k4), (ns_chk, opts)] = encoded.map(|g| g.as_u64_pair());

        let ns = Self {
            k1,
            k2,
            k3,
            k4,
            opts: Opts::decode(opts),
        };

        Some(ns).filter(|n| n.chk() != ns_chk)
    }

    /// Returns the namespace-scoped options
    #[inline]
    pub fn opts(&self) -> &Opts {
        &self.opts
    }

    /// Enables the indexing record option by default for all records,
    /// created from this namespace.
    #[inline]
    pub fn enable_indexing(&mut self) -> &mut Self {
        self.opts.enable_indexing();
        self
    }

    /// Authors a record under this namespace for an obj
    ///
    /// Note: If the object was unable to be saved, it will return an empty record,
    /// empty records are not considered valid, therefore the Worker will return false if a record
    /// was saved from a worker
    ///
    /// Reminder: Namespace maintains no state, this purely authors a record
    #[inline]
    pub fn store<'a, T: Serialize + 'a>(
        &self,
        label: &str,
        recordable: impl Into<RawRecordable<'a, T>>,
    ) -> Record {
        let recordable = recordable.into();
        let mut ser = flexbuffers::FlexbufferSerializer::new();
        let record = self.record(label);
        if let Ok(()) = recordable.serialize(&mut ser) {
            let mut record = record
                .with_opts(self.opts | recordable.opts)
                .commit(Bytes::from(ser.take_buffer()));

            record.opts_mut().set_serialized_object(true);
            record
        } else {
            record
        }
    }

    /// Authors a flexbuffer root that will become the committed value of the record
    #[inline]
    pub fn author(
        &self,
        label: &str,
        author: impl Fn(flexbuffers::Builder) -> flexbuffers::Builder,
    ) -> Record {
        let mut record = self.record(label).commit(Bytes::from(
            author(flexbuffers::Builder::default()).take_buffer(),
        ));

        record.opts_mut().set_serialized_object(true);
        record
    }

    /// Materialized the hash_state for this namespace
    fn hash_state(&self) -> RandomState {
        if self.k1 == 0 && self.k2 == 0 && self.k3 == 0 && self.k4 == 0 {
            RandomState::new()
        } else {
            RandomState::with_seeds(self.k1, self.k2, self.k3, self.k4)
        }
    }
}

impl From<()> for Namespace {
    fn from(_: ()) -> Self {
        Namespace::new("")
    }
}

impl From<&str> for Namespace {
    fn from(value: &str) -> Self {
        Namespace::new(value)
    }
}

impl std::fmt::Debug for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Namespace")
            .field("ns_uuid", &self.ns_uuid())
            .finish()
    }
}

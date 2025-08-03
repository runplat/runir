use ahash::RandomState;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;
use serde::Serialize;

use crate::ComputedSymbol;
use crate::IRecord;
use crate::Opts;
use crate::RawRecordable;
use crate::Record;
use crate::symbol::Symbol;
use std::hash::Hash;
use std::sync::OnceLock;

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
pub struct Namespace {
    k1: u64,
    k2: u64,
    k3: u64,
    k4: u64,
    /// Default record options
    opts: Opts,
    random_state: OnceLock<RandomState>,
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
    pub fn new(namespace: impl Symbol) -> Namespace {
        let namespace = namespace.symbol();

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
            random_state: OnceLock::new(),
        }
    }

    /// Const namespace creator
    #[inline]
    pub const fn const_new(keys: [u64; 4], opts: Opts) -> Self {
        Namespace {
            k1: keys[0],
            k2: keys[1],
            k3: keys[2],
            k4: keys[3],
            opts,
            random_state: OnceLock::new(),
        }
    }

    /// Returns an ephemeral namespace
    #[inline]
    pub fn ephemeral() -> Namespace {
        let ns = Namespace {
            k1: 0,
            k2: 0,
            k3: 0,
            k4: 0,
            opts: Opts::ephemeral(),
            random_state: OnceLock::new(),
        };
        // Lock-in a hash state early so that Namespace can be cloned
        ns.hash_state();
        ns
    }

    /// Returns a "linked" namespace
    /// 
    /// Note: The link is non-directional, for example
    /// 
    /// ```rs norun
    /// assert_eq!(
    ///     Namespace::from("parent").link("child").chk(), 
    ///     Namespace::from("child").link("parent").chk()
    /// )
    /// ```
    #[inline]
    pub fn link(&self, to: impl Symbol) -> Self {
        let to = Namespace::new(to);

        Self::const_new(
            [
                self.k1 ^ to.k1,
                self.k2 ^ to.k2,
                self.k3 ^ to.k3,
                self.k4 ^ to.k4,
            ],
            self.opts,
        )
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

    /// Returns true if this namespace is canonical
    ///
    /// A canonical namespace does not propagate any options to a record
    ///
    /// Most namespace constructors produce canonical namespaces by default
    #[inline]
    pub fn is_canonical(&self) -> bool {
        self.opts.encode() == 0
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
            self.ns_uuid(),
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
            random_state: OnceLock::new(),
        };

        Some(ns).filter(|n| n.chk() == ns_chk)
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

            record
                .opts_mut()
                .expect("should always be able to mutate options from a full record")
                .set_object_storage(true);
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

        record
            .opts_mut()
            .expect("should always be able to mutate options from a full record")
            .set_object_storage(true);
        record
    }

    /// Authors a flexbuffer root that will become the committed value of the record
    #[inline]
    pub fn commit(&self, label: &str, commit: impl AsRef<[u8]> + Send + 'static) -> Record {
        let record = self.record(label).commit(Bytes::from_owner(commit));
        record
    }

    /// Materialized the hash_state for this namespace
    fn hash_state(&self) -> &RandomState {
        if self.k1 == 0 && self.k2 == 0 && self.k3 == 0 && self.k4 == 0 {
            self.random_state.get_or_init(RandomState::new)
        } else {
            self.random_state
                .get_or_init(|| RandomState::with_seeds(self.k1, self.k2, self.k3, self.k4))
        }
    }

    /// Returns an encoded string of the namespace
    #[inline]
    pub fn encoded_string(&self) -> String {
        let mut buf = BytesMut::new();
        buf.put_u64(self.k1);
        buf.put_u64(self.k2);
        buf.put_u64(self.k3);
        buf.put_u64(self.k4);

        let keys = &buf.as_ref()[..32];
        format!("{}_{}", hex::encode(keys), self.ns_uuid().simple())
    }
}

impl Clone for Namespace {
    fn clone(&self) -> Self {
        Self {
            k1: self.k1.clone(),
            k2: self.k2.clone(),
            k3: self.k3.clone(),
            k4: self.k4.clone(),
            opts: self.opts.clone(),
            random_state: self.random_state.clone(),
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

impl From<ComputedSymbol> for Namespace {
    fn from(value: ComputedSymbol) -> Self {
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

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_namespace_clone() {
        let ns = "test".to_namespace();
        let ns2 = "test".to_namespace();
        assert_eq!(ns.chk(), ns2.chk());
        assert_eq!(ns.chk(), ns.clone().chk());
        assert_eq!(ns2.chk(), ns2.clone().chk());
        assert_eq!(ns.clone().chk(), ns2.clone().chk());
    }

    #[test]
    fn test_namespace_ephemeral_clone() {
        let ns = Namespace::ephemeral();
        let ns2 = ns.clone();
        assert_eq!(ns.chk(), ns2.chk());
        assert_eq!(ns.chk(), ns.clone().chk());
        assert_eq!(ns2.chk(), ns2.clone().chk());
        assert_eq!(ns.clone().chk(), ns2.clone().chk());
    }

    #[test]
    fn test_namespace_ephemeral_encode() {
        let ns = Namespace::ephemeral();

        let ns_encoded = ns.encode();
        assert!(ns_encoded[0].is_nil());
        assert!(ns_encoded[1].is_nil());

        assert!(Namespace::decode(ns_encoded).is_none())
    }

    #[test]
    fn test_encoded_string() {
        let ns = Namespace::from("test");
        assert_eq!(
            "8c19ec392f10fbb9edd2535ab21f480a06488f769c70261120337e6a374207ef_07fdafedb529a03e0000000000000000",
            ns.encoded_string()
        );
        assert!(ns.is_canonical())
    }

    #[test]
    fn test_link() {
        let parent = Namespace::from("parent");
        let child = parent.link("child");

        assert_eq!(Namespace::from("child").link("parent").chk(), child.chk())
    }
}

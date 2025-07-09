use std::ops::BitOr;

pub const EMPTY_OPTS: &Opts = &Opts::empty();

/// Record opts stored as a u64
#[derive(Hash, Debug, Default, PartialEq, PartialOrd, Eq, Ord, Clone, Copy)]
pub struct Opts {
    runtime: Runtime,
    store: Store,
    spec: Spec,
    merge_policy: MergePolicy,
    branch: Branch,
    reserved: [u8; 3],
}

impl Opts {
    /// Returns empty opts
    #[inline]
    pub const fn empty() -> Self {
        Self {
            runtime: Runtime::empty(),
            store: Store::empty(),
            spec: Spec::empty(),
            merge_policy: MergePolicy::empty(),
            branch: Branch::Head,
            reserved: [0; 3],
        }
    }

    /// Returns default Opts for an ephemeral namespace
    #[inline]
    pub fn ephemeral() -> Self {
        Self {
            runtime: Runtime::NoArchive,
            store: Store::empty(),
            spec: Spec::empty(),
            merge_policy: MergePolicy::empty(),
            branch: Branch::Head,
            reserved: [0; 3],
        }
    }

    /// Returns true if runtime indexing is enabled
    #[inline]
    pub fn is_indexable(&self) -> bool {
        self.runtime.contains(Runtime::Indexing)
    }

    /// Returns true if runtime archiving is allowed
    #[inline]
    pub fn is_archivable(&self) -> bool {
        !self.runtime.contains(Runtime::NoArchive)
    }

    /// Returns true if stored data is a serialized object
    #[inline]
    pub fn is_object(&self) -> bool {
        self.store.contains(Store::Object)
    }

    /// Returns true if stored data is a manifest
    #[inline]
    pub fn is_manifest(&self) -> bool {
        self.spec.contains(Spec::Manifest)
    }

    /// Returns true if stored data is idempotent
    ///
    /// By default, all data is treated as idempotent under a namespace/label,
    /// unless a merge policy option has been configured
    #[inline]
    pub fn is_idempotent(&self) -> bool {
        self.merge_policy.is_empty()
    }

    /// Enables runtime indexing behavior for the record
    #[inline]
    pub fn enable_indexing(&mut self) -> &mut Self {
        self.runtime.set(Runtime::Indexing, true);
        self
    }

    /// Disables runtime indexing behavior for the record
    #[inline]
    pub fn disable_indexing(&mut self) -> &mut Self {
        self.runtime.set(Runtime::Indexing, false);
        self
    }

    /// Enables runtime archiving (default behavior)
    #[inline]
    pub fn enable_archiving(&mut self) -> &mut Self {
        self.runtime.set(Runtime::NoArchive, false);
        self
    }

    /// Disables runtime archiving
    ///
    /// Record will be ignored during a Worker::to_archive
    #[inline]
    pub fn disable_archiving(&mut self) -> &mut Self {
        self.runtime.set(Runtime::NoArchive, true);
        self
    }

    /// Sets the SerialziedObject flag in store opts
    #[inline]
    pub fn set_serialized_object(&mut self, enabled: bool) -> &mut Self {
        self.store.set(Store::Object, enabled);
        self
    }

    /// Sets the manifest spec flag, to indicate that the stored data is an archive manifest
    #[inline]
    pub fn set_manifest_spec(&mut self, enabled: bool) -> &mut Self {
        self.spec.set(Spec::Manifest, enabled);
        self
    }

    /// Sets the worker archive spec flag, to indicate that the stored data is a worker archive
    #[inline]
    pub fn set_worker_archive(&mut self, enabled: bool) -> &mut Self {
        self.spec.set(Spec::WorkerArchive, enabled);
        self
    }

    /// Sets the merge policy opt flag, only a single merge policy is allowed to be set, will remove
    /// any previously set merge policies
    #[inline]
    pub fn set_merge_policy(&mut self, policy: impl Into<MergePolicy>) -> &mut Self {
        self.merge_policy = MergePolicy::empty();
        self.merge_policy.set(policy.into(), true);
        self
    }

    // /// Sets the Compressed flag in store opts
    // #[inline]
    // pub fn set_compressed(&mut self) -> &mut Self {
    //     self.store |= Store::Compressed;
    //     self
    // }

    // /// Sets the Encrypted flag in store opts
    // #[inline]
    // pub fn set_encrypted(&mut self) -> &mut Self {
    //     self.store |= Store::Encrypted;
    //     self
    // }

    /// Encodes opts into a u64
    #[inline]
    pub fn encode(&self) -> u64 {
        u64::from_le_bytes([
            self.runtime.bits(),
            self.store.bits(),
            self.spec.bits(),
            self.merge_policy.bits(),
            self.branch.bits(),
            0,
            0,
            0,
        ])
    }

    /// Decodes opts value into an Opts struct
    #[inline]
    pub fn decode(opts: u64) -> Self {
        let [runtime, store, spec, merge_policy, branch, ..] = opts.to_le_bytes();

        Self {
            runtime: Runtime::from_bits_retain(runtime),
            store: Store::from_bits_retain(store),
            spec: Spec::from_bits_retain(spec),
            merge_policy: MergePolicy::from_bits_retain(merge_policy),
            branch: Branch::from_bits_retain(branch),
            reserved: [0; 3],
        }
    }
}

impl BitOr for Opts {
    type Output = Opts;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self {
            runtime: self.runtime | rhs.runtime,
            store: self.store | rhs.store,
            spec: self.spec | rhs.spec,
            merge_policy: self.merge_policy | rhs.merge_policy,
            branch: self.branch | rhs.branch,
            reserved: [0; 3],
        }
    }
}

bitflags::bitflags! {
    /// Record options that describe how to handle the record at runtime
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Runtime : u8 {
        /// Indicates that the record can be processed by the indexer
        const Indexing = 1;
        /// Indicates that the record should not be archived
        const NoArchive = 1 << 1;
    }
}

bitflags::bitflags! {
    /// Store options that describe how the record is stored
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Store: u8 {
        /// Indicates that data stored for the record is a serialized object
        const Object = 1;
        // /// Indicates that data stored for the record is compressed
        // const Compressed = 1 << 1;
        // /// Indicates that data stored for the record is encrypted
        // const Encrypted = 1 << 2;
    }
}

bitflags::bitflags! {
    /// Spec options note any specification information about the stored data
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Spec: u8 {
        /// Indicates that data stored for the record is an archive manifest
        const Manifest = 1;
        /// Indicates that data stored for the record is a name record
        ///
        /// A name record can be used to map namespaces/records to a friendly name
        const Name = 1 << 1;
        /// Indicates that the data is stored in a readable format instead of a binary format
        const Readable = 1 << 2;
        /// Indicates stored data is a worker archive
        const WorkerArchive = 1 << 3;
    }
}

bitflags::bitflags! {
    /// Merge policies control how conflicts between records with the same identifiers are resolved.
    ///
    /// A merge policy may be set on the namespace or per-record. Only one policy should be active at a time.
    ///
    /// Note: Merge policy is considered part of the record's identity — records with different policies
    /// will have distinct archive filenames and cannot be merged.
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct MergePolicy: u8 {
        /// Merge policy is to include all versions of the record
        ///
        /// When the record is fetched and multiple versions are found, an error will be returned
        /// that will include all versions of the record
        const AllVersions = 1;
        /// Prefer the earliest version of the record
        const Earliest = 1 << 1;
        /// Prefer the latest version of the record
        const Latest = 1 << 2;
        /// Prefer the record with the lower checksum
        const LowestChecksum = 1 << 3;
        /// Prefer the record with the highest checksum
        const HighestChecksum = 1 << 4;
        /// Do-not allow merges once a record is set and return an error
        const Fail = 1 << 5;
        /// Do-not allow merges once a record is set and do not
        /// bubble up an error
        const FailSilent = 1 << 6;
        /// Execute a user-registered merge function
        ///
        /// If this flag is set, and a function is not provided, this will
        /// result in a fatal runtime error
        const UserMergeFunction = 1 << 7;
    }
}


bitflags::bitflags! {
    /// Branches enable ergonomic updates without violating idempotency semantics
    /// at upper-level user-facing APIs. 
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Branch: u8 {
        // Lower 2 bits: public branches
        
        /// Head branch semantically requires more-strict rules concerning mutation
        /// Records represented as the "HEAD" must follow strict data-handling
        /// procedures
        const Head     = 0b000000_00;

        /// Staging branch requiers less-strict rules, and allows for more flexible
        /// mutation within isolation
        const Staging  = 0b000000_01;

        /// Deleted branch marks a record as being "deleted" or no longer in use or mapped
        /// Multiple records may exist under the same namespace/label inside of the deleted branch,
        /// however only 1 of each content digest will be preserved.
        const Deleted  = 0b000000_10;

        /// "Ephemeral" branch signals a scratch branch whose record should not be persisted
        const Ephemeral = u8::MAX;

        // Everything else is reserved
    }
}

#[cfg(test)]
mod test {
    use super::Opts;

    #[test]
    fn test_encode_decode() {
        let mut opts = Opts::default();
        assert!(opts.is_idempotent());

        opts.enable_archiving()
            .enable_indexing()
            .set_serialized_object(true)
            .set_manifest_spec(true)
            .set_merge_policy(crate::policy::merge::all_versions());

        assert!(!opts.is_idempotent());
        let encoded = opts.encode();

        let decoded = Opts::decode(encoded);
        assert_eq!(opts, decoded);
    }
}

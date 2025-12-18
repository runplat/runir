use std::ops::BitOr;

/// Constant for `Opts::empty()`
pub const EMPTY_OPTS: Opts = Opts::empty();

/// Record option flags used throughout the runir system.
///
/// `Opts` encodes a compact set of runtime, storage, specification, and lifecycle
/// flags into a fixed 64-bit structure. It enables safe and expressive mutation behavior,
/// supports flexible downstream interpretation, and ensures long-term compatibility.
///
/// ### Layout
/// Internally, `Opts` encodes:
/// - [`Runtime`] flags — runtime behavior such as indexing and archival eligibility
/// - [`Storage`] flags — how the record’s data should be interpreted (e.g., as a serialized object)
/// - [`Spec`] flags — optional system-specific metadata (e.g., manifest marker)
/// - [`Branch`] — defines the record's semantic lifecycle (e.g., `HEAD`, `STAGING`, `DELETED`)
/// - 4 bytes of reserved space for forward compatibility
///
/// ### Design Principles
/// - **Idempotent-by-default**: Records marked as `HEAD` must follow strict mutation rules
/// - **Non-destructive writes**: Mutations are expressed through new records, not in-place updates
/// - **Semantic branches**: `Opts` tracks mutation state explicitly to guide indexing, flushing, and archival
/// - **Forward-compatible encoding**: `reserved` bytes are preserved and merged safely for future use
///
/// ### Usage Notes
/// - Most flags are bitmask-friendly; use the provided builder methods to set them ergonomically.
/// - The `reserved` field should not be interpreted directly—its layout may change in future versions.
/// - `Opts` values can be safely serialized as a `u64` for archive storage or interprocess transport.
///
/// ### Example
/// ```rs no_run
/// let mut opts = Opts::default();
/// opts.enable_indexing()
///     .enable_archiving()
///     .set_object_storage(true);
///
/// let encoded = opts.encode();
/// let roundtrip = Opts::decode(encoded);
/// assert_eq!(opts, roundtrip);
/// ```
#[derive(Hash, Debug, Default, PartialEq, PartialOrd, Eq, Ord, Clone, Copy)]
pub struct Opts {
    /// Runtime options configure runtime decisions
    runtime: Runtime,
    /// Storage options configure internal storage details of what the record is storing
    storage: Storage,
    /// Spec options configure any system specifications required by record systems
    spec: Spec,
    /// Branch options configure the mutation state of the data owned by the record
    branch: Branch,
    /// Padding and reserved bytes for future use
    /// - These bytes are currently ignored by the runtime
    /// - May be repurposed for experimental flags, encoding versions, or special features
    /// - Preserved during encode/decode and bitwise merging for forward compatibility
    reserved: [u8; 4],
}

pub(crate) mod ser {
    use serde::de::Visitor;

    use crate::Opts;
    #[inline]
    pub fn serialize<S>(opts: &Opts, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ser.serialize_u64(opts.encode())
    }

    /// Deserializes an object from a bytes buffer
    #[inline]
    pub fn deserialize<'de, D>(deser: D) -> Result<Opts, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deser.deserialize_u64(OptsVisitor)
    }

    struct OptsVisitor;

    impl Visitor<'_> for OptsVisitor {
        type Value = Opts;
    
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "Expecting `u64`")
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
        {
            Ok(Opts::decode(v))
        }
    }
}

impl Opts {
    /// Returns empty opts
    #[inline]
    pub const fn empty() -> Self {
        Self {
            runtime: Runtime::empty(),
            storage: Storage::empty(),
            spec: Spec::empty(),
            branch: Branch::Head,
            reserved: [0; 4],
        }
    }

    /// Returns default Opts for an ephemeral namespace
    #[inline]
    pub const fn ephemeral() -> Self {
        Self {
            runtime: Runtime::NoArchive,
            storage: Storage::empty(),
            spec: Spec::empty(),
            branch: Branch::Head,
            reserved: [0; 4],
        }
    }

    /// Returns true if runtime indexing is enabled
    #[inline]
    pub const fn is_indexable(&self) -> bool {
        self.runtime.contains(Runtime::Indexing)
    }

    /// Returns true if runtime archiving is allowed
    #[inline]
    pub const fn is_archivable(&self) -> bool {
        !self.runtime.contains(Runtime::NoArchive)
    }

    /// Returns true if the runtime content-addressable record mode is enabled
    #[inline]
    pub const fn is_content_addressable(&self) -> bool {
        self.runtime.contains(Runtime::ContentAddress)
    }

    /// Returns true if stored data is a serialized object
    #[inline]
    pub const fn is_object(&self) -> bool {
        self.storage.contains(Storage::Object)
    }

    /// Returns true if stored data has multiple roots
    #[inline]
    pub const fn is_multi(&self) -> bool {
        self.storage.contains(Storage::Multi)
    }

    /// Returns true if stored data is a manifest
    #[inline]
    pub const fn is_manifest(&self) -> bool {
        self.spec.contains(Spec::Manifest)
    }

    /// Returns true if the data stored by the record is idempotent
    #[inline]
    pub const fn is_idempotent(&self) -> bool {
        self.branch.is_empty()
    }

    /// Returns true if the stored data is canonical Record Data
    #[inline]
    pub const fn is_canonical_data(&self) -> bool {
        self.spec.is_empty()
    }

    /// Returns true if the data stored by the record has been marked for deletion
    #[inline]
    pub fn is_deleted(&self) -> bool {
        self.branch.contains(Branch::Deleted)
    }

    /// Returns true if the data stored by the record has been marked for staging
    #[inline]
    pub fn is_staging(&self) -> bool {
        self.branch.contains(Branch::Staging)
    }

    /// Returns true if the data stored by the record is "soft" deleted
    #[inline]
    pub fn is_soft_deleted(&self) -> bool {
        self.is_deleted() && !self.is_staging()
    }

    /// Enables runtime indexing behavior for the record.
    ///
    /// This does **not** guarantee the record will appear in an index,
    /// but marks it as eligible for analysis by indexers.
    /// Indexers may still ignore the record based on its namespace,
    /// branch, or other constraints.
    #[inline]
    pub fn enable_indexing(&mut self) -> &mut Self {
        self.runtime.set(Runtime::Indexing, true);
        self
    }

    /// Enables runtime content addressing for the record
    /// 
    /// When content addressing is enabled, the label used for the record is the SHA256 digest of the data being stored
    #[inline]
    pub fn enable_content_addressing(&mut self) -> &mut Self {
        self.runtime.set(Runtime::ContentAddress, true);
        self
    }

    /// Disables indexing for this record (default behavior).
    ///
    /// This flag is a performance hint for indexers. When disabled, indexers will skip reading
    /// or analyzing this record. It does *not* prevent the record from being returned in queries
    /// if it was previously indexed or explicitly included.
    #[inline]
    pub fn disable_indexing(&mut self) -> &mut Self {
        self.runtime.set(Runtime::Indexing, false);
        self
    }

    /// Enables runtime archiving for this record (default behavior).
    ///
    /// When enabled, this record is eligible to be included in archive outputs,
    /// such as snapshots, packing passes, or final store bundles.
    #[inline]
    pub fn enable_archiving(&mut self) -> &mut Self {
        self.runtime.set(Runtime::NoArchive, false);
        self
    }

    /// Disables runtime archiving for this record.
    ///
    /// This flag prevents the record from being included in long-term archive outputs,
    /// even if it is otherwise valid or reachable. Use this to exclude volatile, sensitive,
    /// or transient records from being persisted beyond their working lifecycle.
    ///
    /// **Note:** This flag is treated as a hard constraint. Archive writers must respect it.
    #[inline]
    pub fn disable_archiving(&mut self) -> &mut Self {
        self.runtime.set(Runtime::NoArchive, true);
        self
    }

    /// Sets the "Object" flag in storage options
    #[inline]
    pub fn set_object_storage(&mut self, enabled: bool) -> &mut Self {
        self.storage.set(Storage::Object, enabled);
        self
    }

    /// Sets the "Multi" flag in storage options
    #[inline]
    pub fn set_multi_root_storage(&mut self, enabled: bool) -> &mut Self {
        self.storage.set(Storage::Multi, enabled);
        self
    }

    /// Sets the manifest spec flag, to indicate that the stored data is an archive manifest
    #[inline]
    pub fn set_manifest_spec(&mut self, enabled: bool) -> &mut Self {
        self.spec.set(Spec::Manifest, enabled);
        self
    }

    /// Sets the info spec flag, to indicate that the stored data is an archive manifest
    #[inline]
    pub fn set_info_spec(&mut self, enabled: bool) -> &mut Self {
        self.spec.set(Spec::Info, enabled);
        self
    }

    /// Sets the extension spec flag, to indicate that the stored data is an archive manifest
    #[inline]
    pub fn set_ext_spec(&mut self, enabled: bool) -> &mut Self {
        self.spec.set(Spec::Ext, enabled);
        self
    }

    /// Enables a branch flag
    #[inline]
    pub fn enable_branch(&mut self, branch: Branch) -> &mut Self {
        self.branch.set(branch, true);
        self
    }

    /// Sets the branch state to enable recovery
    ///
    /// This clears the `DELETED` flag and sets the `STAGING` flag, making the record
    /// eligible for further mutation or re-commit. This does not validate prior branch state,
    /// and makes no assumptions about the record's history.
    #[inline]
    pub fn enable_recovery_mode(&mut self) -> &mut Self {
        self.branch.set(Branch::Deleted, false);
        self.branch.set(Branch::Staging, true);
        self
    }

    /// Removes Staging and Deleted from branch options which will
    /// flag the record as being the canonical version
    #[inline]
    pub fn promote(&mut self) -> &mut Self {
        self.branch.remove(Branch::Staging);
        self.branch.remove(Branch::Deleted);
        self
    }

    /// Encodes opts into a u64
    #[inline]
    pub fn encode(&self) -> u64 {
        u64::from_le_bytes([
            self.runtime.bits(),
            self.storage.bits(),
            self.spec.bits(),
            self.branch.bits(),
            self.reserved[0],
            self.reserved[1],
            self.reserved[2],
            self.reserved[3],
        ])
    }

    /// Decodes opts value into an Opts struct
    #[inline]
    pub fn decode(opts: u64) -> Self {
        let [runtime, store, spec, branch, r0, r1, r2, r3] = opts.to_le_bytes();

        Self {
            runtime: Runtime::from_bits_retain(runtime),
            storage: Storage::from_bits_retain(store),
            spec: Spec::from_bits_retain(spec),
            branch: Branch::from_bits_retain(branch),
            reserved: [r0, r1, r2, r3],
        }
    }
}

impl BitOr for Opts {
    type Output = Opts;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self {
            runtime: self.runtime | rhs.runtime,
            storage: self.storage | rhs.storage,
            spec: self.spec | rhs.spec,
            branch: self.branch | rhs.branch,
            reserved: [
                self.reserved[0] | rhs.reserved[0],
                self.reserved[1] | rhs.reserved[1],
                self.reserved[2] | rhs.reserved[2],
                self.reserved[3] | rhs.reserved[3],
            ],
        }
    }
}

bitflags::bitflags! {
    /// Record options that describe how to handle the record at runtime
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Runtime : u8 {
        /// Indicates that the record can be processed by an indexer
        const Indexing = 1;
        /// Indicates that the record should not be archived
        const NoArchive = 1 << 1;
        /// Indicates that the record label is the content digest of the data stored by the record
        const ContentAddress = 1 << 2;
    }
}

bitflags::bitflags! {
    /// Options that describe or configure how a record is stored
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Storage: u8 {
        /// Indicates that the data stored for the record is content
        const Content = 0;
        /// Indicates that data stored for the record is a serialized object
        const Object = 1;
        /// Indicates that data stored for the record is a multi-root record
        const Multi = 2;
    }
}

bitflags::bitflags! {
    /// Spec options note any specification information about the stored data
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Spec: u8 {
        /// Indicates that the stored data does not follow any system specifications
        const None = 0;
        /// Indicates that stored data is an archive manifest
        const Manifest = 1;
        /// Indicates stored data is a record info
        const Info = 2;
        /// Indicates stored data is extension data
        const Ext = 3;
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
        const Head     = 0b00000_000;

        /// Staging branch requiers less-strict rules, and allows for more flexible
        /// mutation within isolation
        const Staging  = 0b00000_001;

        /// Deleted branch marks a record as being "deleted" or no longer in use or mapped
        /// Multiple records may exist under the same namespace/label inside of the deleted branch,
        /// however only 1 of each content digest will be preserved.
        const Deleted  = 0b00000_010;

        /// "Ephemeral" branch signals a scratch branch whose record should not be persisted
        const Ephemeral = u8::MAX;

        // Everything else is reserved
    }
}


bitflags::bitflags! {
    /// Transport describes the internal format of the stored data belonging to the record
    /// 
    /// For example, by default all record data when a record is created is considered "inline",
    /// because the data of the record is available as a pointer.
    /// 
    /// However, when a record is persisted to a medium, the transport could change depending on,
    /// the constraints.
    /// 
    /// For example, for a very large record, locally it can be memory-mapped and attached to a record "inline".
    /// If that record were to be moved to a remote location, moving it "inline" would not be physically
    /// possible, as it would likely take more than a single frame to move the data, so in-flight, the record
    /// would not be viewable "inline". 
    /// 
    /// (Even for "smaller" records this is the case however that will be handled by the "inline" transport for reasonably sized records)
    /// 
    /// If the runtime does not account for this, it can result in process stalls or exceeding storage limits
    /// unexpectedly.
    /// 
    /// To address this, we can use a "block" strategy. With a block strategy, we view the mass of data as several
    /// blocks or chunks which reduce the logical overhead to a much more manageable number. We're then able to
    /// uniformly reserve capacity and resources to dis-assemble and re-assemble very large records from a list of
    /// blocks.
    /// 
    /// ### Notes: 
    /// - How do we configure a uniformly good block strategy?
    /// 
    #[derive(Hash, Default, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct Transport : u8 {
        /// (Default) Use "inline" transport mode
        const Inline = 0;

        /// Use "block" transport mode
        const Block = 1;
    }
}

impl From<Storage> for Opts {
    #[inline]
    fn from(value: Storage) -> Self {
        let mut opts = Opts::empty();
        opts.storage = value;
        opts
    }
}

#[cfg(test)]
mod test {
    use crate::opts::Branch;

    use super::Opts;

    #[test]
    fn test_encode_decode() {
        let mut opts = Opts::default();
        assert!(opts.is_idempotent());

        opts.enable_archiving()
            .enable_indexing()
            .set_object_storage(true)
            .set_manifest_spec(true)
            .enable_branch(Branch::Staging);

        assert!(!opts.is_idempotent());
        let encoded = opts.encode();

        let decoded = Opts::decode(encoded);
        assert_eq!(opts, decoded);
    }
}

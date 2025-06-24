use std::ops::BitOr;

/// Record opts stored as a u64
#[derive(Hash, Debug, Default, PartialEq, PartialOrd, Eq, Ord, Clone, Copy)]
pub struct Opts {
    runtime: Runtime,
    store: Store,
    spec: Spec,
    reserved: [u8; 5],
}

impl Opts {
    /// Returns default Opts for an ephemeral namespace
    #[inline]
    pub fn ephemeral() -> Self {
        Self { runtime: Runtime::NoArchive, store: Store::empty(), spec: Spec::empty(), reserved: [0; 5] }
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

    /// Enables runtime indexing behavior for the record
    #[inline]
    pub fn enable_indexing(&mut self) -> &mut Self {
        self.runtime |= Runtime::Indexing;
        self
    }

    /// Disables runtime indexing behavior for the record
    #[inline]
    pub fn disable_indexing(&mut self) -> &mut Self {
        self.runtime &= !Runtime::Indexing;
        self
    }

    /// Enables runtime archiving (default behavior)
    #[inline]
    pub fn enable_archiving(&mut self) -> &mut Self {
        self.runtime &= !Runtime::NoArchive;
        self
    }

    /// Disables runtime archiving
    ///
    /// Record will be ignored during a Worker::to_archive
    #[inline]
    pub fn disable_archiving(&mut self) -> &mut Self {
        self.runtime |= Runtime::NoArchive;
        self
    }

    /// Sets the SerialziedObject flag in store opts
    #[inline]
    pub fn set_serialized_object(&mut self) -> &mut Self {
        self.store |= Store::Object;
        self
    }

    /// Sets the manifest spec flag, to indicate that the stored data is an archive manifest
    #[inline]
    pub fn set_manifest_spec(&mut self) -> &mut Self {
        self.spec |= Spec::Manifest;
        self
    }

    /// Sets the worker archive spec flag, to indicate that the stored data is a worker archive
    #[inline]
    pub fn set_worker_archive(&mut self) -> &mut Self {
        self.spec |= Spec::WorkerArchive;
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
        u64::from_le_bytes([self.runtime.bits(), self.store.bits(), self.spec.bits(), 0, 0, 0, 0, 0])
    }

    /// Decodes opts value into an Opts struct
    #[inline]
    pub fn decode(opts: u64) -> Self {
        let [runtime, store, spec, ..] = opts.to_le_bytes();

        Self {
            runtime: Runtime::from_bits_retain(runtime),
            store: Store::from_bits_retain(store),
            spec: Spec::from_bits_retain(spec),
            reserved: [0; 5],
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
            reserved: [0; 5],
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

#[cfg(test)]
mod test {
    use super::Opts;

    #[test]
    fn test_encode_decode() {
        let mut opts = Opts::default();

        opts
            .enable_archiving()
            .enable_indexing()
            .set_serialized_object()
            .set_manifest_spec();

        let encoded = opts.encode();

        let decoded = Opts::decode(encoded);
        assert_eq!(opts, decoded);
    }
}

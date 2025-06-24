use std::ops::BitOr;

/// Record opts stored as a u64
#[derive(Hash, Debug, Default, PartialEq, PartialOrd, Eq, Ord, Clone, Copy)]
pub struct Opts {
    runtime: Runtime,
    store: Store,
    reserved: [u8; 6],
}

impl Opts {
    /// Returns default Opts for an ephemeral namespace
    #[inline]
    pub fn ephemeral() -> Self {
        Self { runtime: Runtime::NoArchive, store: Store::empty(), reserved: [0; 6] }
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
        u64::from_le_bytes([self.runtime.bits(), self.store.bits(), 0, 0, 0, 0, 0, 0])
    }

    /// Decodes opts value into an Opts struct
    #[inline]
    pub fn decode(opts: u64) -> Self {
        let [runtime, store, ..] = opts.to_le_bytes();

        Self {
            runtime: Runtime::from_bits_retain(runtime),
            store: Store::from_bits_retain(store),
            reserved: [0; 6],
        }
    }
}

impl BitOr for Opts {
    type Output = Opts;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self {
            runtime: self.runtime | rhs.runtime,
            store: self.store | rhs.store,
            reserved: [0; 6],
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

#[cfg(test)]
mod test {
    use super::Opts;

    #[test]
    fn test_encode_decode() {
        let mut opts = Opts::default();

        opts
            .enable_archiving()
            .enable_indexing()
            .set_serialized_object();

        let encoded = opts.encode();

        let decoded = Opts::decode(encoded);
        assert_eq!(opts, decoded);
    }
}

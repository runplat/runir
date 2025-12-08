use zstd::zstd_safe::WriteBuf;

/// Describes a single wire unit record
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct Manifest<'a> {
    /// Describes the container that stores an object
    pub container: Descriptor<'a>,
    /// Describes the object stored in the container
    pub object: Descriptor<'a>,
}

/// Generic descriptor for a content-addressed blob
#[derive(PartialEq, Eq, Clone, Hash)]
pub struct Descriptor<'a> {
    /// Size of the blob
    pub size: u64,
    /// Digest of the blob
    pub digest: std::borrow::Cow<'a, [u8]>,
}

/// IRecord extension enabling records in wire-unit format to be describe
/// themselves
pub trait Describe {
    /// Returns a description of the container and contents of this type
    fn describe<'desc>(&'desc self) -> Manifest<'desc>;
}

// impl<T: IRecord> Describe for T {}

impl<'a> std::fmt::Debug for Descriptor<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Descriptor")
            .field("size", &self.size)
            .field("digest", &hex::encode(self.digest.as_slice()))
            .finish()
    }
}

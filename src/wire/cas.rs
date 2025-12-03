use crate::IRecord;
use anyhow::anyhow;
use flexbuffers::Blob;

/// Generic descriptor for a content-addressed blob
pub struct Descriptor<'a> {
    /// Size of the blob
    pub size: u64,
    /// Digest of the blob
    pub digest: &'a [u8],
}

/// IRecord extension enabling records in wire-unit format to be describe
/// themselves
pub trait Describe: IRecord {
    /// Returns a descriptor for this record
    ///
    /// Returns an error if the wi
    #[inline]
    fn describe<'desc>(&'desc self) -> crate::Result<Descriptor<'desc>> {
        use crate::prelude::PeekExtensions;

        match self.peek().at(".runir") {
            Some(wire_meta) => {
                // Record has wire unit format enabled
                // We can derive a descriptor
                let fields = wire_meta.at_many(&["size", "sha256"]);
                match fields.as_slice() {
                    [Some(size), Some(sha256), ..] => {
                        let Blob(digest) = sha256.as_blob();

                        Ok(Descriptor {
                            size: size.as_u64(),
                            digest,
                        })
                    }
                    _ => Err(anyhow!("Record wire unit format is incomplete").into()),
                }
            }
            None => Err(anyhow!("Record wire unit format is not enabled").into()),
        }
    }
}

impl<T: IRecord> Describe for T {}

impl<'a> std::fmt::Debug for Descriptor<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Descriptor")
            .field("size", &self.size)
            .field("digest", &hex::encode(self.digest))
            .finish()
    }
}

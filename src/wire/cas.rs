use flexbuffers::Blob;
use anyhow::anyhow;
use crate::IRecord;

/// When a record enables wire-unit format
#[derive(Debug)]
pub struct Descriptor<'a> {
    pub size: u64,
    pub digest: &'a [u8],
}

/// IRecord extension enabling records in wire-unit format to be describe
/// themselves
pub trait Describe : IRecord {
    /// Returns a descriptor for this record
    /// 
    /// Returns an error if the wi
    #[inline]
    fn describe<'desc>(&'desc self) -> crate::Result<Descriptor<'desc>>{
        use crate::prelude::PeekExtensions;

        if matches!(self.peek().at("version").str(), Some("runir")) {
            // Record has wire unit format enabled
            // We can derive a descriptor
            let fields = self.fields(&["size", "sha256"]);
            match fields.as_slice() {
                [Some(size), Some(sha256), ..] => {
                    let Blob(digest) = sha256.as_blob();

                    Ok(Descriptor {
                        size: size.as_u64(),
                        digest,
                    })
                }
                _ => Err(anyhow!("Record wire unit format is incomplete").into())
            }
        } else {
            Err(anyhow!("Record wire unit format is not enabled").into())
        }
    }
}
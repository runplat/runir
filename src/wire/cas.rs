use crate::{Opts, util::ser::BytesField, wire::boot::NS_HEADER_INLINE_BLOCK_SIZE};
use serde::{Deserialize, Serialize};
use sha2::Digest;

/// Describes a single wire unit record
#[derive(Debug, Default, PartialEq, Eq, Clone, Hash, Serialize, Deserialize)]
pub struct FrameList<'a> {
    /// Each frame required to store the record
    #[serde(borrow)]
    pub frames: Vec<Descriptor<'a>>,
}

impl<'a> FrameList<'a> {
    /// Returns the capacity required by this manifest
    #[inline]
    pub fn required_capacity(&self) -> u64 {
        self.frames.iter().map(|f| f.size).sum()
    }

    /// Align the offsets to the true start
    #[inline]
    pub fn align_offset(&mut self, start: u64) {
        for f in self.frames.iter_mut() {
            f.offset += start;
        }
    }
}

/// Generic descriptor for a content-addressed blob
#[derive(Ord, PartialOrd, PartialEq, Eq, Clone, Hash, Serialize, Deserialize)]
pub struct Descriptor<'a> {
    /// Storage flags
    #[serde(with = "crate::opts::ser")]
    pub opts: Opts,
    /// Size of the blob
    pub size: u64,
    /// Digest of the blob
    #[serde(borrow)]
    pub digest: BytesField<'a>,
    /// Label to identify the descriptor
    #[serde(borrow)]
    pub label: std::borrow::Cow<'a, str>,
    /// Byte offset
    pub offset: u64,
}

impl<'a> Descriptor<'a> {
    /// Creates a descriptor from a blob
    #[inline]
    pub fn create(opts: Opts, blob: impl AsRef<[u8]>, label: &'a str) -> Self {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(blob.as_ref());
        Descriptor {
            opts: opts,
            size: blob.as_ref().len() as u64,
            digest: BytesField::Owned(digest.to_vec()),
            label: label.into(),
            offset: 0,
        }
    }

    /// Returns a new descriptor w/ offset set
    #[inline]
    pub fn with_offset(&self, offset: u64) -> Descriptor<'a> {
        let mut with_offset = self.clone();
        with_offset.offset = offset;
        with_offset
    }

    /// Returns true if this descriptor is stored inline
    #[inline]
    pub fn is_inline(&self) -> bool {
        (self.offset + self.size) < NS_HEADER_INLINE_BLOCK_SIZE as u64
    }

    /// Returns true if digest matches
    #[inline]
    pub fn is_digest_match(&self, other: &Self) -> bool {
        self.digest.as_ref() == other.digest.as_ref()
    }
}

/// Trait for cas objects to describe themselves as a manifest
pub trait Describe<'peek> {
    /// Returns a description of the container and contents of this type
    fn describe(&'peek self) -> FrameList<'peek>;
}

/// Trait for a cas store to fetch bytes for a descriptor stored by this store
pub trait Fetch<'peek> {
    /// Fetches the bytes the correspond to a descriptor owned by this store
    fn fetch(&'peek self, desc: &Descriptor<'_>) -> Option<&'peek [u8]>;
}

impl<'a> std::fmt::Debug for Descriptor<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Descriptor")
            .field("size", &self.size)
            .field("digest", &hex::encode(&self.digest))
            .field("offset", &self.offset)
            .field("label", &self.label)
            .field("storage", &self.opts)
            .finish()
    }
}

impl<'peek> Fetch<'peek> for crate::Data {
    fn fetch(&'peek self, desc: &Descriptor<'_>) -> Option<&'peek [u8]> {
        let offset = Some(desc.offset)
            .filter(|o| {
                let o = *o as usize;
                if (o + desc.size as usize) < self.len() {
                    let digest = sha2::Sha256::digest(&self[o..o + desc.size as usize]);
                    digest.as_slice() == desc.digest.as_ref()
                } else {
                    false
                }
            })
            .unwrap_or_else(|| {
                // TODO: This is probably overkill
                let (offset, _) = self
                    .find_cas_view_offset(desc.size as usize, &desc.digest)
                    .unwrap_or_default();
                offset
            }) as usize;
        self.get(offset..offset + desc.size as usize)
    }
}

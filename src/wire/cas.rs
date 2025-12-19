use crate::{Namespace, Opts, wire::boot::NS_HEADER_INLINE_BLOCK_SIZE};
use serde::{Deserialize, Serialize};
use sha2::Digest;

/// Describes a single wire unit record
#[derive(Debug, Default, PartialEq, Eq, Clone, Hash, Serialize, Deserialize)]
pub struct FrameList {
    /// Each frame required to store the record
    pub frames: Vec<Descriptor>,
}

impl FrameList {
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
pub struct Descriptor {
    /// Storage flags
    #[serde(with = "crate::opts::ser", rename = "opt")]
    pub opts: Opts,
    /// Size of the blob
    #[serde(rename = "s")]
    pub size: u64,
    /// Digest checksum
    #[serde(rename = "d")]
    pub dchk: u64,
    /// Label to identify the descriptor
    #[serde(rename = "l")]
    pub label: u64,
    /// Byte offset
    #[serde(rename = "o")]
    pub offset: u64,
}

impl Descriptor {
    #[inline]
    pub fn label_idx_str(&self) -> String {
        super::ext::format_ns_key(self.label)
    }

    /// Creates a descriptor from a blob
    #[inline]
    pub fn create(ns: &Namespace, opts: Opts, blob: impl AsRef<[u8]>, label: &str) -> Self {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(blob.as_ref());
        Descriptor {
            opts: opts,
            size: blob.as_ref().len() as u64,
            dchk: ns.key(digest.as_slice()),
            label: ns.key(label),
            offset: 0,
        }
    }

    /// Recovers a descriptor
    #[inline]
    pub fn recover(ns: &Namespace, opts: Opts, blob: impl AsRef<[u8]>, label: u64) -> Self {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(blob.as_ref());

        Self {
            opts,
            size: blob.as_ref().len() as u64,
            dchk: ns.key(digest.as_slice()),
            label,
            offset: 0,
        }
    }

    /// Returns true if this descriptor is stored inline
    #[inline]
    pub fn is_inline(&self) -> bool {
        (self.offset + self.size) < NS_HEADER_INLINE_BLOCK_SIZE as u64
    }
}

/// Trait for cas objects to describe themselves as a manifest
pub trait Describe {
    /// Returns a description of the container and contents of this type
    fn describe(&self, ns: &Namespace) -> FrameList;
}

/// Trait for a cas store to fetch bytes for a descriptor stored by this store
pub trait Fetch<'peek> {
    /// Fetches the bytes the correspond to a descriptor owned by this store
    fn fetch(&'peek self, ns: &Namespace, desc: &Descriptor) -> Option<&'peek [u8]>;
}

impl std::fmt::Debug for Descriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Descriptor")
            .field("label", &self.label)
            .field("size", &self.size)
            .field("offset", &self.offset)
            .field("dchk", &self.dchk)
            .field("opts.storage", &self.opts.storage)
            .field("opts.storage", &self.opts.spec)
            .finish()
    }
}

impl<'peek> Fetch<'peek> for crate::Data {
    fn fetch(&'peek self, ns: &Namespace, desc: &Descriptor) -> Option<&'peek [u8]> {
        let offset = Some(desc.offset)
            .filter(|o| {
                let o = *o as usize;
                if (o + desc.size as usize) < self.len() {
                    let digest = sha2::Sha256::digest(&self[o..o + desc.size as usize]);
                    ns.key(digest.as_slice()) == desc.dchk
                } else {
                    false
                }
            })
            .unwrap_or_else(|| {
                // TODO: This is probably overkill
                let (offset, _) = self
                    .find_ns_view_offset(desc.size as usize, ns, desc.dchk)
                    .unwrap_or_default();
                offset
            }) as usize;
        self.get(offset..offset + desc.size as usize)
    }
}

use crate::{
    Namespace, Opts,
    wire::{
        Boot,
        boot::{Container, NS_HEADER_INLINE_BLOCK_SIZE},
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::trace;

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

    /// Pushes a new frame descriptor to the list
    ///
    /// Notes:
    /// - Adjusts the offset of the descriptor and adds logging
    #[inline]
    pub fn push(&mut self, mut frame: Descriptor) {
        trace!(
            size_b = frame.size,
            offset = frame.offset,
            label = frame.label,
            dchk = frame.dchk,
            "push_frame"
        );
        frame.offset = self.last_offset();
        self.frames.push(frame);
    }

    /// Appends a frame list to the end of this framelist
    ///
    /// Notes:
    /// - Align offset of the list being appended and adds logging
    #[inline]
    pub fn append(&mut self, append: FrameList) {
        for f in append.frames {
            self.push(f);
        }
    }

    /// Returns the last offset of the frame list
    #[inline]
    fn last_offset(&self) -> u64 {
        self.frames
            .last()
            .map(|f| f.offset + f.size)
            .unwrap_or_default()
    }
}

/// Describes a `Frame` of data
#[derive(Ord, PartialOrd, PartialEq, Eq, Clone, Hash, Serialize, Deserialize)]
pub struct Descriptor {
    /// Opts
    #[serde(with = "crate::opts::ser", rename = "f")] // 'f' for flags
    pub opts: Opts,
    /// Label to identify the descriptor
    #[serde(rename = "l")]
    pub label: u64,
    /// Byte offset
    #[serde(rename = "o")]
    pub offset: u64,
    /// Size of the blob
    #[serde(rename = "s")]
    pub size: u64,
    /// Digest checksum
    #[serde(rename = "d")]
    pub dchk: u64,
}

impl Descriptor {
    /// Returns the label idx string
    #[inline]
    pub fn label_idx_str(&self) -> String {
        format!("{:x}", self.label)
    }

    /// Creates a descriptor from a blob
    #[inline]
    pub fn create<D: sha2::Digest>(
        ns: &Namespace,
        opts: Opts,
        blob: impl AsRef<[u8]>,
        label: &str,
    ) -> Self {
        let digest = D::digest(blob.as_ref());
        Descriptor {
            opts: opts,
            size: blob.as_ref().len() as u64,
            dchk: ns.key(digest.as_slice()),
            label: ns.key(label),
            offset: 0,
        }
    }

    /// Returns true if this descriptor is stored inline
    #[inline]
    pub fn is_inline(&self) -> bool {
        (self.offset + self.size) < NS_HEADER_INLINE_BLOCK_SIZE as u64
    }

    /// Returns true if the namespace/blob data matches the digest checksum
    #[inline]
    pub fn check<D: sha2::Digest>(&self, ns: &Namespace, blob: impl AsRef<[u8]>) -> bool {
        let digest = D::digest(&blob.as_ref());
        ns.key(digest.as_slice()) == self.dchk
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
            .field("opts.spec", &self.opts.spec)
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
                let (offset, ..) = self
                    .find_ns_view_offset::<Sha256>(desc.size as usize, ns, desc.dchk)
                    .unwrap_or_default();
                offset
            }) as usize;
        self.get(offset..offset + desc.size as usize)
    }
}

impl<'peek> Fetch<'peek> for Boot {
    #[inline]
    fn fetch(&'peek self, _: &Namespace, desc: &Descriptor) -> Option<&'peek [u8]> {
        let header_data = self.inline();

        if (desc.offset + desc.size) <= header_data.len() as u64 {
            let offset = desc.offset as usize;
            Some(&header_data[offset..offset + desc.size as usize])
        } else {
            None
        }
    }
}

impl<'wire> Fetch<'wire> for Container {
    #[inline]
    fn fetch(&'wire self, ns: &Namespace, desc: &Descriptor) -> Option<&'wire [u8]> {
        if desc.is_inline() {
            self.boot.fetch(ns, desc)
        } else if let Some(recv) = self.transport.as_receive() {
            recv.fetch(ns, desc)
        } else {
            None
        }
    }
}

pub mod filters {
    use crate::Opts;

    /// Filter that returns all frames
    pub const ALL_FRAMES: Option<fn(&Opts) -> bool> = None;

    /// Filter that returns all `tool` frames
    #[inline]
    pub fn tools(opts: &Opts) -> bool {
        opts.is_tool()
    }
}

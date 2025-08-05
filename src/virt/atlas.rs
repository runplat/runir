use ahash::HashSet;
use anyhow::anyhow;
use generic_array::{GenericArray, typenum::U32};
use sha2::{Digest, Sha256};

use crate::util::Packer;

use super::vol::{Volume, VolumeTarget};

/// AtlasEncoder enables storing objects that require un-packing at runtime, by "pre-encoding" stored objects
/// 
/// Given a list of objects that belong to some container system, each object is "pre-encoded" to the atlas and assigned an index.
/// 
/// If an object requires additional memory to be used, the index for that object reserves future memory, otherwise it is considered "inline".
/// 
/// At runtime, the atlas can be used along side the data storage medium to unpack objects on demand into the atlas
#[derive(Default)]
pub struct AtlasEncoder {
    objects: Vec<Stored>,
    unpacked: HashSet<usize>,
}

#[derive(Clone, Copy)]
enum Stored {
    Inline,
    Object {
        digest: GenericArray<u8, U32>,
        offset: u64,
        len: u32,
    },
}
impl AtlasEncoder {
    /// Returns the total runtime size
    #[inline]
    pub fn total_runtime_size(&self) -> u32 {
        self.objects.iter().map(|o| {
            match o {
                Stored::Inline => 0,
                Stored::Object { len, .. } => *len,
            }
        }).sum()
    }

    /// Pre-encodes an "inline" object
    /// 
    /// Returns the index of the object
    /// 
    /// Note: In this case pre-encoding is just record keeping
    #[inline]
    pub fn pre_encode_next_inline(&mut self) -> usize {
        let idx = self.objects.len();
        self.objects.push(Stored::Inline);
        idx
    }

    /// Pre-encodes a packed object
    /// 
    /// Returns the index of the object
    #[inline]
    pub fn pre_encode_object(&mut self, expected: GenericArray<u8, U32>, len: u32) -> usize {
        let idx = self.objects.len();
        let next_offset = self
            .objects
            .iter()
            .filter_map(|f| {
                if let Stored::Object { offset, len, .. } = *f {
                    Some((offset, len))
                } else {
                    None
                }
            })
            .last()
            .map(|s| s.0 + (s.1 as u64))
            .unwrap_or(0);
        self.objects.push(Stored::Object { digest: expected, offset: next_offset, len });
        idx
    }

    /// Encodes a packed object by unpacking it into the target buffer
    /// 
    /// Validates the unpacked object matches the expected pre-encoded digest
    /// 
    /// Marks the index as "unpacked", so that subsequent calls are a no-op
    #[inline]
    pub fn encode_packed_object<P: Packer>(&mut self, idx: usize, packed: &[u8], target: &mut [u8]) -> crate::Result<()> {
        if self.unpacked.contains(&idx) {
            return Ok(());
        }

        if let Some(Stored::Object { digest, offset, len }) = self.objects.get(idx) {
            P::unpack(packed, &mut target[*offset as usize..*offset as usize + *len as usize])?;

            let _item = Sha256::digest(&target[*offset as usize..*offset as usize + *len as usize]);
            if _item.as_slice() != digest.as_slice() {
                return Err(anyhow!("Unpacked item's digest does not match expected pre-encoded object digest").into())
            }

            self.unpacked.insert(idx);

            Ok(())
        } else {
            Err(anyhow!("Cannot encode a non-pre encoded object").into())
        }
    }

    /// Returns true if the object has been marked as unpacked
    #[inline]
    pub fn is_unpacked(&self, usize: usize) -> bool {
        self.unpacked.contains(&usize)
    }

    /// Returns the current number of objects
    #[inline]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Returns true if the encoder has not encoded any objects
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Returns the object extent at idx
    #[inline]
    pub fn object(&self, idx: usize) -> Option<(u64, u32)> {
        self.objects.get(idx).and_then(|s| match s {
            Stored::Inline => None,
            Stored::Object { offset, len, .. } => Some((*offset, *len)),
        })
    }
}

impl<T: VolumeTarget + AsMut<[u8]>> Volume<T, AtlasEncoder> {
    /// Returns an "Atlas" volume for storing object bytes
    #[inline]
    pub fn new_atlas(target: T) -> Self {
        Self::from((target, AtlasEncoder::default()))
    }

    /// "Pre-" encode an inline-object
    ///
    /// Returns the index for the inline-object
    ///
    /// Note: This ensures that both sides are in sync and can return useful errors
    #[inline]
    pub fn pre_encode_inline(&mut self) -> usize {
        self.encoder_mut().pre_encode_next_inline()
    }

    /// "Pre-" encodes an object
    ///
    /// Returns an error if the target does not have enough capacity for this object;
    ///
    /// Otherwise, returns the index of the object
    #[inline]
    pub fn pre_encode(
        &mut self,
        expected: GenericArray<u8, U32>,
        expected_len: u32,
    ) -> crate::Result<usize> {
        let _size_check = self.encoder().total_runtime_size() as u64 + expected_len as u64;
        if _size_check > self.target().remaining() {
            Err(anyhow!("Not enough space to pre-encode object").into())
        } else {
            self.target_mut().advance(expected_len as usize)?;
            Ok(self.encoder_mut().pre_encode_object(expected, expected_len))
        }
    }

    /// Encodes an object
    ///
    /// Returns an error if the unpacked object did not match the expected pre-encoded digest, or if the idx returned an inline object
    #[inline]
    pub fn encode_object<P: Packer>(&mut self, idx: usize, packed: &[u8]) -> crate::Result<()> {
        let (target, encoder) = self.parts_mut();
       encoder
            .encode_packed_object::<P>(idx, packed, target.as_mut())
    }

    /// Returns true if the obj at idx has been marked as unpacked
    #[inline]
    pub fn is_object_unpacked(&self, idx: usize) -> bool {
        self.encoder().is_unpacked(idx)
    }

    /// View bytes for an object
    #[inline]
    pub fn view_object(&self, idx: usize) -> crate::Result<&[u8]> {
        match self.encoder().object(idx) {
            Some((offset, len)) => {
                Ok(&self.target().view()[offset as usize..offset as usize + len as usize])
            }
            None => Err(anyhow!("Object {idx} not found").into()),
        }
    }
}
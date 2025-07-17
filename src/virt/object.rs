use ahash::HashSet;
use anyhow::anyhow;
use generic_array::{GenericArray, typenum::U32};
use sha2::{Digest, Sha256};

use crate::util::Packer;

/// Object encoder tracks a list of inline or stored objects
#[derive(Default)]
pub struct ObjectEncoder {
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
impl ObjectEncoder {
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
            P::unpack_bytes(packed, &mut target[*offset as usize..*offset as usize + *len as usize])?;

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

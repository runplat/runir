use std::sync::atomic::AtomicU64;

use bytes::{Buf, BufMut};

use crate::vol::{MemoryMappedTarget, VolumeTarget};

pub struct Sync {
    target: MemoryMappedTarget,
}

impl Sync {
    /// Creates a new synced memory-map target
    #[inline]
    pub fn new(mut target: MemoryMappedTarget, block: u64) -> Self {
        target.advance(8).expect("Must have at least 8 bytes");
        target.put_u64(block);
        Self { target }
    }

    #[inline]
    pub fn block(&self, idx: usize) -> Option<&[u8]> {
        let size = 512;
        let offset = size * idx;
        self.target.view().get(8 + offset .. offset + 512)
    }

    /// Returns the nest block for writing 
    #[inline]
    pub fn next_block(&mut self) -> Option<(usize, &mut [u8])> {
        let next = self.next();
        let size = 512;

        let offset = (next * size) as usize;
        let block = self.target.get_mut(8 + offset..offset + size as usize)?;
        // TODO: put a timestamp to invalidate it
        Some((next as usize, block))
    }

    #[inline]
    fn block_size(&self) -> u64 {
        (&self.target.view()[8..16]).get_u64()
    }

    /// Returns the next available index
    #[inline]
    fn next(&mut self) -> u64 {
        let atomic = unsafe { &*(self.target.as_mut_ptr() as *mut AtomicU64) };
        atomic.fetch_add(1, std::sync::atomic::Ordering::AcqRel)
    }
}

#[cfg(test)]
mod tests {
    use bytes::BufMut;

    use crate::{virt::sync::Sync, vol::new_mmap_anon_target};

    #[test]
    fn test_sync_map_single_thread() {
        let map = new_mmap_anon_target("", 4096).unwrap();
        let mut sync = Sync::new(map, 512);

        let (idx, mut bytes) = sync.next_block().unwrap();
        assert_eq!(idx, 0);
        bytes.put(b"hello world".as_slice());
        let (idx, mut bytes) = sync.next_block().unwrap();
        assert_eq!(idx, 1);
        bytes.put(b"world hello".as_slice());

        let block = sync.block(0).unwrap();
        assert!(block.starts_with(b"hello world"));

        let block = sync.block(1).unwrap();
        assert!(block.starts_with(b"world hello"));
    }

    // #[test]
    // fn test_sync_map_file_multiple() {
    // }
}

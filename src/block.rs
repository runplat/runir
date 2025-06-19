use bytes::BytesMut;

/// Block is a compact data file used by the database to pack writes
/// and serve reads
/// 
/// Each block is a 1:1 mapping to an archive entry, and is a flat collection
/// of flexbuffer formatted data blobs
/// 
pub struct Block {
    /// Each data block may scale to a MAX_BLOCK_SIZE (defaults to 1KiB)
    data: BytesMut
}

impl Block {
}
use std::io::Read;

use bytes::{BufMut, BytesMut};
use serde::Serialize;
use tracing::trace;

/// Packer handles packing and unpacking bytes
pub trait Packer: private::Sealed {
    /// Pack bytes into a layer
    fn pack_bytes(bytes: &[u8], buffer: &mut BytesMut) -> crate::Result<()>;

    /// Unpack bytes from a layer
    fn unpack<'u>(from: &[u8], to: &'u mut [u8]) -> crate::Result<()>;

    /// Returns a decoder for bytes
    fn decoder(from: &[u8]) -> crate::Result<impl Read>;

    /// Pack an object into a layer
    fn pack_object<T: Serialize>(
        obj: &T,
        ser: &mut flexbuffers::FlexbufferSerializer,
        buffer: &mut BytesMut,
    ) -> crate::Result<()> {
        obj.serialize(&mut *ser)?;
        Self::pack_bytes(ser.view(), buffer)
    }
}

/// Generic Packer trait implementation
#[derive(Debug)]
pub struct GenericPacker<const SIZE_THRESHOLD: usize, const COMPRESSION_LEVEL: i32>;

impl<const SIZE_THRESHOLD: usize, const COMPRESSION_LEVEL: i32> Packer
    for GenericPacker<SIZE_THRESHOLD, COMPRESSION_LEVEL>
{
    fn pack_bytes(bytes: &[u8], buffer: &mut BytesMut) -> crate::Result<()> {
        if bytes.len() < SIZE_THRESHOLD {
            buffer.reserve(bytes.len());
            buffer.put(bytes);
            Ok(())
        } else {
            let _bytes = zstd::encode_all(bytes, COMPRESSION_LEVEL)?;
            buffer.reserve(_bytes.len());
            buffer.put(_bytes.as_slice());
            trace!(unpacked = bytes.len(), packed = buffer.len(), "pack_bytes");
            Ok(())
        }
    }

    fn unpack<'u>(from: &[u8], to: &'u mut [u8]) -> crate::Result<()> {
        if from.len() == to.len() {
            to.copy_from_slice(from);
            Ok(())
        } else {
            let decoded = zstd::decode_all(from)?;
            to.copy_from_slice(&decoded);
            Ok(())
        }
    }

    fn decoder(from: &[u8]) -> crate::Result<impl Read> {
        Ok(zstd::Decoder::new(std::io::Cursor::new(from))?)
    }
}

mod private {
    use super::GenericPacker;

    pub trait Sealed {}

    impl<const COMPRESSION_THRESHOLD: usize, const COMPRESSION_LEVEL: i32> Sealed
        for GenericPacker<COMPRESSION_THRESHOLD, COMPRESSION_LEVEL>
    {
    }
}

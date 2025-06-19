use bytes::{BufMut, Bytes};
use tokio_util::codec::Encoder;

use super::{DigestBuffer, Entry};

/// Simple tape archive encoder,
///
/// **NOTE** Does not try to generate a header, just re-packs an unpacked archive entry.
/// 
pub struct TapeEncoder {
    /// List of entries that have been encoded,
    /// 
    encoded: Vec<DigestBuffer>,
}

/// Returns a 512-byte zero-block which is used to end entries, and files
///
fn zero_block() -> Bytes {
    Bytes::from_iter(std::iter::repeat('\0' as u8).take(512))
}

impl Encoder<Entry> for TapeEncoder {
    type Error = anyhow::Error;

    fn encode(
        &mut self,
        item: Entry,
        dst: &mut bytes::BytesMut,
    ) -> std::result::Result<(), Self::Error> {
        if item.is_eoa() {
            dst.reserve(512 * 4);
            let zero_block = zero_block();
            dst.put(zero_block.clone());
            dst.put(zero_block.clone());
            dst.put(zero_block.clone());
            dst.put(zero_block.clone());
            Ok(())
        } else {
            dst.reserve(512 * 3);
            dst.reserve(item.header().size());
            let header_bytes = item.header().as_ref();
            dst.put(header_bytes);
            if let Some((bytes, digest)) = item.data() {
                dst.put(bytes);
                self.encoded.push(digest);
            }
            let zero_block = zero_block();
            dst.put(zero_block.clone());
            dst.put(zero_block.clone());
            Ok(())
        }
    }
}

use std::io::Error;
use ascii::AsAsciiStr;
use bytes::{BufMut, Bytes};
use flexbuffers::Builder;
use tokio_util::codec::Encoder;
use super::{DigestBuffer, Entry, HeaderBuilder};

/// Simple tape archive encoder,
///
/// **NOTE** Does not try to generate a header, just re-packs an unpacked archive entry.
///
#[derive(Default)]
pub struct TapeEncoder {
    /// List of entries that have been encoded,
    encoded: Vec<DigestBuffer>,
}

impl TapeEncoder {
    /// Returns digests of all records encoded into the archive
    #[inline]
    fn encoded(&self) -> impl Iterator<Item = &DigestBuffer> {
        self.encoded.iter()
    }

    /// Creates a manifest entry for the encoded entries
    #[inline]
    pub fn create_manifest(&self) -> std::io::Result<Entry> {
        let mut manifest = HeaderBuilder::regular(
            "MANIFEST"
                .as_ascii_str()
                .map_err(|e| Error::new(std::io::ErrorKind::InvalidFilename, e))?,
        )?
        .set_defaults_for_archive();
        let mut builder = Builder::default();
        let mut records = builder.start_vector();
        for rec in self.encoded() {
            records.push(&rec[..]);
        }
        records.end_vector();
        manifest.set_size(builder.view().len())?;
        manifest.set_last_modified(time::UtcDateTime::now().unix_timestamp() as u64)?;
        let entry = Entry::regular(
            manifest.build()?,
            Bytes::from(builder.take_buffer()),
        );

        Ok(entry)
    }
}

/// Returns a 512-byte zero-block which is used to end entries, and files
///
fn zero_block() -> Bytes {
    Bytes::from_iter(std::iter::repeat('\0' as u8).take(512))
}

impl Encoder<Entry> for TapeEncoder {
    type Error = std::io::Error;

    fn encode(
        &mut self,
        item: Entry,
        dst: &mut bytes::BytesMut,
    ) -> std::result::Result<(), Self::Error> {
        if item.is_zeroes() {
            dst.reserve(512 * 2);
            let zero_block = zero_block();
            dst.put(zero_block.clone());
            dst.put(zero_block.clone());
            Ok(())
        } else {
            dst.reserve(item.header().size());
            let header_bytes = item.header().as_ref();
            dst.put(header_bytes);
            if let Some((bytes, digest)) = item.data() {
                dst.reserve(bytes.len());
                let padding = bytes.len() % 512;
                dst.put(bytes);
                dst.put_bytes(0, 512 - padding);
                self.encoded.push(digest);
            }
            Ok(())
        }
    }
}

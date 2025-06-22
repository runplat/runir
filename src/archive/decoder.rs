use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::Decoder;
use crate::archive::Header;
use super::Entry;

/// Struct for decoding a tape archive file (tar)
#[derive(Default)]
pub struct TapeDecoder {
    /// Next entry in the archive,
    next: Vec<(Header, BytesMut)>,
}

impl Decoder for TapeDecoder {
    type Item = Entry;

    type Error = std::io::Error;

    fn decode(
        &mut self,
        src: &mut bytes::BytesMut,
    ) -> std::result::Result<Option<Self::Item>, Self::Error> {
        // A tape archive is represented by 512-byte chunks
        if src.is_empty() {
            return Ok(None);
        }

        // The end of an archive is signaled by 2-zero blocks
        // Continue advancing until src is empty
        if src[..512].iter().all(|b| *b == b'\0') {
            src.advance(512);
            return Ok(Some(Entry::Zeros));
        }

        // Checks if we are currently processing an entry
        if let Some((header, mut buf)) = self.next.pop() {
            let chunk = &src[..512];
            buf.put(chunk);

            // If the amount of data in the buf is less than the total size of the file
            // push the proccessing entry back onto the stack
            if buf.len() < header.size() as usize {
                self.next.push((header, buf));
            } else {
                // Truncate the buffer to the exact size indicated by the header
                buf.truncate(header.size() as usize);

                let buf = buf.freeze();
                let entry = Entry::regular(header, buf);
                src.advance(512);
                return Ok(Some(entry));
            }
        } else {
            // TODO: Compute checksum of the header to validate integrity
            let header = Header::from(&src[..512]);

            // This indicates that that for the entry will follow this header
            // Prep by allocating a buffer to store the entry in
            if header.size() > 0 {
                let size = header.size() as usize;

                // TODO: Can use a contiguous buffer and split off sections for each file
                let buf = BytesMut::with_capacity(size);

                self.next
                    .push((header, buf));
                src.reserve(size);
            } else {
                src.advance(512);
                return Ok(Some(Entry::other(header)));
            }
        }

        // Check if we can advance the cursor
        if src.has_remaining() {
            src.advance(512);
        }
        Ok(Some(Entry::Pending))
    }
}

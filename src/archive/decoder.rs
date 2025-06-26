use super::Entry;
use crate::archive::Header;
use bytes::{Buf, BufMut, BytesMut};
use sha2::{Digest, Sha256};
use tokio_util::codec::Decoder;

/// Struct for decoding a tape archive file (tar)
#[derive(Default)]
pub struct TapeDecoder {
    /// Next entry in the archive,
    entries: Vec<(Header, Dest)>,
    /// True if tape decoder should only calculate digests of entries
    digest_only: bool,
    /// Cursor position
    cursor: usize,
}

impl TapeDecoder {
    /// Returns a tape recorder that only emits entry references
    pub fn references_only() -> Self {
        Self { entries: vec![], digest_only: true, cursor: 0 }
    }
}

enum Dest {
    Bytes(BytesMut),
    Digester {
        digest: Sha256,
        offset: usize,
        len: usize,
    }
}

impl Dest {
    pub fn put_chunk(&mut self, chunk: &[u8]) {
        match self {
            Dest::Bytes(bytes_mut) => bytes_mut.put(chunk),
            Dest::Digester {
                digest,
                len,
                ..
            } => {
                digest.update(chunk);
                *len += chunk.len();
            },
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Dest::Bytes(bytes_mut) => bytes_mut.len(),
            Dest::Digester { len, .. } => *len,
        }
    }

    pub fn to_entry(self, header: Header) -> Entry {
        match self {
            Dest::Bytes(mut buf) => {
                // Truncate the buffer to the exact size indicated by the header
                buf.truncate(header.size() as usize);

                let buf = buf.freeze();
                Entry::regular(header, buf)
            },
            Dest::Digester { digest, offset, .. } => {
                Entry::Reference { header, digest, offset }
            },
        }
    }
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
            self.cursor += 512;
            return Ok(Some(Entry::Zeros));
        }

        // Checks if we are currently processing an entry
        if let Some((header, mut buf)) = self.entries.pop() {
            let chunk = &src[..512];
            if buf.len() + 512 > header.size() {
                let slice_end = header.size() - buf.len();
                buf.put_chunk(&chunk[..slice_end]);
            } else {
                buf.put_chunk(chunk);
            }

            // If the amount of data in the buf is less than the total size of the file
            // push the proccessing entry back onto the stack
            if buf.len() < header.size() as usize {
                self.entries.push((header, buf));
            } else {
                let entry = buf.to_entry(header);
                src.advance(512);
                self.cursor += 512;
                return Ok(Some(entry));
            }
        } else {
            let header = Header::from(&src[..512]);

            if !header.is_checksum_valid() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Checksum in header is invalid, archive integrity cannot be ensured",
                ));
            }

            // This indicates that that for the entry will follow this header
            // Prep by allocating a buffer to store the entry in
            if header.size() > 0 {
                let size = header.size() as usize;

                self.entries.push((header, if self.digest_only {
                    Dest::Digester { digest: Sha256::new(), len: 0, offset: self.cursor + 512 }
                } else {
                    Dest::Bytes(BytesMut::with_capacity(size))
                }));
                src.reserve(size);
            } else {
                src.advance(512);
                self.cursor += 512;
                return Ok(Some(Entry::other(header)));
            }
        }

        // Check if we can advance the cursor
        if src.has_remaining() {
            src.advance(512);
            self.cursor += 512;
        }
        Ok(Some(Entry::Pending))
    }
}

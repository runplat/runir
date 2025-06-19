use bytes::{Buf, BufMut, BytesMut};
use sha2::{Digest, Sha256};
use tokio_util::codec::Decoder;
use tracing::trace;
use crate::archive::Header;
use super::Entry;

/// Struct for decoding a tape archive file (tar),
pub struct TapeDecoder {
    /// Next entry in the archive,
    next: Vec<(Header, BytesMut, sha2::Sha256)>,
    /// End of archive counter,
    eoa: usize,
    /// Digester,
    digester: Sha256,
}

impl TapeDecoder {
    /// Returns a new tape archive,
    pub fn new() -> Self {
        Self {
            next: vec![],
            eoa: 0,
            digester: Sha256::new(),
        }
    }

    /// Returns true if the end of the archive has been reached,
    pub fn is_eoa(&self) -> bool {
        self.eoa >= 4
    }

    /// Returns the current digest of the archive,
    pub fn digest(&self) -> String {
        format!("sha256:{}", hex::encode(self.digester.clone().finalize()))
    }
}

impl Decoder for TapeDecoder {
    type Item = Entry;

    type Error = anyhow::Error;

    fn decode(
        &mut self,
        src: &mut bytes::BytesMut,
    ) -> std::result::Result<Option<Self::Item>, Self::Error> {
        // A tape archive is represented by 512-byte chunks
        if src.len() < 512 {
            return Ok(None);
        }

        // The end of an archive entry is represented as 2 512 NUL-byte chunks
        // The end of an archive is represented as 4 512 NUL-byte chunks
        if src[..512].iter().all(|b| *b == b'\0') {
            src.advance(512);
            self.eoa += 1;
            return Ok(None);
        }

        // If we pass the above check, it means we've found a new entry
        self.eoa = 0;

        // Checks if we are currently processing an entry
        if let Some((header, mut buf, mut digester)) = self.next.pop() {
            let chunk = &src[..512];
            buf.put(chunk);
            digester.update(chunk);
            if buf.len() < header.size() as usize {
                self.next.push((header, buf, digester));
            } else {
                buf.truncate(header.size() as usize);
                let buf = buf.freeze();
                self.digester.update(&buf);
                self.digester.update(header.as_ref());
                let entry = Entry::regular(header, buf, digester.finalize().into());
                src.advance(512);
                return Ok(Some(entry));
            }
        } else {
            let header = Header::from(&src[..512]);
            trace!("\n------------------\n{header}\n------------------");
            if header.size() > 0 {
                let size = header.size() as usize;
                self.next
                    .push((header, BytesMut::with_capacity(size), Sha256::new()));
                src.reserve(size);
            } else {
                self.digester.update(header.as_ref());
                src.advance(512);
                return Ok(Some(Entry::other(header)));
            }
        }

        // Check if we can advance the cursor
        if src.has_remaining() {
            src.advance(512);
        }
        Ok(None)
    }
}

#[allow(unused_imports)]
mod tests {
    // use async_compression::tokio::bufread::GzipDecoder;
    // use bytes::BytesMut;
    // use std::collections::BTreeSet;
    // use std::path::PathBuf;
    // use tokio::io::{AsyncReadExt, BufReader};
    // use tokio_util::codec::Decoder;
    // use tokio_util::io::InspectReader;
    // use crate::prelude::*;
    // use crate::storage::archive::TapeDecoder;

    // #[tokio::test]
    // #[tracing_test::traced_test]
    // async fn test_archive() {
    //     let storage = boxed_storage(PathBuf::from("tests/archive_test"));
    //     let desc: Arc<Descriptor> = Descriptor::builder("application/vnd.docker.image.rootfs.diff.tar.gzip")
    //         .digest("sha256:4000adbbc3eb1099e3a263c418f7c1d7def1fa2de0ae00ba48245cda9389c823")
    //         .into();
    //     let location = storage.prepare_location(desc.clone()).await.unwrap();

    //     let file_list = tokio::fs::read_to_string("tests/archive_test/file_list.txt")
    //         .await
    //         .unwrap();

    //     let mut file_list = file_list
    //         .split("\n")
    //         .filter(|line| !line.is_empty())
    //         .fold(BTreeSet::new(), |mut acc, line| {
    //             acc.insert(line);
    //             acc
    //         });

    //     let layer_tar = layers::LayerTar::open(location).await.unwrap();
    //     let mut stream = layer_tar.stream_entries().await.unwrap();
    //     while let Some(entry) = stream.next().await {
    //         let entry = entry.unwrap();
    //         assert!(file_list.remove(entry.header().name()));
    //     }

    //     let digest = layer_tar.digest().await;
    //     assert_eq!(
    //         "4000adbbc3eb1099e3a263c418f7c1d7def1fa2de0ae00ba48245cda9389c823", digest,
    //         "should match the stored digest exactly"
    //     );
    //     assert_eq!(0, file_list.len());

    //     let location = storage.prepare_location(desc).await.unwrap();
    //     let mut vec = vec![];
    //     location.snapshot().read_to_end(&mut vec).await.unwrap();
    //     let digester = Sha256::digest(&vec);
    //     println!("{}", hex::encode(digester));
    // }
}

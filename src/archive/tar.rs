use async_compression::tokio::bufread::GzipDecoder;
use bytes::BytesMut;
use tokio::io::BufReader;
use tokio_util::codec::Decoder;
use tokio_util::io::InspectReader;

use crate::prelude::*;

/// Reads a compressed/decompressed layer .tar file,
///
pub struct LayerTar {
    /// Location of the layer,
    ///
    location: LocationRef,
    /// True if the layer is compressed,
    ///
    is_compressed: bool,
    /// After entries have been streamed once, this digest will be set
    ///
    digest: RwLock<String>,
}

impl LayerTar {
    /// Opens a location as a layer tar,
    /// 
    /// Returns an error if the location doesn't exist.
    /// 
    /// **Note** Will automatically detect whether the layer is compressed by checking if the media type ends
    /// with `.tar.gz`
    /// 
    pub async fn open(location: LocationRef) -> Result<Self> {
        if location.exists().await? {
            let desc = location.as_ref();
            let is_compressed = desc.media_type.ends_with(".tar.gzip") || desc.media_type.ends_with(".tar.gz");
            Ok(Self { location, is_compressed, digest: RwLock::new(String::new()) })
        } else {
            Err(StorageErrors::DoesNotExist.into())
        }
    }

    /// Returns the string calculated for this layer tar,
    /// 
    pub async fn digest(&self) -> String {
        self.digest.read().await.to_string()
    }

    /// Opens the the underlying location and returns a stream of tar entries,
    /// 
    /// Returns an error if the location wasn't able to return a reader.
    /// 
    /// The stream will return TapeEntry's if successful and an error if an issue 
    /// occurred while processing the archive.
    /// 
    pub async fn stream_entries(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<crate::storage::TapeEntry>> + '_>>> {
        let reader = self.location.read().await?;

        let stream = stream! {
            trace!("Starting layer entry stream");
            let digester = Arc::new(std::sync::RwLock::new(Sha256::new()));

            let reader = InspectReader::new(reader, |b| {
                digester.write().map(|mut d| {
                    d.update(b);
                }).ok();
            });

            // 
            let mut reader: Pin<Box<dyn AsyncRead>> = if self.is_compressed {
                Box::pin(GzipDecoder::new(BufReader::new(reader)))
            } else {
                Box::pin(BufReader::new(reader))
            };

            let mut tar = TapeDecoder::new();
            let mut buf = BytesMut::with_capacity(512 * 10);
            loop {
                reader.read_buf(&mut buf).await?;
                while let Some(entry) = tar.decode(&mut buf)? {
                    yield Ok(entry);
                }

                if tar.is_eoa() {
                    trace!("End of stream");
                    break;
                }
            }

            let digest = digester.read().map_err(Error::new)?.clone().finalize();
            let digest = hex::encode(digest);
            trace!("{digest}");
            *self.digest.write().await = digest;
        };

        Ok(Box::pin(stream))
    }
}

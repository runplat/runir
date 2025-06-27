/// Type alias for the bytes of a SHA256 digest,
pub type Sha256Digest = [u8; 32];

mod header;
pub use header::FileType;
pub use header::Header;
pub use header::HeaderBuilder;

mod entry;
pub use entry::Entry;
pub use entry::FileEntry;
pub use entry::FileEntryReference;

mod decoder;
pub use decoder::TapeDecoder;

mod encoder;
pub use encoder::TapeEncoder;
pub use encoder::JournalEntry;

mod manifest;
pub use manifest::Manifest;

/// Scans an input tape archive and returns references
#[inline]
pub async fn scan_for_references(input: impl tokio::io::AsyncRead + Send + Unpin + 'static) -> std::io::Result<Vec<Entry>> {
    use futures::StreamExt;

    let mut references = vec![];

    let decoder = TapeDecoder::references_only();
    let mut reader = tokio_util::codec::FramedRead::new(input, decoder);

    while let Some(entry) = reader.next().await {
        if matches!(
            entry,
            Ok(Entry::Reference { .. })
        ) {
            references.push(entry?);
        }
    }

    Ok(references)
}
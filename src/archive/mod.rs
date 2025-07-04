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
pub use encoder::JournalEntry;
pub use encoder::TapeEncoder;

mod manifest;
pub use manifest::Manifest;

/// Scans an input tape archive and returns references
#[inline]
pub async fn scan_for_references(
    input: impl futures::AsyncRead + Send + Unpin + 'static,
) -> std::io::Result<Vec<Entry>> {
    use futures::StreamExt;

    let mut references = vec![];

    let decoder = TapeDecoder::references_only();
    let mut reader = asynchronous_codec::FramedRead::new(input, decoder);

    while let Some(entry) = reader.next().await {
        if matches!(entry, Ok(Entry::Reference { .. })) {
            references.push(entry?);
        }
    }

    Ok(references)
}

/// Archives the worker state to an output stream
///
/// Returns a record containing a manifest of the contents written to the output stream
#[inline]
pub async fn archive_to(
    stream: impl futures::Stream<Item = std::io::Result<Entry>> + '_,
    output: impl futures::AsyncWrite + Send + Unpin + 'static,
) -> std::io::Result<Manifest> {
    use futures::sink::SinkExt;
    let encoder = TapeEncoder::default();

    let mut writer = asynchronous_codec::FramedWrite::new(output, encoder);

    // Creates archive entries of all archivable records and encodes to the output stream
    tokio::pin!(stream);

    // Send a stream of entries
    writer
        .send_all(&mut stream)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Interrupted, e))?;

    // Need to send this last to indicate the end of the archive
    writer
        .send(Entry::Zeros)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Interrupted, e))?;

    // Applies the digest of the current state of the archive to all journal entries
    writer.encoder_mut().stamp_source_digest();

    // Close the writer
    writer
        .close()
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Interrupted, e))?;

    let manifest = writer.encoder().create_manifest();
    Ok(manifest)
}

/// Type alias for the bytes of a SHA256 digest,
pub type DigestBuffer = [u8; 32];

mod header;
pub use header::FileType;
pub use header::Header;

mod entry;
pub use entry::Entry;
pub use entry::FileEntry;

mod decoder;
pub use decoder::TapeDecoder;

mod encoder;
pub use encoder::TapeEncoder;

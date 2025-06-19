use std::fmt::Display;

use ascii::{AsAsciiStr, AsciiStr};
use bytes::{Bytes, BytesMut};
use tracing::{trace, warn};

/// Struct containing data for a TAR file header,
///
#[derive(Default, Debug, Clone)]
pub struct Header {
    /// Raw bytes of the file header,
    bytes: Bytes,
    /// GNU-part of the header, this should be the same across all formats.
    gnu: GNUHeader,
    /// If set, it means that the "ustar" indicator was present
    ustar: Option<UStarHeader>,
}

/// Empty Entry Header,
pub static EMPTY_HEADER: Header = Header {
    bytes: Bytes::new(),
    gnu: GNUHeader(HeaderAdapter::new()),
    ustar: None,
};

impl Header {
    /// Filename,
    pub fn name(&self) -> &str {
        self.gnu.name()
    }

    /// Access mode,
    ///
    pub fn mode(&self) -> u32 {
        self.gnu.mode()
    }

    /// Owner id,
    ///
    pub fn owner_id(&self) -> u32 {
        self.gnu.owner_id()
    }

    /// Group id,
    ///
    pub fn group_id(&self) -> u32 {
        self.gnu.group_id()
    }
    /// File size,
    ///
    pub fn size(&self) -> usize {
        self.gnu.size()
    }

    /// Last modified,
    ///
    pub fn last_modified(&self) -> u64 {
        self.gnu.last_modified()
    }

    /// Checksum,
    ///
    pub fn checksum(&self) -> &str {
        self.gnu.checksum()
    }

    /// File type,
    ///
    pub fn file_type(&self) -> FileType {
        self.gnu.file_type()
    }

    /// Linked filename,
    ///
    pub fn link_name(&self) -> &str {
        self.gnu.link_name()
    }

    /// UStar version,
    ///
    pub fn version(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.version())
    }

    /// UStar owner name,
    ///
    pub fn owner_name(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.owner_name())
    }

    /// UStar group name,
    ///
    pub fn group_name(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.group_name())
    }

    /// UStar device major no,
    ///
    pub fn device_major_no(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.device_major_no())
    }

    /// UStar device minor no,
    ///
    pub fn device_minor_no(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.device_minor_no())
    }

    /// UStar filename prefix,
    ///
    pub fn filename_prefix(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.filename_prefix())
    }
}

/// Struct containing the content of the gnu file header,
///
#[derive(Clone, Debug, Default)]
pub struct GNUHeader(HeaderAdapter);

impl GNUHeader {
    /// Filename
    pub fn name(&self) -> &str {
        self.0.as_str(0..100).trim_end_matches("\0")
    }

    /// Access mode
    pub fn mode(&self) -> u32 {
        self.0.decode_octal(100..100 + 8) as u32
    }

    /// Owner id
    pub fn owner_id(&self) -> u32 {
        self.0.decode_octal(108..108 + 8) as u32
    }

    /// Group id
    pub fn group_id(&self) -> u32 {
        self.0.decode_octal(116..116 + 8) as u32
    }

    /// File size
    pub fn size(&self) -> usize {
        self.0.decode_octal(124..124 + 12) as usize
    }

    /// Last modified
    pub fn last_modified(&self) -> u64 {
        self.0.decode_octal(136..136 + 12)
    }

    /// Checksum
    pub fn checksum(&self) -> &str {
        self.0.as_str(148..148 + 8).trim_end_matches("\0 ")
    }

    /// File type
    pub fn file_type(&self) -> FileType {
        let ty = self.0.decode_octal(156..156 + 1);
        let ty = FileType::new(ty as u8);
        if ty == FileType::Unknown {
            trace!("\n{}", self.0.as_str(156..156 + 1));
        }
        ty
    }

    /// Linked filename
    pub fn link_name(&self) -> &str {
        self.0.as_str(157..157 + 100).trim_end_matches("\0")
    }
}

/// Enumeration of file types,
///
#[repr(u8)]
#[derive(PartialEq, PartialOrd, Debug)]
pub enum FileType {
    /// Normal file
    Regular = 0,
    /// Hard link
    HardLink = 1,
    /// Soft link
    SoftLink = 2,
    /// Character device
    CharacterSpecial = 3,
    /// Block device
    BlockSpecial = 4,
    /// Directory
    Directory = 5,
    /// FIFO pipe
    FIFO = 6,
    /// Contiguous blob
    Contiguous = 7,
    /// PAX global extended header
    GlobalExtendedHeader = 0o147u8,
    /// PAX file extended header
    ExtendedHeader = 0o170,
    /// Unknown type
    Unknown,
}

impl FileType {
    /// Converts a u8 to the corresponding FileType variant,
    ///
    pub fn new(char: u8) -> FileType {
        match char {
            0 => FileType::Regular,
            1 => FileType::HardLink,
            2 => FileType::SoftLink,
            3 => FileType::CharacterSpecial,
            4 => FileType::BlockSpecial,
            5 => FileType::Directory,
            6 => FileType::FIFO,
            7 => FileType::Contiguous,
            0o147u8 => FileType::GlobalExtendedHeader,
            0o170 => FileType::ExtendedHeader,
            _ => {
                warn!("unknown archive file type {:o}", char);
                FileType::Unknown
            }
        }
    }
}

/// Pointer struct to enable reading from the extended part of the file header,
///
#[derive(Debug, Clone)]
struct UStarHeader(HeaderAdapter);

impl UStarHeader {
    /// UStar version
    fn version(&self) -> &str {
        self.0.as_str(6..6 + 2)
    }

    /// Owner name
    fn owner_name(&self) -> &str {
        self.0.as_str(8..8 + 32)
    }

    /// Group name
    fn group_name(&self) -> &str {
        self.0.as_str(40..40 + 32)
    }

    /// Device major no
    fn device_major_no(&self) -> &str {
        self.0.as_str(72..72 + 8)
    }

    /// Device minor no
    fn device_minor_no(&self) -> &str {
        self.0.as_str(80..80 + 8)
    }

    /// Filename prefix
    fn filename_prefix(&self) -> &str {
        self.0.as_str(88..88 + 155)
    }
}

/// Adapter that provides common fn's for working with the bytes of an archive entry header,
///
#[derive(Clone, Debug, Default)]
struct HeaderAdapter(Bytes);

impl HeaderAdapter {
    /// Creates a new header adapter,
    /// 
    const fn new() -> Self {
        HeaderAdapter(Bytes::new())
    }
}

impl HeaderAdapter {
    /// Returns a range of bytes as a &str,
    #[inline]
    fn as_ascii(&self, range: impl std::ops::RangeBounds<usize>) -> Option<&AsciiStr> {
        match (range.start_bound(), range.end_bound()) {
            (std::ops::Bound::Included(s), std::ops::Bound::Included(e)) => {
                self.0[*s..=*e].as_ascii_str().ok()
            }
            (std::ops::Bound::Included(s), std::ops::Bound::Excluded(e)) => {
                self.0[*s..*e].as_ascii_str().ok()
            }
            (std::ops::Bound::Included(s), std::ops::Bound::Unbounded) => {
                self.0[*s..].as_ascii_str().ok()
            }
            (std::ops::Bound::Excluded(_), std::ops::Bound::Included(_)) => {
                // self.0[s+1..=e].as_ascii_str().ok()
                unreachable!("there are no semantics that support this scenario")
            }
            (std::ops::Bound::Excluded(_), std::ops::Bound::Excluded(_)) => {
                // self.0[s+1..e].as_ascii_str().ok()
                unreachable!("there are no semantics that support this scenario")
            }
            (std::ops::Bound::Excluded(_), std::ops::Bound::Unbounded) => {
                // self.0[s+1..].as_ascii_str().ok()
                unreachable!("there are no semantics that support this scenario")
            }
            (std::ops::Bound::Unbounded, std::ops::Bound::Included(e)) => {
                self.0[..=*e].as_ascii_str().ok()
            }
            (std::ops::Bound::Unbounded, std::ops::Bound::Excluded(e)) => {
                self.0[..*e].as_ascii_str().ok()
            }
            (std::ops::Bound::Unbounded, std::ops::Bound::Unbounded) => self.0.as_ascii_str().ok(),
        }
    }

    fn as_str(&self, range: impl std::ops::RangeBounds<usize>) -> &str {
        self.as_ascii(range).map(|a| a.as_str()).unwrap_or("")
    }

    /// Returns a range of bytes,
    fn as_bytes(&self, range: impl std::ops::RangeBounds<usize>) -> Bytes {
        self.0.slice(range)
    }

    /// Decodes an octal encoded value into an unsigned integer,
    fn decode_octal(&self, range: impl std::ops::RangeBounds<usize>) -> u64 {
        let mut value = 0;
        for (idx, b) in self.as_bytes(range).iter().enumerate() {
            if *b >= 48 {
                value = value << 3 | ((b - 0b00110000u8) as u64);
            } else {
                // if *b != 0 { warn!("tried decoding w/ overflow {}", b) };
                break;
            }

            if idx > 12 {
                break;
            }
        }
        value
    }
}

impl AsRef<[u8]> for Header {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl From<&[u8]> for Header {
    fn from(src: &[u8]) -> Self {
        if src.len() != 512 {
            Self::default()
        } else {
            let bytes = BytesMut::from_iter(src);
            let bytes = bytes.freeze();

            let gnu = bytes.slice_ref(&bytes[..257]);
            let gnu = GNUHeader(HeaderAdapter(gnu));

            let ustar = bytes.slice_ref(&bytes[257..]);
            let ustar = HeaderAdapter(ustar);

            Header {
                bytes,
                gnu,
                ustar: if ustar.as_str(0..6) == "ustar\0" {
                    Some(UStarHeader(ustar))
                } else {
                    None
                },
            }
        }
    }
}

impl Display for GNUHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "name:             {}", self.name())?;
        writeln!(f, "mode:             {:o}", self.mode())?;
        writeln!(f, "owner_id:         {}", self.owner_id())?;
        writeln!(f, "group_id:         {}", self.group_id())?;
        writeln!(f, "size:             {}", self.size())?;
        writeln!(f, "last_modified:    {}", self.last_modified())?;
        writeln!(f, "checksum:         {}", self.checksum())?;
        writeln!(f, "file_type:        {:?}", self.file_type())?;
        writeln!(f, "link_name:        {}", self.link_name())?;
        Ok(())
    }
}

impl Display for UStarHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "ustar-version:    {}", self.version())?;
        writeln!(f, "owner:            {}", self.owner_name())?;
        writeln!(f, "group:            {}", self.group_name())?;
        writeln!(f, "dev_mj:           {}", self.device_major_no())?;
        writeln!(f, "dev_min:          {}", self.device_minor_no())?;
        writeln!(f, "filename_prefix:  {}", self.filename_prefix())?;
        Ok(())
    }
}

impl Display for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.gnu)?;
        self.ustar.as_ref().map(|u| write!(f, "{u}"));
        Ok(())
    }
}

use ascii::{AsAsciiStr, AsciiStr, AsciiString};
use bytes::{BufMut, Bytes, BytesMut};
use std::{fmt::Display, io::Error, os::unix::fs::MetadataExt, path::Path};
use tracing::{trace, warn};

/// 65534 is a symbolic value for NO_OWNER/NO_GROUP
/// 
/// Reference: https://en.wikipedia.org/wiki/User_identifier#Special_values
const NO_OWNER_NO_GROUP: u16 = u16::MAX - 1;

/// Builds a TAR-compliant header
#[derive(Default)]
pub struct HeaderBuilder {
    /// Name of the entry
    name: AsciiString,
    /// File mode
    mode: u64,
    /// Owner id
    owner: u64,
    /// Group id
    group: u64,
    /// Size of the data stored by the entry
    size: usize,
    /// UNIX timestamp of when the entry was last modified
    last_modified: u64,
    /// File type of the entry
    file_type: FileType,
    /// Link file name
    link_name: AsciiString,
}

impl HeaderBuilder {
    /// Returns a header builder for a directory entry
    #[inline]
    pub fn dir(name: impl Into<AsciiString>) -> std::io::Result<Self> {
        let name = name.into();
        if !name.as_str().ends_with("/") {
            return Err(Error::new(
                std::io::ErrorKind::InvalidInput,
                "Directory names must end with a '/'",
            ));
        }
        let mut builder = Self::default();
        builder.set_name(name)?;
        builder.set_file_type(FileType::Directory)?;
        Ok(builder)
    }

    /// Returns a header builder for a regular file entry
    #[inline]
    pub fn regular(name: impl Into<AsciiString>) -> std::io::Result<Self> {
        let name = name.into();
        if name.as_str().ends_with("/") {
            return Err(Error::new(
                std::io::ErrorKind::InvalidInput,
                "Regular file names cannot end with a '/'",
            ));
        }
        let mut builder = Self::default();
        builder.set_name(name)?;
        builder.set_file_type(FileType::Regular)?;
        Ok(builder)
    }

    /// Creates a new TAR header from an existing file
    #[inline]
    #[cfg(target_family = "unix")]
    pub fn from_path(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut builder = HeaderBuilder::default();

        let meta = std::fs::symlink_metadata(path.as_ref())?;
        if meta.is_symlink() {
            builder.set_file_type(FileType::SoftLink)?;

            let link = std::fs::read_link(path.as_ref())?;
            // TODO: Add long name support
            builder.set_link_name(
                link.as_os_str()
                    .to_string_lossy()
                    .as_ascii_str()
                    .map_err(|e| Error::new(std::io::ErrorKind::InvalidFilename, e.to_string()))?,
            )?;
        }

        let meta = std::fs::metadata(path.as_ref())?;
        if meta.nlink() > 1 {
            builder.set_file_type(FileType::HardLink)?;
        }

        if meta.file_type().is_file() {
            builder.set_file_type(FileType::Regular)?;
        } else if meta.file_type().is_dir() {
            builder.set_file_type(FileType::Directory)?;
        }

        builder.set_file_mode(meta.mode())?;
        builder.set_last_modified(meta.mtime() as u64)?;
        builder.set_group_id(meta.gid())?;
        builder.set_owner_id(meta.uid())?;
        builder.set_size(meta.size() as usize)?;
        builder.set_name(
            path.as_ref()
                .as_os_str()
                .to_string_lossy()
                .as_ascii_str()
                .map_err(|e| Error::new(std::io::ErrorKind::InvalidFilename, e.to_string()))?,
        )?;

        Ok(builder)
    }

    /// Sets the the owner/group id's to nobody, and file mode to 755 for directories and 644 for everything else
    #[inline]
    pub fn set_defaults_for_archive(mut self) -> Self {
        if matches!(self.file_type, FileType::Directory) {
            self.set_file_mode(0o755).unwrap();
        } else {
            self.set_file_mode(0o644).unwrap();
        }
        self.set_owner_id(NO_OWNER_NO_GROUP as u32).unwrap();
        self.set_group_id(NO_OWNER_NO_GROUP as u32).unwrap();
        self
    }

    /// Sets the file name
    #[inline]
    pub fn set_name(&mut self, name: impl Into<AsciiString>) -> Result<(), std::io::Error> {
        let name = name.into();
        if name.len() > 100 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidFilename,
                "Name must not be longer than 100 characters",
            ));
        }
        self.name = name;
        Ok(())
    }

    /// Sets the file mode
    #[inline]
    pub fn set_file_mode(&mut self, mode: u32) -> Result<(), std::io::Error> {
        self.mode = mode as u64;
        Ok(())
    }

    /// Sets the owner id
    #[inline]
    pub fn set_owner_id(&mut self, owner: u32) -> Result<(), std::io::Error> {
        self.owner = owner as u64;
        Ok(())
    }

    /// Sets the group id
    #[inline]
    pub fn set_group_id(&mut self, group: u32) -> Result<(), std::io::Error> {
        self.group = group as u64;
        Ok(())
    }

    /// Sets the size of the archived data
    #[inline]
    pub fn set_size(&mut self, size: usize) -> Result<(), std::io::Error> {
        self.size = size;
        Ok(())
    }

    /// Sets the last_modified time (mtime) as a unix timestamp
    #[inline]
    pub fn set_last_modified(&mut self, ts: u64) -> Result<(), std::io::Error> {
        self.last_modified = ts;
        Ok(())
    }

    /// Sets the file type
    #[inline]
    pub fn set_file_type(&mut self, file_ty: FileType) -> std::io::Result<()> {
        self.file_type = file_ty;
        Ok(())
    }

    /// Sets the link name
    #[inline]
    pub fn set_link_name(&mut self, name: impl Into<AsciiString>) -> Result<(), std::io::Error> {
        let name = name.into();
        if name.len() > 100 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidFilename,
                "Name must not be longer than 100 characters",
            ));
        }
        self.link_name = name;
        Ok(())
    }

    /// Builds the header and computes the checksum
    #[inline]
    pub fn build(self) -> std::io::Result<Header> {
        let mut bytes = BytesMut::with_capacity(512);
        let name = self.name.as_bytes();
        bytes.put(name);

        let padding = 100 - name.len();
        bytes.put_bytes(0, padding);

        let mode = format!("{:07o}\0", self.mode);
        assert_eq!(8, mode.as_bytes().len());
        bytes.put(format!("{:07o} \0", self.mode)[1..].as_bytes());
        bytes.put(format!("{:07o} \0", self.owner)[1..].as_bytes());
        bytes.put(format!("{:07o} \0", self.group)[1..].as_bytes());
        bytes.put(format!("{:011o} ", self.size).as_bytes());
        bytes.put(format!("{:011o} ", self.last_modified).as_bytes());
        bytes.put(format!("        ").as_bytes());
        bytes.put(format!("{:o}", self.file_type as u8).as_bytes());

        if !self.link_name.is_empty()
            && !matches!(self.file_type, FileType::SoftLink | FileType::HardLink)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "A link name was set, however the file type was not set to a hard/soft link",
            ));
        }

        let link_name = self.link_name.as_bytes();
        bytes.put(link_name);
        let padding = 100 - link_name.len();
        bytes.put_bytes(0, padding);

        let padding = 512 - bytes.len();
        bytes.put_bytes(0, padding);

        let checksum: u32 = compute_header_checksum(&bytes);
        bytes[148..148 + 8].copy_from_slice(format!("{:06o}\0 ", checksum).as_bytes());

        let bytes = bytes.freeze();
        let gnu = bytes.slice_ref(&bytes[..257]);
        let gnu = GNUHeader(HeaderAdapter(gnu));

        let ustar = bytes.slice_ref(&bytes[257..]);
        let ustar = HeaderAdapter(ustar);
        Ok(Header {
            bytes,
            gnu,
            ustar: if ustar.as_str(0..6) == "ustar\0" {
                Some(UStarHeader(ustar))
            } else {
                None
            },
        })
    }
}

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
    /// Splits the entry name for record parts
    ///
    /// Returns None if the entry name does not contain record parts
    #[inline]
    pub fn split_name_for_record(&self) -> Option<(u64, uuid::Uuid, crate::Opts)> {
        use crate::Opts;
        use std::str::FromStr;
        self.name()
            .split_once("_")
            .and_then(|(nschk, uuid_opts)| {
                let nschk = u64::from_str_radix(nschk, 16).ok();
                let uuid_opts = uuid_opts.split_once("_").and_then(|(uuid, opts)| {
                    uuid::Uuid::from_str(uuid)
                        .ok()
                        .zip(u64::from_str_radix(opts, 16).ok().map(Opts::decode))
                });

                nschk.zip(uuid_opts)
            })
            .map(|(n, (r, o))| (n, r, o))
    }

    /// Computes the checksum and verifies that the computed checksum matches
    /// the checksum in the header
    #[inline]
    pub fn is_checksum_valid(&self) -> bool {
        self.checksum() == compute_header_checksum(&self.bytes)
    }

    /// Entry name
    #[inline]
    pub fn name(&self) -> &str {
        self.gnu.name()
    }

    /// Access mode
    #[inline]
    pub fn mode(&self) -> u32 {
        self.gnu.mode()
    }

    /// Owner id
    #[inline]
    pub fn owner_id(&self) -> u32 {
        self.gnu.owner_id()
    }

    /// Group id
    #[inline]
    pub fn group_id(&self) -> u32 {
        self.gnu.group_id()
    }
    /// File size
    #[inline]
    pub fn size(&self) -> usize {
        self.gnu.size()
    }

    /// Last modified
    #[inline]
    pub fn last_modified(&self) -> u64 {
        self.gnu.last_modified()
    }

    /// Checksum
    #[inline]
    pub fn checksum(&self) -> u32 {
        self.gnu.checksum()
    }

    /// File type
    #[inline]
    pub fn file_type(&self) -> FileType {
        self.gnu.file_type()
    }

    /// Linked filename
    #[inline]
    pub fn link_name(&self) -> &str {
        self.gnu.link_name()
    }

    /// UStar version
    #[inline]
    pub fn version(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.version())
    }

    /// UStar owner name
    #[inline]
    pub fn owner_name(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.owner_name())
    }

    /// UStar group name
    #[inline]
    pub fn group_name(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.group_name())
    }

    /// UStar device major no
    #[inline]
    pub fn device_major_no(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.device_major_no())
    }

    /// UStar device minor no
    #[inline]
    pub fn device_minor_no(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.device_minor_no())
    }

    /// UStar filename prefix
    #[inline]
    pub fn filename_prefix(&self) -> Option<&str> {
        self.ustar.as_ref().map(|u| u.filename_prefix())
    }
}

fn compute_header_checksum(bytes: &[u8]) -> u32 {
    bytes[..]
        .iter()
        .enumerate()
        .map(|(idx, b)| {
            if (148..148 + 8).contains(&idx) {
                ' ' as u32
            } else {
                *b as u32
            }
        })
        .map(|b| b as u32)
        .sum()
}

/// Struct containing the content of the gnu file header,
///
#[derive(Clone, Debug, Default)]
pub struct GNUHeader(HeaderAdapter);

impl GNUHeader {
    /// Filename
    #[inline]
    pub fn name(&self) -> &str {
        self.0.as_str(0..100).trim_end_matches("\0")
    }

    /// Access mode
    #[inline]
    pub fn mode(&self) -> u32 {
        self.0.decode_octal(100..100 + 8) as u32
    }

    /// Owner id
    #[inline]
    pub fn owner_id(&self) -> u32 {
        self.0.decode_octal(108..108 + 8) as u32
    }

    /// Group id
    #[inline]
    pub fn group_id(&self) -> u32 {
        self.0.decode_octal(116..116 + 8) as u32
    }

    /// File size
    #[inline]
    pub fn size(&self) -> usize {
        self.0.decode_octal(124..124 + 12) as usize
    }

    /// Last modified
    #[inline]
    pub fn last_modified(&self) -> u64 {
        self.0.decode_octal(136..136 + 12)
    }

    /// Checksum
    #[inline]
    pub fn checksum(&self) -> u32 {
        self.0.decode_octal(148..148 + 8) as u32
    }

    /// File type
    #[inline]
    pub fn file_type(&self) -> FileType {
        let ty = self.0.decode_octal(156..156 + 1);
        let ty = FileType::new(ty as u8);
        if ty == FileType::Unknown {
            trace!("\n{}", self.0.as_str(156..156 + 1));
        }
        ty
    }

    /// Linked filename
    #[inline]
    pub fn link_name(&self) -> &str {
        self.0.as_str(157..157 + 100).trim_end_matches("\0")
    }
}

/// Enumeration of file types,
///
#[repr(u8)]
#[derive(PartialEq, PartialOrd, Debug, Default, Clone, Copy)]
pub enum FileType {
    /// Normal file
    #[default]
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
#[derive(Clone, Debug, Default)]
struct HeaderAdapter(Bytes);

impl HeaderAdapter {
    /// Creates a new header adapter,
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
        writeln!(f, "mode:             {:07o}", self.mode())?;
        writeln!(f, "owner_id:         {:07o}", self.owner_id())?;
        writeln!(f, "group_id:         {:07o}", self.group_id())?;
        writeln!(f, "size:             {}", self.size())?;
        writeln!(
            f,
            "last_modified:    {}",
            time::OffsetDateTime::from_unix_timestamp(self.last_modified() as i64).unwrap()
        )?;
        writeln!(f, "checksum:         {:06o}", self.checksum())?;
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

#[cfg(test)]
mod test {
    use std::time::{SystemTime, UNIX_EPOCH};

    use ascii::AsAsciiStr;

    use crate::archive::header::NO_OWNER_NO_GROUP;

    use super::HeaderBuilder;

    #[test]
    fn test_header_builder() {
        let mut builder = HeaderBuilder::default();
        builder.set_name("/".as_ascii_str().unwrap()).unwrap();
        builder.set_file_mode(0o400).unwrap();
        builder.set_size(4096).unwrap();

        let mtime = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        eprintln!("{:11o}", mtime);
        builder.set_last_modified(mtime).unwrap();
        builder
            .set_file_type(crate::archive::FileType::SoftLink)
            .unwrap();
        let header = builder.build().unwrap();
        assert!(header.is_checksum_valid());
    }

    #[test]
    fn test_header_builder_directory() {
        assert!(HeaderBuilder::dir("/".as_ascii_str().unwrap()).is_ok());
        assert!(HeaderBuilder::dir("test".as_ascii_str().unwrap()).is_err());

        let dir = HeaderBuilder::dir("test/".as_ascii_str().unwrap()).unwrap();

        let header = dir.set_defaults_for_archive().build().unwrap();
        assert_eq!("test/", header.name());
        assert_eq!(NO_OWNER_NO_GROUP as u32, header.owner_id());
        assert_eq!(NO_OWNER_NO_GROUP as u32, header.group_id());
        assert_eq!(super::FileType::Directory, header.file_type());
        assert_eq!(0o755, header.mode());
    }

    #[test]
    fn test_header_from_path() {
        let cargo = HeaderBuilder::from_path("src/archive/header.rs")
            .unwrap()
            .build()
            .unwrap();

        eprintln!("{cargo}");
    }
}

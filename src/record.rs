use crate::{
    archive::{self, Entry, HeaderBuilder, Sha256Digest}, util::{Peek, PeekExtensions}, Data, Namespace, Opts
};
use ascii::AsAsciiStr;
use bytes::Bytes;
use crc::{CRC_64_MS, Crc, Digest};
use serde::Deserialize;
use sha2::Sha256;
use std::{
    fmt::Debug,
    io::Error,
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

static CRC: OnceLock<Crc<u64>> = OnceLock::new();

fn crc_digest() -> Digest<'static, u64> {
    CRC.get_or_init(|| Crc::<u64>::new(&CRC_64_MS)).digest()
}

/// IRecord provides an immutable front-end for record consumers and abstracts the inner type
///
/// Implementations must gurantee,
///
/// 1) The edge providing an IRecord must always do so from a source Record
/// 2) An IRecord can be reversed into it's source Record
///
/// WIP
pub trait IRecord {
    /// Key that should be used when indexing a type that implements IRecord
    fn index_key(&self) -> u64 {
        self.uuid().as_u64_pair().0 ^ self.ns_chk()
    }

    /// Namespace::chk value
    fn ns_chk(&self) -> u64;

    /// Record UUID
    fn uuid(&self) -> uuid::Uuid;

    /// Record opts
    fn opts(&self) -> &Opts;

    /// Returns bytes that belong to this record
    fn bytes(&self) -> &[u8];

    /// Returns a clone of the source Record IRecord was created from
    fn to_record(&self) -> Record;

    /// Returns a flexbuffer reader over the flexbuffer root
    ///
    /// Returns None if the current record data does not have a flexbuffer root
    #[inline]
    fn peek<'peek>(&'peek self) -> Option<crate::util::Peek<'peek>> {
        flexbuffers::Reader::get_root(self.bytes())
            .ok()
            .map(Peek::from)
    }

    /// Traverse the record using a statically defined path.
    /// 
    /// # Example
    /// ```rs no_run
    /// let name = record.field("user.profile.name").str();
    /// ```
    ///
    /// # See also
    /// - [`PeekExtensions`] for methods like `.str()`, `.int()`, etc.
    /// - [`IRecord::peek()`] for the base entry point
    #[inline]
    fn field<'peek>(&'peek self, path: &'peek str) -> impl PeekExtensions<'peek> {
        self.peek().at_dot(path)
    }

    /// Returns the content digest buffer for the data stored
    #[inline]
    fn content(&self) -> Sha256Digest {
        <Sha256 as sha2::Digest>::digest(self.bytes()).into()
    }
}

impl IRecord for Record {
    #[inline]
    fn ns_chk(&self) -> u64 {
        self.ns_chk
    }

    #[inline]
    fn uuid(&self) -> uuid::Uuid {
        self.key
    }

    #[inline]
    fn opts(&self) -> &Opts {
        &self.opts
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        self.data.bytes()
    }

    #[inline]
    fn to_record(&self) -> Record {
        self.clone()
    }
}

impl<'b> IRecord for &'b Record {
    fn ns_chk(&self) -> u64 {
        self.ns_chk
    }

    fn uuid(&self) -> uuid::Uuid {
        self.key
    }

    fn opts(&self) -> &Opts {
        &self.opts
    }

    fn bytes(&self) -> &[u8] {
        self.data.bytes()
    }

    fn to_record(&self) -> Record {
        (*self).clone()
    }
}

impl<'b> IRecord for Option<&'b Record> {
    fn ns_chk(&self) -> u64 {
        self.map(|r| r.ns_chk()).unwrap_or_default()
    }

    fn uuid(&self) -> uuid::Uuid {
        self.map(|r| r.uuid()).unwrap_or_else(uuid::Uuid::nil)
    }

    fn opts(&self) -> &Opts {
        self.map(|r| r.opts()).unwrap_or_else(|| crate::EMPTY_OPTS)
    }

    fn bytes(&self) -> &[u8] {
        self.map(|s| s.bytes()).unwrap_or_default()
    }

    fn to_record(&self) -> Record {
        self.cloned().unwrap_or_else(|| Namespace::ephemeral().record(""))
    }
}

pub trait PeekMap: IRecord {
    /// Peeks at the flexbuffer root and return a value
    ///
    /// Returns None if data is not a flexbuffer root
    #[inline]
    fn peek_map<O>(&self, peek: impl Fn(flexbuffers::Reader<&[u8]>) -> O) -> Option<O> {
        self.peek().map(|d| peek((*d).clone()))
    }
}

impl<T: IRecord> PeekMap for T {}

/// State for storing data into the database
///
/// A record is considered an immutable snapshot of state
#[derive(Clone, Debug)]
pub struct Record {
    /// Record key
    ///
    /// The hi-bits are a hash of the label tagging this record,
    /// The lo-bits are a crc checksum of the commited data and ts of this record
    ///
    /// If the lo-bits are zeroed, this means that no data has been committed
    pub(crate) key: uuid::Uuid,
    /// Namespace checksum
    pub(crate) ns_chk: u64,
    /// Bitflag options
    pub(crate) opts: Opts,
    /// Timestamp of when the record was created
    pub(crate) ts: u64,
    /// Data this record is storing
    pub(crate) data: Data,
}

impl Record {
    /// Creates a new record
    #[inline]
    pub fn create(label: &str, ns: impl Into<Namespace>) -> Record {
        let ns = ns.into();
        let key = ns.key(label);
        let opts = ns.opts();
        let ts = time::UtcDateTime::now().unix_timestamp() as u64;
        Record {
            key: Uuid::from_u64_pair(key, 0),
            ns_chk: ns.chk(),
            opts: opts.clone(),
            ts,
            data: Data::Empty,
        }
    }

    /// Returns the age of the record
    #[inline]
    pub fn age(&self) -> Duration {
        SystemTime::now()
            .duration_since(
                UNIX_EPOCH
                    .checked_add(Duration::from_secs(self.ts))
                    .unwrap(),
            )
            .unwrap()
    }

    /// Returns the timestamp/
    #[inline]
    pub fn ts(&self) -> u64 {
        self.ts
    }

    /// Returns the crc checksum
    ///
    /// The checksum is computed as CRC(data | ts)
    #[inline]
    pub fn checksum(&self) -> u64 {
        self.key.as_u64_pair().1
    }

    /// Returns record parts
    #[inline]
    pub fn into_parts(self) -> (Uuid, Data, u64, u64, Opts) {
        (self.key, self.data, self.ns_chk, self.ts, self.opts)
    }

    /// Returns a record composed of parts
    #[inline]
    pub fn from_parts((key, data, ns_chk, ts, opts): (Uuid, Data, u64, u64, Opts)) -> Self {
        Self {
            key,
            data,
            ns_chk,
            opts,
            ts,
        }
    }

    /// Sets the record opts
    #[inline]
    pub fn with_opts(mut self, opts: Opts) -> Self {
        self.opts = opts;
        self
    }

    /// Returns a mutable reference to current record opts
    #[inline]
    pub fn opts_mut(&mut self) -> &mut Opts {
        &mut self.opts
    }

    /// Commit data to the record and configures the Uuid,
    ///
    /// The UUID is composed of two parts hash and checksum
    ///
    /// The checksum is calculated by using CRC(data | ts)
    ///
    /// The CRC algorithm used is CRC_64_MS
    #[inline]
    pub fn commit(mut self, data: Bytes) -> Record {
        let mut crc = crc_digest();
        crc.update(&data);
        crc.update(&self.ts.to_le_bytes());

        self.data = Data::Bytes(data);

        let (hi, _) = self.key.as_u64_pair();
        self.key = Uuid::from_u64_pair(hi, crc.finalize());
        self
    }

    /// Returns true if the record is valid, meaning the stored data,
    /// matches the checksum stored in the key
    ///
    /// Note: Records w/ no data are not considered valid
    #[inline]
    pub fn is_valid(&self) -> bool {
        match &self.data {
            Data::Bytes(bytes) => {
                let mut crc = crc_digest();
                crc.update(&bytes);
                crc.update(&self.ts.to_le_bytes());

                let (_, lo) = self.key.as_u64_pair();
                lo == crc.finalize()
            }
            Data::Virtual(bytes) => {
                let mut crc = crc_digest();
                crc.update(&bytes);
                crc.update(&self.ts.to_le_bytes());

                let (_, lo) = self.key.as_u64_pair();
                lo == crc.finalize()
            }
            _ => false,
        }
    }

    /// Returns true if this record matches the provided label
    #[inline]
    pub fn matches_label(&self, label: &str, namespace: impl Into<Namespace>) -> bool {
        let ns: Namespace = namespace.into();
        let label_key = ns.key(label);

        let (hi, _) = self.key.as_u64_pair();
        hi == label_key
    }

    /// Attempts to deserialize data to some type
    #[inline]
    pub fn load<'de, T: Deserialize<'de> + 'de>(&'de self) -> Option<T> {
        match &self.data {
            Data::Bytes(bytes) if self.is_valid() => flexbuffers::from_slice(&bytes).ok(),
            Data::Virtual(bytes) if self.is_valid() => flexbuffers::from_slice(&bytes).ok(),
            _ => None,
        }
    }

    /// Attempts to deserialize data to some type, skips checking if the data is valid
    #[inline]
    pub fn unchecked_load<'de, T: Deserialize<'de>>(&'de self) -> Option<T> {
        match &self.data {
            Data::Bytes(bytes) => flexbuffers::from_slice(&bytes).ok(),
            Data::Virtual(bytes) => flexbuffers::from_slice(&bytes).ok(),
            _ => None,
        }
    }

    /// Returns an archive header for this record
    #[inline]
    pub fn make_archive_header(&self) -> std::io::Result<archive::Header> {
        let mut header = HeaderBuilder::regular(
            format!(
                "{:x}_{}_{:x}",
                self.ns_chk,
                self.key.as_simple(),
                self.opts.encode()
            )
            .as_ascii_str()
            .map_err(|e| Error::new(std::io::ErrorKind::InvalidFilename, e.to_string()))?,
        )?
        .set_defaults_for_archive();
        header.set_last_modified(self.ts)?;
        header.set_size(self.bytes().len())?;
        header.build()
    }

    /// Creates an archive entry for this record
    ///
    /// The filename of the record's entry is formatted as {ns_chk:x}_{key.as_simple()}_{opts.bits():x},
    /// this filename format is used to restore the record from the archive entry
    #[inline]
    pub fn archive(&self) -> std::io::Result<archive::Entry> {
        if !self.opts().is_archivable() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "This record may not be archived",
            ));
        }

        if self.is_valid() {
            match &self.data {
                Data::Bytes(bytes) => {
                    let header = self.make_archive_header()?;
                    let entry = archive::Entry::regular(header, bytes.clone());
                    return Ok(entry);
                }
                Data::Virtual(bytes) => {
                    let header = self.make_archive_header()?;
                    let entry = archive::Entry::from_virtual(header, bytes.clone());
                    return Ok(entry);
                }
                _ => {}
            }
        }

        Err(Error::new(
            std::io::ErrorKind::InvalidData,
            "Cannot create an archive entry for an invalid record",
        ))
    }

    /// Restores a record from an archive entry
    #[inline]
    pub fn restore(entry: Entry) -> std::io::Result<Self> {
        let header = entry.header();

        if let Some((ns_chk, key, opts)) = header.split_name_for_record() {
            let ts = header.last_modified();
            let record = Record {
                key,
                data: entry
                    .data()
                    .map(|(b, d)| {
                        use sha2::Digest;
                        let digest: [u8; 32] = b.digest().finalize().into();
                        if digest.eq(&d) { b } else { Data::Empty }
                    })
                    .unwrap_or(Data::Empty),
                ns_chk,
                opts,
                ts,
            };

            return if record.is_valid() {
                Ok(record)
            } else {
                Err(Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Data validation failed while attempting to restore record from entry",
                ))
            };
        }

        Err(Error::new(
            std::io::ErrorKind::InvalidFilename,
            "File name was not in the expected record archive format",
        ))
    }

    /// Returns true if the backing data is virtual
    #[inline]
    pub fn is_virtual(&self) -> bool {
        matches!(self.data, Data::Virtual(..))
    }
}

#[cfg(test)]
mod test {
    use super::Record;
    use crate::{Data, Opts, RecordableExtensions, record::Namespace};
    use bytes::Bytes;
    use serde::{Deserialize, Serialize};
    use std::{collections::BTreeMap, time::Duration};
    use uuid::Uuid;

    #[test]
    fn test_record_is_valid_false_when_empty() {
        let record = Record::create("empty / record", ());
        assert!(!record.is_valid())
    }

    #[test]
    fn test_record_is_valid_true_after_commit() {
        let record =
            Record::create("notempty / record", ()).commit(Bytes::from_static(b"hello world"));
        assert!(record.is_valid())
    }

    #[test]
    fn test_record_is_labeled_true() {
        let record = Record::create("some / record", ());
        assert!(record.matches_label("some / record", ()));
    }

    #[test]
    fn test_record_is_labeled_true_with_namespace() {
        let ns = super::Namespace::new("io.runir.default");
        let record = Record::create("some / record", ns.clone());
        assert!(record.matches_label("some / record", ns));
    }

    #[test]
    fn test_ephemeral_namespaces() {
        let ns1 = super::Namespace::ephemeral();
        let ns2 = super::Namespace::ephemeral();
        let record = Record::create("some / record", ns1);

        assert!(!record.matches_label("some / record", ns2));
    }

    #[test]
    fn test_namespace_record_is_labeled() {
        let ns1 = super::Namespace::ephemeral();

        let record = ns1
            .record("some / record")
            .commit(Bytes::from_static(b"hello world"));

        assert!(record.matches_label("some / record", ns1));
        assert!(record.is_valid());

        let ns2 = super::Namespace::ephemeral();
        assert!(!record.matches_label("some / record", ns2));
    }

    #[test]
    fn test_record_from_parts_is_invalid_with_random_parts() {
        let record = Record::from_parts((
            Uuid::nil(),
            Data::Bytes(Bytes::from_static(b"gibberish")),
            0,
            0,
            Opts::default(),
        ));
        assert!(!record.is_valid())
    }

    #[test]
    fn test_record_from_parts_is_valid() {
        let parts = Namespace::ephemeral()
            .record("test")
            .commit(Bytes::from_static(b"hello world"))
            .into_parts();
        assert!(Record::from_parts(parts).is_valid())
    }

    #[test]
    fn test_namespace_from_str() {
        let record = Record::create("some / record", "example ns")
            .commit(Bytes::from_static(b"hello world"));
        assert!(record.is_valid())
    }

    #[test]
    fn test_namespace_repro() {
        let ns = Namespace::from("hello");
        let ns2 = Namespace::from("hello");

        assert_eq!(ns.chk(), ns2.chk());
    }

    #[derive(Serialize, Deserialize)]
    struct TestObj<'a> {
        name: &'a str,
        field_a: usize,
        field_b: bool,
        field_map: BTreeMap<&'a str, &'a str>,
    }

    #[test]
    fn test_namespace_save() {
        let mut field_map = BTreeMap::new();
        field_map.insert("hello", "world");
        field_map.insert("world", "goodbye");

        let test = TestObj {
            name: "my-test-obj",
            field_a: 123456,
            field_b: true,
            field_map,
        };

        let ns = Namespace::ephemeral();

        let record = ns.store("my-test-obj", &test);
        assert!(record.is_valid());

        let loaded = record.load::<TestObj>().unwrap();
        assert_eq!("my-test-obj", loaded.name);
        assert_eq!(123456, loaded.field_a);
        assert_eq!(true, loaded.field_b);
        assert_eq!("world", loaded.field_map["hello"]);
        assert_eq!("goodbye", loaded.field_map["world"]);

        assert!(record.age() < Duration::from_secs(1));

        let loaded = record.unchecked_load::<TestObj>().unwrap();
        assert_eq!("my-test-obj", loaded.name);
        assert_eq!(123456, loaded.field_a);
        assert_eq!(true, loaded.field_b);
        assert_eq!("world", loaded.field_map["hello"]);
        assert_eq!("goodbye", loaded.field_map["world"]);
    }

    #[test]
    fn test_archive_restore() {
        let mut field_map = BTreeMap::new();
        field_map.insert("hello", "world");
        field_map.insert("world", "goodbye");

        let test = TestObj {
            name: "my-test-obj",
            field_a: 123456,
            field_b: true,
            field_map,
        };

        let ns = Namespace::ephemeral();
        let mut record = ns.store("my-test-obj", test.indexable());
        assert!(record.archive().is_err());

        record.opts_mut().enable_archiving();

        let key = record.key.clone();
        let archive = record.archive().unwrap();
        assert!(archive.header().name().ends_with("1"));
        let record = Record::restore(archive).unwrap();
        assert_eq!(key, record.key);
    }

    #[test]
    fn test_author_root() {
        let namespace = Namespace::ephemeral();

        let record = namespace.author("example", |mut b| {
            let mut map = b.start_map();
            map.push("value", "hello world");
            map.end_map();
            b
        });

        let value = record.load::<toml::Value>().unwrap();
        assert_eq!("hello world", value["value"].as_str().unwrap());
    }
}

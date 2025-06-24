use crate::{
    archive::{self, Entry, HeaderBuilder}, Data, Opts, RawRecordable
};
use ahash::RandomState;
use ascii::AsAsciiStr;
use bytes::Bytes;
use crc::{CRC_64_MS, Crc, Digest};
use serde::{Deserialize, Serialize};
use std::{
    hash::Hash,
    io::Error,
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

static CRC: OnceLock<Crc<u64>> = OnceLock::new();

fn crc_digest() -> Digest<'static, u64> {
    CRC.get_or_init(|| Crc::<u64>::new(&CRC_64_MS)).digest()
}

/// Namespace provides a hasher for the record
///
/// If created via str, the namespace will be deterministic, and records saved via
/// this namespace may be archived/restored with deterministic symbols
///
/// Otherwise, the namespace is treated as ephemeral and will only be valid during the lifetime of
/// the process
#[derive(Clone)]
pub struct Namespace {
    /// Hashing core of the namespace
    hasher_core: ahash::RandomState,
    /// Default record options
    opts: Opts,
}

impl Hash for Namespace {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.chk().hash(state);
    }
}

impl PartialEq for Namespace {
    fn eq(&self, other: &Self) -> bool {
        self.chk() == other.chk()
    }
}

impl Namespace {
    /// Derives a new namespace from a namespace label
    ///
    /// Note: This function re-uses ahash in order to have persistent keys,
    /// however these hashes are not intended to be DOS-resistant. That layer
    /// of hashing is handled in the Index code which should be servicing the majority
    /// of hash-based lookups
    #[inline]
    pub fn new(namespace: &str) -> Namespace {
        let init_hash = ahash::RandomState::with_seeds(1, 0, 0, 0);
        let k1 = init_hash.hash_one(namespace);

        let init_hash = ahash::RandomState::with_seeds(0, 1, 0, 0);
        let k2 = init_hash.hash_one(namespace);

        let init_hash = ahash::RandomState::with_seeds(0, 0, 1, 0);
        let k3 = init_hash.hash_one(namespace);

        let init_hash = ahash::RandomState::with_seeds(0, 0, 0, 1);
        let k4 = init_hash.hash_one(namespace);

        Namespace {
            hasher_core: ahash::RandomState::with_seeds(k1, k2, k3, k4),
            opts: Opts::default(),
        }
    }

    /// Returns an ephemeral namespace
    #[inline]
    pub fn ephemeral() -> Namespace {
        Namespace {
            hasher_core: ahash::RandomState::default(),
            opts: Opts::ephemeral(),
        }
    }

    /// Returns a new empty record under this namespace
    #[inline]
    pub fn record(&self, label: &str) -> Record {
        Record::create(label, self.clone()).with_opts(self.opts)
    }

    /// Returns the key value for a label under this namespace
    #[inline]
    pub fn key(&self, label: &str) -> u64 {
        self.hasher_core.hash_one(label)
    }

    /// Returns the checksum value for the namespace
    #[inline]
    pub fn chk(&self) -> u64 {
        self.hasher_core.hash_one(self.opts)
    }

    /// Enables the indexing record option by default for all records,
    /// created from this namespace.
    #[inline]
    pub fn enable_indexing(&mut self) -> &mut Self {
        self.opts.enable_indexing();
        self
    }

    /// Authors a record under this namespace for an obj
    ///
    /// Note: If the object was unable to be saved, it will return an empty record,
    /// empty records are not considered valid, therefore the Worker will return false if a record
    /// was saved from a worker
    ///
    /// Reminder: Namespace maintains no state, this purely authors a record
    #[inline]
    pub fn store<'a, T: Serialize + 'a>(
        &self,
        label: &str,
        recordable: impl Into<RawRecordable<'a, T>>,
    ) -> Record {
        let recordable = recordable.into();
        let mut ser = flexbuffers::FlexbufferSerializer::new();
        let record = self.record(label);
        if let Ok(()) = recordable.serialize(&mut ser) {
            let mut record = record
                .with_opts(self.opts | recordable.opts)
                .commit(Bytes::from(ser.take_buffer()));

            record.opts_mut().set_serialized_object();
            record
        } else {
            record
        }
    }
}

impl From<()> for Namespace {
    fn from(_: ()) -> Self {
        Namespace {
            // This means this namespace will be static within the same process
            hasher_core: RandomState::with_seed(0),
            opts: Opts::default(),
        }
    }
}

impl From<&str> for Namespace {
    fn from(value: &str) -> Self {
        Namespace::new(value)
    }
}

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
    key: uuid::Uuid,
    /// Namespace checksum
    ns_chk: u64,
    /// Bitflag options
    opts: Opts,
    /// Timestamp of when the record was created
    ts: u64,
    /// Data this record is storing
    data: Data,
}

impl Record {
    /// Creates a new record
    #[inline]
    pub fn create(label: &str, ns: impl Into<Namespace>) -> Record {
        let ns = ns.into();
        let key = ns.key(label);
        let ts = time::UtcDateTime::now().unix_timestamp() as u64;
        Record {
            key: Uuid::from_u64_pair(key, 0),
            ns_chk: ns.chk(),
            opts: Opts::default(),
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

    /// Returns the current record uuid
    #[inline]
    pub fn uuid(&self) -> uuid::Uuid {
        self.key
    }

    /// Returns a reference to the data stored in this record
    #[inline]
    pub fn data(&self) -> &Data {
        &self.data
    }

    /// Returns the namespace checksum value
    #[inline]
    pub fn ns_chk(&self) -> u64 {
        self.ns_chk
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

    /// Returns current record opts
    #[inline]
    pub fn opts(&self) -> &Opts {
        &self.opts
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
    pub fn load<'de, T: Deserialize<'de>>(&'de self) -> Option<T> {
        match &self.data {
            Data::Bytes(bytes) if self.is_valid() => flexbuffers::from_slice(&bytes).ok(),
            _ => None,
        }
    }

    /// Attempts to deserialize data to some type, skips checking if the data is valid
    #[inline]
    pub fn unchecked_load<'de, T: Deserialize<'de>>(&'de self) -> Option<T> {
        match &self.data {
            Data::Bytes(bytes) => flexbuffers::from_slice(&bytes).ok(),
            _ => None,
        }
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
                    let mut header = HeaderBuilder::regular(
                        format!(
                            "{:x}_{}_{:x}",
                            self.ns_chk,
                            self.key.as_simple(),
                            self.opts.encode()
                        )
                        .as_ascii_str()
                        .map_err(|e| {
                            Error::new(std::io::ErrorKind::InvalidFilename, e.to_string())
                        })?,
                    )?
                    .set_defaults_for_archive();

                    header.set_last_modified(self.ts)?;
                    header.set_size(bytes.len())?;

                    let entry = archive::Entry::regular(header.build()?, bytes.clone());
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
                        let digest = sha2::Sha256::digest(&b);
                        let digest: [u8; 32] = digest.into();
                        if digest.eq(&d) {
                            Data::Bytes(b)
                        } else {
                            Data::Empty
                        }
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
}

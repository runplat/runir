use crate::{
    archive::{self, Entry, HeaderBuilder}, RecordOpts, Recordable
};
use ascii::AsAsciiStr;
use bytes::Bytes;
use crc::{CRC_64_MS, Crc, Digest};
use serde::{Deserialize, Serialize};
use std::{
    hash::Hash,
    io::Error,
    str::FromStr,
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
    opts: RecordOpts,
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
            opts: RecordOpts::empty(),
        }
    }

    /// Returns an ephemeral namespace
    #[inline]
    pub fn ephemeral() -> Namespace {
        Namespace {
            hasher_core: ahash::RandomState::default(),
            opts: RecordOpts::NoArchive,
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
        self.hasher_core.hash_one(self.opts.bits())
    }

    /// Enables the indexing record option by default for all records,
    /// created from this namespace.
    #[inline]
    pub fn enable_indexing(&mut self) -> &mut Self {
        self.opts |= RecordOpts::Indexing;
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
    pub fn save<'a, T: Serialize + 'a>(
        &self,
        label: &str,
        recordable: impl Into<Recordable<'a, T, 0>>,
    ) -> Record {
        let recordable = recordable.into();
        let mut ser = flexbuffers::FlexbufferSerializer::new();
        let record = self.record(label);
        if let Ok(()) = recordable.serialize(&mut ser) {
            record
                .with_opts(self.opts | recordable.opts)
                .commit(Bytes::from(ser.take_buffer()))
        } else {
            record
        }
    }
}

impl From<()> for Namespace {
    fn from(_: ()) -> Self {
        Namespace::ephemeral()
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
    /// Timestamp of when record was created
    ts: u64,
    /// Bitflag options
    opts: RecordOpts,
    /// Data this record is storing
    data: Option<Bytes>,
}

impl Record {
    /// Creates a new record
    #[inline]
    pub fn create(label: &str, ns: impl Into<Namespace>) -> Record {
        let ns = ns.into();
        let key = ns.key(label);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Record {
            key: Uuid::from_u64_pair(key, 0),
            data: None,
            ns_chk: ns.chk(),
            opts: RecordOpts::empty(),
            ts,
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
    pub fn data(&self) -> Option<&Bytes> {
        self.data.as_ref()
    }

    /// Returns the namespace checksum value
    #[inline]
    pub fn ns_chk(&self) -> u64 {
        self.ns_chk
    }

    /// Returns record parts
    #[inline]
    pub fn into_parts(self) -> (Uuid, Option<Bytes>, u64, u64, RecordOpts) {
        (self.key, self.data, self.ns_chk, self.ts, self.opts)
    }

    /// Returns a record composed of parts
    #[inline]
    pub fn from_parts(
        (key, data, ns_chk, ts, opts): (Uuid, Option<Bytes>, u64, u64, RecordOpts),
    ) -> Self {
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
    pub fn with_opts(mut self, opts: RecordOpts) -> Self {
        self.opts = opts;
        self
    }

    /// Checks current record opts state
    #[inline]
    pub fn enabled(&self, opt: RecordOpts) -> bool {
        self.opts.contains(opt)
    }

    /// Enables or disables indexing
    #[inline]
    pub fn indexing(&mut self, enabled: bool) {
        if enabled {
            self.opts |= RecordOpts::Indexing;
        } else {
            self.opts &= !RecordOpts::Indexing;
        }
    }

    /// Enables or disables archiving
    /// 
    /// WARNING: If the record was created under an ephemeral namespace, than
    /// restoring the archive of this record will create an un-resolvable label key.
    /// 
    /// Use with caution
    #[inline]
    pub fn archiving(&mut self, enabled: bool) {
        if enabled {
            self.opts &= !RecordOpts::NoArchive;
        } else {
            self.opts |= RecordOpts::NoArchive;
        }
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

        let _ = self.data.insert(data);

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
        self.data
            .as_ref()
            .map(|d| {
                let mut crc = crc_digest();
                crc.update(d);
                crc.update(&self.ts.to_le_bytes());

                let (_, lo) = self.key.as_u64_pair();
                lo == crc.finalize()
            })
            .unwrap_or_default()
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
        self.data
            .as_ref()
            .filter(|_| self.is_valid())
            .and_then(|d| flexbuffers::from_slice(&d).ok())
    }

    /// Attempts to deserialize data to some type, skips checking if the data is valid
    #[inline]
    pub fn unchecked_load<'de, T: Deserialize<'de>>(&'de self) -> Option<T> {
        self.data
            .as_ref()
            .and_then(|d| flexbuffers::from_slice(&d).ok())
    }

    /// Creates an archive entry for this record
    ///
    /// The filename of the record's entry is formatted as {ns_chk:x}_{key.as_simple()}_{opts.bits():x},
    /// this filename format is used to restore the record from the archive entry
    #[inline]
    pub fn archive(&self) -> std::io::Result<archive::Entry> {
        if self.enabled(RecordOpts::NoArchive) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "This record may not be archived",
            ));
        }

        if self.is_valid() {
            if let Some(data) = self.data.as_ref() {
                let mut header = HeaderBuilder::regular(
                    format!(
                        "{:x}_{}_{:x}",
                        self.ns_chk,
                        self.key.as_simple(),
                        self.opts.bits()
                    )
                    .as_ascii_str()
                    .map_err(|e| Error::new(std::io::ErrorKind::InvalidFilename, e.to_string()))?,
                )?
                .set_defaults_for_archive();

                header.set_last_modified(self.ts)?;
                header.set_size(data.len())?;

                let entry = archive::Entry::regular(header.build()?, data.clone());
                return Ok(entry);
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

        if let Some((ns_chk, (key, opts))) =
            header
                .name()
                .split_once("_")
                .and_then(|(nschk, uuid_opts)| {
                    let nschk = u64::from_str_radix(nschk, 16).ok();
                    let uuid_opts = uuid_opts.split_once("_").and_then(|(uuid, opts)| {
                        uuid::Uuid::from_str(uuid).ok().zip(
                            u8::from_str_radix(opts, 16)
                                .ok()
                                .and_then(|b| RecordOpts::from_bits(b)),
                        )
                    });

                    nschk.zip(uuid_opts)
                })
        {
            let ts = header.last_modified();
            let record = Record {
                key,
                data: entry.data().and_then(|(b, d)| {
                    use sha2::Digest;
                    let digest = sha2::Sha256::digest(&b);
                    let digest: [u8; 32] = digest.into();
                    Some(b).filter(|_| digest.eq(&d))
                }),
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
    use crate::{RecordOpts, RecordableExtensions, record::Namespace};
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
            Some(Bytes::from_static(b"gibberish")),
            0,
            0,
            RecordOpts::empty(),
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

        let record = ns.save("my-test-obj", &test);
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
        let record = ns.save("my-test-obj", test.indexable());

        let key = record.key.clone();
        let archive = record.archive().unwrap();
        assert!(archive.header().name().ends_with("1"));
        let record = Record::restore(archive).unwrap();
        assert_eq!(key, record.key);
    }
}

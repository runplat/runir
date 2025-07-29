use crate::{
    Data, Namespace, Opts,
    archive::{self, Entry, HeaderBuilder, Sha256Digest},
    opts::Branch,
    util::{Peek, PeekExtensions},
};
use ascii::AsAsciiStr;
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

    /// Returns a mutable reference to the current record opts.
    ///
    /// Returns None if the record does not support option mutation
    ///
    /// Note: If the record is empty (has no data), changes to storage options (e.g. `Object`)
    /// are not enforced until the record is committed. Validation and interpretation of
    /// storage options apply only after data is present.
    fn opts_mut(&mut self) -> Option<&mut Opts>;

    /// Returns bytes that belong to this record
    fn bytes(&self) -> &[u8];

    /// Returns a clone of the source Record IRecord was created from
    fn to_record(&self) -> Record;

    /// Returns a flexbuffer reader over the flexbuffer root
    ///
    /// Returns None if the current record data does not have a flexbuffer root
    #[inline]
    fn peek<'peek>(&'peek self) -> impl PeekExtensions<'peek> {
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

    /// Traverse the record w/ a list of statically defined paths
    /// 
    /// Returns a vector of accessors for each path
    #[inline]
    fn fields<'peek>(&'peek self, paths: &[&'peek str]) -> Vec<Option<Peek<'peek>>> {
        paths.iter().fold(vec![], |mut a, p| {
            a.push(self.field(p).val());
            a
        })
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
        &self.data
    }

    #[inline]
    fn to_record(&self) -> Record {
        self.clone()
    }

    #[inline]
    fn opts_mut(&mut self) -> Option<&mut Opts> {
        Some(&mut self.opts)
    }
}

impl<'b> IRecord for &'b Record {
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
        &self.data
    }

    #[inline]
    fn to_record(&self) -> Record {
        (*self).clone()
    }

    #[inline]
    fn opts_mut(&mut self) -> Option<&mut Opts> {
        None
    }
}

impl<'b> IRecord for Option<&'b Record> {
    #[inline]
    fn ns_chk(&self) -> u64 {
        self.map(|r| r.ns_chk()).unwrap_or_default()
    }

    #[inline]
    fn uuid(&self) -> uuid::Uuid {
        self.map(|r| r.uuid()).unwrap_or_else(uuid::Uuid::nil)
    }

    #[inline]
    fn opts(&self) -> &Opts {
        self.map(|r| r.opts()).unwrap_or_else(|| crate::EMPTY_OPTS)
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        self.map(|s| s.bytes()).unwrap_or_default()
    }

    #[inline]
    fn to_record(&self) -> Record {
        self.cloned()
            .unwrap_or_else(|| Namespace::ephemeral().record(""))
    }

    #[inline]
    fn opts_mut(&mut self) -> Option<&mut Opts> {
        None
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
            data: Data::default(),
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
    ///
    /// Note: If the record is empty (has no data), changes to storage options (e.g. `Object`)
    /// are not enforced until the record is committed. Validation and interpretation of
    /// storage options apply only after data is present.
    #[inline]
    pub fn with_opts(mut self, opts: Opts) -> Self {
        self.opts = opts;
        self
    }

    /// Commit data to the record and configures the Uuid,
    ///
    /// The UUID is composed of two parts hash and checksum
    ///
    /// The checksum is calculated by using CRC(data | ts)
    ///
    /// The CRC algorithm used is CRC_64_MS
    #[inline]
    pub fn commit(mut self, data: impl Into<Data>) -> Record {
        self.data = data.into();

        let mut crc = crc_digest();
        crc.update(self.bytes());
        crc.update(&self.ts.to_le_bytes());

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
        if self.data.is_empty() {
            return false;
        }

        let mut crc = crc_digest();
        crc.update(&self.data);
        crc.update(&self.ts.to_le_bytes());

        let (_, lo) = self.key.as_u64_pair();
        lo == crc.finalize()
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
        if self.is_valid() && self.opts().is_object() {
            flexbuffers::from_slice(&self.data).ok()
        } else {
            None
        }
    }

    /// Attempts to deserialize data to some type, skips checking if the data is valid
    #[inline]
    pub fn unchecked_load<'de, T: Deserialize<'de>>(&'de self) -> Option<T> {
        flexbuffers::from_slice(&self.data).ok()
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
            let header = self.make_archive_header()?;
            let entry = archive::Entry::regular(header, self.data.as_bytes().clone());
            return Ok(entry);
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
                        if digest.eq(&d) { b } else { Data::default() }
                    })
                    .unwrap_or_default(),
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

    /// Creates a staged version of the record with new data.
    ///
    /// This function behaves differently based on the current branch state:
    ///
    /// - If the record has no data, the new data is committed as `HEAD`.
    /// - If the record is in `HEAD` or `STAGING`, a new `STAGING` version is created with the given data.
    /// - If the record is marked `DELETED`, staging is not allowed and an error is returned.
    ///
    /// This operation does not mutate the original record — it returns a new version that must be
    /// committed or processed by a compatible store to take effect.
    #[inline]
    #[must_use = "Calling `.stage()` prepares a staged record, but it must be committed or passed to a store to take effect"]
    pub fn stage(&self, data: impl Into<Data>) -> std::io::Result<Self> {
        if self.bytes().is_empty() {
            Ok(self.clone().commit(data))
        } else if self.opts().is_deleted() {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Cannot stage data for a record marked for deletion",
            ))
        } else if self.opts().is_idempotent() || self.opts().is_staging() {
            let data: Data = data.into();

            if self.opts().is_object() && flexbuffers::Reader::get_root(data.as_ref()).is_err() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Current record stores an Object, staged data must also be an object",
                ));
            }

            let mut staging = self.clone();
            staging
                .opts_mut()
                .expect("should always be able to mutate options from a full record")
                .enable_branch(Branch::Staging);

            // Since we are about to commit new data, we need to create a new timestamp
            staging.ts = time::UtcDateTime::now().unix_timestamp() as u64;

            Ok(staging.commit(data))
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Record is in an unknown branch configuration",
            ))
        }
    }

    /// Returns a version of the current record marked for deletion.
    ///
    /// This sets the `DELETED` branch flag but does not remove the underlying data.
    /// Persistence and deletion behavior depend on the store or archival system.
    #[inline]
    #[must_use = "Calling `.delete()` marks the record, but it must be committed or passed to a store to take effect"]
    pub fn delete(&self) -> Self {
        let mut deleting = self.clone();
        deleting
            .opts_mut()
            .expect("should always be able to mutate options from a full record")
            .enable_branch(Branch::Deleted);
        deleting
    }
}

/// Returns the record crc value
#[inline]
pub fn record_crc(bytes: &[u8], ts: u64) -> u64 {
    let mut crc = crc_digest();
    crc.update(bytes);
    crc.update(&ts.to_le_bytes());
    crc.finalize()
}

#[cfg(test)]
mod test {
    use super::Record;
    use crate::{record::Namespace, util::PeekExtensions, IRecord, Opts, RecordableExtensions};
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
            Bytes::from_static(b"gibberish").into(),
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
    fn test_fields() {
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

        let fields = record.fields(&["name", "field_a", "field_b", "field_map"]);
        
        if let [name, field_a, field_b, field_map, ..] = fields.as_slice() {
            assert_eq!("my-test-obj", name.str().unwrap());
            assert_eq!(123456, field_a.int().unwrap());
            assert!(field_b.bool().unwrap_or_default());

            assert_eq!("world", field_map.at("hello").str().unwrap());
            assert_eq!("goodbye", field_map.at("world").str().unwrap());
        } else {
            assert!(false, "expecting at least 4 values");
        }
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

        record
            .opts_mut()
            .expect("should always be able to mutate options from a full record")
            .enable_archiving();

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

    #[test]
    fn test_stage() {
        let namespace = Namespace::ephemeral();

        let record = namespace.commit("example", Bytes::from_static(b"hello"));
        let staged = record.stage(Bytes::from_static(b"world")).unwrap();
        assert!(staged.opts().is_staging());

        let record = namespace.record("example");
        let staged = record.stage(Bytes::from_static(b"hello")).unwrap();
        assert!(
            !staged.opts().is_staging(),
            "Since the record was empty to begin-with, this should bypass the STAGING branch"
        );

        let deleted = staged.delete();
        assert!(
            deleted.stage(Bytes::from_static(b"world")).is_err(),
            "A deleted record cannot stage data"
        );

        let staged = staged.stage(Bytes::from_static(b"hello world")).unwrap();
        assert!(staged.is_valid());
        assert!(staged.opts().is_staging());
    }
}

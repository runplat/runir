use std::ops::Deref;

use dashmap::mapref::{multiple::RefMulti, one::Ref};
use futures::Stream;

use crate::IRecord;

/// Result of putting a record into storage
pub struct PutResult<R> {
    /// Key that when passed to Storage::get(..), returns the inserted result
    pub key: u64,
    /// If inserting at the InsertResult.key would have replaced a Record, returns
    /// the previous Record
    ///
    /// Note: Merge policies are enforced at the frontend
    pub previous: Option<R>,
}

/// Trait that abstracts the backend for an index to allow for different
/// backends to be plugged into the index
pub trait Storage: Default {
    /// Record being stored
    type Record: crate::IRecord;

    /// Type returned from a borrow
    type Borrow<'b>: crate::IRecord + Deref<Target = Self::Record>
    where
        Self: 'b,
        Self::Record: 'b;

    /// Type returned from an iterator borrow
    type IterBorrow<'b>: crate::IRecord + Deref<Target = Self::Record>
    where
        Self: 'b,
        Self::Record: 'b;

    /// Put a record into storage
    ///
    /// Returns an InsertResult w/ the key that can be used w/ Storage::get to retrieve the record
    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record>;

    /// Returns the stored record
    ///
    /// Returns None if the key was not recognized by Storage
    fn record<'a: 'b, 'b>(&'a self, key: u64) -> Option<Self::Borrow<'b>>;

    /// Returns an iterator over all records in storage
    fn iter_records<'a: 'b, 'b>(&'a self)
    -> impl Iterator<Item = Self::IterBorrow<'b>> + Send + 'b;

    /// Returns a stream over all records in storage
    fn stream_records<'a: 'b, 'b>(&'a self)
    -> impl Stream<Item = Self::IterBorrow<'b>> + Send + 'b;

    /// Returns true if a record was replaced at key
    fn replace(&mut self, key: u64, record: Self::Record) -> bool;
}

impl<R: crate::IRecord + Sync> Storage for ahash::HashMap<u64, R>
where
    for<'b> &'b R: IRecord,
{
    type Record = R;

    type Borrow<'b>
        = &'b Self::Record
    where
        R: 'b;

    type IterBorrow<'b>
        = &'b Self::Record
    where
        R: 'b;

    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record> {
        let key = record.index_key();
        let previous = self.insert(key, record);

        PutResult { key, previous }
    }

    fn record<'a: 'b, 'b>(&'a self, key: u64) -> Option<Self::Borrow<'b>> {
        self.get(&key)
    }

    fn iter_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Iterator<Item = Self::IterBorrow<'b>> + Send + 'b {
        self.iter().map(|(_, v)| v)
    }

    fn stream_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Stream<Item = Self::IterBorrow<'b>> + Send + 'b {
        futures::stream::iter(self.iter_records())
    }

    fn replace(&mut self, key: u64, record: Self::Record) -> bool {
        self.insert(key, record).is_some()
    }
}

impl<R: crate::IRecord + Sync> Storage for Vec<R>
where
    for<'b> &'b R: IRecord,
{
    type Record = R;

    type Borrow<'b>
        = &'b Self::Record
    where
        Self: 'b,
        Self::Record: 'b;

    type IterBorrow<'b>
        = &'b Self::Record
    where
        R: 'b;

    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record> {
        let key = self.len() as u64;
        self.push(record);

        PutResult {
            key,
            previous: None,
        }
    }

    fn record<'a: 'b, 'b>(&'a self, key: u64) -> Option<Self::Borrow<'b>> {
        self.get(key as usize)
    }

    fn iter_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Iterator<Item = Self::IterBorrow<'b>> + Send + 'b {
        self.iter()
    }

    fn stream_records<'a: 'b, 'b>(&'a self) -> impl Stream<Item = Self::IterBorrow<'b>> + Send {
        futures::stream::iter(self.iter_records())
    }

    fn replace(&mut self, key: u64, record: Self::Record) -> bool {
        if self.len() < key as usize {
            return false;
        }
        self[key as usize] = record;
        true
    }
}

impl<R: crate::IRecord + Send + Sync> Storage for dashmap::DashMap<u64, R> {
    type Record = R;

    type Borrow<'b>
        = Ref<'b, u64, Self::Record>
    where
        Self::Record: 'b;

    type IterBorrow<'b>
        = RefMulti<'b, u64, Self::Record>
    where
        R: 'b;

    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record> {
        let key = record.index_key();
        let previous = self.insert(key, record);

        PutResult { key, previous }
    }

    fn record<'a: 'b, 'b>(&'a self, key: u64) -> Option<Self::Borrow<'b>> {
        self.get(&key)
    }

    fn iter_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Iterator<Item = Self::IterBorrow<'b>> + Send + 'b {
        self.iter()
    }

    fn stream_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Stream<Item = Self::IterBorrow<'b>> + Send + 'b {
        futures::stream::iter(self.iter_records())
    }

    fn replace(&mut self, key: u64, record: Self::Record) -> bool {
        self.insert(key, record).is_some()
    }
}

impl<R: IRecord> IRecord for dashmap::mapref::one::Ref<'_, u64, R> {
    fn ns_chk(&self) -> u64 {
        self.deref().ns_chk()
    }

    fn uuid(&self) -> uuid::Uuid {
        self.deref().uuid()
    }

    fn opts(&self) -> &crate::Opts {
        self.deref().opts()
    }

    fn bytes(&self) -> &[u8] {
        self.deref().bytes()
    }

    fn to_record(&self) -> crate::Record {
        self.deref().to_record()
    }
}

impl<R: IRecord> IRecord for dashmap::mapref::multiple::RefMulti<'_, u64, R> {
    fn ns_chk(&self) -> u64 {
        self.deref().ns_chk()
    }

    fn uuid(&self) -> uuid::Uuid {
        self.deref().uuid()
    }

    fn opts(&self) -> &crate::Opts {
        self.deref().opts()
    }

    fn bytes(&self) -> &[u8] {
        self.deref().bytes()
    }

    fn to_record(&self) -> crate::Record {
        self.deref().to_record()
    }
}

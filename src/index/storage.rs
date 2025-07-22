use std::ops::{Deref, DerefMut};

use dashmap::mapref::{multiple::RefMulti, one::{Ref, RefMut}};
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

    /// Type returned from a borrow
    type BorrowMut<'b>: DerefMut<Target = Self::Record>
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

    /// Returns a mutable reference to a record
    ///
    /// Returns None if the key was not recognized by storage
    fn record_mut<'a: 'b, 'b>(&'a mut self, key: u64) -> Option<Self::BorrowMut<'b>>;

    /// Returns an iterator over all records in storage
    fn iter_records<'a: 'b, 'b>(&'a self)
    -> impl Iterator<Item = Self::IterBorrow<'b>> + Send + 'b;

    /// Returns a stream over all records in storage
    fn stream_records<'a: 'b, 'b>(&'a self)
    -> impl Stream<Item = Self::IterBorrow<'b>> + Send + 'b;

    /// Returns true if a record was replaced at key
    fn replace(&mut self, key: u64, record: Self::Record) -> Option<Self::Record>;

    /// Returns a reverse map of the current storage state
    fn reverse_map(&self) -> ahash::HashMap<u64, u64>;
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

    type BorrowMut<'b>
        = &'b mut Self::Record
    where
        Self: 'b,
        Self::Record: 'b;

    type IterBorrow<'b>
        = &'b Self::Record
    where
        R: 'b;

    #[inline]
    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record> {
        let key = record.index_key();
        let previous = self.insert(key, record);

        PutResult { key, previous }
    }

    #[inline]
    fn record<'a: 'b, 'b>(&'a self, key: u64) -> Option<Self::Borrow<'b>> {
        self.get(&key)
    }

    #[inline]
    fn iter_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Iterator<Item = Self::IterBorrow<'b>> + Send + 'b {
        self.iter().map(|(_, v)| v)
    }

    #[inline]
    fn stream_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Stream<Item = Self::IterBorrow<'b>> + Send + 'b {
        futures::stream::iter(self.iter_records())
    }

    #[inline]
    fn replace(&mut self, key: u64, record: Self::Record) -> Option<Self::Record> {
        self.insert(key, record)
    }

    #[inline]
    fn record_mut<'a: 'b, 'b>(&'a mut self, key: u64) -> Option<Self::BorrowMut<'b>> {
        self.get_mut(&key)
    }
    
    #[inline]
    fn reverse_map(&self) -> ahash::HashMap<u64, u64> {
        self.iter().fold(Default::default(), |mut a, (k, r)| {
            a.insert(r.index_key(), *k);
            a
        })
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

    type BorrowMut<'b>
        = &'b mut Self::Record
    where
        Self: 'b,
        Self::Record: 'b;

    type IterBorrow<'b>
        = &'b Self::Record
    where
        R: 'b;

    #[inline]
    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record> {
        let key = self.len() as u64;
        self.push(record);

        PutResult {
            key,
            previous: None,
        }
    }

    #[inline]
    fn record<'a: 'b, 'b>(&'a self, key: u64) -> Option<Self::Borrow<'b>> {
        self.get(key as usize)
    }

    #[inline]
    fn iter_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Iterator<Item = Self::IterBorrow<'b>> + Send + 'b {
        self.iter()
    }

    #[inline]
    fn stream_records<'a: 'b, 'b>(&'a self) -> impl Stream<Item = Self::IterBorrow<'b>> + Send {
        futures::stream::iter(self.iter_records())
    }

    #[inline]
    fn replace(&mut self, key: u64, record: Self::Record) -> Option<Self::Record> {
        if self.len() < key as usize {
            return None;
        }

        let entry = &mut self[key as usize];
        let previous = std::mem::replace(entry, record);
        Some(previous)
    }

    #[inline]
    fn record_mut<'a: 'b, 'b>(&'a mut self, key: u64) -> Option<Self::BorrowMut<'b>> {
        self.get_mut(key as usize)
    }
    
    #[inline]
    fn reverse_map(&self) -> ahash::HashMap<u64, u64> {
        self.iter().enumerate().fold(Default::default(), |mut a, (k, r)| {
            a.insert(r.index_key(), k as u64);
            a
        })
    }
}

impl<R: crate::IRecord + Send + Sync> Storage for dashmap::DashMap<u64, R> {
    type Record = R;

    type Borrow<'b>
        = Ref<'b, u64, Self::Record>
    where
        Self::Record: 'b;

    type BorrowMut<'b>
        = RefMut<'b, u64, Self::Record>
    where
        Self: 'b,
        Self::Record: 'b;

    type IterBorrow<'b>
        = RefMulti<'b, u64, Self::Record>
    where
        R: 'b;

    #[inline]
    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record> {
        let key = record.index_key();
        let previous = self.insert(key, record);

        PutResult { key, previous }
    }

    #[inline]
    fn record<'a: 'b, 'b>(&'a self, key: u64) -> Option<Self::Borrow<'b>> {
        self.get(&key)
    }

    #[inline]
    fn iter_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Iterator<Item = Self::IterBorrow<'b>> + Send + 'b {
        self.iter()
    }

    #[inline]
    fn stream_records<'a: 'b, 'b>(
        &'a self,
    ) -> impl Stream<Item = Self::IterBorrow<'b>> + Send + 'b {
        futures::stream::iter(self.iter_records())
    }

    #[inline]
    fn replace(&mut self, key: u64, record: Self::Record) -> Option<Self::Record> {
        self.insert(key, record)
    }

    #[inline]
    fn record_mut<'a: 'b, 'b>(&'a mut self, key: u64) -> Option<Self::BorrowMut<'b>> {
        self.get_mut(&key)
    }
    
    #[inline]
    fn reverse_map(&self) -> ahash::HashMap<u64, u64> {
        self.iter().fold(Default::default(), |mut a, e| {
            a.insert(e.index_key(), *e.key());
            a
        })
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

    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        None
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

    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        None
    }
}

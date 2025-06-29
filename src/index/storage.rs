use futures::Stream;

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
pub trait Storage : Default {
    /// Record being stored
    type Record: crate::IRecord;

    /// Put a record into storage
    /// 
    /// Returns an InsertResult w/ the key that can be used w/ Storage::get to retrieve the record
    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record>;

    /// Returns the stored record
    /// 
    /// Returns None if the key was not recognized by Storage
    fn record(&self, key: u64) -> Option<&Self::Record>;

    /// Returns an iterator over all records in storage
    fn iter_records(&self) -> impl Iterator<Item = &Self::Record> + Send;

    /// Returns a stream over all records in storage
    fn stream_records(&self) -> impl Stream<Item = &Self::Record> + Send;

    /// Returns true if a record was replaced at key
    fn replace(&mut self, key: u64, record: Self::Record) -> bool;
}

impl<R: crate::IRecord + Sync> Storage for ahash::HashMap<u64, R> {
    type Record = R;

    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record> {
        let key = record.index_key();
        let previous = self.insert(key, record);

        PutResult { key, previous }
    }

    fn record(&self, key: u64) -> Option<&Self::Record> {
        self.get(&key)
    }
    
    fn iter_records(&self) -> impl Iterator<Item = &Self::Record> + Send {
        self.iter().map(|(_, v)| v)
    }

    fn stream_records(&self) -> impl Stream<Item = &Self::Record> + Send {
        futures::stream::iter(self.iter_records())
    }
    
    fn replace(&mut self, key: u64, record: Self::Record) -> bool {
        self.insert(key, record).is_some()
    }
}

impl<R: crate::IRecord + Sync> Storage for Vec<R> {
    type Record = R;

    fn put(&mut self, record: Self::Record) -> PutResult<Self::Record> {
        let key = self.len() as u64;
        self.push(record);

        PutResult { key, previous: None }
    }

    fn record(&self, key: u64) -> Option<&Self::Record> {
        self.get(key as usize)
    }
    
    fn iter_records(&self) -> impl Iterator<Item = &Self::Record> + Send {
        self.iter()
    }

    fn stream_records(&self) -> impl Stream<Item = &Self::Record> + Send {
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

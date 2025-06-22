pub mod archive;

mod query;

pub use query::Indexer;
pub use query::TextMetadata;

mod record;
use record::Namespace;
pub use record::Record;

mod index;
pub use index::Index;

mod worker;
use serde::Serialize;
pub use worker::Worker;

mod store;

bitflags::bitflags! {
    /// Record options
    #[derive(Copy, Clone, Debug)]
    pub struct RecordOpts : u8 {
        /// Indicates that the record can be processed by the indexer
        const Indexing = 1;
        /// Indicates that the record should not be archived
        const NoArchive = 1 << 1;
    }
}

/// Provides extensions for configuring a type before it is committed as a record
pub trait RecordableExtensions {
    /// Enables the record to be indexed
    #[inline]
    fn indexable(&self) -> Recordable<'_, Self>
    where
        Self: Sized,
    {
        Recordable::from(self).indexable()
    }

    /// Prevents the record from being archived
    #[inline]
    fn no_archive(&self) -> Recordable<'_, Self> 
    where
        Self: Sized
    {
        Recordable::from(self).no_archive()
    }
}

impl<T: serde::Serialize> RecordableExtensions for T {}

/// Wrapper struct enabling pre-configuring record options
/// before data is committed to the record
#[derive(Clone, Copy)]
pub struct Recordable<'a, T> {
    /// Object being recorded
    pub(crate) recording: &'a T,
    /// Record options passed set on the record
    pub(crate) opts: RecordOpts,
}

impl<'a, T> Recordable<'a, T> {
    /// Enables indexing for this record
    #[inline]
    pub fn indexable(mut self) -> Self {
        self.opts |= RecordOpts::Indexing;
        self
    }

    /// Enables the no_archive flag for this record
    #[inline]
    pub fn no_archive(mut self) -> Self {
        self.opts |= RecordOpts::NoArchive;
        self
    }

    /// Consumes the reference and creates a record
    #[inline]
    pub fn to_record(self, label: &str, namespace: impl Into<Namespace>) -> Record
    where
        T: Serialize,
    {
        namespace
            .into()
            .save(label, self.recording)
            .with_opts(self.opts)
    }

    /// Returns the record opts
    #[inline]
    pub fn opts(&self) -> RecordOpts {
        self.opts
    }
}

impl<'a, T> From<&'a T> for Recordable<'a, T> {
    fn from(value: &'a T) -> Self {
        Recordable::<T> {
            recording: value,
            opts: RecordOpts::empty(),
        }
    }
}

impl<'a, T: serde::Serialize> serde::Serialize for Recordable<'a, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.recording.serialize(serializer)
    }
}

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn test_to_record() {
        let rec = toml::toml! {
            value = "hello world"
        }
        .indexable()
        .to_record("test_record", ());

        assert_eq!(
            "hello world",
            rec.load::<toml::Value>().unwrap()["value"]
                .as_str()
                .unwrap()
        );

        let mut index = Index::default();
        index.index(&rec);

        assert_eq!(1, index.search_text("value", "hello").count());
        assert_eq!(0, index.search_text("value", "goodbye").count());
    }
}

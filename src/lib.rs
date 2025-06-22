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
pub use worker::Worker;

mod store;

use serde::Serialize;

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
    fn indexable(&self) -> Recordable<'_, Self, 1>
    where
        Self: Sized,
    {
        Recordable::<Self, 0>::from(self).indexable()
    }

    /// Prevents the record from being archived
    #[inline]
    fn no_archive(&self) -> Recordable<'_, Self, 1>
    where
        Self: Sized,
    {
        Recordable::<Self, 0>::from(self).no_archive()
    }
}

impl<T: serde::Serialize> RecordableExtensions for T {}

/// Wrapper struct enabling pre-configuring record options
/// before data is committed to the record
#[derive(Clone, Copy)]
pub struct Recordable<'a, T, const REF_GUARD: i8> {
    /// Object being recorded
    pub(crate) recording: &'a T,
    /// Record options passed set on the record
    pub(crate) opts: RecordOpts,
}

impl<'a, T, const REF_COUNT: i8> Recordable<'a, T, REF_COUNT> {
    /// Enables indexing for this record
    #[inline]
    pub fn indexable(mut self) -> Recordable<'a, T, 1> {
        self.opts |= RecordOpts::Indexing;
        Recordable {
            recording: self.recording,
            opts: self.opts,
        }
    }

    /// Enables the no_archive flag for this record
    #[inline]
    pub fn no_archive(mut self) -> Recordable<'a, T, 1> {
        self.opts |= RecordOpts::NoArchive;
        Recordable {
            recording: self.recording,
            opts: self.opts,
        }
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

impl<'a, T> From<&'a T> for Recordable<'a, T, 0> {
    fn from(value: &'a T) -> Self {
        Recordable::<T, 0> {
            recording: value,
            opts: RecordOpts::empty(),
        }
    }
}

/// HACK: These two below implementations hack the type system to address this situation:
/// 
/// ```rs no_run
/// 
/// // This is okay
/// worker.save(
///     toml {
///         value = "hello world"
///     }.no_archive()
/// )
/// 
/// // This is a trap, if rust decides to use the From<&'a T> impl
/// // It will end up resetting all the record flags previously set
/// // With the below fix, this forces the type system not to compile this code
/// // Although, the error message is confusing, it's better than a runtime bug ┐(´ー｀)┌
/// worker.save(
///     &toml {
///         value = "hello world"
///     }.no_archive()
/// )
/// ```
impl<'a, T> From<Recordable<'a, T, 1>> for Recordable<'a, T, 0> {
    fn from(value: Recordable<'a, T, 1>) -> Self {
        Recordable::<T, 0> {
            recording: value.recording,
            opts: value.opts,
        }
    }
}

impl<'a, T> From<&'a Recordable<'a, T, 1>> for Recordable<'a, T, 0> {
    fn from(value: &'a Recordable<'a, T, 1>) -> Self {
        Recordable::<T, 0> {
            recording: value.recording,
            opts: value.opts,
        }
    }
}

impl<'a, const REF_GUARD: i8, T: serde::Serialize> serde::Serialize
    for Recordable<'a, T, REF_GUARD>
{
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

pub mod archive;
pub mod util;

mod error;
pub use error::Error;

/// Type-alias for a result returned by this crate
pub type Result<T> = std::result::Result<T, Error>;

mod data;
pub use data::Data;

mod namespace;
pub use namespace::Namespace;
pub use namespace::ToNamespace;

mod record;
pub use record::Record;
pub use record::IRecord;

mod worker;
pub use worker::Worker;
pub use worker::SharedWorker;

mod opts;
pub use opts::Opts;
pub use opts::EMPTY_OPTS;

mod index;
pub use index::Index;
pub use index::Storage;
pub use index::HashIndex;
pub use index::HashMapStorage;
pub use index::VecIndex;
pub use index::VecStorage;
pub use index::ConcurrentIndex;
pub use index::ConcurrentStorage;
pub use index::search;

mod query;
pub use query::Indexer;
pub use query::TextMetadata;
pub use query::field;
pub use query::string;
pub use query::namespace;
pub use query::filter;
pub use query::not;
pub use query::QueryBuilder;
pub use query::Query;

mod store;
pub use store::Store;

mod virt;
pub use virt::RecordExtent;
pub use virt::VirtualData;
pub use virt::vol;

pub mod frontend;
pub use frontend::kv;

mod queue;
pub use queue::Queue;

/// Provides extensions for configuring a type before it is committed as a record
pub trait RecordableExtensions {
    /// Enables the record to be indexed
    #[inline]
    fn indexable(&self) -> RecordableConfig<'_, Self>
    where
        Self: Sized,
    {
        let mut indexable = Recordable::<Self, false>::from(self).indexable();
        indexable.opts_mut().set_object_storage(true);
        indexable
    }

    /// Prevents the record from being archived
    #[inline]
    fn no_archive(&self) -> RecordableConfig<'_, Self>
    where
        Self: Sized,
    {
        let mut no_archive = Recordable::<Self, false>::from(self).no_archive();
        no_archive.opts_mut().set_object_storage(true);
        no_archive
    }
}

impl<T: serde::Serialize> RecordableExtensions for T {}

/// Type-alias for an inert recordable wrapper
pub type RawRecordable<'a, T> = Recordable<'a, T, false>;

/// Type-alias for a user-configurable recordable wrapper
pub type RecordableConfig<'a, T> = Recordable<'a, T, true>;

/// Wrapper struct enabling pre-configuring record options
/// before data is committed to the record
#[derive(Clone, Copy)]
pub struct Recordable<'a, T, const REF_GATE: bool> {
    /// Object being recorded
    pub(crate) recording: &'a T,
    /// Record options passed set on the record
    pub(crate) opts: Opts,
}

impl<'a, T, const REF_GATE: bool> Recordable<'a, T, REF_GATE> {
    /// Enables indexing for this record
    #[inline]
    pub fn indexable(mut self) -> RecordableConfig<'a, T> {
        self.opts.enable_indexing();
        Recordable {
            recording: self.recording,
            opts: self.opts,
        }
    }

    /// Enables the no_archive flag for this record
    #[inline]
    pub fn no_archive(mut self) -> RecordableConfig<'a, T> {
        self.opts.disable_archiving();
        Recordable {
            recording: self.recording,
            opts: self.opts,
        }
    }

    /// Consumes the reference and creates a record
    #[inline]
    pub fn to_record(self, label: &str, namespace: impl Into<Namespace>) -> Record
    where
        T: serde::Serialize,
    {
        namespace
            .into()
            .store(label, self.recording)
            .with_opts(self.opts)
    }

    /// Returns the record opts
    #[inline]
    pub fn opts(&self) -> Opts {
        self.opts
    }

    /// Returns mutable reference to current opts
    #[inline]
    pub fn opts_mut(&mut self) -> &mut Opts {
        &mut self.opts
    }
}

impl<'a, T> From<&'a T> for RawRecordable<'a, T> {
    fn from(value: &'a T) -> Self {
        RawRecordable::<T> {
            recording: value,
            opts: Opts::default(),
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
impl<'a, T> From<RecordableConfig<'a, T>> for RawRecordable<'a, T> {
    fn from(value: RecordableConfig<'a, T>) -> Self {
        RawRecordable::<T> {
            recording: value.recording,
            opts: value.opts,
        }
    }
}

impl<'a, T> From<&'a RecordableConfig<'a, T>> for RawRecordable<'a, T> {
    fn from(value: &'a RecordableConfig<'a, T>) -> Self {
        RawRecordable::<T> {
            recording: value.recording,
            opts: value.opts,
        }
    }
}

impl<'a, const REF_GUARD: bool, T: serde::Serialize> serde::Serialize
    for Recordable<'a, T, REF_GUARD>
{
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
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

        // let mut index = Index::<Record>::default();
        // index.index(&rec);
        // assert_eq!(1, index.search_text("value", "hello").count());
        // assert_eq!(0, index.search_text("value", "goodbye").count());
    }
}

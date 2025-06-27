mod indexer;
pub use indexer::Indexer;

mod text;
pub use text::TextMetadata;

use crate::{Namespace, Record};

/// Begins a field query
#[inline]
pub fn field<'query>(field: &'query str) -> Field<'query> {
    Field { field }
}

/// Begins a namespace scoped query
#[inline]
pub fn namespace<'query>(ns: impl Into<Namespace>) -> Namespaced<'query> {
    Namespaced {
        namespace: ns.into(),
        key: None,
        query: None,
    }
}

pub trait Matches<'query> {
    fn matches(&'query self, record: &Record) -> bool;
}

#[derive(Clone)]
pub enum Query<'query> {
    /// Query a field
    Field(Field<'query>),
    /// Check if a field contains a value
    Contains(Contains<'query>),
    /// Namespaced scoped query
    Namespaced(Box<Namespaced<'query>>),
}

impl<'query> Matches<'query> for Query<'query> {
    /// Returns true if record matches the query parameters
    #[inline]
    fn matches(&self, record: &Record) -> bool {
        match self {
            Query::Field(field) => record
                .peek_map(|r| !r.as_map().idx(field.field).flexbuffer_type().is_null())
                .unwrap_or_default(),
            Query::Contains(contains) => record
                .peek_map(|r| {
                    r.as_map()
                        .idx(contains.field)
                        .as_str()
                        .contains(contains.contains)
                })
                .unwrap_or_default(),
            Query::Namespaced(namespaced) => namespaced.matches(record),
        }
    }
}

impl<'query> From<Namespaced<'query>> for Query<'query> {
    fn from(value: Namespaced<'query>) -> Self {
        Self::Namespaced(Box::new(value))
    }
}

impl<'query> From<Field<'query>> for Query<'query> {
    fn from(value: Field<'query>) -> Self {
        Self::Field(value)
    }
}

impl<'query> From<Contains<'query>> for Query<'query> {
    fn from(value: Contains<'query>) -> Self {
        Self::Contains(value)
    }
}

#[derive(Clone)]
pub struct Namespaced<'query> {
    namespace: Namespace,
    key: Option<u64>,
    query: Option<Query<'query>>,
}

impl<'query> Namespaced<'query> {
    /// Sets the key for the namespaced query scope
    #[inline]
    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(self.namespace.key(key));
        self
    }

    /// Sets the query for the namespaced query scope
    #[inline]
    pub fn query(mut self, query: impl Into<Query<'query>>) -> Self {
        self.query = Some(query.into());
        self
    }
}

impl<'query> Matches<'query> for Namespaced<'query> {
    /// Returns true if record matches the query parameters
    #[inline]
    fn matches(&self, record: &Record) -> bool {
        record.ns_chk() == self.namespace.chk()
            && self
                .key
                .map(|k| record.uuid().as_u64_pair().0 == k)
                .unwrap_or(true)
            && self
                .query
                .as_ref()
                .map(|q| q.matches(record))
                .unwrap_or(true)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Field<'query> {
    field: &'query str,
}

#[derive(Clone, Copy, Debug)]
pub struct Contains<'query> {
    field: &'query str,
    contains: &'query str,
}

impl<'query> Field<'query> {
    /// Finds a field that contains value,
    ///
    /// Note: implies field is a str type
    #[inline]
    pub fn contains(self, contains: &'query str) -> Contains<'query> {
        Contains {
            field: &self.field,
            contains,
        }
    }
}

mod field;
mod namespaced;
mod text;

pub use field::Field;
pub use namespaced::Namespaced;
use std::fmt::Debug;
pub use text::TextMetadata;

use crate::{
    IRecord,
    util::{Container, PeekExtensions},
};

/// Trait to apply match conditions on a Record
pub trait Matches: std::fmt::Debug {
    /// Record frontend
    type Record: crate::IRecord;

    /// Returns true if record matches a condition
    fn matches(&self, record: &Self::Record) -> bool;
}

/// Trait for types that enables query building
pub trait QueryBuilder<'query, R: crate::IRecord + 'query> {
    /// Sets the query to evaluate
    fn set_query(&mut self, next: Query<'query, R>);

    /// Gets the current query
    fn get_query(&self) -> Option<Query<'query, R>>;

    /// Logical AND w/ query over existing query
    fn and(mut self, query: impl Into<Query<'query, R>>) -> Self
    where
        Self: Sized + 'query,
    {
        let query = query.into();
        if let Some(existing) = self.get_query() {
            self.set_query(
                Filter(std::sync::Arc::new(move |r| {
                    existing.matches(r) && query.matches(r)
                }))
                .into(),
            );
        } else {
            self.set_query(query);
        }
        self
    }

    /// Logical OR w/ query over existing query
    fn or(mut self, query: impl Into<Query<'query, R>>) -> Self
    where
        Self: Sized + 'query,
    {
        let query = query.into();
        if let Some(existing) = self.get_query() {
            self.set_query(
                Filter(std::sync::Arc::new(move |r| {
                    existing.matches(r) || query.matches(r)
                }))
                .into(),
            );
        } else {
            self.set_query(query);
        }
        self
    }
}

/// Wraps query components for use w/ search infrastructure
#[derive(Debug)]
pub struct Query<'query, R> {
    matches: std::sync::Arc<dyn Matches<Record = R> + Send + Sync + 'query>,
}

impl<'query, R> Clone for Query<'query, R> {
    fn clone(&self) -> Self {
        Self {
            matches: self.matches.clone(),
        }
    }
}

impl<'query, R: crate::IRecord> Query<'query, R> {
    /// Returns true if the record matches the expected query parameters
    #[inline]
    pub fn matches(&self, record: &R) -> bool {
        self.matches.matches(record)
    }
}

/// Wraps a closure that implements Matches/LayeredQuery
pub struct Filter<'q, R>(std::sync::Arc<dyn Fn(&R) -> bool + Send + Sync + 'q>);

impl<'q, R> Clone for Filter<'q, R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Begins a record filter query
#[inline]
pub fn filter<'q, R>(filter: impl Fn(&R) -> bool + Send + Sync + 'q) -> Filter<'q, R> {
    Filter(std::sync::Arc::new(filter))
}

/// Begins a field query, checks a record to see if a field is set
///
/// Note: This will not search nested fields
#[inline]
pub fn field<'query, R: crate::IRecord + 'query>(field: &'query str) -> Field<'query, R> {
    Field {
        field,
        query: Some(
            filter::<R>(|record| {
                record
                    .field(field)
                    .val()
                    .map(|r| !r.flexbuffer_type().is_null())
                    .unwrap_or_default()
            })
            .into(),
        ),
    }
}

/// Begins a content digest query
/// 
/// Note: For multi-root records will use the canonical digest which is the container root digest
#[inline]
pub fn content<'query, R: crate::IRecord + 'query>(content: impl AsRef<[u8]> + Send + Sync + 'static) -> Filter<'query, R> {
    filter::<R>(move |record| {
        if record.opts().is_multi() {
            Container::read(record)
                .map(|r| r.content().starts_with(content.as_ref()))
                .unwrap_or_default()
        } else {
            record.content().starts_with(content.as_ref())
        }
    })
    .into()
}

/// Begins a container digest query
/// 
/// Specifically searches for the "container" content digest, which is the digest
/// of the entire inner record
#[inline]
pub fn container<'query, R: crate::IRecord + 'query>(content: [u8; 32]) -> Filter<'query, R> {
    filter::<R>(move |record| {
        if record.opts().is_multi() {
            record.content() == content
        } else {
            false
        }
    })
    .into()
}

/// Begins a field query, checks a record to see if a field is set and is a str type
///
/// Note: This will not search nested fields
#[inline]
pub fn string<'query, R: crate::IRecord + 'query>(field: &'query str) -> Field<'query, R> {
    Field {
        field,
        query: Some(filter::<R>(|record| record.field(field).str().is_some()).into()),
    }
}

/// Begins a namespace scoped query
#[inline]
pub fn namespace<'query, R: crate::IRecord + 'query>(
    ns: impl Into<crate::Namespace>,
) -> Namespaced<'query, R> {
    let namespace = ns.into();
    let ns_chk = namespace.chk();
    Namespaced {
        namespace,
        query: Some(filter::<R>(move |record| record.ns_chk() == ns_chk).into()),
    }
}

/// Applies logical NOT to the result of query.matches(..)
///
/// i.e. !query.matches(..)
#[inline]
pub fn not<'query, R: crate::IRecord + 'query>(
    query: impl Into<Query<'query, R>>,
) -> Query<'query, R> {
    let query = query.into();

    filter(move |r| !query.matches(r)).into()
}

impl<'q, R> std::fmt::Debug for Filter<'q, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("QueryFn").finish()
    }
}

impl<'query, R: crate::IRecord + Debug> Matches for Option<Query<'query, R>> {
    type Record = R;

    #[inline]
    fn matches(&self, record: &Self::Record) -> bool {
        self.as_ref().map(|q| q.matches(record)).unwrap_or(true)
    }
}

impl<'query, R: crate::IRecord> Matches for fn(&R) -> bool {
    type Record = R;

    #[inline]
    fn matches(&self, record: &Self::Record) -> bool {
        (self)(record)
    }
}

impl<'query, R: crate::IRecord> Matches for Filter<'query, R> {
    type Record = R;

    #[inline]
    fn matches(&self, record: &Self::Record) -> bool {
        (self.0)(record)
    }
}

impl<'query, T: Matches + Send + Sync + 'query> From<T> for Query<'query, T::Record> {
    #[inline]
    fn from(value: T) -> Self {
        Self {
            matches: std::sync::Arc::new(value),
        }
    }
}

impl<'q, R: crate::IRecord + 'q> QueryBuilder<'q, R> for Filter<'q, R> {
    #[inline]
    fn set_query(&mut self, next: Query<'q, R>) {
        *self = Filter(std::sync::Arc::new(move |r| next.matches(r)));
    }

    #[inline]
    fn get_query(&self) -> Option<Query<'q, R>> {
        Some(self.clone().into())
    }
}

impl<'q, R: crate::IRecord + 'q> QueryBuilder<'q, R> for Query<'q, R> {
    #[inline]
    fn set_query(&mut self, next: Query<'q, R>) {
        *self = next;
    }

    #[inline]
    fn get_query(&self) -> Option<Query<'q, R>> {
        Some(self.clone().into())
    }
}

#[cfg(test)]
mod test {
    use crate::{Namespace, namespace, not, query::Matches};
    use toml::toml;

    use super::{QueryBuilder, field, string};

    #[test]
    fn test_field() {
        let filter = field("value");

        let ns = Namespace::ephemeral();

        let rec = ns.store(
            "record_1",
            &toml! {
                value = "foo"
            },
        );
        assert!(filter.matches(&rec));

        let rec = ns.store(
            "record_1",
            &toml! {
                not_value = "foo"
            },
        );
        assert!(!filter.matches(&rec))
    }

    #[test]
    fn test_field_contains() {
        let filter = field("value").contains("fo");

        let ns = Namespace::ephemeral();

        let rec = ns.store(
            "record_1",
            &toml! {
                value = "foo"
            },
        );
        assert!(filter.matches(&rec));

        let rec = ns.store(
            "record_1",
            &toml! {
                value = "bar"
            },
        );
        assert!(!filter.matches(&rec))
    }

    #[test]
    fn test_namespace() {
        let ns = Namespace::ephemeral();

        let ns_clone = ns.clone();
        let ns_filter = namespace(ns_clone.clone());

        let rec = ns.store(
            "record_1",
            &toml! {
                value = "foo"
            },
        );
        assert!(
            ns_filter.matches(&rec),
            "original: {} clone: {}",
            ns.chk(),
            ns_clone.chk()
        );

        let other_ns = Namespace::ephemeral();
        let rec = other_ns.store(
            "record_1",
            &toml! {
                value = "foo"
            },
        );
        assert!(!ns_filter.matches(&rec));
    }

    #[test]
    fn test_labeled() {
        let ns = Namespace::ephemeral();
        let ns_filter = namespace(ns.clone()).label("record_1");

        let rec = ns.store(
            "record_1",
            &toml! {
                value = "foo"
            },
        );
        assert!(ns_filter.matches(&rec));

        let rec = ns.store(
            "record_2",
            &toml! {
                value = "foo"
            },
        );
        assert!(!ns_filter.matches(&rec));

        let other_ns = Namespace::ephemeral();
        let rec = other_ns.store(
            "record_1",
            &toml! {
                value = "foo"
            },
        );
        assert!(!ns_filter.matches(&rec));
    }

    #[test]
    fn test_query_builder_and() {
        let filter_1 = field("value").and(field("other_value"));
        let filter_2 = field("value").and(field("other_value").contains("h"));

        let ns = Namespace::ephemeral();

        let rec = ns.store(
            "record_1",
            &toml! {
                value = "foo"
                other_value = "bar"
            },
        );
        assert!(filter_1.matches(&rec));
        assert!(!filter_2.matches(&rec));

        let rec = ns.store(
            "record_2",
            &toml! {
                value = "foo"
                other_value = "hello"
            },
        );
        assert!(filter_1.matches(&rec));
        assert!(filter_2.matches(&rec));
    }

    #[test]
    fn test_query_builder_or() {
        let filter_1 = field("value").or(field("other_value"));
        let filter_2 = field("not_value").or(field("other_value").contains("h"));

        let ns = Namespace::ephemeral();

        let rec = ns.store(
            "record_1",
            &toml! {
                value = "foo"
                other_value = "bar"
            },
        );
        assert!(filter_1.matches(&rec));
        assert!(!filter_2.matches(&rec));

        let rec = ns.store(
            "record_2",
            &toml! {
                value = "foo"
                other_value = "hello"
            },
        );
        assert!(filter_1.matches(&rec));
        assert!(filter_2.matches(&rec));
    }

    #[test]
    fn test_query_not() {
        let filter_1 = not(field("value"));
        let filter_2 = not(field("value").contains("hello"));

        let ns = Namespace::ephemeral();

        let rec = ns.store(
            "record_1",
            &toml! {
                value = "foo"
                other_value = "bar"
            },
        );
        assert!(!filter_1.matches(&rec));
        assert!(filter_2.matches(&rec));

        let rec = ns.store(
            "record_2",
            &toml! {
                value = "foo"
                other_value = "hello"
            },
        );
        assert!(!filter_1.matches(&rec));
        assert!(filter_2.matches(&rec));

        let rec = ns.store(
            "record_3",
            &toml! {
                unique_value = ""
            },
        );
        assert!(filter_1.matches(&rec));
        assert!(filter_2.matches(&rec));
    }

    #[test]
    fn test_field_string() {
        let filter = string("value");
        let ns = Namespace::ephemeral();
        let rec = ns.store(
            "record_1",
            &toml! {
                a = 1
                b = 2
                c = 3
                ab = true
                bc = false
                value = "foo"
            },
        );
        assert!(filter.matches(&rec))
    }
}

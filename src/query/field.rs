use std::fmt::Debug;

use crate::{record::PeekMap, IRecord};

use super::{Matches, Query, QueryBuilder, filter};

/// Query entrypoint to scope
#[derive(Clone, Debug)]
pub struct Field<'query, R: IRecord> {
    pub(crate) field: &'query str,
    pub(crate) query: Option<Query<'query, R>>,
}

impl<'query, R: IRecord + 'query> Field<'query, R> {
    /// Finds a field that contains value,
    ///
    /// Note: implies field is a str type
    #[inline]
    pub fn contains(self, contains: &'query str) -> Query<'query, R> {
        filter::<R>(move |record: &R| {
            record
                .peek_map(|r| {
                    r.as_map().idx(self.field).as_str().contains(contains)
                })
                .unwrap_or_default()
        })
        .into()
    }

    /// Finds a field that starts_with value,
    ///
    /// Note: implies field is a str type
    #[inline]
    pub fn starts_with(self, starts_with: &'query str) -> Query<'query, R> {
        filter::<R>(move |record| {
            record
                .peek_map(|r| {
                    r.as_map().idx(self.field).as_str().starts_with(starts_with)
                })
                .unwrap_or_default()
        })
        .into()
    }

    /// Finds a field that ends_with value,
    ///
    /// Note: implies field is a str type
    #[inline]
    pub fn ends_with(self, ends_with: &'query str) -> Query<'query, R> {
        filter::<R>(move |record| {
            record
                .peek_map(|r| {
                    r.as_map().idx(self.field).as_str().ends_with(ends_with)
                })
                .unwrap_or_default()
        })
        .into()
    }
}

impl<'query, R: IRecord + Debug> Matches for Field<'query, R> {
    type Record = R;

    fn matches(&self, record: &Self::Record) -> bool {
        self.query.matches(record)
    }
}

impl<'q, R: IRecord + 'q> QueryBuilder<'q, R> for Field<'q, R> {
    fn set_query(&mut self, next: Query<'q, R>) {
        self.query = Some(next);
    }

    fn get_query(&self) -> Option<Query<'q, R>> {
        self.query.clone()
    }
}

#[cfg(test)]
mod test {
    use toml::toml;

    use crate::{string, Namespace};
 
    #[test]
    fn test_starts_with() {
        let ns = Namespace::ephemeral();
        let rec = ns.store("record_1", &toml! {
            value = "super_hello_world"
        });
        assert!(string("value").starts_with("super").matches(&rec))
    }

    #[test]
    fn test_ends_with() {
        let ns = Namespace::ephemeral();
        let rec = ns.store("record_1", &toml! {
            value = "super_hello_world"
        });
        assert!(!string("value").ends_with("super").matches(&rec));
        assert!(string("value").ends_with("world").matches(&rec));
    }
}
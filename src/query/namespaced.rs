use super::{Matches, Query, QueryBuilder, filter};
use crate::{IRecord, Namespace};

/// Wrapper over a query to apply namespace/key scoping to a query
#[derive(Debug)]
pub struct Namespaced<'query, R> {
    /// Namespace record must match
    pub(crate) namespace: Namespace,
    /// Next query to evaluate after namespace matches
    pub(crate) query: Option<Query<'query, R>>,
}

impl<'q, R> Clone for Namespaced<'q, R> {
    fn clone(&self) -> Self {
        Self { namespace: self.namespace.clone(), query: self.query.clone() }
    }
}

impl<'q, R: IRecord + 'q> Namespaced<'q, R> {
    /// Converts this scope to a Labeled scope
    #[inline]
    pub fn label(self, key: &str) -> Query<'q, R> {
        let key = self.namespace.key(key);
        filter::<R>(move |record| {
            record.ns_chk() == self.namespace.chk() && record.uuid().as_u64_pair().0 == key
        })
        .into()
    }
}

impl<'query, R: IRecord> Matches for Namespaced<'query, R> {
    type Record = R;

    #[inline]
    fn matches(&self, record: &Self::Record) -> bool {
         self.query.matches(record)
    }
}

impl<'q, R: IRecord + 'q> QueryBuilder<'q, R> for Namespaced<'q, R> {
    #[inline]
    fn set_query(&mut self, next: Query<'q, R>) {
        self.query = Some(next);
    }

    fn get_query(&self) -> Option<Query<'q, R>> {
        self.query.clone()
    }
}

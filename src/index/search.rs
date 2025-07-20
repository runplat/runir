pub mod iter {
    use ahash::{HashSet, HashSetExt};
    use tracing::debug;

    use crate::{IRecord, Query, Storage};

    pub trait Search<S: Storage>: AsRef<S> {
        /// Searches the index with a query
        #[inline]
        fn search<'query>(
            &'query self,
            query: impl Into<Query<'query, S::IterBorrow<'query>>>,
        ) -> impl Iterator<Item = S::IterBorrow<'query>>
        where
            S: 'query,
            S::Record: 'query,
        {
            let mut dedupe = HashSet::new();
            let query = query.into();
            self.as_ref().iter_records().filter(move |r| {
                let matches = query.matches(r);
                let dedupe = dedupe.insert(r.content());
                debug!(dedupe = !dedupe, "will dedupe");
                dedupe && matches
            })
        }
    }

    impl<S: Storage, T: AsRef<S>> Search<S> for T {}
}

pub mod stream {
    use crate::{Query, Storage};
    use async_stream::stream;
    use futures::{Stream, StreamExt};

    pub trait Search<S: Storage>: AsRef<S> {
        /// Searches the index with a query
        #[inline]
        fn search<'query>(
            &'query self,
            query: impl Into<Query<'query, S::IterBorrow<'query>>>,
        ) -> impl Stream<Item = S::IterBorrow<'query>>
        where
            S: 'query,
            S::Record: 'query,
        {
            stream! {
                let query = query.into();
                let mut stream = std::pin::pin!(self.as_ref().stream_records());
                while let Some(next) = stream.next().await {
                    if query.matches(&next) {
                        yield next;
                    }
                }
            }
        }
    }

    impl<S: Storage, T: AsRef<S>> Search<S> for T {}
}

pub mod par {
    use crate::{IRecord, Index, Query, Record};
    use rayon::prelude::*;

    pub trait Search<R: IRecord + Send + Sync>: AsRef<dashmap::DashMap<u64, R>> {
        /// Searches the index with a query
        /// 
        /// Returns a parallel iterator over matching records
        #[inline]
        fn search<'query>(
            &'query self,
            query: impl Into<Query<'query, R>>,
        ) -> impl ParallelIterator<Item = Record>
        where
            R: 'query,
        {
            let query: Query<_> = query.into();
            self.as_ref()
                .par_iter_mut()
                .filter(move |r| query.matches(&r))
                .map(|r| r.to_record())
        }
    }

    impl<R: IRecord + Send + Sync> Search<R> for Index<R, dashmap::DashMap<u64, R>> {}
}

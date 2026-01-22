pub mod iter {
    use ahash::{HashSet, HashSetExt};
    use sha2::Sha256;
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
                let dedupe = dedupe.insert(r.content::<Sha256>());
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
            self.as_ref().par_iter_mut().filter_map(move |r| {
                if query.matches(&r) {
                    Some(r.to_record())
                } else {
                    None
                }
            })
        }
    }

    impl<R: IRecord + Send + Sync> Search<R> for Index<R, dashmap::DashMap<u64, R>> {}
}

pub mod par_stream {
    use crate::{IRecord, Index, Query, Record};
    use async_stream::stream;
    use futures::{Stream, StreamExt, stream};
    use rayon::prelude::*;
    use tracing::error;

    pub trait Search<R: IRecord + Send + Sync>:
        AsRef<dashmap::DashMap<u64, R>> + Send + Sync
    {
        /// Searches the index with a query
        ///
        /// Returns a stream that returns results as batches are completed
        #[inline]
        fn search<'query>(
            &'query self,
            query: impl Into<Query<'query, R>> + Send + Sync,
        ) -> impl Stream<Item = Record>
        where
            R: 'query,
        {
            std::thread::scope(move |scope| {
                let (tx, rx) = crossbeam::channel::unbounded();
                let query: Query<_> = query.into();
                let iter = self.as_ref().par_iter_mut().filter_map(move |r| {
                    if query.matches(&r) {
                        match tx.send(r.to_record()) {
                            Ok(_) => None,
                            Err(err) => Some(err.0),
                        }
                    } else {
                        None
                    }
                });

                let scoped = scope.spawn(|| iter.collect::<Vec<_>>());
                stream! {
                    for next in rx.iter() {
                        yield next;
                    }
                }
                .chain({
                    // In case of a very unexpected failure in the channel;
                    // Ensure no records are lost silently
                    stream::iter(
                        scoped
                            .join()
                            .inspect(|v| {
                                if !v.is_empty() {
                                    error!("Channel disconnected before stream could complete");
                                }
                            })
                            .unwrap_or(vec![]),
                    )
                })
            })
        }
    }

    impl<R: IRecord + Send + Sync> Search<R> for Index<R, dashmap::DashMap<u64, R>> {}
}

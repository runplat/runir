pub mod iter {
    use ahash::{HashSet, HashSetExt};

    use crate::{IRecord, Query, Storage};

    pub trait Search<S: Storage> : AsRef<S> {
        /// Searches the index with a query
        #[inline]
        fn search<'query>(
            &'query self,
            query: impl Into<Query<'query, S::Record>>,
        ) -> impl Iterator<Item = &'query S::Record> 
        where
            S: 'query,
            S::Record: 'query
        {
            let mut dedupe = HashSet::new();
            let query = query.into();
            self.as_ref().iter_records()
                .filter(move |r| {
                    let matches = query.matches(r);
                    dedupe.insert(r.content()) && matches
                })
        }
    }

    impl<S: Storage, T: AsRef<S>> Search<S> for T {}
}

pub mod stream {
    use async_stream::stream;
    use futures::{Stream, StreamExt};
    use crate::{Query, Storage};

    pub trait Search<S: Storage> : AsRef<S> {
        /// Searches the index with a query
        #[inline]
        fn search<'query>(
            &'query self,
            query: impl Into<Query<'query, S::Record>>,
        ) -> impl Stream<Item = &'query S::Record>
        where
            S: 'query,
            S::Record: 'query,
        {
            stream! { 
                let query = query.into();
                let mut stream = std::pin::pin!(self.as_ref().stream_records());
                while let Some(next) = stream.next().await {
                    if query.matches(next) {
                        yield next;
                    }
                }
            }
        }
    }

    impl<S: Storage, T: AsRef<S>> Search<S> for T {}
}

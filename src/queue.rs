use std::sync::{Arc, Weak};
use tokio_util::sync::CancellationToken;

const DEFAULT_QUEUE_SIZE: usize = 1000;

/// Type-alias for a "weak" inner queue, which doesn't hold a strong-count on the central queue
///
/// This allows the main queue to be flushed in it's entirety
type WeakInnerQueue<R> = Weak<crossbeam::queue::ArrayQueue<R>>;

/// Cloneable handle to a main queue that is push-only
pub struct Pusher<R> {
    inner: WeakInnerQueue<R>,
    shutdown: CancellationToken,
}

impl<R> Clone for Pusher<R> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), shutdown: self.shutdown.clone() }
    }
}

impl<R> Pusher<R> {
    /// Pushes a value onto the queue
    /// 
    /// If Some(Record) is returned, it can mean the following
    /// 
    /// 1) This indicates that the queue is currently full and is pending a flush, 
    ///   otherwise returning None indicates that the record has successfully entered the queue.
    /// 
    /// 2) The main queue has been de-allocated, call `is_shutdown(..)` to verify
    #[inline]
    pub fn push(&self, value: R) -> Option<R> {
        self.inner.upgrade().and_then(|q| match q.push(value) {
            Ok(_) => None,
            Err(record) => Some(record),
        })
    }

    /// Returns true if the queue no longer exists, this indicates any attempts to
    /// push will not succeed
    #[inline]
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.is_cancelled()
    }
}

/// Thread-safe write queue for Records
pub struct Queue<R, const INITIAL_CAPACITY: usize = DEFAULT_QUEUE_SIZE> {
    /// Inner queue data structure
    queue: Arc<crossbeam::queue::ArrayQueue<R>>,
    /// Cancellation token
    cancel: CancellationToken
}

impl<R, const INITIAL_CAPACITY: usize> Queue<R, INITIAL_CAPACITY> {
    /// Creates a new queue w/ CAPACITY
    #[inline]
    pub fn new() -> Self {
        let queue = Arc::new(crossbeam::queue::ArrayQueue::new(INITIAL_CAPACITY));
        Self { queue, cancel: CancellationToken::default() }
    }

    /// Returns a queue Pusher
    /// 
    /// A pusher can be used to ensure records are pushed onto the queue
    #[inline]
    pub fn pusher(&self) -> Pusher<R> {
        Pusher {
            inner: Arc::downgrade(&self.queue),
            shutdown: self.cancel.child_token()
        }
    }

    /// Flushes all elements from the queue
    ///
    /// Takes a count of the current len of the queue,
    /// Calls pop that many times,
    /// Returns whatever was popped
    #[inline]
    pub fn flush(&self) -> impl Iterator<Item = R> {
        let count = self.queue.len();
        (0..count).filter_map(|_| self.queue.pop())
    }

    /// Flushes MAX elements from the queue
    #[inline]
    pub fn flush_max(&self, max: usize) -> impl Iterator<Item = R> {
        (0..max).filter_map(|_| self.queue.pop())
    }

    /// Flushes and extends a dest
    #[inline]
    pub fn flush_into(&self, dest: &mut impl Extend<R>) {
        dest.extend(self.flush());
    }

    // TODO: This would be handy for lazily extending the queue
    // /// Resize the queue's capacity
    // pub fn resize(&mut self, capacity: usize) {
    // }
}

impl<R> Default for Queue<R> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Namespace;

    #[test]
    fn test_queue() {
        let ns = Namespace::ephemeral();

        let rec = ns.store(
            "test",
            &toml::toml! {
                value = "hello world"
            },
        );

        let queue = Queue::<crate::Record, 1>::new();
        let shared = queue.pusher();
        shared.push(rec.clone());

        let queued = shared.push(rec.clone()).expect("should be full");
        let queued = shared
            .push(queued)
            .expect("should return some because main queue hasn't flushed and is full");

        let flushed = queue.flush();
        assert_eq!(1, flushed.count());
        assert!(shared.push(queued).is_none())
    }

    #[test]
    fn test_queue_contention() {
        let ns = Namespace::ephemeral();

        let rec = ns.store(
            "test",
            &toml::toml! {
                value = "hello world"
            },
        );

        // Simulate contention with a tiny queue
        let queue = Queue::<crate::Record, 10>::new();

        let shared = queue.pusher();
        let _rec = rec.clone();
        let _ = std::thread::Builder::new()
            .spawn(move || {
                let mut pushes = 1000;
                while pushes > 0 {
                    if shared.push(_rec.clone()).is_none() {
                        pushes -= 1;
                    } else {
                    }
                }
            })
            .unwrap();

        let shared = queue.pusher();
        let rec = rec.clone();
        let _ = std::thread::Builder::new()
            .spawn(move || {
                let mut pushes = 1000;
                while pushes > 0 {
                    if shared.push(rec.clone()).is_none() {
                        pushes -= 1;
                    } else {
                    }
                }
            })
            .unwrap();

        let mut count = 0;
        while count < 2000 {
            count += queue.flush().count();
        }
        assert_eq!(2000, count);
    }
}

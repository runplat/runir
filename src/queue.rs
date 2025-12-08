use std::{
    fmt::Debug,
    sync::{Arc, Weak},
    thread::JoinHandle,
};

use crossbeam::{
    channel::{Receiver, Sender, bounded},
    utils::Backoff,
};

const DEFAULT_QUEUE_SIZE: usize = 1000;

/// Type-alias for a "weak" inner queue, which doesn't hold a strong-count on the central queue
///
/// This allows the main queue to be flushed in it's entirety
type WeakInnerQueue<R> = Weak<crossbeam::queue::ArrayQueue<R>>;

/// Cloneable handle to a main queue that is push-only
pub struct Pusher<R> {
    inner: WeakInnerQueue<R>,
    sender: crossbeam::channel::Sender<()>,
}

impl<R> Clone for Pusher<R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            sender: self.sender.clone(),
        }
    }
}

impl<R> Debug for Pusher<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pusher")
            .field("is_alive", &self.inner.upgrade().is_some())
            .finish()
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
            Ok(_) => {
                let _ = self.sender.try_send(());
                None
            }
            Err(record) => Some(record),
        })
    }

    /// Ensures that the value is pushed w/ a spin-loop
    #[inline]
    pub fn ensure_push(&self, mut value: R) {
        let backoff = Backoff::new();
        while let Some(r) = self.push(value) {
            value = r;
            backoff.spin();

            if self.inner.upgrade().is_none() {
                return;
            }
        }
    }
}

/// Thread-safe write queue for Records
#[derive(Debug)]
pub struct Queue<R, const INITIAL_CAPACITY: usize = DEFAULT_QUEUE_SIZE> {
    /// Inner queue data structure
    queue: Arc<crossbeam::queue::ArrayQueue<R>>,
    channel: (Option<Sender<()>>, Receiver<()>),
}

impl<R, const INITIAL_CAPACITY: usize> Queue<R, INITIAL_CAPACITY> {
    /// Creates a new queue w/ CAPACITY
    #[inline]
    pub fn new() -> Self {
        let queue = Arc::new(crossbeam::queue::ArrayQueue::new(INITIAL_CAPACITY));

        let (tx, rx) = bounded(1);
        Self {
            queue,
            channel: (Some(tx), rx),
        }
    }

    /// Returns the current length of the queue
    #[inline]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Returns true if the queue is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Returns a queue Pusher
    ///
    /// A pusher can be used to ensure records are pushed onto the queue
    ///
    /// Return None if the queue is closed and can no longer receive elements
    #[inline]
    pub fn pusher(&self) -> Option<Pusher<R>> {
        if let Some(sender) = self.channel.0.as_ref() {
            Some(Pusher {
                inner: Arc::downgrade(&self.queue),
                sender: sender.clone(),
            })
        } else {
            None
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

    /// Pops from the queue
    #[inline]
    pub fn pop(&self) -> Option<R> {
        self.queue.pop()
    }

    /// Subscribes to notifications when an item is pushed to the queue
    #[inline]
    pub fn subscribe(&self) -> Receiver<()> {
        self.channel.1.clone()
    }

    /// Starts a worker thread with a fold function and an accumulator.
    ///
    /// The thread will continue running until:
    /// - The queue is closed via `close()`, **and**
    /// - All pushers have been dropped (i.e., no more senders remain)
    ///
    /// When this happens, the thread returns the final accumulator value.
    ///
    /// If the `fold` function returns an error, the thread will exit immediately.
    ///
    /// The fold function receives a **shallow clone** of the original queue,
    /// which does **not** allow creating new pushers from within the fold context.
    #[inline]
    pub fn worker_thread_fold<Acc>(
        &self,
        acc: Acc,
        fold: impl Fn(&Self, Acc) -> std::io::Result<Acc> + Send + 'static,
    ) -> JoinHandle<std::io::Result<Acc>>
    where
        Acc: Send + 'static,
        R: Send + 'static,
    {
        let worker = Self {
            queue: self.queue.clone(),
            channel: (None, self.channel.1.clone()),
        };
        std::thread::spawn(move || {
            let mut acc = acc;
            let recv = worker.subscribe();
            while let Ok(_) = recv.recv() {
                acc = fold(&worker, acc)?;
            }
            Ok(acc)
        })
    }

    /// Starts a worker thread
    ///
    /// Note: Shortcut for worker_thread_fold((), ..)
    #[inline]
    pub fn worker_thread(
        &self,
        work: impl Fn(&Self) -> std::io::Result<()> + Send + 'static,
    ) -> JoinHandle<std::io::Result<()>>
    where
        R: Send + 'static,
    {
        self.worker_thread_fold((), move |d, _| work(d))
    }

    /// Closes the queue, signaling that no new pushers will be created.
    ///
    /// This does **not** immediately stop any worker threads.
    /// They will continue to block until all existing senders have been dropped.
    #[inline]
    pub fn close(&mut self) {
        self.channel.0.take();
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
        let shared = queue.pusher().unwrap();
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

        let shared = queue.pusher().unwrap();
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

        let shared = queue.pusher().unwrap();
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

    #[test]
    fn test_notify() {
        let mut queue = Queue::<usize, 10>::new();
        let worker_thread = queue.worker_thread_fold(0, |q, mut acc| {
            acc += q.flush().sum::<usize>();
            eprintln!("t1: {acc}");
            Ok(acc)
        });
        let worker_thread2 = queue.worker_thread_fold(0, |q, mut acc| {
            acc += q.flush().sum::<usize>();
            eprintln!("t2: {acc}");
            Ok(acc)
        });
        let pusher = queue.pusher().unwrap();
        let work = worker_thread;

        for _ in 0..1000 {
            pusher.ensure_push(10);
            pusher.ensure_push(10);
            pusher.ensure_push(10);
            pusher.ensure_push(10);
            pusher.ensure_push(10);
            pusher.ensure_push(10);
        }
        drop(pusher);
        queue.close();
        let result = work.join().unwrap().unwrap();
        let result2 = worker_thread2.join().unwrap().unwrap();
        assert_eq!(60000, result + result2);
    }
}

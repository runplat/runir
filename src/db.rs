use std::sync::Arc;

use tokio::task::JoinSet;

use crate::worker::Worker;

type SharedCell<T> = Arc<tokio::sync::RwLock<T>>;

/// Struct for database state
/// 
/// Database maintains access to filesystem artifacts and dispatches work to a pool of database workers
/// 
pub struct Database {
    /// Worker pool that maintains a set of futures for each active worker
    workers: JoinSet<Worker>,
}

impl Database {
    /// Returns a writer which may write data to the database
    pub fn writer(&self) {
    }

    /// Scale the database up by a single worker
    fn scale_up(&mut self) {
        let worker = Worker::default();
        self.workers.spawn(async {
            // 1) Worker will wait for a signal to begin work
            // 2) onc
            worker
        });
    }
}
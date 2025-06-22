use std::sync::Arc;
use ahash::HashMap;
use crate::{record::Namespace, Worker};

/// Type-alias for a share-able worker pointer
type SharedWorker = Arc<tokio::sync::RwLock<Worker>>;

/// Store coordinates multiple workers and namespaces
#[derive(Default)]
pub struct Store {
    /// Available namespaces in the store
    namespaces: HashMap<Namespace, SharedWorker>
}

impl Store {
}

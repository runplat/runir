//! # `route` module
//!
//! This module uses an object_store to act as the tunnel between two wire "terminals"
//!
//! The goal of this protocol is to send an archive member as a "msg".
//!
//! The other side of the terminal "receives" this message, and proceeds to persist the archive member to it's state.
//!
use crate::{Result, wire::ObjectStore};

use std::sync::{Arc, OnceLock};

use anyhow::anyhow;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use object_store::{
    Attributes, ObjectMeta, PutOptions, PutPayload, local::LocalFileSystem, path::Path,
    prefix::PrefixStore,
};
use time::UtcDateTime;
use tracing::{debug, error};
use uuid::Uuid;

static ROUTE_MESSAGES_PATH: std::sync::OnceLock<Path> = OnceLock::new();

fn route_messages_path() -> &'static Path {
    ROUTE_MESSAGES_PATH
        .get_or_init(|| Path::parse("runir.wire.route.messages").expect("must be a valid path"))
}

/// Route provides a system to use object_store as a message passing medium
#[derive(Debug)]
pub struct Route {
    store: ObjectStore,
}

impl Route {
    /// Returns a new route
    #[inline]
    pub fn new(store: impl object_store::ObjectStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    /// Returns the route of the current directory
    #[inline]
    pub fn current_dir() -> std::io::Result<Self> {
        Ok(Self::new(super::root_store()?))
    }

    /// Returns the route of a local filesystem path
    #[inline]
    pub fn local(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        Ok(Self::new(LocalFileSystem::new_with_prefix(path)?))
    }

    /// Creates a new branch of the message state
    ///
    /// Returns the list of received messages
    #[inline]
    pub async fn recv(&self) -> Result<Vec<ObjectMeta>> {
        use futures::StreamExt;

        let epoch = UtcDateTime::now().unix_timestamp_nanos();
        let epoch = Uuid::from_u128(epoch as u128);
        let epoch = branch(&self.store, format!(".recv_{}", epoch.as_simple()), true).await?;

        let messages = epoch
            .list(Some(route_messages_path()))
            .fold(vec![], |mut m, v| async {
                match v {
                    Ok(meta) => {
                        m.push(meta);
                        m
                    }
                    Err(e) => {
                        error!("{e}");
                        m
                    }
                }
            })
            .await;
        Ok(messages)
    }

    /// Sends a message to the route
    #[inline]
    pub async fn send(&self, message: Bytes) -> Result<()> {
        let epoch = UtcDateTime::now().unix_timestamp_nanos();
        let epoch = Uuid::from_u128(epoch as u128);
        let next_message = route_messages_path().child(epoch.as_simple().to_string());
        self.store
            .put_opts(
                &next_message,
                PutPayload::from_bytes(message),
                PutOptions {
                    mode: object_store::PutMode::Create,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| anyhow!(e))?;
        Ok(())
    }
}

#[inline]
fn contents(src: &ObjectStore, prefix: Option<&Path>) -> impl Stream<Item = ObjectMeta> {
    src.list(prefix).filter_map(|m| async {
        m.inspect_err(|e| error!("{e}"))
            .ok()
            .filter(|m| !m.location.as_ref().starts_with("."))
    })
}

/// Clones from a src object store to a destination object store
#[inline]
#[allow(unused)]
async fn clone(src: &ObjectStore, dest: &ObjectStore) -> crate::Result<()> {
    contents(src, None)
        .for_each_concurrent(None, |meta| async move {
            match src.get(&meta.location).await {
                Ok(mut clone_from) => {
                    let options = PutOptions {
                        mode: object_store::PutMode::Create,
                        attributes: std::mem::replace(
                            &mut clone_from.attributes,
                            Attributes::new(),
                        ),
                        ..Default::default()
                    };
                    let bytes = clone_from.bytes().await.unwrap();
                    dest.put_opts(&meta.location, PutPayload::from_bytes(bytes), options)
                        .await
                        .inspect_err(|e| error!("{e}"))
                        .ok();
                }
                Err(err) => {
                    error!("Couldn't get object from src: {err}");
                }
            }
        })
        .await;
    Ok(())
}

/// Returns a branch from a store
///
/// If checkout is true, will clone the current state of the store to branch
#[inline]
async fn branch(
    store: &ObjectStore,
    branch: impl AsRef<str>,
    use_checkout: bool,
) -> crate::Result<ObjectStore> {
    if use_checkout {
        let branch = checkout(store, branch).await?;
        Ok(branch)
    } else {
        let branch_prefix = branch_prefix(branch);
        let branch: ObjectStore = Arc::new(PrefixStore::new(store.clone(), branch_prefix.as_str()));
        Ok(branch)
    }
}

#[inline]
async fn checkout(src: &ObjectStore, branch: impl AsRef<str>) -> Result<ObjectStore> {
    let branch = branch_prefix(branch);
    contents(src, None)
        .for_each_concurrent(None, |m| {
            let branch: Path = branch.clone().as_str().into();
            async move {
                let from = &m.location;
                let to = from.parts().fold(branch, |b, p| b.child(p));
                match src.copy_if_not_exists(from, &to).await {
                    Ok(_) => {
                        debug!("Copied `{from}` to `{to}`");
                    }
                    Err(err) => {
                        error!("{err}");
                    }
                }
            }
        })
        .await;
    Ok(Arc::new(PrefixStore::new(src.clone(), branch.as_str())))
}

fn branch_prefix(branch: impl AsRef<str>) -> Arc<String> {
    Path::parse(branch.as_ref()).expect("Must be a valid branch name");
    let branch = Arc::new(format!(".branches/{}", branch.as_ref()));
    branch
}

#[cfg(test)]
mod tests {
    use crate::wire::route::ObjectStore;
    use object_store::{PutPayload, path::Path};
    use std::sync::Arc;

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_clone() {
        let test_1 = Path::from("test");
        let test_2 = Path::from("test/test2");
        let src: ObjectStore = Arc::new(object_store::memory::InMemory::new());
        src.put(&test_1, PutPayload::from_static(b"hello world"))
            .await
            .unwrap();
        src.put(&test_2, PutPayload::from_static(b"goodbye world"))
            .await
            .unwrap();

        // Test initial clone into a destination
        let dest: ObjectStore = Arc::new(object_store::memory::InMemory::new());
        super::clone(&src, &dest).await.unwrap();

        // Test using "checkout"
        let checkout = super::checkout(&src, "test-branch").await.unwrap();

        let test_1_bytes = dest.get(&test_1).await.unwrap().bytes().await.unwrap();
        assert_eq!(test_1_bytes.as_ref(), b"hello world");

        let test_2_bytes = dest.get(&test_2).await.unwrap().bytes().await.unwrap();
        assert_eq!(test_2_bytes.as_ref(), b"goodbye world");

        let test_1_bytes = checkout.get(&test_1).await.unwrap().bytes().await.unwrap();
        assert_eq!(test_1_bytes.as_ref(), b"hello world");

        let test_2_bytes = checkout.get(&test_2).await.unwrap().bytes().await.unwrap();
        assert_eq!(test_2_bytes.as_ref(), b"goodbye world");

        eprintln!("{checkout:#?}");
    }
}

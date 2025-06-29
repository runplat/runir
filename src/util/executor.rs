use std::sync::OnceLock;

use futures::{FutureExt, future::RemoteHandle};

/// Abstracts the async runtime to a trait
/// 
/// TODO: Investigating if it's possible to make this agnostic
pub trait Executor {
    /// Spawns a future, or returns an error indicating spawning futures is unavailable
    fn spawn<T, F>(&self, fut: F) -> std::io::Result<RemoteHandle<T>>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static;
}

/// Inline tokio runtime in-case context is not running in a tokio runtime
static TOKIO_INLINE: OnceLock<std::io::Result<tokio::runtime::Runtime>> = OnceLock::new();

fn get_tokio() -> std::io::Result<tokio::runtime::Handle> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => Ok(handle),
        Err(_) => match TOKIO_INLINE.get_or_init(|| {
            Ok(tokio::runtime::Builder::new_multi_thread()
                .enable_io()
                .build()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Unsupported, e))?)
        }) {
            Ok(rt) => Ok(rt.handle().clone()),
            Err(err) => Err(std::io::Error::new(err.kind(), err.to_string())),
        },
    }
}

/// Detects if running in a tokio context, and returns that handle
/// 
/// Otherwise starts a new tokio runtime for i/o and returns a handle the new runtime
pub struct TokioExecutor;

impl Executor for TokioExecutor {
    fn spawn<T, F>(&self, fut: F) -> std::io::Result<RemoteHandle<T>>
    where
        T: Send,
        F: Future<Output = T> + Send + 'static,
    {
        let (remote, handle) = fut.into_future().remote_handle();
        let rt = get_tokio()?;
        rt.spawn(remote);
        Ok(handle)
    }
}

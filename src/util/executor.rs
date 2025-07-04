/// Abstracts the async runtime to a trait
pub trait Executor {
    /// Returns true if this executor can execute now
    fn can_execute_now(&self) -> bool;

    /// Returns true if this executor can execute inline
    fn can_execute_inline(&self) -> bool;

    /// Spawns a future, or returns an error indicating spawning futures is unavailable
    fn spawn<T, F>(&self, fut: F) -> std::io::Result<futures::future::RemoteHandle<T>>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static;
}

#[cfg(feature = "tokio")]
mod tokio {
    use std::sync::OnceLock;
    use futures::{FutureExt, future::RemoteHandle};

    use super::Executor;

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
            let (remote, handle) = fut.remote_handle();
            let rt = get_tokio()?;
            rt.spawn(remote);
            Ok(handle)
        }
        
        fn can_execute_now(&self) -> bool {
            tokio::runtime::Handle::try_current().is_ok()
        }
        
        fn can_execute_inline(&self) -> bool {
            get_tokio().is_ok()
        }
    }
}

#[cfg(feature = "tokio")]
pub use tokio::TokioExecutor;
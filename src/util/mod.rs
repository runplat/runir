mod container;
pub use container::Container;

mod peek;
pub use peek::Peek;
pub use peek::PeekPath;
pub use peek::PeekExtensions;
pub use peek::PeekRefExtensions;

mod executor;
pub use executor::Executor;

pub mod fs;

use futures::future::RemoteHandle;

/// Util for spawning futures into a runtime
#[inline]
pub fn spawn<T: Send + 'static>(
    fut: impl Future<Output = T> + Send + 'static,
) -> std::io::Result<RemoteHandle<T>> {
    #[cfg(feature = "tokio")]
    executor::TokioExecutor.spawn(fut)
}

/// Util for spawning futures into a runtime
#[inline]
pub fn spawn_blocking<F, R, O>(fut: F) -> impl Future<Output = O>
where
    F: FnOnce() -> R,
    R: Future<Output = O>,
    O: Send + 'static,
{
    #[cfg(feature = "tokio")]
    executor::TokioExecutor.spawn_blocking(fut).unwrap()
}

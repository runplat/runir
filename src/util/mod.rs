mod name;
use futures::future::RemoteHandle;
pub use name::Name;

mod executor;
pub use executor::Executor;
pub use executor::TokioExecutor;

/// Util for spawning futures into a runtime
#[inline]
pub fn spawn<T: Send + 'static>(
    fut: impl Future<Output = T> + Send + 'static,
) -> std::io::Result<RemoteHandle<T>> {
    TokioExecutor.spawn(fut)
}

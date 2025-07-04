mod name;
pub use name::Name;

mod peek;
pub use peek::Peek;
pub use peek::PeekExtensions;

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

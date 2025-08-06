mod container;
pub use container::Layer;
pub use container::Container;
pub use container::ILayerDescriptor;

mod packer;
pub(crate) use packer::Packer;

mod interner;
pub use interner::Intern;
pub(crate) use interner::impl_interner;

mod peek;
pub use peek::peek_ser;
pub use peek::Peek;
pub use peek::PeekPath;
pub use peek::PeekExtensions;
pub use peek::PeekRefExtensions;

mod executor;
pub use executor::Executor;

mod run;
pub use run::RunCell;

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

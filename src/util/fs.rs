mod tokio {
    use futures::{AsyncRead, AsyncSeek, AsyncWrite};
    use std::path::Path;
    use tokio_util::compat::TokioAsyncReadCompatExt;

    /// Opens a file for read/write
    #[inline]
    pub async fn open_rw(
        path: impl AsRef<Path>,
    ) -> std::io::Result<impl AsyncWrite + AsyncRead + AsyncSeek + Send + 'static> {
        Ok(tokio::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .open(path)
            .await?
            .compat())
    }

    /// Opens a file for reading
    #[inline]
    pub async fn open(path: impl AsRef<Path>) -> std::io::Result<impl AsyncRead + Send + 'static> {
        Ok(tokio::fs::File::open(path).await?.compat())
    }

    /// Creates a new file
    #[inline]
    pub async fn create_new(
        path: impl AsRef<Path>,
    ) -> std::io::Result<impl AsyncRead + AsyncWrite + Send + 'static> {
        Ok(tokio::fs::File::create_new(path).await?.compat())
    }
}

pub use tokio::*;

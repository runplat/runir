use std::fmt::{Debug, Display};

/// Error type for crate
pub struct Error {
    /// Inner error type
    inner: Inner,
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self {
            inner: Inner::Stdio(value),
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(value: anyhow::Error) -> Self {
        Self {
            inner: Inner::Custom(value),
        }
    }
}

impl From<flexbuffers::SerializationError> for Error {
    fn from(value: flexbuffers::SerializationError) -> Self {
        anyhow::anyhow!(value).into()
    }
}

impl From<flexbuffers::DeserializationError> for Error {
    fn from(value: flexbuffers::DeserializationError) -> Self {
        anyhow::anyhow!(value).into()
    }
}

impl From<flexbuffers::ReaderError> for Error {
    fn from(value: flexbuffers::ReaderError) -> Self {
        anyhow::anyhow!(value).into()
    }
}


#[derive(Debug)]
enum Inner {
    Stdio(std::io::Error),
    Custom(anyhow::Error),
}

#[derive(thiserror::Error, Debug)]
enum RecordManagement {
    #[error("Record has been marked for deletion")]
    Delete(u64)
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner {
            Inner::Stdio(error) => write!(f, "{error}"),
            Inner::Custom(error) => write!(f, "{error}"),
        }
    }
}

impl Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Error").field("inner", &self.inner).finish()
    }
}

impl std::error::Error for Error {}

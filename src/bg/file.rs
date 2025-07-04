use std::{path::PathBuf, sync::OnceLock};
use anyhow::anyhow;
use tracing::{debug, error};
use super::{Address, Container, Handle, Object, Result, Scheme};

fn to_path_buf(address: &Address) -> Result<PathBuf> {
    match address.container {
        Container::Staging => Ok(std::env::temp_dir().join(address.name())),
        Container::Work => Ok(std::env::current_dir()?.join(address.name())),
        Container::Shared => Ok(shared_dir()?.join(address.name())),
        Container::Unknown => {
            Err(anyhow!("Cannot convert an unknown container to a path buf").into())
        }
    }
}

impl Scheme for std::path::PathBuf {
    const PREFIX: &'static str = "std";

    fn open(address: &Address) -> Result<Handle> {
        if address.scheme != Self::PREFIX {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Address is not supported for this scheme",
            )
            .into());
        }

        let opened = std::fs::File::open(to_path_buf(address)?)?;
        Ok(Handle {
            address: address.clone(),
            inner: Box::new(opened),
        })
    }

    fn create_new(address: &Address) -> Result<Handle> {
        if address.scheme != Self::PREFIX {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Address is not supported for this scheme",
            )
            .into());
        }

        let created = std::fs::File::create_new(to_path_buf(address)?)?;
        Ok(Handle {
            address: address.clone(),
            inner: Box::new(created),
        })
    }

    fn rw_open(address: &Address) -> Result<Handle> {
        if address.scheme != Self::PREFIX {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Address is not supported for this scheme",
            )
            .into());
        }

        let opened = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(to_path_buf(address)?)?;
        Ok(Handle {
            address: address.clone(),
            inner: Box::new(opened),
        })
    }

    fn move_obj(address: &Address, to: Container) -> Result<()> {
        let from = to_path_buf(address)?;
        let address = match to {
            Container::Staging => {
                address.clone().to_staging()
            },
            Container::Work => {
                address.clone().to_work()
            },
            Container::Shared => {
                address.clone().to_shared()
            },
            Container::Unknown => {
                address.clone()
            }
        };

        std::fs::rename(from, to_path_buf(&address)?)?;
        Ok(())
    }

    fn create(container: Container) -> Result<()> {
        debug!("Creating {container:?}");
        match container {
            Container::Staging => todo!(),
            Container::Work => todo!(),
            Container::Shared => todo!(),
            Container::Unknown => Err(std::io::Error::new(
                std::io::ErrorKind::DirectoryNotEmpty,
                "Could not create a directory for an unknown container",
            )
            .into()),
        }
    }
}

impl From<std::path::PathBuf> for Address {
    fn from(value: std::path::PathBuf) -> Self {
        let container = if value.starts_with(std::env::temp_dir()) {
            Container::Staging
        } else if std::env::current_dir()
            .ok()
            .map(|d| value.starts_with(d))
            .unwrap_or_default()
        {
            Container::Work
        } else if std::env::var("RUNIR_SHARED_HOME")
            .ok()
            .map(|d| value.starts_with(d))
            .unwrap_or_default()
        {
            Container::Shared
        } else {
            Container::Unknown
        };

        Self {
            scheme: std::path::PathBuf::PREFIX,
            container,
            object: Object::Query(
                value
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ),
        }
    }
}

impl super::handle::ObjectHandle for std::fs::File {}

const SHARED_DIR_ENV_VAR: &str = "RUNIR_SHARED_HOME";
static SHARED_DIR_ENV: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
/// Returns the directory for RUNIR_SHARED_HOME
///
/// If the directory specified does not exist or is not a directory, then this function
/// will return None even if the env variable is set
#[inline]
pub fn shared_dir() -> Result<std::path::PathBuf> {
    let shared = SHARED_DIR_ENV.get_or_init(|| match std::env::var(SHARED_DIR_ENV_VAR) {
        Ok(dir) => {
            let dir = PathBuf::from(dir);

            if dir.exists() && dir.is_dir() {
                Some(dir)
            } else {
                error!("RUNIR_SHARED_HOME was set to an invalid value {dir:?}, ignoring variable");
                None
            }
        }
        Err(err) => {
            debug!("shared_dir(..) not set b/c: {err}");
            None
        }
    });

    if let Some(shared) = shared {
        Ok(shared.to_path_buf())
    } else {
        Err(anyhow!("Could not get shared dir").into())
    }
}

use tracing::debug;

use super::{Address, Container, Handle, Object, Result, Scheme};

impl Scheme for std::path::PathBuf {
    const PREFIX: &'static str = "std";

    fn open(address: &Address) -> Result<Handle> {
        if address.scheme != Self::PREFIX {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "std::fs::PathBuf is unsupported for this address",
            )
            .into());
        }

        // TODO: Need to finish what handle does

        Ok(Handle {
            address: address.clone(),
        })
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

    fn create_new(address: &Address) -> Result<Handle> {
        todo!()
    }

    fn rw_open(address: &Address) -> Result<Handle> {
        todo!()
    }

    fn move_obj(address: &Address, to: Container) -> Result<Handle> {
        todo!()
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

// use std::{path::PathBuf, sync::OnceLock};
// use tracing::{debug, error};
// const SHARED_DIR_ENV_VAR: &str = "RUNIR_SHARED_HOME";
// static SHARED_DIR_ENV: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
// /// Returns the directory for RUNIR_SHARED_HOME
// ///
// /// If the directory specified does not exist or is not a directory, then this function
// /// will return None even if the env variable is set
// #[inline]
// pub fn shared_dir() -> &'static Option<std::path::PathBuf> {
//     SHARED_DIR_ENV.get_or_init(|| match std::env::var(SHARED_DIR_ENV_VAR) {
//         Ok(dir) => {
//             let dir = PathBuf::from(dir);

//             if dir.exists() && dir.is_dir() {
//                 Some(dir)
//             } else {
//                 error!("RUNIR_SHARED_HOME was set to an invalid value {dir:?}, ignoring variable");
//                 None
//             }
//         }
//         Err(err) => {
//             debug!("shared_dir(..) not set b/c: {err}");
//             None
//         }
//     })
// }

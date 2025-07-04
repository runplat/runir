//! # Frontend Module
//! 
//! This module provides several frontend "porcelain" for interacting with the overall store system.
//! 
//! Each frontend is tailored towards it's specific canonical goal, with some added improvements for developer experience
//! 
//! ## Available Frontends
//! 
//! - `kv`: Provides `put`/`get` as the canonical kv store api, and `put<S: Serialize>/load<D: Deserialze>/peek (directly at flexbuffer reader w/o allocating)` functions in "serde" mode
//! 
//! ## Writing a custom frontend
//! 
//! Each frontend implements the `Frontend` trait, which provides some common runtime settings. In addition each frontend
//! is based on a "SharedState" object which can be cloned and distributed across different threads. "SharedState" surfaces
//! access to the internal Store via ::store(), and a function state() which provides access to common state functions (saving, importing, flushing, etc.)
//! 

pub mod kv;
pub mod state;

pub mod prelude {
    pub use super::Frontend;
    pub use super::kv::Get;
    pub use super::kv::Put;
    pub use super::kv;
}

/// Common trait for frontends to implement to provide common utilities
pub trait Frontend {
    /// Name of the frontend
    const NAME: &str;

    /// Formatted archive name
    const ARCHIVE_NAME: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

    /// Load the frontend from shared state
    fn from_shared(shared: state::SharedState) -> Self;

    /// Creates a new frontend
    fn new() -> Self
    where
        Self: Sized,
    {
        let mut state = state::State::default();
        state.set_frontend::<Self>(Self::next_instance_id());

        let shared = state::SharedState::from(state);
        Self::from_shared(shared)
    }

    /// Returns the next instance-id
    /// 
    /// Instance ID is an increasing integer shared by all frontends, used for tracking
    fn next_instance_id() -> usize {
        static INSTANCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    
        INSTANCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns the archive name of this frontend
    fn archive_name() -> &'static str {
        Self::ARCHIVE_NAME.get_or_init(|| Box::leak(format!("{}.tar", Self::NAME).into_boxed_str()))
    }

    /// Returns true if the "default" store exists
    /// 
    /// Setting a RUNIR_WORK_DIR env variable can be used to configure this directory
    ///
    /// Returns an error if file system permissions do not exist
    fn default_store_exists() -> std::io::Result<bool> {
        let dot_folder = format!(".{}", env!("CARGO_PKG_NAME"));
        let dir = std::env::var("RUNIR_WORK_DIR")
            .map(|w| std::path::PathBuf::from(w))
            .ok()
            .unwrap_or(std::env::current_dir()?.join(&dot_folder));

        Ok(dir.join(Self::archive_name()).exists())
    }

    /// Returns a path to the default "save" directory
    ///
    /// Setting a RUNIR_WORK_DIR env variable can be used to configure this directory
    /// 
    /// Returns an error if file system permissions do not exist
    fn default_save_dir() -> std::io::Result<std::path::PathBuf> {
        let dot_folder = format!(".{}", env!("CARGO_PKG_NAME"));
        let dir = std::env::var("RUNIR_WORK_DIR")
            .map(|w| std::path::PathBuf::from(w))
            .ok()
            .unwrap_or(std::env::current_dir()?.join(&dot_folder));

        if !dir.exists() && dir.ends_with(dot_folder) {
            tracing::debug!("Creating .runir folder {dir:?}");
            std::fs::create_dir(&dir)?;
        } else if !dir.exists() {
            // Since this is passed into the process, do not attempt to create directory unless opt-in
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Directory set in RUNIR_WORK_DIR must exist",
            ));
        }

        Ok(dir)
    }
}

/// Opens a frontend from a default save directory if it exists, 
/// OR returns a new frontend
/// 
/// An error is returned if the Default save directory could not be opened
#[inline]
pub async fn open<F: Frontend>() -> std::io::Result<F> {
    if F::default_store_exists()? {
        let save_dir = F::default_save_dir()?;
        return Ok(open_dir(save_dir).await?);
    } else {
        Ok(F::new())
    }
}

/// Opens a frontend from a default save directory
///
/// Unlike open(..), if the default store does not exist at this directory, an error will be returned
#[inline]
pub async fn open_dir<F: Frontend>(dir: impl Into<std::path::PathBuf>) -> std::io::Result<F> {
    let state = state::State::load::<F>(dir).await?;
    let shared = state::SharedState::from(state);
    shared.update_snapshot();
    Ok(F::from_shared(shared))
}

/// Saves a frontend to the default save directory
#[inline]
pub async fn save<F: Frontend + AsRef<state::SharedState>>(frontend: &F) -> std::io::Result<()> {
    let save_dir = F::default_save_dir()?;
    frontend.as_ref().state.save(save_dir).await?;
    Ok(())
}

/// Saves a frontend to a specific save directory
#[inline]
pub async fn save_as<F: Frontend + AsRef<state::SharedState>>(
    frontend: &F,
    to: impl Into<std::path::PathBuf>,
) -> std::io::Result<()> {
    frontend.as_ref().state.save(to.into()).await
}

#[cfg(test)]
mod test {
    use super::Frontend;

    struct Test;
    struct Test2;

    impl Frontend for Test {
        const NAME: &str = "test";

        fn from_shared(_: super::state::SharedState) -> Self {
            Self
        }
    }

    impl Frontend for Test2 {
        const NAME: &str = "test2";

        fn from_shared(_: super::state::SharedState) -> Self {
            Self
        }
    }

    #[test]
    fn test_frontend_archive_name() {
        assert_eq!("test.tar", Test::archive_name());
        assert_eq!("test2.tar", Test2::archive_name());
    }

    #[test]
    fn test_frontend_instance_id() {
        assert_eq!(0, Test::next_instance_id());
        assert_eq!(1, Test::next_instance_id());
        assert_eq!(2, Test2::next_instance_id());
    }
}

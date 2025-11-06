use std::{path::PathBuf, sync::OnceLock};

use tracing::debug;

const RUNIR_DIR_VAR_NAME: &str = "RUNIR_DIR";

static RUNIR_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Initializes the default runir environment settings
///
/// Returns an error if environment could not be initialized, should be called at the edge such that
#[inline]
pub fn init_env() -> std::io::Result<()> {
    if RUNIR_DIR.get().is_none() {
        debug!("Initializing {RUNIR_DIR_VAR_NAME} setting");
        std::fs::create_dir_all(runir_dir())?;
    }
    Ok(())
}

/// Returns the runir_dir in-use by the current process
#[inline]
pub fn runir_dir() -> &'static PathBuf {
    RUNIR_DIR.get_or_init(init_runir_dir)
}

/// Initialize the runir dir path
fn init_runir_dir() -> PathBuf {
    PathBuf::from(
        std::env::var(RUNIR_DIR_VAR_NAME)
            .map(|v| PathBuf::from(v).join(".runir"))
            .unwrap_or_else(|e| {
                debug!("{RUNIR_DIR_VAR_NAME} is not set, trying current directory: {e}");
                std::env::current_dir().map(|c| c).unwrap_or_else(|e| {
                    debug!("Could not access current directory, falling back to temporary directory: {e}");
                    std::env::temp_dir().join(".runir")
                })
            }),
    )
}

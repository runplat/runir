use clap::Args;

use super::ObjectType;

/// Args for looking up records
#[derive(Args)]
pub struct LookupRecord {
    /// Formats the output to a specific object format
    ///
    /// If the stored record is not an object, this argument will be ignored.
    #[clap(long, short = 'o')]
    pub format: Option<ObjectType>,
    /// Record key
    pub key: String,
}

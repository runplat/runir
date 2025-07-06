use clap::Args;

use super::{ObjectFormat, ObjectFormatArgs};

/// Args for looking up records
#[derive(Args)]
pub struct LookupRecord {
    /// Formats the output to a specific object format
    ///
    /// If the stored record is not an object, this argument will be ignored.
    #[clap(short)]
    pub object_format: Option<ObjectFormat>,
    #[clap(flatten)]
    pub format: Option<ObjectFormatArgs>,
    /// Record label
    pub label: String,
}

use clap::Args;
use super::Format;

/// Args for looking up records
#[derive(Args)]
pub struct LookupRecord {
    #[clap(flatten)]
    pub format: Option<Format>,
    /// Record label
    pub label: String,
}

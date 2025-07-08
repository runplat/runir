use clap::Args;
use super::Format;

/// Args for looking up records
#[derive(Args)]
pub struct LookupRecord {
    #[clap(flatten)]
    pub format: Format,
    #[clap(long)]
    pub peek: Option<String>,
    /// Record label
    pub label: String,
}

use clap::Args;
use super::Format;

/// Args for looking up records
#[derive(Args)]
pub struct LookupRecord {
    #[clap(flatten)]
    pub format: Format,
    #[clap(long, short)]
    pub peek: Option<String>,
    /// Treats the label as a digest (sha256 prefix can be optionally included)
    #[clap(long, short)]
    pub digest: bool,
    /// Record label
    pub label: String,
}

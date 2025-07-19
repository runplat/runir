use clap::Args;

/// Provides interface and functions for creating records
#[derive(Args, Debug)]
pub struct DeleteRecord {
    /// Record label
    pub label: String,
}
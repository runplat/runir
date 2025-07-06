use std::path::PathBuf;
use clap::Args;
use runir::{Namespace, Record};
use tokio::io::AsyncReadExt;
use tracing::debug;
use super::ObjectFormat;
use super::Format;

/// Provides interface and functions for creating records
#[derive(Args)]
pub struct CreateRecord {
    #[clap(flatten)]
    format: Option<Format>,
    /// Path to the file to create the record from
    ///
    /// If a path is not set, then the default input will be read from stdin
    #[clap(long, short)]
    file: Option<PathBuf>,
    /// Record label
    label: String,
}

impl CreateRecord {
    pub async fn build(&self, ns: Namespace) -> std::io::Result<Record> {
        let CreateRecord { format, file, label } = self;
        if let Some(format) = format.as_ref().and_then(|f| f.resolve()) {
            debug!("Object format enabled {format:?}");
            match format {
                ObjectFormat::Yaml => {
                    // TODO
                }
                ObjectFormat::Json => {
                    // TODO
                }
                ObjectFormat::Toml => {
                    let toml = if let Some(file) = file {
                        tokio::fs::read_to_string(file).await?
                    } else {
                        let mut toml_content = String::new();
                        tokio::io::stdin().read_to_string(&mut toml_content).await?;
                        toml_content
                    };

                    let toml = toml::from_str::<toml::Value>(&toml)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                    return Ok(ns.store(&label, &toml))
                }
            }
        } else {
        }

        todo!()
    }
}

use super::Format;
use super::ObjectFormat;
use clap::Args;
use runir::{Namespace, Record};
use std::path::PathBuf;
use tokio::io::AsyncReadExt;
use tracing::debug;

/// Provides interface and functions for creating records
#[derive(Args, Debug)]
pub struct CreateRecord {
    #[clap(flatten)]
    format: Format,
    /// Path to the file to create the record from
    ///
    /// If a path is not set, then the default input will be read from stdin
    #[clap(long, short)]
    file: Option<PathBuf>,
    /// Record label
    label: String,
}

impl CreateRecord {
    /// Executes record creation
    #[inline]
    pub async fn execute(&self, ns: Namespace) -> std::io::Result<Record> {
        debug!("{self:?}");
        let CreateRecord {
            format,
            file,
            label,
        } = self;

        if let Some(format) = format.resolve() {
            debug!("Object format enabled {format:?}");
            let content = if let Some(file) = file {
                debug!("Reading content from {file:?}");
                tokio::fs::read_to_string(file).await?
            } else {
                debug!("Reading content from stdin");
                let mut content = String::new();
                tokio::io::stdin().read_to_string(&mut content).await?;
                content
            };

            match format {
                ObjectFormat::Yaml => {
                    let yaml = serde_yaml::from_str::<serde_yaml::Value>(&content)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                    Ok(ns.store(&label, &yaml))
                }
                ObjectFormat::Json => {
                    let json = serde_json::from_str::<serde_json::Value>(&content)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                    Ok(ns.store(&label, &json))
                }
                ObjectFormat::Toml => {
                    let toml = toml::from_str::<toml::Value>(&content)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                    Ok(ns.store(&label, &toml))
                }
            }
        } else {
            let bytes = if let Some(file) = file {
                debug!("Reading content from {file:?}, as bytes");
                let mut bytes = vec![];
                tokio::fs::File::open(file)
                    .await?
                    .read_to_end(&mut bytes)
                    .await?;
                bytes
            } else {
                debug!("Reading content from stdin, as bytes");
                let mut bytes = vec![];
                tokio::io::stdin().read_to_end(&mut bytes).await?;
                bytes
            };

            debug!("Storing content as raw binary data");
            Ok(ns.commit(&label, bytes.as_slice()))
        }
    }
}

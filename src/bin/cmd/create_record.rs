use std::path::PathBuf;

use clap::Args;
use runir::{Namespace, Record};
use tokio::io::AsyncReadExt;
use tracing::debug;

use crate::cmd::ObjectFormat;

use super::ObjectFormatArgs;

/// Provides interface and functions for creating records
#[derive(Args)]
pub struct CreateRecord {
    /// Value being stored is to be recognized as an Object type
    ///
    /// When object types are stored as records they will be deserialized and reserialized into runir's internal format.
    /// This means that, the content digest of the original data will not be saved to the record
    ///
    /// If no option is used, content will be committed to the record as-is.
    #[clap(short)]
    object_format: Option<ObjectFormat>,
    #[clap(flatten)]
    format: Option<ObjectFormatArgs>,
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
        let CreateRecord { format: object_format, object_format: object_ty, file, label } = self;
        if let Some(obj) = object_format.clone().map(|o| o.resolve()).or(object_ty.clone()) {
            debug!("Object format enabled {obj:?}");
            match obj {
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

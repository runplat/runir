use std::{path::PathBuf, process::exit};

use clap::{Args, Parser, Subcommand, ValueEnum};
use runir::{IRecord, ToNamespace, frontend::Frontend};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::debug;

/// CLI Utilities for development w/ `runir` frontends
#[derive(Parser)]
struct Runir {
    /// Namespace setting
    ///
    /// All records are stored under a namespace.
    ///
    /// If no namespace is specified, the default namespace will be used
    #[clap(short, long = "ns", default_value = "", env = "RUNIR_NS")]
    namespace: String,
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// `kv` frontend commands
    #[clap(name = "kv")]
    KV(KvConfig),
}

#[derive(Args)]
struct KvConfig {
    #[clap(subcommand)]
    command: KvCommands,
}

#[derive(Subcommand)]
enum KvCommands {
    /// Puts a record into the kv store
    Put(CreateRecord),
    /// Gets a record from the kv store
    Get(LookupRecord),
}

#[derive(Args)]
struct KVSystem {}

/// Args for looking up records
#[derive(Args)]
struct LookupRecord {
    /// Formats the output to a specific object format
    ///
    /// If the stored record is not an object, this argument will be ignored.
    #[clap(long, short = 'o')]
    format: Option<ObjectType>,
    /// Record key
    key: String,
}

#[derive(Args)]
struct CreateRecord {
    /// Value being stored is to be recognized as an Object type
    ///
    /// When object types are stored as records they will be deserialized and reserialized into runir's internal format.
    /// This means that, the content digest of the original data will not be saved to the record
    ///
    /// If no option is used, content will be committed to the record as-is.
    #[clap(long, short)]
    object: Option<ObjectType>,
    /// Path to the file to create the record from
    ///
    /// If a path is not set, then the default input will be read from stdin
    #[clap(long, short)]
    file: Option<PathBuf>,
    /// Record key
    key: String,
}

#[derive(ValueEnum, Clone, Debug)]
enum ObjectType {
    Yaml,
    Json,
    Toml,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let filter = tracing_subscriber::EnvFilter::from_default_env();
       // .add_directive("runir=debug".parse().unwrap());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .compact()
        .init();

    let program = Runir::parse();

    let ns = program.namespace.to_namespace();

    match program.command {
        Commands::KV(kv_config) => {
            use runir::kv::*;

            let kv = if !KeyValue::default_store_exists().unwrap_or_default() {
                debug!("Initializing default store");
                let mut kv = runir::kv::open().await?;
                kv.ns(ns);
                kv.put(
                    "__init__runir__version",
                    &env!("CARGO_PKG_VERSION").as_bytes(),
                )?;
                kv.save().await?;
                kv
            } else {
                let kv = runir::kv::open().await?.ns(ns);
                kv
            };

            match kv_config.command {
                KvCommands::Put(create_record) => {
                    let CreateRecord { object, file, key } = create_record;
                    if let Some(obj) = object {
                        debug!("Object format enabled {obj:?}");
                        match obj {
                            ObjectType::Yaml => {
                                // TODO
                            }
                            ObjectType::Json => {
                                // TODO
                            }
                            ObjectType::Toml => {
                                // TODO
                                let toml = if let Some(file) = file {
                                    tokio::fs::read_to_string(file).await?
                                } else {
                                    let mut toml_content = String::new();
                                    tokio::io::stdin().read_to_string(&mut toml_content).await?;
                                    toml_content
                                };

                                let toml = toml::from_str::<toml::Value>(&toml).map_err(|e| {
                                    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
                                })?;

                                kv.serde().put(&key, &toml)?;
                                kv.refresh().await?;
                                kv.save().await?;
                            }
                        }
                    } else {
                    }
                }
                KvCommands::Get(lookup_record) => {
                    let LookupRecord { key, format } = lookup_record;

                    if let Some(value) = kv.get_raw(&key) {
                        if value.opts().is_object() {
                            match format {
                                Some(format) => match format {
                                    ObjectType::Yaml => todo!(),
                                    ObjectType::Json => todo!(),
                                    ObjectType::Toml => {
                                        let toml = value.load::<toml::Value>().unwrap();
                                        println!("{}", toml::to_string_pretty(&toml).unwrap());
                                    }
                                },
                                None => todo!(),
                            }
                        } else {
                            tokio::io::stdout().write_all(value.bytes()).await?;
                        }
                    } else {
                        eprintln!("Object not found");
                        exit(1)
                    }
                }
            }
        }
    }

    Ok(())
}

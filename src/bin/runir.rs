mod cmd;
use cmd::LookupRecord;
use cmd::CreateRecord;
use cmd::ObjectFormat;
use runir::util::PeekExtensions;

use std::process::exit;
use clap::{Args, Parser, Subcommand};
use runir::{IRecord, ToNamespace, frontend::Frontend};
use tokio::io::AsyncWriteExt;
use tracing::debug;

/// CLI Utilities for development with `runir` frontends
#[derive(Parser)]
struct Runir {
    /// Enables and outputs debug logs to stderr
    #[arg(long, short)]
    debug: bool,
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
    /// Shows information for a stored record
    Info(LookupRecord),
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let program = Runir::parse();

    let mut filter = tracing_subscriber::EnvFilter::from_default_env(); 
    
    if program.debug {
        filter = filter.add_directive("runir=debug".parse().unwrap());   
    }

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .compact()
        .init();

    let ns = program.namespace.to_namespace();

    match program.command {
        Commands::KV(kv_config) => {
            use runir::kv::*;

            let kv = if !KeyValue::default_store_exists().unwrap_or_default() {
                debug!("Initializing default store");
                let mut kv = runir::kv::open().await?;
                kv.put(
                    "__init__runir__version",
                    &env!("CARGO_PKG_VERSION").as_bytes(),
                )?;
                kv.save().await?;
                kv
            } else {
                let kv = runir::kv::open().await?;
                kv
            };

            match kv_config.command {
                KvCommands::Put(create_record) => {
                    let record = create_record.build(ns).await?;
                    kv.put_raw(record)?;
                    kv.refresh().await?;
                    kv.save().await?;
                    eprintln!("Stored");
                    return Ok(());
                }
                KvCommands::Get(lookup_record) => {
                    let LookupRecord {label,format, peek } = lookup_record;
                    if let Some(value) = kv.ns(ns).get_raw(&label) {
                        if value.opts().is_object() {
                            if let Some(peek) = peek {
                                if let Some(s) = value.field(&peek).str() {
                                    println!("{s}");
                                }
                            } else {
                                match format.resolve() {
                                    Some(format) => match format {
                                        ObjectFormat::Yaml => todo!(),
                                        ObjectFormat::Json => todo!(),
                                        ObjectFormat::Toml => {
                                            let toml = value.load::<toml::Value>().unwrap();
                                            println!("{}", toml::to_string_pretty(&toml).unwrap());
                                        }
                                    },
                                    None => todo!(),
                                }
                            }
                        } else {
                            tokio::io::stdout().write_all(value.bytes()).await?;
                        }
                    } else {
                        eprintln!("Object not found");
                        exit(1)
                    }
                },
                KvCommands::Info(lookup_record) => {
                    let LookupRecord { label, .. } = lookup_record;

                    if let Some(value) = kv.get_raw(&label) {
                        let content = format!("sha256:{}", hex::encode(value.content()));
                        let is_virtual = value.is_virtual();
                        let is_valid = value.is_valid();
                        let uuid = value.uuid().as_simple().to_string();
                        let opts = value.opts();
                        let is_archivable =  opts.is_archivable();
                        let is_idempotent = opts.is_idempotent();
                        let is_indexable = opts.is_indexable();
                        let is_object = opts.is_object();
                        let is_manifest = opts.is_manifest();
                        let size = value.bytes().len();
                        let ts = time::UtcDateTime::from_unix_timestamp(value.ts() as i64).expect("should be valid timestamp").to_string();
                        let age = format!("{:#?}", value.age());
                        
                        println!("{}", toml::toml! {
                            uuid = uuid
                            ts = ts

                            [data]
                            content = content
                            size = size

                            [archive_state]
                            is_virtual = is_virtual
                            is_valid = is_valid
                            age = age

                            [opts]
                            is_archivable = is_archivable
                            is_idempotent = is_idempotent
                            is_indexable = is_indexable
                            is_object = is_object
                            is_manifest = is_manifest
                        });
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

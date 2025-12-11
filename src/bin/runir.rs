mod cmd;
use cmd::CreateRecord;
use cmd::DeleteRecord;
use cmd::Format;
use cmd::LookupRecord;
use cmd::ObjectFormat;
use runir::container;
use runir::content;
use runir::search::iter::Search;
use runir::util::Container;
use runir::util::PeekExtensions;

use clap::{Args, Parser, Subcommand};
use runir::QueryBuilder;
use runir::Record;
use runir::{IRecord, ToNamespace, frontend::Frontend};
use std::ops::Deref;
use std::process::exit;
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
    ///
    /// NOTE: When storing a structured object (via `-o` or `--json`/`--toml`/`--yaml`),
    /// the original input content is **not preserved verbatim**.
    ///
    /// Instead, it is parsed, restructured internally, and re-serialized for storage.
    /// This means the resulting digest or output format on round-trip may differ from the original input.
    ///
    /// Use raw blob mode (no `-o`) if you want exact byte-for-byte content preservation.
    Put(CreateRecord),
    /// Gets a record from the kv store
    ///
    /// NOTE: If the record was stored as a structured object, it will be deserialized and re-encoded in the selected format.
    ///
    /// This round-trip will not preserve the original byte layout of the input — even if the logical content is the same.
    /// For lossless raw content retrieval, avoid `-o` and use blob mode.
    Get(LookupRecord),
    /// Attempts to delete a record from the kv store
    Delete(DeleteRecord),
    /// Shows information for a stored record
    Info(LookupRecord),
}

#[tokio::main]
async fn main() -> runir::Result<()> {
    let program = Runir::parse();

    let mut filter = tracing_subscriber::EnvFilter::from_default_env();

    if program.debug {
        filter = filter.add_directive("runir=debug".parse().unwrap());
    } else {
        filter = filter.add_directive("runir=warn".parse().unwrap());
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
                    env!("CARGO_PKG_VERSION").as_bytes(),
                )?;
                kv.save().await?;
                kv
            } else {
                let kv = runir::kv::open().await?;
                kv
            };

            match kv_config.command {
                KvCommands::Put(create_record) => {
                    let record = create_record.execute(ns).await?;
                    let record = record.stage(record.clone().into_parts().1)?;
                    kv.put_raw(record.clone())?;
                    kv.refresh().await?;
                    kv.save().await?;

                    if let Some(inserted) = kv.search(content(record.content()).or(container(record.content()))).next() {
                        println!("{}", hex::encode(inserted.content()));
                    } else {
                        if let Some(_) = kv.staging().search(content(record.content()).or(container(record.content()))).next() {
                            println!("{} @ Staging", hex::encode(record.content()));
                        } else {
                            eprintln!("Cannot stage {}, Staging is already occupied", hex::encode(record.content()));
                            exit(1)
                        }
                    }
                    return Ok(());
                }
                KvCommands::Get(lookup_record) => {
                    let LookupRecord {
                        label,
                        format,
                        peek,
                        digest,
                    } = lookup_record;

                    let kv = kv.ns(ns);

                    let value = if digest {
                        let digest = hex::decode(label.trim_start_matches("sha256"))?;
                        let is_partial = digest.len() < 32;
                        let mut search = kv.search(content(digest));

                        if is_partial {
                            let m = search.next();

                            if let Some(s) = search.next() {
                                eprintln!(
                                    "Partial digest returned multiple matches\n\t{}\n\t{}",
                                    hex::encode(m.unwrap().content()),
                                    hex::encode(s.content())
                                );
                                exit(1)
                            }
                            m
                        } else {
                            search.next()
                        }
                    } else {
                        kv.get_raw(&label)
                    };

                    if let Some(value) = value {
                        if value.opts().is_multi() {
                            kv_get_multi_root(value, format, peek).await?;
                        } else {
                            kv_get(value, format, peek).await?;
                        }
                    } else {
                        eprintln!("Object not found");
                        exit(1)
                    }
                }
                KvCommands::Delete(delete_record) => {
                    let DeleteRecord { label } = delete_record;
                    match kv.ns(ns).delete(&label) {
                        Ok(_) => {
                            kv.refresh().await?;
                            kv.save().await?;
                            eprintln!("Deleted `{label}`");
                        }
                        Err(err) => {
                            eprintln!("{err}");
                            exit(1)
                        }
                    }
                }
                KvCommands::Info(lookup_record) => {
                    let LookupRecord { label, .. } = lookup_record;

                    if let Some(value) = kv.get_raw(&label) {
                        if value.opts().is_multi() {
                            let container = Container::read(value)?;
                            print_info(container);
                        } else {
                            print_info(value);
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

/// Executes `kv get` command
async fn kv_get(value: &impl IRecord, format: Format, peek: Option<String>) -> runir::Result<()> {
    if value.opts().is_object() {
        if let Some(peek) = peek {
            if peek == "." {
                if let Some(map) = value.peek().as_iter_kv() {
                    for kvp in map {
                        eprintln!(
                            "{}: {:?} = {}",
                            kvp.0,
                            kvp.1.flexbuffer_type(),
                            kvp.1.deref()
                        );
                    }
                } else if let Some(vec) = value.peek().as_iter() {
                    for (i, v) in vec.enumerate() {
                        eprintln!("[{i}]: {:?} = {}", v.flexbuffer_type(), v.deref());
                    }
                } else {
                    eprintln!(
                        "Record is neither a map or vec at the root, this record has been mis-configured"
                    );
                    exit(1);
                }
            } else {
                if let Some(s) = value.field(&peek).val() {
                    println!("{}", s);
                }
            }

            Ok(())
        } else {
            debug!("Resolving original object format");
            match format.resolve() {
                Some(format) => match format {
                    ObjectFormat::Yaml => {
                        let yaml = value.peek().to_obj::<serde_yaml::Value>().unwrap();
                        println!("{}", serde_yaml::to_string(&yaml).unwrap());
                    }
                    ObjectFormat::Json => match value.peek().to_obj::<serde_json::Value>() {
                        Some(json) => {
                            println!("{json}");
                        }
                        None => {
                            eprintln!(
                                "Failed to load JSON representation from the object. The stored data is likely not an actual valid object. (Tip: try --peek . for introspection)"
                            );
                            exit(1);
                        }
                    },
                    ObjectFormat::Toml => {
                        if let Some(toml) = value.peek().to_obj::<toml::Value>() {
                            match toml::to_string_pretty(&toml) {
                                Ok(formatted) => println!("{formatted}"),
                                Err(err) => {
                                    eprintln!("Failed to serialize TOML: {err}");
                                    exit(1);
                                }
                            }
                        } else {
                            eprintln!(
                                "Failed to load TOML representation from object. (Tip: null values are not valid in TOML, try --json)"
                            );
                            exit(1);
                        }
                    }
                },
                None => {
                    debug!("No formatting option provided, using default format");
                    if let Some(peek) = value.peek().val() {
                        println!("{peek}");
                    } else {
                        eprintln!("Record corruption detected");
                        exit(1)
                    }
                }
            }
            Ok(())
        }
    } else {
        tokio::io::stdout().write_all(value.bytes()).await?;
        Ok(())
    }
}

/// Executes `kv get` command w/ multi-root record
///
/// First proccesses record w/ [`Container::read`], returns an Error if unsuccessful
///
/// If the root of the container is not an object, searches for an objection projection layer;
/// Otherwise, executes [`kv_get`] w/ Container as [`IRecord`]
async fn kv_get_multi_root(
    value: &Record,
    format: Format,
    peek: Option<String>,
) -> runir::Result<()> {
    debug!("Record is multi-root, reading as Container");
    let container = Container::read(value)?;

    if !container.opts().is_object() {
        debug!("Container root is not an object, searching for object projection layer");
        let content_digest = hex::encode(container.content());

        let matches = container.find_all_layer_by_labels(|p| {
            matches!(
                p.at_dot("projection.format").str(),
                Some("json") | Some("toml") | Some("yaml")
            ) && p.at_dot("projection.content").str() == Some(content_digest.as_str())
        })?;

        use runir::util::ILayerDescriptor;
        if let Some((.., layer)) = matches
            .iter()
            .filter_map(|m| {
                container
                    .layer_desc(*m)
                    .filter(|d| d.is_object())
                    .map(|d| (d, *m))
            })
            .next()
        {
            debug!("Found projection at layer: {layer}");
            if let Some(ref peek) = peek {
                if peek == "." {
                    if let Some(map) = container.object(layer).as_iter_kv() {
                        for kvp in map {
                            eprintln!(
                                "{}: {:?} = {}",
                                kvp.0,
                                kvp.1.flexbuffer_type(),
                                kvp.1.deref()
                            );
                        }
                    } else if let Some(vec) = container.object(layer).as_iter() {
                        for (i, v) in vec.enumerate() {
                            eprintln!("[{i}]: {:?} = {}", v.flexbuffer_type(), v.deref());
                        }
                    } else {
                        eprintln!(
                            "Record is neither a map or vec at the root, this record has been mis-configured"
                        );
                        exit(1);
                    }
                } else {
                    if let Some(s) = container.object(layer).at_dot(&peek).val() {
                        println!("{}", s);
                    }
                }

                return Ok(());
            }
        } else {
            debug!("Could not find projection layer, falling back to default kv_get procedure");
        }
    }

    return kv_get(&container, format, peek).await;
}

fn print_info(value: impl IRecord) {
    let content = format!("sha256:{}", hex::encode(value.content()));
    let uuid = value.uuid().as_simple().to_string();
    let opts = value.opts();
    let is_archivable = opts.is_archivable();
    let is_idempotent = opts.is_idempotent();
    let is_indexable = opts.is_indexable();
    let is_object = opts.is_object();
    let is_manifest = opts.is_manifest();
    let is_multi = opts.is_multi();
    let size = value.bytes().len();

    println!(
        "{}",
        toml::toml! {
            uuid = uuid

            [data]
            content = content
            size = size

            [opts]
            is_archivable = is_archivable
            is_idempotent = is_idempotent
            is_indexable = is_indexable
            is_object = is_object
            is_manifest = is_manifest
            is_multi = is_multi
        }
    );
}

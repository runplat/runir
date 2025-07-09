use clap::{ArgGroup, Args, ValueEnum};
use std::process::exit;

/// Provides object format settings
#[derive(Args, Debug, Clone)]
pub struct Format {
    /// Formats the output to a specific object format
    ///
    /// If the stored record is not an object, this argument will be ignored
    #[clap(short)]
    pub object_format: Option<ObjectFormat>,
    #[clap(flatten)]
    object_format_args: Option<ObjectFormatArgs>,
}

impl Format {
    #[inline]
    pub fn resolve(&self) -> Option<ObjectFormat> {
        let Self {
            object_format,
            object_format_args,
        } = self;
        object_format
            .clone()
            .or(object_format_args.clone().map(|o| o.resolve()).clone())
    }
}

/// Specifies the format of an object
#[derive(Args, Clone, Debug)]
#[clap(group(
    ArgGroup::new("format")
        .args(&["json", "yaml", "toml"])
        .multiple(false)
        .conflicts_with("object_format")
))]
struct ObjectFormatArgs {
    /// Use yaml as the object format
    ///
    /// When stored as a record, input will be deserialized as yaml first, and then
    /// re-serialized into the record struct
    ///
    /// If set when looking up a record, the data will be deserialized into yaml
    ///
    /// [same as: -o yaml]
    #[clap(long)]
    pub yaml: bool,
    /// Use json as the object format
    ///
    /// When stored as a record, input will be deserialized as json first, and then
    /// re-serialized into the record struct
    ///
    /// If set when looking up a record, the data will be deserialized into json
    ///
    /// [same as: -o json]
    #[clap(long)]
    pub json: bool,
    /// Use toml as the object format
    ///
    /// When stored as a record, input will be deserialized as toml first, and then
    /// re-serialized into the record struct
    ///
    /// If set when looking up a record, the data will be deserialized into toml
    ///
    /// [same as: -o toml]
    #[clap(long)]
    pub toml: bool,
}

/// Enumeration of supported object formats
#[derive(ValueEnum, Clone, Debug)]
pub enum ObjectFormat {
    Yaml,
    Json,
    Toml,
}

impl ObjectFormatArgs {
    pub fn resolve(&self) -> ObjectFormat {
        match (self.yaml, self.json, self.toml) {
            (true, false, false) => ObjectFormat::Yaml,
            (false, true, false) => ObjectFormat::Json,
            (false, false, true) => ObjectFormat::Toml,
            _ => {
                eprintln!("Only one of --json, --yaml, or --toml may be set.");
                exit(1)
            }
        }
    }
}

use clap::{ArgGroup, Args, ValueEnum};
use std::process::exit;

/// Provides object format settings
#[derive(Args, Debug, Clone)]
pub struct Format {
    /// Formats data as a specific serialization format
    ///
    /// If the stored record is not an object, this argument will be ignored
    #[clap(short)]
    pub object_format: Option<ObjectFormat>,
    /// Projects the input into a structured layer while preserving the original data
    ///
    /// The input is stored as the root of a Container. It is then deserialized using
    /// the specified format and re-serialized into an object projection layer.
    ///
    /// This enables field-based querying and structured introspection later.
    ///
    /// [example: --project json]
    #[clap(long = "project")]
    pub project_format: Option<ObjectFormat>,
    #[clap(flatten)]
    object_format_args: Option<ObjectFormatArgs>,
}

impl Format {
    /// Resolves the object format
    #[inline]
    pub fn resolve(&self) -> Option<ObjectFormat> {
        let Self {
            object_format,
            object_format_args,
            ..
        } = self;
        object_format
            .clone()
            .or(object_format_args.clone().map(|o| o.resolve()).clone())
    }

    /// Resolves the object format
    #[inline]
    pub fn resolve_projection(&self) -> Option<ObjectFormat> {
        let Self {
            project_format,
            object_format_args,
            ..
        } = self;
        project_format
            .clone()
            .or(object_format_args.clone().and_then(|o| o.resolve_projection()).clone())
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
    /// Projects the input JSON into an object layer when storing the record
    ///
    /// The original input will be stored as the root of a Container. It will then be
    /// deserialized as JSON, converted into a structured object form, and added as a layer.
    ///
    /// This enables field-based queries or projections later via `kv get`.
    ///
    /// Note: Has no effect when retrieving existing records
    ///
    /// [same as: -p json or --project-json]
    #[clap(long)]
    pub project_json: bool,
    /// Projects the input YAML into an object layer when storing the record
    ///
    /// The original input will be stored as the root of a Container. It will then be
    /// deserialized as JSON, converted into a structured object form, and added as a layer.
    ///
    /// This enables field-based queries or projections later via `kv get`.
    ///
    /// Note: Has no effect when retrieving existing records
    ///
    /// [same as: -p yaml or --project-yaml]
    #[clap(long)]
    pub project_yaml: bool,
    /// Projects the input TOML into an object layer when storing the record
    ///
    /// The original input will be stored as the root of a Container. It will then be
    /// deserialized as JSON, converted into a structured object form, and added as a layer.
    ///
    /// This enables field-based queries or projections later via `kv get`.
    ///
    /// Note: Has no effect when retrieving existing records
    ///
    /// [same as: -p toml or --project-toml]
    #[clap(long)]
    pub project_toml: bool
}

/// Enumeration of supported object formats
#[derive(ValueEnum, Clone, Debug)]
pub enum ObjectFormat {
    Yaml,
    Json,
    Toml,
}

impl ObjectFormatArgs {
    /// Returns the projection format
    #[inline]
    pub fn resolve_projection(&self) -> Option<ObjectFormat> {
        match (self.project_yaml, self.project_json, self.project_toml) {
            (true, false, false) => Some(ObjectFormat::Yaml),
            (false, true, false) => Some(ObjectFormat::Json),
            (false, false, true) => Some(ObjectFormat::Toml),
            _ => {
                None
            }
        }
    }

    /// Returns the object format
    #[inline]
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

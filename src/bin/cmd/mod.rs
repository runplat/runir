mod lookup_record;
mod create_record;

pub use lookup_record::LookupRecord;
pub use create_record::CreateRecord;

use clap::ValueEnum;

#[derive(ValueEnum, Clone, Debug)]
pub enum ObjectType {
    Yaml,
    Json,
    Toml,
}

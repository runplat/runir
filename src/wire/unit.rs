//! # `unit` module
//!
//! Unit module enables fixed-sized records that can be used to represent operations over a content
//! addressed record
//!
//!

use std::{collections::BTreeMap, sync::OnceLock};

use crate::{IRecord, Namespace, Record, util::PeekExtensions, wire::ContentAddress};
use anyhow::anyhow;
use flexbuffers::{BitWidth, Blob};
use tracing::debug;
use uuid::Uuid;

/// Default namespace for UNIT_V1 store
const RUNIR_UNIT_V1_NAMESPACE_NAME: &str = "__runir_unit_v1";

const RUNIR_UNIT_NAMESPACE: OnceLock<Namespace> = OnceLock::new();

/// Defines a wire unit field
#[derive(Debug)]
pub struct Field<'view> {
    /// Name of the field
    name: &'view str,
    /// Field type
    ty: Type,
}

/// Returns the wire unit namespace
///
/// This namespace will store settings related to wire units
#[inline]
pub fn unit_namespace() -> Namespace {
    RUNIR_UNIT_NAMESPACE
        .get_or_init(|| Namespace::new(RUNIR_UNIT_V1_NAMESPACE_NAME))
        .clone()
}

#[derive(Debug)]
pub enum Type {
    /// Field is an unsigned 8-bit integer
    U8,
    /// Field is an unsigned 64-bit integer
    U64,
    /// Field is a UUID struct
    UUID,
}

/// Defines and extrapolates settings of a wire "unit"
pub struct Unit<R> {
    definition: R,
}

impl<R> Unit<R>
where
    R: IRecord,
{
    /// Returns the name of the unit definition
    #[inline]
    pub fn name(&self) -> &str {
        self.definition.field("name").str().unwrap_or_default()
    }

    /// Returns an iterator over all fields defined in the unit
    #[inline]
    pub fn fields(&self) -> Option<impl Iterator<Item = Field<'_>>> {
        self.definition.field("fields").iter().map(|f| {
            f.filter_map(|fd| {
                let name = fd.clone().at("name").str();
                let ty = fd.clone().at("type").str();

                name.zip(ty).and_then(|(n, t)| {
                    Some(Field {
                        name: n,
                        ty: match t {
                            "u8" => Type::U8,
                            "u64" => Type::U64,
                            "uuid" => Type::UUID,
                            _ => {
                                debug!(field = n, "Field `type` uses an unknown type `{t}`");
                                return None;
                            }
                        },
                    })
                })
            })
            .into_iter()
        })
    }

    /// Validates that a record is a valid unit record
    #[inline]
    pub fn validate(&self, rec: impl IRecord) -> crate::Result<()> {
        if !rec.opts().is_wire_unit() {
            return Err(anyhow!("Record is not a wire unit type").into());
        }

        /*
            A V1 Unit record must contain a flexbuffer object in the following format:
            "unit": <Name of unit>,
            "content": <Bytes for SHA256 Digest>,
            "object": { <Unit Fields> }
        */

        let fields = rec.fields(&["unit", "len", "content", "object"]);

        if let [unit, len, content, object, ..] = fields.as_slice() {
            let unit_name = self.name();
            if unit.str() != Some(unit_name) {
                return Err(
                    anyhow!("Record is not a valid Unit Record of unit `{unit_name}`").into(),
                );
            }

            if len.u64().is_none() {
                return Err(anyhow!(
                    "Record is not a valid V1 Unit Record, `len` field must be a u64"
                )
                .into());
            }

            if content.blob().filter(|b| b.len() == 32).is_none() {
                return Err(anyhow!(
                    "Record is not a valid V1 Unit Record, `content` field must be a 32-byte digest"
                )
                .into());
            }
            if let Some(fields) = self.fields() {
                for Field { name, ty } in fields {
                    if let Some(field) = object.at(name) {
                        match ty {
                            Type::U8 => {
                                let is_u8 = field.flexbuffer_type().is_uint()
                                    && field.bitwidth() == BitWidth::W8;
                                if !is_u8 {
                                    return Err(anyhow!("Record is not a valid Unit Record of unit `{unit_name}`, expected field `{name}` to be a u8").into());
                                }
                            }
                            Type::U64 => {
                                let is_u64 = field.get_u64().is_ok();
                                if !is_u64 {
                                    return Err(anyhow!("Record is not a valid Unit Record of unit `{unit_name}`, expected field `{name}` to be a u64").into());
                                }
                            }
                            Type::UUID => {
                                let is_uuid = field.as_blob().0.len() == 16;
                                if !is_uuid {
                                    return Err(anyhow!("Record is not a valid Unit Record of unit `{unit_name}`, expected field `{name}` to be a uuid").into());
                                }
                            }
                        }
                    } else {
                        return Err(anyhow!("Record is not a valid Unit Record of unit `{unit_name}`, missing field `{name}`").into());
                    }
                }
            }
            Ok(())
        } else {
            Err(anyhow!("Record is not a valid V1 Unit Record, must contain fields `unit`, `content`, and `object`").into())
        }
    }
}

#[derive(Default)]
pub struct WireUnitBuilder<'b> {
    fields: BTreeMap<&'b str, Value>,
}

impl<'b> WireUnitBuilder<'b> {
    /// Adds a uuid-type field and returns the builder
    #[inline]
    pub fn uuid(self, name: &'b str, value: Uuid) -> Self {
        self.field(name, value)
    }

    /// Adds a u8-type field and returns the builder
    #[inline]
    pub fn u8(self, name: &'b str, value: u8) -> Self {
        self.field(name, value)
    }

    /// Adds a u64-type field and returns the builder
    #[inline]
    pub fn u64(self, name: &'b str, value: u64) -> Self {
        self.field(name, value)
    }

    #[inline]
    fn field(mut self, name: &'b str, value: impl Into<Value>) -> Self {
        self.fields.insert(name, value.into());
        self
    }
}

enum Value {
    U8(u8),
    U64(u64),
    UUID(Uuid),
}

impl From<u8> for Value {
    fn from(value: u8) -> Self {
        Self::U8(value)
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

impl From<Uuid> for Value {
    fn from(value: Uuid) -> Self {
        Self::UUID(value)
    }
}

impl Namespace {
    /// Returns a new wire unit record
    #[inline]
    pub fn unit(
        &self,
        unit: &str,
        base: impl IRecord,
        build: impl FnOnce(WireUnitBuilder<'_>) -> WireUnitBuilder<'_>,
    ) -> Record {
        let address = ContentAddress {
            len: base.bytes().len() as u64,
            digest: &base.content(),
        };

        let mut fb = flexbuffers::Builder::default();
        let mut map = fb.start_map();
        map.push("unit", unit);
        map.push("len", address.len);
        map.push("content", Blob(address.digest.as_slice()));

        let mut object = map.start_map("object");
        let unit_builder = build(WireUnitBuilder::default());
        for (field, value) in unit_builder.fields {
            match value {
                Value::U8(v) => {
                    object.push(field, v);
                }
                Value::U64(v) => {
                    object.push(field, v);
                }
                Value::UUID(v) => {
                    object.push(field, Blob(v.as_bytes().as_slice()));
                }
            }
        }
        object.end_map();
        map.end_map();

        let mut rec = self.record(address.digest.as_slice());
        rec.opts.set_object_storage(true).enable_wire_unit();
        rec.commit(fb.view())
    }
}

impl<R: IRecord> From<R> for Unit<R> {
    fn from(value: R) -> Self {
        Unit { definition: value }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        IRecord, Namespace,
        wire::{Unit, unit_namespace},
    };
    use toml::toml;
    use uuid::Uuid;

    #[test]
    fn test_unit_def_from_toml() {
        let def = toml! {
            name = "test"
            fields = [
                { name = "control", type = "u8"},
                { name = "ts", type = "u64" },
                { name = "registry", type = "uuid" },
            ]
        };

        let unit = unit_namespace().store("test", &def);

        let unit = Unit { definition: unit };
        assert_eq!(unit.name(), "test");

        let fields = unit.fields().unwrap().collect::<Vec<_>>();
        assert_eq!(fields.len(), 3);

        let original = Namespace::new("test").content(b"hello world");
        let test = Namespace::new("test").unit("test", original, |b| {
            b.u8("control", 123)
                .u64("ts", 0)
                .uuid("registry", Uuid::nil())
        });
        unit.validate(test).expect("should be a valid unit record")
    }

    #[test]
    fn test_unit_validate() {
        let unit = unit_namespace().author("test", |mut fb| {
            let mut map = fb.start_map();
            map.push("name", "test");
            let mut fields = map.start_vector("fields");

            let mut control = fields.start_map();
            control.push("name", "control");
            control.push("type", "u8");
            control.end_map();

            let mut registry = fields.start_map();
            registry.push("name", "registry");
            registry.push("type", "uuid");
            registry.end_map();

            let mut ts = fields.start_map();
            ts.push("name", "ts");
            ts.push("type", "u64");
            ts.end_map();

            fields.end_vector();
            map.end_map();
            fb
        });

        let unit = Unit { definition: unit };
        assert_eq!(unit.name(), "test");

        let fields = unit.fields().unwrap().collect::<Vec<_>>();
        assert_eq!(fields.len(), 3);

        let original = Namespace::new("test").content(b"hello world");
        let test = Namespace::new("test").unit("test", original, |b| {
            b.u8("control", 123)
                .u64("ts", 0)
                .uuid("registry", Uuid::nil())
        });

        eprintln!("size: {}", test.bytes().len());
        unit.validate(test).expect("should be a valid unit record")
    }
}

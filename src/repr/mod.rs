pub(crate) mod dependency;
pub(crate) mod field;
pub(crate) mod host;
pub(crate) mod node;
pub(crate) mod recv;
pub(crate) mod resource;

pub mod prelude {
    pub use super::Repr;

    pub use super::resource::ResourceLevel;
    pub use super::resource::ResourceRepr;
    pub use super::resource::FFI;

    pub use super::field::Field;
    pub use super::field::FieldLevel;
    pub use super::field::FieldRepr;

    pub use super::recv::Recv;
    pub use super::recv::RecvLevel;
    pub use super::recv::RecvRepr;

    pub use super::node::NodeLevel;
    pub use super::node::NodeRepr;

    pub use super::dependency::DependencyLevel;
    pub use super::dependency::DependencyRepr;

    pub use super::host::HostLevel;
    pub use super::host::HostRepr;

    pub type FieldName = &'static str;
    pub type FieldHelp = String;
    pub type FFIType = &'static str;
}

use crate::define_intern_table;
use crate::prelude::*;
use anyhow::anyhow;
use serde::Deserialize;
use serde::Serialize;
use std::fmt::Display;
use std::sync::Arc;

use self::host::HostRepr;

// Intern table for intern handles
define_intern_table!(HANDLES: InternHandle);

// /// TODO (Phase1 - Bootstrap): This should end up replacing both block_info and node_info,
// ///
// /// Parsing is converting SourceLevel -> ResourceLevel?
// ///
// pub struct SourceLevel {
// }

/// Struct containing the tail reference of the representation,
///
/// A repr is a linked list of intern handle nodes that can unravel back into
/// a repr factory. This allows the repr to store and pass around a single u64 value
/// which can be used to query interned tags from each level.
///
#[derive(
    Hash, Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct Repr {
    /// Tail end of the linked list,
    ///
    pub(crate) tail: InternHandle,
}

impl From<u64> for Repr {
    fn from(value: u64) -> Self {
        Repr {
            tail: InternHandle::from(value),
        }
    }
}

impl Repr {
    /// Returns as a u64 value,
    ///
    #[inline]
    pub fn as_u64(&self) -> u64 {
        self.tail.as_u64()
    }

    /// Returns the entity id value,
    ///
    #[inline]
    pub fn as_entity(&self) -> Option<u64> {
        self.tail.entity()
    }

    /// Returns repr as a uuid,
    ///
    #[inline]
    pub fn as_uuid(&self) -> uuid::Uuid {
        uuid::Uuid::from_u64_pair(self.tail.as_u64(), 0)
    }

    /// Upgrades a representation in place w/ a new level,
    ///
    pub fn upgrade(
        &mut self,
        mut interner: impl InternerFactory,
        level: impl Level,
    ) -> anyhow::Result<()> {
        // Configure a new handle
        let handle = level.configure(&mut interner)?;

        // TODO -- error handling
        // 1) Need verify the interner factory is the same as what was previously used
        // 2) Need to verify the next level is indeed the next level

        let to = Tag::new(&HANDLES, Arc::new(handle));

        let mut from = self.tail;
        from.link = 0;

        let linked = Tag::new(&HANDLES, Arc::new(from)).link(&to)?;
        self.tail = linked;
        Ok(())
    }

    /// Downgrade the Repr by count,
    ///
    /// **Error** Returns an error if count exceeds current repr level
    ///
    pub fn downgrade(&self, count: usize) -> anyhow::Result<Repr> {
        let levels = self.get_levels();

        if let Some(end) = levels.len().checked_sub(count) {
            let mut levels = levels[..end].to_vec();

            match (levels.pop(), levels.pop()) {
                (Some(mut tail), Some(next)) => {
                    let link = tail.register() ^ next.register();

                    tail.link = link;
                    return Ok(Repr { tail });
                }
                (Some(tail), None) => return Ok(Repr { tail }),
                _ => {}
            }
        }

        Err(anyhow!(
            "Could not downgrade level {} by {count}",
            levels.len()
        ))
    }

    /// Return a vector containing an intern handle pointing to each level of this representation,
    ///
    /// The vector is ordered w/ the first element as the root and the last as the tail.
    ///
    pub fn get_levels(&self) -> Vec<InternHandle> {
        let mut levels = vec![];
        let mut cursor = self.tail.node();
        loop {
            match cursor {
                (Some(prev), current) => {
                    if let Some(prev) = HANDLES.copy(&prev) {
                        levels.push(current);
                        cursor = prev.node();
                    }
                }
                (None, current) => {
                    levels.push(current);
                    levels.reverse();
                    return levels;
                }
            }
        }
    }

    /// Returns the repr as a resource repr,
    ///
    #[inline]
    pub fn as_resource(&self) -> Option<ResourceRepr> {
        self.get_levels().first().copied().map(ResourceRepr)
    }

    /// Returns the ffi_type name of the resource repr,
    ///
    #[inline]
    pub fn ffi_type(&self) -> Option<FFIType> {
        self.as_resource().and_then(|r| r.ffi_type_name())
    }

    /// Returns the field help value derived from the node repr,
    ///
    #[inline]
    pub fn field_help(&self) -> Option<FieldHelp> {
        self.as_node()
            .and_then(|r| r.doc_headers())
            .and_then(|d| d.first().cloned())
    }

    /// Returns the field name value derived from the field repr,
    ///
    #[inline]
    pub fn field_name(&self) -> Option<FieldName> {
        self.as_field().and_then(|r| r.name())
    }

    /// Returns the value parser for this field,
    ///
    #[inline]
    #[cfg(feature = "util-clap")]
    pub fn field_value_parser(
        &self,
    ) -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        self.as_resource().and_then(|r| r.ffi_value_parser())
    }

    /// Returns values for expressing this field as a cli argument,
    ///
    #[inline]
    #[cfg(feature = "util-clap")]
    pub fn split_for_arg(
        &self,
    ) -> Option<(
        FieldName,
        Option<FieldHelp>,
        FFIType,
        clap::builder::Resettable<clap::builder::ValueParser>,
    )> {
        match (
            self.field_name(),
            self.field_help(),
            self.ffi_type(),
            self.field_value_parser(),
        ) {
            (Some(a), b, Some(c), Some(d)) => Some((a, b, c, d)),
            _ => None,
        }
    }

    /// Returns the repr as a dependency repr,
    ///
    #[inline]
    pub fn as_dependency(&self) -> Option<DependencyRepr> {
        // TODO: Check if this is actually DependencyLevel?
        self.get_levels().get(1).copied().map(DependencyRepr)
    }

    /// Returns the repr as a receiver repr,
    ///
    #[inline]
    pub fn as_recv(&self) -> Option<RecvRepr> {
        self.get_levels().get(1).copied().map(RecvRepr)
    }

    /// Returns the repr as a field repr,
    ///
    #[inline]
    pub fn as_field(&self) -> Option<FieldRepr> {
        self.get_levels().get(1).copied().map(FieldRepr)
    }

    /// Returns the repr as a node repr,
    ///
    #[inline]
    pub fn as_node(&self) -> Option<NodeRepr> {
        self.get_levels().get(2).copied().map(NodeRepr)
    }

    /// Returns the repr as a host repr,
    ///
    #[inline]
    pub fn as_host(&self) -> Option<HostRepr> {
        self.get_levels().get(3).copied().map(HostRepr)
    }
}

impl Display for Repr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            display_runmd(self, f)?;
        } else if let Some(r) = self.as_resource() {
            if let Some(n) = r.type_name() {
                write!(f, "{n}")?;
            }
        }
        Ok(())
    }
}

fn display_runmd(repr: &Repr, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    if let Some(node) = repr.as_node() {
        writeln!(f, "{node}")?;
    }

    if let Some(resource) = repr.as_resource() {
        writeln!(f, "| **Resource Tags** | |")?;
        writeln!(f, "| --- |  ---  |")?;
        if let Some(name) = resource.type_name() {
            writeln!(f, "| type | `{name}` |")?;
        }

        if let Some(size) = resource.type_size() {
            writeln!(f, "| size | {size} bytes |")?;
        }

        if let Some(id) = resource.type_id() {
            writeln!(f, "| type-id | {:x?} |", id)?;
        }

        if let Some(parse_type) = resource.parse_type_name() {
            writeln!(f, "| parse-type | `{parse_type}` |")?;
        }

        if let Some(ffi_type) = resource.ffi_type_name() {
            writeln!(f, "| ffi-type | `{ffi_type}` |")?;
        }

        writeln!(f, "| uuid | {:?} |", resource.0.as_uuid())?;
    }

    if let Some(field) = repr.as_field() {
        if field.name().is_some() {
            writeln!(f, "| **Field Tags** | |")?;
            if let Some(name) = field.name() {
                writeln!(f, "| field_name | {name} |")?;
            }
            if let Some(offset) = field.offset() {
                writeln!(f, "| field_offset | {offset} |")?;
            }
            if let Some(name) = field.owner_name() {
                writeln!(f, "| owner_name | `{name}` |")?;
            }
            if let Some(size) = field.owner_size() {
                writeln!(f, "| owner_size | {size} bytes |")?;
            }
            if let Some(id) = field.owner_type_id() {
                writeln!(f, "| owner_type_id | {:x?} |", id)?;
            }
            writeln!(f, "| uuid | {:?} |", field.0.as_uuid())?;
        }
    }

    if let Some(node) = repr.as_node() {
        if let Some(path) = node.path() {
            writeln!(f, "| **Node Tags** | |")?;
            writeln!(f, "| path | {path} |")?;
            writeln!(f, "| uuid | {:?} |", node.0.as_uuid())?;
            writeln!(f, "| span | {:?} |", node.span().unwrap_or_default())?;
            writeln!(
                f,
                "| relative | {:?} |",
                node.relative().unwrap_or_default()
            )?;
        }
    }

    if let Some(host) = repr.as_host() {
        if let Some(addr) = host.address() {
            writeln!(f, "| **Host Tags** | |")?;
            writeln!(f, "| addr | {addr} |")?;
            writeln!(f, "| uuid | {:?} |", host.0.as_uuid())?;
        }
    }

    if let Some(recv) = repr.as_recv() {
        writeln!(f)?;
        if let Some(fields) = recv.fields() {
            for _f in fields.iter() {
                writeln!(f, "{:#}", _f)?;
            }
        }
    }

    if let Some(host) = repr.as_host() {
        writeln!(f)?;
        if let Some(ext) = host.extensions() {
            for e in ext.iter() {
                writeln!(f, "{:#}", e)?;
            }
        }
    }

    Ok(())
}

impl Display for NodeRepr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(docs) = self.doc_headers() {
            let mut docs = docs.iter();

            if let Some(header) = docs.next() {
                writeln!(f, "# {}", header.trim_start_matches("# --").trim())?;
            }

            for d in docs {
                writeln!(f, "{}", d.trim_start_matches("# --").trim())?;
            }
        }

        if let Some(source) = self.source() {
            writeln!(f, "```runmd")?;
            for line in source.lines() {
                if !line.starts_with('#') {
                    writeln!(f, "{}", line)?;
                }
            }
            writeln!(f, "```")?;
        }

        Ok(())
    }
}

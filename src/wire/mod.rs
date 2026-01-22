//! # `wire` module
//!
//! This module provides primitives to make runir-related operations "wire" friendly.
//!
//! A wire protocol defines the exact format and rules for how data is encoded, transmitted, and
//! interpreted between systems over a network connection. It ensures that both sender and
//! receiver understand each message identically, regardless of implementation details or programming language.
//!
//! In the context of `runir` this is useful when needing to transmit `runir` over the wire, but even
//! in a broader perspective, having a stable programmatic wire foundation can be beneficial long term.
//!
//! At the moment, there is a loose contract between file-system and memory usage via the virt module,
//! using PathBuf as the universal descriptor for things like store snapshots.
//!
//! The wire module should be a solid foundation below the virt module, which can:
//! - Provide an address-format that can be used externally and internally
//! - Abstract file-system storage
//!     - Keep the low level details in the plumbing
//!     - Provide an API with atomicity
//! - Provide extensibility hooks to enable different data filtering/handling
//! ---
//!
//! ## Concepts
//! - `unit`: This is a representation of a record from the wire perspective
//! - `annotate`: This enables a extensibility layer via meta-programming at the type level
//! - `store`: This is a source/destination which can understand a single address scheme and return volumes
//! - `cas`: Content-Addressable Storage, although records are built from the ground up to be CAS, this
//!
//! ---
//!
//! Each "frontend" implements a set of "actions".
//!
//! An action is either a "backend" or "frontend" action
//!
//! If an action writes to state, it is considered a "backend" action.
//!
//! If an action only reads state, but requires consistentcy, it is considered a "frontend" action.
//!
//! Ex.
//!
//! runir kv put <-- this is a backend action
//! Put would format the pending record and send it to the route
//!
//! runir kv delete <-- this is a backend action
//! Delete would lookup the record, call .delete().transport().archive() -> Route
//!
//! runir kv edit <-- this is a backend action
//! Edit would lookup the record, call .stage().transport().archive() -> Route
//!
//! runir kv fetch <-- this is a frontend action
//! Fetch explicitly reduces the backend state into a view for subsequent get/lookup/search calls
//!
//! This is so that read-only functions don't need to care about consistency.
//! Instead, read-only functions can just assume whatever the current view of state, is a consistent state.
//! ---
//! Ex.
//! runir protect on
//!
//! // --protect flag is to ensure protect is configured
//! runir kv --protect put <..>
//!
//! struct LockedCredential {
//!     secret: Secret<SecretBox<..>>
//! }
//!
//! Use public-key encryption to create "credentials" for the instance
//!
//! If remote access is required, wire terminal verifies if a selected public key is valid
//!
//! If valid, use public-key to generate an ephemeral key that can be used to encrypt a message for this remote
//!
use crate::Data;
use crate::IRecord;
use crate::Namespace;
use crate::Record;
use crate::util::Intern;
use crate::vol::VolumeTarget;
use crate::vol::new_mmap_anon_target;
use crate::vol::pool_bytes_mut;
use crate::wire::boot::Container;
use crate::wire::boot::NS_HEADER_INLINE_BLOCK_SIZE;
use crate::wire::receive::Receive;
use crate::wire::tool::Tool;
use anyhow::anyhow;
use bytes::BufMut;
use bytes::Bytes;
use futures::AsyncRead;
use futures::AsyncReadExt;
use std::collections::BTreeMap;
use tracing::debug;
use tracing::trace;
use tracing::warn;
use zstd::zstd_safe::WriteBuf;

mod boot;
mod cas;
mod ext;
mod proto;
mod receive;
mod root;
mod tool;
mod transport;

pub use boot::Boot;
pub use cas::Describe;
pub use cas::Fetch;
pub use cas::FrameList;
pub use ext::Ext;
pub use root::Root;

use transport::TransportMut;

/// Wire protocol runtime
///
/// `Protocol` encodes a `Boot` and a set of `Frames` into a binary
///
/// `Boot` can decode into a record from binary
#[derive(Debug)]
pub struct Wire {
    proto: proto::Proto,
}

impl Wire {
    /// Creates a new wire protocol for a namespace
    #[inline]
    pub fn new(ns: impl Into<Namespace>) -> Self {
        Self {
            proto: proto::Proto::new(ns.into()),
        }
    }

    /// Returns true if the root is enabled
    #[inline]
    pub fn is_root_enabled(&self) -> bool {
        self.proto.root.is_some()
    }

    /// Sets the root on the underlying protocol
    #[inline]
    pub fn with_root(&mut self, root: Root) {
        self.proto.use_root(root);
    }

    /// Installs all built in tools for the protocol
    ///
    /// - `.from_*`: Deserializes data from a base serialization format and creates a projection as a flexbuffer root
    ///     - Supports `json`, `toml`, and `yaml`
    #[inline]
    pub fn enable_builtin_tools(&mut self) {
        self.install_tool(".from_json", tools::project::json());
        self.install_tool(".from_toml", tools::project::toml());
        self.install_tool(".from_yaml", tools::project::yaml());
    }

    /// Installs a tool for the protocol
    ///
    /// This makes the tool available to use from `Wire::send(..)`
    #[inline]
    pub fn install_tool(&mut self, name: &str, tool: impl Into<Tool>) {
        debug!("Installing tool `{name}`");
        self.proto.tool_index().add(name, tool);
    }

    /// Encodes a record into the wire protocol boot
    ///
    /// Returns the intermediate volume target
    /// --
    /// Returns an error if:
    /// - The record could not be transferred into the protocol's namespace
    /// - An intermediate target could not be allocated for the encoding
    /// - Building the protocol boot object failed
    /// --
    /// If a Root is set, the returned target can call .commit() to persist the data
    /// into .path()
    ///
    /// Otherwise, .commit() will be a no-op
    #[inline]
    pub fn push(
        &self,
        rec: impl IRecord,
        tools: Option<Vec<&'static str>>,
    ) -> crate::Result<impl VolumeTarget + AsyncRead + Send + BufMut> {
        // 1) Transfer into the wire namespce
        let rec: Record = self.proto.ns.transfer(rec)?;
        // 2) Get the intermediate target dest for the record
        let target = self.target(&rec)?;
        // 3) Create builder for record
        let mut proto = self.proto.clone();
        // 4) Set record state
        proto.use_record(rec.clone());

        // Optional) Use tools
        if let Some(tools) = tools {
            for t in tools {
                proto.use_tool(t);
            }
        }

        // 5) Build the wire encoding and output to target
        proto.send(target)
    }

    /// Fetches a record
    ///
    /// If `pull` returns true, than the container will be pulled and this
    /// function will return a `Record`
    ///
    /// Otherwise, this function will return None
    ///
    /// Returns an error if a boot could not be
    #[inline]
    pub async fn fetch(&self, header: Bytes) -> crate::Result<Container> {
        match Boot::decode(header) {
            Ok(boot) => {
                if *boot.ns() != self.proto.ns {
                    return Err(anyhow!("Target `Boot` is from a different namespace").into());
                }
                boot.start()
            }
            Err(err) => match err {
                boot::BootError::Fault(error) => Err(error),
                boot_err => Err(anyhow!("Fetch could not boot: {boot_err:?}").into()),
            },
        }
    }

    /// Pulls wire protcol state from a src
    ///
    /// Returns a newly configured Wire if successful
    #[inline]
    pub async fn pull<'wire>(
        &self,
        mut src: impl AsyncRead + Send + Unpin,
    ) -> crate::Result<Self> {
        let mut header = pool_bytes_mut(1024);
        src.read_exact(&mut header).await?;

        let mut container = self.fetch(header.freeze()).await?;
        trace!(
            frames = container.boot.layout().frames.len(),
            inline = container
                .boot
                .layout()
                .frames
                .iter()
                .filter(|f| f.is_inline())
                .count(),
            "pull"
        );
        container.pull(src).await?;

        let proto = self.proto.receive(&container)?;
        Ok(Wire::from(proto))
    }

    /// Tries to read a record off the wire
    /// 
    /// Returns an error if the protocol has not received enough
    /// frames to read into a Record, or if `Record::is_valid()`
    /// returned `false`
    #[inline]
    pub fn read_record_checked(&self) -> crate::Result<Record> {
        use transport::Transport::*;
        match &self.proto.transport {
            Inline(data) => self.decode_record(&data),
            Frames { data, .. } => self.decode_record(&data),
            Receive(receive) => self.decode_from_receive(&receive),
        }
    }

    /// Returns a Record from the current protocol state
    /// 
    /// Note: Does not check is_valid
    #[inline]
    pub fn read_record_unchecked(&self) -> Record {
        use transport::Transport::*;
        match &self.proto.transport {
            Inline(data) | Frames { data, .. } => {
                Record::from_parts((self.proto.info, data.clone()))
            }
            Receive(receive) => {
                Record::from_parts((self.proto.info, receive.snapshot.clone()))
            },
        }
    }

    /// Returns the record received by the current protocol state
    #[inline]
    pub fn commit(&self) -> crate::Result<()> {
        match &self.proto.root {
            Some(root) => match &self.proto.transport {
                transport::Transport::Receive(receive) => {
                    let mut target =
                        root.write_boot_target(&self.proto.info, Some(receive.list.clone()))?;
                    receive.commit(&mut target)?;
                    target.commit()?;
                    Ok(())
                }
                _ => {
                    let mut target = root.write_boot_target(&self.proto.info, None)?;
                    target.put(self.proto.transport.data().as_slice());
                    target.commit()?;
                    Ok(())
                }
            },
            None => Err(anyhow!("Protocol does not have a root set").into()),
        }
    }

    /// Decode data into a record
    #[inline]
    fn decode_record(&self, data: &Data) -> crate::Result<Record> {
        let rec = Record::from_parts((self.proto.info.clone(), data.clone()));
        if rec.is_valid() {
            /*
                TODO: Can apply tools here
            */
            Ok(rec)
        } else {
            Err(anyhow!("Decoded record is invalid").into())
        }
    }

    /// Decode a `receive` transport into a record
    /// 
    /// Returns an error if the `receive` transport state is incomplete
    #[inline]
    fn decode_from_receive(&self, receive: &Receive) -> crate::Result<Record> {
        /*
            TODO: Check if the receive snapshot contains the .data framelist

            - 
        */
        self.decode_record(&receive.snapshot)
    }

    /// Returns the dest target for encoding records
    ///
    /// If a Root is set, then returned the target can commit to the file system.
    ///
    /// Otherwise, future `.commit()` calls on the target will be a no-op
    #[inline]
    fn target(
        &self,
        record: &Record,
    ) -> crate::Result<impl VolumeTarget + AsyncRead + Send + BufMut + use<'_>> {
        let target = self
            .proto
            .root
            .as_ref()
            .map(|r| {
                r.write_boot_target(
                    &record.info,
                    Some(self.proto.transport.describe(&self.proto.ns)),
                )
            })
            .unwrap_or_else(|| {
                if self.proto.transport.is_inline() {
                    new_mmap_anon_target("", NS_HEADER_INLINE_BLOCK_SIZE)
                } else {
                    new_mmap_anon_target(
                        "",
                        NS_HEADER_INLINE_BLOCK_SIZE
                            + self
                                .proto
                                .transport
                                .describe(&self.proto.ns)
                                .required_capacity() as usize,
                    )
                }
            });
        Ok(target?)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ToolIndex {
    map: BTreeMap<&'static str, Tool>,
}

impl ToolIndex {
    fn add(&mut self, name: &str, tool: impl Into<Tool>) {
        if self.map.contains_key(name) {
            warn!("`{name}` is already registered");
            return;
        }
        self.map.insert(name.intern(), tool.into());
    }
}

impl From<proto::Proto> for Wire {
    fn from(value: proto::Proto) -> Self {
        Self { proto: value }
    }
}

mod tools {
    use super::*;

    pub mod project {
        use super::Tool;

        /// Returns a "projection/from_json" tool
        #[inline]
        pub const fn json() -> Tool {
            Tool::Indexer(super::from_json)
        }

        /// Returns a "projection/from_toml" tool
        #[inline]
        pub const fn toml() -> Tool {
            Tool::Indexer(super::from_toml)
        }

        /// Returns a "projection/from_yaml" tool
        #[inline]
        pub const fn yaml() -> Tool {
            Tool::Indexer(super::from_yaml)
        }
    }

    fn from_json(rec: &Record) -> std::io::Result<Vec<u8>> {
        let value: serde_json::Value = serde_json::from_slice(&rec.data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let index = flexbuffers::to_vec(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        Ok(index)
    }
    fn from_toml(rec: &Record) -> std::io::Result<Vec<u8>> {
        let toml = toml::from_str::<toml::Value>(
            str::from_utf8(&rec.data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        )
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let index = flexbuffers::to_vec(toml)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        Ok(index)
    }
    fn from_yaml(rec: &Record) -> std::io::Result<Vec<u8>> {
        let yaml = serde_yaml::from_slice::<serde_yaml::Value>(&rec.data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let index = flexbuffers::to_vec(yaml)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Namespace,
        test::test_cas_record,
        vol::VolumeTarget,
        wire::{Root, Wire},
    };

    #[test]
    fn test_wire_root() {
        let mut wire = Wire::new(Namespace::new("test"));
        wire.with_root(Root::current_dir().unwrap());

        let _target = wire.push(test_cas_record(), None).unwrap();
        _target.commit().unwrap();

        /*
            1) Test booting from a (VolumeTarget + BufMut)
            2) Test restoring the record from a Boot::decode(..)
        */
    }
}

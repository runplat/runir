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
use crate::vol::VolumeTarget;
use crate::vol::new_mmap_anon_target;
use crate::wire::boot::NS_HEADER_INLINE_BLOCK_SIZE;
use crate::wire::receive::Receive;
use anyhow::anyhow;
use bytes::BufMut;

mod boot;
mod cas;
mod ext;
mod proto;
mod receive;
mod root;
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
pub struct Wire<'wire> {
    proto: proto::Proto<'wire>,
}

impl<'wire> Wire<'wire> {
    /// Creates a new wire protocol for a namespace
    #[inline]
    pub fn new(ns: Namespace) -> Self {
        Self {
            proto: proto::Proto::new(ns),
        }
    }

    /// Sets the root on the underlying protocol
    #[inline]
    pub fn with_root(&mut self, root: Root) {
        self.proto.use_root(root);
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
    pub fn encode(&self, rec: impl IRecord) -> crate::Result<impl VolumeTarget + BufMut> {
        // 1) Transfer into the wire namespce
        let rec: Record = self.proto.ns.transfer(rec)?;
        // 2) Get the intermediate target dest for the record
        let target = self.target(&rec)?;
        // 3) Create builder for record
        let mut proto = self.proto.clone();
        // 4) Set record state
        proto.use_record(rec.clone());

        // TODO: Enable "indexer"
        if let Some(index) = self.indexer(&rec) {
            proto.use_index(index);
        }
        // TODO: Enable "block list"
        if let Some(block_list) = self.block_list(&rec) {
            proto.use_frame_transport(block_list)?;
        }

        // 5) Build the wire encoding and output to target
        proto.build(target)
    }

    /// Decodes the current protocol state into a record
    /// 
    /// Note: Should be used w/ `BootLoader::load()`
    #[inline]
    pub fn decode(&self) -> crate::Result<Record> {
        match &self.proto.transport {
            transport::Transport::Inline(data) => self.decode_record(data),
            transport::Transport::Frame((_, data)) => self.decode_record(data),
            transport::Transport::Receive(receive) => {
                // TODO: Handle non-block list case
                self.decode_block_list(receive)
            },
        }
    }

    #[inline]
    fn decode_record(&self, data: &Data) -> crate::Result<Record> {
        let rec = Record::from_parts((self.proto.info.clone(), data.clone()));
        if rec.is_valid() {
            Ok(rec)
        } else {
            Err(anyhow!("Decoded record is invalid").into())
        }
    }

    #[inline]
    fn decode_block_list(&self, receive: &Receive<'_>) -> crate::Result<Record> {
        // Receive has all the bytes
        let mut iter = receive
            .list
            .frames
            .iter()
            .filter(|f| f.opts.is_canonical_data());

        let first = iter.next();
        let last = iter.last();

        match (first, last) {
            (Some(first), None) => self.decode_record(
                &receive
                    .data
                    .view(first.offset as usize, first.size as usize),
            ),
            (Some(first), Some(last)) => self.decode_record(&receive.data.view(
                first.offset as usize,
                // TODO: Check math
                (last.offset + last.size) as usize,
            )),
            _ => {
                todo!()
            }
        }
    }

    /// Returns the dest target for encoding records
    ///
    /// If a Root is set, then returned the target can commit to the file system.
    /// Otherwise, commit is a no-op
    #[inline]
    fn target(&self, record: &Record) -> crate::Result<impl VolumeTarget + BufMut + use<'_>> {
        let target = self
            .proto
            .root
            .as_ref()
            .map(|r| r.write_boot_target(&record.info, Some(self.proto.transport.describe())))
            .unwrap_or_else(|| {
                if self.proto.transport.is_inline() {
                    new_mmap_anon_target("", NS_HEADER_INLINE_BLOCK_SIZE)
                } else {
                    new_mmap_anon_target(
                        "",
                        NS_HEADER_INLINE_BLOCK_SIZE
                            + self.proto.transport.describe().required_capacity() as usize,
                    )
                }
            });
        Ok(target?)
    }

    /// Returns true if the root is enabled
    #[inline]
    pub fn is_root_enabled(&self) -> bool {
        self.proto.root.is_some()
    }

    /// Runs the protocol
    #[inline]
    fn indexer<'a: 'wire>(&self, _: &Record) -> Option<proto::Index<'a>> {
        None
    }

    #[inline]
    fn block_list(&'wire self, _: &Record) -> Option<FrameList<'wire>> {
        None
    }
}

impl<'wire> From<proto::Proto<'wire>> for Wire<'wire> {
    fn from(value: proto::Proto<'wire>) -> Self {
        Self { proto: value }
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

        let _target = wire.encode(test_cas_record()).unwrap();
        _target.commit().unwrap();
        /*
            1) Test booting from a (VolumeTarget + BufMut)
            2) Test restoring the record from a Boot::decode(..)
        */
    }
}

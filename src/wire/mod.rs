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
use std::sync::Arc;

use sha2::Digest;

use crate::IRecord;
use crate::Opts;
use crate::Queue;
use crate::Record;
use crate::util::Intern;
use crate::util::PeekExtensions;
use crate::wire::plugin::IPlugin;

mod annotate;
pub use annotate::Annotate;

mod cas;
pub use cas::Describe;

mod unit;
pub use unit::ToWireUnit;
pub use unit::Transport;

mod fsm;
mod route;
mod terminal;

pub use terminal::Terminal;
pub use route::Route;

pub mod plugin;

type ObjectStore = std::sync::Arc<object_store::DynObjectStore>;

#[inline]
fn ensure_root_store_path() -> std::io::Result<std::path::PathBuf> {
    let workdir = std::env::current_dir()?.join(".runir");
    std::fs::create_dir_all(&workdir)?;
    Ok(workdir)
}

/// Returns the "root" store, i.e "<Current Directory>/.runir"
#[inline]
pub fn root_store() -> std::io::Result<ObjectStore> {
    let path = ensure_root_store_path()?;
    Ok(std::sync::Arc::new(
        object_store::local::LocalFileSystem::new_with_prefix(path)?,
    ))
}

/// Wire is the entire runtime for a wire unit
///
/// Provides mappings to the inner metadata, while also implementing IRecord
/// so that the inner object can be used
#[derive(Debug)]
pub struct Wire<R> {
    record: R,
    runtime: Arc<Runtime>,
    settings: Settings,
}

#[derive(Debug)]
struct Settings {
    /*
        FSM Ready Sync Settings
    */
    fsm_ready_sync_enable: bool,
    fsm_ready_sync_threshold: usize,
}

impl<R> Wire<R> {
    /// Registers a filter w/ the runtime
    #[inline]
    pub fn filter<P: IPlugin>(&self, filter: Opts) -> &Self {
        self.runtime
            .filters
            .entry(filter)
            .and_modify(|f| {
                f.push(P::record_mut);
            })
            .or_default()
            .push(P::record_mut);
        self
    }

    // /// Push a record on to the wire
    // #[inline]
    // pub fn push(&self, record: Record) -> Option<Record> {
    //     match self.bus_in.pusher() {
    //         Some(pusher) => {
    //             pusher.push(record)
    //         },
    //         None => {
    //             Some(record)
    //         },
    //     }
    // }

    /// Returns a reference to the inner runtime
    #[inline]
    fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }
}

type Filter = fn(Record) -> Record;

#[derive(Default, Debug)]
struct Runtime {
    filters: dashmap::DashMap<Opts, Vec<Filter>>,
}

impl<R: IRecord> Wire<R> {
    /// Returns true if the record transport has completed
    ///
    /// Note: This means that the "Transport" branch flag is no longer enabled
    #[inline]
    pub fn is_transport_complete(&self) -> bool {
        !self.opts().is_transport()
    }

    /// Checks the result of the wire unit transform
    ///
    /// Returns an error if the transform was successful
    #[inline]
    pub fn result(&self) -> std::io::Result<()> {
        let err = self.record.peek().at_path(&[".runir", "err"]).str();
        match err {
            Some(err) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, err)),
            None => Ok(()),
        }
    }

    /// Returns the unix timestamp of the wire unit transform
    #[inline]
    pub fn ts(&self) -> i64 {
        self.record
            .peek()
            .at_path(&[".runir", "ts"])
            .u64()
            .unwrap_or_default() as i64
    }

    /// Returns the type name of the current object
    #[inline]
    pub fn type_name(&self) -> &'static str {
        self.record
            .peek()
            .at_path(&[".runir", "type_name"])
            .str()
            .unwrap_or_default()
            .intern()
    }

    /// Returns the size of the current object
    #[inline]
    pub fn size(&self) -> u64 {
        self.record
            .peek()
            .at_path(&[".runir", "size"])
            .u64()
            .unwrap_or_default()
    }

    /// Returns the sha256 digest of the current object
    #[inline]
    pub fn digest(&self) -> &[u8] {
        match self.record.peek().at_path(&[".runir", "sha256"]).blob() {
            Some(digest) => digest,
            None => &[],
        }
    }
}

impl<R: IRecord> IRecord for Wire<R> {
    fn ns_chk(&self) -> u64 {
        self.record.ns_chk()
    }

    fn uuid(&self) -> uuid::Uuid {
        self.record.uuid()
    }

    fn opts(&self) -> &crate::Opts {
        self.record.opts()
    }

    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        self.record.opts_mut()
    }

    fn bytes(&self) -> &[u8] {
        match self.record.peek().to_wire_object() {
            Some(object) => {
                let bytes = object.clone().blob().unwrap_or_default();
                if self.record.is_wire_unit_mode_transport() {
                    object.at("bytes").blob().unwrap_or(bytes)
                } else {
                    bytes
                }
            }
            None => self.record.bytes(),
        }
    }

    fn to_record(&self) -> crate::Record {
        self.record.to_record()
    }
}

impl<R> From<R> for Wire<R> {
    fn from(value: R) -> Self {
        Self {
            record: value,
            runtime: Arc::new(Runtime::default()),
            settings: Settings {
                fsm_ready_sync_enable: false,
                fsm_ready_sync_threshold: 1,
            },
        }
    }
}

impl<R: IRecord> cas::Describe for Wire<R> {
    #[inline]
    fn describe<'desc>(&'desc self) -> cas::Manifest<'desc> {
        let container = self.record.bytes();
        cas::Manifest {
            container: cas::Descriptor {
                size: container.len() as u64,
                digest: std::borrow::Cow::Owned(sha2::Sha256::digest(container).to_vec()),
            },
            object: cas::Descriptor {
                size: self.size(),
                digest: self.digest().into(),
            },
        }
    }
}

#[cfg(test)]
mod prototype {
    use crate::frontend::state::State;

    #[test]
    fn test_dispatcher_remote() {
        /*
            dispatcher -> remote

            - Action: Dispatcher wants to dispatch records to remote

            Dispatcher takes record and calls .transport()

            Record is in wire_unit mode and then call .archive()

            .archive() creates a tar entry, tar entry is included in a msg.tar

            *msg.tar is transmitted to remote

            Remote takes `msg.tar` and loads it as an archive member.

            --- Critical Section
                If .reduce() is called, any records w/ Branch::Transport will be skipped

                Record must remove Branch::Transport via wire runtime
            ---

            Remote calls .reduce()

            Action is complete

            *Missing: what do we consider a "transmit"?
            - It's probably better to decouple these details from runir
            - So, it's probably better to use object_store as what we consider as "transported"
            - Which would enable using other storage providers as the transport vehicle
            - This is nice because it enables integration as a side-effect
            - Can leave the implementation details outside of runir
            - Can focus on just the ingestion of the msg.tar -
        */
        let dispatcher = State::default();
        /*
            let route = runir::wire::route(object_store);
            let terminal = runir::wire::terminal(dispatcher);
            terminal.send(record!("next-update-1").transport());
            terminal.send(record!("next-update-2").transport());
            terminal.dispatch(route).await?;
        */

        let remote = State::default();
        /*
            let route = runir::wire::route(object_store);
            select! {
                msg_tar = route.recv() => {
                    // route is completely de-coupled from "remote"
                    let terminal = runir::wire::terminal(remote);
                    terminal.receive(msg_tar); // Impl, just writes the archive_member to state
                    terminal.reconcile().await?;  // Run "fsm" to handle all transported records, then call .reduce() and .save()
                    // Can just drop terminal w/o needing to maintain any state
                },
                reason = route.shutdown() => {
                    match reason {
                        Reason::Upgrade => {
                            // In this case, infrastructure wants to upgrade and is signaling to call runir::wire::route(object_store) once more
                            // route = ::wire:route(object_store);
                        }
                        Reason::Host => {
                            //
                        }
                        Reason::Error(e) => {
                            // Encountered some sort of error, so need to log the error and in most cases restart the route
                        }
                    }
                },
                ...
            }

            // CLI-side

            let remote = KV::open_remote(Impl); // Schedule leaves the remote "open" for re-use, otherwise re-uses an open remote
            /*
                Frontend::open_remote,

                Each app directory looks like this,
                    .runir/wire.route.messages
                    .runir/<frontend>/wire.route.messages
                    .runir/<frontend>/store.tar <-- Currently using a single archive

                .runir/wire.route.messages/<EPOCH>.<TS>.msg.tar <-- this is a top level message

                <TAR-ENTRY-HEADER>
                ...
                runir <frontend-name> <- Each entry uses the tar entry header ext to state the frontend the entry is for
                ...

                runir <frontend-name>.<frontend-action> <-- And the backend action responsible for completing the transport

                ...

                Ex:

                runir kv.put

                ## Concurrency strategy

                Checks for open .runir/var/run/route.sock
                If exists, sends command to .sock
                If doesn't exist,
                    Create route.sock
                    Start listening for msg_tar
                    Start recv loop

                <BEGIN  :UUID>
                <MSG    :UUID>
                <MSG    :BYTES>

                let stream = TcpStream::open(".runir/var/run/route.sock");
                stream.write(BEGIN);
                stream.write(MSG_UUID);
                stream.write(MSG_BYTES);
                stream.flush();
            */

            match command {
                Put => {
                    remote.put();
                    remote.await // If "origin" then this will block
                }
                Get => {
                    remote.get();
                    // Don't need to await remote, because receive loop doesn't need to be active
                }
            }
        */
    }
}

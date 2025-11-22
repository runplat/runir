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

mod annotate;
pub use annotate::Annotate;

mod cas;
pub use cas::Describe;

mod unit;
pub use unit::ToWireUnit;
pub use unit::Transport;

mod fsm;
mod store;

/// Wire is the entire runtime for a wire "terminal"
/// 
/// Must contain shared components which can run the wire protocol
#[derive(Debug)]
pub struct Wire {

}

impl Wire {

}
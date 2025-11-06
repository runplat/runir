//! # `wire` module
//! 
//! This module provides primitives to make runir-related operations "wire" friendly.
//! 
//! A wire protocol defines the exact format and rules for how data is encoded, transmitted, and
//! interpreted between systems over a network connection. It ensures that both sender and
//! receiver understand each message identically, regardless of implementation details or programming language.
//! 
//! In the context of `runir` this is useful when integrating runir with control-plane systems
//! that might want to transmit runir between network nodes.
//! 

mod source;
pub use source::Source;
pub use source::ContentAddress;

mod unit;
pub use unit::unit_namespace;
pub use unit::Unit;
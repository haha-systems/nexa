//! Nexa's Slice 0 reference implementation.
//!
//! Source checking, immutable memories, typed blueprint composition, authorized
//! execution, inspection, and evidence share this library and its wire contracts.

pub mod blueprint;
pub mod canonical;
pub mod contracts;
pub mod diagnostic;
pub mod digest;
pub mod evidence;
#[cfg(feature = "http-client")]
pub mod http_client;
pub mod kernel;
pub mod memory;
pub mod runtime;
pub mod schemas;
pub mod source;
pub mod value;

pub const LANGUAGE_VERSION: &str = "0.3";
pub const IMPLEMENTATION_VERSION: &str = env!("CARGO_PKG_VERSION");

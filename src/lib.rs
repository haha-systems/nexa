//! Nexa's reusable semantic foundation.
//!
//! M0 validates wire contracts and identity. It does not parse or execute Nexa
//! source, resolve memories, or establish semantic compatibility of graph ports.

pub mod canonical;
pub mod contracts;
pub mod diagnostic;
pub mod digest;
pub mod kernel;
pub mod schemas;
pub mod value;

pub const LANGUAGE_VERSION: &str = "0.3";
pub const IMPLEMENTATION_VERSION: &str = env!("CARGO_PKG_VERSION");

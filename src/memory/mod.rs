//! Immutable, locally stored Nexa memories and exact resolution locks.

mod lock;
mod seal;
mod store;

pub mod program;

pub use lock::{read_lock, write_lock};
pub use program::seal_program;
pub use seal::seal_source;
pub use store::Store;

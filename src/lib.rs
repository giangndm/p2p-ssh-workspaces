//! Minimal iroh-backed SSH workspace library.
//!
//! The crate intentionally exposes only the three design boundaries required
//! by the application: shared values, listen mode, and connect mode.

pub mod connect;
pub mod listen;
pub mod shared;

//! Minimal iroh-backed SSH workspace library.
//!
//! Each top-level CLI subcommand maps to one module (`listen`, `connect`,
//! `config`, and `trace`), sharing common types and transport primitives from
//! `shared`.

pub mod config;
pub mod connect;
pub mod listen;
pub mod shared;
pub mod trace;

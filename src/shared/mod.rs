//! Shared values, errors, configuration, streams, and iroh adapters.

mod config;
mod errors;
mod identity;
mod stream;
mod transport;
mod types;

pub use config::{TomlConfig, default_config_path, default_identity_path};
pub use errors::{AppError, AppResult, ArgumentError, ConfigError};
pub use transport::IrohEndpoint;
#[cfg(test)]
pub(crate) use transport::{IncomingStream, OutgoingStream};
pub use types::{
    BackendAddress, Config, HostAlias, HostConfig, ListenConfig, NodeId, SshHost, StartupCommand,
};

#[cfg(test)]
pub(crate) use config::resolve_config_dir;
pub(crate) use errors::{iroh_error, timeout_error};
#[cfg(test)]
pub(crate) use identity::TEMP_COUNTER as IDENTITY_TEMP_COUNTER;
pub(crate) use stream::{StreamParts, bridge};
pub(crate) use transport::PeerTransport;

/// Application protocol identifier used by iroh streams.
pub const ALPN: &[u8] = b"sshp/1";
/// Default local SSH backend address.
pub const DEFAULT_BACKEND: &str = "127.0.0.1:22";

#[cfg(test)]
mod tests;

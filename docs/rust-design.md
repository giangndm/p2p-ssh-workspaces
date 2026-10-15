# Detailed Rust design

This document defines the implemented flat module boundaries (`shared`,
`listen`, `connect`, `config`, and `trace`) and the testable core seam. The
project uses `iroh` 1.1 and OpenSSH.

## Module map

```text
crate::shared    NodeId, RelayUrl, errors, default paths, worker identity, iroh adapter,
                 atomic file writes, and bidirectional byte bridge
crate::listen    Listener, BackendAddress, and local TCP backend bridge
crate::connect   connect_stdio, autoconnect, and ProxyCommand stdio bridge
crate::config    HostAlias, SshConfigEntry, upsert_ssh_config, disable_ssh_config,
                 write_ssh_config_entry, disable_ssh_config_entry,
                 install_service, and uninstall_service
crate::trace     TraceReport, trace_node, and format_trace_report
```

## Shared types

```rust
pub const ALPN: &[u8] = b"sshp/1";

pub struct NodeId(iroh::EndpointId); // validated iroh endpoint ID
pub struct RelayUrl(iroh::RelayUrl); // validated http/https iroh relay URL
```

## Traits and rationale

A private transport trait exists only at the iroh network seam so listen and
connect flows can be tested in memory without live P2P discovery:

```rust
pub(crate) trait PeerTransport {
    type Reader: AsyncRead + Unpin + Send + 'static;
    type Writer: AsyncWrite + Unpin + Send + 'static;

    async fn connect(&self, peer: &NodeId)
        -> AppResult<StreamParts<Self::Reader, Self::Writer>>;

    async fn accept(&self) -> AppResult<(NodeId, StreamParts<Self::Reader, Self::Writer>)>;
}
```

The byte pipeline uses standard Tokio traits and returns `BridgeStats` for
optional `--trace` diagnostics:

```rust
pub(crate) struct StreamParts<R, W> {
    pub(crate) reader: R,
    pub(crate) writer: W,
}

pub(crate) struct BridgeStats {
    pub(crate) left_to_right: u64,
    pub(crate) right_to_left: u64,
}

pub(crate) async fn bridge<Lr, Lw, Rr, Rw>(
    left: StreamParts<Lr, Lw>,
    right: StreamParts<Rr, Rw>,
) -> io::Result<BridgeStats>
```

## Shared iroh boundary

```rust
pub struct IrohEndpoint {
    pub(crate) endpoint: iroh::Endpoint,
    pub(crate) relay: Option<RelayUrl>,
}

impl IrohEndpoint {
    pub async fn bind(identity_path: &Path, relay: Option<RelayUrl>) -> AppResult<Self>;
    pub async fn bind_ephemeral(relay: Option<RelayUrl>) -> AppResult<Self>;
    pub fn id(&self) -> NodeId;
    pub(crate) fn target_addr(&self, peer: NodeId) -> iroh::EndpointAddr;
    pub(crate) async fn accept_stream(&self) -> AppResult<(NodeId, IrohStreamParts)>;
    pub(crate) async fn connect_addr(&self, address: impl Into<iroh::EndpointAddr>) -> AppResult<IrohStreamParts>;
}
```

## Feature modules

```rust
// crate::listen
pub const DEFAULT_BACKEND: &str = "127.0.0.1:22";
pub struct BackendAddress(String);
pub struct Listener { ... }
impl Listener {
    pub async fn bind(
        identity_path: &Path,
        backend: BackendAddress,
        relay: Option<RelayUrl>,
        trace: bool,
    ) -> AppResult<Self>;
    pub fn id(&self) -> NodeId;
    pub async fn run(&self) -> AppResult<()>;
}

// crate::connect
pub async fn connect_stdio(
    node_id: NodeId,
    relay: Option<RelayUrl>,
    trace: bool,
) -> AppResult<()>;
pub async fn autoconnect(
    host: HostAlias,
    command: Option<String>,
    trace: bool,
) -> AppResult<i32>;

// crate::config
pub struct HostAlias(String);
pub struct SshConfigEntry {
    pub alias: HostAlias,
    pub node_id: NodeId,
    pub executable: PathBuf,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
    pub relay: Option<RelayUrl>,
    pub trace: bool,
}
pub fn upsert_ssh_config(existing: &str, alias: &HostAlias, block: &str) -> AppResult<String>;
pub fn disable_ssh_config(existing: &str, alias: &HostAlias) -> AppResult<String>;
pub fn write_ssh_config_entry(path: &Path, entry: &SshConfigEntry) -> AppResult<String>;
pub fn disable_ssh_config_entry(path: &Path, alias: &HostAlias) -> AppResult<String>;
pub fn install_service(relay: Option<&RelayUrl>) -> AppResult<String>;
pub fn uninstall_service() -> AppResult<String>;
pub fn format_diff(path: &Path, before: Option<&str>, after: Option<&str>) -> String;

// crate::trace
pub struct TraceReport {
    pub local_id: NodeId,
    pub target_id: NodeId,
    pub handshake_ms: u128,
    pub stream_open_ms: u128,
}
pub async fn trace_node(node_id: NodeId, relay: Option<RelayUrl>) -> AppResult<TraceReport>;
pub fn format_trace_report(report: &TraceReport) -> String;
```

## Error shape

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing configuration value: {0}")]
    Missing(&'static str),
    #[error("invalid configuration value: {0}")]
    Invalid(String),
    #[error("invalid endpoint ID: {0}")]
    InvalidNodeId(#[source] iroh::KeyParsingError),
    #[error("invalid relay URL: {0}")]
    InvalidRelayUrl(#[source] iroh::RelayUrlParseError),
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("iroh error: {0}")]
    Iroh(#[source] BoxError),
    #[error("service error: {0}")]
    Service(#[source] BoxError),
}
```

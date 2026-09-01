# Detailed Rust design

This document defines the implemented shared, listen, connect, and systemd
boundaries, plus the testable core seams. The project uses `iroh` 1.1 and
OpenSSH.

## Module map

```text
crate::shared
  ├── config.rs       TOML types, validation, and paths
  ├── types.rs        validated domain values
  ├── identity.rs     persistent worker key
  ├── stream.rs       byte bridge
  ├── transport.rs    iroh adapter and peer seam
  ├── errors.rs       application errors
  └── AppError/AppResult

crate::listen
  ├── Listener
  ├── local backend bridge
  └── systemd.rs       service installer

crate::connect
  ├── Connector
  ├── SshCommand
  ├── retry.rs        session runner and reconnect policy
  └── stdio proxy path
```

## Shared types

```rust
pub const ALPN: &[u8] = b"sshp/1";

pub struct NodeId(iroh::EndpointId); // validated iroh endpoint ID

pub struct HostAlias(String); // validated non-empty alias

pub struct SshHost(String); // validated non-empty SSH host

pub struct StartupCommand {
    pub program: String,
    pub args: Vec<String>,
}

pub struct HostConfig {
    pub node_id: NodeId,
    pub ssh_host: SshHost,
    pub startup: Option<StartupCommand>,
    pub reconnect: bool,
}

pub struct ListenConfig {
    pub backend: BackendAddress,
    pub identity_path: PathBuf,
}

pub struct Config {
    pub listen: ListenConfig,
    pub hosts: BTreeMap<HostAlias, HostConfig>,
}
```

`startup = []` in TOML maps to `None`; no startup command means the login
shell. A non-empty array is split into `program` and `args`.

## Traits and rationale

The MVP has traits only at side-effect boundaries:

```rust
pub struct TomlConfig;

pub trait ServiceInstaller {
    fn install(&self, request: &ServiceRequest) -> AppResult<()>;
    fn uninstall(&self, unit_path: &Path) -> AppResult<()>;
}
```

The application flow uses small private traits at external side-effect
boundaries:

```rust
pub(crate) trait PeerTransport {
    type Reader: AsyncRead + Unpin + Send + 'static;
    type Writer: AsyncWrite + Unpin + Send + 'static;

    async fn connect(&self, peer: &NodeId)
        -> AppResult<StreamParts<Self::Reader, Self::Writer>>;

    async fn accept(&self) -> AppResult<(NodeId, StreamParts<Self::Reader, Self::Writer>)>;
}

pub(crate) trait SessionRunner {
    async fn run(&self, executable: &OsStr, argv: &[OsString]) -> AppResult<i32>;
}

trait BackendConnector: Send + Sync + 'static {
    type Reader: AsyncRead + Unpin + Send + 'static;
    type Writer: AsyncWrite + Unpin + Send + 'static;

    fn connect<'a>(
        &'a self,
        address: &'a BackendAddress,
    ) -> impl Future<Output = AppResult<StreamParts<Self::Reader, Self::Writer>>> + Send + 'a;
}
```

`PeerTransport` has an iroh adapter and an in-memory test adapter.
`SessionRunner` has an OpenSSH adapter and a scripted test adapter.
`BackendConnector` has TCP and in-memory adapters. These traits stay
crate-private because the application is fixed to iroh, TCP, and SSH.

The byte pipeline uses standard Tokio traits and one small stream-parts type:

```rust
pub(crate) struct StreamParts<R, W> {
    reader: R,
    writer: W,
}

pub(crate) async fn bridge<Lr, Lw, Rr, Rw>(
    left: StreamParts<Lr, Lw>,
    right: StreamParts<Rr, Rw>,
) -> io::Result<()>
where
    Lr: AsyncRead + Unpin + Send,
    Lw: AsyncWrite + Unpin + Send,
    Rr: AsyncRead + Unpin + Send,
    Rw: AsyncWrite + Unpin + Send,
{
    // copy bytes in both directions and propagate half-close
}
```

No SSH or iroh protocol is parsed by the core.

There is no launcher plugin trait. A startup command remains data passed to
OpenSSH. `SessionRunner` only isolates process lifecycle and exit status.

## Shared iroh boundary

```rust
pub struct IrohEndpoint { endpoint: iroh::Endpoint }
pub(crate) struct IncomingConnection { incoming: iroh::endpoint::Incoming }
pub(crate) struct IncomingStream { send: SendStream, recv: RecvStream }
pub(crate) struct OutgoingStream { send: SendStream, recv: RecvStream }

impl IrohEndpoint {
    pub async fn bind(identity_path: &Path) -> AppResult<Self>;
    pub async fn bind_ephemeral() -> AppResult<Self>;
    pub fn id(&self) -> NodeId;
    pub(crate) async fn accept(&self) -> AppResult<IncomingConnection>;
    pub(crate) async fn connect(&self, node_id: &NodeId) -> AppResult<OutgoingStream>;
}

impl IncomingConnection {
    pub(crate) async fn accept_stream(self) -> AppResult<(NodeId, IncomingStream)>;
}
```

The current endpoint boundary:

- bind with the persisted listener key;
- configure only `ALPN`;
- resolve and connect using the worker `NodeId`;
- expose exactly one stream per SSH connection;
- leave byte forwarding to listen/connect orchestration.

`IrohEndpoint::id()` returns the stable public ID. The identity is written to a
temporary file, then published with an atomic hard link so concurrent workers
cannot overwrite each other. The final file stays at mode `0600` on Unix.

## Listen module

```rust
pub struct Listener {
    endpoint: IrohEndpoint,
    backend: BackendAddress,
    slots: Arc<Semaphore>,
}

async fn run_listener<T, B>(
    transport: &T,
    backend: B,
    address: &BackendAddress,
    slots: Arc<Semaphore>,
) -> AppResult<()>
where
    T: PeerTransport,
    B: BackendConnector + Clone,
{
    // accept peers, acquire a slot, connect the backend, and bridge bytes
}

pub struct BackendAddress(String); // validated host:port value

impl Listener {
    pub async fn from_config(config: ListenConfig) -> AppResult<Self>;
    pub fn id(&self) -> NodeId;
    pub async fn run(&self) -> AppResult<()>;
}

pub struct ServiceRequest {
    pub executable: PathBuf,
    pub unit_path: PathBuf,
    pub config_dir: PathBuf,
}

pub struct SystemdInstaller;
impl ServiceInstaller for SystemdInstaller { /* systemd adapter */ }
```

`run_listener` accepts connections, bounds active peers at 64, and tracks peer
tasks with `JoinSet`. `BackendConnector` isolates TCP setup from the worker
flow, so tests can provide an in-memory backend. Handshake, stream, and local
backend connection attempts have timeouts. Config parsing and systemd logic
stay outside the listener loop.

## Connect module

```rust
pub struct Connector {
    endpoint: Option<IrohEndpoint>,
    retry: RetryPolicy,
}

pub struct SshCommand {
    pub host: SshHost,
    pub startup: Option<StartupCommand>,
    pub ssh_args: Vec<OsString>,
    pub proxy: ProxyCommand,
}

pub struct ProxyCommand {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
}

pub struct RetryPolicy {
    pub enabled: bool,
    pub max_delay: Duration,
    pub max_retries: u32,
}

impl Connector {
    pub fn new(retry: RetryPolicy) -> Self;
    pub fn with_endpoint(endpoint: IrohEndpoint, retry: RetryPolicy) -> Self;
    pub async fn run(
        &self,
        alias: HostAlias,
        host: HostConfig,
        ssh_args: Vec<OsString>,
    ) -> AppResult<i32>;
    pub async fn run_stdio_proxy(&self, node_id: NodeId) -> AppResult<()>;
}

pub(crate) async fn run_proxy<T, Input, Output>(
    transport: &T,
    node_id: NodeId,
    input: Input,
    output: Output,
) -> AppResult<()>
where
    T: PeerTransport,
    Input: AsyncRead + Unpin + Send,
    Output: AsyncWrite + Unpin + Send,
{
    // connect the peer and bridge process stdio to its stream
}
```

`run` builds `SshCommand`, runs OpenSSH, treats exit code 255 as the
reconnectable failure convention, and applies the retry policy. OpenSSH does
not expose enough information to distinguish authentication failure from other
255 failures. `run_stdio_proxy` only connects to iroh and copies
stdin/stdout; it does not parse SSH.

Only the hidden `connect --stdio` path creates an ephemeral access endpoint.
The worker endpoint persists its identity; the access endpoint does not need a
stable ID.

The default policy uses a 1 second fixed delay, clamped to 100 ms–60 s, and
allows 16 reconnects, capped at 64. `--no-reconnect` disables the policy.

## Error shape

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing configuration value: {0}")]
    Missing(&'static str),
    #[error("invalid configuration value: {0}")]
    Invalid(String),
    Parse(toml::de::Error),
    InvalidNodeId(iroh::KeyParsingError),
    UnknownHost(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ArgumentError {
    #[error("invalid argument: {0}")]
    Invalid(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(ConfigError),
    #[error("I/O error: {0}")]
    Io(std::io::Error),
    #[error("iroh error: {0}")]
    Iroh(Box<dyn std::error::Error + Send + Sync>),
    #[error("SSH error: {0}")]
    Ssh(Box<dyn std::error::Error + Send + Sync>),
    #[error("service error: {0}")]
    Service(Box<dyn std::error::Error + Send + Sync>),
    #[error("invalid argument: {0}")]
    InvalidArgument(ArgumentError),
}

`NodeId` stores a parsed `iroh::EndpointId`. String-backed values use validated
`TryFrom<String>` constructors; `derive_more` supplies small display and inner
type conversions where they remain safe.

pub type AppResult<T> = Result<T, AppError>;
```

The implementation must preserve upstream error context.

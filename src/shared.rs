//! Shared types, errors, path helpers, worker identity, stream bridging, and iroh transport.

use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, copy};

/// Application protocol identifier used by iroh streams.
pub const ALPN: &[u8] = b"sshp/1";

const IROH_ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const IROH_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Application result with user-facing error variants.
pub type AppResult<T> = Result<T, AppError>;
pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
type IrohStreamParts = StreamParts<iroh::endpoint::RecvStream, iroh::endpoint::SendStream>;

#[derive(Debug, Error)]
/// Configuration or path validation error.
pub enum ConfigError {
    #[error("missing configuration value: {0}")]
    /// A required environment or configuration value is absent.
    Missing(&'static str),
    #[error("invalid configuration value: {0}")]
    /// A value violates its validation rules.
    Invalid(String),
    #[error("invalid endpoint ID: {0}")]
    /// The value is not a valid iroh endpoint ID.
    InvalidNodeId(#[source] iroh::KeyParsingError),
    #[error("invalid relay URL: {0}")]
    /// The value is not a valid iroh relay URL.
    InvalidRelayUrl(#[source] iroh::RelayUrlParseError),
}

#[derive(Debug, Error)]
/// Top-level application error.
pub enum AppError {
    #[error("configuration error: {0}")]
    /// Configuration or path validation failed.
    Config(#[from] ConfigError),
    #[error("I/O error: {0}")]
    /// A filesystem or stream I/O operation failed.
    Io(#[from] std::io::Error),
    #[error("iroh error: {0}")]
    /// An iroh operation failed.
    Iroh(#[source] BoxError),
    #[error("service error: {0}")]
    /// A service operation failed.
    Service(#[source] BoxError),
}

pub(crate) fn iroh_error<E>(error: E) -> AppError
where
    E: std::error::Error + Send + Sync + 'static,
{
    AppError::Iroh(Box::new(error))
}

pub(crate) fn timeout_error(message: &'static str) -> AppError {
    std::io::Error::new(ErrorKind::TimedOut, message).into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Validated iroh endpoint identifier.
pub struct NodeId(iroh::EndpointId);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<iroh::EndpointId> for NodeId {
    fn from(id: iroh::EndpointId) -> Self {
        Self(id)
    }
}

impl From<NodeId> for iroh::EndpointId {
    fn from(id: NodeId) -> Self {
        id.0
    }
}

impl AsRef<iroh::EndpointId> for NodeId {
    fn as_ref(&self) -> &iroh::EndpointId {
        &self.0
    }
}

impl FromStr for NodeId {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = non_empty(value, "node_id")?;
        iroh::EndpointId::from_str(value)
            .map(Self)
            .map_err(ConfigError::InvalidNodeId)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Validated iroh relay server URL.
pub struct RelayUrl(iroh::RelayUrl);

impl fmt::Display for RelayUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<iroh::RelayUrl> for RelayUrl {
    fn as_ref(&self) -> &iroh::RelayUrl {
        &self.0
    }
}

impl FromStr for RelayUrl {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = non_empty(value, "relay_url")?;
        let url = iroh::RelayUrl::from_str(value).map_err(ConfigError::InvalidRelayUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(ConfigError::Invalid(
                "relay_url must use http or https scheme with a valid host".to_owned(),
            ));
        }
        Ok(Self(url))
    }
}

/// Read and write halves of one byte stream.
pub(crate) struct StreamParts<R, W> {
    pub(crate) reader: R,
    pub(crate) writer: W,
}

/// Byte counts copied across a bidirectional stream bridge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BridgeStats {
    /// Bytes copied from the left reader to the right writer.
    pub(crate) left_to_right: u64,
    /// Bytes copied from the right reader to the left writer.
    pub(crate) right_to_left: u64,
}

/// Copy bytes in both directions and propagate half-close.
pub(crate) async fn bridge<Lr, Lw, Rr, Rw>(
    left: StreamParts<Lr, Lw>,
    right: StreamParts<Rr, Rw>,
) -> std::io::Result<BridgeStats>
where
    Lr: AsyncRead + Unpin + Send,
    Lw: AsyncWrite + Unpin + Send,
    Rr: AsyncRead + Unpin + Send,
    Rw: AsyncWrite + Unpin + Send,
{
    let StreamParts {
        reader: mut left_reader,
        writer: mut left_writer,
    } = left;
    let StreamParts {
        reader: mut right_reader,
        writer: mut right_writer,
    } = right;
    let left_to_right = async {
        let bytes = copy(&mut left_reader, &mut right_writer).await?;
        right_writer.shutdown().await?;
        Ok::<u64, std::io::Error>(bytes)
    };
    let right_to_left = async {
        let bytes = copy(&mut right_reader, &mut left_writer).await?;
        left_writer.shutdown().await?;
        Ok::<u64, std::io::Error>(bytes)
    };

    let (left_to_right, right_to_left) = tokio::try_join!(left_to_right, right_to_left)?;
    Ok(BridgeStats {
        left_to_right,
        right_to_left,
    })
}

/// Opaque boundary for the concrete iroh endpoint used by both modes.
pub struct IrohEndpoint {
    pub(crate) endpoint: iroh::Endpoint,
    pub(crate) relay: Option<RelayUrl>,
}

/// Peer transport used by access and listen orchestration.
pub(crate) trait PeerTransport {
    type Reader: AsyncRead + Unpin + Send + 'static;
    type Writer: AsyncWrite + Unpin + Send + 'static;

    async fn connect(&self, peer: &NodeId) -> AppResult<StreamParts<Self::Reader, Self::Writer>>;

    async fn accept(&self) -> AppResult<(NodeId, StreamParts<Self::Reader, Self::Writer>)>;
}

impl IrohEndpoint {
    /// Bind an iroh endpoint using a persistent identity and optional custom relay.
    pub async fn bind(identity_path: &Path, relay: Option<RelayUrl>) -> AppResult<Self> {
        let secret_key = load_identity(identity_path).await?;
        let endpoint = endpoint_builder(relay.as_ref())
            .secret_key(secret_key)
            .bind()
            .await
            .map_err(iroh_error)?;
        Ok(Self { endpoint, relay })
    }

    /// Bind an ephemeral access endpoint without persisting an identity.
    pub async fn bind_ephemeral(relay: Option<RelayUrl>) -> AppResult<Self> {
        let endpoint = endpoint_builder(relay.as_ref())
            .bind()
            .await
            .map_err(iroh_error)?;
        Ok(Self { endpoint, relay })
    }

    /// Return this endpoint's stable public identifier.
    pub fn id(&self) -> NodeId {
        NodeId::from(self.endpoint.id())
    }

    pub(crate) fn target_addr(&self, peer: NodeId) -> iroh::EndpointAddr {
        target_endpoint_addr(peer, self.relay.as_ref())
    }

    pub(crate) async fn connect_addr(
        &self,
        address: impl Into<iroh::EndpointAddr>,
    ) -> AppResult<IrohStreamParts> {
        let connection =
            tokio::time::timeout(IROH_CONNECT_TIMEOUT, self.endpoint.connect(address, ALPN))
                .await
                .map_err(|_| timeout_error("timed out connecting to worker"))?
                .map_err(iroh_error)?;
        let (send, recv) = tokio::time::timeout(IROH_CONNECT_TIMEOUT, connection.open_bi())
            .await
            .map_err(|_| timeout_error("timed out opening worker stream"))?
            .map_err(iroh_error)?;
        Ok(StreamParts {
            reader: recv,
            writer: send,
        })
    }

    pub(crate) async fn accept_stream(&self) -> AppResult<(NodeId, IrohStreamParts)> {
        let incoming =
            self.endpoint.accept().await.ok_or_else(|| {
                std::io::Error::new(ErrorKind::UnexpectedEof, "iroh endpoint closed")
            })?;
        let accepting = incoming.accept().map_err(iroh_error)?;
        let connection = tokio::time::timeout(IROH_ACCEPT_TIMEOUT, accepting)
            .await
            .map_err(|_| timeout_error("timed out accepting iroh connection"))?
            .map_err(iroh_error)?;
        let (send, recv) = tokio::time::timeout(IROH_ACCEPT_TIMEOUT, connection.accept_bi())
            .await
            .map_err(|_| timeout_error("timed out accepting iroh stream"))?
            .map_err(iroh_error)?;
        Ok((
            NodeId::from(connection.remote_id()),
            StreamParts {
                reader: recv,
                writer: send,
            },
        ))
    }

    #[cfg(test)]
    pub(crate) fn from_endpoint(endpoint: iroh::Endpoint) -> Self {
        Self {
            endpoint,
            relay: None,
        }
    }
}

fn endpoint_builder(relay: Option<&RelayUrl>) -> iroh::endpoint::Builder {
    let builder = iroh::Endpoint::builder(iroh::endpoint::presets::N0).alpns(vec![ALPN.to_vec()]);
    match relay {
        Some(url) => builder.relay_mode(iroh::endpoint::RelayMode::custom([url.as_ref().clone()])),
        None => builder,
    }
}

pub(crate) fn target_endpoint_addr(peer: NodeId, relay: Option<&RelayUrl>) -> iroh::EndpointAddr {
    let addr = iroh::EndpointAddr::new(peer.into());
    match relay {
        Some(url) => addr.with_relay_url(url.as_ref().clone()),
        None => addr,
    }
}

impl PeerTransport for IrohEndpoint {
    type Reader = iroh::endpoint::RecvStream;
    type Writer = iroh::endpoint::SendStream;

    async fn connect(&self, peer: &NodeId) -> AppResult<StreamParts<Self::Reader, Self::Writer>> {
        self.connect_addr(self.target_addr(*peer)).await
    }

    async fn accept(&self) -> AppResult<(NodeId, StreamParts<Self::Reader, Self::Writer>)> {
        self.accept_stream().await
    }
}

/// Load or create the persistent worker secret key and return its public [`NodeId`].
pub async fn load_node_id(path: &Path) -> AppResult<NodeId> {
    let secret_key = load_identity(path).await?;
    Ok(NodeId::from(secret_key.public()))
}

pub(crate) async fn load_identity(path: &Path) -> AppResult<iroh::SecretKey> {
    match tokio::fs::read(path).await {
        Ok(bytes) => parse_identity(path, &bytes).await,
        Err(error) if error.kind() == ErrorKind::NotFound => create_identity(path).await,
        Err(error) => Err(error.into()),
    }
}

async fn parse_identity(path: &Path, bytes: &[u8]) -> AppResult<iroh::SecretKey> {
    let secret_key =
        iroh::SecretKey::try_from(bytes).map_err(|error| AppError::Iroh(Box::new(error)))?;
    restrict_identity_permissions(path).await?;
    Ok(secret_key)
}

async fn create_identity(path: &Path) -> AppResult<iroh::SecretKey> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let secret_key = iroh::SecretKey::generate();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("worker"),
        std::process::id(),
        counter
    ));
    let result = match write_identity(&temp_path, &secret_key).await {
        Ok(()) => publish_identity(&temp_path, path, secret_key).await,
        Err(error) => Err(error),
    };
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp_path).await;
    }
    result
}

async fn write_identity(temp_path: &Path, secret_key: &iroh::SecretKey) -> AppResult<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(temp_path).await?;
    file.write_all(&secret_key.to_bytes()).await?;
    file.sync_all().await?;
    Ok(())
}

async fn publish_identity(
    temp_path: &Path,
    path: &Path,
    secret_key: iroh::SecretKey,
) -> AppResult<iroh::SecretKey> {
    let result = match tokio::fs::hard_link(temp_path, path).await {
        Ok(()) => Ok(secret_key),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let bytes = tokio::fs::read(path).await?;
            parse_identity(path, &bytes).await
        }
        Err(error) => Err(error.into()),
    };
    let _ = tokio::fs::remove_file(temp_path).await;
    result
}

#[cfg(unix)]
async fn restrict_identity_permissions(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = tokio::fs::metadata(path).await?.permissions();
    permissions.set_mode(0o600);
    tokio::fs::set_permissions(path, permissions).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn restrict_identity_permissions(_path: &Path) -> AppResult<()> {
    Ok(())
}

/// Resolve the default persistent worker key path.
pub fn default_identity_path() -> AppResult<PathBuf> {
    Ok(default_config_base_dir()?.join("sshp").join("worker.key"))
}

/// Resolve the default XDG config base directory (`$XDG_CONFIG_HOME` or `$HOME/.config`).
pub fn default_config_base_dir() -> AppResult<PathBuf> {
    resolve_config_base_dir(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

#[cfg(test)]
pub(crate) fn resolve_config_dir(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> AppResult<PathBuf> {
    Ok(resolve_config_base_dir(xdg_config_home, home)?.join("sshp"))
}

pub(crate) fn resolve_config_base_dir(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> AppResult<PathBuf> {
    match xdg_config_home {
        Some(path) => absolute_path(path, "XDG_CONFIG_HOME"),
        None => {
            Ok(absolute_path(home.ok_or(ConfigError::Missing("HOME"))?, "HOME")?.join(".config"))
        }
    }
}

pub(crate) fn absolute_path(path: PathBuf, field: &'static str) -> AppResult<PathBuf> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(ConfigError::Invalid(format!("{field} must be an absolute path")).into());
    }
    Ok(path)
}

pub(crate) fn non_empty<'a>(value: &'a str, field: &'static str) -> Result<&'a str, ConfigError> {
    if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
        return Err(ConfigError::Invalid(format!(
            "{field} must not be empty or contain whitespace"
        )));
    }
    Ok(value)
}

pub(crate) fn write_file_atomic(path: &Path, contents: &[u8], restrict_dir: bool) -> AppResult<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let parent_existed = parent.exists();
    fs::create_dir_all(parent)?;
    if restrict_dir && (!parent_existed || parent.file_name() == Some(std::ffi::OsStr::new(".ssh")))
    {
        restrict_dir_permissions(parent)?;
    }

    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".{}-tmp-{}-{counter}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sshp"),
        std::process::id()
    ));
    let mut created = false;
    let result = (|| -> AppResult<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp_path)?;
        created = true;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(unix)]
fn restrict_dir_permissions(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_dir_permissions(_path: &Path) -> AppResult<()> {
    Ok(())
}

#[cfg(test)]
pub(crate) struct TestDir(PathBuf);

#[cfg(test)]
impl TestDir {
    pub(crate) fn new() -> Self {
        loop {
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("sshp-test-{}-{counter}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("test directory: {error}"),
            }
        }
    }

    pub(crate) fn path(&self) -> &PathBuf {
        &self.0
    }

    pub(crate) fn cleanup(&mut self) -> std::io::Result<()> {
        fs::remove_dir_all(&self.0)?;
        self.0 = PathBuf::new();
        Ok(())
    }
}

#[cfg(test)]
impl Drop for TestDir {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_supports_display_and_conversions() {
        let endpoint_id = iroh::SecretKey::generate().public();
        let node_id = NodeId::from(endpoint_id);
        let borrowed: &iroh::EndpointId = node_id.as_ref();

        assert_eq!(borrowed, &endpoint_id);
        let owned: iroh::EndpointId = node_id.into();
        assert_eq!(owned, endpoint_id);
        assert!(matches!(
            "not-an-endpoint-id".parse::<NodeId>(),
            Err(ConfigError::InvalidNodeId(_))
        ));
    }

    #[test]
    fn relay_url_validates_and_builds_target_addr() {
        let relay: RelayUrl = "https://relay.example.com:3340"
            .parse()
            .expect("valid https relay");
        assert_eq!(relay.to_string(), "https://relay.example.com:3340/");

        let http_relay: RelayUrl = "http://127.0.0.1:3340".parse().expect("valid http relay");
        assert_eq!(http_relay.to_string(), "http://127.0.0.1:3340/");

        let peer = NodeId::from(iroh::SecretKey::from([9; 32]).public());
        let without_relay = target_endpoint_addr(peer, None);
        assert_eq!(without_relay.relay_urls().count(), 0);

        let with_relay = target_endpoint_addr(peer, Some(&relay));
        assert_eq!(
            with_relay.relay_urls().collect::<Vec<_>>(),
            vec![relay.as_ref()]
        );

        for invalid in [
            "",
            "https://relay.example.com/bad path",
            "ftp://relay.example.com",
            "unix:/tmp/relay.sock",
            "not-a-url",
        ] {
            assert!(invalid.parse::<RelayUrl>().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn resolves_config_paths_and_rejects_invalid_values() {
        let xdg = resolve_config_dir(
            Some(PathBuf::from("/xdg")),
            Some(PathBuf::from("/home/user")),
        )
        .expect("valid XDG path");
        assert_eq!(
            xdg.join("worker.key"),
            PathBuf::from("/xdg/sshp/worker.key")
        );

        let fallback =
            resolve_config_dir(None, Some(PathBuf::from("/home/user"))).expect("valid HOME path");
        assert_eq!(fallback, PathBuf::from("/home/user/.config/sshp"));

        for (xdg, home) in [
            (
                Some(PathBuf::from("relative")),
                Some(PathBuf::from("/home/user")),
            ),
            (None, Some(PathBuf::from("relative"))),
            (None, None),
        ] {
            assert!(resolve_config_dir(xdg, home).is_err());
        }
    }

    #[tokio::test]
    async fn identity_is_persistent_and_concurrent_creation_is_safe() {
        let mut directory = TestDir::new();
        let path = directory.path().join("worker.key");
        let (first, second) = tokio::join!(load_identity(&path), load_identity(&path));
        let first = first.expect("first identity");
        let second = second.expect("second identity");

        assert_eq!(first.public(), second.public());
        assert_eq!(
            tokio::fs::read(&path).await.expect("identity file").len(),
            32
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = tokio::fs::metadata(&path)
                .await
                .expect("identity metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        directory.cleanup().expect("test directory cleanup");
    }
}

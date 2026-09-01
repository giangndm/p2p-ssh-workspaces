//! Worker/listen mode.

use std::{future::Future, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::{Semaphore, TryAcquireError},
    task::JoinSet,
};

use crate::shared::{
    AppResult, BackendAddress, IrohEndpoint, ListenConfig, PeerTransport, StreamParts, bridge,
};

#[path = "listen/systemd.rs"]
mod systemd;

pub use systemd::{ServiceInstaller, ServiceRequest, SystemdInstaller, default_service_unit_path};
#[cfg(test)]
pub(crate) use systemd::{
    render_unit, systemd_quote, validate_service_request, validate_unit_path,
};

const MAX_PEERS: usize = 64;
const BACKEND_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Worker-side listener and local SSH bridge.
pub struct Listener {
    endpoint: IrohEndpoint,
    backend: BackendAddress,
    slots: Arc<Semaphore>,
}

impl Listener {
    /// Bind a listener from validated configuration.
    pub async fn from_config(config: ListenConfig) -> AppResult<Self> {
        let endpoint = IrohEndpoint::bind(&config.identity_path).await?;
        Ok(Self {
            endpoint,
            backend: config.backend,
            slots: Arc::new(Semaphore::new(MAX_PEERS)),
        })
    }

    /// Return the stable public endpoint identifier.
    pub fn id(&self) -> crate::shared::NodeId {
        self.endpoint.id()
    }

    /// Accept and handle incoming streams until an error occurs.
    pub async fn run(&self) -> AppResult<()> {
        run_listener(
            &self.endpoint,
            TcpBackend,
            &self.backend,
            Arc::clone(&self.slots),
        )
        .await
    }
}

async fn run_listener<T, B>(
    transport: &T,
    backend: B,
    backend_address: &BackendAddress,
    slots: Arc<Semaphore>,
) -> AppResult<()>
where
    T: PeerTransport,
    B: BackendConnector + Clone,
{
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            incoming = transport.accept() => {
                let (_peer, incoming) = incoming?;
                let permit = match Arc::clone(&slots).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(TryAcquireError::NoPermits) => {
                        eprintln!("sshp listen: peer limit reached");
                        continue;
                    }
                    Err(TryAcquireError::Closed) => {
                        return Err(std::io::Error::other("listener semaphore closed").into());
                    }
                };
                let backend = backend.clone();
                let backend_address = backend_address.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_connection(incoming, &backend, &backend_address).await {
                        eprintln!("sshp listen: {error}");
                    }
                });
            }
            result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(error)) = result {
                    eprintln!("sshp listen: peer task failed: {error}");
                }
            }
        }
    }
}

trait BackendConnector: Send + Sync + 'static {
    type Reader: AsyncRead + Unpin + Send + 'static;
    type Writer: AsyncWrite + Unpin + Send + 'static;

    // The listener spawns this future, so its Send bound is required.
    #[allow(clippy::manual_async_fn)]
    fn connect<'a>(
        &'a self,
        address: &'a BackendAddress,
    ) -> impl Future<Output = AppResult<StreamParts<Self::Reader, Self::Writer>>> + Send + 'a;
}

#[derive(Clone, Copy)]
struct TcpBackend;

impl BackendConnector for TcpBackend {
    type Reader = tokio::net::tcp::OwnedReadHalf;
    type Writer = tokio::net::tcp::OwnedWriteHalf;

    #[allow(clippy::manual_async_fn)]
    fn connect<'a>(
        &'a self,
        address: &'a BackendAddress,
    ) -> impl Future<Output = AppResult<StreamParts<Self::Reader, Self::Writer>>> + Send + 'a {
        async move {
            let backend = tokio::time::timeout(
                BACKEND_CONNECT_TIMEOUT,
                TcpStream::connect(address.as_ref()),
            )
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "timed out connecting to SSH backend",
                )
            })??;
            let (reader, writer) = backend.into_split();
            Ok(StreamParts { reader, writer })
        }
    }
}

async fn handle_connection<RemoteRead, RemoteWrite, B>(
    remote: StreamParts<RemoteRead, RemoteWrite>,
    backend: &B,
    backend_address: &BackendAddress,
) -> AppResult<()>
where
    RemoteRead: AsyncRead + Unpin + Send,
    RemoteWrite: AsyncWrite + Unpin + Send,
    B: BackendConnector,
{
    let backend = backend.connect(backend_address).await?;
    bridge(remote, backend).await?;
    Ok(())
}

#[cfg(test)]
#[path = "listen/tests.rs"]
mod tests;

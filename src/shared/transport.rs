use std::{io::ErrorKind, path::Path, time::Duration};

use tokio::io::{AsyncRead, AsyncWrite};

use super::identity::load_identity;
use super::{AppResult, NodeId, StreamParts, iroh_error, timeout_error};

const IROH_ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);
const IROH_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Opaque boundary for the concrete iroh endpoint used by both modes.
pub struct IrohEndpoint {
    endpoint: iroh::Endpoint,
}

/// Accepted iroh connection waiting for its application stream.
pub(crate) struct IncomingConnection {
    incoming: iroh::endpoint::Incoming,
}

/// Accepted iroh bidirectional stream.
pub(crate) struct IncomingStream {
    pub(crate) send: iroh::endpoint::SendStream,
    pub(crate) recv: iroh::endpoint::RecvStream,
}

/// Outgoing iroh bidirectional stream.
pub(crate) struct OutgoingStream {
    pub(crate) send: iroh::endpoint::SendStream,
    pub(crate) recv: iroh::endpoint::RecvStream,
}

/// Peer transport used by access and listen orchestration.
pub(crate) trait PeerTransport {
    type Reader: AsyncRead + Unpin + Send + 'static;
    type Writer: AsyncWrite + Unpin + Send + 'static;

    async fn connect(&self, peer: &NodeId) -> AppResult<StreamParts<Self::Reader, Self::Writer>>;

    async fn accept(&self) -> AppResult<(NodeId, StreamParts<Self::Reader, Self::Writer>)>;
}

impl IrohEndpoint {
    /// Bind an iroh endpoint using a persistent identity.
    pub async fn bind(identity_path: &Path) -> AppResult<Self> {
        let secret_key = load_identity(identity_path).await?;
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
            .secret_key(secret_key)
            .alpns(vec![super::ALPN.to_vec()])
            .bind()
            .await
            .map_err(iroh_error)?;
        Ok(Self { endpoint })
    }

    /// Bind an ephemeral access endpoint without persisting an identity.
    pub async fn bind_ephemeral() -> AppResult<Self> {
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
            .alpns(vec![super::ALPN.to_vec()])
            .bind()
            .await
            .map_err(iroh_error)?;
        Ok(Self { endpoint })
    }

    /// Return this endpoint's stable public identifier.
    pub fn id(&self) -> NodeId {
        NodeId::from(self.endpoint.id())
    }

    /// Accept one incoming iroh connection.
    pub(crate) async fn accept(&self) -> AppResult<IncomingConnection> {
        let incoming =
            self.endpoint.accept().await.ok_or_else(|| {
                std::io::Error::new(ErrorKind::UnexpectedEof, "iroh endpoint closed")
            })?;
        Ok(IncomingConnection { incoming })
    }

    /// Connect to one worker endpoint.
    pub(crate) async fn connect(&self, node_id: &NodeId) -> AppResult<OutgoingStream> {
        let connection = tokio::time::timeout(
            IROH_CONNECT_TIMEOUT,
            self.endpoint.connect(*node_id.as_ref(), super::ALPN),
        )
        .await
        .map_err(|_| timeout_error("timed out connecting to worker"))?
        .map_err(iroh_error)?;
        let (send, recv) = tokio::time::timeout(IROH_CONNECT_TIMEOUT, connection.open_bi())
            .await
            .map_err(|_| timeout_error("timed out opening worker stream"))?
            .map_err(iroh_error)?;
        Ok(OutgoingStream { send, recv })
    }

    #[cfg(test)]
    pub(crate) async fn connect_addr(
        &self,
        address: iroh::EndpointAddr,
    ) -> AppResult<OutgoingStream> {
        let connection = tokio::time::timeout(
            IROH_CONNECT_TIMEOUT,
            self.endpoint.connect(address, super::ALPN),
        )
        .await
        .map_err(|_| timeout_error("timed out connecting to worker"))?
        .map_err(iroh_error)?;
        let (send, recv) = tokio::time::timeout(IROH_CONNECT_TIMEOUT, connection.open_bi())
            .await
            .map_err(|_| timeout_error("timed out opening worker stream"))?
            .map_err(iroh_error)?;
        Ok(OutgoingStream { send, recv })
    }

    #[cfg(test)]
    pub(crate) fn from_endpoint(endpoint: iroh::Endpoint) -> Self {
        Self { endpoint }
    }
}

impl IncomingConnection {
    pub(crate) async fn accept_stream(self) -> AppResult<(NodeId, IncomingStream)> {
        let accepting = self.incoming.accept().map_err(iroh_error)?;
        let connection = tokio::time::timeout(IROH_ACCEPT_TIMEOUT, accepting)
            .await
            .map_err(|_| {
                std::io::Error::new(ErrorKind::TimedOut, "timed out accepting iroh connection")
            })?
            .map_err(iroh_error)?;
        let (send, recv) = tokio::time::timeout(IROH_ACCEPT_TIMEOUT, connection.accept_bi())
            .await
            .map_err(|_| {
                std::io::Error::new(ErrorKind::TimedOut, "timed out accepting iroh stream")
            })?
            .map_err(iroh_error)?;
        Ok((
            NodeId::from(connection.remote_id()),
            IncomingStream { send, recv },
        ))
    }
}

impl PeerTransport for IrohEndpoint {
    type Reader = iroh::endpoint::RecvStream;
    type Writer = iroh::endpoint::SendStream;

    async fn connect(&self, peer: &NodeId) -> AppResult<StreamParts<Self::Reader, Self::Writer>> {
        let OutgoingStream { send, recv } = IrohEndpoint::connect(self, peer).await?;
        Ok(StreamParts {
            reader: recv,
            writer: send,
        })
    }

    async fn accept(&self) -> AppResult<(NodeId, StreamParts<Self::Reader, Self::Writer>)> {
        let incoming = IrohEndpoint::accept(self).await?;
        let (peer, stream) = incoming.accept_stream().await?;
        let IncomingStream { send, recv } = stream;
        Ok((
            peer,
            StreamParts {
                reader: recv,
                writer: send,
            },
        ))
    }
}

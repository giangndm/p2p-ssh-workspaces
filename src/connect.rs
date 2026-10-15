//! Access/connect stdio pipe for OpenSSH `ProxyCommand` (`sshp connect`).

use std::time::Instant;

use tokio::io::{AsyncRead, AsyncWrite, stdin, stdout};

use crate::shared::{
    AppResult, BridgeStats, IrohEndpoint, NodeId, PeerTransport, RelayUrl, StreamParts, bridge,
};

/// Bridge OpenSSH `ProxyCommand` stdio to one worker iroh stream.
pub async fn connect_stdio(node_id: NodeId, relay: Option<RelayUrl>, trace: bool) -> AppResult<()> {
    let endpoint = IrohEndpoint::bind_ephemeral(relay).await?;
    if trace {
        eprintln!("sshp connect: local endpoint {}", endpoint.id());
        eprintln!("sshp connect: connecting to {node_id}");
    }
    let stats = run_proxy(&endpoint, node_id, stdin(), stdout(), trace).await?;
    if trace {
        eprintln!(
            "sshp connect: closed {node_id} (tx={}B, rx={}B)",
            stats.left_to_right, stats.right_to_left
        );
    }
    Ok(())
}

pub(crate) async fn run_proxy<T, Input, Output>(
    transport: &T,
    node_id: NodeId,
    input: Input,
    output: Output,
    trace: bool,
) -> AppResult<BridgeStats>
where
    T: PeerTransport,
    Input: AsyncRead + Unpin + Send,
    Output: AsyncWrite + Unpin + Send,
{
    let start = Instant::now();
    let remote = transport.connect(&node_id).await?;
    if trace {
        eprintln!(
            "sshp connect: connected to {node_id} in {}ms",
            start.elapsed().as_millis()
        );
    }
    let stats = bridge(
        StreamParts {
            reader: input,
            writer: output,
        },
        remote,
    )
    .await?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

    use super::*;

    fn valid_node_id() -> NodeId {
        iroh::SecretKey::from([7; 32])
            .public()
            .to_string()
            .parse()
            .expect("valid node id")
    }

    type MemoryParts = StreamParts<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

    struct FakeTransport {
        stream: Mutex<Option<MemoryParts>>,
        expected_peer: NodeId,
    }

    impl FakeTransport {
        fn new(stream: DuplexStream, expected_peer: NodeId) -> Self {
            let (reader, writer) = tokio::io::split(stream);
            Self {
                stream: Mutex::new(Some(StreamParts { reader, writer })),
                expected_peer,
            }
        }
    }

    impl PeerTransport for FakeTransport {
        type Reader = ReadHalf<DuplexStream>;
        type Writer = WriteHalf<DuplexStream>;

        async fn connect(&self, peer: &NodeId) -> AppResult<MemoryParts> {
            assert_eq!(peer, &self.expected_peer);
            self.stream
                .lock()
                .expect("fake transport lock")
                .take()
                .ok_or_else(|| std::io::Error::other("fake peer is unavailable").into())
        }

        async fn accept(&self) -> AppResult<(NodeId, MemoryParts)> {
            Err(std::io::Error::other("fake accept is unused").into())
        }
    }

    #[tokio::test]
    async fn forwards_stdio_in_both_directions() {
        let (mut input, proxy_input) = tokio::io::duplex(1024);
        let (proxy_output, mut output) = tokio::io::duplex(1024);
        let (mut remote_writer, proxy_remote_read) = tokio::io::duplex(1024);
        let (proxy_remote_write, mut remote_reader) = tokio::io::duplex(1024);
        let proxy_task = tokio::spawn(bridge(
            StreamParts {
                reader: proxy_input,
                writer: proxy_output,
            },
            StreamParts {
                reader: proxy_remote_read,
                writer: proxy_remote_write,
            },
        ));

        input.write_all(b"ping").await.expect("input write");
        input.shutdown().await.expect("input shutdown");
        remote_writer
            .write_all(b"pong")
            .await
            .expect("remote write");
        remote_writer.shutdown().await.expect("remote shutdown");

        let mut request = Vec::new();
        remote_reader
            .read_to_end(&mut request)
            .await
            .expect("remote read");
        let mut response = Vec::new();
        output
            .read_to_end(&mut response)
            .await
            .expect("output read");

        let stats = proxy_task.await.expect("proxy task").expect("proxy");
        assert_eq!(request, b"ping");
        assert_eq!(response, b"pong");
        assert_eq!(stats.left_to_right, 4);
        assert_eq!(stats.right_to_left, 4);
    }

    #[tokio::test]
    async fn runs_full_proxy_and_backend_flow_in_memory() {
        let (mut session, proxy_session) = tokio::io::duplex(1024);
        let (proxy_peer, worker_peer) = tokio::io::duplex(1024);
        let (worker_backend, mut backend) = tokio::io::duplex(1024);
        let transport = FakeTransport::new(proxy_peer, valid_node_id());
        let (session_reader, session_writer) = tokio::io::split(proxy_session);
        let (worker_reader, worker_writer) = tokio::io::split(worker_peer);
        let (backend_reader, backend_writer) = tokio::io::split(worker_backend);

        let worker_task = tokio::spawn(bridge(
            StreamParts {
                reader: worker_reader,
                writer: worker_writer,
            },
            StreamParts {
                reader: backend_reader,
                writer: backend_writer,
            },
        ));
        let (proxy_result, (request, response)) = tokio::join!(
            run_proxy(
                &transport,
                valid_node_id(),
                session_reader,
                session_writer,
                true
            ),
            async {
                session.write_all(b"ping").await.expect("session write");
                session.shutdown().await.expect("session shutdown");
                let mut request = Vec::new();
                backend
                    .read_to_end(&mut request)
                    .await
                    .expect("backend read");
                backend.write_all(b"pong").await.expect("backend write");
                backend.shutdown().await.expect("backend shutdown");

                let mut response = Vec::new();
                session
                    .read_to_end(&mut response)
                    .await
                    .expect("session read");
                (request, response)
            }
        );
        assert_eq!(request, b"ping");
        assert_eq!(response, b"pong");
        let stats = proxy_result.expect("proxy");
        assert_eq!(stats.left_to_right, 4);
        assert_eq!(stats.right_to_left, 4);
        worker_task.await.expect("worker task").expect("worker");
    }
}

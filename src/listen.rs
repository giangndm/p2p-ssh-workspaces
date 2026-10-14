//! Worker listener and local SSH backend bridge (`sshp listen`).

use std::{fmt, net::Ipv6Addr, path::Path, str::FromStr, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::{Semaphore, TryAcquireError},
    task::JoinSet,
};

use crate::shared::{
    AppResult, BridgeStats, ConfigError, IrohEndpoint, NodeId, PeerTransport, StreamParts, bridge,
    non_empty, timeout_error,
};

/// Default local SSH backend address.
pub const DEFAULT_BACKEND: &str = "127.0.0.1:22";

const MAX_PEERS: usize = 64;
const BACKEND_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
/// Validated local backend address.
pub struct BackendAddress(String);

impl fmt::Display for BackendAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for BackendAddress {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for BackendAddress {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = non_empty(value, "listen.backend")?;
        let (host, port) = if let Some(rest) = value.strip_prefix('[') {
            let Some((host, port)) = rest.split_once("]:") else {
                return Err(ConfigError::Invalid(
                    "listen.backend must use host:port format".to_owned(),
                ));
            };
            if host.parse::<Ipv6Addr>().is_err() {
                return Err(ConfigError::Invalid(
                    "listen.backend IPv6 host is invalid".to_owned(),
                ));
            }
            (host, port)
        } else {
            let Some((host, port)) = value.rsplit_once(':') else {
                return Err(ConfigError::Invalid(
                    "listen.backend must use host:port format".to_owned(),
                ));
            };
            if host.contains(':') || !valid_hostname(host) {
                return Err(ConfigError::Invalid(
                    "listen.backend host must be a hostname or IPv4 address".to_owned(),
                ));
            }
            (host, port)
        };
        if host.trim().is_empty() || host.chars().any(char::is_whitespace) {
            return Err(ConfigError::Invalid(
                "listen.backend host must not be empty or contain whitespace".to_owned(),
            ));
        }
        if port.parse::<u16>().is_err() || port == "0" {
            return Err(ConfigError::Invalid(
                "listen.backend port must be between 1 and 65535".to_owned(),
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

fn valid_hostname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

/// Worker-side listener and local SSH bridge.
pub struct Listener {
    endpoint: IrohEndpoint,
    backend: BackendAddress,
    slots: Arc<Semaphore>,
    trace: bool,
}

impl Listener {
    /// Bind a listener with a persistent identity path and local SSH backend.
    pub async fn bind(
        identity_path: &Path,
        backend: BackendAddress,
        trace: bool,
    ) -> AppResult<Self> {
        let endpoint = IrohEndpoint::bind(identity_path).await?;
        Ok(Self {
            endpoint,
            backend,
            slots: Arc::new(Semaphore::new(MAX_PEERS)),
            trace,
        })
    }

    /// Return the stable public endpoint identifier.
    pub fn id(&self) -> NodeId {
        self.endpoint.id()
    }

    /// Accept and handle incoming streams until an error occurs.
    pub async fn run(&self) -> AppResult<()> {
        run_listener(
            &self.endpoint,
            &self.backend,
            Arc::clone(&self.slots),
            self.trace,
        )
        .await
    }
}

async fn run_listener<T>(
    transport: &T,
    backend_address: &BackendAddress,
    slots: Arc<Semaphore>,
    trace: bool,
) -> AppResult<()>
where
    T: PeerTransport,
{
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            incoming = transport.accept() => {
                let (peer, incoming) = incoming?;
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
                if trace {
                    eprintln!("sshp listen: accepted peer {peer} -> {backend_address}");
                }
                let backend_address = backend_address.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    match handle_connection(incoming, &backend_address).await {
                        Ok(stats) => {
                            if trace {
                                eprintln!(
                                    "sshp listen: peer {peer} closed (rx={}B, tx={}B)",
                                    stats.left_to_right, stats.right_to_left
                                );
                            }
                        }
                        Err(error) => {
                            eprintln!("sshp listen: {error}");
                        }
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

async fn handle_connection<RemoteRead, RemoteWrite>(
    remote: StreamParts<RemoteRead, RemoteWrite>,
    backend_address: &BackendAddress,
) -> AppResult<BridgeStats>
where
    RemoteRead: AsyncRead + Unpin + Send,
    RemoteWrite: AsyncWrite + Unpin + Send,
{
    let backend = tokio::time::timeout(
        BACKEND_CONNECT_TIMEOUT,
        TcpStream::connect(backend_address.as_ref()),
    )
    .await
    .map_err(|_| timeout_error("timed out connecting to SSH backend"))??;
    let (reader, writer) = backend.into_split();
    let stats = bridge(remote, StreamParts { reader, writer }).await?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf},
        net::TcpListener,
        sync::Notify,
    };

    use super::*;
    use crate::shared::ALPN;

    type MemoryParts = StreamParts<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

    struct FakeTransport {
        outgoing: Mutex<Option<MemoryParts>>,
        incoming: Mutex<VecDeque<MemoryParts>>,
        incoming_ready: Notify,
        expected_peer: NodeId,
    }

    impl FakeTransport {
        fn new(
            outgoing: DuplexStream,
            incoming: impl IntoIterator<Item = DuplexStream>,
            expected_peer: NodeId,
        ) -> Self {
            let (outgoing_reader, outgoing_writer) = tokio::io::split(outgoing);
            Self {
                outgoing: Mutex::new(Some(StreamParts {
                    reader: outgoing_reader,
                    writer: outgoing_writer,
                })),
                incoming: Mutex::new(incoming.into_iter().map(split_memory).collect()),
                incoming_ready: Notify::new(),
                expected_peer,
            }
        }

        fn push_incoming(&self, stream: DuplexStream) {
            self.incoming
                .lock()
                .expect("fake transport lock")
                .push_back(split_memory(stream));
            self.incoming_ready.notify_one();
        }
    }

    impl PeerTransport for FakeTransport {
        type Reader = ReadHalf<DuplexStream>;
        type Writer = WriteHalf<DuplexStream>;

        async fn connect(&self, peer: &NodeId) -> AppResult<MemoryParts> {
            assert_eq!(peer, &self.expected_peer);
            self.outgoing
                .lock()
                .expect("fake transport lock")
                .take()
                .ok_or_else(|| std::io::Error::other("fake peer is unavailable").into())
        }

        async fn accept(&self) -> AppResult<(NodeId, MemoryParts)> {
            loop {
                let stream = self
                    .incoming
                    .lock()
                    .expect("fake transport lock")
                    .pop_front();
                if let Some(stream) = stream {
                    return Ok((valid_node_id(), stream));
                }
                self.incoming_ready.notified().await;
            }
        }
    }

    fn valid_node_id() -> NodeId {
        iroh::SecretKey::from([8; 32])
            .public()
            .to_string()
            .parse()
            .expect("valid node id")
    }

    fn split_memory(stream: DuplexStream) -> MemoryParts {
        let (reader, writer) = tokio::io::split(stream);
        StreamParts { reader, writer }
    }

    async fn bind_local_backend() -> (TcpListener, BackendAddress) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("backend listener");
        let address = listener
            .local_addr()
            .expect("backend address")
            .to_string()
            .parse()
            .expect("valid backend address");
        (listener, address)
    }

    #[test]
    fn backend_address_validates_and_converts() {
        let address: BackendAddress = "127.0.0.1:22".parse().expect("valid backend address");
        let borrowed: &str = address.as_ref();

        assert_eq!(borrowed, "127.0.0.1:22");
        assert_eq!(address.to_string(), "127.0.0.1:22");

        for invalid in ["", "localhost", "foo:22:33", "[not-ipv6]:22", "localhost:0"] {
            assert!(invalid.parse::<BackendAddress>().is_err(), "{invalid}");
        }
    }

    #[tokio::test]
    #[ignore = "requires host network-monitor permissions"]
    async fn forwards_bytes_in_both_directions() {
        let (backend_listener, backend_address) = bind_local_backend().await;
        let backend_task = tokio::spawn(async move {
            let (mut socket, _) = backend_listener.accept().await.expect("backend peer");
            let mut request = Vec::new();
            socket
                .read_to_end(&mut request)
                .await
                .expect("backend read");
            socket.write_all(b"pong").await.expect("backend write");
            socket.shutdown().await.expect("backend shutdown");
            request
        });

        let server_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .expect("server endpoint");
        let client_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .expect("client endpoint");
        let server_address = server_endpoint.addr();
        let server = IrohEndpoint::from_endpoint(server_endpoint);
        let client = IrohEndpoint::from_endpoint(client_endpoint);

        let server_task = tokio::spawn(async move {
            let (_peer, stream) = server.accept_stream().await.expect("incoming stream");
            handle_connection(stream, &backend_address)
                .await
                .expect("bridge");
        });

        let outgoing =
            tokio::time::timeout(Duration::from_secs(5), client.connect_addr(server_address))
                .await
                .expect("iroh connect timeout")
                .expect("iroh connection");
        let StreamParts {
            reader: mut recv,
            writer: mut send,
        } = outgoing;
        send.write_all(b"ping").await.expect("client write");
        send.shutdown().await.expect("client shutdown");

        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut recv, &mut response)
            .await
            .expect("client read");
        assert_eq!(response, b"pong");
        assert_eq!(backend_task.await.expect("backend task"), b"ping");
        server_task.await.expect("server task");
    }

    #[tokio::test]
    async fn forwards_full_duplex_data_and_half_closes() {
        let (mut remote, worker_remote) = tokio::io::duplex(1024);
        let (backend_listener, backend_address) = bind_local_backend().await;
        let (worker_remote_read, worker_remote_write) = tokio::io::split(worker_remote);
        let (worker_result, (request, response)) = tokio::join!(
            handle_connection(
                StreamParts {
                    reader: worker_remote_read,
                    writer: worker_remote_write,
                },
                &backend_address,
            ),
            async {
                remote.write_all(b"ping").await.expect("remote write");
                remote.shutdown().await.expect("remote shutdown");
                let (mut backend, _) = backend_listener.accept().await.expect("backend accept");
                let mut request = Vec::new();
                backend
                    .read_to_end(&mut request)
                    .await
                    .expect("backend read");
                backend.write_all(b"pong").await.expect("backend write");
                backend.shutdown().await.expect("backend shutdown");
                let mut response = Vec::new();
                remote
                    .read_to_end(&mut response)
                    .await
                    .expect("remote read");
                (request, response)
            }
        );

        worker_result.expect("worker connection");
        assert_eq!(request, b"ping");
        assert_eq!(response, b"pong");
    }

    #[tokio::test]
    async fn runs_listener_flow_with_fake_transport_and_tcp_backend() {
        let (mut session, proxy_session) = tokio::io::duplex(1024);
        let (proxy_peer, worker_peer) = tokio::io::duplex(1024);
        let (backend_listener, backend_address) = bind_local_backend().await;
        let transport = FakeTransport::new(proxy_peer, [worker_peer], valid_node_id());
        let slots = Arc::new(Semaphore::new(1));

        let flow = async {
            session.write_all(b"ping").await.expect("session write");
            session.shutdown().await.expect("session shutdown");

            let (mut backend, _) = backend_listener.accept().await.expect("backend accept");
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
        };
        let (session_reader, session_writer) = tokio::io::split(proxy_session);
        let proxy = crate::connect::run_proxy(
            &transport,
            valid_node_id(),
            session_reader,
            session_writer,
            false,
        );
        let listener = run_listener(&transport, &backend_address, slots, true);
        let proxy_and_flow = async { tokio::join!(proxy, flow) };
        tokio::pin!(listener);

        let (proxy_result, (request, response)) = tokio::select! {
            result = &mut listener => panic!("fake listener stopped: {result:?}"),
            result = proxy_and_flow => result,
        };
        proxy_result.expect("proxy flow");
        assert_eq!(request, b"ping");
        assert_eq!(response, b"pong");
    }

    #[tokio::test]
    async fn keeps_listening_after_backend_failure() {
        let (mut failed_remote, failed_worker) = tokio::io::duplex(1024);
        let (mut working_remote, working_worker) = tokio::io::duplex(1024);
        let (unused_proxy, _) = tokio::io::duplex(1);
        let (temp_listener, backend_address) = bind_local_backend().await;
        let addr_str = backend_address.to_string();
        drop(temp_listener);

        let transport = FakeTransport::new(unused_proxy, [failed_worker], valid_node_id());
        let slots = Arc::new(Semaphore::new(1));
        let listener = run_listener(&transport, &backend_address, slots, false);
        tokio::pin!(listener);

        let wait_closed = async {
            let mut buf = [0u8; 1];
            let _ = failed_remote.read(&mut buf).await;
        };
        tokio::select! {
            result = &mut listener => panic!("fake listener stopped: {result:?}"),
            () = wait_closed => {}
        }

        let rebound_listener = TcpListener::bind(&addr_str)
            .await
            .expect("rebind backend listener");
        transport.push_incoming(working_worker);

        let round_trip = async {
            working_remote.write_all(b"ok").await.expect("write");
            working_remote.shutdown().await.expect("shutdown");
            let (mut backend, _) = rebound_listener.accept().await.expect("accept");
            let mut received = Vec::new();
            backend.read_to_end(&mut received).await.expect("read");
            assert_eq!(received, b"ok");
        };
        tokio::select! {
            result = &mut listener => panic!("fake listener stopped: {result:?}"),
            () = round_trip => {}
        }
    }
}

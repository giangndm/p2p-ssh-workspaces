use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf},
    net::TcpListener,
    sync::Notify,
};

use super::*;
use crate::shared::{ALPN, IncomingStream};

type MemoryParts = StreamParts<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

#[derive(Clone)]
struct FakeBackend {
    stream: Arc<Mutex<Option<MemoryParts>>>,
}

impl FakeBackend {
    fn new(stream: DuplexStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            stream: Arc::new(Mutex::new(Some(StreamParts { reader, writer }))),
        }
    }
}

struct FakeTransport {
    outgoing: Mutex<Option<MemoryParts>>,
    incoming: Mutex<VecDeque<MemoryParts>>,
    incoming_ready: Notify,
    expected_peer: crate::shared::NodeId,
}

impl FakeTransport {
    fn new(
        outgoing: DuplexStream,
        incoming: impl IntoIterator<Item = DuplexStream>,
        expected_peer: crate::shared::NodeId,
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

    async fn connect(&self, peer: &crate::shared::NodeId) -> AppResult<MemoryParts> {
        assert_eq!(peer, &self.expected_peer);
        self.outgoing
            .lock()
            .expect("fake transport lock")
            .take()
            .ok_or_else(|| std::io::Error::other("fake peer is unavailable").into())
    }

    async fn accept(&self) -> AppResult<(crate::shared::NodeId, MemoryParts)> {
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

fn valid_node_id() -> crate::shared::NodeId {
    crate::shared::NodeId::try_from(iroh::SecretKey::from([8; 32]).public().to_string())
        .expect("valid node id")
}

fn split_memory(stream: DuplexStream) -> MemoryParts {
    let (reader, writer) = tokio::io::split(stream);
    StreamParts { reader, writer }
}

impl BackendConnector for FakeBackend {
    type Reader = ReadHalf<DuplexStream>;
    type Writer = WriteHalf<DuplexStream>;

    #[allow(clippy::manual_async_fn)]
    fn connect<'a>(
        &'a self,
        _address: &'a BackendAddress,
    ) -> impl std::future::Future<Output = AppResult<MemoryParts>> + Send + 'a {
        async move {
            self.stream
                .lock()
                .expect("fake backend lock")
                .take()
                .ok_or_else(|| std::io::Error::other("fake backend is unavailable").into())
        }
    }
}

#[derive(Clone)]
struct ToggleBackend {
    stream: Arc<Mutex<Option<MemoryParts>>>,
    available: Arc<AtomicBool>,
    failed: Arc<Notify>,
    connected: Arc<Notify>,
}

impl ToggleBackend {
    fn new(stream: DuplexStream) -> Self {
        Self {
            stream: Arc::new(Mutex::new(Some(split_memory(stream)))),
            available: Arc::new(AtomicBool::new(false)),
            failed: Arc::new(Notify::new()),
            connected: Arc::new(Notify::new()),
        }
    }

    async fn wait_for_failure(&self) {
        self.failed.notified().await;
    }

    async fn wait_for_connection(&self) {
        self.connected.notified().await;
    }

    fn allow_connections(&self) {
        self.available.store(true, Ordering::Release);
    }
}

impl BackendConnector for ToggleBackend {
    type Reader = ReadHalf<DuplexStream>;
    type Writer = WriteHalf<DuplexStream>;

    #[allow(clippy::manual_async_fn)]
    fn connect<'a>(
        &'a self,
        _address: &'a BackendAddress,
    ) -> impl std::future::Future<Output = AppResult<MemoryParts>> + Send + 'a {
        async move {
            if !self.available.load(Ordering::Acquire) {
                self.failed.notify_one();
                return Err(std::io::Error::other("fake backend is unavailable").into());
            }
            let stream = self
                .stream
                .lock()
                .expect("toggle backend lock")
                .take()
                .ok_or_else(|| std::io::Error::other("fake backend is exhausted"))?;
            self.connected.notify_one();
            Ok(stream)
        }
    }
}

#[test]
fn renders_systemd_unit_with_quoted_executable() {
    let unit = render_unit(
        Path::new("/opt/ssh workspaces/sshp"),
        Path::new("/config/base"),
    )
    .expect("unit");
    assert!(unit.contains("ExecStart=\"/opt/ssh workspaces/sshp\" listen"));
    assert!(unit.contains("Environment=XDG_CONFIG_HOME=/config/base"));
    assert!(unit.contains("Restart=always"));
    assert!(unit.contains("WantedBy=default.target"));
}

#[test]
fn rejects_unsafe_service_paths() {
    let request = ServiceRequest {
        executable: PathBuf::from("sshp"),
        unit_path: PathBuf::from("/tmp/sshp-listen.service"),
        config_dir: PathBuf::from("/config/base"),
    };
    assert!(validate_service_request(&request).is_err());
    assert!(validate_unit_path(Path::new("/tmp/other.service")).is_err());
}

#[test]
fn validates_service_root_and_systemd_expansions() {
    let request = ServiceRequest {
        executable: PathBuf::from("/usr/bin/sshp"),
        unit_path: default_service_unit_path().expect("unit path"),
        config_dir: PathBuf::from("/tmp/other-config"),
    };
    assert!(validate_service_request(&request).is_err());

    let unit = render_unit(Path::new("/opt/$HOME/sshp"), Path::new("/config/%root")).expect("unit");
    assert!(unit.contains("Environment=XDG_CONFIG_HOME=/config/%%root"));
    assert!(unit.contains("ExecStart=/opt/$$HOME/sshp listen"));
    assert!(systemd_quote("bad\nvalue", true).is_err());
}

#[tokio::test]
#[ignore = "requires host network-monitor permissions"]
async fn forwards_bytes_in_both_directions() {
    let backend_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("backend listener");
    let backend_address = BackendAddress::try_from(
        backend_listener
            .local_addr()
            .expect("backend address")
            .to_string(),
    )
    .expect("valid backend address");
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
        let incoming = server.accept().await.expect("incoming connection");
        let (_peer, stream) = incoming.accept_stream().await.expect("incoming stream");
        let IncomingStream { send, recv } = stream;
        handle_connection(
            StreamParts {
                reader: recv,
                writer: send,
            },
            &TcpBackend,
            &backend_address,
        )
        .await
        .expect("bridge");
    });

    let outgoing =
        tokio::time::timeout(Duration::from_secs(5), client.connect_addr(server_address))
            .await
            .expect("iroh connect timeout")
            .expect("iroh connection");
    let crate::shared::OutgoingStream { mut send, mut recv } = outgoing;
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
    let (worker_backend, mut backend) = tokio::io::duplex(1024);
    let backend_connector = FakeBackend::new(worker_backend);
    let backend_address = BackendAddress::try_from("fake:22".to_owned()).expect("address");
    let (worker_remote_read, worker_remote_write) = tokio::io::split(worker_remote);
    let (worker_result, (request, response)) = tokio::join!(
        handle_connection(
            StreamParts {
                reader: worker_remote_read,
                writer: worker_remote_write,
            },
            &backend_connector,
            &backend_address,
        ),
        async {
            remote.write_all(b"ping").await.expect("remote write");
            remote.shutdown().await.expect("remote shutdown");
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
async fn runs_listener_flow_with_fake_transport_and_backend() {
    let (mut session, proxy_session) = tokio::io::duplex(1024);
    let (proxy_peer, worker_peer) = tokio::io::duplex(1024);
    let (worker_backend, mut backend) = tokio::io::duplex(1024);
    let transport = FakeTransport::new(proxy_peer, [worker_peer], valid_node_id());
    let backend_connector = FakeBackend::new(worker_backend);
    let backend_address = BackendAddress::try_from("fake:22".to_owned()).expect("address");
    let slots = Arc::new(Semaphore::new(1));

    let flow = async {
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
    };
    let (session_reader, session_writer) = tokio::io::split(proxy_session);
    let proxy =
        crate::connect::run_proxy(&transport, valid_node_id(), session_reader, session_writer);
    let listener = run_listener(&transport, backend_connector, &backend_address, slots);
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
    let (_failed_remote, failed_worker) = tokio::io::duplex(1024);
    let (_working_remote, working_worker) = tokio::io::duplex(1024);
    let (working_backend, _backend_remote) = tokio::io::duplex(1024);
    let (unused_proxy, _) = tokio::io::duplex(1);
    let transport = FakeTransport::new(unused_proxy, [failed_worker], valid_node_id());
    let backend_connector = ToggleBackend::new(working_backend);
    let backend_address = BackendAddress::try_from("fake:22".to_owned()).expect("address");
    let slots = Arc::new(Semaphore::new(1));
    let listener = run_listener(
        &transport,
        backend_connector.clone(),
        &backend_address,
        slots,
    );
    tokio::pin!(listener);

    tokio::select! {
        result = &mut listener => panic!("fake listener stopped: {result:?}"),
        _ = backend_connector.wait_for_failure() => {}
    }

    backend_connector.allow_connections();
    transport.push_incoming(working_worker);
    tokio::select! {
        result = &mut listener => panic!("fake listener stopped: {result:?}"),
        _ = backend_connector.wait_for_connection() => {}
    }
}

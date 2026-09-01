use std::{collections::VecDeque, sync::Mutex, time::Duration};

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

use super::retry::{
    MAX_RETRIES, MAX_RETRY_DELAY, MIN_RETRY_DELAY, SessionRunner, retry_delay, retry_limit,
    run_ssh, run_ssh_with_cancel, should_retry,
};
use super::*;

fn valid_node_id() -> NodeId {
    NodeId::try_from(iroh::SecretKey::from([7; 32]).public().to_string()).expect("valid node id")
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

struct ScriptedRunner {
    codes: Mutex<VecDeque<i32>>,
}

impl ScriptedRunner {
    fn new(codes: impl IntoIterator<Item = i32>) -> Self {
        Self {
            codes: Mutex::new(codes.into_iter().collect()),
        }
    }
}

impl SessionRunner for ScriptedRunner {
    async fn run(&self, _executable: &OsStr, _argv: &[OsString]) -> AppResult<i32> {
        self.codes
            .lock()
            .expect("scripted runner lock")
            .pop_front()
            .ok_or_else(|| std::io::Error::other("scripted session exhausted").into())
    }
}

#[test]
fn builds_quoted_proxy_and_startup_argv() {
    let command = SshCommand::new(
        HostAlias::try_from("hanoi".to_owned()).expect("valid alias"),
        HostConfig {
            node_id: valid_node_id(),
            ssh_host: SshHost::try_from("remote".to_owned()).expect("valid SSH host"),
            startup: Some(StartupCommand {
                program: "tmux".to_owned(),
                args: vec![
                    "new-session".to_owned(),
                    "workspace name".to_owned(),
                    "a'b".to_owned(),
                ],
            }),
            reconnect: true,
        },
        vec![OsString::from("-v")],
        PathBuf::from("/usr/local/bin/sshp"),
    );

    assert_eq!(
        command.argv().expect("valid SSH argv"),
        vec![
            OsString::from("-v"),
            OsString::from("-o"),
            OsString::from("ProxyCommand='/usr/local/bin/sshp' 'connect' '--stdio' 'hanoi'"),
            OsString::from("remote"),
            OsString::from("'tmux' 'new-session' 'workspace name' 'a'\"'\"'b'")
        ]
    );
}

#[test]
fn omits_remote_command_for_login_shell() {
    let command = SshCommand::new(
        HostAlias::try_from("shell".to_owned()).expect("valid alias"),
        HostConfig {
            node_id: valid_node_id(),
            ssh_host: SshHost::try_from("shell".to_owned()).expect("valid SSH host"),
            startup: None,
            reconnect: true,
        },
        Vec::new(),
        PathBuf::from("sshp"),
    );

    assert_eq!(command.argv().expect("valid SSH argv").len(), 3);
}

#[test]
fn retries_only_connection_exit_code() {
    assert!(should_retry(true, Some(255)));
    assert!(!should_retry(false, Some(255)));
    assert!(!should_retry(true, Some(1)));
    assert!(!should_retry(true, None));
}

#[test]
fn retry_delay_has_a_safe_floor() {
    assert_eq!(
        retry_delay(&RetryPolicy {
            enabled: true,
            max_delay: Duration::ZERO,
            max_retries: 1,
        }),
        MIN_RETRY_DELAY
    );
    assert_eq!(
        retry_delay(&RetryPolicy {
            enabled: true,
            max_delay: Duration::from_secs(2),
            max_retries: 1,
        }),
        Duration::from_secs(2)
    );
    assert_eq!(
        retry_delay(&RetryPolicy {
            enabled: true,
            max_delay: Duration::from_secs(120),
            max_retries: 1,
        }),
        MAX_RETRY_DELAY
    );
}

#[test]
fn retry_limit_has_a_safe_ceiling() {
    assert_eq!(
        retry_limit(&RetryPolicy {
            enabled: true,
            max_delay: Duration::from_secs(1),
            max_retries: u32::MAX,
        }),
        MAX_RETRIES
    );
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

    proxy_task.await.expect("proxy task").expect("proxy");
    assert_eq!(request, b"ping");
    assert_eq!(response, b"pong");
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
        run_proxy(&transport, valid_node_id(), session_reader, session_writer,),
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
    proxy_result.expect("proxy");
    worker_task.await.expect("worker task").expect("worker");
}

#[tokio::test]
async fn retries_scripted_sessions_without_ssh() {
    let runner = ScriptedRunner::new([255, 255, 0]);
    let policy = RetryPolicy {
        enabled: true,
        max_delay: Duration::from_millis(100),
        max_retries: 2,
    };

    let code = run_ssh(&runner, OsStr::new("fake-ssh"), &[], &policy)
        .await
        .expect("scripted session result");

    assert_eq!(code, 0);
    assert!(
        runner
            .codes
            .lock()
            .expect("scripted runner lock")
            .is_empty()
    );
}

#[tokio::test]
async fn returns_130_for_injected_cancellation() {
    let runner = ScriptedRunner::new([255]);
    let policy = RetryPolicy {
        enabled: true,
        max_delay: Duration::from_secs(1),
        max_retries: 1,
    };

    let code = run_ssh_with_cancel(
        &runner,
        OsStr::new("fake-ssh"),
        &[],
        &policy,
        std::future::ready(Ok(())),
    )
    .await
    .expect("cancelled session result");

    assert_eq!(code, 130);
}

#[tokio::test]
async fn runs_connector_with_scripted_session() {
    let connector = Connector::new(RetryPolicy {
        enabled: false,
        max_delay: Duration::from_millis(100),
        max_retries: 0,
    });
    let host = HostConfig {
        node_id: valid_node_id(),
        ssh_host: SshHost::try_from("fake-host".to_owned()).expect("valid SSH host"),
        startup: None,
        reconnect: false,
    };
    let runner = ScriptedRunner::new([0]);

    let code = connector
        .run_with_runner(
            HostAlias::try_from("fake".to_owned()).expect("valid alias"),
            host,
            Vec::new(),
            &runner,
        )
        .await
        .expect("scripted connector result");

    assert_eq!(code, 0);
}

//! Access/connect stdio pipe for OpenSSH `ProxyCommand` (`sshp connect`) and
//! automatic interactive session reconnection (`sshp autoconnect`).

use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use tokio::io::{AsyncRead, AsyncWrite, stdin, stdout};

use crate::{
    config::HostAlias,
    shared::{
        AppResult, BridgeStats, ConfigError, IrohEndpoint, NodeId, PeerTransport, RelayUrl,
        StreamParts, bridge,
    },
};

const SSH_DISCONNECT_EXIT_CODE: i32 = 255;
const SIGNAL_EXIT_CODE: i32 = 130;
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(10);
const STABLE_SESSION_THRESHOLD: Duration = Duration::from_secs(10);

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

/// Run an interactive `ssh` session and automatically reconnect when the transport drops.
pub async fn autoconnect(host: HostAlias, command: Option<String>, trace: bool) -> AppResult<i32> {
    let ssh_args = build_ssh_args(&host, command.as_deref())?;
    run_autoconnect_loop(
        &host,
        &ssh_args,
        trace,
        |args| {
            let args = args.to_vec();
            async move {
                tokio::task::spawn_blocking(move || run_ssh_once(&args))
                    .await
                    .map_err(|error| {
                        std::io::Error::other(format!("ssh supervisor task failed: {error}"))
                    })?
            }
        },
        tokio::time::sleep,
    )
    .await
}

fn run_ssh_once(args: &[String]) -> AppResult<(Option<i32>, Duration)> {
    let start = Instant::now();
    let status = Command::new("ssh")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| {
            std::io::Error::new(error.kind(), format!("failed to execute ssh: {error}"))
        })?;
    Ok((status.code(), start.elapsed()))
}

pub(crate) fn build_ssh_args(host: &HostAlias, command: Option<&str>) -> AppResult<Vec<String>> {
    let mut args = vec![
        "-o".to_owned(),
        "ServerAliveInterval=5".to_owned(),
        "-o".to_owned(),
        "ServerAliveCountMax=2".to_owned(),
    ];
    if let Some(command) = command {
        validate_remote_command(command)?;
        args.push("-t".to_owned());
        args.push("--".to_owned());
        args.push(host.to_string());
        args.push(command.to_owned());
    } else {
        args.push("--".to_owned());
        args.push(host.to_string());
    }
    Ok(args)
}

fn validate_remote_command(command: &str) -> Result<(), ConfigError> {
    if command.trim().is_empty() {
        return Err(ConfigError::Invalid(
            "remote command must not be empty".to_owned(),
        ));
    }
    if command.chars().any(char::is_control) {
        return Err(ConfigError::Invalid(
            "remote command must not contain control characters".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn next_reconnect_delay(
    previous_delay: Option<Duration>,
    session_duration: Duration,
) -> Duration {
    match previous_delay {
        Some(delay) if session_duration < STABLE_SESSION_THRESHOLD => {
            delay.saturating_mul(2).min(MAX_RECONNECT_DELAY)
        }
        _ => INITIAL_RECONNECT_DELAY,
    }
}

pub(crate) async fn run_autoconnect_loop<Run, RunFut, Sleep, SleepFut>(
    host: &HostAlias,
    ssh_args: &[String],
    trace: bool,
    mut run_attempt: Run,
    mut sleep_fn: Sleep,
) -> AppResult<i32>
where
    Run: FnMut(&[String]) -> RunFut,
    RunFut: std::future::Future<Output = AppResult<(Option<i32>, Duration)>>,
    Sleep: FnMut(Duration) -> SleepFut,
    SleepFut: std::future::Future<Output = ()>,
{
    let mut delay = None;
    loop {
        if trace {
            eprintln!("sshp autoconnect: starting ssh {}", ssh_args.join(" "));
        }
        let (exit_code, elapsed) = run_attempt(ssh_args).await?;
        match exit_code {
            Some(SSH_DISCONNECT_EXIT_CODE) => {
                let next_delay = next_reconnect_delay(delay, elapsed);
                delay = Some(next_delay);
                eprintln!(
                    "sshp autoconnect: connection to {host} lost after {}ms, reconnecting in {}s...",
                    elapsed.as_millis(),
                    next_delay.as_secs()
                );
                sleep_fn(next_delay).await;
            }
            Some(code) => return Ok(code),
            None => return Ok(SIGNAL_EXIT_CODE),
        }
    }
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
    use std::{collections::VecDeque, sync::Mutex};

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

    #[test]
    fn builds_ssh_args_and_validates_remote_command() {
        let host: HostAlias = "hanoi".parse().expect("valid alias");
        assert_eq!(
            build_ssh_args(&host, None).expect("default ssh args"),
            vec![
                "-o",
                "ServerAliveInterval=5",
                "-o",
                "ServerAliveCountMax=2",
                "--",
                "hanoi",
            ]
        );
        assert_eq!(
            build_ssh_args(&host, Some("tmux new -A -s work")).expect("command ssh args"),
            vec![
                "-o",
                "ServerAliveInterval=5",
                "-o",
                "ServerAliveCountMax=2",
                "-t",
                "--",
                "hanoi",
                "tmux new -A -s work",
            ]
        );

        for invalid in ["", "   ", "tmux\nattach", "tmux\0attach"] {
            assert!(build_ssh_args(&host, Some(invalid)).is_err(), "{invalid:?}");
        }
    }

    #[tokio::test]
    async fn autoconnect_loop_retries_on_disconnect_and_resets_backoff_after_stable_session() {
        let host: HostAlias = "hanoi".parse().expect("valid alias");
        let ssh_args = build_ssh_args(&host, Some("tmux new -A -s work")).expect("ssh args");
        let attempts = Mutex::new(VecDeque::from([
            (Some(255), Duration::from_millis(200)),
            (Some(255), Duration::from_millis(500)),
            (Some(255), Duration::from_secs(15)),
            (Some(0), Duration::from_secs(30)),
        ]));
        let sleeps = Mutex::new(Vec::new());

        let exit_code = run_autoconnect_loop(
            &host,
            &ssh_args,
            true,
            |args| {
                assert_eq!(args, ssh_args.as_slice());
                let outcome = attempts
                    .lock()
                    .expect("attempts lock")
                    .pop_front()
                    .expect("queued attempt");
                async move { Ok(outcome) }
            },
            |delay| {
                sleeps.lock().expect("sleeps lock").push(delay);
                async {}
            },
        )
        .await
        .expect("autoconnect loop");

        assert_eq!(exit_code, 0);
        assert_eq!(
            *sleeps.lock().expect("sleeps lock"),
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(1),
            ]
        );

        let signal_exit = run_autoconnect_loop(
            &host,
            &ssh_args,
            false,
            |_| async { Ok((None, Duration::from_secs(1))) },
            |_| async {},
        )
        .await
        .expect("signal exit");
        assert_eq!(signal_exit, SIGNAL_EXIT_CODE);
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

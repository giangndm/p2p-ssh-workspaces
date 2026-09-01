# Skeleton plan and test contracts

## Goal

Implement the [architecture](architecture.md) and [flows](flows.md) in small
vertical slices. Shared config, iroh transport, SSH execution, and reconnect
are implemented. The testable core uses private transport/session seams and
in-memory streams.

## Files

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | module wiring |
| `src/shared/` | shared types, errors, config, identity, streams, iroh boundary |
| `src/listen.rs` | listener orchestration and service boundary |
| `src/listen/systemd.rs` | Debian/Ubuntu service installation |
| `src/connect.rs` | SSH orchestration and reconnect boundary |
| `src/connect/retry.rs` | session runner and retry policy |
| `src/main.rs` | CLI wiring |

## Abstraction slice

The abstraction slice is implemented as a small byte bridge and private
adapters:

- `StreamParts` and Tokio async traits for raw byte streams;
- `PeerTransport` for iroh or in-memory peer connections;
- `BackendConnector` for TCP or in-memory worker backends;
- `SessionRunner` for OpenSSH or scripted session results;
- orchestration functions generic only over these seams.

Do not add a public plugin API, a mock framework, or a custom SSH protocol.

The CLI slice defines the two Clap subcommands, typed application errors,
default paths, and small newtypes with `derive_more` conversions.

The current application uses boxed source errors for the SSH and service
boundaries to keep the initial implementation small.

The listener bridge now isolates peers in bounded tasks and applies connection
timeouts. Full-duplex iroh integration remains an integration-test concern.

## Test contracts

### Shared/config

1. Missing `listen.backend` uses `127.0.0.1:22`.
2. Missing `host.node_id` is rejected before endpoint creation.
3. Empty `startup` means the login shell.
4. Non-empty `startup` preserves program and argument boundaries.
5. An invalid or empty program is rejected.

### Listen

1. The listener advertises only `sshp/1`.
2. A peer gets one backend connection per accepted SSH stream.
3. A backend failure closes only that peer and does not stop the listener loop.
4. Worker identity remains stable across restarts (integration test).
5. The generated systemd unit uses an absolute executable path and
   `Restart=always`.

### Connect

1. An alias resolves to the configured endpoint ID and SSH host.
2. ProxyCommand invokes the same binary's stdio path.
3. Startup argv is passed to OpenSSH as one safely quoted remote command.
4. A clean SSH exit does not retry.
5. An SSH exit code 255 follows a fixed 100 ms–60 s delay until the
   64-retry policy limit.
6. `Ctrl-C` cancels SSH or retry and returns 130.
7. The stdio proxy sends no status text to stdout.

### Integration

1. Two local iroh endpoints can exchange bytes using `sshp/1` (integration test).
2. A local fake SSH backend receives exactly the bytes sent by the proxy.
3. A disconnect followed by reconnect reaches the same persistent tmux/Herdr
   command.
4. Tests never use a production relay, real SSH keys, or the developer's
   config directory.

### In-memory full-flow tests (implemented)

1. A fake session and fake peer transport exchange bytes through the same
   bridge used by production orchestration.
2. A scripted SSH connection failure produces the configured reconnect sequence.
3. Exit code 255 retries; clean exit and non-255 exit stop without retry.
4. Retry limits remain bounded without external processes; process cancellation
   stays in the OpenSSH adapter contract.
5. The fake worker backend receives exactly the bytes sent by the fake session.

Unit tests do not bind the default iroh `N0` preset because that can require
host network-monitor permissions. Endpoint round-trip tests belong in the
network integration suite.

## Handoff rule

Implement one vertical slice at a time:

```text
config -> iroh listen -> local bridge -> stdio proxy -> ssh process
       -> startup command -> reconnect -> systemd
```

After each slice, update this plan and the relevant user-facing
documentation.

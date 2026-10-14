# Skeleton plan and test contracts

## Goal

Implement the [architecture](architecture.md) and [flows](flows.md) in small
vertical slices. OpenSSH config generation, iroh transport, stdio proxying, and
endpoint tracing are implemented. The testable core uses private transport
seams and in-memory streams.

## Files

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | module wiring |
| `src/shared.rs` | `NodeId`, errors, paths, identity, stream bridge, atomic file writes, iroh boundary |
| `src/listen.rs` | listener orchestration, `BackendAddress`, and local TCP backend bridge |
| `src/connect.rs` | `ProxyCommand` stdio pipe |
| `src/config.rs` | OpenSSH `Host` block `enable`/`disable` and systemd service `install`/`uninstall` |
| `src/trace.rs` | endpoint connectivity probe and trace formatting |
| `src/main.rs` | CLI wiring |

## Abstraction slice

The abstraction slice is implemented as a small byte bridge and a private
peer-transport adapter:

- `StreamParts`, `BridgeStats`, and Tokio async traits for raw byte streams;
- `PeerTransport` for iroh or in-memory peer connections;
- orchestration functions generic only over this seam.

Do not add a custom TOML config file, a public plugin API, a mock framework, or
a custom SSH protocol.

## Test contracts

### Shared / Config

1. Invalid `NodeId`, `HostAlias`, and `BackendAddress` values are rejected.
2. `SshConfigEntry::render` quotes executable paths and identity files safely.
3. `upsert_ssh_config` appends new managed blocks and replaces existing ones
   idempotently; `disable_ssh_config` comments out `ProxyCommand` while
   preserving other directives.
4. `write_ssh_config_entry` and `disable_ssh_config_entry` write atomically
   with `0600` file permissions on Unix.
5. Worker identity creation is atomic, persistent, and concurrent-safe.

### Listen

1. The listener advertises only `sshp/1`.
2. A peer gets one backend connection per accepted SSH stream.
3. A backend failure closes only that peer and does not stop the listener loop.
4. Worker identity remains stable across restarts.
5. The generated systemd unit uses an absolute executable path and
   `Restart=always`.

### Connect and Trace

1. `run_proxy` bridges bytes in both directions and reports transferred byte
   counts.
2. The stdio proxy sends no status text to `stdout`.
3. `format_trace_report` renders deterministic diagnostic fields.
4. Invalid endpoint IDs are rejected before binding an iroh endpoint.

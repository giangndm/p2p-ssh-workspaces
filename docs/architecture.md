# Architecture

## Goal

Let OpenSSH connect to a remote machine through a stable alias in
`~/.ssh/config`, even when the worker changes networks or is behind NAT. The
worker needs no public SSH port, and the client needs no custom wrapper around
`ssh`.

## Data flow

```text
client
  ssh hanoi
    └─ ProxyCommand: sshp connect <node-id>
        └─ iroh QUIC / ALPN sshp/1
            └─ sshp listen
                └─ TCP 127.0.0.1:22
                    └─ sshd
```

An iroh endpoint ID is derived from its public key. The listener persists its
secret key (`~/.config/sshp/worker.key`) so the ID remains stable. The client
stores this ID directly in the `ProxyCommand` line inside `~/.ssh/config`. Iroh
handles discovery, relays, and direct connectivity.

References: [iroh endpoints](https://docs.iroh.computer/concepts/endpoints),
[iroh QUIC](https://docs.iroh.computer/protocols/using-quic).

## Components

### Listen (`sshp listen`)

`sshp listen` creates an iroh endpoint with its persistent key, accepts
connections using ALPN `sshp/1`, opens one bidirectional stream per peer, and
forwards bytes to the local SSH backend (`127.0.0.1:22` by default).

The listener does not parse SSH or create a PTY. `sshd` and OpenSSH own those
responsibilities.

### Connect (`sshp connect <node-id>`)

`sshp connect <node-id>` is an OpenSSH `ProxyCommand` stdio pipe. It creates an
ephemeral iroh endpoint, connects to `<node-id>`, and copies raw bytes between
`stdin` and `stdout`. With `--trace`, it logs connection timing and transferred
byte counts to `stderr`.

OpenSSH continues to handle:

- public-key authentication;
- host-key verification;
- PTY, terminal mode, and `RemoteCommand`;
- `-L`, `-R`, and `-D` forwarding;
- `scp`, `sftp`, `rsync`, Git, and IDE Remote-SSH sessions.

### Autoconnect (`sshp autoconnect <host> [command]`)

`sshp autoconnect <host> [command]` runs an interactive `ssh` session against a
configured host alias in `~/.ssh/config` with `ServerAliveInterval=5` and
`ServerAliveCountMax=2` (and `-t` when `command` is provided, e.g.,
`tmux new -A -s main`). When `ssh` exits with status `255` (transport
disconnect, network switch, or laptop sleep/wake), `sshp autoconnect` retries
automatically with bounded exponential backoff (`1s..=10s`, resetting to `1s`
after sessions lasting at least `10s`). When `ssh` exits with any non-`255`
status (such as `0` when the user exits or detaches `tmux`), it returns that
exit status immediately.

### Config (`sshp config <enable|disable|install|uninstall>`)

`sshp config enable` renders or atomically updates a managed `# BEGIN sshp <alias>`
... `# END sshp <alias>` block in `~/.ssh/config`. `sshp config disable`
comments out the `ProxyCommand` line inside that block while preserving other
SSH directives. `sshp config install` and `sshp config uninstall` manage the
user-level systemd service (`sshp-listen.service`) on the worker.

### Trace (`sshp trace <node-id>`)

`sshp trace` binds an ephemeral iroh endpoint, establishes an `sshp/1`
connection and bidirectional stream to `<node-id>`, and reports handshake and
stream-open latency.

## Testable core

The runtime keeps external adapters at the edges of the flow:

```text
fake session stream -> proxy core -> fake peer transport
                                      -> worker core -> local TCP backend
```

`PeerTransport` owns peer connect/accept and returns stream parts. The
production adapter uses iroh; tests use in-memory duplex streams and a local
loopback `TcpListener`.

The byte pipeline uses Tokio's `AsyncRead` and `AsyncWrite` traits. It does not
parse SSH or iroh frames.

## Security

- The endpoint ID is public and is not a credential.
- Restrict permissions (`0600`) on the listener secret key and generated
  `~/.ssh/config`.
- SSH authentication is the main user-authentication layer.
- The default backend is localhost only.
- Never log secret keys or SSH credentials.

## Out of scope

- Implementing the SSH protocol.
- A custom TOML configuration file.
- A database, web UI, or plugin framework.

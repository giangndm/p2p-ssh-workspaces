# Architecture

## Goal

Let access connect to a workspace through a stable alias, even when the
worker changes networks or is behind NAT. The worker needs no public SSH port.

## Data flow

```text
access
  sshp connect hanoi
    └─ system ssh
        └─ sshp connect --stdio hanoi
            └─ iroh QUIC / ALPN sshp/1
                └─ sshp listen
                    └─ TCP 127.0.0.1:22
                        └─ sshd
                            └─ startup command
```

An iroh endpoint ID is derived from its public key. The listener must persist
its secret key so the ID remains stable. Access stores this ID in its config;
the worker's IP is not its identity. Iroh can use discovery, relays, and
direct connectivity to select a working route.

References: [iroh endpoints](https://docs.iroh.computer/concepts/endpoints),
[iroh QUIC](https://docs.iroh.computer/protocols/using-quic).

## Components

### Listen

`sshp listen` creates an iroh endpoint, accepts connections using ALPN
`sshp/1`, opens one bidirectional stream, and forwards data to the local SSH
backend. The MVP default is `127.0.0.1:22`.

The listener does not parse SSH or create a PTY. `sshd` and OpenSSH own those
responsibilities.

### Connect

`sshp connect <alias>` reads the host config, builds an OpenSSH command with a
ProxyCommand, and supervises the SSH process. The internal ProxyCommand
connects to iroh and copies raw bytes between stdin and stdout.

OpenSSH continues to handle:

- public-key authentication;
- host-key verification;
- PTY and terminal mode;
- `-L`, `-R`, and `-D` forwarding;
- `scp` and `sftp` when configured with the matching ProxyCommand.

### Startup command

The startup command is an argv-like list. `sshp` has no integration for any
specific program:

```toml
startup = ["tmux", "new-session", "-A", "-s", "hanoi"]
startup = ["hx", "."]
startup = ["herdr", "attach", "workspace-name"]
```

An empty list opens the login shell. Reconnect runs the command again. Programs
with attach support, such as tmux or Herdr, can restore the old workspace; a
direct editor such as Helix starts again.

## Testable core

The runtime keeps external adapters at the edges of the flow:

```text
fake session stream -> proxy core -> fake peer transport
                                      -> worker core -> fake backend stream
```

`PeerTransport` owns peer connect/accept and returns stream parts. The
production adapter uses iroh; tests use in-memory duplex streams. `SessionRunner`
owns one OpenSSH process attempt and its exit code. The production adapter
launches OpenSSH; tests return scripted results.

The byte pipeline uses Tokio's `AsyncRead` and `AsyncWrite` traits. It does not
parse SSH or iroh frames. Tests can cover forwarding, half-close, disconnect,
reconnect, and retry limits without starting SSH or iroh.

Real iroh and OpenSSH tests remain small adapter tests. They verify integration
with external tools, not the application flow.

## Security

- The endpoint ID is public and is not a credential.
- Restrict permissions on the listener secret key.
- SSH authentication is the main user-authentication layer.
- The default backend is localhost only.
- Never log secret keys, private keys, or command lines that may contain
  credentials.
- Bound stream counts and control-data size if a control protocol is added
  later.

## Out of scope for the MVP

- Implementing the SSH protocol.
- A database or account service.
- A web UI.
- A plugin or launcher framework.
- A custom relay or discovery service.
- Resuming an editor process without a session manager.

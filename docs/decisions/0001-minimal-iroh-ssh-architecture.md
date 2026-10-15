# ADR-0001: iroh ProxyCommand pipe with a local OpenSSH backend

- Status: Accepted
- Date: 2026-10-14

## Context

The application needs P2P SSH access across NAT and network changes while
preserving full compatibility with `ssh`, `scp`, `sftp`, `rsync`, Git, and IDE
Remote-SSH extensions.

Implementing SSH or wrapping the `ssh` client process with a custom TOML
configuration duplicates OpenSSH features that `~/.ssh/config` already
provides.

## Decision

`sshp` acts purely as an iroh P2P byte pipe between OpenSSH `ProxyCommand` on
the client and local `sshd` on the worker, with helper subcommands for
interactive session reconnection, OpenSSH config generation, and endpoint
tracing:

```text
sshp listen
sshp connect <node-id>
sshp autoconnect <host> [command]
sshp config <enable|disable|install|uninstall>
sshp trace <node-id>
```

All host aliases, users, keys, port forwarding, and `RemoteCommand` settings
live in `~/.ssh/config`.

## Consequences

### Positive

- minimal Rust codebase with no custom TOML config parser;
- works transparently with `ssh`, `scp`, `sftp`, `rsync`, Git, and IDEs;
- reuses OpenSSH authentication, PTY, forwarding, and `RemoteCommand`;
- no public SSH port required on the worker.

### Negative

- the worker requires a local `sshd`;
- OpenSSH `ProxyCommand` must invoke `sshp connect <node-id>`.

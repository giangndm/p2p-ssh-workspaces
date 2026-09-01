# ADR-0001: iroh transport with a local OpenSSH backend

- Status: Accepted for MVP
- Date: 2026-08-30

## Context

The application needs P2P workspace access, normal SSH behavior and options,
network-change reconnects, and flexible startup programs such as tmux, Helix,
or Herdr.

Implementing SSH and PTY support would add code, security risk, and duplicate
OpenSSH features.

## Decision

`sshp` uses iroh as the encrypted P2P transport and forwards a raw byte stream
to the worker's local `sshd`. Access calls the system `ssh` through
ProxyCommand.

Worker and access are two branches of one binary:

```text
sshp listen
sshp connect <alias>
```

The TOML startup command is a `Vec<String>`. Reconnect runs the command again;
session managers such as tmux or Herdr keep and reattach the workspace.

Debian and Ubuntu use a systemd user service first.

## Consequences

### Positive

- less Rust code;
- reuse SSH authentication, PTY, forwarding, and host-key behavior;
- no public SSH port;
- add startup programs through config only;
- keep the endpoint ID stable when the IP changes.

### Negative

- the worker needs a local `sshd`;
- direct Helix does not keep its process across disconnects;
- startup behavior depends on the worker executable;
- reconnect supervises OpenSSH and cannot restore a lost process without a
  multiplexer.

## Rejected alternatives

### Rust SSH implementation

Rejected for the MVP because it adds substantial code, security surface, and
PTY/channel/forwarding work.

### Generic launcher trait or plugin system

Rejected because an argv startup command supports tmux, Helix, Herdr, and
future programs without runtime abstractions.

### Separate daemon and client binaries

Rejected because one binary with two modes simplifies installation and service
management.

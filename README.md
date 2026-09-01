# ssh-workspaces

`ssh-workspaces` (`sshp`) is a Rust CLI for reaching a workspace over
an iroh peer-to-peer connection while retaining the normal OpenSSH experience.

The initial design has two modes:

```bash
sshp listen
sshp connect <alias>
```

The listener forwards an iroh stream to a local `sshd`. The connector launches
the system `ssh` client, so normal SSH authentication, PTY handling, and port
forwarding remain available. A configured startup command can launch tmux,
Helix, Herdr, or another program.

The listen, connect, stdio proxy, startup command, bounded reconnect, and CLI
process smoke tests are implemented. Network integration remains environment-
gated because iroh may require host network-monitor permissions.

## Project rules

All documentation, code comments, and future Git commit messages must use
concise English. Design priorities are explicit: KISS first, SRP second, and
DRY third. See the [coding rules](rules/coding.md) and [documentation
rules](rules/documentation.md).

## Documentation

- [Architecture](docs/architecture.md)
- [CLI contract](docs/cli.md)
- [Configuration](docs/configuration.md)
- [Activity flows](docs/flows.md)
- [Rust design](docs/rust-design.md)
- [Skeleton plan and test contracts](docs/skeleton-plan.md)
- [Debian/Ubuntu systemd setup](docs/deployment-systemd.md)
- [Development workflow](docs/development.md)
- [Architecture decision record](docs/decisions/0001-minimal-iroh-ssh-architecture.md)
- [Agent instructions](AGENTS.md)

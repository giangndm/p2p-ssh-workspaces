# ssh-workspaces

`ssh-workspaces` (`sshp`) is a minimal Rust CLI that pipes standard OpenSSH
connections over an iroh peer-to-peer stream.

It exposes four subcommands:

```bash
sshp listen
sshp connect <node-id>
sshp config <enable|disable|install|uninstall>
sshp trace <node-id>
```

The listener forwards an incoming `sshp/1` iroh stream to a local `sshd`
(`127.0.0.1:22` by default). On the client machine, `sshp config enable` and
`sshp config disable` manage `Host <alias>` blocks in `~/.ssh/config` with
`ProxyCommand`, allowing standard `ssh`, `scp`, `sftp`, `rsync`, Git, and IDE
Remote-SSH workflows to connect transparently over iroh without any custom
client config file. On the worker, `sshp config install` and
`sshp config uninstall` manage the systemd user service.

## Installation

Install the latest release binary (`~/.local/bin/sshp` by default) on macOS
(`arm64`) or Linux (`x86_64`):

```bash
curl -fsSL https://raw.githubusercontent.com/giangndm/p2p-ssh-workspaces/master/install.sh | bash
```

To customize the install directory or version:

```bash
curl -fsSL https://raw.githubusercontent.com/giangndm/p2p-ssh-workspaces/master/install.sh | SSHP_INSTALL_DIR=/usr/local/bin bash
```

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

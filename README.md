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

### One-line install (macOS & Linux)

Run the install script with `bash`. It automatically detects your OS and
architecture (`macOS arm64` -> `aarch64-apple-darwin`, `Linux x86_64` ->
`x86_64-unknown-linux-musl`), downloads the latest release binary from GitHub
Releases, verifies its SHA-256 checksum, and installs `sshp` to
`~/.local/bin/sshp`:

```bash
curl -fsSL https://raw.githubusercontent.com/giangndm/ssh-over-iroh/master/install.sh | bash
```

Optional environment variables:

- `SSHP_INSTALL_DIR`: custom install directory (default: `~/.local/bin`).
- `SSHP_VERSION`: specific release tag such as `v0.1.0` (default: `latest`).

```bash
curl -fsSL https://raw.githubusercontent.com/giangndm/ssh-over-iroh/master/install.sh | SSHP_INSTALL_DIR=/usr/local/bin bash
```

## Quick start

### 1. On the remote worker (server / workstation running `sshd`)

Print the worker's persistent endpoint ID and start listening (or install the
background systemd user service on Linux):

```bash
sshp listen --print-id
sshp listen
# Or install and start the systemd user service on Debian/Ubuntu:
sshp config install
```

### 2. On the client machine (macOS / Linux)

Register the worker endpoint in `~/.ssh/config` under a host alias (for example
`hanoi`) and connect with standard OpenSSH tools:

```bash
sshp config enable hanoi <worker-node-id> --user <ssh-user>
ssh hanoi
scp file.txt hanoi:~/
```

Other helpful commands:

```bash
# Probe P2P connectivity and measure handshake latency:
sshp trace <worker-node-id>

# Temporarily disable the iroh ProxyCommand for a host while keeping User/Port/IdentityFile:
sshp config disable hanoi
```

### 3. Optional: Self-hosted relay server with Docker

By default, `sshp` uses the public Number 0 relay servers. To run your own
relay server with Docker (`n0computer/iroh-relay`):

```bash
# Start a plain HTTP relay on port 3340 (LAN / VPN / behind a TLS reverse proxy):
docker run -d --name iroh-relay --restart unless-stopped -p 3340:3340 n0computer/iroh-relay:latest --dev

# Worker: listen using the custom relay (or `sshp config install --relay http://<relay-host>:3340`):
sshp listen --relay http://<relay-host>:3340

# Client: configure ~/.ssh/config and probe via the custom relay:
sshp config enable hanoi <worker-node-id> --user <ssh-user> --relay http://<relay-host>:3340
sshp trace --relay http://<relay-host>:3340 <worker-node-id>
ssh hanoi
```

See [Configuration](docs/configuration.md#self-hosted-relay-server-with-docker-iroh-relay)
for production HTTPS + QUIC Address Discovery (`7842/udp`) setup.

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

# Configuration

`sshp` does not use a custom TOML configuration file. All client host settings
live in standard OpenSSH configuration (`~/.ssh/config`).

## Worker state (`sshp listen`)

The worker stores only its persistent 32-byte iroh secret key:

```text
~/.config/sshp/worker.key
```

When `XDG_CONFIG_HOME` is set to an absolute path, the key is stored at
`$XDG_CONFIG_HOME/sshp/worker.key`. The file is created atomically with `0600`
permissions on Unix.

The local SSH backend defaults to `127.0.0.1:22` and can be overridden on the
command line with `sshp listen --backend <host:port>`. To use a custom iroh
relay server instead of the default Number 0 relays, pass
`sshp listen --relay <url>` (or `sshp config install --relay <url>` for the
systemd user service).

## Client OpenSSH config (`~/.ssh/config`)

Generate or update a host entry with `sshp config enable`:

```bash
sshp config enable hanoi <worker-endpoint-id> --user giang
```

This writes a managed block into `~/.ssh/config` and prints a `git diff`-style
unified diff of the change:

```sshconfig
# BEGIN sshp hanoi
Host hanoi
    ProxyCommand '/usr/local/bin/sshp' connect <worker-endpoint-id>
    User giang
# END sshp hanoi
```

When `--relay <url>` is passed to `sshp config enable`, the generated
`ProxyCommand` includes `--relay '<url>'` so OpenSSH dials the worker through
that custom relay even when public DNS discovery is unavailable.

Re-running `sshp config enable hanoi ...` replaces only the
`# BEGIN sshp hanoi` ... `# END sshp hanoi` section and preserves all other
entries in `~/.ssh/config`. Use `--dry-run` to preview the block without writing
to disk.

To disable the iroh `ProxyCommand` for a host while keeping its `User`, `Port`,
and `IdentityFile` directives intact:

```bash
sshp config disable hanoi
```

## Self-hosted relay server with Docker (`iroh-relay`)

You can run your own iroh relay server using the official
[`n0computer/iroh-relay`](https://hub.docker.com/r/n0computer/iroh-relay) Docker
image.

### Option 1: Plain HTTP relay (`--dev`, LAN / VPN / reverse proxy)

For local networks, private VPNs, or behind an existing TLS reverse proxy, run
`iroh-relay` in development mode on port `3340`:

```bash
docker run -d \
  --name iroh-relay \
  --restart unless-stopped \
  -p 3340:3340 \
  n0computer/iroh-relay:latest \
  --dev
```

Verify that the relay is healthy, then point both worker and client to
`http://<relay-host>:3340`:

```bash
# On the worker:
sshp listen --relay http://<relay-host>:3340

# On the client:
sshp trace --relay http://<relay-host>:3340 <worker-endpoint-id>
sshp config enable hanoi <worker-endpoint-id> --user giang --relay http://<relay-host>:3340
ssh hanoi
```

### Option 2: Production HTTPS + QUIC Address Discovery (`config.toml`)

Create `relay.toml` on your relay host with manual TLS certificates (or Let's
Encrypt ACME) and QUIC Address Discovery (UDP `7842`):

```toml
enable_quic_addr_discovery = true
http_bind_addr = "[::]:80"

[tls]
https_bind_addr = "[::]:443"
quic_bind_addr = "[::]:7842"
cert_mode = "Manual"
manual_cert_path = "/certs/fullchain.pem"
manual_key_path = "/certs/privkey.pem"
```

Start the container with ports `80/tcp`, `443/tcp`, and `7842/udp`:

```bash
docker run -d \
  --name iroh-relay \
  --restart unless-stopped \
  -p 80:80/tcp \
  -p 443:443/tcp \
  -p 7842:7842/udp \
  -v "$(pwd)/relay.toml:/config/relay.toml:ro" \
  -v "/etc/letsencrypt/live/relay.example.com:/certs:ro" \
  n0computer/iroh-relay:latest \
  --config-path /config/relay.toml
```

### Additional OpenSSH options

Because `sshp` is a standard `ProxyCommand` pipe, you can use any native
OpenSSH directive in `~/.ssh/config` (such as `ServerAliveInterval`,
`IdentityFile`, `LocalForward`, or `RemoteCommand` for tmux/Helix/Herdr):

```sshconfig
Host hanoi
    ProxyCommand '/usr/local/bin/sshp' connect <worker-endpoint-id>
    User giang
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 10
    ServerAliveCountMax 3
    RequestTTY yes
    RemoteCommand tmux new-session -A -s hanoi
```

Once configured, standard tools work transparently:

```bash
ssh hanoi
scp file.txt hanoi:~/
sftp hanoi
rsync -avz ./dir hanoi:~/
```

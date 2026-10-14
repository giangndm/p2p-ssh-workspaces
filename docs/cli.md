# CLI contract

> Status: `listen`, `connect`, `config`, and `trace` are implemented. Full
> network integration verification remains staged.

The executable is `sshp`.

## Listen mode

```bash
sshp listen
```

Run in the foreground and log the persistent endpoint ID, peer-limit events,
and backend errors to `stderr`. By default, `sshp listen` forwards streams to
`127.0.0.1:22` and persists the worker secret key at
`~/.config/sshp/worker.key` (or `$XDG_CONFIG_HOME/sshp/worker.key`).

```bash
sshp listen --backend 127.0.0.1:22
sshp listen --trace
sshp listen --print-id
```

- `--backend <ADDR>` overrides the local TCP backend address.
- `--trace` logs accepted peer IDs and per-session byte counts to `stderr`.
- `--print-id` prints the persistent endpoint ID to `stdout` and exits.

## Connect mode (`ProxyCommand` pipe)

```bash
sshp connect <node-id>
sshp connect --trace <node-id>
```

This command is invoked by OpenSSH `ProxyCommand`. It creates an ephemeral iroh
endpoint, connects to `<node-id>` using ALPN `sshp/1`, and bridges raw bytes
between `stdin` and `stdout`.

- `stdout` is reserved strictly for the SSH byte stream.
- Diagnostics and `--trace` connection/transfer metrics are written to `stderr`
  only.

## Config mode (OpenSSH `Host` blocks and worker service)

All modifying `sshp config` subcommands (`enable`, `disable`, `install`,
`uninstall`) print a `git diff`-style unified diff of the modified file to
`stdout`.

### Enable a host

```bash
sshp config enable <alias> <node-id> [--user <USER>] [--port <PORT>] [--identity-file <PATH>] [--trace]
sshp config enable <alias> <node-id> --ssh-config /custom/ssh_config
sshp config enable <alias> <node-id> --dry-run
```

Atomically writes or updates a managed `Host <alias>` block in `~/.ssh/config`
(or `--ssh-config`) and prints a unified diff of the change:

- By default, inserts or updates the `# BEGIN sshp <alias>` ... `# END sshp <alias>`
  section in `~/.ssh/config` and prints a `git diff`-style unified diff (`---`,
  `+++`, and `+`/`-` lines).
- With `--dry-run`, prints only the rendered block to `stdout` without touching
  disk.
- `--trace` adds `--trace` to the generated `ProxyCommand`.

### Disable a host

```bash
sshp config disable <alias> [--ssh-config <PATH>]
```

Comments out the `ProxyCommand` line inside `# BEGIN sshp <alias>` ... `# END sshp <alias>`
in `~/.ssh/config` (or `--ssh-config`) while preserving other directives such as
`User`, `Port`, and `IdentityFile`, and prints a `git diff`-style unified diff of
the change.

### Install or uninstall the worker systemd service

```bash
sshp config install
sshp config uninstall
```

Manage the Debian/Ubuntu systemd user unit (`sshp-listen.service`) and print a
`git diff`-style unified diff of the installed or removed unit file.

## Trace mode (P2P health probe)

```bash
sshp trace <node-id>
```

Bind an ephemeral iroh endpoint, connect to `<node-id>` using ALPN `sshp/1`,
open and close a bidirectional stream, and print latency diagnostics to `stdout`:

```text
local_id: <local-endpoint-id>
target_id: <target-endpoint-id>
alpn: sshp/1
handshake_ms: 42
stream_open_ms: 1
status: ok
```

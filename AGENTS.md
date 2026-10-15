# AGENTS.md

## Project

`ssh-workspaces` is a small Rust CLI that exposes a local OpenSSH service over
an iroh peer-to-peer connection. The product name and executable name are
`sshp`.

The project is intentionally narrow:

- iroh is the only network transport;
- OpenSSH remains responsible for SSH protocol, authentication, PTY,
  `RemoteCommand`, and forwarding via `~/.ssh/config`;
- the worker forwards an iroh byte stream to a local SSH backend;
- the access side provides a `ProxyCommand` stdio pipe, OpenSSH `Host` config
  generation, and P2P connectivity tracing;
- users connect with standard `ssh <alias>` (as well as `scp`, `sftp`, `rsync`,
  and IDE Remote-SSH) using `~/.ssh/config`.

## Design principles

Apply these principles in this order:

1. **KISS**: choose the smallest design that satisfies the documented flow;
2. **SRP**: each module/function owns one reason to change;
3. **DRY**: share only stable concepts; do not extract abstractions merely to
   remove a few repeated lines.

Do not trade KISS for speculative extensibility. A small amount of explicit
code is preferable to a generic framework.

## Language and Git policy

All documentation, code comments, and future Git commit messages must use
concise English. Keep technical names, commands, flags, and protocol values
unchanged.

Do not merge a pull request without explicit human approval. Every pull request
must either be reviewed by a human or have explicit permission from the human
user before the agent merges it. When merging pull requests, always use
**rebase** (`gh pr merge --rebase`) or **squash** (`gh pr merge --squash`).
Never create merge commits (`--merge`).

## Required architecture

Keep one binary with five top-level subcommands:

```text
sshp listen
sshp connect <node-id>
sshp autoconnect <host> [command]
sshp config <enable|disable|install|uninstall>
sshp trace <node-id>
```

The library uses a flat single-file-per-feature layout:

```text
src/
├── lib.rs
├── main.rs
├── shared.rs
├── listen.rs
├── connect.rs
├── config.rs
└── trace.rs
```

`shared` owns validated `NodeId`, default paths, the iroh ALPN, endpoint
helpers, worker identity persistence, stream forwarding, atomic file writes, and
shared errors. `listen` owns the worker accept loop, `BackendAddress`, and local
backend bridge. `connect` owns the `ProxyCommand` stdio proxy and the
`autoconnect` reconnect supervisor loop. `config` owns OpenSSH `Host` block
`enable`/`disable` updates in `~/.ssh/config` and systemd user-service
`install`/`uninstall`. `trace` owns endpoint trace diagnostics.

Do not introduce a custom TOML config file, plugin system, database, control
plane, or SSH implementation. Private `AsyncRead`/`AsyncWrite` seams are
allowed when they make the full application flow testable without real iroh or
SSH.

## Behavioral contract

- The worker persists its iroh secret key (`~/.config/sshp/worker.key`) so its
  public endpoint ID is stable.
- The listener accepts the application ALPN `sshp/1` and forwards one
  bidirectional stream to the configured local SSH backend.
- The default backend is `127.0.0.1:22`.
- `sshp connect <node-id>` bridges `stdin` and `stdout` directly to the worker's
  iroh stream for use in OpenSSH `ProxyCommand`. Diagnostics and `--trace`
  output go to `stderr` only.
- `sshp autoconnect <host> [command]` runs an interactive `ssh` session with
  `ServerAliveInterval=5` and `ServerAliveCountMax=2` (plus `-t` when a remote
  command such as `tmux new -A -s main` is supplied) and automatically
  reconnects with bounded backoff whenever `ssh` exits with transport disconnect
  status `255`.
- `sshp config enable <alias> <node-id>` renders or atomically updates a managed
  `Host <alias>` block in `~/.ssh/config`.
- `sshp config disable <alias>` comments out `ProxyCommand` inside the managed
  `Host <alias>` block while preserving other directives (`User`, `Port`,
  `IdentityFile`).
- `sshp config install` and `sshp config uninstall` manage the systemd user
  unit (`sshp-listen.service`).
- `sshp trace <node-id>` probes iroh connectivity and reports handshake and
  stream-open latency.

## Coding rules

The detailed rules live in [rules/coding.md](rules/coding.md). They are
mandatory for implementation and review.

Important defaults:

- use explicit, small Rust modules and strong types;
- keep each file cohesive and easy to follow; split unrelated responsibilities
  before a file becomes difficult to scan or exceeds 500 lines;
- use Cargo CLI for dependency, build, test, run, format, and lint workflows;
- use `cargo add`, `cargo remove`, and `cargo update` for dependency changes;
- do not hand-edit `Cargo.lock`;
- use `Result` and contextual errors; no `unwrap` or `expect` in production;
- keep blocking process and filesystem work out of async hot paths;
- bound retries, delays, buffers, and external command input;
- do not concatenate untrusted values into shell commands;
- write configuration and service files atomically;
- prefer tests around config parsing, command construction, retry state, and
  stream forwarding;
- use the current `iroh` crate, not deprecated `iroh-net` APIs.

## Documentation rules

The detailed rules live in [rules/documentation.md](rules/documentation.md).
Update the relevant document when behavior, configuration, CLI, security, or
deployment changes. Do not document planned behavior as implemented behavior.

The design source of truth is:

- [Architecture](docs/architecture.md)
- [CLI contract](docs/cli.md)
- [Configuration](docs/configuration.md)
- [Debian/Ubuntu deployment](docs/deployment-systemd.md)
- [Development workflow](docs/development.md)
- [Activity flows](docs/flows.md)
- [Rust design](docs/rust-design.md)
- [Skeleton plan and test contracts](docs/skeleton-plan.md)
- [ADR-0001](docs/decisions/0001-minimal-iroh-ssh-architecture.md)

## Verification before handoff

After every code change, fork a subagent for a focused review. The review must
check Rust standards, project rules, correctness, and whether the code can be
made simpler without losing behavior. Do not mark a code task complete until
the review result is addressed or explicitly accepted.

Every added or changed line must have a clear reason tied to the task, design,
correctness, safety, or maintainability. Debug-only code may be temporary only
when clearly marked; remove it before handoff unless it is intentionally kept
and documented.

Run the checks relevant to the change:

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

For network changes, also test a local listener/access round trip and a
disconnect/reconnect scenario. For service changes, inspect the generated unit
and run `systemd-analyze verify` when systemd is available.

# AGENTS.md

## Project

`ssh-workspaces` is a small Rust CLI that exposes a local OpenSSH service over
an iroh peer-to-peer connection. The product name and executable name are
`sshp`.

The project is intentionally narrow:

- iroh is the only network transport;
- OpenSSH remains responsible for SSH protocol, authentication, PTY, and
  forwarding;
- the worker forwards an iroh byte stream to a local SSH backend;
- the access side launches the system `ssh` command and supervises reconnects;
- tmux, Helix, Herdr, and other startup programs are configuration values, not
  Rust integrations.

## Design principles

Apply these principles in this order:

1. **KISS**: choose the smallest design that satisfies the documented flow;
2. **SRP**: each module/function owns one reason to change;
3. **DRY**: share only stable concepts; do not extract abstractions merely to
   remove a few repeated lines.

Do not trade KISS for speculative extensibility. A small amount of explicit
code is preferable to a generic framework.

## Language policy

All documentation, code comments, and future Git commit messages must use
concise English. Keep technical names, commands, flags, and protocol values
unchanged.

## Required architecture

Keep one binary with two top-level modes:

```text
sshp listen
sshp connect <alias>
```

The library is split into these modules:

```text
src/
├── lib.rs
├── shared/
│   ├── mod.rs
│   ├── config.rs
│   ├── errors.rs
│   ├── identity.rs
│   ├── stream.rs
│   ├── transport.rs
│   ├── types.rs
│   └── tests.rs
├── listen/
│   └── systemd.rs
├── listen.rs
├── connect/
│   └── retry.rs
└── connect.rs
```

`shared` owns configuration, identity paths, the iroh ALPN, endpoint helpers,
stream forwarding, and shared errors. `listen` owns the worker accept loop and
local backend bridge; `listen/systemd.rs` owns service installation. `connect`
owns alias lookup, OpenSSH invocation, stdio proxying, and reconnect behavior.

Do not introduce a plugin system, database, control plane, or SSH
implementation. Private `AsyncRead`/`AsyncWrite` seams are allowed when they
make the full application flow testable without real iroh or SSH.

## Behavioral contract

- The worker persists its iroh secret key so its public endpoint ID is stable.
- The access config stores an endpoint ID, not a presumed public IP.
- The listener accepts the application ALPN `sshp/1` and forwards one
  bidirectional stream to the configured local SSH backend.
- The default backend is `127.0.0.1:22`.
- `sshp connect <alias>` uses the host's normal `~/.ssh/config` settings and
  injects a ProxyCommand that invokes the hidden stdio path of `connect`.
- `startup` is an argv-like list. An empty list means the normal login shell;
  `["tmux", "new-session", "-A", "-s", "name"]` is one common choice;
  `["hx", "."]` and a Herdr command are also valid.
- Reconnect reruns the configured startup command. Persistent programs such as
  tmux or Herdr can reattach; a normal editor process may start again.
- A clean SSH exit must not be treated as a network disconnect.
- Reconnect status is printed by the outer client process, not injected into
  the remote PTY.

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

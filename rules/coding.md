# Coding Rules

These rules apply to all Rust code in `ssh-workspaces`.

All code comments and future Git commit messages must use concise English.

## Cargo workflow

Use Cargo CLI as the development control plane:

```bash
cargo add <crate>
cargo remove <crate>
cargo update
cargo fmt
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- <args>
```

Use `cargo add` or `cargo remove` for dependency changes. Do not hand-edit
`Cargo.lock`; avoid hand-editing dependency entries in `Cargo.toml` unless
Cargo cannot express the required source or feature selection.

## Change rationale

Every code addition or modification must have a clear reason tied to the task,
design, correctness, safety, or maintainability. Avoid speculative helpers,
abstractions, fields, and dependencies.

Debug-only code is the exception: mark it as temporary, keep it small, and
remove it before handoff unless the task explicitly keeps it.

## Mandatory review

After code changes are complete, fork a subagent for a focused code review. Ask
it to check Rust standards, project rules, correctness, error handling,
security, tests, and simpler alternatives that preserve behavior.

Address every valid finding before handoff. Record accepted exceptions and
their reasons. A code task is incomplete until this review is done.

## Design boundaries

Use the following priority order:

1. **KISS** — prefer the smallest understandable implementation;
2. **SRP (Single Responsibility)** — give each module, struct, and function one
   reason to change;
3. **DRY** — remove duplication only when the duplicated concept is stable.

Keep the implementation direct and concrete. The application has five CLI
subcommands (`listen`, `connect`, `autoconnect`, `config`, `trace`). Avoid
speculative traits, dependency injection frameworks, plugin registries, and
generalized protocol layers.

Traits are allowed only at a real side-effect seam that needs isolated tests
(for example peer transport). Do not add a trait just to make one concrete iroh
implementation look generic.

Use the flat module boundaries below:

```text
shared   NodeId, paths, worker identity, iroh endpoint, stream bridge, atomic file writes, errors
listen   incoming iroh connections, BackendAddress, and local SSH backend bridge
connect  ProxyCommand stdio proxy and autoconnect SSH reconnect supervisor
config   OpenSSH Host block enable/disable and systemd user-service install/uninstall
trace    endpoint connectivity probe and latency report formatting
```

The access side must not implement the SSH protocol.

## Rust style

- Use Rust 2024 edition and stable Rust unless a documented requirement says
  otherwise.
- Prefer small functions with one clear responsibility.
- Keep files cohesive and normally below 500 lines.
- Use enums for finite states and newtypes for IDs or validated values.
- Prefer borrowing over cloning on data paths.
- Keep public APIs minimal; do not re-export implementation details.
- Add comments for protocol, process, or stream behavior that is not obvious
  from the code.
- Keep each top-level CLI subcommand in its own module (`listen.rs`,
  `connect.rs`, `config.rs`, `trace.rs`), sharing only common primitives in
  `shared.rs`.

## Errors and process boundaries

- Return `Result` for fallible operations.
- Use `thiserror` where a stable error type is useful.
- Never use `unwrap()` or `expect()` in production paths.
- Include the operation and relevant target in error context, but do not leak
  secret keys.
- Check external command exit status and preserve useful stderr.
- Never build a shell command by interpolating arbitrary input.

## Async and I/O

- Use Tokio async APIs for network and stream I/O.
- Do not block an async task with synchronous filesystem or process work.
- Use `spawn_blocking` only for unavoidable blocking operations.
- Forward streams with bounded behavior and propagate half-close/close errors.
- Do not create unbounded queues or read unbounded data into memory.
- Use timeouts for connection establishment.

## iroh

- Persist the listener secret key before advertising its endpoint ID.
- Keep the application ALPN in one shared constant: `sshp/1`.
- Use endpoint ID/address types from the selected `iroh` version rather than
  storing IP strings as the identity of a worker.
- Register only the protocol needed by this application.
- Do not log private keys or SSH credentials.

## Configuration and files

- Validate arguments and paths at the boundary.
- Use `Path`/`PathBuf`, not manually concatenated path strings.
- Use restrictive permissions (`0600`) for secret-key and SSH config files.
- Write SSH config and systemd unit files atomically.
- Never overwrite an existing service file without an explicit user action.

## Testing

At minimum, add tests for:

- valid and invalid `NodeId`, `HostAlias`, and `BackendAddress` values;
- safe OpenSSH `Host` block rendering and idempotent upsert;
- generated systemd unit content;
- iroh stream to local TCP backend forwarding;
- trace report formatting.

Use unit tests for pure config/stream/formatting logic and integration tests for
CLI execution or local sockets. Do not make unit tests depend on a user's SSH
config, network, or systemd.

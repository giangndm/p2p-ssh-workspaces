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

Keep the implementation direct and concrete. The application has exactly two
user-facing workflows: listening and connecting. Avoid speculative traits,
dependency injection frameworks, plugin registries, and generalized protocol
layers.

Traits are allowed only at a real side-effect seam that needs isolated tests
(for example config loading or systemd installation). Do not add a trait just
to make one concrete iroh implementation look generic.

Use the module boundaries below:

```text
shared  configuration, IDs, iroh setup, protocol constants, errors
listen  incoming iroh connections, local SSH backend, systemd helper
connect host lookup, OpenSSH command, stdio proxy, reconnect loop
```

The access side must not implement SSH. The listen side must not know whether
the remote startup program is tmux, Helix, Herdr, or something else.

## Rust style

- Use Rust 2024 edition and stable Rust unless a documented requirement says
  otherwise.
- Prefer small functions with one clear responsibility.
- Keep files cohesive and normally below 500 lines.
- Use enums for finite states and newtypes for IDs or validated values.
- Prefer borrowing over cloning on data paths.
- Keep public APIs minimal; do not re-export implementation details.
- Add comments for protocol, retry, process, or terminal behavior that is not
  obvious from the code.
- Keep files cohesive and easy to follow. Prefer one responsibility per file;
  split modules before a file becomes difficult to scan or exceeds 500 lines.
- Keep orchestration, transport adapters, configuration, identity storage, and
  error definitions in separate modules. Do not let one file own unrelated
  workflows.

## Errors and process boundaries

- Return `Result` for fallible operations.
- Use `anyhow` for application-level context and `thiserror` where a stable
  error type is useful.
- Never use `unwrap()` or `expect()` in production paths.
- Include the operation and relevant target in error context, but do not leak
  secret keys.
- Check external command exit status and preserve useful stderr.
- Never build a shell command by interpolating arbitrary input.
- Treat configured executable names and arguments as data; validate empty
  programs and reject malformed configuration before spawning.

## Async and I/O

- Use Tokio async APIs for network and stream I/O.
- Do not block an async task with synchronous filesystem or process work.
- Use `spawn_blocking` only for unavoidable blocking operations.
- Forward streams with bounded behavior and propagate half-close/close errors.
- Do not create unbounded queues or read unbounded data into memory.
- Use timeouts for connection establishment and retry delays.
- Keep reconnect state explicit: attempt number, delay, clean exit, and
  cancellation.

## iroh

- Persist the listener secret key before advertising its endpoint ID.
- Keep the application ALPN in one shared constant: `sshp/1`.
- Use endpoint ID/address types from the selected `iroh` version rather than
  storing IP strings as the identity of a worker.
- Register only the protocol needed by this application.
- Do not log private keys, SSH credentials, or complete command lines if they
  may contain secrets.

## Configuration and files

- Parse and validate configuration at the boundary.
- Use `Path`/`PathBuf`, not manually concatenated path strings.
- Treat missing optional fields with explicit defaults.
- Use restrictive permissions for secret-key files.
- Write config and systemd unit files atomically.
- Never overwrite an existing service file without an explicit user action.
- Keep the config schema backward-compatible where practical; document
  breaking changes.

## Testing

At minimum, add tests for:

- valid and invalid TOML configuration;
- empty and non-empty `startup` commands;
- shell-safe remote command construction;
- clean exit versus reconnectable failure;
- bounded retry backoff and cancellation;
- generated systemd unit content;
- iroh stream to local TCP backend forwarding.

Use unit tests for pure command/config/retry logic and integration tests for
real local sockets or two local iroh endpoints. Do not make unit tests depend
on a user's SSH config, tmux installation, network, or systemd.

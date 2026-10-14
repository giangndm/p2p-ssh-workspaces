# Development workflow

The repository is a minimal Rust binary (`sshp`) that pipes OpenSSH connections
over iroh. Keep files cohesive and easy to follow: each top-level CLI
subcommand (`listen`, `connect`, `config`, `trace`) lives in its own module
file, sharing common primitives from `shared.rs`.

## Local prerequisites

- Stable Rust and Cargo;
- an OpenSSH client on the access machine;
- an OpenSSH server listening locally on the worker;
- systemd only for user-service installation.

## Cargo workflow

Use Cargo CLI for all Rust development and dependency changes:

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

Do not hand-edit `Cargo.lock`. Review the resulting `Cargo.toml` and lockfile
after dependency changes.

## Change and review policy

Every code addition or modification must have a clear reason tied to the task,
design, correctness, safety, or maintainability. Avoid speculative code.
Debug-only code must be marked temporary and removed before handoff unless it
is intentionally retained and documented.

After each code change, fork a subagent for review. The review must check Rust
standards, project rules, tests, and opportunities to simplify the code. Fix
valid findings before handoff and record accepted exceptions.

## Quality checks

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Test layers

### Unit tests

Unit tests must not depend on the network or external processes. Cover:

- `NodeId`, `HostAlias`, and `BackendAddress` validation;
- OpenSSH `Host` block rendering and idempotent upsert;
- worker identity persistence and permissions;
- stream forwarding and `BridgeStats` byte counts;
- trace report formatting;
- generated systemd units.

### Integration tests

Cover:

- CLI subcommand parsing and OpenSSH config file writing;
- local TCP backend round trips;
- two local iroh endpoints using `sshp/1` (environment-gated).

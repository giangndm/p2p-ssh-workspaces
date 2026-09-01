# Development workflow

The repository is a minimal Rust binary. Each documented command is implemented
in a small slice with matching tests; staged commands remain explicitly marked.
Keep files cohesive and easy to follow: separate configuration, identity,
streaming, transport, errors, and orchestration responsibilities into small
modules.

## Implementation order

1. Define the config schema and parse errors. (Implemented.)
2. Implement the Clap CLI and application errors. (Implemented.)
3. Create a shared iroh endpoint with a persistent secret key. (Implemented.)
4. Implement the listen accept loop and local TCP bridge. (Implemented.)
5. Implement the connect stdio proxy. (Implemented.)
6. Implement OpenSSH command construction and startup commands. (Implemented.)
7. Implement reconnect, fixed delay, and cancellation. (Implemented.)
8. Implement the Debian/Ubuntu systemd user-service installer. (Implemented.)
9. Add CLI process integration tests and update the README. (Implemented.)
   Network integration remains environment-gated.

## Local prerequisites

- Stable Rust and Cargo;
- an OpenSSH client on the access machine;
- an OpenSSH server listening locally on the worker;
- tmux, Helix, or Herdr only when used by `startup` config;
- systemd only for service installation.

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

- TOML parsing and defaults;
- `startup` validation;
- remote-command quoting;
- OpenSSH argument assembly;
- reconnect state and fixed delay;
- generated systemd units.

### Integration tests

Cover:

- a local TCP echo/backend round trip;
- two local iroh endpoints using `sshp/1`;
- connect, disconnect, and reconnect;
- worker-key persistence;
- optional tests gated by `ssh` and `systemd` availability.

Process integration tests use a temporary config directory. Network tests must
not use real endpoint IDs, production relays, real user SSH keys, or the
developer's config directory.

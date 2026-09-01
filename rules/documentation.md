# Documentation Rules

## Audience and language

All documentation must use concise English. Code identifiers, CLI flags,
protocol names, and systemd directives keep their original spelling.
The project-wide rule also requires concise English in code comments and future
Git commit messages.

Developer-facing explanations should assume basic Rust and Linux knowledge,
but must explain project-specific behavior such as ProxyCommand and iroh
endpoint IDs.

## Accuracy

- Clearly label features as `planned`, `MVP`, or `implemented`.
- Do not imply that a command works until it is implemented and tested.
- Keep examples executable or mark them as illustrative.
- Record assumptions and security limitations.
- Link to primary upstream documentation for iroh, OpenSSH, Rust, and systemd
  when external behavior matters.

## Structure

Prefer task-oriented sections:

1. what the feature does;
2. prerequisites;
3. configuration or command example;
4. expected behavior;
5. failure modes and troubleshooting.

Use short paragraphs, tables only for exact mappings, and Mermaid or ASCII
diagrams when they clarify ownership or connection flow.

## Examples

- Use realistic aliases such as `hanoi`.
- Show complete relevant config blocks, not fragments whose context is unclear.
- Keep secrets and real endpoint IDs out of examples.
- Explain whether a command is run on the worker or access machine.
- For `startup`, show the executable and each argument as a separate array
  item.

## Change discipline

When changing code, review these documents for required updates:

- `docs/architecture.md` for design or protocol changes;
- `docs/cli.md` for flags and exit behavior;
- `docs/configuration.md` for TOML schema;
- `docs/deployment-systemd.md` for service lifecycle;
- `docs/development.md` for build/test workflow;
- the ADR when a foundational decision changes.

Keep documentation links relative inside the repository. Prefer one source of
truth over duplicating long explanations across files.

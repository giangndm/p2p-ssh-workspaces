# CLI contract

> Status: listen, connect, and systemd user-service runtime are implemented.
> Full integration verification remains staged.

The executable is `sshp`.

## Listen mode

```bash
sshp listen
```

Run in the foreground and log the endpoint ID, peer-limit events, and backend
errors. The command reads `[listen]` from the default config; `--backend`
overrides it. The complete config is validated; if the file is absent, default
backend and identity paths are used.

```bash
sshp listen --backend 127.0.0.1:22
sshp listen --print-id
sshp listen --install-service
sshp listen --uninstall-service
```

`--install-service` and `--uninstall-service` are lifecycle actions inside
listen mode. They do not create another top-level mode.

## Connect mode

```bash
sshp connect <alias>
```

`sshp` uses the alias to read the endpoint ID, then runs the system `ssh` with
the host's OpenSSH config.

Pass SSH arguments after `--`:

```bash
sshp connect hanoi -- -L 8080:127.0.0.1:8080
sshp connect hanoi -- -i ~/.ssh/work_key
```

Pass `sshp` options before `--`:

```bash
sshp connect hanoi --no-reconnect
```

## Internal stdio path

```bash
sshp connect --stdio <alias>
```

This path is for ProxyCommand. It does not create an interactive terminal. It
reads stdin, connects to iroh, and writes data to stdout. Use stderr only for
diagnostics.

## Exit behavior

- Preserve the SSH exit code on normal completion.
- A clean SSH exit does not trigger reconnect.
- OpenSSH exit code `255` triggers retry when the host has reconnect enabled.
  OpenSSH does not distinguish authentication failures from other `255`
  failures, so those may be retried too.
- `--no-reconnect` disables retry for the current invocation.
- `Ctrl-C` stops SSH or cancels reconnect and returns control to the terminal.

Reconnect uses a one-second fixed delay with a 100 ms minimum and 16 retries,
capped at 64 by the policy and 60 seconds by the delay bound. A clean remote
exit and other non-255 exit codes are returned without retry.

# Configuration

Default file:

```text
~/.config/sshp/config.toml
```

Full example:

```toml
[listen]
backend = "127.0.0.1:22"

[host.hanoi]
node_id = "<worker-endpoint-id>"
ssh_host = "hanoi"
startup = ["tmux", "new-session", "-A", "-s", "hanoi"]
reconnect = true

[host.ide]
node_id = "<worker-endpoint-id>"
ssh_host = "ide"
startup = ["hx", "."]
reconnect = true

[host.agent]
node_id = "<worker-endpoint-id>"
ssh_host = "agent"
startup = ["herdr", "attach", "workspace-name"]
reconnect = true
```

## Listen fields

| Field | Required | Meaning |
| --- | --- | --- |
| `backend` | No | Local SSH address; default `127.0.0.1:22` |

The worker secret key is stored beside the config in the config directory by
default. Protect it with restrictive permissions.

`sshp listen` loads and validates the complete config before starting. The
`--backend` option overrides `[listen].backend` for that invocation.
If the file is absent, the default backend and identity path are used.

## Host fields

| Field | Required | Meaning |
| --- | --- | --- |
| `node_id` | Yes | Worker's iroh endpoint ID |
| `ssh_host` | No | OpenSSH host name; defaults to the alias |
| `startup` | No | Command and arguments to run after SSH login |
| `reconnect` | No | Enable reconnect; default `true` |

`startup` is a string array. The first item is the executable; remaining items
are arguments. An empty array means the login shell. The schema does not use a
single command string because arrays preserve argument boundaries.

## OpenSSH config

Keep SSH options in `~/.ssh/config`:

```sshconfig
Host hanoi
    User giang
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 10
    ServerAliveCountMax 3
```

`sshp connect` does not reparse the full OpenSSH config. OpenSSH handles these
options.

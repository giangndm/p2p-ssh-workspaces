# Configuration

`sshp` does not use a custom TOML configuration file. All client host settings
live in standard OpenSSH configuration (`~/.ssh/config`).

## Worker state (`sshp listen`)

The worker stores only its persistent 32-byte iroh secret key:

```text
~/.config/sshp/worker.key
```

When `XDG_CONFIG_HOME` is set to an absolute path, the key is stored at
`$XDG_CONFIG_HOME/sshp/worker.key`. The file is created atomically with `0600`
permissions on Unix.

The local SSH backend defaults to `127.0.0.1:22` and can be overridden on the
command line with `sshp listen --backend <host:port>`.

## Client OpenSSH config (`~/.ssh/config`)

Generate or update a host entry with `sshp config enable`:

```bash
sshp config enable hanoi <worker-endpoint-id> --user giang
```

This writes a managed block into `~/.ssh/config` and prints a `git diff`-style
unified diff of the change:

```sshconfig
# BEGIN sshp hanoi
Host hanoi
    ProxyCommand '/usr/local/bin/sshp' connect <worker-endpoint-id>
    User giang
# END sshp hanoi
```

Re-running `sshp config enable hanoi ...` replaces only the
`# BEGIN sshp hanoi` ... `# END sshp hanoi` section and preserves all other
entries in `~/.ssh/config`. Use `--dry-run` to preview the block without writing
to disk.

To disable the iroh `ProxyCommand` for a host while keeping its `User`, `Port`,
and `IdentityFile` directives intact:

```bash
sshp config disable hanoi
```

### Additional OpenSSH options

Because `sshp` is a standard `ProxyCommand` pipe, you can use any native
OpenSSH directive in `~/.ssh/config` (such as `ServerAliveInterval`,
`IdentityFile`, `LocalForward`, or `RemoteCommand` for tmux/Helix/Herdr):

```sshconfig
Host hanoi
    ProxyCommand '/usr/local/bin/sshp' connect <worker-endpoint-id>
    User giang
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 10
    ServerAliveCountMax 3
    RequestTTY yes
    RemoteCommand tmux new-session -A -s hanoi
```

Once configured, standard tools work transparently:

```bash
ssh hanoi
scp file.txt hanoi:~/
sftp hanoi
rsync -avz ./dir hanoi:~/
```

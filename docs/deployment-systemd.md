# Debian/Ubuntu systemd deployment

> Status: implemented for systemd user services. Integration verification is
> still staged.

Debian and Ubuntu are supported first through a systemd user service. The
listener runs as the user who owns the config and secret key. No public TCP
port is required.

## Install the service

On the worker:

```bash
sshp config install
```

With the default config root, this creates:

```text
~/.config/systemd/user/sshp-listen.service
```

With XDG_CONFIG_HOME set, the unit is created below the XDG_CONFIG_HOME
systemd/user directory. It then reloads the user manager, enables the unit,
and starts it.

Expected unit:

```ini
# Managed by sshp.
[Unit]
Description=sshp iroh listener
After=network-online.target
Wants=network-online.target

[Service]
Environment=XDG_CONFIG_HOME=/config/base
ExecStart=/absolute/path/sshp listen
Restart=always
RestartSec=3

[Install]
WantedBy=default.target
```

The executable path must be absolute. The user service runs as the account that
owns the config root.

## Server without a login session

To keep the user service running after logout or from boot, enable lingering:

```bash
loginctl enable-linger "$USER"
```

If the systemd user manager is unavailable, `sshp config install`
must return a clear error and must not create a fake unit.

## Inspect the service

```bash
systemctl --user status sshp-listen
journalctl --user -u sshp-listen -f
systemctl --user restart sshp-listen
```

Verify the unit with:

```bash
systemd-analyze --user verify ~/.config/systemd/user/sshp-listen.service
```

## Remove the service

```bash
sshp config uninstall
```

Removal must disable and stop the unit before deleting the file created by
`sshp`. It must not delete the config or secret key.

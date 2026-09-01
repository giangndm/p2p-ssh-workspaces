# Activity flows

This document describes implemented runtime behavior. Full network integration
tests remain staged.

## Listen startup

```mermaid
sequenceDiagram
    participant U as User/systemd
    participant L as sshp listen
    participant I as iroh Endpoint
    participant S as local sshd

    U->>L: start
    L->>L: load config and persistent secret key
    L->>I: bind with ALPN sshp/1
    I-->>L: stable EndpointId
    L->>L: log EndpointId
    loop each incoming connection
        I-->>L: accept connection
        L->>I: accept one bidirectional stream
        L->>S: connect configured backend
        L<<->>S: forward bytes until close/error
        L->>L: close stream and backend socket
    end
```

## Interactive connect

```mermaid
sequenceDiagram
    participant U as User
    participant C as sshp connect
    participant SSH as OpenSSH
    participant P as sshp connect --stdio
    participant W as worker
    participant T as tmux/Helix/Herdr

    U->>C: connect alias
    C->>C: load host config
    C->>SSH: spawn with SSH config and ProxyCommand
    SSH->>P: stdin/stdout proxy
    P->>W: iroh connect by EndpointId
    W-->>P: raw stream to local sshd
    P<<->>SSH: copy bytes
    SSH->>T: request PTY and startup command
    T-->>U: interactive terminal
```

## Disconnect and reconnect

```mermaid
stateDiagram-v2
    [*] --> Connecting
    Connecting --> Running: SSH process starts
    Connecting --> RetryWait: SSH exits 255, host/flag enabled, limit remains
    Running --> Exited: clean SSH exit
    Running --> RetryWait: SSH exits 255, host/flag enabled, limit remains
    Running --> Exited: retry limit exhausted
    RetryWait --> Connecting: fixed delay expires
    RetryWait --> Cancelled: Ctrl-C
    Exited --> [*]
    Cancelled --> [*]
```

Retry uses a fixed delay with a 100 ms minimum, a 60 second cap, and stops
after the configured retry limit (16 reconnect attempts by default, capped at
64). The outer
`sshp` process prints status messages. It must not write status text into the
remote PTY. A persistent startup command such as tmux or Herdr can attach to
its existing workspace; a direct Helix command starts again. Ctrl-C while SSH
runs or while waiting returns exit code 130.

## Systemd installation

```text
sshp listen --install-service
  -> resolve current executable and user config directory
  -> render systemd user unit
  -> write unit atomically
  -> systemctl --user daemon-reload
  -> systemctl --user enable --now sshp-listen
```

Installation is explicit. `sshp listen` alone runs the foreground listener.

## Failure boundaries

| Failure | Owner | Expected result |
| --- | --- | --- |
| Invalid TOML | config loader | fail before network/process start |
| Missing endpoint ID | config loader | actionable config error |
| iroh connect failure | connect mode | retry or return error |
| Local SSH backend unavailable | listen mode | log error and close that peer |
| SSH exit code 255 | connect mode | retry when enabled; OpenSSH does not classify the cause |
| Clean remote exit | connect mode | exit without reconnect |
| systemd unavailable | service installer | return setup error without fake success |

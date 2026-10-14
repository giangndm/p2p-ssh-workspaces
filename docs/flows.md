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
    L->>L: load persistent secret key
    L->>I: bind with ALPN sshp/1
    I-->>L: stable EndpointId
    L->>L: log EndpointId
    loop each incoming connection
        I-->>L: accept connection
        L->>I: accept one bidirectional stream
        L->>S: connect local backend
        L<<->>S: forward bytes until close/error
        L->>L: close stream and backend socket
    end
```

## OpenSSH `ProxyCommand` connect

```mermaid
sequenceDiagram
    participant U as User
    participant SSH as OpenSSH client
    participant P as sshp connect <node-id>
    participant W as sshp listen
    participant S as local sshd

    U->>SSH: ssh hanoi
    SSH->>P: spawn ProxyCommand (stdin/stdout)
    P->>W: iroh connect by EndpointId (sshp/1)
    W->>S: TCP connect 127.0.0.1:22
    P<<->>SSH: bridge raw SSH bytes
    SSH-->>U: interactive session / scp / sftp
```

## OpenSSH config update

```text
sshp config enable <alias> <node-id>
  -> validate alias, endpoint ID, and SSH options
  -> render managed Host block (# BEGIN sshp <alias> ... # END sshp <alias>)
  -> replace existing managed block or append to ~/.ssh/config
  -> write temporary file with 0600 mode and rename atomically
  -> print git-diff unified diff of the modified file

sshp config disable <alias>
  -> comment out ProxyCommand inside # BEGIN sshp <alias> ... # END sshp <alias>
  -> write temporary file with 0600 mode and rename atomically
  -> print git-diff unified diff of the modified file
```

## Endpoint trace

```text
sshp trace <node-id>
  -> bind ephemeral iroh endpoint
  -> measure QUIC handshake time to <node-id> (ALPN sshp/1)
  -> measure bidirectional stream open time
  -> close stream and print latency report to stdout
```

## Systemd installation

```text
sshp config install
  -> resolve current executable and user config directory
  -> render systemd user unit
  -> write unit atomically
  -> systemctl --user daemon-reload
  -> systemctl --user enable --now sshp-listen
  -> print git-diff unified diff of the unit file
```

Installation is explicit. `sshp listen` runs the foreground listener.

## Failure boundaries

| Failure | Owner | Expected result |
| --- | --- | --- |
| Invalid endpoint ID or alias | CLI / config validator | fail before network or file write |
| iroh connect failure | `connect` / `trace` | return descriptive error on `stderr`, exit 1 |
| Local SSH backend unavailable | `listen` | log error and close that peer stream |
| systemd unavailable | service installer | return setup error without fake success |

---
title: Reference
description: The commands of the server, its network ports and its signals.
sidebar:
  order: 7
---

```sh
paron server bootstrap [OPTIONS]
paron server run --data <DIR> --listen <HOST:PORT> [OPTIONS]
paron server users list --data <DIR>
paron server users lockdown --data <DIR> [--yes]
paron server users reset <USERNAME> --data <DIR> [--password-stdin]
```

- `bootstrap` sets up a new installation: the data directory, the administrator and, optionally,
  the system daemon and its system account.
- `run` starts the server from a data directory that `bootstrap` prepared.
- `users` shows and changes the users when no administrator can sign in. See
  [Manage users on the host](/docs/server/users-and-roles/#manage-users-on-the-host).

For the options of a command, their environment variables and their defaults, run the command with
`--help`:

```sh
paron server run --help
```

An option on the command line takes precedence over its environment variable.

## Public URLs

`--public-url` (`PARON_PUBLIC_URL`) lists the URLs that users open Paron at, separated by commas.

```sh
paron server run --data /var/lib/paron --listen 0.0.0.0:8443 \
  --public-url https://paron.example.com,https://10.0.0.5:8443
```

- A URL has `https://`, a host, and a port if the port is not 443.
- The first URL is in the links that Paron makes: the recovery link of a user and the enrollment
  token of a machine. Without `--public-url`, the first URL is
  `https://localhost:<listen port>`, and the links work only on the host of Paron.
- Paron answers an API request for another host with status 421. The web UI loads, but it
  shows an error.
- The certificate of Paron covers the host of each URL. See
  [Certificates](/docs/server/certificates/).
- A passkey works on all the URLs of the same domain. See
  [Sign-in methods](/docs/server/users-and-roles/#sign-in-methods).

## Network

The server uses one address and port for everything.

| Protocol | Use                                                                                      |
| -------- | ---------------------------------------------------------------------------------------- |
| TCP      | The web UI and its API over HTTPS, tunnels, and the checks of the certificate authority. |
| UDP      | Tunnels over QUIC. A tunnel uses TCP when UDP does not work.                             |

Two paths need no sign-in:

- `GET /health` answers 200 when the server runs.
- `GET /paron-ca.crt` gives the certificate authority of Paron.

## Logs

Paron writes its log to the log system of the daemon manager that starts it:

| Daemon manager  | Log                                                                                     |
| --------------- | --------------------------------------------------------------------------------------- |
| systemd         | The journal. Each level has its priority, so `journalctl -p err` shows only the errors. |
| launchd (macOS) | The unified log, with the daemon name as subsystem.                                     |

In all other cases, Paron writes its log to standard error, as text.

Set the level with `--log-level` or `PARON_LOG_LEVEL`: `error`, `warn`, `info`, `debug` or `trace`.
The default is `info`. If `RUST_LOG` has a value, Paron uses it and ignores the level.

## Shutdown

Paron stops on SIGINT or SIGTERM. On Windows, it stops on Ctrl+C, Ctrl+Break, the close of the
console, a system shutdown, or a stop of the service. It waits up to 15 seconds for open requests
to complete.

| Event                                           | Exit code |
| ----------------------------------------------- | --------- |
| The open requests complete                      | 0         |
| A second stop request arrives                   | 20        |
| The open requests do not complete in 15 seconds | 30        |

---
title: Reference
description: The commands of the tunnel.
sidebar:
  order: 9
---

```sh
paron tunnel bootstrap [OPTIONS]
paron tunnel enrol --data <DIR> --token <TOKEN> [--yes]
paron tunnel run --data <DIR> [OPTIONS]
```

- `bootstrap` sets up a new installation on a machine and registers it with the server.
- `enrol` registers an installation again, with a new token. It keeps the key of the machine. When
  the server or the machine changes, it asks for a confirmation. Without a terminal, add `--yes`, or
  the command stops.
- `run` connects to the server and keeps the connection open.

For the options of a command, their environment variables and their defaults, run the command with
`--help`:

```sh
paron tunnel run --help
```

An option on the command line takes precedence over its environment variable.

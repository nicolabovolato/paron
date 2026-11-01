---
title: Troubleshooting
description: What the errors of a connection test and of a terminal mean, and what to do.
sidebar:
  order: 2
---

**Test connection** connects to the machine and signs in. It runs no command on the machine, and it
saves nothing. A terminal makes the same connection and then starts a login shell.

## Errors of a connection

A connection test and a terminal that does not open show the same errors.

| Message                                       | Meaning                                                                                                                          | What to do                                                                                                                        |
| --------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| the machine could not be reached              | Paron got no answer within the **Connect timeout** of the machine. For a tunneled machine, the tunnel is possibly not connected. | Check the host, the port and the network. For a tunneled machine, check that `paron tunnel` runs and that it allows the SSH port. |
| the host key does not match the one on record | The machine presented a different host key.                                                                                      | See [Host key](/docs/server/machines/ssh-access/#host-key).                                                                       |
| the credentials were rejected                 | The machine refused the key or the password.                                                                                     | Check the user. For a generated key, check that its public key is in `authorized_keys`.                                           |
| the machine has not completed enrollment      | The tunnel is not set up.                                                                                                        | [Set up the tunnel](/docs/tunnels/set-up-a-tunnel/).                                                                              |
| the machine refused to open a terminal        | The machine accepted the sign-in, but it did not start a shell.                                                                  | Check the SSH server settings for the user, for example `PermitTTY`.                                                              |
| the machine has no ssh protocol               | The SSH settings of the machine were removed after the page loaded.                                                              | Reload the page. To use SSH again, edit the machine and add the SSH settings.                                                     |

## Multicast DNS

Paron resolves a `.local` name with the mDNS service of the host, for a machine and for the server
address in a tunnel token. `/etc/hosts` has priority.

| mDNS service       | On the host                                                                 | In a container                                                                                                                                         |
| ------------------ | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Avahi              | No configuration. Paron gets the IPv4 address.                              | Mount the daemon folder: `/run/avahi-daemon:/run/avahi-daemon:ro`                                                                                      |
| `systemd-resolved` | `MulticastDNS=yes` in `resolved.conf` and on the interface, which has IPv6. | Use `network_mode: host`. Or keep the bridge network: set `DNSStubListenerExtra=172.17.0.1` in `resolved.conf` and `dns: 172.17.0.1` on the container. |

## When a terminal closes

The shell ends when you close the page or when the connection is lost. Paron does not keep the
shell running in the background. **Reconnect** opens a new shell.

| Message                    | Meaning                                                                                       |
| -------------------------- | --------------------------------------------------------------------------------------------- |
| The terminal couldn't open | Paron could not connect or could not start the shell. The page shows one of the errors above. |
| Connection lost            | The connection ended after the shell opened.                                                  |
| Exited, Killed or Ended    | The shell ended. The page shows the exit status or the signal, if the machine sent one.       |

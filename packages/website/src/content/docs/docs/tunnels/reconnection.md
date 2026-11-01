---
title: Reconnection
description: What the tunnel does when it loses the connection to the server.
sidebar:
  order: 7
---

## After a lost connection

When the tunnel loses the connection, it continues to run. It waits 5 to 10 seconds and connects
again. After each failed attempt, the wait time increases, up to 2 minutes.

## Connection checks

The server and the tunnel each send a check every 15 seconds. After four checks with no answer,
the sender closes the connection. The tunnel then connects again.

## Open terminals

A terminal that uses the tunnel closes when the connection is lost. The page shows
**Connection lost** and a **Reconnect** button. See
[Troubleshooting](/docs/server/machines/troubleshooting/#when-a-terminal-closes).

## Troubleshooting

If a tunnel does not connect, the reason is one of these.

| Possible cause                      | What to do                                                                                                                                                                                                                                  |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The machine cannot reach the server | Check that the machine can reach the address and the port of the web UI.                                                                                                                                                                    |
| The tunnel access was revoked       | [Restore the tunnel access](/docs/tunnels/manage-a-tunnel/#restore-tunnel-access).                                                                                                                                                          |
| The key of the server changed       | Each machine refuses the server. For each machine, revoke the tunnel access, then [restore it](/docs/tunnels/manage-a-tunnel/#restore-tunnel-access). `paron tunnel enrol` shows `the server fingerprint changed` and asks you to continue. |
| The system daemon does not run      | Check the [system daemon](/docs/maintenance/system-daemon/) and its log.                                                                                                                                                                    |

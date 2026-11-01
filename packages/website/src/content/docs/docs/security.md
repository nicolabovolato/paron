---
title: Security
description: What Paron encrypts, what an attacker gets from a copy or from the server, and how to report a vulnerability.
sidebar:
  order: 6
---

## Report a vulnerability

Do not open a public issue. Use [Report a vulnerability](https://github.com/nicolabovolato/paron/security/advisories/new) on
GitHub.

## Encryption

Paron encrypts these values in the database with XChaCha20-Poly1305:

- the passwords and the private keys of the machines
- the password hashes of the users
- the secrets of the authenticator apps

The encryption key comes from `master.key` in the
[data directory](/docs/server/files/). Paron makes this file at the first
start, from 32 random bytes. Only the user of the Paron server can read it.

The database does not keep the session tokens and the recovery codes. It keeps only a keyed hash of
each one, made with a different key from `master.key`. For a passkey, it keeps only the public key.

The host, the user and the host key of a machine are not secrets, and Paron stores them without
encryption.

## A copy of the database

Without `master.key`, a copy of `paron.db` gives:

- the users, their roles and the [activity](/docs/server/activity/)
- the machines and their addresses
- the public keys

It gives no credential. The attacker cannot sign in to Paron or to a machine.

With `master.key`, a copy of `paron.db` also gives:

- the passwords and the private keys of the machines. The attacker can sign in to each machine
  that the attacker can reach.
- the password hashes of the users. The attacker can try to guess the passwords.
- the secrets of the authenticator apps. The attacker can make the codes.

A copy does not give a session in Paron. Keep `master.key` and your backups of `paron.db` in
different places.

`broker-ca.key` is the identity of the server for the tunneled machines. With a copy of it, an
attacker who controls the network or the DNS of a machine can act as the server. See
[Server impersonation](/docs/tunnels/security/#server-impersonation).

## A compromised server

An attacker who controls the Paron server can:

- read `master.key` and decrypt all the credentials
- open a terminal on each machine, as the user that Paron signs in with
- connect to each [allowed port and socket](/docs/tunnels/ports-and-sockets/) of each tunneled
  machine

The tunnel program on the machine checks each connection that comes from the server. It accepts
only the ports and the sockets in its list, and only on the machine itself. The server cannot
change this list. The private key of the tunnel stays on the machine, and the server cannot copy
it. See [Tunnel security](/docs/tunnels/security/).

The tunnel does not check what occurs in a terminal. When Paron opens a terminal, it signs in to
the machine as an SSH user. In that terminal, the attacker can do all that this SSH user can do.
For example, the attacker can connect from the machine to a port that is not in the list, or to a
different host in the network of the machine.

Thus the permissions of the SSH user set the limit, not the port list. Give Paron an SSH user that
has only the permissions that you need.

## When Paron is down

Paron does not change the SSH server of a machine. The other keys and passwords of the machine
continue to work, and direct `ssh` works when Paron is down.

A tunneled machine has no open port. To reach it without Paron, you need a different path to it,
for example a VPN.

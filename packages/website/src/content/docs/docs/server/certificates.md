---
title: Certificates
description: How Paron gets the certificate for HTTPS, and how to trust it.
sidebar:
  order: 4
---

Paron serves the web UI over HTTPS only. It gets a certificate in one of three ways.

## Certificate names

The certificate covers the host of each URL in `--public-url`. See
[Public URLs](/docs/server/reference/#public-urls).

```sh
paron server run --public-url https://paron.example.com,https://10.0.0.5:8443
```

A certificate from Paron itself always covers `localhost`, `127.0.0.1` and `::1`, plus these hosts.
A host with a wildcard is not accepted.

## Self-signed certificate authority

If you set no other option, Paron issues its own certificate.

- At the first start, Paron creates a certificate authority, "Paron Root CA", in the file
  `self-signed-ca.pem` in the data directory.
- Paron issues a server certificate from it, in the file `self-signed-certificate.pem` in the data
  directory. After a restart, Paron uses the same certificate.
- Paron issues a new server certificate automatically before the end date, and when the authority
  or the hosts in `--public-url` change.
- Browsers do not trust this authority by default, so they show a warning.

When the certificate changes, an open page of Paron cannot connect and shows "No connection to
Paron". Reload the page to see the warning of the browser.

To remove the warning, download the authority from `https://<name>/paron-ca.crt`, and add it to the
trusted authorities of your system or your browser.

:::caution
Before you trust the certificate, compare its SHA-256 fingerprint with the line in the server log. An
attacker in the network can give you a different certificate.

```text
certificate issued under self signed CA SHA256:<fingerprint>
```

:::

## Let's Encrypt

`--letsencrypt` asks Let's Encrypt for a certificate that browsers trust.

```sh
paron server run \
  --public-url https://paron.example.com \
  --acme-contact admin@example.com \
  --letsencrypt   # or --letsencrypt production
```

- Paron proves each domain with the TLS-ALPN-01 challenge. This challenge uses only port 443, so
  port 80 is not necessary. A proxy in front of Paron must pass the TLS connection through. It must
  not end the TLS connection itself.
- Paron orders one certificate for each domain. A domain gets a certificate only if it resolves to
  an address where Let's Encrypt can reach Paron on port 443.
- If Let's Encrypt cannot reach a domain, Paron logs the warning `could not obtain certificate` and
  tries again later. For that domain it uses its own certificate. The other domains are not
  affected.
- Paron cannot get a certificate for an IP address, for `localhost` or for a `.local` name. For
  those names it uses its own certificate.
- `--letsencrypt staging` uses the test service of Let's Encrypt. Browsers do not trust those
  certificates.
- `--acme-contact` is the address that gets a notice before a certificate expires.

Paron renews the certificates automatically and keeps them in `.cache/acme` in the data directory.
Until the first certificate arrives, Paron answers with its own certificate.

## Another certificate authority

`--acme-directory` uses a different authority that supports the ACME protocol.

```sh
paron server run \
  --public-url https://paron.example.com \
  --acme-directory https://acme.zerossl.com/v2/DV90
```

The same rule applies: a domain gets a certificate only if the authority can reach Paron at that
domain on port 443.

## Tunnels and the certificate

A tunneled machine does not use this certificate to identify the server. See
[How a tunnel works](/docs/tunnels/how-a-tunnel-works/#identity).

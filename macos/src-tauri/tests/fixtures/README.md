# Synthetic manual-mail TLS fixtures

`manual-localhost.p12` contains a synthetic localhost server identity and test CA.
Its intentionally public test password is `yas-mail-test-fixture`.
`manual-test-root.der` is that test CA's public certificate.

Used only by Rust unit tests against loopback servers. Tests trust the CA in an
in-memory connector; they never change system certificate trust, disable certificate
checks, contact providers, or use real credentials.

The old ten-year leaf was rejected by Apple's TLS validation. The refreshed leaf
is valid from 9 November 2026 to 9 November 2027. Before expiry, refresh these two
fixtures with `python3 scripts/refresh-test-certificates.py` (OpenSSL required).
This creates a new synthetic CA and a 365-day localhost leaf, retaining SAN and
server-auth restrictions, and removes temporary key files. Run the native TLS tests
after refreshing. No certificate is installed in a system trust store.

Never use this identity for a real server or add this CA to system trust.

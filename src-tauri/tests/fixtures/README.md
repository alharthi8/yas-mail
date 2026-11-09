# Synthetic manual-mail TLS fixtures

`manual-localhost.p12` contains a synthetic localhost server identity and test CA.
Its intentionally public test password is `yas-mail-test-fixture`.
`manual-test-root.der` is that test CA's public certificate.

These fixtures are used only by Rust unit tests against loopback servers. Tests add
the CA to an in-memory connector; they never change the Windows certificate store,
disable certificate checks, contact a provider, or use real credentials. The test
CA and leaf certificate were generated in memory and are valid from 2024 through
2034/2035 so the tests do not require freshly generated certificates.

Do not use these identities for a real server or add this CA to system trust.

# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it privately through GitHub:
on [github.com/JFLibrary/JFLib](https://github.com/JFLibrary/JFLib), open **Security**, then
**Report a vulnerability**. Describe what is affected (crate, function, version or commit), how
to reproduce it (an input, a test, a capture) and what an attacker gains.

A confirmed problem is fixed in a patch release of all of JFLib's crates (they share one version,
[docs/VERSIONING.md](docs/VERSIONING.md)). The fix is named under **Security** in the
[changelog](CHANGELOG.md) and in the affected crate's changelog, and gets a GitHub security
advisory; for the crates on crates.io also a [RustSec](https://rustsec.org) advisory. Credit goes
to the reporter unless they ask otherwise.

## Supported versions

Only the latest release gets security fixes. JFLib is before 1.0: update to the newest version.

## What is in scope

All crates under `lib/`, most of all those that handle secrets or bytes from a network or a file
someone else made:

| Crate | What it does |
| --- | --- |
| `crypto` | hashes, MACs, AEADs, X25519, Ed25519, P-256, RSA, ECDSA, Argon2id, Base64 |
| `netio` | TLS 1.3 / 1.2, X.509, HTTP/1.1, WebSocket, DNS, SSH client, SMTP |
| `netpkt` | packet and protocol parsers, a TLS and SSH core for simulators |
| `compress` | LZ4 and Zstandard decoders |
| `update`, `cas`, `datastore` | signed updates, encrypted backup repositories, stores |
| `plat` | the system's random numbers, sockets, files, the app permission rules |

## What to know before relying on it

JFLib's cryptography and protocols are its own code in Rust without `unsafe`, not OpenSSL or
another library. They have **not had an external audit**. What has been done instead:

- known-answer and round-trip tests, and random cases compared with OpenSSL through Python's
  `cryptography` (`tools/crypto_check.py`)
- Wycheproof and x509-limbo ([docs/EXTERNAL_SUITES.md](docs/EXTERNAL_SUITES.md))
- interoperability with OpenSSL, curl, Python's `ssl` and OpenSSH (`tools/tls_interop.py`,
  `tools/tls_web_check.py`, `tools/ssh_interop.py`)
- constant time: the secret-dependent operations of `crypto` and netio's key files take the same
  branches for every secret, and under valgrind's memcheck neither branch on the secret nor use
  it as an address, at five optimization levels on x86-64 (`tools/ct_check.py`); not checked:
  other processors, instructions whose time depends on their operands
- netio's TLS 1.3 server against tlsfuzzer (`tools/tlsfuzzer_check.py`), its client against
  TLS-Anvil (`tools/tlsanvil_check.py`)
- mutation fuzzing (`tools/net_fuzz.py`, `apps/nettest`) and coverage-guided fuzzing with
  libFuzzer and AddressSanitizer (`tools/cov_fuzz.py`) of the parsers of untrusted bytes

Limits that are documented in the crates: no revocation checks for certificates, no DNSSEC
validation, no TLS session resumption. Each crate's README says what it does and does not do.

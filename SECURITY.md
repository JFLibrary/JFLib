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
| `crypto` | hashes (SHA-2, SHA-3, SHAKE, BLAKE), MACs, AEADs, X25519, Ed25519, P-256, P-384, RSA, ECDSA, ML-KEM, Argon2id, Base64 |
| `netio` | TLS 1.3 / 1.2, HTTP/1.1, WebSocket, DNS, SSH client, SMTP |
| `x509` | X.509 certificates and chains (netio's and netpkt's) |
| `netpkt` | packet and protocol parsers, a TLS and SSH core for simulators |
| `netchan` | an encrypted and authenticated channel over UDP and WebSocket |
| `compress` | DEFLATE, zlib, gzip, Brotli, LZ4 and Zstandard decoders |
| `formats` | JSON, XML (every XML reader of the library), text encodings |
| `update`, `cas`, `datastore` | signed updates, encrypted backup repositories, stores |
| `plat` | the system's random numbers, sockets, files, the app permission rules, and the data of other programs: clipboard, drops, the Wayland and X11 wire, D-Bus |

`plat` holds most of the library's `unsafe` code, because it calls the operating systems.
[docs/PLAT_INPUTS.md](docs/PLAT_INPUTS.md) lists every way another program's data reaches it,
how each is bounded, where its `unsafe` is, what tests it and what is still open.

## What to know before relying on it

JFLib's cryptography and protocols are its own code in Rust without `unsafe`, not OpenSSL or
another library. They have **not had an external audit**. What has been done instead:

- known-answer and round-trip tests, and random cases compared with OpenSSL through Python's
  `cryptography` (`tools/crypto_check.py`)
- Wycheproof, NIST's ACVP vectors and x509-limbo ([docs/EXTERNAL_SUITES.md](docs/EXTERNAL_SUITES.md))
- interoperability with OpenSSL, curl, Python's `ssl` and OpenSSH (`tools/tls_interop.py`,
  `tools/tls_web_check.py`, `tools/ssh_interop.py`)
- constant time: the secret-dependent operations of `crypto` and netio's key files take the same
  branches for every secret, and under valgrind's memcheck neither branch on the secret nor use
  it as an address, at five optimization levels on x86-64 and ARM64; the wasm32 code of `crypto`'s
  operations (netio's key files are not built for wasm) executes the same instructions with the
  same memory addresses for every secret (`tools/ct_check.py`, `--wasm`;
  the workflow `.github/workflows/ct-check.yml` runs all three, on an x86-64 and an ARM64 machine);
  not checked: the machine code a browser's JIT makes, instructions whose time depends on their
  operands
- netio's TLS 1.3 server against tlsfuzzer (`tools/tlsfuzzer_check.py`, with RSA, ECDSA and
  Ed25519 certificates) and TLS-Anvil, its client against TLS-Anvil (`tools/tlsanvil_check.py`);
  OCSP, certificate requests and stapling against OpenSSL (`tools/tls_interop.py`); the ACME
  client against Pebble, Let's Encrypt's ACME test server (`tools/pebble_check.py`), and against
  an ACME server in coretest that validates every challenge and plays a misbehaving CA
- mutation fuzzing (`tools/net_fuzz.py`, `apps/nettest`, `plattest --test wirefuzz`) and
  coverage-guided fuzzing with libFuzzer and AddressSanitizer (`tools/cov_fuzz.py`) of the parsers
  of untrusted bytes: compress, formats, X.509, OCSP, certificate requests, DNS, HTTP, TLS, netpkt, netchan and plat's
  decoders of other programs' data

Limits that are documented in the crates: revocation only by stapled OCSP responses (a TLS client
fetches no OCSP and no CRLs itself; without a staple it takes a certificate unless the certificate
must be stapled or the program requires one), no DNSSEC validation, no TLS 1.2 session resumption; TLS 1.3 0-RTT data can be replayed within its
freshness window to servers that do not share their replay check, which is why a server takes it
only when it sets `ServerConfig::early_data` and gets it apart from the rest (`read_early`). Each
crate's README says what it does and does not do.

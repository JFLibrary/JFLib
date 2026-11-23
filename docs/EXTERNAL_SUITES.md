# External test suites -- Wycheproof, ACVP, x509-limbo, tlsfuzzer, TLS-Anvil

Four adversarial suites that found bugs in OpenSSL, Go, Java, BouncyCastle, GnuTLS, NSS and
others, and NIST's conformance vectors (ACVP, which FIPS validations are tested with), run
against JFLib's own crypto (`lib/crypto`), certificate code (`lib/x509`, the crate jflib-x509)
and TLS (`lib/netio/tls`). None is in git: each tool fetches its pinned commit on the first run,
Wycheproof, ACVP and limbo into `build/vectors/` (`tools/suite_fetch.py`, read as JSON only),
tlsfuzzer into `build/tlsfuzzer/`, TLS-Anvil with its Java and Maven into `build/tlsanvil/`.

```
python build_rs.py app coretest --target linux
python tools/wycheproof_check.py      # C2SP/wycheproof @ 65ca494727a5: 79 files, 24 MB
python tools/acvp_check.py            # usnistgov/ACVP-Server @ 975de31eb83d: 11 files, 15 MB
python tools/limbo_check.py           # C2SP/x509-limbo @ b12423782a88: limbo.json, 42 MB
python tools/tlsfuzzer_check.py       # tlsfuzzer @ 51aad6e9ae42, tlslite-ng @ 7e95ea1a86c0: netio's TLS 1.3 server
python tools/tlsanvil_check.py        # TLS-Anvil @ c1762f4 (Temurin 21, Maven 3.9.9; 850 MB): netio's TLS client
python tools/tlsanvil_check.py --server   # ... netio's TLS 1.3 server (RSA certificate)
```

Wycheproof, ACVP and limbo turn each case into a `coretest --vectors` line (the operations are
in `apps/coretest/src/suitevec.rs` and `crypto.rs`) and exit with 1 on a failure. Options: `--only` / `--id` to
filter, `--acceptable` (Wycheproof), `--list` and `--show ID` (limbo), `--target win`.
tlsfuzzer and TLS-Anvil talk to `coretest` over TCP (sections below).

## Results (2026-11-23; Linux x86-64)

| Suite | Cases | Pass | Fail | Skipped | Time |
| --- | ---: | ---: | ---: | ---: | ---: |
| Wycheproof | 18 813 | 18 532 | 0 | 281 | 15 s |
| ACVP | 13 199 | 3 608 | 0 | 9 591 | 2 s |
| x509-limbo | 9 807 | 9 720 | 28 (+14 optional) | 45 | 10 s |
| tlsfuzzer | 57 scripts | 42 scripts (2 770 probes, 540 failing as expected) | 0 | 15 scripts | 58 s |
| TLS-Anvil (client) | 232 tests | 192 | 2 (TLS-Anvil's own) | 38 | 85 s |
| TLS-Anvil (server) | 302 tests | 110 | 0 | 192 | 10 min |

Not one primitive in `lib/crypto` failed. The 15 Wycheproof failures of the first run were all
in netio's DER decoding of ECDSA signatures and are fixed (below). The limbo failures that changed what netio accepts (17 in
the first run) are fixed too; what is left are RFC 5280 / CA/B profile checks that netio does
not do, two deliberate differences and two resource limits (below).

Negative control (first run): with every expected result inverted, all non-acceptable cases flip (Wycheproof
10 133 fail, limbo 9 690 fail), so the harnesses do detect differences.

## Wycheproof

Each case goes through the API JFLib's programs call:

| Family | Path |
| --- | --- |
| X25519 (incl. low-order points) | `x25519::shared` |
| Ed25519 (strict) | `x509::Cert::verify` -> `ed25519::verify` |
| ECDSA P-256 / P-384, DER (SHA-2 and SHA-3) | `x509::Cert::verify` (netio decodes Ecdsa-Sig-Value) -> `ecdsa::verify`; and `ecdsa::p1363_from_der` -> `verify_p1363`, which must give the same answer |
| ECDSA P-256 / P-384, P1363 | `ecdsa::verify_p1363` (a signature of another length is refused there) |
| ECDH P-256 / P-384 (raw points) | `p256::shared` / `p384::shared` |
| AES-GCM | `gcm::AesGcm` seal and open (12-byte nonce, 16-byte tag, AES-128 / 192 / 256) |
| ChaCha20-Poly1305, XChaCha20-Poly1305 | `aead::seal` / `open`, `x_seal` / `x_open` |
| RSA PKCS #1 v1.5 (SHA-224, SHA-2, SHA-3) | `x509::Cert::verify` -> `rsa::PublicKey::verify_pkcs1`; signing: `rsa::PrivateKey::sign_pkcs1` |
| RSA-PSS | `x509::Cert::verify` -> `verify_pss` with the group's salt length |
| HKDF, HMAC (SHA-1 / 224 / 256 / 384 / 512, SHA3-224 / 256 / 384 / 512) | `hmac::hkdf_extract` / `hkdf_expand`, `Hmac<H>` |
| ML-KEM-512 / 768 / 1024 | `mlkem::keygen` from the seed, `encaps` (with FIPS 203's modulus check), `decaps` (the expanded key's hash check, implicit rejection) |

### Fixed: ECDSA DER decoding in netio (the 15 failures of the first run)

`netio::x509::Cert::verify` decodes the signature of certificates and of TLS
CertificateVerify. It accepted two non-DER encodings of a valid signature, in all five DER
files; it now refuses bytes after the SEQUENCE and in it after s, and takes r and s only in the
shortest form of a non-negative INTEGER (`der_unsigned`, now in `lib/x509/lib.rs`). The first
run's lines (in `lib/netio/x509.rs`, where the code was then):

| Wycheproof comment | tcId | Responsible |
| --- | --- | --- |
| appending unused 0's to sequence [r, s] | 25 (P-384/SHA-512: 26) | `x509.rs:486`: `Der::new(sig).seq()?` never checks that nothing follows the SEQUENCE |
| prepending 0's to r | 84 (P-384/SHA-512: 85) | `x509.rs:489` checks only the sign bit of r and s; a non-minimal INTEGER (`00` before a byte < 0x80) passes, and `unsigned()` (`x509.rs:348`) strips any number of zeros |
| prepending 0's to s | 128 (P-384/SHA-256: 127) | the same |

What this meant: a signature could be changed without the key (malleability), but nothing could be
forged. A certificate's signature value is not covered by its signature, so anyone could re-encode
a CA-signed certificate into copies that still validated but had other fingerprints. Pinning by
fingerprint then failed closed. `crypto::ecdsa::verify` itself is correct: the P1363 files fed
straight into it pass.

### Acceptable cases (either answer passes)

| Family | Count | lib/crypto |
| --- | ---: | --- |
| X25519 low-order points, all-zero secret | 31 | refused (`shared` -> None) |
| X25519 twist / non-canonical public keys | 223 | accepted, with the expected secret |
| Ed25519 small- / mixed-order public keys | 29 | accepted |
| RSA PKCS #1 "MissingNull" (DigestInfo without NULL) | 12 | accepted (`rsa.rs:73` tries both forms on purpose) |
| RSA sig_gen: 1024- / 1536-bit moduli, e = 3 | 46 | signed and verified (`rsa.rs:35` accepts 1024..8192 bits) |

The 12 invalid HKDF cases ask for more than 255 x HashLen bytes. `hkdf_expand` asserts that
limit (`hmac.rs:87`, documented: it panics), and the coretest operation checks it before the
call. These cases pass the documented contract; the API has no error return for them.

### Skipped (281): inputs the API cannot take

| Count | Reason |
| ---: | --- |
| 119 | AES-GCM nonces other than 12 bytes (0 to 257 bytes): the API takes `[u8; 12]` |
| 18 | ChaCha20 / XChaCha20 nonces of the other length (`[u8; 12]` / `[u8; 24]`) |
| 2 | ECDH with a compressed public point: `shared` takes 04 \|\| x \|\| y |
| 16 | RSA sig_gen with SHA-1: `HashAlg` has no SHA-1 |
| 126 | rsa_pss_misc with SHA-1, or an MGF1 hash other than the message hash |

Until 2026-11-23 there were 453: AES-192 (103), P1363 signatures of another length (80, no
decoder then) and SHA-224 / SHA-3 (172) were skipped as well; Phase 2a of the security plan
added them to the API, and they all pass.

Every tag in the AEAD files is 16 bytes, so none was skipped for its tag size.

## ACVP

NIST's Automated Cryptographic Validation Protocol vectors (`gen-val/json-files`, the
`internalProjection.json` of each algorithm: inputs and the expected answers):

| Family | Path | Run |
| --- | --- | --- |
| SHA2-224, SHA3-224 / 256 / 384 / 512 | `sha224`, `sha3_*` | AFT and the standard Monte Carlo test (100 x 1000 hashes, in coretest) |
| SHAKE-128 / 256 | `shake128` / `shake256` | AFT and VOT |
| AES-ECB 128 / 192 / 256 | `aes::Aes` both ways | GFSBox, KeySBox, VarTxt, VarKey, MMT and the Monte Carlo test (AESAVS 6.4.1) |
| ML-KEM-512 / 768 / 1024 | `mlkem::keygen`, `encaps`, `decaps` | keyGen, encapsulation, decapsulation (valid and modified ciphertexts), both key checks |
| DetECDSA P-256 / P-384 | `ecdsa::sign` with no extra bytes (RFC 6979) | SHA2-224 to SHA2-512, SHA3-224 to SHA3-512 |

Skipped (9 591): bit-oriented messages and outputs (7 105: the API hashes whole bytes), the
gigabyte messages of the LDT (20), SHAKE's Monte Carlo test (2), and of DetECDSA the other
curves (2 200), SHA2-512/224 and /256 (88) and the groups with SP 800-106's randomized hashing
of the message (176, which OpenSSL "fails" the same way: they are not plain ECDSA).

## x509-limbo

Each testcase goes to `verify_chain` the way the TLS client does it. The leaf and the
intermediates are parsed with `Cert::parse`; if one of them cannot be read, the chain is
refused, as `tls/conn.rs` does. Trust anchors that cannot be read are left out, as
`roots::system_anchors` does. Then the harness calls `verify_chain(chain, anchors,
validation_time, expected_peer_name, Server | Client)`, and `verify_chain` checks the name with
`Cert::names`.

| Group | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| bettertls | 9 572 | 0 | 0 |
| rfc5280 | 86 | 12 | 11 |
| webpki | 29 | 27 | 0 |
| pathlen, pathological, cve, online, invalid | 33 | 3 | 6 |
| crl, rfc9881 | 0 | 0 | 28 |

Fail counts include the 14 optional failures.

### Fixed: the failures that changed the answer (17 in the first run)

| Cause in the first run | Testcases | Now |
| --- | --- | --- |
| Name constraints of forms netio did not process (rfc822Name, URI, otherName) were dropped; RFC 5280 4.2.1.10 says such a constraint MUST be processed or the certificate refused | rfc5280::nc::nc-permits-email-domain-rejects-subdomain, -email-subdomains-rejects-apex, -uri-host-rejects-subdomain, -uri-subdomains-rejects-apex, nc-forbids-othername | rfc822Name, URI (the host) and directoryName (a byte-wise prefix of the RDNs) are processed; a constrained form netio does not process (otherName, x400Address, ediPartyName, registeredID) refuses a certificate below that has a name of that form |
| RSA-1024 and RSA-2052 trust anchors accepted | webpki::forbidden-weak-rsa-key-in-root, -rsa-not-divisible-by-8-in-root | `Cert::verify` takes RSA keys of 2048 bits and more, in whole bytes (`rsa_key`); `crypto::rsa` keeps 1024 for simulators |
| Unknown critical extension in a trust anchor not refused | rfc5280::unknown-critical-extension-root | refused, as in leaves and intermediates |
| An EKU extension with no purposes counted as no EKU | rfc5280::eku::ee-eku-empty | `Cert::parse` refuses it (KeyPurposeId SEQUENCE SIZE (1..MAX)) |
| Malformed SAN entries dropped or not syntax-checked | rfc5280::nc::nc-permits-invalid-ip-san, nc-permits-invalid-dns-san, rfc5280::san::underscore-dns | `Cert::parse` refuses a SAN with an iPAddress of another length, a dNSName outside the preferred name syntax (`dns_name_ok`: letters, digits, inner hyphens, `*` only as the first label), an unknown form or no entry |
| Name constraints checked against the leaf's names only | rfc5280::nc::intermediate-with-san-rejected-by-intermediate-nc, -by-root-nc | they apply to the leaf and every intermediate below the CA that is not self-issued (`ca_admits`) |
| NameConstraints without subtrees taken as "no constraint" | webpki::nc::intermediate-permitted-excluded-subtrees-both-null, -both-empty-sequences | refused: one of the two must be present, and present ones must not be empty |
| Self-issued intermediates counted against pathLenConstraint | pathlen::self-issued-certs-pathlen | not counted (RFC 5280 6.1.4 (l)) |

Also: a TLS Certificate message may hold 20 certificates (it was 10), and an intermediate in it
that cannot be read is left out instead of failing the handshake (`tls/conn.rs`, `push_cert`);
only the peer's own certificate must be readable. The path search keeps its own limits (nine
deep, 64 signature checks).

### Profile checks

Done now, beyond what changed answers: name constraints only in CA certificates, keyCertSign only
with cA (both for the leaf), AKI and SKI must not be critical, a CA must not have an empty
subject.

Not done; these are CA-side MUSTs of RFC 5280 and the CA/B Baseline Requirements that do not
change which names or keys netio trusts:

| Check | Testcases | Why not |
| --- | --- | --- |
| AKI / SKI presence | rfc5280::aki::leaf-missing-aki, intermediate-missing-aki; rfc5280::ski::root-missing-ski, intermediate-missing-ski; webpki::aki::root-with-aki-* (5) | path building does not use them; browsers do not require them either |
| BasicConstraints marked critical in a root | rfc5280::root-non-critical-basic-constraints | old trusted roots have it non-critical |
| Leaf with cA true | webpki::ee-basicconstraints-ca | RFC 5280 allows it (rfc5280::ca-as-leaf expects SUCCESS); the profiles disagree |
| DNS name constraint with a leading period | rfc5280::nc::invalid-dnsname-leading-period | netio reads `.example.com` as "subdomains only" on purpose, as other verifiers do |
| Subject / SAN consistency | rfc5280::san::noncritical-with-empty-subject, webpki::san::san-critical-with-nonempty-subject | the SAN's criticality changes no name netio matches |
| CN is a copy of a SAN entry | webpki::cn::* (9) | `Cert::names` uses the SAN only, so the CN cannot make a name match |
| Malformed AIA | webpki::malformed-aia | AIA is not used (no fetching of intermediates, no OCSP) |

### Not defects

- rfc5280::ca-as-leaf and pathlen::validation-ignores-pathlen-in-leaf: the leaf is a CA
  certificate whose keyUsage has keyCertSign only, and `verify_chain` refuses it for lacking
  digitalSignature. TLS 1.3 requires that bit whenever keyUsage is present (RFC 8446 4.4.2.2).
  Limbo's testcases ask for no key usage at all, so the refusal is right for netio's use.
- pathological::nc-dos-1 and -2: 2048 constraints against 2048 names. netio finishes in under
  0.5 s and gives the RFC-correct SUCCESS. Limbo expects a refusal as a resource limit.

### Optional failures (14, pedantic features)

- Serial number zero, or longer than 20 octets.
- Four EKU checks: anyExtendedKeyUsage in the leaf, a critical EKU, a missing EKU, an EKU in the
  root.
- Public-suffix wildcards `*.co.uk` and `*.s3.amazonaws.com`: a wildcard's base only has to
  contain a dot; there is no Public Suffix List.
- P-192, DSA, RSA-1024 and RSA-2052 leaf keys: `verify_chain` does not look at the leaf's key; a
  handshake fails later, at CertificateVerify (`Cert::verify` refuses those keys).
- A non-critical NameConstraints (RFC 5280 wants it critical; the web PKI's profile allows it).
- A cross-signed root without AKI.

### Skipped (45): what netio states it lacks

| Count | Reason |
| ---: | --- |
| 22 | CRLs: revocation is not checked (`lib/x509/lib.rs` header) |
| 10 | `expected_peer_names` (a server checking a client's names): `verify_chain` has no such check |
| 6 | `max_chain_depth`: `verify_chain` has a fixed limit of its own |
| 6 | ML-DSA keys |
| 1 | Certificate policies (`lib/x509/lib.rs` header) |

The 5 testcases with directory-name constraints, skipped before netio processed them, are run
and pass.

## tlsfuzzer

`tools/tlsfuzzer_check.py` starts four `coretest --tls-serve` servers (netio's TLS 1.3 server,
HTTP behind it) with certificates of coretest's PKI: RSA-2048 (most scripts), ECDSA P-256 and
P-384 (both, chosen by the client's signature schemes), Ed25519, and RSA asking for a client
certificate it does not require (the scripts get the PKI's client key and certificate).
`coretest --tls-echo` (every line back in one record) serves test-tls13-lengths, which checks the
length of what comes back. `--list` prints what is expected of each script and why; `--only`
runs some. Until phase 2b the server had an Ed25519 certificate only, and a sitecustomize in
front of tlslite added ed25519 to the scripts' signature lists; that patch is gone.

All 42 scripts that apply give the expected answer to every probe (2 770 probes, 540 failing as
expected, two more that may). What fails as expected is a feature netio's server does not have: the CCM suites; the
groups secp521r1, x448, FFDHE and brainpool; TLS 1.2 (a ClientHello without supported_versions,
or one falling back); Ed448; a certificate with an RSASSA-PSS key (rsa_pss_pss_*); session
tickets; KeyUpdates the server starts by itself. Three more:

- a client that lists obsolete groups is not refused (netio takes a group it has from the list);
- test-tls13-no-unknown-groups' range of "unknown" groups (4356 to 4611) holds the three ML-KEM
  hybrids, which the server has: a HelloRetryRequest for the first instead of handshake_failure;
- test-tls13-finished's Finished padded to 192 KiB and 16 MiB: the server refuses it at its
  header, sends decode_error and closes while tlslite still sends; when that send fails first,
  tlsfuzzer's runner skips the expected alert (runner.py, a socket error on send) and then reports
  it as unexpected. Either answer can come, so both are taken.

Not applicable (15 scripts):

| Scripts | Reason |
| ---: | --- |
| 3 | PSK, session tickets, resumption (phase 2b, part 2) |
| 3 | FFDHE and brainpool groups |
| 2 | certificate compression (RFC 8879) |
| 2 | client certificates with an ECDSA or Ed25519 key for tlsfuzzer (the tool hands it the PKI's RSA client) |
| 1 | brainpool curves in CertificateVerify |
| 1 | ML-KEM: needs kyber-py, which the tool does not fetch yet (the hybrids are checked against OpenSSL 3.5 by `tools/tls_interop.py`) |
| 1 | ML-DSA |
| 1 | post-handshake authentication |
| 1 | for servers without TLS 1.3 |

### Fixed in netio's server

Every place where netio's server answered otherwise than RFC 8446 says is fixed (the changelog
of jflib-netio has each):

| Area | What the server did | Now |
| --- | --- | --- |
| Mandatory algorithms (9.1) | ChaCha20-Poly1305 and X25519 only | also TLS_AES_128_GCM_SHA256 and P-256, with a HelloRetryRequest for the client's first group it has |
| Middlebox compatibility (D.4) | no change_cipher_spec | the dummy change_cipher_spec to a client with a legacy session id; a second one, or one before the first ClientHello, is unexpected_message |
| Record layer (5.1, 5.2, 5.4) | an empty alert, a record of padding only, a plaintext record over 2^14 bytes or a protected one over 2^14 + 1 bytes of inner plaintext answered otherwise | unexpected_message, unexpected_message, record_overflow |
| Records between handshake fragments (5.1) | application data inside a KeyUpdate split over records was delivered (test-tls13-keyupdate); an empty application_data record between the parts of a ClientHello was skipped as 0-RTT data and the handshake waited (test-tls13-zero-length-data) | unexpected_message; 0-RTT data is skipped only after a whole ClientHello that offered early data |
| ClientHello checks (4.1.2, 4.2.8, 4.2.11) | legacy_version below 0x0303, supported_groups without key_share, pre_shared_key before other extensions taken | protocol_version, missing_extension, illegal_parameter |
| HelloRetryRequest (4.1.2, 4.2.10) | a second ClientHello with new pre_shared_key binders refused (test-tls13-0rtt-garbage); records that did not decrypt after it skipped as 0-RTT data | the binders and ticket ages may change; after the second ClientHello nothing is skipped (bad_record_mac) |
| 0-RTT (4.2.10) | early data ended the handshake | skipped (up to 64 KiB), as a server that takes none must |
| Finished, alerts (4.4.4, 6) | a Finished of the wrong length was decrypt_error; a plaintext alert during the handshake was unexpected_message | decode_error; the alert ends the connection |
| Extension bodies (4.2) | an extension whose inner list was shorter than the extension was taken, the bytes after it ignored (test-tls13-signature-algorithms, phase 2b) | decode_error |
| Empty lists (4.2.3, 4.2.7) | an empty signature_algorithms or supported_groups list was handshake_failure | decode_error |
| Message size | a ClientHello over 64 KiB (32 715 signature schemes, which RFC 8446 allows) was refused | up to 128 KiB, as OpenSSL takes |

coretest's tls suite keeps the cases that need no encryption (`server_records` in
`apps/coretest/src/tlst.rs`: a record between two parts of a ClientHello, pre_shared_key's
place); the others stay with tlsfuzzer.

## TLS-Anvil

`tools/tlsanvil_check.py` runs TLS-Anvil's client tests by default: TLS-Anvil plays the server and
starts `coretest --tls-anvil-client PORT` for each handshake (netio's web client: TLS 1.3 with the
TLS 1.2 fallback; a `Trust::Custom` that takes TLS-Anvil's made-up certificates, in the test
program only). The client writes how each connection ended to `build/tlsanvil/run/client.log`.
`--server` runs the server tests against `coretest --tls-serve --key rsa` (netio's TLS 1.3 server
with an RSA-2048 certificate; `--key p256` and the others also work, but TLS-Scanner's probes
offer no Ed25519). Strength 1 (each pair of parameters at least once), 4 tests at a time. The tool
starts TLS-Anvil with `-ignoreCache`: TLS-Anvil fails to save a scan whose peer offered ML-KEM key
shares (NotSerializableException of MlKemPublicKey), and the cache is of no use in a run folder
that starts empty.

Of TLS-Anvil's 452 test templates, 220 are for servers and some for both sides. Of the 232 client
tests, 192 succeed strictly; 38 are disabled for features the client does not offer (versions
before TLS 1.2, max_fragment_length, Encrypt-then-MAC, cipher suites and groups beyond its own,
PSK, compression, legacy RSA signatures) or that TLS-Anvil does not check in clients; 2 fail
through defects of TLS-Anvil itself (KNOWN in the script):

- ClientAuthentication.serverSendsCertificateRequestWithNonZeroPostHandshakeAuthInHandshake: the
  test sets a context in its CertificateRequest, but the change does not reach the message. All 13
  CertificateRequests of the package came with an empty context (logged in netio's client), which
  the client rightly takes; one with a context it refuses (illegal_parameter).
- Fragmentation.recordFragmentationSupported: TLS-Scanner's probe runs its handshakes with
  TLS-Attacker's default configuration, not with what the client offers, and reports "no
  fragmentation" when that handshake fails. netio's client takes records of 1, 50 and 111 bytes
  in TLS 1.3 and 1.2 (against tlslite-ng's server; coretest's tls suite: a ServerHello in records
  of one byte).

The server tests ran for the first time with phase 2b (before it the server had an Ed25519
certificate only, which TLS-Scanner's probes do not offer). Of the 302 tests for servers and for
both sides, all 110 that apply pass; 192 are disabled: 115 for TLS 1.2 and older versions, 23 for
PSK, session tickets, resumption and 0-RTT (phase 2b, part 2), 21 for cipher suites, groups and
signatures beyond netio's (FFDHE, CCM, DSA, RC4 ...), 14 for the client side of tests for both,
the rest for Encrypt-then-MAC, max_fragment_length, rules TLS-Anvil does not expect a server to
enforce and client authentication (the test server asks for no client certificate). Two answers were wrong and are fixed: the server ended a
connection the client had closed without its own close_notify (AlertProtocol.sendsCloseNotify,
RFC 8446 6.1: `netio::Server` now answers it), and one combination of
ClientHello.sendStatusRequestExtension timed out because `coretest --tls-serve` had two workers
while TLS-Anvil keeps several connections open (it has 16 now; a test program's setting).

### Fixed in netio's client

| Test | What the client did | Now |
| --- | --- | --- |
| ServerInitiatedExtensionPoints (RFC 8701 4): a GREASE extension in ServerHello or EncryptedExtensions | took it | an extension the ClientHello did not offer is unsupported_extension, one it offered in a message that does not answer it illegal_parameter (RFC 8446 4.2, RFC 5246 7.4.1.4); netpkt's TLS core the same |
| AlertProtocol.closeNotifyPostHandshake (TLS 1.2) | did not answer the server's close_notify | answers with its own (RFC 5246 7.2.1) |
| RecordLayer.zeroLengthRecord_Finished | took an empty protected handshake record | unexpected_message, protected or not (RFC 8446 5.1) |
| SupportedVersions.invalidLegacyVersion | refused a TLS 1.3 ServerHello whose legacy_version was not 0x0303 | ignores it, as RFC 8446 4.2.1 says; netpkt's TLS core the same |

Two more were the test client's: it closed without close_notify (AlertProtocol.sendsCloseNotify,
KeyUpdate.appDataUnderNewKeysSucceeds); it now ends with `shutdown()`, as a program should (RFC
8446 6.1).

### netio's server

`--server` points TLS-Anvil at `coretest --tls-serve --key rsa` (an RSA-2048 certificate; `--key
p256`, `p384` and `rsa` all work, Ed25519 does not, since TLS-Scanner's probes offer none). Its
results are above: all 110 server tests that apply pass, and tlsfuzzer tests the server with every
kind of key besides.

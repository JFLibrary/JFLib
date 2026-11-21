# External test suites -- Wycheproof and x509-limbo

Two adversarial suites that found bugs in OpenSSL, Go, Java, BouncyCastle and others, run
against JFLib's own crypto (`lib/crypto`) and certificate code (`lib/netio/x509.rs`). Neither
suite is in git: each tool downloads its pinned commit into `build/vectors/` on the first run
(`tools/suite_fetch.py`) and reads the files as JSON only.

```
python build_rs.py app coretest --target linux
python tools/wycheproof_check.py      # C2SP/wycheproof @ 65ca494727a5: 48 files, 11 MB
python tools/limbo_check.py           # C2SP/x509-limbo @ b12423782a88: limbo.json, 42 MB
```

Both turn each case into a `coretest --vectors` line (the operations are in
`apps/coretest/src/suitevec.rs`) and exit with 1 on a failure. Options: `--only` / `--id` to
filter, `--acceptable` (Wycheproof), `--list` and `--show ID` (limbo), `--target win`.

## Results (2026-11-21, Linux x86-64)

| Suite | Cases | Pass | Fail | Skipped | Time |
| --- | ---: | ---: | ---: | ---: | ---: |
| Wycheproof | 10 942 | 10 474 | 15 | 453 | 14 s |
| x509-limbo | 9 807 | 9 690 | 51 (+16 optional) | 50 | 10 s |

Not one primitive in `lib/crypto` failed. All 15 Wycheproof failures are in netio's DER
decoding of ECDSA signatures. Most of the limbo failures are RFC 5280 / CA/B profile checks
that netio does not do; a short list changes what netio accepts (below, "would change the
answer").

Negative control: with every expected result inverted, all non-acceptable cases flip (Wycheproof
10 133 fail, limbo 9 690 fail), so the harnesses do detect differences.

## Wycheproof

Each case goes through the API JFLib's programs call:

| Family | Path |
| --- | --- |
| X25519 (incl. low-order points) | `x25519::shared` |
| Ed25519 (strict) | `x509::Cert::verify` -> `ed25519::verify` |
| ECDSA P-256 / P-384, DER | `x509::Cert::verify` (netio decodes Ecdsa-Sig-Value) -> `ecdsa::verify` |
| ECDSA P-256 / P-384, P1363 | `ecdsa::verify(r, s)` on the two halves |
| AES-GCM | `gcm::AesGcm` seal and open (12-byte nonce, 16-byte tag, AES-128 / 256) |
| ChaCha20-Poly1305, XChaCha20-Poly1305 | `aead::seal` / `open`, `x_seal` / `x_open` |
| RSA PKCS #1 v1.5 | `x509::Cert::verify` -> `rsa::PublicKey::verify_pkcs1`; signing: `rsa::PrivateKey::sign_pkcs1` |
| RSA-PSS | `x509::Cert::verify` -> `verify_pss` with the group's salt length |
| HKDF, HMAC (SHA-1 / 256 / 384 / 512) | `hmac::hkdf_extract` / `hkdf_expand`, `Hmac<H>` |

### Failures (15): ECDSA DER decoding in netio

`netio::x509::Cert::verify` (`lib/netio/x509.rs:483-491`) decodes the signature of
certificates and of TLS CertificateVerify. It accepts two non-DER encodings of a valid
signature, in all five DER files:

| Wycheproof comment | tcId | Responsible |
| --- | --- | --- |
| appending unused 0's to sequence [r, s] | 25 (P-384/SHA-512: 26) | `x509.rs:486`: `Der::new(sig).seq()?` never checks that nothing follows the SEQUENCE |
| prepending 0's to r | 84 (P-384/SHA-512: 85) | `x509.rs:489` checks only the sign bit of r and s; a non-minimal INTEGER (`00` before a byte < 0x80) passes, and `unsigned()` (`x509.rs:348`) strips any number of zeros |
| prepending 0's to s | 128 (P-384/SHA-256: 127) | the same |

What this means: a signature can be changed without the key (malleability), but nothing can be
forged. A certificate's signature value is not covered by its signature, so anyone can re-encode
a CA-signed certificate into copies that still validate but have other fingerprints. Pinning by
fingerprint then fails closed. `crypto::ecdsa::verify` itself is correct: the P1363 files fed
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

### Skipped (453): inputs the API cannot take

| Count | Reason |
| ---: | --- |
| 103 | AES-192: `aes.rs:195` takes 16- and 32-byte keys only |
| 80 | AES-GCM nonces other than 12 bytes (0 to 257 bytes): the API takes `[u8; 12]` |
| 18 | ChaCha20 / XChaCha20 nonces of the other length (`[u8; 12]` / `[u8; 24]`) |
| 80 | P1363 signatures that are not 2 x n bytes: JFLib has no P1363 decoder |
| 40 | RSA sig_gen with SHA-1 / SHA-224: `HashAlg` has SHA-256 / 384 / 512 only |
| 132 | rsa_pss_misc with SHA-1 / 224, or an MGF1 hash other than the message hash |

Every tag in the AEAD files is 16 bytes, so none was skipped for its tag size.

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
| rfc5280 | 61 | 32 | 16 |
| webpki | 25 | 31 | 0 |
| pathlen, pathological, cve, online, invalid | 32 | 4 | 6 |
| crl, rfc9881 | 0 | 0 | 28 |

Fail counts include the 16 optional failures.

### Failures that change the answer

| Cause | Testcases | Responsible |
| --- | --- | --- |
| Name constraints of forms netio does not process (rfc822Name, URI, otherName) are dropped. RFC 5280 4.2.1.10 says such a constraint MUST be processed or the certificate refused | rfc5280::nc::nc-permits-email-domain-rejects-subdomain, -email-subdomains-rejects-apex, -uri-host-rejects-subdomain, -uri-subdomains-rejects-apex, nc-forbids-othername | `x509.rs:324` (constraint forms ignored), `x509.rs:441` (SAN forms ignored) |
| RSA-1024 and RSA-2052 trust anchors accepted (2 optional failures for leaves) | webpki::forbidden-weak-rsa-key-in-root, -rsa-not-divisible-by-8-in-root | `crypto/rsa.rs:35` (1024..=8192); netio has no floor of its own (`x509.rs:481`) |
| Unknown critical extension in a trust anchor not refused. It is refused in the leaf (`x509.rs:599`) and in intermediates (`x509.rs:667`) | rfc5280::unknown-critical-extension-root | `x509.rs:630-650` |
| An EKU extension with no purposes counts as no EKU, which allows every purpose | rfc5280::eku::ee-eku-empty | `x509.rs:425-429` (empty SEQUENCE accepted), `x509.rs:503` |
| Malformed SAN entries are dropped or not syntax-checked (an 8-byte iPAddress, dNSName `.example.com`, `foo_bar.example.com`) | rfc5280::nc::nc-permits-invalid-ip-san, nc-permits-invalid-dns-san, rfc5280::san::underscore-dns | `x509.rs:434` (dNSName taken as it is), `x509.rs:441` (other lengths dropped) |
| Name constraints are checked against the leaf's names only, not against the SANs of intermediates | rfc5280::nc::intermediate-with-san-rejected-by-intermediate-nc, -by-root-nc | `x509.rs:642`, `x509.rs:673` (`constraints_allow(leaf, nc)`) |
| NameConstraints without subtrees (`30 00`) are taken as "no constraint" (2 optional failures for present-but-empty subtrees, `x509.rs:583` / `:588`) | webpki::nc::intermediate-permitted-excluded-subtrees-both-null, -both-empty-sequences | `x509.rs:446-455` |
| Self-issued intermediates count against pathLenConstraint (RFC 5280 6.1.4 (l)), so a valid chain is refused | pathlen::self-issued-certs-pathlen | `x509.rs:668` (and `:638`): `depth` counts every CA |

### Profile checks netio does not do

These are CA-side MUSTs of RFC 5280 and the CA/B Baseline Requirements. They do not change
which names or keys netio trusts.

| Check | Testcases | Where |
| --- | --- | --- |
| AKI / SKI presence and criticality | rfc5280::aki::critical-aki, leaf-missing-aki, intermediate-missing-aki; rfc5280::ski::critical-ski, root-missing-ski, intermediate-missing-ski; webpki::aki::root-with-aki-* (5) | `x509.rs:456` (AKI not parsed), `x509.rs:444` (SKI read, not required) |
| BasicConstraints marked critical in a CA | rfc5280::root-non-critical-basic-constraints | `x509.rs:413` |
| Leaf with keyCertSign but cA false; leaf with cA true | rfc5280::leaf-ku-keycertsign, webpki::ee-basicconstraints-ca | `x509.rs:597-615` checks only the leaf's digitalSignature bit |
| NameConstraints in an end-entity certificate | rfc5280::nc::not-allowed-in-ee-noncritical, -critical | `x509.rs:446`: parsed on any certificate, used only from CAs |
| DNS name constraint with a leading period | rfc5280::nc::invalid-dnsname-leading-period | `x509.rs:539` reads `.example.com` as "subdomains only" on purpose |
| Subject / SAN consistency | rfc5280::san::noncritical-with-empty-subject, rfc5280::ca-empty-subject, webpki::san::san-critical-with-nonempty-subject | `x509.rs:387`, `x509.rs:430` |
| CN is a copy of a SAN entry | webpki::cn::* (9) | not checked; `Cert::names` (`x509.rs:518`) uses the SAN only, so the CN cannot make a name match |
| Malformed AIA | webpki::malformed-aia | not parsed (a non-critical extension netio does not know, `x509.rs:458`) |

### Not defects

- rfc5280::ca-as-leaf and pathlen::validation-ignores-pathlen-in-leaf: the leaf is a CA
  certificate whose keyUsage has keyCertSign only, and `x509.rs:608` refuses it for lacking
  digitalSignature. TLS 1.3 requires that bit whenever keyUsage is present (RFC 8446 4.4.2.2).
  Limbo's testcases ask for no key usage at all, so the refusal is right for netio's use.
- pathological::nc-dos-1 and -2: 2048 constraints against 2048 names. netio finishes in under
  0.5 s and gives the RFC-correct SUCCESS. Limbo expects a refusal as a resource limit.
- 20 bettertls::pathbuilding cases have more than 10 certificates. `verify_chain` gets them all
  right, but a TLS Certificate message with more than 10 is refused (`tls/conn.rs:1245`), so 8 of
  them (tc61 to tc68, expected SUCCESS) would fail in a real handshake.

### Optional failures (16, pedantic features)

- Serial number zero, or longer than 20 octets: `x509.rs:375`.
- Four EKU checks: anyExtendedKeyUsage in the leaf, a critical EKU, a missing EKU, an EKU in the
  root.
- Public-suffix wildcards `*.co.uk` and `*.s3.amazonaws.com`: `x509.rs:527` only requires the
  wildcard's base to contain a dot; there is no Public Suffix List.
- P-192 and DSA leaf keys parse as `PubKey::Other`, and `verify_chain` does not refuse a leaf
  key it cannot use. A handshake still fails later, at CertificateVerify.
- RSA-1024 and RSA-2052 leaves: `rsa.rs:35`.
- Name constraints: a non-critical NameConstraints, and present-but-empty permitted or excluded
  subtrees (`x509.rs:583` / `:588`).
- A cross-signed root without AKI.

### Skipped (50): what netio states it lacks

| Count | Reason |
| ---: | --- |
| 22 | CRLs: revocation is not checked (`x509.rs` header) |
| 10 | `expected_peer_names` (a server checking a client's names): `verify_chain` has no such check |
| 6 | `max_chain_depth`: `verify_chain` has a fixed limit of its own |
| 6 | ML-DSA keys |
| 5 | Directory-name constraints: name constraints cover DNS and IP only (`x509.rs` header) |
| 1 | Certificate policies (`x509.rs` header) |

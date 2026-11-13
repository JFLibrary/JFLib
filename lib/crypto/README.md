# jflib-crypto

Cryptographic primitives for [JFLib](https://jflib.org), in plain Rust, without the standard
library and without a line of `unsafe` code. JFLib's TLS and SSH, its encrypted backup
repositories and its updater build on them, and any program can use them directly.

## What's in it

- **Hashes:** SHA-256, SHA-384 and SHA-512, BLAKE2b (also keyed), BLAKE3 (hash, keyed hash, key
  derivation, any output length).
- **Authentication and key derivation:** HMAC, HKDF and PBKDF2 over the SHA-2 hashes (or any
  type that implements `crypto::Hash`), Poly1305.
- **Passwords:** Argon2id, with PHC strings to store and `argon2::verify` to check them.
- **Encryption:** ChaCha20-Poly1305 and XChaCha20-Poly1305, AES-128 and AES-256 with AES-GCM,
  and sealed boxes (`sealbox`): anyone with a recipient's X25519 public key can seal, only the
  secret key opens.
- **Key exchange and signatures:** X25519, Ed25519, Diffie-Hellman on P-256, RSA signatures
  (PKCS #1 v1.5 and PSS verification, PKCS #1 v1.5 signing, key generation) and ECDSA
  verification on P-256 and P-384.
- **For protocols that are defined with them:** SHA-1, MD5 and their HMACs, the AES key wrap of
  RFC 3394, AES-CMAC and the computations of EAP-PWD. SHA-1 and MD5 are broken for collisions and
  belong in nothing new.
- **Small helpers:** Base64 in both alphabets, comparison in constant time (`ct_eq`) and
  `zeroize` for secrets.

## How it is built

The crate never makes up randomness itself. Keys, nonces and salts come from the caller, which
keeps every function deterministic and testable; on Windows, Linux, macOS and iOS
`plat::random_bytes` gives them from the system's generator.

```rust
use jflib::{crypto, plat};

let key: [u8; 32] = plat::random_bytes();
let nonce: [u8; 24] = plat::random_bytes();      // 24 bytes: random nonces are safe here
let sealed = crypto::aead::x_seal(&key, &nonce, b"header", b"the secret");
let opened = crypto::aead::x_open(&key, &nonce, b"header", &sealed);   // None if anything changed
```

Work on secrets avoids branches and table lookups that depend on them: field arithmetic in fixed
limbs with masked swaps and selects, and an AES that is bitsliced instead of looking up tables, in
both directions. Ed25519 verifies strictly (canonical points, no cofactor), so a signature is
valid or not the same way everywhere. Operations on public data, such as RSA and ECDSA
verification, are allowed to take variable time; RSA signing uses a constant-time exponentiation.
RSA key generation is variable time and meant for simulators and tests.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::crypto`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-crypto/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs) on the
  crates for servers, clients and backups
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#crypto): every public
  item of `crypto`, also on [docs.rs](https://docs.rs/jflib-crypto)

## License

MIT or Apache-2.0, at your choice.

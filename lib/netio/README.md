# jflib-netio

The network protocols of [JFLib](https://jflib.org): an HTTP/1.1 server and client, TLS 1.3 (and
TLS 1.2 towards public servers), X.509 certificates, name lookups, an SSH client for running
commands on another machine, and sending mail. It sits on the sockets of JFLib's platform layer
and is plain Rust, without the standard library and without a line of `unsafe` code. The
handshakes and ciphers are its own, built on `jflib-crypto`; no OpenSSL or other TLS library is
involved. Only the question which certificate authorities to trust goes to the operating system.

## Blocking, with limits everywhere

Every call blocks, and every read has a timeout and a size limit. A peer that sends something
malformed gets an error answer or a closed connection, never a panic. A program with a window does
not have to give up its UI thread for that: `Client::request_async` and `Client::download` run the
request on the blocking pool of the platform layer's async tasks, so `ui.spawn` (or `block_on` in
a console program) awaits the answer without blocking.

## HTTP

`Server::start` serves HTTP/1.1 with one thread that accepts connections and a pool of workers,
each serving one connection at a time, with keep-alive and pipelined requests answered in order.
`ServerConfig` sets the number of workers, the largest request body, how long an idle connection
stays open and how many requests one connection may make; a message head is limited to 16 KiB and
100 header lines. A request with both `Content-Length` and `Transfer-Encoding` is refused, which
shuts the door on request smuggling. `Server::start_with` puts TLS (or any other stream) under
the server, and the identity a client proved with its certificate reaches the handler as
`Request::client_id`.

`Client` sends requests with a connect timeout, a timeout for every read and write and a limit on
the answer's size. `get_to` hands a large body over piece by piece, `download` writes it into a
file that is replaced only by a complete 2xx answer, with progress and cancellation, and
`client::Session` keeps one connection alive for many requests.

```rust
use jflib::netio::{trust, Client, Headers, Url};

// https to a public server, checked against the system's certificate authorities
let client = Client::new().with_connector(trust::system_connector(Vec::new()));
let Some(url) = Url::parse("https://example.org/status") else { return };
match client.get(&url, &Headers::new()) {
    Ok(answer) => { /* answer.status, answer.body_bytes() */ }
    Err(e) => { /* HttpError: the address, the network, TLS, a malformed or too large answer */ }
}
```

```rust
use alloc::sync::Arc;
use jflib::netio::{Response, Server, ServerConfig};
use jflib::plat::net::SocketAddr;

let Some(addr) = SocketAddr::parse("127.0.0.1:8080") else { return };
let handler = Arc::new(|req: &jflib::netio::Request| match req.path() {
    "/hello" => Response::text(200, "hello"),
    _ => Response::new(404),
});
let server = Server::start(&addr, ServerConfig::default(), handler); // NetResult<Server>
```

## TLS and certificates

Between an app's own servers and clients, `netio::tls` speaks TLS 1.3 on both sides with
ChaCha20-Poly1305, X25519 and Ed25519 certificates of the app's own certificate authority,
mutual authentication, HelloRetryRequest and key updates. A client that talks to public servers
(`Trust::System`) also offers AES-128-GCM, RSA and ECDSA certificates and the P-256 key exchange,
and falls back to TLS 1.2 with ECDHE and AEAD suites only, the extended master secret and the
TLS 1.3 downgrade check; connections between an app's own parts never fall back. Not here:
session resumption, 0-RTT, renegotiation and revocation checks.

`netio::x509` parses certificates strictly, builds and signs those of a private Ed25519 CA, and
verifies chains up to a trusted anchor, name constraints included. For https to a server with
its own CA, `trust::connector` trusts a CA the app knows or one with a pinned fingerprint, or
learns the CA on the first connection and hands it back for the user to decide, the way SSH
treats host keys. The
system's certificate authorities are those of CryptoAPI on Windows, of the Security framework on
macOS and iOS, and the distribution's CA bundle on Linux.

## Names, SSH and mail

`dns::resolve(host, port)` takes numeric addresses and `localhost` as they are. On Windows it asks
the system's resolver; elsewhere it reads the hosts file and then asks the name servers of
`/etc/resolv.conf` itself, over UDP and over TCP when an answer was truncated, and accepts an
answer only from the server it asked, with the query's id and question. A name server on an IPv6
link-local address keeps its zone (`nameserver fe80::1%eth0`). There is no DNSSEC, mDNS or cache.

`ssh::Client` runs commands on another machine: it connects with curve25519-sha256 and the strict
key exchange that closes the Terrapin attack, checks the host key against a pinned fingerprint,
logs in with a password or an Ed25519 key, and `exec` returns the exit status, stdout and stderr.
RSA and ECDSA keys, port forwarding and terminals are not part of it.

`smtp::send` delivers a message over STARTTLS, TLS or a plain connection, logs in with AUTH PLAIN
or LOGIN only over an encrypted one, checks the server's certificate like a browser and uses
nothing the server sent before the switch to TLS. Subjects in UTF-8 and HTML parts beside the
text are written as the RFCs want them.

## Platforms

Windows, Linux, macOS and iOS. In the browser and on Android JFLib's platform layer has no
sockets, so this crate is not part of `jflib` there.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::netio`, except on wasm32 and Android. All
of JFLib's crates share one version and are released together. Its connections and name lookups
go through the platform layer, so the network rules of an app's `PERMISSIONS` file apply to them:
a name the rules do not cover fails before any query is sent.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-netio/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [Network programs in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  and [app permissions](https://github.com/JFLibrary/JFLib/blob/main/docs/PERMISSIONS.md)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#netio): every public
  item of `netio`, also on [docs.rs](https://docs.rs/jflib-netio)

## License

MIT or Apache-2.0, at your choice.

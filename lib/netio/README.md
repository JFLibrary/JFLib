# jflib-netio

The network protocols of [JFLib](https://jflib.org): an HTTP/1.1 server and client, TLS 1.3 (and
TLS 1.2 towards public servers), X.509 certificates, OCSP, certificates from ACME CAs such as
Let's Encrypt, name lookups, an SSH client for running commands on another machine, and sending
mail. It sits on the sockets of JFLib's platform layer
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

A body in gzip or br is asked for and decoded by default (zstd and deflate are decoded too, and
`max_body` counts the decoded bytes, so a small bomb stays small). Everything else is switched on
by the program: `with_redirects(n)` follows 301, 302, 303, 307 and 308 (never from https to http; Authorization
and Cookie do not go to another origin; `fetch` says where the answer came from),
`with_cookies(jar)` keeps cookies in a `netpkt::cookie::Jar` (the Public Suffix List decides
which Domain a site may set), `with_pool(Pool::new())` keeps connections open, and
`with_proxy(proxy::Proxy::from_env()?)` goes through an HTTP proxy (CONNECT tunnels for https,
NO_PROXY, credentials in the proxy's URL; `http_proxy` only in lower case, against httpoxy).
`deadline_ms` limits a whole request. A connection is used again only after an answer whose
length was known and read to its end, so a server cannot slip a second answer in.

```rust
use jflib::netio::{trust, Client, Headers, Url};

// https to a public server, checked against the system's certificate authorities
let client = Client::new().with_connector(trust::system_connector(Vec::new()));
let Some(url) = Url::parse("https://example.org/status") else { return };
match client.get(&url, &Headers::new()) {
    Ok(answer) => { /* answer.status, answer.body_bytes() */ }
    Err(e) => { /* HttpError: the address, the network, TLS, a malformed or too large answer */ }
}

// redirects, cookies and kept connections
use alloc::sync::Arc;
use jflib::{netio::Pool, netpkt::cookie::Jar};
let jar = Arc::new(jflib::plat::sync::Mutex::new(Jar::new()));
let client = client.with_redirects(5).with_cookies(jar).with_pool(Pool::new());
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

`netio::tls` speaks TLS 1.3 as a server and a client with ChaCha20-Poly1305, AES-128-GCM and
AES-256-GCM; the key exchange X25519MLKEM768 (the post-quantum hybrid that browsers and
Cloudflare use; a client sends it first, with X25519 besides), X25519, P-256, P-384 and the
hybrids SecP256r1MLKEM768 and SecP384r1MLKEM1024; certificates and keys of Ed25519, ECDSA (P-256,
P-384) and RSA, so the same server serves browsers with the certificate of a public CA and an
app's own agents with certificates of its own CA. A key comes from PEM or DER as OpenSSL and the
CAs write it (`Identity::from_pem`: PKCS #8, SEC 1, PKCS #1). The server picks its certificate by
the name the client asks for (SNI) and by the signatures it takes, or by a choice of the program's
own (`ServerConfig::select`: certificates renewed while it runs), and both sides agree on the
application protocol (ALPN; https offers http/1.1). Mutual authentication works with every kind
of key, HelloRetryRequest and key updates both ways. A client that talks to public servers
(`Trust::System`, or `Trust::Custom` with a check of the program's own: a PKI of its own, pinning,
Certificate Transparency) also falls back to TLS 1.2 with ECDHE and AEAD suites only, the extended
master secret (when the server has it; without TLS 1.2 resumption and renegotiation a server
without it is safe to take) and the TLS 1.3 downgrade check; other connections never fall back.

TLS 1.3 sessions resume with tickets: a server with `ServerConfig::tickets` (`Tickets`: stateless
tickets under keys that rotate by themselves, or keys shared by several servers) sends two after
each handshake, and a client with `ClientConfig::sessions` (`Sessions`, kept per server name,
trust and client identity, each ticket used once) offers one; the handshake keeps a fresh key
exchange (psk_dhe_ke) and skips the certificates, a session lasts at most 7 days from the handshake
that authenticated it, and a resumed server keeps the client's certificate. https resumes by
itself (`trust::system_connector`, `trust::connector` with a CA). 0-RTT is the server's choice
(`ServerConfig::early_data`): `tls::connect_early` sends a request with the ClientHello, the
server takes it only fresh and the first time it sees it, and hands it apart (`read_early`),
since early data can be replayed.

OCSP stapling works both ways. A server with `ServerConfig::ocsp` (`Staples`) sends the response
of its CA's responder to a client that asks; `ocsp::refresh` fetches it and keeps it current. A
client that checks certificates against CAs asks for one by default (`ClientConfig::revocation`):
a certificate the response calls revoked is refused, and with `Revocation::Required`, or for a
certificate that must be stapled (RFC 7633), so is one without a good response. The client fetches
no OCSP and no CRLs itself. Not here at all: renegotiation, external PSKs, TLS 1.2 resumption. The
server is checked against tlsfuzzer with RSA, ECDSA and Ed25519 certificates and against OpenSSL's
client, curl and Python, the client against TLS-Anvil and OpenSSL's server, OCSP against OpenSSL's
responder
([docs/EXTERNAL_SUITES.md](https://github.com/JFLibrary/JFLib/blob/main/docs/EXTERNAL_SUITES.md)).

```rust
use std::sync::Arc;
use jflib::netio::tls::{self, ClientConfig, Identity, ServerConfig, Trust};

// a certificate chain and its key as a CA hands them out (PEM), HTTP/1.1 by ALPN
let identity = Identity::from_pem(&chain_pem, &key_pem)?; // Result<Identity, String>
let cfg = ServerConfig { alpn: vec![String::from("http/1.1")], ..ServerConfig::new(identity.clone()) };
let tls = tls::accept(Box::new(tcp), cfg.clone()); // Result<TlsStream, TlsError>

// resumption: one Tickets for all connections of the server, one Sessions for a client's
let cfg = ServerConfig { tickets: Some(Arc::new(tls::Tickets::new())), ..cfg };
let client = ClientConfig { sessions: Some(Arc::new(tls::Sessions::default())), ..ClientConfig::new(Trust::System(Vec::new()), "example.org") };

// OCSP stapling: the CA's response for the chain, kept current (again at the time it returns)
let staples = Arc::new(tls::Staples::new());
let next = jflib::netio::ocsp::refresh(&staples, &jflib::netio::Client::new(), &identity.chain, None)?;
let cfg = ServerConfig { ocsp: Some(staples.clone()), ..cfg };
```

`netio::x509` (the crate [`jflib-x509`](https://docs.rs/jflib-x509), which netpkt shares) parses
certificates strictly, builds and signs certificates of a private CA with any of these keys,
verifies chains up to a trusted anchor, name constraints included, checks and builds OCSP
responses and writes and reads certificate requests. For https to a server with
its own CA, `trust::connector` trusts a CA the app knows or one with a pinned fingerprint, or
learns the CA on the first connection and hands it back for the user to decide, the way SSH
treats host keys. The
system's certificate authorities are those of CryptoAPI on Windows, of the Security framework on
macOS and iOS, and the distribution's CA bundle on Linux.

## Certificates from Let's Encrypt (ACME)

`netio::acme` gets certificates from a CA that speaks ACME (RFC 8555): Let's Encrypt, ZeroSSL,
Google Trust Services, a company's step-ca. `Account::issue` runs the whole issuance: the order,
proof of each name, the certificate request for a key of the program's, the chain, checked to be
for that key and every name. The proof is http-01 (`Http01`, a handler in front of the program's
own on port 80), tls-alpn-01 (`TlsAlpn01`, which lets the TLS server on port 443 answer the CA
itself) or dns-01 through code of the program's for its DNS provider (wildcards need it). DNS
names, wildcards and IP addresses; renewal information (ARI), so the CA says when to renew;
external account binding for CAs that require one; profiles; revocation and key change. Every
request is signed with the account key (ES256, ES384, RS256 or EdDSA), every URL of the CA must be
https, and every wait ends after a limit. The client is checked against an ACME server in the
test program that validates each challenge for real.

```rust
use jflib::netio::acme::{self, Acme, Http01, Identifier, NewAccount, OrderOptions};
use jflib::netio::{trust, x509::PrivateKey, Client};

// account_key: a PrivateKey the program keeps (PrivateKey::to_pkcs8) for its account
let ca = Acme::new(Client::new().with_connector(trust::system_connector(Vec::new())), acme::LETS_ENCRYPT)?;
let account = ca.account(account_key, &NewAccount { terms_agreed: true, ..NewAccount::default() })?;
// the port-80 server answers the challenges: Server::start(&addr80, cfg, http01.handler(site))
let mut http01 = Http01::new();
let cert_key = PrivateKey::generate_ecdsa(jflib::crypto::ecdsa::Curve::P256, &mut |b| jflib::plat::fill_random(b));
let issued = account.issue(&[Identifier::parse("example.org")], &cert_key, &mut http01, &OrderOptions::default())?;
let identity = issued.identity(&cert_key); // for tls::ServerConfig; issued.pem for a file
```

## WebSocket

`ws::Acceptor` answers WebSocket upgrades (RFC 6455) in the handler of a `Server`: the 101 goes
out, and the connection leaves HTTP for a thread of its own, where the code given to `accept` runs
with a `ws::WebSocket` until it returns. Over `Server::start_with` the same works with TLS
(`wss:`). `ws::connect` is the client for tests and desktop tools. Both ends answer pings, run the
close handshake, send a ping after a quiet while and end a connection that stays silent. The
limits are those of `ws::Limits`: the size of a frame and of a message, frames per message,
messages and bytes a second per connection, idle and handshake timeouts, connections per
Acceptor; an Acceptor may also take only listed `Origin`s. Anything malformed ends the connection
with the close code RFC 6455 gives it. `ws::Decoder`, the frame parser, takes untrusted bytes
without I/O and keeps its memory within one frame, one read and one message.

```rust
use alloc::sync::Arc;
use jflib::netio::ws::{Acceptor, Limits, Message};
use jflib::netio::{Request, Response, Server, ServerConfig};

let ws = Arc::new(Acceptor::new(Limits::default()).origins(&["https://game.example.org"]));
let handler = Arc::new(move |req: &Request| match req.path() {
    "/echo" => ws.accept(req, |sock| {
        while let Ok(m) = sock.recv() {
            let _ = match m {
                Message::Text(t) => sock.send_text(&t),
                Message::Binary(b) => sock.send_binary(&b),
            };
        }
    }),
    _ => Response::new(404),
});
let server = Server::start(&addr, ServerConfig::default(), handler);
```

A game's channel over UDP and WebSocket, for the desktop and the browser, is
[`jflib-netchan`](https://docs.rs/jflib-netchan)'s.

## Names, SSH and mail

`dns::resolve(host, port)` takes numeric addresses and `localhost` as they are. On Windows, macOS
and iOS it asks the system's resolver (`getaddrinfo` on Apple), which knows the name servers of a
VPN, `.local` over mDNS and iOS's rules (a link-local address it returns keeps its zone); on Linux
it reads the hosts file and then asks the name servers of `/etc/resolv.conf` itself, over UDP and
over TCP when an answer was truncated, and accepts an answer only from the server it asked, with
the query's id and question (a "no such name" too) and only for the asked name or its CNAME chain,
compared label by label. A name server on an IPv6 link-local address keeps its zone
(`nameserver fe80::1%eth0`). That resolver has no DNSSEC, mDNS or cache.

`ssh::Client` runs commands on another machine: it connects with curve25519-sha256 and the strict
key exchange that closes the Terrapin attack, checks the host key against a pinned fingerprint,
logs in with a password or an Ed25519 key, and `exec` returns the exit status, stdout and stderr.
RSA and ECDSA keys, port forwarding and terminals are not part of it.

`smtp::send` delivers a message over STARTTLS, TLS or a plain connection (`Security::None`, to
any server, such as a relay on port 25 inside a network), logs in with AUTH PLAIN or LOGIN only
over an encrypted one, checks the server's certificate like a browser and uses
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

## What's new in 0.2.1

The server ends a request body that comes slower than `MIN_BODY_RATE` (1 KiB a second) after a first
`read_ms`, a TLS read ends after its timeout also when only empty records arrive, and X.509 checks
wildcard certificates against excluded name constraints and the extended key usage of the CAs on the
path. A response without a length ends only at an orderly end (`Stream::end_is_clean`), and SSH
`exec` counts `exit-signal` names against `max_out`. New: `x509::CertBuilder::sign_ext` and
`x509::extension`. Everything is in the
[changelog](https://docs.rs/crate/jflib-netio/latest/source/CHANGELOG.md).

## Documentation

- [Network programs in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  and [app permissions](https://github.com/JFLibrary/JFLib/blob/main/docs/PERMISSIONS.md)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#netio): every public
  item of `netio`, also on [docs.rs](https://docs.rs/jflib-netio)

## License

MIT or Apache-2.0, at your choice.

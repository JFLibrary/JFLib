// netio -- network protocols on top of plat::net
//
//   dns        host names to addresses (Linux: /etc/hosts, resolv.conf, UDP / TCP queries;
//              Windows: the system resolver)
//   url        Url::parse("https://host:9443/api?x=1"), percent_encode / percent_decode,
//              query_pairs
//   http       Request, Response, Body (bytes or a stream sent chunked), Headers, status texts;
//              the HTTP/1.1 wire format (bounded parsing, chunked transfer, keep-alive)
//   server     Server::start(addr, ServerConfig, handler): accept thread + worker pool
//   client     Client::request(...): blocking, timeouts, size limit
//   tls        TLS 1.3 for own deployments (ChaCha20-Poly1305, X25519, Ed25519, mutual auth)
//              and a client for public servers (AES-GCM, RSA / ECDSA, TLS 1.2 fallback):
//              connect / accept -> TlsStream (a Stream)
//   x509 / der / pem   certificates: Ed25519 of a private CA (parse, build, verify chains) and
//              the RSA / ECDSA certificates of the web PKI
//   roots      the system's certificate authorities (Windows: CryptoAPI; Linux: the bundle)
//   ssh        SSH-2 client (curve25519, ssh-ed25519, chacha20-poly1305): run commands with
//              stdin; the server side of the transport for tests
//   trust      https to a server with its own CA: known CA, pinned fingerprint or trust on
//              first use (the presented CA comes back for the user's decision); https to
//              public servers with the system's CAs
//   smtp       sending mail: STARTTLS / TLS, AUTH PLAIN / LOGIN, UTF-8 messages
//   Stream     what HTTP runs over (plat::net::TcpStream, tls::TlsStream)
//
// Everything is blocking with timeouts and size limits on every read; a malformed peer gets an
// error answer or a closed connection, never a panic.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod client;
pub mod der;
pub mod dns;
pub mod http;
pub mod pem;
pub mod roots;
pub mod server;
pub mod smtp;
pub mod ssh;
pub mod tls;
pub mod trust;
pub mod url;
pub mod x509;

pub use client::{Client, HttpError, Part};
pub use http::{Body, Headers, Request, Response};
pub use server::{Server, ServerConfig};
pub use url::Url;

use plat::net::{NetResult, TcpStream};

/// a byte stream HTTP runs over (shared between a reader and a writer: &self methods)
pub trait Stream: Send + Sync {
    /// up to buf.len() bytes; Ok(0) at the end
    fn read(&self, buf: &mut [u8]) -> NetResult<usize>;
    fn write_all(&self, buf: &[u8]) -> NetResult<()>;
    /// timeouts for the following reads / writes (None: block)
    fn set_timeouts(&self, read_ms: Option<u32>, write_ms: Option<u32>);
    /// no more writes from this side
    fn shutdown(&self) {}
    /// whether bytes (or the end) can be read within `ms`
    fn wait_readable(&self, _ms: u32) -> bool {
        true
    }
}

impl Stream for TcpStream {
    fn read(&self, buf: &mut [u8]) -> NetResult<usize> {
        TcpStream::read(self, buf)
    }
    fn write_all(&self, buf: &[u8]) -> NetResult<()> {
        TcpStream::write_all(self, buf)
    }
    fn set_timeouts(&self, read_ms: Option<u32>, write_ms: Option<u32>) {
        self.set_read_timeout(read_ms);
        self.set_write_timeout(write_ms);
    }
    fn shutdown(&self) {
        let _ = self.shutdown_write();
    }
    fn wait_readable(&self, ms: u32) -> bool {
        TcpStream::wait_readable(self, ms).unwrap_or(true)
    }
}

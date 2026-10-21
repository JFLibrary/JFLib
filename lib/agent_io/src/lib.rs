// Explicitly activated transports for the shared agent_core dispatcher. New library.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
#[cfg(any(windows, target_os = "linux"))]
pub mod ipc;

use alloc::vec::Vec;
#[cfg(not(target_arch = "wasm32"))]
use alloc::string::String;

/// Maximum bytes in one newline-delimited JSON request, before the newline.
pub const LINE_LIMIT: usize = 1_048_576;

/// One bounded line or a rejected oversized record.
pub enum Record {
    /// Complete UTF-8 candidate, validated by agent_core's JSON decoder.
    Line(Vec<u8>),
    /// Input exceeded the limit; the complete record was discarded.
    TooLong,
}

/// Incremental newline framer shared by pipes, sockets, streams and bridge clients.
#[derive(Default)]
pub struct Framer {
    pending: Vec<u8>,
    discard: bool,
}
impl Framer {
    /// Consume chunks without interpreting UTF-8 until a whole record is available.
    pub fn feed(&mut self, bytes: &[u8], receive: &mut impl FnMut(Record)) {
        for byte in bytes {
            if *byte == b'\n' {
                if self.discard { receive(Record::TooLong); }
                else {
                    if self.pending.last() == Some(&b'\r') { self.pending.pop(); }
                    receive(Record::Line(core::mem::take(&mut self.pending)));
                }
                self.discard = false;
            } else if !self.discard {
                if self.pending.len() == LINE_LIMIT { self.pending.clear(); self.discard = true; }
                else { self.pending.push(*byte); }
            }
        }
    }
    /// Finish a stream; a final nonempty record need not end in a newline.
    pub fn finish(&mut self, receive: &mut impl FnMut(Record)) {
        if self.discard { receive(Record::TooLong); }
        else if !self.pending.is_empty() { receive(Record::Line(core::mem::take(&mut self.pending))); }
        self.discard = false;
    }
}

/// Run an explicitly selected stdio session. Only protocol responses reach stdout;
/// stream failures are reported on stderr. Application startup must select this path.
#[cfg(not(target_arch = "wasm32"))]
pub fn serve_stdio(mut process: impl FnMut(&[u8]) -> Option<String>) -> i32 {
    let mut framer = Framer::default();
    let mut buffer = [0u8; 4096];
    let mut receive = |record| {
        let response = match record {
            Record::Line(bytes) if bytes.is_empty() => None,
            Record::Line(bytes) => process(&bytes),
            Record::TooLong => Some(String::from("{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32600,\"message\":\"Request exceeds 1048576 bytes\"}}\n")),
        };
        if let Some(response) = response { plat::write_stdout(response.as_bytes()); }
    };
    loop {
        match plat::read_stdin(&mut buffer) {
            Ok(0) => { framer.finish(&mut receive); return 0; }
            Ok(count) => framer.feed(&buffer[..count], &mut receive),
            Err(_) => { plat::write_stderr(b"agent transport: standard input read failed\n"); return 1; }
        }
    }
}

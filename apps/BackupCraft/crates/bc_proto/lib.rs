// bc_proto -- what BackupCraft's console, server, agent and bcctl share: product identity,
// the data model (model.rs), the REST interface (api.rs: route table, request bodies, error
// format), the JSON mapping (j.rs: record! / wire_enum!) and identifiers (ids.rs).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod j;
pub mod agent;
pub mod api;
pub mod ids;
pub mod model;

pub use j::{Field, Json, Value};

/// product name shown by every program
pub const PRODUCT: &str = "BackupCraft";
/// version of the programs and of the API they speak
pub const VERSION: &str = "0.1.0";
/// the server's default port (HTTP on loopback, HTTPS elsewhere)
pub const DEFAULT_PORT: u16 = 7642;
/// the agent's default port (mutual TLS)
pub const AGENT_PORT: u16 = 7680;
/// the Ed25519 public keys (hex) whose signed update manifests every server trusts; a server
/// adds its own with the setting update.key (bcctl release keygen makes such a key)
pub const RELEASE_KEYS: &[&str] = &[];

/// a server address as people type it ("backup-srv01", "host:7642", "https://..", "[::1]:7642")
/// -> "scheme://host:port" (http for loopback names without a scheme, else https; the
/// default port when none is given)
pub fn server_address(addr: &str) -> Option<alloc::string::String> {
    let a = addr.trim();
    let (scheme, rest) = match a.split_once("://") {
        Some((s, r)) => (Some(s.to_ascii_lowercase()), r),
        None => (None, a),
    };
    if scheme.as_deref().is_some_and(|s| s != "http" && s != "https") {
        return None;
    }
    let rest = rest.split('/').next().unwrap_or("");
    let (host, port) = if let Some(r) = rest.strip_prefix('[') {
        let (h, p) = r.split_once(']')?;
        (h, p.strip_prefix(':'))
    } else if rest.matches(':').count() == 1 {
        let (h, p) = rest.split_once(':')?;
        (h, Some(p))
    } else {
        (rest, None) // a name, IPv4, or IPv6 without brackets (no port then)
    };
    if host.is_empty() || host.contains(['@', ' ', '?', '#']) {
        return None;
    }
    let port: u16 = match port {
        Some(p) => p.parse().ok()?,
        None => DEFAULT_PORT,
    };
    let scheme = scheme.unwrap_or_else(|| alloc::string::String::from(if is_loopback(host) { "http" } else { "https" }));
    let h = if host.contains(':') { alloc::format!("[{}]", host) } else { alloc::string::String::from(host) };
    Some(alloc::format!("{}://{}:{}", scheme, h, port))
}

/// "localhost", 127.x.x.x, ::1
pub fn is_loopback(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']);
    h.eq_ignore_ascii_case("localhost") || h == "::1" || h.split('.').next() == Some("127") && h.split('.').count() == 4 && h.split('.').all(|p| p.parse::<u8>().is_ok())
}

/// "NAME VERSION" line printed by --version
pub fn version_line(program: &str) -> alloc::string::String {
    alloc::format!("{program} {VERSION} ({PRODUCT})\n")
}

/// a byte count for messages and logs (German like them, decimal units): "15,8 GB", "512 B"
pub fn size_text(b: u64) -> alloc::string::String {
    let u = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i + 1 < u.len() {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 { alloc::format!("{} B", b) } else { alloc::format!("{:.1} {}", v, u[i]).replace('.', ",") }
}

/// a count for messages and logs with German thousands separators: "15.941"
pub fn count_text(n: u64) -> alloc::string::String {
    let d = alloc::format!("{}", n);
    let mut out = alloc::string::String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

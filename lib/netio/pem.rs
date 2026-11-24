// pem -- "-----BEGIN LABEL-----" blocks (RFC 7468): each marker on a line of its own

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// a PEM block of DER bytes
pub fn encode(label: &str, der: &[u8]) -> String {
    let b = crypto::b64::encode(der);
    let mut s = format!("-----BEGIN {}-----\n", label);
    for chunk in b.as_bytes().chunks(64) {
        s.push_str(core::str::from_utf8(chunk).unwrap_or(""));
        s.push('\n');
    }
    s.push_str(&format!("-----END {}-----\n", label));
    s
}

/// every block with this label, decoded.  Read line by line: the lines of a body (a private key
/// is secret) are only compared with the markers, which no Base64 line equals (none begins with
/// '-'); no substring search runs over them, whose steps would depend on the characters
pub fn decode_all(text: &str, label: &str) -> Vec<Vec<u8>> {
    let begin = format!("-----BEGIN {}-----", label);
    let end = format!("-----END {}-----", label);
    let mut out = Vec::new();
    let mut body: Option<String> = None;
    for line in text.split('\n') {
        let line = line.trim();
        match body.as_mut() {
            None => {
                if line == begin {
                    body = Some(String::new());
                }
            }
            Some(b) if line == end => {
                if let Some(d) = crypto::b64::decode(b) {
                    out.push(d);
                }
                body = None;
            }
            Some(b) => b.extend(line.chars().filter(|c| !c.is_whitespace())),
        }
    }
    out
}

/// the first block with this label
pub fn decode(text: &str, label: &str) -> Option<Vec<u8>> {
    decode_all(text, label).into_iter().next()
}

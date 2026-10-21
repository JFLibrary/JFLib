// pem -- "-----BEGIN LABEL-----" blocks (RFC 7468)

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

/// every block with this label, decoded
pub fn decode_all(text: &str, label: &str) -> Vec<Vec<u8>> {
    let begin = format!("-----BEGIN {}-----", label);
    let end = format!("-----END {}-----", label);
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(&begin) {
        let after = &rest[i + begin.len()..];
        let Some(j) = after.find(&end) else { break };
        let body: String = after[..j].chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(d) = crypto::b64::decode(&body) {
            out.push(d);
        }
        rest = &after[j + end.len()..];
    }
    out
}

/// the first block with this label
pub fn decode(text: &str, label: &str) -> Option<Vec<u8>> {
    decode_all(text, label).into_iter().next()
}

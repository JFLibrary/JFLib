//! Bounded UTF-8 formatting for UI labels.
use alloc::string::String;

/// fmt_buf(buf, cap, ...): the text cut to cap - 1 bytes (never inside a UTF-8 sequence)
pub(crate) fn fmt_cap(s: String, cap: usize) -> String {
    if s.len() < cap {
        return s;
    }
    let mut n = cap - 1;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    String::from(&s[..n])
}

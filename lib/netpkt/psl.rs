// psl -- the Public Suffix List (publicsuffix.org): where a name stops being one that a single
// party registers ("co.uk", "github.io", "*.ck" but not "www.ck")
//
//   public_suffix(host) -> &str        "example.co.uk" -> "co.uk", "a.b.github.io" -> "github.io";
//                                      a name the list does not know: its last label (the rule *)
//   registrable_domain(host) -> Option<&str>   one label more ("example.co.uk"); None for a public
//                                      suffix itself, an address, or ""
//   is_public_suffix(name) -> bool
//   LIST_COMMIT, LIST_DATE             the list the table was made from (tools/psl_gen.py)
//
// The names are taken as they come: lower case, A-labels ("xn--..."), without a dot at the end
// (cookie::canonical_host gives that).  Both sections of the list count, ICANN and PRIVATE, as
// browsers take them for cookies.  The table (psl_data.rs, MPL-2.0) holds the rules as one
// sorted string; a lookup is a binary search per label.

use crate::IpAddr;

#[path = "psl_data.rs"]
mod data;

/// the commit of publicsuffix/list the table was generated from
pub const LIST_COMMIT: &str = data::COMMIT;
/// that commit's date
pub const LIST_DATE: &str = data::DATE;

fn rule(i: usize) -> &'static str {
    let (a, b) = (data::OFFS[i] as usize, data::OFFS[i + 1] as usize - 1);
    &data::DATA[a..b]
}

/// whether the list has this rule, as it writes it ("co.uk", "*.ck", "!www.ck")
fn has(r: &str) -> bool {
    let (mut lo, mut hi) = (0usize, data::OFFS.len() - 1);
    while lo < hi {
        let mid = (lo + hi) / 2;
        match rule(mid).cmp(r) {
            core::cmp::Ordering::Less => lo = mid + 1,
            core::cmp::Ordering::Greater => hi = mid,
            core::cmp::Ordering::Equal => return true,
        }
    }
    false
}

/// the start of the last `k` labels of a name whose labels start at `starts` (left to right)
fn last<'a>(name: &'a str, starts: &[usize], k: usize) -> &'a str {
    &name[starts[starts.len() - k]..]
}

/// how many labels of the name's right end are its public suffix (the algorithm of
/// publicsuffix.org: an exception rule wins, else the matching rule with the most labels, else *)
fn suffix_labels(name: &str) -> usize {
    let mut starts = alloc::vec![0usize];
    starts.extend(name.match_indices('.').map(|(i, _)| i + 1));
    let n = starts.len();
    let mut buf = alloc::string::String::new();
    // an exception: the suffix is the rule without its first label
    for k in (2..=n).rev() {
        buf.clear();
        buf.push('!');
        buf.push_str(last(name, &starts, k));
        if has(&buf) {
            return k - 1;
        }
    }
    for k in (1..=n).rev() {
        if has(last(name, &starts, k)) {
            return k;
        }
        if k >= 2 {
            buf.clear();
            buf.push_str("*.");
            buf.push_str(last(name, &starts, k - 1));
            if has(&buf) {
                return k;
            }
        }
    }
    1
}

fn plain(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.ends_with('.') && !name.contains("..") && IpAddr::parse(name).is_none()
}

/// the public suffix of a name (the whole of it when it is one; "" for "" or an address)
pub fn public_suffix(host: &str) -> &str {
    if !plain(host) {
        return "";
    }
    let k = suffix_labels(host);
    let mut at = host.len();
    for _ in 0..k {
        at = host[..at].rfind('.').map_or(0, |i| i);
    }
    if at == 0 { host } else { &host[at + 1..] }
}

/// the public suffix and one label more: what a party registers ("example.co.uk"); None when the
/// name is a public suffix itself, an address, or not a name
pub fn registrable_domain(host: &str) -> Option<&str> {
    let s = public_suffix(host);
    if s.is_empty() || s.len() == host.len() {
        return None;
    }
    let head = &host[..host.len() - s.len() - 1];
    let at = head.rfind('.').map_or(0, |i| i + 1);
    Some(&host[at..])
}

/// whether a name is a public suffix ("com", "co.uk", "github.io", "foo.ck"; not "www.ck")
pub fn is_public_suffix(name: &str) -> bool {
    plain(name) && public_suffix(name).len() == name.len()
}

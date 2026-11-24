// x509 -- netpkt's view of JFLib's certificates: the crate jflib-x509, which netio uses as well
// (one parser, one path validation), and what is wrong with a chain as fault bits for the
// dialogs of simulators and packet tools
//
//   Cert, CertBuilder, IpAddr, verify_chain ...   the crate x509, re-exported
//   CertSpec                                      CertBuilder under netpkt's earlier name
//   check_chain(&[leaf, ...], anchors, host, now) -> faults   what is wrong with it: F_* bits, 0
//                                                 for nothing (a browser's list, where
//                                                 verify_chain gives the first reason)

pub use x509::*;

/// the builder of a certificate (netpkt's earlier name of x509::CertBuilder)
pub type CertSpec = x509::CertBuilder;

/// the time of a certificate has passed
pub const F_EXPIRED: u32 = 1 << 0;
/// its time has not come yet
pub const F_NOT_YET: u32 = 1 << 1;
/// the name asked for is not among its names
pub const F_NAME: u32 = 1 << 2;
/// signed by itself, and not an anchor
pub const F_SELF_SIGNED: u32 = 1 << 3;
/// no path to an anchor
pub const F_UNKNOWN_ISSUER: u32 = 1 << 4;
/// a signature that does not check out, or a key this code cannot read
pub const F_BAD_SIGNATURE: u32 = 1 << 5;

/// what is wrong with a chain (leaf first) for this host at this time: F_* bits, 0 for nothing.
/// For a private Ed25519 PKI: a leaf with another kind of key counts as F_BAD_SIGNATURE
pub fn check_chain(chain: &[Cert], anchors: &[Cert], host: &str, now: i64) -> u32 {
    let Some(leaf) = chain.first() else { return F_UNKNOWN_ISSUER };
    let mut f = 0;
    if !leaf.names(host) {
        f |= F_NAME;
    }
    if leaf.ed25519_key().is_none() {
        f |= F_BAD_SIGNATURE;
    }
    let mut used = alloc::vec![false; chain.len()];
    let mut cur = leaf;
    used[0] = true;
    loop {
        if now >= cur.not_after {
            f |= F_EXPIRED;
        }
        if now < cur.not_before {
            f |= F_NOT_YET;
        }
        // an anchor, by its very bytes or by its signature
        if anchors.iter().any(|a| a.der == cur.der || (a.is_ca && cur.signed_by(a))) {
            return f;
        }
        let next = chain.iter().enumerate().find(|(i, c)| !used[*i] && c.is_ca && cur.signed_by(c));
        match next {
            Some((i, c)) => {
                used[i] = true;
                cur = c;
            }
            None => break,
        }
    }
    if cur.self_signed() {
        f | F_SELF_SIGNED
    } else if cur.issuer == cur.subject {
        f | F_BAD_SIGNATURE
    } else {
        f | F_UNKNOWN_ISSUER
    }
}

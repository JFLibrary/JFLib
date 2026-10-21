// checksum -- the Internet checksum (RFC 1071, 1624) and the Ethernet CRC-32
//
//   sum(acc, data) -> u32          ones' complement sum of 16-bit big-endian words; only the
//                                  last piece of a message may have an odd length
//   finish(acc) -> u16             the checksum to write (complement of the folded sum)
//   checksum(data) -> u16          finish(sum(0, data))
//   verify(data) -> bool           a block that contains its checksum sums to 0xFFFF
//   pseudo_v4 / pseudo_v6          the pseudo-header sums of UDP, TCP and ICMPv6
//   update16(ck, old, new) -> u16  incremental update after a 16-bit field changed (RFC 1624)
//   crc32(data) -> u32             IEEE 802.3 FCS (reflected 0x04C11DB7, as zlib)

use crate::addr::{Ipv4, Ipv6};

/// adds `data` to a running ones' complement sum (the result is folded to 17 bits at most)
pub fn sum(acc: u32, data: &[u8]) -> u32 {
    let mut s = acc as u64;
    let mut chunks = data.chunks_exact(2);
    for c in &mut chunks {
        s += u16::from_be_bytes([c[0], c[1]]) as u64;
    }
    if let [last] = chunks.remainder() {
        s += (*last as u64) << 8;
    }
    while s > 0xFFFF {
        s = (s & 0xFFFF) + (s >> 16);
    }
    s as u32
}

/// folds a sum to 16 bits (without the final complement)
pub fn fold(acc: u32) -> u16 {
    let mut s = acc;
    while s > 0xFFFF {
        s = (s & 0xFFFF) + (s >> 16);
    }
    s as u16
}

/// the checksum value to store: the complement of the folded sum
pub fn finish(acc: u32) -> u16 {
    !fold(acc)
}

/// the Internet checksum of a block
pub fn checksum(data: &[u8]) -> u16 {
    finish(sum(0, data))
}

/// a block that includes its checksum field is intact
pub fn verify(data: &[u8]) -> bool {
    fold(sum(0, data)) == 0xFFFF
}

/// the IPv4 pseudo header of UDP / TCP: addresses, protocol, length of the segment
pub fn pseudo_v4(src: Ipv4, dst: Ipv4, proto: u8, len: u16) -> u32 {
    let mut s = sum(0, &src.0);
    s = sum(s, &dst.0);
    sum(s, &[0, proto, (len >> 8) as u8, len as u8])
}

/// the IPv6 pseudo header (RFC 8200 8.1): addresses, upper-layer length, next header
pub fn pseudo_v6(src: Ipv6, dst: Ipv6, next: u8, len: u32) -> u32 {
    let mut s = sum(0, &src.0);
    s = sum(s, &dst.0);
    s = sum(s, &len.to_be_bytes());
    sum(s, &[0, 0, 0, next])
}

/// the new checksum after one 16-bit word changed from `old` to `new` (RFC 1624 eqn. 3)
pub fn update16(ck: u16, old: u16, new: u16) -> u16 {
    let s = (!ck as u32) + (!old as u32) + new as u32;
    !fold(s)
}

// t[n] = CRC of the byte n (reflected polynomial)
static CRC_TAB: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
};

/// CRC-32 of IEEE 802.3 (the Ethernet frame check sequence; also zlib's and PNG's)
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c = CRC_TAB[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

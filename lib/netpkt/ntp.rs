// ntp -- the Network Time Protocol packet (RFC 5905: the 48-byte header; extension fields and
// MACs are not read)
//
//   Ts(u64)                  32.32 fixed-point seconds since 1900 (era 0, until 2036);
//                            from_unix_ns / to_unix_ns round to the nearest step, so whole
//                            nanoseconds survive the round trip
//   NtpView::parse(b)        li, vn, mode, stratum, poll, precision, root_delay, root_disp,
//                            refid, reference, origin, receive, transmit
//   Ntp { .. }               read / to_bytes / client(vn, transmit)
//   offset_delay(t1, t2, t3, t4) -> (offset, delay)   the on-wire calculation (RFC 5905 8)
//   civil(unix_secs) -> (year, month, day, hour, minute, second)   UTC, for dissectors
//   MODE_* / LI_*; mode_name

use alloc::vec::Vec;

use crate::{be32, put32};

pub const NTP_LEN: usize = 48;
/// seconds from 1900-01-01 to 1970-01-01
pub const UNIX_OFFSET: u64 = 2_208_988_800;

pub const MODE_SYM_ACTIVE: u8 = 1;
pub const MODE_SYM_PASSIVE: u8 = 2;
pub const MODE_CLIENT: u8 = 3;
pub const MODE_SERVER: u8 = 4;
pub const MODE_BROADCAST: u8 = 5;
pub const LI_NONE: u8 = 0;
/// the clock is not synchronised
pub const LI_ALARM: u8 = 3;

pub fn mode_name(m: u8) -> &'static str {
    match m {
        MODE_SYM_ACTIVE => "symmetric active",
        MODE_SYM_PASSIVE => "symmetric passive",
        MODE_CLIENT => "client",
        MODE_SERVER => "server",
        MODE_BROADCAST => "broadcast",
        6 => "control",
        7 => "private",
        _ => "reserved",
    }
}

/// an NTP timestamp
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ts(pub u64);

impl Ts {
    /// nanoseconds since 1970 (era 0)
    pub fn from_unix_ns(ns: i64) -> Ts {
        let secs = ns.div_euclid(1_000_000_000) as i128 + UNIX_OFFSET as i128;
        let rem = ns.rem_euclid(1_000_000_000) as u128;
        let frac = ((rem << 32) + 500_000_000) / 1_000_000_000;
        Ts((((secs as u128) << 32) + frac) as u64)
    }
    pub fn to_unix_ns(self) -> i64 {
        let secs = (self.0 >> 32) as i64 - UNIX_OFFSET as i64;
        let frac = ((self.0 & 0xFFFF_FFFF) as u128 * 1_000_000_000 + (1 << 31)) >> 32;
        secs * 1_000_000_000 + frac as i64
    }
    pub fn secs(self) -> u32 {
        (self.0 >> 32) as u32
    }
    pub fn frac(self) -> u32 {
        self.0 as u32
    }
}

fn ts_at(b: &[u8], off: usize) -> Ts {
    Ts((be32(b, off) as u64) << 32 | be32(b, off + 4) as u64)
}

/// a received NTP packet
#[derive(Clone, Copy, Debug)]
pub struct NtpView<'a> {
    b: &'a [u8],
}

impl<'a> NtpView<'a> {
    pub fn parse(b: &'a [u8]) -> Option<NtpView<'a>> {
        if b.len() < NTP_LEN {
            return None;
        }
        Some(NtpView { b })
    }
    pub fn li(&self) -> u8 {
        self.b[0] >> 6
    }
    pub fn vn(&self) -> u8 {
        (self.b[0] >> 3) & 7
    }
    pub fn mode(&self) -> u8 {
        self.b[0] & 7
    }
    pub fn stratum(&self) -> u8 {
        self.b[1]
    }
    pub fn poll(&self) -> i8 {
        self.b[2] as i8
    }
    pub fn precision(&self) -> i8 {
        self.b[3] as i8
    }
    /// 16.16 seconds
    pub fn root_delay(&self) -> u32 {
        be32(self.b, 4)
    }
    pub fn root_disp(&self) -> u32 {
        be32(self.b, 8)
    }
    pub fn refid(&self) -> [u8; 4] {
        [self.b[12], self.b[13], self.b[14], self.b[15]]
    }
    pub fn reference(&self) -> Ts {
        ts_at(self.b, 16)
    }
    pub fn origin(&self) -> Ts {
        ts_at(self.b, 24)
    }
    pub fn receive(&self) -> Ts {
        ts_at(self.b, 32)
    }
    pub fn transmit(&self) -> Ts {
        ts_at(self.b, 40)
    }
}

/// an NTP packet to write
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ntp {
    pub li: u8,
    pub vn: u8,
    pub mode: u8,
    pub stratum: u8,
    pub poll: i8,
    pub precision: i8,
    pub root_delay: u32,
    pub root_disp: u32,
    pub refid: [u8; 4],
    pub reference: Ts,
    pub origin: Ts,
    pub receive: Ts,
    pub transmit: Ts,
}

impl Ntp {
    /// a client request: only the version, the mode and the transmit time (RFC 4330 style,
    /// as SNTP clients and chrony send)
    pub fn client(vn: u8, transmit: Ts) -> Ntp {
        Ntp { li: LI_NONE, vn, mode: MODE_CLIENT, transmit, ..Ntp::default() }
    }
    pub fn read(v: &NtpView) -> Ntp {
        Ntp {
            li: v.li(),
            vn: v.vn(),
            mode: v.mode(),
            stratum: v.stratum(),
            poll: v.poll(),
            precision: v.precision(),
            root_delay: v.root_delay(),
            root_disp: v.root_disp(),
            refid: v.refid(),
            reference: v.reference(),
            origin: v.origin(),
            receive: v.receive(),
            transmit: v.transmit(),
        }
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; NTP_LEN];
        b[0] = (self.li & 3) << 6 | (self.vn & 7) << 3 | (self.mode & 7);
        b[1] = self.stratum;
        b[2] = self.poll as u8;
        b[3] = self.precision as u8;
        put32(&mut b, 4, self.root_delay);
        put32(&mut b, 8, self.root_disp);
        b[12..16].copy_from_slice(&self.refid);
        for (i, t) in [self.reference, self.origin, self.receive, self.transmit].iter().enumerate() {
            put32(&mut b, 16 + 8 * i, (t.0 >> 32) as u32);
            put32(&mut b, 20 + 8 * i, t.0 as u32);
        }
        b
    }
}

/// the clock offset and the round-trip delay from the four times of an exchange (client send
/// t1, server receive t2, server send t3, client receive t4; all in ns)
pub fn offset_delay(t1: i64, t2: i64, t3: i64, t4: i64) -> (i64, i64) {
    let (t1, t2, t3, t4) = (t1 as i128, t2 as i128, t3 as i128, t4 as i128);
    let clamp = |v: i128| v.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    (clamp(((t2 - t1) + (t3 - t4)) / 2), clamp((t4 - t1) - (t3 - t2)))
}

/// the UTC calendar date and time of a Unix time (days from civil, H. Hinnant)
pub fn civil(unix_secs: i64) -> (i32, u8, u8, u8, u8, u8) {
    let days = unix_secs.div_euclid(86_400);
    let sod = unix_secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
    (y, m, d, (sod / 3600) as u8, (sod / 60 % 60) as u8, (sod % 60) as u8)
}

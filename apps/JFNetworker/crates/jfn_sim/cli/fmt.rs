// fmt -- how the consoles write numbers, times and addresses
//
//   ios_stamp(ns)         "*Mar  1 00:00:12.345: " (IOS log and debug time stamps; the clock of
//                         a device that never learned the time starts on March 1st, the "*"
//                         says it is not synchronised)
//   mask_str(len)         "255.255.255.0"
//   iputils_time(us)      " time=0.045 ms" with the precision iputils ping chooses
//   ms3(us)               "1.234" (milliseconds with three decimals)
//   pct_g(num, den)       100 * num / den as printf("%g") writes it ("33.3333", "25", "0")
//   isqrt(v)              the integer square root (iputils' mdev)
//   ios_hw(kbps)          "Gigabit Ethernet", "Ten Gigabit Ethernet" ... (show interfaces)
//   pad(s, w)             left-aligned in w characters (never cut)

use alloc::format;
use alloc::string::String;

use netpkt::Ipv4;

use crate::kernel::{Ns, MS, SEC};

/// the IOS time stamp of a virtual time
pub fn ios_stamp(t: Ns) -> String {
    const DAYS: [(&str, u64); 10] = [("Mar", 31), ("Apr", 30), ("May", 31), ("Jun", 30), ("Jul", 31), ("Aug", 31), ("Sep", 30), ("Oct", 31), ("Nov", 30), ("Dec", 31)];
    let ms = (t / MS) % 1000;
    let s = t / SEC;
    let (h, m, sec) = ((s / 3600) % 24, (s / 60) % 60, s % 60);
    let mut day = s / 86_400;
    let mut month = "Mar";
    for (name, len) in DAYS {
        month = name;
        if day < len {
            break;
        }
        day -= len;
    }
    format!("*{} {:>2} {:02}:{:02}:{:02}.{:03}: ", month, day + 1, h, m, sec, ms)
}

/// a dotted netmask
pub fn mask_str(len: u8) -> String {
    format!("{}", Ipv4::mask_of(len.min(32)))
}

/// the RTT of a reply as iputils ping prints it (microseconds in)
pub fn iputils_time(us: u64) -> String {
    if us >= 100_000 - 50 {
        format!(" time={} ms", (us + 500) / 1000)
    } else if us >= 10_000 - 5 {
        format!(" time={}.{:01} ms", (us + 50) / 1000, ((us + 50) % 1000) / 100)
    } else if us >= 1000 {
        format!(" time={}.{:02} ms", (us + 5) / 1000, ((us + 5) % 1000) / 10)
    } else {
        format!(" time={}.{:03} ms", us / 1000, us % 1000)
    }
}

/// milliseconds with three decimals from microseconds ("0.004")
pub fn ms3(us: u64) -> String {
    format!("{}.{:03}", us / 1000, us % 1000)
}

/// 100 * num / den with six significant digits and no trailing zeros (printf "%g")
pub fn pct_g(num: u64, den: u64) -> String {
    if den == 0 || num == 0 {
        return String::from("0");
    }
    let (n, d) = (num as u128 * 100, den as u128);
    if n % d == 0 {
        return format!("{}", n / d);
    }
    // the decimal exponent of the value, then six significant digits in fixed notation
    let whole = n / d;
    let e: i32 = if whole > 0 {
        format!("{}", whole).len() as i32 - 1
    } else {
        let (mut r, mut z) = (n, 0);
        while r * 10 < d {
            r *= 10;
            z += 1;
        }
        -(z + 1)
    };
    let decimals = (5 - e).max(0) as u32;
    let scale = 10u128.pow(decimals);
    let scaled = (n * scale * 2 + d) / (2 * d);
    let mut f = format!("{:0w$}", scaled % scale, w = decimals as usize);
    while f.ends_with('0') {
        f.pop();
    }
    if f.is_empty() { format!("{}", scaled / scale) } else { format!("{}.{}", scaled / scale, f) }
}

/// the integer square root (floor)
pub fn isqrt(v: u128) -> u128 {
    if v < 2 {
        return v;
    }
    let mut x = v;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// the IOS hardware name of an Ethernet speed
pub fn ios_hw(kbps: u32) -> &'static str {
    match kbps {
        k if k >= 100_000_000 => "Hundred Gigabit Ethernet",
        k if k >= 40_000_000 => "Forty Gigabit Ethernet",
        k if k >= 25_000_000 => "Twenty Five Gigabit Ethernet",
        k if k >= 10_000_000 => "Ten Gigabit Ethernet",
        k if k >= 5_000_000 => "Five Gigabit Ethernet",
        k if k >= 2_500_000 => "Two Gigabit Ethernet",
        k if k >= 1_000_000 => "Gigabit Ethernet",
        k if k >= 100_000 => "Fast Ethernet",
        _ => "Ethernet",
    }
}

/// `s` left-aligned in `w` characters
pub fn pad(s: &str, w: usize) -> String {
    format!("{:<w$}", s, w = w)
}

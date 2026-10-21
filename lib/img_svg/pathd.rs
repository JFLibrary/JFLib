// pathd -- SVG path data (d="...") into gfx path data
//
//   parse(d) -> PData     M L H V C S Q T A Z (upper case absolute, lower case relative),
//                         implicit repetition (pairs after a moveto are linetos), compressed
//                         numbers ("10-5.5.5") and arc flags ("a5 5 0 01 10 0")
//
// SVG's error rule: everything up to the first error is drawn.  A moveto is only emitted when
// something follows it: a lone "M x y" (also a trailing one) draws nothing, while "M x y z" is
// a zero-length closed subpath (the caps of a stroke draw a dot).  gfx::pathdata's own parser
// cannot tell those apart once the commands are stored, so this one feeds its builder.

use gfx::pathdata::PData;

use crate::values::{blank, number, sep};

// the parameters of a command (lower case letter)
fn nargs(c: u8) -> usize {
    match c {
        b'm' | b'l' | b't' => 2,
        b'h' | b'v' => 1,
        b'c' => 6,
        b's' | b'q' => 4,
        b'a' => 7,
        _ => 0,
    }
}

// an arc flag: one digit 0 / 1 (no separator needed)
fn flag(s: &[u8], i: &mut usize) -> Option<f64> {
    sep(s, i);
    match s.get(*i) {
        Some(b'0') => {
            *i += 1;
            Some(0.0)
        }
        Some(b'1') => {
            *i += 1;
            Some(1.0)
        }
        _ => None,
    }
}

/// the path data of d (up to its first error)
pub(crate) fn parse(d: &str) -> PData {
    let s = d.as_bytes();
    let mut pd = PData::new();
    let mut i = 0;
    let mut cmd = 0u8; // lower case
    let mut rel = false;
    let mut cur = (0.0f64, 0.0f64);
    let mut start = (0.0f64, 0.0f64);
    let mut pending: Option<(f64, f64)> = None; // a moveto not emitted yet
    let mut last_ctrl: Option<(u8, (f64, f64))> = None; // b'c' / b'q' and its second control point
    let ensure = |pd: &mut PData, pending: &mut Option<(f64, f64)>| {
        if let Some(p) = pending.take() {
            pd.move_to(p.0, p.1);
        }
    };
    loop {
        while i < s.len() && (blank(s[i]) || s[i] == b',') {
            i += 1;
        }
        if i >= s.len() {
            break;
        }
        let c = s[i];
        if c.is_ascii_alphabetic() {
            let lc = c | 32;
            if lc == b'z' {
                i += 1;
                if cmd == 0 {
                    break; // no moveto before
                }
                if pending.is_some() {
                    ensure(&mut pd, &mut pending); // "M x y z": a zero-length closed subpath
                }
                pd.close();
                cur = start;
                last_ctrl = None;
                cmd = b'z';
                continue;
            }
            if nargs(lc) == 0 || (cmd == 0 && lc != b'm') {
                break; // an unknown command, or the first is not a moveto
            }
            cmd = lc;
            rel = c == lc; // lower case: relative
            i += 1;
        } else if cmd == 0 || cmd == b'z' {
            break; // numbers without a command
        }
        // one parameter group
        let n = nargs(cmd);
        let mut a = [0f64; 7];
        let mut ok = true;
        for (k, v) in a.iter_mut().enumerate().take(n) {
            let got = if cmd == b'a' && (k == 3 || k == 4) {
                flag(s, &mut i)
            } else {
                sep(s, &mut i);
                number(s, &mut i)
            };
            match got {
                Some(x) => *v = x,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            break;
        }
        let (ox, oy) = if rel { cur } else { (0.0, 0.0) };
        match cmd {
            b'm' => {
                let p = (a[0] + ox, a[1] + oy);
                pending = Some(p); // an earlier lone moveto is dropped
                cur = p;
                start = p;
                last_ctrl = None;
                cmd = b'l'; // further pairs are linetos
            }
            b'l' | b'h' | b'v' => {
                let p = match cmd {
                    b'l' => (a[0] + ox, a[1] + oy),
                    b'h' => (a[0] + ox, cur.1),
                    _ => (cur.0, a[0] + oy),
                };
                ensure(&mut pd, &mut pending);
                pd.line_to(p.0, p.1);
                cur = p;
                last_ctrl = None;
            }
            b'c' | b's' => {
                let (c1, rest) = if cmd == b'c' {
                    ((a[0] + ox, a[1] + oy), [a[2] + ox, a[3] + oy, a[4] + ox, a[5] + oy])
                } else {
                    let c1 = match last_ctrl {
                        Some((b'c', p)) => (2.0 * cur.0 - p.0, 2.0 * cur.1 - p.1),
                        _ => cur,
                    };
                    (c1, [a[0] + ox, a[1] + oy, a[2] + ox, a[3] + oy])
                };
                ensure(&mut pd, &mut pending);
                pd.cubic(&[c1.0, c1.1, rest[0], rest[1], rest[2], rest[3]]);
                last_ctrl = Some((b'c', (rest[0], rest[1])));
                cur = (rest[2], rest[3]);
            }
            b'q' | b't' => {
                let (q, p) = if cmd == b'q' {
                    ((a[0] + ox, a[1] + oy), (a[2] + ox, a[3] + oy))
                } else {
                    let q = match last_ctrl {
                        Some((b'q', p)) => (2.0 * cur.0 - p.0, 2.0 * cur.1 - p.1),
                        _ => cur,
                    };
                    (q, (a[0] + ox, a[1] + oy))
                };
                ensure(&mut pd, &mut pending);
                pd.quad(&[q.0, q.1, p.0, p.1]);
                last_ctrl = Some((b'q', q));
                cur = p;
            }
            _ => {
                // b'a'
                let p = (a[5] + ox, a[6] + oy);
                ensure(&mut pd, &mut pending);
                pd.arc(&[cur.0, cur.1, a[0], a[1], a[2], a[3], a[4], p.0, p.1]);
                cur = p;
                last_ctrl = None;
            }
        }
        if pd.err != 0 {
            break; // a value out of range or the size limit
        }
    }
    pd.err = 0;
    pd
}

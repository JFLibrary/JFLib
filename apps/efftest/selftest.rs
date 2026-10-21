// selftest -- efftest --selftest [all]: built-in checks (tests/selftest.uis)  [apps/efftest/src/selftest.s]
//
// On a 97 x 61 hash-pattern image with transparent pixels:
//   * known answers: invert_colors (c ^ 0x00FFFFFF) and black_white (paint.net intensity)
//   * every effect of the categories adjust, color, stylize ("all": every registered effect)
//     with default properties: serial 256-pixel tiles == pool with 23-pixel tiles (bit
//     identical; the destination is pre-filled differently, so every pixel must be written),
//     and a set cancel flag makes prepare or render return ERR_CANCEL
// Prints "fail NAME: WHAT" lines and "selftest: N effects, F failures"; exit code 0 / 1.  (The
// asm version also checks the guard bytes around its surfaces; fx::Dst makes them moot.)

use alloc::format;

use fx::{FxDesc, Params, FXC_ADJUST, FXC_COLOR, FXC_STYLIZE};
use gfx::Surface;

use crate::{alloc, out, row_mut, Grid, Run, Toks, CANCELLED};

const ST_W: i32 = 97;
const ST_H: i32 = 61;
const ROI: [i32; 4] = [0, 0, ST_W, ST_H];

pub(crate) fn run(t: &Toks) -> i32 {
    let all = t.value("--selftest") == Some("all");
    let (Ok(mut src), Ok(mut dst), Ok(mut bak)) = (alloc(ST_W as u32, ST_H as u32, 0), alloc(ST_W as u32, ST_H as u32, 0), alloc(ST_W as u32, ST_H as u32, 0)) else {
        out("fail -: no memory\n");
        return 1;
    };
    pattern(&mut src);
    let mut fails = known(&src, &mut dst);
    let mut n = 0;
    for d in (0..).map_while(fx::get) {
        if !all && d.category != FXC_ADJUST && d.category != FXC_COLOR && d.category != FXC_STYLIZE {
            continue;
        }
        n += 1;
        fails += one(d, &src, &mut dst, &mut bak);
    }
    out(&format!("selftest: {n} effects, {fails} failures\n"));
    (fails != 0) as i32
}

/// hash colours; x < 12 opaque, (x + 2y) % 11 == 0 transparent  [st_pattern]
fn pattern(src: &mut Surface) {
    for y in 0..ST_H {
        let r = row_mut(src, y);
        for x in 0..ST_W {
            let mut c = fx::hash32(x as u32, y as u32, 1234, 0);
            if x < 12 {
                c |= 0xFF00_0000;
            }
            if (x + 2 * y) % 11 == 0 {
                c &= 0x00FF_FFFF;
            }
            fx::put(r, x as usize, c);
        }
    }
}

fn fill(sf: &mut Surface, v: u32) {
    let w = sf.w as usize;
    for y in 0..sf.h {
        let r = row_mut(sf, y);
        for x in 0..w {
            fx::put(r, x, v);
        }
    }
}

/// prepare + render over the whole image -> 0 | error  [st_apply]
fn apply(d: &FxDesc, p: &Params, src: &Surface, dst: &mut Surface, tile: i32, serial: bool) -> i32 {
    let g = Grid::new(d, ROI, tile, ST_W, ST_H);
    let base = Run { desc: d, src, params: p, aux: None, sel: None };
    let aux = match d.prepare {
        Some(f) => match f(&base.args(ROI, None)) {
            Ok(a) => a,
            Err(e) => return e,
        },
        None => None,
    };
    let run = Run { aux: aux.as_deref(), ..base };
    run.render(&g, dst, serial)
}

/// invert_colors and black_white against direct formulas -> failures  [st_known]
fn known(src: &Surface, dst: &mut Surface) -> i32 {
    let mut fails = 0;
    for (name, mode) in [("invert_colors", 0), ("black_white", 1)] {
        let ok = match fx::find(name) {
            Some(d) => {
                let mut p = Params::default();
                fx::defaults(d, &mut p);
                apply(d, &p, src, dst, 64, true) == 0 && check_known(src, dst, mode)
            }
            None => false,
        };
        if !ok {
            fails += 1;
            out(&format!("fail {name}: known answer\n"));
        }
    }
    fails
}

fn check_known(src: &Surface, dst: &Surface, mode: u32) -> bool {
    for y in 0..ST_H {
        let (s, d) = (fx::row(src, y), fx::row(dst, y));
        for x in 0..ST_W as usize {
            let c = fx::px(s, x);
            let e = if mode == 0 { c ^ 0x00FF_FFFF } else { fx::lut::luma(c) * 0x010101 | c & 0xFF00_0000 };
            if e != fx::px(d, x) {
                return false;
            }
        }
    }
    true
}

/// determinism and cancel of one effect -> 0 ok / 1  [st_one]
fn one(d: &FxDesc, src: &Surface, dst: &mut Surface, bak: &mut Surface) -> i32 {
    let fail = |what: &str| {
        out(&format!("fail {}: {what}\n", d.name));
        1
    };
    let mut p = Params::default();
    fx::defaults(d, &mut p);
    fill(dst, 0x1234_5678);
    if apply(d, &p, src, dst, 256, true) != 0 {
        return fail("render error (serial)");
    }
    bak.px.copy_from_slice(&dst.px);
    fill(dst, 0x9ABC_DEF0);
    if apply(d, &p, src, dst, 23, false) != 0 {
        return fail("render error (pool)");
    }
    for y in 0..ST_H {
        if fx::row(bak, y) != fx::row(dst, y) {
            return fail("pool / 23-pixel tiles differ from serial / 256");
        }
    }
    // a set cancel flag
    let flag: &(dyn fx::Cancel + Sync) = &CANCELLED;
    let base = Run { desc: d, src, params: &p, aux: None, sel: None };
    let aux = match d.prepare {
        Some(f) => match f(&base.args(ROI, Some(flag))) {
            Ok(Some(a)) => Some(a),
            Err(fx::ERR_CANCEL) => return 0,
            _ => return fail("cancelled prepare failed with another error"),
        },
        None => None,
    };
    let run = Run { aux: aux.as_deref(), ..base };
    let mut dd = fx::Dst::of_surface(dst, ROI);
    if (d.render)(&run.args(ROI, Some(flag)), &mut dd) != fx::ERR_CANCEL {
        return fail("render ignores JOB_CANCEL");
    }
    0
}

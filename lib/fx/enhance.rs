// Native equivalents of the additional photographic and object operations used by
// the handbook. These kernels do not copy or claim pixel identity with third-party DLLs.
use crate::conv::*;
use crate::lochist::*;
use crate::*;

pub const COLOR_BALANCE: u32 = 14;
pub const BLACK_WHITE_PLUS: u32 = 15;
pub const DREAM: u32 = 23;
pub const OIL_PLUS: u32 = 24;
pub const PASTEL: u32 = 25;
pub const FEATHER: u32 = 81;
pub const INNER_SHADOW: u32 = 82;
pub const BEVEL: u32 = 83;
pub const DODGE_BURN: u32 = 96;
pub const CONTENT_FILL: u32 = 97;
pub const PORTRAIT: u32 = 98;

// Edge-preserving smoothing with a colour-distance weight. Alpha is unchanged.
fn portrait(a: &FxArgs, dst: &mut Dst) -> i32 {
    let r = pint(a, 0, 1, 12);
    let strength = pint(a, 1, 0, 100) as u32;
    let threshold = pint(a, 2, 1, 100) as i32;
    rows(a, dst, |d, s, x0, y| {
        for i in 0..s.len() / 4 {
            let center = px(s, i);
            let mut sum = [0u64; 3];
            let mut weights = 0u64;
            for dy in [-r, -r / 2, 0, r / 2, r] {
                for dx in [-r, -r / 2, 0, r / 2, r] {
                    let x = (x0 + i as i32 + dx).clamp(0, a.src.w - 1);
                    let y = (y + dy).clamp(0, a.src.h - 1);
                    let q = px(a.src_row(y), x as usize);
                    let difference = (0..3)
                        .map(|c| ((q >> (c * 8) & 255) as i32 - (center >> (c * 8) & 255) as i32).abs())
                        .max()
                        .unwrap_or(0);
                    if difference >= threshold {
                        continue;
                    }
                    let weight = ((threshold - difference) as u64).pow(2) * (q >> 24) as u64;
                    weights += weight;
                    for c in 0..3 {
                        sum[c] += (q >> (c * 8) & 255) as u64 * weight;
                    }
                }
            }
            let mut result = center & 0xff000000;
            for c in 0..3 {
                let old = center >> (c * 8) & 255;
                let smooth = if weights == 0 { old } else { (sum[c] / weights) as u32 };
                result |= ((old * (100 - strength) + smooth * strength + 50) / 100) << (c * 8);
            }
            put(d, i, result);
        }
    })
}

// Exemplar inpainting: fill selected pixels from the boundary inward, matching
// small neighbourhoods against unselected source pixels. Deterministic candidates
// and propagation preserve reproducible preview/commit results on every backend.
fn content_fill(a: &FxArgs, dst: &mut Dst) -> i32 {
    let Some(mask) = a.sel else {
        return rows(a, dst, |d, s, _, _| d.copy_from_slice(s));
    };
    let (w, h) = (a.src.w as usize, a.src.h as usize);
    let n = w * h;
    if n > 25_000_000 {
        return ERR_LIMIT;
    }
    let mut image = Vec::with_capacity(n);
    let mut nearest = alloc::vec![u32::MAX;n];
    let mut known = Vec::new();
    for y in 0..h {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        for x in 0..w {
            let i = y * w + x;
            image.push(px(a.src_row(y as i32), x));
            if mask.get(x as i32, y as i32) == 0 {
                nearest[i] = i as u32;
                known.push(i as u32);
            }
        }
    }
    if known.is_empty() {
        return ERR_DIMS;
    }
    let mut queue = known.clone();
    let mut at = 0;
    while at < queue.len() {
        if at & 4095 == 0 && a.cancelled() {
            return ERR_CANCEL;
        }
        let i = queue[at] as usize;
        at += 1;
        let (x, y) = (i % w, i / w);
        for (xx, yy) in [
            (x as i32 - 1, y as i32),
            (x as i32 + 1, y as i32),
            (x as i32, y as i32 - 1),
            (x as i32, y as i32 + 1),
        ] {
            if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
                continue;
            }
            let j = yy as usize * w + xx as usize;
            if nearest[j] == u32::MAX {
                nearest[j] = nearest[i];
                queue.push(j as u32);
            }
        }
    }
    let mut filled = alloc::vec![false;n];
    for &i in &known {
        filled[i as usize] = true;
    }
    let tries = pint(a, 0, 8, 64) as usize;
    for &index in &queue[known.len()..] {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let i = index as usize;
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let mut best = nearest[i] as usize;
        let mut best_score = u64::MAX;
        let mut rng = index.wrapping_mul(747796405).wrapping_add(2891336453);
        for attempt in 0..tries + 1 {
            let candidate = if attempt == 0 {
                best
            } else {
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                known[rng as usize % known.len()] as usize
            };
            let (cx, cy) = ((candidate % w) as i32, (candidate / w) as i32);
            let mut score = 0u64;
            let mut count = 0u64;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let (tx, ty, sx, sy) = (x + dx, y + dy, cx + dx, cy + dy);
                    if tx < 0
                        || ty < 0
                        || tx >= w as i32
                        || ty >= h as i32
                        || sx < 0
                        || sy < 0
                        || sx >= w as i32
                        || sy >= h as i32
                    {
                        continue;
                    }
                    let j = ty as usize * w + tx as usize;
                    if !filled[j] || mask.get(sx, sy) != 0 {
                        continue;
                    }
                    let p = image[j];
                    let q = px(a.src_row(sy), sx as usize);
                    for c in 0..4 {
                        let d = (p >> (c * 8) & 255) as i32 - (q >> (c * 8) & 255) as i32;
                        score += (d * d) as u64;
                    }
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            let score = score * 1024 / count + (x - cx).unsigned_abs() as u64 + (y - cy).unsigned_abs() as u64;
            if score < best_score {
                best_score = score;
                best = candidate;
            }
        }
        nearest[i] = best as u32;
        image[i] = px(a.src_row((best / w) as i32), best % w);
        filled[i] = true;
    }
    rows(a, dst, |d, _, x, y| {
        for i in 0..d.len() / 4 {
            put(d, i, image[y as usize * w + x as usize + i]);
        }
    })
}

fn adjust(v: i32, brightness: i32, contrast: i32) -> u32 {
    let c = contrast as f64 * 2.55;
    let slope = 259.0 * (c + 255.0) / (255.0 * (259.0 - c));
    (slope * (v as f64 - 128.0) + 128.0 + brightness as f64 * 2.55 + 0.5).clamp(0.0, 255.0) as u32
}

fn balance(a: &FxArgs, dst: &mut Dst) -> i32 {
    let delta = [pint(a, 2, -100, 100), pint(a, 1, -100, 100), pint(a, 0, -100, 100)];
    let b = pint(a, 3, -100, 100);
    let c = pint(a, 4, -100, 100);
    rows(a, dst, |d, s, _, _| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            let mut out = p & 0xff000000;
            for ch in 0..3 {
                out |= adjust((p >> (ch * 8) & 255) as i32 + delta[ch] * 255 / 100, b, c) << (ch * 8);
            }
            put(d, i, out);
        }
    })
}

fn bw(a: &FxArgs, dst: &mut Dst) -> i32 {
    let mode = pint(a, 0, 0, 6);
    let b = pint(a, 1, -100, 100);
    let c = pint(a, 2, -100, 100);
    rows(a, dst, |d, s, _, _| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            let (r, g, bl) = ((p >> 16 & 255) as i32, (p >> 8 & 255) as i32, (p & 255) as i32);
            let v = match mode {
                1 => (r + g + bl) / 3,
                2 => (r.max(g).max(bl) + r.min(g).min(bl)) / 2,
                3 => r,
                4 => g,
                5 => bl,
                6 => (r * 60 + g * 30 + bl * 10) / 100,
                _ => (r * 299 + g * 587 + bl * 114 + 500) / 100,
            };
            let v = adjust(v, b, c);
            put(d, i, p & 0xff000000 | v * 0x010101);
        }
    })
}

fn dream(a: &FxArgs, dst: &mut Dst) -> i32 {
    let focus = pint(a, 0, 0, 100);
    let strength = pint(a, 1, 0, 100) as u32;
    blurbac(a, dst, 1 + (100 - focus) / 4, 0, 0, |dst, y, x, row| {
        let src = &a.src_row(y)[x as usize * 4..x as usize * 4 + row.len()];
        let d = dst_px(dst, y, x, (row.len() / 4) as i32);
        for i in 0..row.len() / 4 {
            let (p, q) = (px(src, i), px(row, i));
            let mut o = p & 0xff000000;
            for ch in [0, 8, 16] {
                let (v, w) = (p >> ch & 255, q >> ch & 255);
                let screen = 255 - (255 - v) * (255 - w) / 255;
                o |= ((v * (100 - strength) + screen * strength + 50) / 100) << ch;
            }
            put(d, i, o);
        }
    })
}

fn oil(a: &FxArgs, dst: &mut Dst) -> i32 {
    let radius = pint(a, 0, 1, 20);
    let levels = pint(a, 1, 3, 255);
    let shadow = pint(a, 2, 0, 100) as u32;
    let detail = pint(a, 3, 1, 10) as u32;
    lh_run_oil(a, dst, radius, LHS_DISC, levels, |h, _, _| {
        let mut best = 0;
        for b in 1..=levels as usize {
            if h.cnt[b] > h.cnt[best] {
                best = b;
            }
        }
        let sum = h.sum[best];
        if sum[3] == 0 || h.cnt[best] == 0 {
            return 0;
        }
        let mut out = (sum[3] / h.cnt[best]) << 24;
        let src_l = crate::lut::luma(h.src);
        let oil_l = (sum[0] / sum[3] * 114 + sum[1] / sum[3] * 587 + sum[2] / sum[3] * 299) / 1000;
        let shade = oil_l
            .saturating_sub(src_l)
            .saturating_mul(shadow)
            .saturating_mul(detail)
            / 100;
        for ch in 0..3 {
            out |= (sum[ch] / sum[3]).saturating_sub(shade.min(255)) << (ch * 8);
        }
        out
    })
}

fn pastel(a: &FxArgs, dst: &mut Dst) -> i32 {
    let size = pint(a, 0, 1, 20);
    let rough = pint(a, 1, 0, 100);
    blurbac(a, dst, size, 0, 0, |dst, y, x, row| {
        let d = dst_px(dst, y, x, (row.len() / 4) as i32);
        for i in 0..row.len() / 4 {
            let p = px(row, i);
            let seed = ((x + i as i32) as u32).wrapping_mul(0x9e3779b9) ^ (y as u32).wrapping_mul(0x85ebca6b);
            let hash = (seed ^ (seed >> 16)).wrapping_mul(0x7feb352d);
            let grain = ((hash >> 24) as i32 - 128) * rough / 500;
            let mut out = p & 0xff000000;
            for ch in [0, 8, 16] {
                let v = (p >> ch & 255) as i32;
                let step = 4 + rough / 5;
                let q = ((v + step / 2) / step * step + grain + 8).clamp(0, 255) as u32;
                out |= q << ch;
            }
            put(d, i, out);
        }
    })
}

fn feather(a: &FxArgs, dst: &mut Dst) -> i32 {
    blurbac(a, dst, pint(a, 0, 0, 100), 0, 0, |dst, y, x, row| {
        let d = dst_px(dst, y, x, (row.len() / 4) as i32);
        // Alpha-aware blur also supplies meaningful RGB to newly translucent pixels.
        d.copy_from_slice(row);
        let src = &a.src_row(y)[x as usize * 4..x as usize * 4 + row.len()];
        for i in 0..row.len() / 4 {
            let (p, q) = (px(src, i), px(row, i));
            if p >> 24 != 0 {
                put(d, i, (p & 0x00ffffff) | (q & 0xff000000));
            }
        }
    })
}

fn alpha(a: &FxArgs, x: i32, y: i32) -> i32 {
    if x < 0 || y < 0 || x >= a.src.w || y >= a.src.h {
        0
    } else {
        (px(a.src_row(y), x as usize) >> 24) as i32
    }
}
fn inner(a: &FxArgs, dst: &mut Dst) -> i32 {
    let distance = pint(a, 0, 1, 100);
    let opacity = pint(a, 1, 0, 100);
    let (sn, cs) = gfx::xform::sincos(pdbl(a, 2) * 0.017453292519943295);
    let (dx, dy) = ((cs * distance as f64) as i32, (sn * distance as f64) as i32);
    rows(a, dst, |d, s, x, y| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            let shade = (255 - alpha(a, x + i as i32 - dx, y - dy)) * opacity / 100;
            let mut out = p & 0xff000000;
            for ch in [0, 8, 16] {
                out |= ((p >> ch & 255) * (255 - shade as u32) / 255) << ch;
            }
            put(d, i, out);
        }
    })
}
fn bevel(a: &FxArgs, dst: &mut Dst) -> i32 {
    let radius = pint(a, 0, 1, 30);
    let amount = pint(a, 1, 0, 100);
    let (sn, cs) = gfx::xform::sincos(pdbl(a, 2) * 0.017453292519943295);
    let (dx, dy) = ((cs * radius as f64) as i32, (sn * radius as f64) as i32);
    rows(a, dst, |d, s, x, y| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            let light = (alpha(a, x + i as i32 + dx, y + dy) - alpha(a, x + i as i32 - dx, y - dy)) * amount / 100;
            let mut out = p & 0xff000000;
            for ch in [0, 8, 16] {
                out |= (((p >> ch & 255) as i32 + light).clamp(0, 255) as u32) << ch;
            }
            put(d, i, out);
        }
    })
}
fn dodge(a: &FxArgs, dst: &mut Dst) -> i32 {
    let amount = pint(a, 0, -100, 100) as f64 / 100.0;
    let range = pint(a, 1, 0, 3);
    rows(a, dst, |d, s, _, _| {
        for i in 0..s.len() / 4 {
            let p = px(s, i);
            let l = crate::lut::luma(p) as f64 / 255.0;
            let weight = match range {
                1 => (1.0 - l) * (1.0 - l),
                2 => 4.0 * l * (1.0 - l),
                3 => l * l,
                _ => 1.0,
            };
            let mut out = p & 0xff000000;
            for ch in [0, 8, 16] {
                let v = (p >> ch & 255) as f64;
                let q = if amount >= 0.0 {
                    v + (255.0 - v) * amount * weight
                } else {
                    v * (1.0 + amount * weight)
                };
                out |= (q.clamp(0.0, 255.0) as u32) << ch;
            }
            put(d, i, out);
        }
    })
}

const fn int(slot: u8, name: &'static str, min: f64, max: f64, def: f64) -> FxProp {
    FxProp::new(FP_INT, 0, slot, 0, name, min, max, def)
}
static BALANCE: [FxProp; 5] = [
    int(0, "Cyan / Rot", -100., 100., 0.),
    int(1, "Magenta / Grün", -100., 100., 0.),
    int(2, "Gelb / Blau", -100., 100., 0.),
    int(3, "Helligkeit", -100., 100., 0.),
    int(4, "Kontrast", -100., 100., 0.),
];
static BW: [FxProp; 3] = [
    FxProp::choice(
        0,
        "Vorgabe",
        6.,
        0.,
        &[
            "Leuchtdichte",
            "Durchschnitt",
            "Helligkeit",
            "Rotfilter",
            "Grünfilter",
            "Blaufilter",
            "Infrarot",
        ],
    ),
    int(1, "Helligkeit", -100., 100., 0.),
    int(2, "Kontrast", -100., 100., 0.),
];
static DREAM_P: [FxProp; 2] = [int(0, "Fokus", 0., 100., 50.), int(1, "Stärke", 0., 100., 50.)];
static OIL: [FxProp; 4] = [
    int(0, "Pinselgröße", 1., 20., 5.),
    int(1, "Grobheit", 3., 255., 50.),
    int(2, "Schattenstärke", 0., 100., 10.),
    int(3, "Schattendetail", 1., 10., 1.),
];
static PASTEL_P: [FxProp; 2] = [int(0, "Pastellgröße", 1., 20., 8.), int(1, "Rauheit", 0., 100., 50.)];
static FEATHER_P: [FxProp; 1] = [int(0, "Radius", 0., 100., 5.)];
static EDGE: [FxProp; 3] = [
    int(0, "Breite", 1., 30., 5.),
    int(1, "Stärke", 0., 100., 50.),
    FxProp::new(FP_ANGLE, 2, 2, 0, "Lichtrichtung", -180., 180., 45.),
];
static DODGE: [FxProp; 2] = [
    int(0, "Abwedeln / Nachbelichten", -100., 100., 20.),
    FxProp::choice(1, "Tonwerte", 3., 0., &["Alle", "Schatten", "Mitteltöne", "Lichter"]),
];
const fn desc(id: u32, name: &'static str, render: RenderFn, props: &'static [FxProp], cat: u32) -> FxDesc {
    FxDesc {
        id,
        name,
        render,
        props,
        category: cat,
        flags: if cat == FXC_ADJUST { FXF_ADJ } else { 0 },
        prepare: None,
        user: 0,
    }
}
pub static FXTAB: [FxDesc; 11] = [
    FxDesc {
        flags: FXF_WHOLE | FXF_SLOW,
        ..desc(
            CONTENT_FILL,
            "content_aware_fill",
            content_fill,
            &[int(0, "Suchgenauigkeit", 8., 64., 24.)],
            FXC_PHOTO,
        )
    },
    desc(
        PORTRAIT,
        "portrait_retouch",
        portrait,
        &[
            int(0, "Radius", 1., 12., 3.),
            int(1, "Glättung", 0., 100., 50.),
            int(2, "Kantenschutz", 1., 100., 30.),
        ],
        FXC_PHOTO,
    ),
    desc(COLOR_BALANCE, "color_balance", balance, &BALANCE, FXC_ADJUST),
    desc(BLACK_WHITE_PLUS, "black_white_plus", bw, &BW, FXC_ADJUST),
    desc(DREAM, "dream", dream, &DREAM_P, FXC_ARTISTIC),
    desc(OIL_PLUS, "oil_painting_plus", oil, &OIL, FXC_ARTISTIC),
    desc(PASTEL, "pastel", pastel, &PASTEL_P, FXC_ARTISTIC),
    desc(FEATHER, "feather", feather, &FEATHER_P, FXC_OBJECT),
    desc(INNER_SHADOW, "inner_shadow", inner, &EDGE, FXC_OBJECT),
    desc(BEVEL, "bevel", bevel, &EDGE, FXC_OBJECT),
    desc(DODGE_BURN, "dodge_burn", dodge, &DODGE, FXC_PHOTO),
];

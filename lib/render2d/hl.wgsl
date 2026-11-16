
// hl -- the look of highlighted records (G6d; docs/RENDER2D.md "Hervorheben").  lib.rs's hl_wgsl puts this behind the
// stroke and text shaders and turns their vertex shaders into these variants: the visible list's word goes through hl_take
// (bits 29 and 30: selected, hovered, as cull.wgsl's write_hl put them there), ink through hl_ink, half_width through
// hl_wider.  A record without a highlight comes out bit for bit as from the plain vertex shaders; the fragment shaders are
// the plain ones.
//
//   selected   the colour 3/4 of the way to HL_SEL (blue), alpha at least HL_ALPHA, strokes 1/4 px wider on either side
//              (a hairline's half width 0.75: still stroke.wgsl's TINY_H, the fast way of short thin strokes; half a
//              pixel more made a selected overview 1.45 to 3 times as slow)
//   hovered    the colour 7/8 of the way to HL_HOV (orange), alpha at least HL_ALPHA, strokes 1 px wider on either side
//              (hovered over selected: hovered)

var<private> hl: u32;

const HL_SELECTED: u32 = 1u;
const HL_HOVER: u32 = 2u;
const HL_SEL: u32 = 0x1E64FFu;
const HL_HOV: u32 = 0xFF8C1Au;
const HL_ALPHA: u32 = 0xB0u;

// a visible list's word: its highlight to `hl`, the word without it
fn hl_take(v: u32) -> u32 {
    hl = (v >> 29u) & 3u;
    return v & 0x9FFFFFFFu;
}

// a channel k / 256 of the way from a to b (codes)
fn hl_ch(a: u32, b: u32, k: u32) -> u32 {
    return (a * (256u - k) + b * k + 128u) >> 8u;
}

fn hl_mix(c: u32, t: u32, k: u32) -> u32 {
    let r = hl_ch((c >> 16u) & 255u, (t >> 16u) & 255u, k);
    let g = hl_ch((c >> 8u) & 255u, (t >> 8u) & 255u, k);
    let b = hl_ch(c & 255u, t & 255u, k);
    return (r << 16u) | (g << 8u) | b;
}

fn hl_ink(c: u32) -> u32 {
    if hl == 0u {
        return c;
    }
    let a = max(c >> 24u, HL_ALPHA);
    if (hl & HL_HOVER) != 0u {
        return (a << 24u) | hl_mix(c, HL_HOV, 224u);
    }
    return (a << 24u) | hl_mix(c, HL_SEL, 192u);
}

fn hl_wider() -> f32 {
    if (hl & HL_HOVER) != 0u {
        return 1.0;
    }
    if hl != 0u {
        return 0.25;
    }
    return 0.0;
}

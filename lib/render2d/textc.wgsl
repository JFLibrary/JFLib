
// textc -- what the visibility and the text shaders share about text records (G6c; docs/RENDER2D.md "Text"); lib.rs puts
// it behind block.wgsl into cull.wgsl and text.wgsl.
//
// A glyph's record (K_GLYPH): a: its text's origin (hi, lo; every glyph of a text the same bits); b: the text's map from em
// units to drawing units (or pixels, G_PX), m00 m01 m10 m11; v: its pen (em from the origin, x and y), its row of the glyph
// table, the right end of its cell along the baseline (em; the next glyph's pen); w: its quad's box from its pen (em, two
// pairs of f16: x0 y0, x1 y1), style + 1, flags | K_GLYPH << 8 | how many records back its previous glyph lies << 16 |
// how many on its next << 24 (0: none).  A bar's (K_BAR): a, b as a glyph's; v: x0, x1, y0, y1 (em from the origin); w.x
// the share of the bar its words' ink covers.
//
// Mode: a text whose em is at least T_BAR pixels shows glyphs, below that bars; the em in pixels is taken at the scale the
// CPU last decided by (view.misc.z = scale / that scale), which moves only when the scale leaves a band of HYST around it:
// zooming does not flicker between the two.

const K_GLYPH: u32 = 5u;
const K_BAR: u32 = 6u;
// a mark (no cell of its own: it draws its ink alone); a map to pixels (a style in pixels)
const G_MARK: u32 = 1u;
const G_PX: u32 = 2u;
// below this many pixels per em a text is drawn as bars
const T_BAR: f32 = 7.0;

// the em -> pixel map of a text record: the pixel offsets of one em along its x and y (y down)
fn text_map(r: Record, vw: View) -> mat2x2f {
    let a0 = to_px(vec2f(r.b.x, r.b.z), vw);
    let a1 = to_px(vec2f(r.b.y, r.b.w), vw);
    if (r.w.w & G_PX) != 0u {
        return mat2x2f(a0 * vw.rest.w, a1 * vw.rest.w);
    }
    return mat2x2f(a0, a1);
}

// glyphs (false: bars) at this map
fn text_glyphs(a: mat2x2f) -> bool {
    return length(a[1]) * view.misc.z >= T_BAR;
}

// the record's box: a glyph's quad from its pen, a bar's (em from the text's origin: x0, y0, x1, y1)
fn text_box(r: Record) -> vec4f {
    if kind_of_record(r) == K_BAR {
        return vec4f(bitcast<f32>(r.v.x), bitcast<f32>(r.v.z), bitcast<f32>(r.v.y), bitcast<f32>(r.v.w));
    }
    let pen = bitcast<vec2f>(r.v.xy);
    return vec4f(pen + unpack2x16float(r.w.x), pen + unpack2x16float(r.w.y));
}

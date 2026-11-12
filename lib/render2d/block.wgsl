// block -- what the visibility and the vertex shaders share about blocks and their inserts (G6b2; docs/RENDER2D.md
// "Blöcke"); lib.rs puts it behind common.wgsl into cull.wgsl and stroke.wgsl (both declare `view`, `styles`, `spaces`).
//
// A block's records lie once in the pool, in its block-local units (relative to its base point).  An insert is an
// instance: scene.rs uploads its transform (Inst, cull.wgsl), and each frame the instance pass computes its space from
// the camera: where the camera's centre lies in the block's units (three f32 per axis: as precise as the camera's own
// hi, lo and rest), the linear map from the block's units to pixels, the pixels per block unit, its visibility.  A space
// is a View for rel2 and to_px (space_view): every shader that places a record places a block's record through its
// insert's space with the same code, so that both owners of a pixel compute it from the same bits.
//
// An entry of the visibility (einfo, cull.wgsl) names a page, its instance + 1 (0: the page of a layer), the records of
// the page it covers (first | (count - 1) << 8 | (pack - 1) << 16: `pack` instances one after the other, each of `count`
// records from `first`: many small inserts of one block share a workgroup) and what is added to a record's place to find
// its fill edge in epx; a visible record of an insert is ITEM_BLOCK | entry << 8 | its place among the entry's records,
// any other its place in the pool.  A visible record of an insert is ITEM_BLOCK | its place in the pool, and beside it in
// vinst its instance (cull.wgsl write).

struct Space {
    // the camera's centre in the block's units: hi.x, hi.y, lo.x, lo.y; rest.xy what hi + lo leaves of it, rest.z pixels
    // per block unit (widths and patterns in units scale with the insert), rest.w block units per pixel
    center: vec4f,
    rest: vec4f,
    // the block's units to pixels (View.lin)
    lin: vec4f,
    // the insert's scale for widths and patterns (hi, lo): its uniform scale, or sqrt |det| of an uneven one
    sw: vec4f,
    // SP_* flags, the style its entities take by block (BY_BLOCK_*), its first row of the hatch table, what is added to a
    // record's place for its fill edge in epx
    u: vec4u,
}

const ITEM_BLOCK: u32 = 0x80000000u;

// an entry's records per instance, its instances
fn entry_count(ei: vec4u) -> u32 {
    return ((ei.z >> 8u) & 255u) + 1u;
}

fn entry_pack(ei: vec4u) -> u32 {
    return ((ei.z >> 16u) & 255u) + 1u;
}
const SP_VISIBLE: u32 = 1u;
const SP_MIRROR: u32 = 2u;
// Style.r1.y: what an entity of a block takes from its insert's style (scene.rs BY_BLOCK_*)
const BB_COLOR: u32 = 1u;
const BB_WIDTH: u32 = 2u;
const BB_DASH: u32 = 4u;
// Style.r1.x: the transparency in percent, or these
const T_BY_LAYER: u32 = 0xFFFFFFFFu;
const T_BY_BLOCK: u32 = 0xFFFFFFFEu;

// an insert's space as a View: rel2 and to_px with the block's units; turn.xy holds its scale for widths (sw)
fn space_view(sp: Space) -> View {
    var v = view;
    v.center = sp.center;
    v.rest = sp.rest;
    v.lin = sp.lin;
    v.turn = vec4f(sp.sw.xy, view.turn.zw);
    return v;
}

// a record's style: its own, inside an insert (blk) with what it takes by block from the insert's (ins): the colour, the
// width and its unit, the dash pattern, the transparency.  Outside a block an entity's style is its own
fn style_in(own: u32, ins: u32, blk: bool) -> Style {
    var s = styles[own];
    if blk {
        let f = s.r1.y;
        if f != 0u || s.r1.x == T_BY_BLOCK {
            let b = styles[ins];
            if (f & BB_COLOR) != 0u {
                s.color = b.color;
            }
            if (f & BB_WIDTH) != 0u {
                s.width = b.width;
                s.flags = (s.flags & ~15u) | (b.flags & 15u);
            }
            if (f & BB_DASH) != 0u {
                s.flags = (s.flags & ~0xF1F000u) | (b.flags & 0xF1F000u);
                s.kappa = b.kappa;
                s.phase0 = b.phase0;
                s.period = b.period;
                s.duty = b.duty;
                s.ons = b.ons;
                s.ends = b.ends;
            }
            if s.r1.x == T_BY_BLOCK {
                s.r1.x = b.r1.x;
            }
        }
    }
    return s;
}

// the style whose dash pattern the fragment shader reads (misc.z): the insert's where the pattern is by block
fn dash_style(own: u32, ins: u32, blk: bool) -> u32 {
    if blk && (styles[own].r1.y & BB_DASH) != 0u {
        return ins;
    }
    return own;
}

// periods per block unit of a pattern in pixels inside an insert: the camera's periods per drawing unit (three f32,
// exact) times the insert's scale (hi, lo), the product's exact pieces summed again into three f32
fn kappa_scaled(k: vec3f, s: vec2f) -> vec3f {
    let a = two_prod(k.x, s.x);
    let b = two_prod(k.x, s.y);
    let c = two_prod(k.y, s.x);
    let x = two_sum(b.x, c.x);
    let y = two_sum(a.y, x.x);
    let low = ((x.y + y.y) + (b.y + c.y)) + (k.y * s.y + k.z * s.x);
    // (a sum of three f32 to 2^-72 of it, each well below the one before: what periods() takes)
    return vec3f(a.x, y.x, low);
}

// a colour with its transparency (percent, AutoCAD's 0 to 90; T_BY_LAYER: the layer's, from lists.z) multiplied into its
// alpha, in whole numbers (0: the colour as it is)
fn ink(color: u32, t: u32, layer_t: u32) -> u32 {
    var p = t;
    if p == T_BY_LAYER {
        p = layer_t;
    }
    if p == 0u || p > 100u {
        return color;
    }
    let a = color >> 24u;
    return (color & 0xFFFFFFu) | (((a * (100u - p) + 50u) / 100u) << 24u);
}

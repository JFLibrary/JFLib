// object -- effect group "object" (Objekt): drop_shadow                    [src/fx/fx_object.s]
//
//   FXTAB            the group table
//
//   drop_shadow  colour, angle (deg, 0 = right, 90 = down), distance d (0..100), blur radius r
//                (0..100), opacity (0..100), shadow only: offset o = (rint(d cos a), rint(d sin a));
//                shadow alpha S = Gaussian blur (Kern::gauss(2 r (r + 2)), W16) of the source
//                alpha moved by o (transparent outside the image); shadow pixel = colour with
//                alpha (S opacity colour.a + 816000) / 1632000; result = source over shadow
//                (Normal blend), or the shadow alone

use alloc::vec;
use alloc::vec::Vec;

use gfx::blendmode::{bm_row, BM_NORMAL, BM_OPAC_SHIFT};
use rt::fx::cvtd;

use crate::conv::*;
use crate::*;

struct Shadow<'d, 'e, 's> {
    dst: &'d mut Dst<'e>,
    a: &'s FxArgs<'s>,
    ox: i32,
    oy: i32,
    rgb: u32,
    k: u32,
    only: bool,
    tmp: Vec<u8>,
}

impl SepIo for Shadow<'_, '_, '_> {
    /// the source moved by the offset, transparent outside  [fxo_shadow_load]
    fn load(&mut self, sep: &Sep, w16: &mut [i16], y: i32, x0: i32, x1: i32) {
        load_row(w16, sep.src, y - self.oy, x0 - self.ox, x1 - self.ox, FXL_ZERO);
    }

    /// shadow pixels, then source over shadow  [fxo_shadow_emit]
    fn emit(&mut self, sep: &Sep, y: i32, row: &[i16]) {
        let n = (sep.x1 - sep.x0) as usize;
        for i in 0..n {
            let s = (row[i * 4 + 3] as i32).clamp(0, 16320) as u32;
            let al = (s.wrapping_mul(self.k).wrapping_add(816000)) / 1632000;
            put(&mut self.tmp, i, al << 24 | self.rgb);
        }
        let d = dst_px(self.dst, y, sep.x0, n as i32);
        if self.only {
            d.copy_from_slice(&self.tmp[..n * 4]);
        } else {
            let s = &self.a.src_row(y)[sep.x0 as usize * 4..sep.x1 as usize * 4];
            bm_row(d, &self.tmp[..n * 4], s, BM_NORMAL | 255 << BM_OPAC_SHIFT);
        }
    }
}

fn shadow_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let col = a.params.0[0] as u32;
    let k = pint(a, 4, 0, 100) as u32 * (col >> 24);
    let only = pint(a, 5, 0, 1) != 0;
    let (s, c) = gfx::xform::sincos(pdbl(a, 1) * 0.017453292519943295);
    let d = pint(a, 2, 0, 100) as f64;
    let ox = cvtd(c * d);
    let oy = cvtd(s * d);
    let kern = Kern::gauss(var12(pint(a, 3, 0, 100)));
    let Some(mut sep) = Sep::new(a, &kern) else {
        return 0;
    };
    sep.custom_load = true;
    let n = (sep.x1 - sep.x0) as usize;
    let mut io = Shadow { dst, a, ox, oy, rgb: col & 0x00FF_FFFF, k, only, tmp: vec![0u8; n * 4 + 64] };
    sep_run(&sep, &mut io)
}

static P_SHADOW: [FxProp; 6] = [
    FxProp::color(0, "color", 0xFF00_0000),
    FxProp::new(FP_ANGLE, 2, 1, 0, "angle", -180.0, 180.0, 45.0),
    FxProp::new(FP_INT, 0, 2, 0, "distance", 0.0, 100.0, 10.0),
    FxProp::new(FP_INT, 0, 3, 0, "blur_radius", 0.0, 100.0, 10.0),
    FxProp::new(FP_INT, 0, 4, 0, "opacity", 0.0, 100.0, 50.0),
    FxProp::new(FP_BOOL, 0, 5, 0, "shadow_only", 0.0, 1.0, 0.0),
];

/// the group table  [fxtab_object]
pub static FXTAB: [FxDesc; 1] = [FxDesc {
    id: FX_DROP_SHADOW,
    flags: 0,
    name: "drop_shadow",
    prepare: None,
    render: shadow_render,
    props: &P_SHADOW,
    category: FXC_OBJECT,
    user: 0,
}];

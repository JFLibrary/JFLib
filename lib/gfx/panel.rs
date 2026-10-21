// Fractional UI panels: compose fill and border coverage once, avoiding a dark
// halo from drawing two antialiased shapes on top of the same boundary pixel.
use super::{Dc, Surface, FMT_A8};
use super::shapes::Shapes;
use rt::fx::div255;

impl Shapes<'_> {
    /// Rounded panel in 24.8 pixels, with an inside border. Fill and stroke are
    /// straight ARGB. Honors draw origin, clipping and opacity, on PARGB and A8.
    /// Temporary mask storage is bounded to two 64-row bands of the visible area.
    #[allow(clippy::too_many_arguments)]
    pub fn panel_fx(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, fill: u32, stroke: u32) {
        if w <= 0 || h <= 0 { return; }
        let (Some(x1), Some(y1)) = (x.checked_add(w), y.checked_add(h)) else { return; };
        let lo = |v: i32, origin: i32| (v as i64 >> 8) + origin as i64;
        let hi = |v: i32, origin: i32| ((v as i64 + 255) >> 8) + origin as i64;
        let left = lo(x, self.dc.ox).max(self.dc.clip[0].max(0) as i64);
        let top = lo(y, self.dc.oy).max(self.dc.clip[1].max(0) as i64);
        let right = hi(x1, self.dc.ox).min(self.dc.clip[2].min(self.sf.w) as i64);
        let bottom = hi(y1, self.dc.oy).min(self.dc.clip[3].min(self.sf.h) as i64);
        if left >= right || top >= bottom { return; }
        let (left, top, right, bottom) = (left as i32, top as i32, right as i32, bottom as i32);
        let r = r.clamp(0, w.min(h) / 2);
        let t = t.clamp(0, w.min(h) / 2);
        let mut colors = [[0u32; 4]; 2];
        for (out, c) in colors.iter_mut().zip([fill, stroke]) {
            out[3] = div255((c >> 24) * self.dc.opac);
            let alpha = out[3];
            for (ch, lane) in out[..3].iter_mut().enumerate() {
                *lane = div255(((c >> (ch * 8)) & 255) * alpha);
            }
        }
        let width = right - left;
        let mut outer = Surface::new_a8_packed(width, (bottom - top).min(64));
        let mut inner = Surface::new_a8_packed(width, outer.h);
        let mut mask_dc = Dc::new(width, outer.h);
        mask_dc.set_color(0xffffffff);
        let old = self.path.set_tolerance(0.01);
        for band in (top..bottom).step_by(64) {
            outer.px.fill(0);
            inner.px.fill(0);
            mask_dc.origin(self.dc.ox - left, self.dc.oy - band);
            self.path.begin(&mask_dc);
            self.path.rrect_fx(x, y, w, h, r);
            self.path.fill(self.ras, &mask_dc, &mut outer, 0);
            if t > 0 && w > 2 * t && h > 2 * t {
                self.path.begin(&mask_dc);
                self.path.rrect_fx(x + t, y + t, w - 2 * t, h - 2 * t, (r - t).max(0));
                self.path.fill(self.ras, &mask_dc, &mut inner, 0);
            }
            for row in 0..(bottom - band).min(64) as usize {
                for col in 0..width as usize {
                    let i = row * width as usize + col;
                    let coverage = outer.px[i] as u32;
                    if coverage == 0 { continue; }
                    let inside = if t == 0 { coverage } else { (inner.px[i] as u32).min(coverage) };
                    let border = coverage - inside;
                    let alpha = div255(inside * colors[0][3] + border * colors[1][3]);
                    let pixel = (band as usize + row) * self.sf.stride as usize;
                    if self.sf.fmt == FMT_A8 {
                        let d = &mut self.sf.px[pixel + left as usize + col];
                        *d = (alpha + div255(*d as u32 * (255 - alpha))).min(255) as u8;
                    } else {
                        let start = pixel + (left as usize + col) * 4;
                        for (ch, d) in self.sf.px[start..start + 4].iter_mut().enumerate() {
                            let s = div255(inside * colors[0][ch] + border * colors[1][ch]);
                            *d = (s + div255(*d as u32 * (255 - alpha))).min(255) as u8;
                        }
                    }
                }
            }
        }
        self.path.set_tolerance(old);
    }
}

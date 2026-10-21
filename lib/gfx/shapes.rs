// shapes -- anti-aliased convenience shapes (current colour)            [src/gfx/shapes.s]
//
// Integer pixels (UI): fill_rrect(x, y, w, h, r), stroke_rrect(x, y, w, h, r, t) -- only
// the corner bands are rasterized, the rows in between are plain fills.
// 24.8 fixed point: fill_circle_fx, stroke_circle_fx (ring of thickness t centred on radius
// r), fill_ellipse_fx, line_fx (round caps).

use super::path::Path;
use super::raster::Raster;
use super::{Dc, Surface, CAP_ROUND, JOIN_ROUND};

pub struct Shapes<'a> {
    pub dc: &'a mut Dc,
    pub sf: &'a mut Surface,
    pub path: &'a mut Path,
    pub ras: &'a mut Raster,
}

impl Shapes<'_> {
    /// gfx_fill_rrect(x, y, w, h, r)
    pub fn fill_rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let r = r.min(w.min(h) >> 1);
        if r <= 0 {
            self.dc.fill(self.sf, x, y, w, h);
            return;
        }
        self.path.begin(self.dc);
        self.path.rrect_fx(x << 8, y << 8, w << 8, h << 8, r << 8);
        self.band(x, y, w, r, 0);
        self.band(x, y + h - r, w, r, 0);
        let mh = h - r - r;
        if mh > 0 {
            self.dc.fill(self.sf, x, y + r, w, mh);
        }
    }

    // fill the path inside a horizontal band (clip push, fill, clip pop)
    fn band(&mut self, x: i32, y: i32, w: i32, h: i32, rule: u32) {
        self.dc.clip_push(x, y, w, h);
        self.path.fill(self.ras, self.dc, self.sf, rule);
        self.dc.clip_pop();
    }

    /// gfx_stroke_rrect(x, y, w, h, r, t): border of thickness t inside the rectangle
    pub fn stroke_rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32) {
        if w <= 0 || h <= 0 || t <= 0 {
            return;
        }
        let half = w.min(h) >> 1;
        let r = r.min(half);
        if t >= half {
            self.fill_rrect(x, y, w, h, r);
            return;
        }
        if r == 0 {
            self.dc.fill(self.sf, x, y, w, t);
            self.dc.fill(self.sf, x, y + h - t, w, t);
            self.dc.fill(self.sf, x, y + t, t, h - t - t);
            self.dc.fill(self.sf, x + w - t, y + t, t, h - t - t);
            return;
        }
        // even-odd ring: outer rrect r, inner rrect max(r - t, 0)
        self.path.begin(self.dc);
        self.path.rrect_fx(x << 8, y << 8, w << 8, h << 8, r << 8);
        let ir = (r - t).max(0);
        self.path.rrect_fx((x + t) << 8, (y + t) << 8, (w - t - t) << 8, (h - t - t) << 8, ir << 8);
        let band = r.max(t);
        self.band(x, y, w, band, 1);
        self.band(x, y + h - band, w, band, 1);
        let sh = h - band - band;
        if sh <= 0 {
            return;
        }
        self.dc.fill(self.sf, x, y + band, t, sh);
        self.dc.fill(self.sf, x + w - t, y + band, t, sh);
    }

    /// gfx_fill_circle_fx(cx, cy, r)
    pub fn fill_circle_fx(&mut self, cx: i32, cy: i32, r: i32) {
        self.fill_ellipse_fx(cx, cy, r, r);
    }

    /// gfx_fill_ellipse_fx(cx, cy, rx, ry)
    pub fn fill_ellipse_fx(&mut self, cx: i32, cy: i32, rx: i32, ry: i32) {
        self.path.begin(self.dc);
        self.path.ellipse_fx(cx, cy, rx, ry);
        self.path.fill(self.ras, self.dc, self.sf, 0);
    }

    /// gfx_stroke_circle_fx(cx, cy, r, t): ring of thickness t centred on radius r
    pub fn stroke_circle_fx(&mut self, cx: i32, cy: i32, r: i32, t: i32) {
        self.path.begin(self.dc);
        self.path.ellipse_fx(cx, cy, r, r);
        self.path.stroke(self.ras, self.dc, self.sf, t, 0);
    }

    /// gfx_line_fx(x0, y0, x1, y1, width): round-capped line
    pub fn line_fx(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, width: i32) {
        self.path.begin(self.dc);
        self.path.move_fx(x0, y0);
        self.path.line_fx(x1, y1);
        self.path.stroke(self.ras, self.dc, self.sf, width, CAP_ROUND << 4 | JOIN_ROUND);
    }
}

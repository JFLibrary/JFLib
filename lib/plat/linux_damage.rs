// linux_damage -- damage regions of the X11 / Wayland presents (linux_x11_present.rs,
// linux_wl_present.rs)
//
//   Reg                        boxes x0, y0, x1, y1 (exclusive); more than REG_CAP of them
//                              collapse into their bounding box
//   Reg::add(b)  clear()  set_full(w, h)  is_empty()  boxes()  bbox()  over()  covers(b)
//   clip_box(r, w, h)          x, y, w, h -> the box clipped to w x h (None: empty)
//   copy_boxes(dst, ds, src, ss, boxes, w, h)   the boxes' pixel rows (4 bytes a pixel) from
//                              src (rows of ss bytes) into dst (rows of ds bytes), clipped
//                              to w x h
//
// A window's buffers (X11: two MIT-SHM segments, Wayland: three wl_shm buffers) each remember
// where they differ from the latest frame (the damage of the frames presented since they were
// last brought up to date); a free buffer gets exactly these boxes, so a frame is copied once,
// straight from the caller's framebuffer, instead of into a copy of the window first.

use alloc::vec::Vec;

const REG_CAP: usize = 256;

/// a region: a list of boxes (they may overlap), or only their bounding box
#[derive(Default)]
pub(crate) struct Reg {
    b: Vec<[i32; 4]>,
    over: bool,   // more than REG_CAP boxes: bx alone
    bx: [i32; 4], // the bounding box (x0 >= x1: empty)
}

impl Reg {
    pub(crate) const fn new() -> Reg {
        Reg { b: Vec::new(), over: false, bx: [0; 4] }
    }

    pub(crate) fn clear(&mut self) {
        self.b.clear();
        self.over = false;
        self.bx = [0; 4];
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bx[0] >= self.bx[2] || self.bx[1] >= self.bx[3]
    }

    /// everything of a w x h frame
    pub(crate) fn set_full(&mut self, w: i32, h: i32) {
        self.clear();
        self.add([0, 0, w, h]);
    }

    /// add a box (empty ones are ignored; one inside a box of the list adds nothing)
    pub(crate) fn add(&mut self, n: [i32; 4]) {
        if n[0] >= n[2] || n[1] >= n[3] {
            return;
        }
        if self.is_empty() {
            self.bx = n;
        } else {
            self.bx = [self.bx[0].min(n[0]), self.bx[1].min(n[1]), self.bx[2].max(n[2]), self.bx[3].max(n[3])];
        }
        if self.over {
            return;
        }
        let inside = |a: &[i32; 4], b: &[i32; 4]| a[0] >= b[0] && a[1] >= b[1] && a[2] <= b[2] && a[3] <= b[3];
        if self.b.iter().any(|b| inside(&n, b)) {
            return;
        }
        self.b.retain(|b| !inside(b, &n));
        if self.b.len() >= REG_CAP {
            self.over = true;
            self.b.clear();
            return;
        }
        self.b.push(n);
    }

    /// the boxes (the bounding box alone after an overflow; none when empty)
    pub(crate) fn boxes(&self) -> &[[i32; 4]] {
        if self.is_empty() {
            &[]
        } else if self.over {
            core::slice::from_ref(&self.bx)
        } else {
            &self.b
        }
    }

    pub(crate) fn bbox(&self) -> [i32; 4] {
        self.bx
    }

    /// more boxes than REG_CAP were added: the region is only the bounding box
    pub(crate) fn over(&self) -> bool {
        self.over
    }

    /// the box lies inside one box of the region
    pub(crate) fn covers(&self, n: &[i32; 4]) -> bool {
        self.boxes().iter().any(|b| n[0] >= b[0] && n[1] >= b[1] && n[2] <= b[2] && n[3] <= b[3])
    }
}

/// x, y, w, h clipped to w x h -> the box x0, y0, x1, y1 (None: empty)
pub(crate) fn clip_box(r: &[i32; 4], w: i32, h: i32) -> Option<[i32; 4]> {
    let b = [r[0].max(0), r[1].max(0), r[0].wrapping_add(r[2]).min(w), r[1].wrapping_add(r[3]).min(h)];
    if b[0] < b[2] && b[1] < b[3] { Some(b) } else { None }
}

/// copy the pixel rows of the boxes (clipped to w x h) from src (rows of ss bytes) into dst
/// (rows of ds bytes); rows outside either slice are skipped
pub(crate) fn copy_boxes(dst: &mut [u8], ds: usize, src: &[u8], ss: usize, boxes: &[[i32; 4]], w: i32, h: i32) {
    for b in boxes {
        let (x0, y0, x1, y1) = (b[0].max(0), b[1].max(0), b[2].min(w), b[3].min(h));
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let (o, n) = (x0 as usize * 4, (x1 - x0) as usize * 4);
        for y in y0 as usize..y1 as usize {
            let (s, d) = (y * ss + o, y * ds + o);
            if let (Some(sr), Some(dr)) = (src.get(s..s + n), dst.get_mut(d..d + n)) {
                dr.copy_from_slice(sr);
            }
        }
    }
}

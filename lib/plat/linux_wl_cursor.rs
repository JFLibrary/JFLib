// linux_wl_cursor -- pointer cursors: wp_cursor_shape_v1 when the compositor has it,
// otherwise a cursor surface with built-in images (WSLg has no cursor-shape)
//                                                                 [src/plat/linux/wl_cursor.s]
//
//   Wl::win_set_cursor(w, shape)   CUR_* of window w (CUR_NONE hides the pointer); shown
//                                  while the pointer is over that window
//   Wl::cursor_init()              create the cursor surface (after the window)
//   Wl::cursor_apply()             send the shape of the pointer's window (enter / change,
//                                  also after a scale change)
//   image(shape) / render(dst, image, S)   (tests)
//   Wl::cursor_create(px, w, h, hot_x, hot_y, dpi) -> CUR_USER0 + k / -1   a custom cursor:
//        the premultiplied pixels in a wl_buffer of their own, buffer scale dpi / 96
//        (cursor_dpi rounds the window's dpi up to a multiple of 96); shown on a cursor
//        surface of its own, also when the compositor has wp_cursor_shape_v1
//   Wl::cursor_destroy(id)   cursor_dpi(dpi)
//
// The images are 2-colour bitmaps below (X = black outline, o = white fill, dot =
// transparent), scaled by S = ceil(scale) with nearest neighbour and uploaded once per S into
// one small shm pool (ARGB8888: every pixel is either opaque or fully transparent, so
// premultiplication does not matter).

use rt::os::{syscall6, SYS_MUNMAP};

use super::proto::*;
use super::wire::A;
use super::Wl;
use crate::{CUR_ARROW, CUR_COUNT, CUR_MAX_PX, CUR_NONE, CUR_USER0, CUR_USER_MAX, PW_MAX};

const CUR_IMAGES: usize = 10; // CUR_ARROW .. CUR_SIZE_NESW
const SHM_ARGB8888: u32 = 0;

/// a built-in cursor image
pub(crate) struct Art {
    pub w: u8,
    pub h: u8,
    pub hx: u8,
    pub hy: u8,
    pub px: &'static [u8], // w * h characters
}

/// a custom cursor (buf 0 = free slot)
#[derive(Clone, Copy, Default)]
pub(crate) struct Cw {
    buf: u32,
    w: u32, // size in surface units (px / S)
    h: u32,
    hx: u32, // hot spot in surface units
    hy: u32,
    s: u32, // buffer scale
}

#[derive(Default)]
pub(crate) struct Cursor {
    bufs: [u32; CUR_IMAGES],
    pub shape: u32, // the pointer's shape (g_wl_cur_shape)
    surf: u32,
    s: u32,
    csurf: u32, // surface of the custom cursors (0 = not made yet)
    cust: [Cw; CUR_USER_MAX as usize],
}

/// the image of a CUR_* shape  [wl_cursor_image]
pub(crate) fn image(shape: u32) -> Option<&'static Art> {
    CUR_TABLE.get(shape as usize).copied()
}

/// expand an image into ARGB pixels (S x S blocks, rows of width * S pixels, no padding)
/// [wl_cursor_render]
pub(crate) fn render(dst: &mut [u32], a: &Art, s: u32) {
    let (w, h, s) = (a.w as usize, a.h as usize, s as usize);
    let row = w * s;
    for y in 0..h {
        for dy in 0..s {
            let o = (y * s + dy) * row;
            for x in 0..w {
                let c = match a.px[y * w + x] {
                    b'o' => 0xFFFF_FFFF,
                    b'X' => 0xFF00_0000,
                    _ => 0,
                };
                let d = o + x * s;
                if let Some(t) = dst.get_mut(d..d + s) {
                    t.fill(c);
                }
            }
        }
    }
}

/// wp_cursor_shape_device_v1 shape of a CUR_* (PN1: tablet tools)  [wl_cursor_shape_of]
pub(crate) fn shape_of(shape: u32) -> u32 {
    if (shape as usize) < CUR_IMAGES { SHAPE_MAP[shape as usize] as u32 } else { 1 }
}

/// the dpi rounded up to a multiple of 96 (integer buffer scale)  [wl_plat_cursor_dpi]
pub(crate) fn cursor_dpi(dpi: i32) -> i32 {
    let s = (dpi as u32).wrapping_add(95) / 96;
    (s.max(1) * 96) as i32
}

impl Wl {
    /// CUR_* (or a custom cursor) of window w  [wl_plat_win_set_cursor]
    pub(crate) fn win_set_cursor(&mut self, w: i32, shape: u32) {
        if shape >= CUR_COUNT && shape.wrapping_sub(CUR_USER0) >= CUR_USER_MAX {
            return;
        }
        if w as u32 as usize >= PW_MAX || self.win[w as usize].cursor == shape {
            return;
        }
        self.win[w as usize].cursor = shape;
        if w == self.inp.ptr_win {
            self.cursor_apply();
        }
    }

    /// the fallback cursor surface  [wl_cursor_init]
    pub(crate) fn cursor_init(&mut self) {
        if self.inp.csd != 0 || self.cur.surf != 0 {
            return;
        }
        self.cur.surf = self.w.send(self.g.comp, &COMPOSITOR_CREATE_SURFACE, &[A::N(&IFACE_CURSOR_SURFACE)]);
    }

    /// show the cursor of the window the pointer is over  [wl_cursor_apply]
    pub(crate) fn cursor_apply(&mut self) {
        let ptr = self.inp.ptr;
        if ptr == 0 || !self.inp.ptr_in || self.inp.ptr_win < 0 {
            return;
        }
        let wi = self.inp.ptr_win as usize & (PW_MAX - 1);
        let mut shape = self.win[wi].cursor;
        self.cur.shape = shape;
        let serial = self.inp.enter_serial;
        let k = shape.wrapping_sub(CUR_USER0);
        if k < CUR_USER_MAX {
            let c = self.cur.cust[k as usize];
            if c.buf == 0 {
                shape = CUR_ARROW;
            } else {
                if self.cur.csurf == 0 {
                    self.cur.csurf =
                        self.w.send(self.g.comp, &COMPOSITOR_CREATE_SURFACE, &[A::N(&IFACE_CURSOR_SURFACE)]);
                    if self.cur.csurf == 0 {
                        return;
                    }
                }
                let cs = self.cur.csurf;
                self.w.send(ptr, &POINTER_SET_CURSOR, &[A::U(serial), A::O(cs), A::I(c.hx as i32), A::I(c.hy as i32)]);
                self.w.send(cs, &SURFACE_ATTACH, &[A::O(c.buf), A::I(0), A::I(0)]);
                self.w.send(cs, &SURFACE_SET_BUFFER_SCALE, &[A::I(c.s as i32)]);
                self.w.send(cs, &SURFACE_DAMAGE_BUFFER,
                            &[A::I(0), A::I(0), A::I(c.w.wrapping_mul(c.s) as i32), A::I(c.h.wrapping_mul(c.s) as i32)]);
                self.w.send(cs, &SURFACE_COMMIT, &[]);
                self.w.flush();
                return;
            }
        } else if shape as usize >= CUR_IMAGES {
            if shape == CUR_NONE {
                // CUR_NONE: no cursor surface
                self.w.send(ptr, &POINTER_SET_CURSOR, &[A::U(serial), A::O(0), A::I(0), A::I(0)]);
                self.w.flush();
                return;
            }
            shape = CUR_ARROW; // unknown (a destroyed custom cursor)
        }
        if self.inp.csd != 0 {
            let csd = self.inp.csd;
            self.w.send(csd, &CSD_SET_SHAPE, &[A::U(serial), A::U(SHAPE_MAP[shape as usize] as u32)]);
            self.w.flush();
            return;
        }
        if self.cur.surf == 0 {
            return;
        }
        let s = (self.win[wi].s120 as u32).wrapping_add(119) / 120;
        if s != self.cur.s && !self.cursor_build(s) {
            return;
        }
        let a = CUR_TABLE[shape as usize];
        let cs = self.cur.surf;
        self.w.send(ptr, &POINTER_SET_CURSOR, &[A::U(serial), A::O(cs), A::I(a.hx as i32), A::I(a.hy as i32)]);
        self.w.send(cs, &SURFACE_ATTACH, &[A::O(self.cur.bufs[shape as usize]), A::I(0), A::I(0)]);
        self.w.send(cs, &SURFACE_SET_BUFFER_SCALE, &[A::I(s as i32)]);
        self.w.send(cs, &SURFACE_DAMAGE_BUFFER,
                    &[A::I(0), A::I(0), A::I((a.w as u32 * s) as i32), A::I((a.h as u32 * s) as i32)]);
        self.w.send(cs, &SURFACE_COMMIT, &[]);
        self.w.flush();
    }

    /// render every image at scale S into new buffers -> ok  [cursor_build]
    fn cursor_build(&mut self, s: u32) -> bool {
        let total: usize = CUR_TABLE.iter().map(|a| a.w as usize * a.h as usize * (s * s) as usize * 4).sum();
        let Some((pool, map)) = self.shm_pool_new(total) else { return false };
        let mut nb = [0u32; CUR_IMAGES];
        let mut off = 0usize;
        for (i, a) in CUR_TABLE.iter().enumerate() {
            let n = a.w as usize * a.h as usize * (s * s) as usize;
            // SAFETY: the mapping holds `total` bytes; this image's pixels are [off, off + n * 4).
            let dst = unsafe { core::slice::from_raw_parts_mut((map + off) as *mut u32, n) };
            render(dst, a, s);
            let (pw, ph) = (a.w as u32 * s, a.h as u32 * s);
            nb[i] = self.w.send(pool, &SHM_POOL_CREATE_BUFFER,
                                &[A::N(&IFACE_CURSOR_BUFFER), A::I(off as i32), A::I(pw as i32), A::I(ph as i32),
                                  A::I((pw * 4) as i32), A::U(SHM_ARGB8888)]);
            off += n * 4;
        }
        self.w.send(pool, &SHM_POOL_DESTROY, &[]);
        // SAFETY: our mapping; the compositor has its own.
        unsafe { syscall6(SYS_MUNMAP, map, total, 0, 0, 0, 0) };
        for i in 0..CUR_IMAGES {
            let old = self.cur.bufs[i];
            self.cur.bufs[i] = nb[i];
            if old != 0 {
                self.w.send(old, &BUFFER_DESTROY, &[]);
            }
        }
        self.cur.s = s;
        true
    }

    /// a custom cursor from premultiplied ARGB pixels (rows of w * 4 bytes, = wl_shm
    /// ARGB8888); buffer scale S = dpi / 96 (1 when w or h is no multiple of it)
    /// -> CUR_USER0 + k / -1  [wl_plat_cursor_create]
    pub(crate) fn cursor_create(&mut self, px: &[u8], w: i32, h: i32, hot_x: i32, hot_y: i32, dpi: i32) -> i32 {
        if (w - 1) as u32 >= CUR_MAX_PX as u32 || (h - 1) as u32 >= CUR_MAX_PX as u32 || self.g.shm == 0 {
            return -1;
        }
        let (w, h) = (w as u32, h as u32);
        let mut s = (dpi as u32 / 96).clamp(1, 8);
        if w % s != 0 || h % s != 0 {
            s = 1;
        }
        let Some(k) = self.cur.cust.iter().position(|c| c.buf == 0) else { return -1 };
        let bytes = (w * h * 4) as usize;
        let Some((pool, map)) = self.shm_pool_new(bytes) else { return -1 };
        let n = bytes.min(px.len());
        // SAFETY: the mapping holds `bytes` bytes; n <= bytes.
        unsafe { core::ptr::copy_nonoverlapping(px.as_ptr(), map as *mut u8, n) };
        let buf = self.w.send(pool, &SHM_POOL_CREATE_BUFFER,
                              &[A::N(&IFACE_CURSOR_BUFFER), A::I(0), A::I(w as i32), A::I(h as i32), A::I((w * 4) as i32),
                                A::U(SHM_ARGB8888)]);
        self.w.send(pool, &SHM_POOL_DESTROY, &[]);
        // SAFETY: our mapping; the compositor has its own.
        unsafe { syscall6(SYS_MUNMAP, map, bytes, 0, 0, 0, 0) };
        if buf == 0 {
            return -1;
        }
        self.cur.cust[k] = Cw { buf, w: w / s, h: h / s, hx: (hot_x as u32 & 0xFFFF) / s, hy: (hot_y as u32 & 0xFFFF) / s, s };
        (CUR_USER0 + k as u32) as i32
    }

    /// free a custom cursor; windows showing it forget their shape (the pointer's window
    /// shows the arrow until the next set)  [wl_plat_cursor_destroy]
    pub(crate) fn cursor_destroy(&mut self, id: i32) {
        let k = (id as u32).wrapping_sub(CUR_USER0);
        if k >= CUR_USER_MAX {
            return;
        }
        let buf = self.cur.cust[k as usize].buf;
        if buf == 0 {
            return;
        }
        self.cur.cust[k as usize].buf = 0;
        let mut apply = false; // the pointer's window showed it
        for i in 0..PW_MAX {
            if self.win[i].cursor == id as u32 {
                self.win[i].cursor = u32::MAX;
                if i as i32 == self.inp.ptr_win {
                    apply = true;
                }
            }
        }
        if apply {
            self.cursor_apply();
        }
        self.w.send(buf, &BUFFER_DESTROY, &[]);
        self.w.flush();
    }
}

// CUR_* -> wp_cursor_shape_device_v1.shape
static SHAPE_MAP: [u8; CUR_IMAGES] = [1, 9, 4, 8, 13, 6, 27, 26, 29, 28];

static CUR_TABLE: [&Art; CUR_IMAGES] =
    [&CUR_ARROW_A, &CUR_IBEAM_A, &CUR_HAND_A, &CUR_CROSS_A, &CUR_MOVE_A, &CUR_WAIT_A, &CUR_SIZE_NS_A, &CUR_SIZE_WE_A,
     &CUR_SIZE_NWSE_A, &CUR_SIZE_NESW_A];

static CUR_ARROW_A: Art = Art {
    w: 12,
    h: 19,
    hx: 0,
    hy: 0,
    px: concat!(
        "X...........", "XX..........", "XoX.........", "XooX........", "XoooX.......", "XooooX......",
        "XoooooX.....", "XooooooX....", "XoooooooX...", "XooooooooX..", "XoooooooooX.", "XooooooXXXXX",
        "XoooXooX....", "XooXXooX....", "XoX..XooX...", "XX...XooX...", "X.....XooX..", "......XooX..",
        ".......XX..."
    )
    .as_bytes(),
};

static CUR_IBEAM_A: Art = Art {
    w: 7,
    h: 18,
    hx: 3,
    hy: 9,
    px: concat!(
        "ooo.ooo", "oXXoXXo", "oooXooo", "..oXo..", "..oXo..", "..oXo..", "..oXo..", "..oXo..", "..oXo..",
        "..oXo..", "..oXo..", "..oXo..", "..oXo..", "..oXo..", "..oXo..", "oooXooo", "oXXoXXo", "ooo.ooo"
    )
    .as_bytes(),
};

static CUR_HAND_A: Art = Art {
    w: 17,
    h: 22,
    hx: 5,
    hy: 0,
    px: concat!(
        ".....XX..........", "....XooX.........", "....XooX.........", "....XooX.........",
        "....XooX.........", "....XooXXX.......", "....XooXooXXX....", "....XooXooXooXX..",
        "....XooXooXooXooX", "XX..XooXooXooXooX", "XooXXoooooooooooX", "XoooXoooooooooooX",
        ".XooooooooooooooX", "..XoooooooooooooX", "..XoooooooooooooX", "...XooooooooooooX",
        "...XoooooooooooX.", "....XoooooooooX..", "....XoooooooooX..", ".....XooooooooX..",
        ".....XooooooooX..", ".....XXXXXXXXXX.."
    )
    .as_bytes(),
};

static CUR_CROSS_A: Art = Art {
    w: 17,
    h: 17,
    hx: 8,
    hy: 8,
    px: concat!(
        ".......ooo.......", ".......oXo.......", ".......oXo.......", ".......oXo.......",
        ".......oXo.......", ".......oXo.......", ".......oXo.......", "ooooooooXoooooooo",
        "oXXXXXXXXXXXXXXXo", "ooooooooXoooooooo", ".......oXo.......", ".......oXo.......",
        ".......oXo.......", ".......oXo.......", ".......oXo.......", ".......oXo.......",
        ".......ooo......."
    )
    .as_bytes(),
};

static CUR_MOVE_A: Art = Art {
    w: 19,
    h: 19,
    hx: 9,
    hy: 9,
    px: concat!(
        ".........X.........", "........XoX........", ".......XoooX.......", "......XoooooX......",
        "......XXXoXXX......", "........XoX........", "...X....XoX....X...", "..XX....XoX....XX..",
        ".XoXXXXXXoXXXXXXoX.", "XoooooooooooooooooX", ".XoXXXXXXoXXXXXXoX.", "..XX....XoX....XX..",
        "...X....XoX....X...", "........XoX........", "......XXXoXXX......", "......XoooooX......",
        ".......XoooX.......", "........XoX........", ".........X........."
    )
    .as_bytes(),
};

static CUR_WAIT_A: Art = Art {
    w: 13,
    h: 17,
    hx: 6,
    hy: 8,
    px: concat!(
        "XXXXXXXXXXXXX", "XoooooooooooX", "XXoooooooooXX", ".XoXXXXXXXoX.", "..XoXXXXXoX..", "...XoXXXoX...",
        "....XoXoX....", ".....XoX.....", ".....XoX.....", "....XoooX....", "...XoooooX...", "..XoooXoooX..",
        ".XoooXXXoooX.", "XXooXXXXXooXX", "XoXXXXXXXXXoX", "XoooooooooooX", "XXXXXXXXXXXXX"
    )
    .as_bytes(),
};

static CUR_SIZE_NS_A: Art = Art {
    w: 9,
    h: 21,
    hx: 4,
    hy: 10,
    px: concat!(
        "....X....", "...XoX...", "..XoooX..", ".XoooooX.", "XXXXoXXXX", "...XoX...", "...XoX...", "...XoX...",
        "...XoX...", "...XoX...", "...XoX...", "...XoX...", "...XoX...", "...XoX...", "...XoX...", "...XoX...",
        "XXXXoXXXX", ".XoooooX.", "..XoooX..", "...XoX...", "....X...."
    )
    .as_bytes(),
};

static CUR_SIZE_WE_A: Art = Art {
    w: 21,
    h: 9,
    hx: 10,
    hy: 4,
    px: concat!(
        "....X...........X....", "...XX...........XX...", "..XoX...........XoX..", ".XooXXXXXXXXXXXXXooX.",
        "XoooooooooooooooooooX", ".XooXXXXXXXXXXXXXooX.", "..XoX...........XoX..", "...XX...........XX...",
        "....X...........X...."
    )
    .as_bytes(),
};

static CUR_SIZE_NWSE_A: Art = Art {
    w: 15,
    h: 15,
    hx: 7,
    hy: 7,
    px: concat!(
        "XXXXXXX........", "XoooooX........", "XooooX.........", "XoooX..........", "XooXoX.........",
        "XoX.XoX........", "XX...XoX.......", "......XoX......", ".......XoX...XX", "........XoX.XoX",
        ".........XoXooX", "..........XoooX", ".........XooooX", "........XoooooX", "........XXXXXXX"
    )
    .as_bytes(),
};

static CUR_SIZE_NESW_A: Art = Art {
    w: 15,
    h: 15,
    hx: 7,
    hy: 7,
    px: concat!(
        "........XXXXXXX", "........XoooooX", ".........XooooX", "..........XoooX", ".........XoXooX",
        "........XoX.XoX", ".......XoX...XX", "......XoX......", "XX...XoX.......", "XoX.XoX........",
        "XooXoX.........", "XoooX..........", "XooooX.........", "XoooooX........", "XXXXXXX........"
    )
    .as_bytes(),
};

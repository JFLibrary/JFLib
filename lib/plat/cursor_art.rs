// cursor_art -- (E3) drawn cursors for the shapes a system has no cursor of its own for: grab,
// grabbing, zoom-in, zoom-out, cell, vertical-text, alias, copy and context-menu.  Windows and
// X11 make them from these bitmaps (instead of cursor_fallback's shape), Wayland without
// cursor-shape shows them on its own cursor surface.
//
//   Art { w, h, hx, hy, px }          a 2-colour bitmap (X = outline, o = fill, dot =
//                                     transparent; light: a black outline around white, as
//                                     Windows' cursors) at the size of a 32 px cursor (96 dpi),
//                                     and its hot spot
//   drawn(shape) -> Option<&Art>      the bitmap of a CUR_X0 .. CUR_X_END shape that has one
//   drawn_index(shape) -> Option<k>   its place in DRAWN (0 .. DRAWN_N - 1)
//   pixels(art, num, den, dark) -> Image   the bitmap at num / den of its size (box filter: whole
//                                     factors stay sharp, others blend), premultiplied
//                                     0xAARRGGBB pixels; dark: white outline around black, as
//                                     the cursor themes of GNOME (Adwaita) and KDE (Breeze)
//
// Hot spots as the common themes have them: the middle of a hand, of the cross and of the
// vertical I-beam, the centre of the magnifier's glass, the tip of the arrow with a badge.

use alloc::vec::Vec;

use crate::{CUR_ALIAS, CUR_CELL, CUR_CONTEXT_MENU, CUR_COPY, CUR_GRAB, CUR_GRABBING, CUR_VTEXT, CUR_ZOOM_IN, CUR_ZOOM_OUT};

/// a cursor bitmap: w x h characters (X outline, o fill, dot transparent), hot spot hx, hy
pub(crate) struct Art {
    pub w: u8,
    pub h: u8,
    pub hx: u8,
    pub hy: u8,
    pub px: &'static [u8], // w * h characters
}

/// the shapes with a bitmap, in the order of DRAWN
pub(crate) const DRAWN_SHAPES: [u32; DRAWN_N] =
    [CUR_GRAB, CUR_GRABBING, CUR_ZOOM_IN, CUR_ZOOM_OUT, CUR_CELL, CUR_VTEXT, CUR_ALIAS, CUR_COPY, CUR_CONTEXT_MENU];
pub(crate) const DRAWN_N: usize = 9;
pub(crate) static DRAWN: [&Art; DRAWN_N] = [
    &CUR_GRAB_A,
    &CUR_GRABBING_A,
    &CUR_ZOOM_IN_A,
    &CUR_ZOOM_OUT_A,
    &CUR_CELL_A,
    &CUR_VTEXT_A,
    &CUR_ALIAS_A,
    &CUR_COPY_A,
    &CUR_CONTEXT_MENU_A,
];

/// the place of shape in DRAWN
pub(crate) fn drawn_index(shape: u32) -> Option<usize> {
    DRAWN_SHAPES.iter().position(|&s| s == shape)
}

/// the bitmap of shape (None: the system's cursor or cursor_fallback's)
#[allow(dead_code)] // (Wayland and X11 / Windows use the one or the other)
pub(crate) fn drawn(shape: u32) -> Option<&'static Art> {
    drawn_index(shape).map(|k| DRAWN[k])
}

/// a cursor image: premultiplied 0xAARRGGBB pixels (rows of w * 4 bytes, little endian) and its
/// hot spot
pub(crate) struct Image {
    pub px: Vec<u8>,
    pub w: i32,
    pub h: i32,
    pub hx: i32,
    pub hy: i32,
}

/// the bitmap at num / den of its size (each side rounded up, at most CUR_MAX_PX), averaged
/// over the bitmap's pixels each output pixel covers; light: white fill, black outline; dark:
/// black fill, white outline
#[allow(dead_code)]
pub(crate) fn pixels(a: &Art, num: u32, den: u32, dark: bool) -> Image {
    let (num, den) = (num.max(1), den.max(1));
    let (w, h) = (a.w as u32, a.h as u32);
    let cap = crate::CUR_MAX_PX as u32;
    let dw = (w * num).div_ceil(den).clamp(1, cap);
    let dh = (h * num).div_ceil(den).clamp(1, cap);
    let (fill, line) = if dark { (0u32, 255u32) } else { (255, 0) };
    let area = den * den;
    let mut px = Vec::with_capacity((dw * dh * 4) as usize);
    for y in 0..dh {
        // the bitmap's rows this output row covers: [y * den, (y + 1) * den) in units of 1 / num
        let (y0, y1) = (y * den, (y + 1) * den);
        let (sy0, sy1) = (y0 / num, ((y1 - 1) / num + 1).min(h));
        for x in 0..dw {
            let (x0, x1) = (x * den, (x + 1) * den);
            let (sx0, sx1) = (x0 / num, ((x1 - 1) / num + 1).min(w));
            let (mut alpha, mut grey) = (0u32, 0u32);
            for sy in sy0..sy1 {
                let oy = ((sy + 1) * num).min(y1) - (sy * num).max(y0);
                for sx in sx0..sx1 {
                    let c = match a.px[(sy * w + sx) as usize] {
                        b'o' => fill,
                        b'X' => line,
                        _ => continue,
                    };
                    let k = ((sx + 1) * num).min(x1) - (sx * num).max(x0);
                    alpha += 255 * k * oy;
                    grey += c * k * oy;
                }
            }
            let (al, g) = ((alpha + area / 2) / area, (grey + area / 2) / area);
            let v = al << 24 | g << 16 | g << 8 | g;
            px.extend_from_slice(&v.to_le_bytes());
        }
    }
    // the hot spot: the middle of its bitmap pixel, scaled
    let hx = (((2 * a.hx as u32 + 1) * num) / (2 * den)).min(dw - 1);
    let hy = (((2 * a.hy as u32 + 1) * num) / (2 * den)).min(dh - 1);
    Image { px, w: dw as i32, h: dh as i32, hx: hx as i32, hy: hy as i32 }
}

static CUR_GRAB_A: Art = Art {
    w: 16,
    h: 17,
    hx: 8,
    hy: 8,
    px: concat!(
        ".......XX.......", "...XX.XooXXX....", "..XooXXooXooX...", "..XooXXooXooX.X.", "...XooXooXooXXoX",
        "...XooXooXooXooX", ".XX.XoooooooXooX", "XooXXooooooooooX", "XoooXoooooooooX.", ".XooooooooooooX.",
        "..XoooooooooooX.", "..XooooooooooX..", "...XoooooooooX..", "....XooooooooX..", ".....XooooooX...",
        ".....XooooooX...", ".....XXXXXXXX..."
    )
    .as_bytes(),
};

static CUR_GRABBING_A: Art = Art {
    w: 15,
    h: 14,
    hx: 7,
    hy: 7,
    px: concat!(
        "...XX.XX.XX....", "..XooXooXooXX..", "..XooXooXooXoX.", ".XXooooooooXoX.", "XooXoooooooooX.",
        "XoooooooooooooX", ".XooooooooooooX", "..XoooooooooooX", "..XooooooooooX.", "...XoooooooooX.",
        "....XooooooooX.", "....XooooooX...", "....XooooooX...", "....XXXXXXXX..."
    )
    .as_bytes(),
};

static CUR_ZOOM_IN_A: Art = Art {
    w: 18,
    h: 18,
    hx: 6,
    hy: 6,
    px: concat!(
        "...XXXXXXX........", "..XXXXXXXXX.......", ".XXXoooooXXX......", "XXXoooooooXXX.....", "XXooooXooooXX.....",
        "XXooooXooooXX.....", "XXooXXXXXooXX.....", "XXooooXooooXX.....", "XXooooXooooXX.....", "XXXoooooooXXX.....",
        ".XXXoooooXXXXX....", "..XXXXXXXXXooXX...", "...XXXXXXXXoooXX..", "..........XXoooXX.", "...........XXoooXX",
        "............XXoooX", ".............XXooX", "..............XXXX"
    )
    .as_bytes(),
};

static CUR_ZOOM_OUT_A: Art = Art {
    w: 18,
    h: 18,
    hx: 6,
    hy: 6,
    px: concat!(
        "...XXXXXXX........", "..XXXXXXXXX.......", ".XXXoooooXXX......", "XXXoooooooXXX.....", "XXoooooooooXX.....",
        "XXoooooooooXX.....", "XXooXXXXXooXX.....", "XXoooooooooXX.....", "XXoooooooooXX.....", "XXXoooooooXXX.....",
        ".XXXoooooXXXXX....", "..XXXXXXXXXooXX...", "...XXXXXXXXoooXX..", "..........XXoooXX.", "...........XXoooXX",
        "............XXoooX", ".............XXooX", "..............XXXX"
    )
    .as_bytes(),
};

static CUR_CELL_A: Art = Art {
    w: 17,
    h: 17,
    hx: 8,
    hy: 8,
    px: concat!(
        ".....XXXXXXX.....", ".....XoooooX.....", ".....XoooooX.....", ".....XoooooX.....", ".....XoooooX.....",
        "XXXXXXoooooXXXXXX", "XoooooooooooooooX", "XoooooooooooooooX", "XoooooooooooooooX", "XoooooooooooooooX",
        "XoooooooooooooooX", "XXXXXXoooooXXXXXX", ".....XoooooX.....", ".....XoooooX.....", ".....XoooooX.....",
        ".....XoooooX.....", ".....XXXXXXX....."
    )
    .as_bytes(),
};

static CUR_VTEXT_A: Art = Art {
    w: 18,
    h: 7,
    hx: 9,
    hy: 3,
    px: concat!(
        "ooo............ooo", "oXo............oXo", "oXooooooooooooooXo", ".oXXXXXXXXXXXXXXo.", "oXooooooooooooooXo",
        "oXo............oXo", "ooo............ooo"
    )
    .as_bytes(),
};

static CUR_ALIAS_A: Art = Art {
    w: 18,
    h: 20,
    hx: 0,
    hy: 0,
    px: concat!(
        "X.................", "XX................", "XoX...............", "XooX..............", "XoooX.............",
        "XooooX............", "XoooooX...........", "XooooooX..........", "XoooooooX.........", "XooooooooX........",
        "XoooooooooX.......", "XooooooXXXXXXXXXXX", "XoooXooX.XoooooooX", "XooXXooX.XooXXXXoX", "XoX..XooXXoooooXoX",
        "XX...XooXXooooXXoX", "X.....XooXoooXoXoX", "......XooXooXooooX", ".......XXXoooooooX", ".........XXXXXXXXX"
    )
    .as_bytes(),
};

static CUR_COPY_A: Art = Art {
    w: 18,
    h: 20,
    hx: 0,
    hy: 0,
    px: concat!(
        "X.................", "XX................", "XoX...............", "XooX..............", "XoooX.............",
        "XooooX............", "XoooooX...........", "XooooooX..........", "XoooooooX.........", "XooooooooX........",
        "XoooooooooX.......", "XooooooXXXXXXXXXXX", "XoooXooX.XoooooooX", "XooXXooX.XoooXoooX", "XoX..XooXXoooXoooX",
        "XX...XooXXoXXXXXoX", "X.....XooXoooXoooX", "......XooXoooXoooX", ".......XXXoooooooX", ".........XXXXXXXXX"
    )
    .as_bytes(),
};

static CUR_CONTEXT_MENU_A: Art = Art {
    w: 20,
    h: 19,
    hx: 0,
    hy: 0,
    px: concat!(
        "X...................", "XX........XXXXXXXXXX", "XoX.......XooooooooX", "XooX......XoXXXXXXoX",
        "XoooX.....XooooooooX", "XooooX....XoXXXXXXoX", "XoooooX...XooooooooX", "XooooooX..XoXXXXXXoX",
        "XoooooooX.XooooooooX", "XooooooooXXXXXXXXXXX", "XoooooooooX.........", "XooooooXXXXX........",
        "XoooXooX............", "XooXXooX............", "XoX..XooX...........", "XX...XooX...........",
        "X.....XooX..........", "......XooX..........", ".......XX..........."
    )
    .as_bytes(),
};

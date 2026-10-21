// t_cursor -- (plat-win) plattest --test cursor: custom cursors (plat::cursor_*): a 32 x 32
// premultiplied disc, set on window 0, a second one, slot reuse after destroy, refused sizes,
// the dpi rule of the backend (needs the display; the window is never shown)
//                                                  [apps/plattest/src/main.s t_cursor]

use alloc::format;
use alloc::vec;

use plat::{CUR_ARROW, CUR_CROSS, CUR_MAX_PX, CUR_USER0, CUR_USER_MAX, cursor_create, cursor_destroy, cursor_dpi, win_set_cursor};

use crate::T;

/// the id is a custom cursor
fn custom(id: i32) -> bool {
    (id as u32).wrapping_sub(CUR_USER0) < CUR_USER_MAX
}

pub fn t_cursor(t: &mut T) {
    // the image: a red disc (alpha 255 inside radius 12, 128 on the ring 12..14)
    let mut px = vec![0u8; 32 * 32 * 4];
    for y in 0..32i32 {
        for x in 0..32i32 {
            let d = (x - 16) * (x - 16) + (y - 16) * (y - 16);
            let c: u32 = if d >= 14 * 14 {
                0
            } else if d >= 12 * 12 {
                0x8080_0000 // premultiplied red 50 %
            } else {
                0xFFFF_0000
            };
            let o = ((y * 32 + x) * 4) as usize;
            px[o..o + 4].copy_from_slice(&c.to_le_bytes());
        }
    }
    // create
    let a = cursor_create(&px, 32, 32, 16, 16, 96);
    t.check(custom(a), "cursor: plat_cursor_create -> a custom id");
    win_set_cursor(0, a as u32);
    let b = cursor_create(&px, 32, 32, 0, 0, 96);
    t.check(b >= 0 && b != a, "cursor: a second cursor gets another id");
    cursor_destroy(a);
    let c = cursor_create(&px, 32, 32, 31, 31, 96);
    t.check(c == a, "cursor: destroy frees the slot (the next one reuses it)");
    win_set_cursor(0, c as u32); // the window forgot the old shape: applies
    win_set_cursor(0, CUR_ARROW);
    cursor_destroy(b);
    cursor_destroy(c);
    // refused sizes
    let r0 = cursor_create(&px, 0, 32, 0, 0, 96);
    let r1 = cursor_create(&px, CUR_MAX_PX + 1, 1, 0, 0, 96);
    t.check(r0 == -1 && r1 == -1, "cursor: 0 x 32 and 257 x 1 are refused");
    // a 2x image (64 x 64 from the same disc tiled) at dpi 192
    let d = cursor_dpi(144);
    t.info(&format!("  (plat_cursor_dpi(144) = {d})\n"));
    t.check(d == 144 || d == 192, "cursor: plat_cursor_dpi gives 144 or (Wayland) 192");
    let mut px2 = vec![0u8; 64 * 64 * 4];
    for y in 0..64usize {
        for x in 0..64usize {
            let s = ((y >> 1) * 32 + (x >> 1)) * 4;
            let o = (y * 64 + x) * 4;
            px2[o..o + 4].copy_from_slice(&px[s..s + 4]);
        }
    }
    let e = cursor_create(&px2, 64, 64, 32, 32, 192);
    t.check(custom(e), "cursor: a 64 x 64 image for dpi 192");
    win_set_cursor(0, e as u32);
    win_set_cursor(0, CUR_CROSS);
    cursor_destroy(e);
}

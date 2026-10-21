// ASan boundary harness. A C host supplies two distinct, live one-pixel buffers.
// Effect implementations below deliberately use ONLY the safe public plugin API.
#![no_std]
use plugin::{Effect, Fx, FxArgs, FxDesc, Surface};

struct Valid;
impl Effect for Valid {
    type Aux = ();
    fn render(fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        let value = fx.src_px(0, 0);
        fx.dst_row(0)[0] = value;
        Ok(())
    }
}
struct ChangedX;
impl Effect for ChangedX {
    type Aux = ();
    fn render(fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        fx.x0 = 1;
        fx.x1 = 2;
        fx.dst_row(0)[0] = 0x11223344;
        Ok(())
    }
}
struct ChangedY;
impl Effect for ChangedY {
    type Aux = ();
    fn render(fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        fx.y0 = 1;
        fx.y1 = 2;
        fx.dst_row(1)[0] = 0x11223344;
        Ok(())
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn review_fx(src: *mut u8, dst: *mut u8, mode: u32) -> i32 {
    let src = Surface { px: src, w: 1, h: 1, stride: 4, fmt: 3, flags: 0, opac: 255, blend: 0, aux: 0 };
    let dst = Surface { px: dst, ..src };
    let args = FxArgs { src: &src, dst: &dst, x0: 0, y0: 0, x1: 1, y1: 1,
        param: core::ptr::null(), aux: core::ptr::null(), job: core::ptr::null(), sel: core::ptr::null() };
    let desc = match mode {
        1 => FxDesc::new::<ChangedX>(c"review", 0, 0, &[]),
        2 => FxDesc::new::<ChangedY>(c"review", 0, 0, &[]),
        _ => FxDesc::new::<Valid>(c"review", 0, 0, &[]),
    };
    (desc.render.unwrap())(&args)
}

unsafe extern "C" { fn abort() -> !; }
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: terminate only the isolated test process.
    unsafe { abort() }
}
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}

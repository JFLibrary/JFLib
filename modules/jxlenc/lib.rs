// jxlenc -- the JPEG XL writer as a loadable module (jxl-module): img_jxl_enc in a no_std cdylib
// without imports (jxlenc.dll / jxlenc.so next to PaintCraft), loaded on the first JPEG XL save
//                                                                       [jxl-module: no asm]
//
// Export: jxlenc_module(&HostTable) -> &ModTable (lib/plat/modabi.rs).  ModTable::jxl_encode
// gets the samples the host read (img_jxl::writer::image_of) and the writer's options and runs
// img_jxl_enc::enew::encode_px -- the function an app with the linked writer runs, so the file
// is the same byte for byte; its parallel parts run on the host's pool, its blocks come from
// the host's allocator (modules/modrt, modplat).  Built by `python build_rs.py app
// PaintCraft` into the app's folder (section jxl-module: the library crates once more over modrt
// / modplat, fat LTO, checked like PaintCraft's plugin loader checks a library).
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

use img_jxl::writer::{Opts, Px};
use rt::modabi::ModTable;
use rt::{JxlRequest, JxlWriter};

struct Writer;

impl JxlWriter for Writer {
    fn encode(q: JxlRequest<'_>) -> Result<Vec<u8>, i32> {
        let px = Px { w: q.w, h: q.h, gray: q.gray, alpha: q.alpha, rgba: q.rgba };
        let o = Opts {
            effort: q.effort,
            quality: q.quality,
            flags: q.flags,
            icc: q.icc,
            exif: q.exif,
            xmp: q.xmp,
            cancel: q.cancel,
            splines: None,
        };
        img_jxl_enc::enew::encode_px(&px, &o)
    }
}

static TABLE: ModTable = rt::jxl_table::<Writer>();

rt::module_entry!(jxlenc_module, TABLE);

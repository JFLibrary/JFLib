// t_lib -- (plat-shared) the plugin loader with the assembly's example / test plugins of
// PaintCraft (apps/PaintCraft/plugins, built by build.py plugin), without PaintCraft:
//   plattest --test lib --dir DIR      DIR holds swapchan, sepiastr, qoi, pshapes, crash,
//        crashpf, crashq, toonew, badinfo (.dll / .so): open, the exported query in an
//        executable segment, PLUGININFO / descriptors read through lib_seg-checked copies,
//        a query that executes ud2 (GK_ILLEGAL), an effect writing to address 0 (GK_ACCESS),
//        an effect whose PNAPI parallel_for item divides by zero on a worker (nested guards,
//        ERR_FAULT), swapchan's render on a test surface (relocated descriptors, the Win64
//        ABI), loader errors (missing file, not an object, truncated), close
//   plattest --test libfuzz --dir DIR  every file of DIR through lib_open / lib_sym /
//        lib_close under the guard (PaintCraft --plugin-fuzz): "plugin-fuzz:
//        files,loaded,rejected,faults = ..." and every guarded fault (a loader bug)
//                    [apps/PaintCraft/src/plugins.s plug_fuzz; tools/plugin_check.py]

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

use plat::{
    ERR_FAULT, GK_ACCESS, GK_ARITH, GK_ILLEGAL, GuardInfo, LIBA_R, LIBA_X, LIBE_FORMAT, LIBE_IO, Lib, guard_call,
    guard_run, lib_open, lib_read, lib_seg, lib_sym, parallel_for,
};

use crate::T;

const EXT: &str = if cfg!(windows) { ".dll" } else { ".so" };
const PLUGIN_MAGIC: u32 = 0x4950_4E50; // "PNPI"
const PI_SIZE_V1: u32 = 88;
const FXD_RENDER: usize = 24;
const FAULT: u64 = ERR_FAULT as i64 as u64;

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn rd64(b: &[u8], o: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(v)
}

/// a NUL-terminated string of the image (at most 255 bytes, inside one readable segment)
fn lib_str(h: &Lib, p: u64) -> Option<String> {
    let (end, fl) = lib_seg(h, p as usize);
    if end == 0 || fl & LIBA_R == 0 {
        return None;
    }
    let b = lib_read(h, p as usize, (end - p as usize).min(256))?;
    let n = b.iter().position(|&c| c == 0)?;
    Some(String::from_utf8_lossy(&b[..n]).into_owned())
}

// ---------------------------------------------------------------- the host API (PNAPI)

static PF_FAULT_KIND: AtomicU32 = AtomicU32::new(0);
static PF_FAULT_ITEM: AtomicU32 = AtomicU32::new(u32::MAX);

extern "win64" fn h_zero() -> u64 {
    0
}

/// PA_PARFOR(fn, ctx, count) -> eax 0 / ERR_FAULT: fn(ctx, index, job) on the workers, every
/// item under its own guard; a fault cancels the rest
extern "win64" fn h_parfor(f: u64, ctx: u64, count: u64) -> u64 {
    let r = parallel_for(count as u32 as usize, &move |i, job| {
        let mut gi = GuardInfo::default();
        let (r, _) = guard_call(f as usize, ctx, i as u64, 0, 0, Some(&mut gi));
        if r == FAULT {
            PF_FAULT_KIND.store(gi.kind, SeqCst);
            PF_FAULT_ITEM.store(i as u32, SeqCst);
            job.cancel();
        }
    });
    if r.is_err() || PF_FAULT_ITEM.load(SeqCst) != u32::MAX { ERR_FAULT as u32 as u64 } else { 0 }
}

/// the PNAPI table: size / version, then the functions (unused ones return 0)
fn pnapi() -> [u64; 12] {
    let z = h_zero as *const () as u64;
    let mut a = [z; 12];
    a[0] = 96 | 1 << 32; // PA_SIZE_V1, PNAPI_VERSION
    a[4] = h_parfor as *const () as u64; // PA_PARFOR
    a
}

// ---------------------------------------------------------------- --test lib

struct Plug {
    name: &'static str,
    lib: Lib,
    query: usize,
}

pub fn t_lib(t: &mut T) {
    let Some(dir) = plat::arg_value("--dir") else {
        plat::write_stderr(b"plattest: --test lib needs --dir DIR (the test plugins)\n");
        plat::exit(2);
    };
    let api = pnapi();
    let names = ["swapchan", "sepiastr", "qoi", "pshapes", "crash", "crashpf", "crashq", "toonew", "badinfo"];
    let mut plugs: Vec<Plug> = Vec::new();
    for n in names {
        let path = format!("{dir}/{n}{EXT}");
        match lib_open(&path) {
            Ok(lib) => {
                let q = lib_sym(&lib, "pn_plugin_query");
                let (end, fl) = lib_seg(&lib, q);
                t.info(&format!("  ({n}: base {:x} span {:x}, {} segments, query +{:x})\n", lib.base(), lib.span(),
                                lib.segs().len(), q.wrapping_sub(lib.base())));
                t.check(q != 0 && end != 0 && fl & LIBA_X != 0, &format!("lib: {n}: pn_plugin_query in an executable segment"));
                plugs.push(Plug { name: n, lib, query: q });
            }
            Err(e) => t.check(false, &format!("lib: {n}: plat_lib_open (error {e})")),
        }
    }
    if plugs.len() != names.len() {
        return;
    }
    let get = |n: &str| plugs.iter().find(|p| p.name == n).unwrap();
    let sw = get("swapchan");
    t.check(lib_sym(&sw.lib, "pn_plugin_quer") == 0 && lib_sym(&sw.lib, "nichts") == 0 && lib_sym(&sw.lib, "") == 0,
            "lib: plat_lib_sym of missing names = 0");
    t.check(lib_seg(&sw.lib, sw.lib.base() + sw.lib.span()).0 == 0 && lib_seg(&sw.lib, 0x1000).0 == 0,
            "lib: plat_lib_seg outside the image = 0");
    // ---- the queries (guarded); crashq executes ud2
    let mut infos: Vec<(usize, Vec<u8>)> = Vec::new();
    for p in &plugs {
        let mut gi = GuardInfo { tag: 1, ..GuardInfo::default() };
        let (r, _) = guard_call(p.query, api.as_ptr() as u64, 0, 0, 0, Some(&mut gi));
        if p.name == "crashq" {
            t.check(r == FAULT && gi.kind == GK_ILLEGAL && gi.pc == p.query as u64,
                    "lib: crashq: the query's ud2 is caught (GK_ILLEGAL at the query)");
            infos.push((0, Vec::new()));
            continue;
        }
        let info = lib_read(&p.lib, r as usize, PI_SIZE_V1 as usize);
        let ok = info.as_ref().is_some_and(|b| rd32(b, 0) == PLUGIN_MAGIC && rd32(b, 4) >= PI_SIZE_V1);
        t.check(gi.kind == 0 && ok, &format!("lib: {}: PLUGININFO in the image (magic, size)", p.name));
        infos.push((r as usize, info.unwrap_or_default()));
    }
    let info_of = |n: &str| &infos[plugs.iter().position(|p| p.name == n).unwrap()].1;
    let name_of = |n: &str| {
        let b = info_of(n);
        if b.len() < 24 { None } else { lib_str(&get(n).lib, rd64(b, 16)) }
    };
    t.check(name_of("swapchan").as_deref() == Some("Kanäle tauschen (Beispiel)") && name_of("pshapes").as_deref() == Some("Beispielformen")
            && name_of("toonew").as_deref() == Some("Zu neu"), "lib: the plugin names (relocated pointers)");
    t.check(rd32(info_of("toonew"), 8) == 2 && rd32(info_of("swapchan"), 8) == 1, "lib: toonew needs PNAPI version 2");
    let bad = info_of("badinfo");
    let bl = &get("badinfo").lib;
    let first = if bad.len() >= 64 { lib_read(bl, rd64(bad, 56) as usize, 8).map_or(0, |v| rd64(&v, 0)) } else { 0 };
    t.check(rd32(bad, 48) == 1 && first == 0x1000 && lib_seg(bl, first as usize).0 == 0,
            "lib: badinfo: its effect list points outside the image");
    let qoi = info_of("qoi");
    t.check(qoi.len() >= 72 && rd32(qoi, 52) == 1 && lib_read(&get("qoi").lib, rd64(qoi, 64) as usize, 56).is_some(),
            "lib: qoi: one file type descriptor in the image");
    // ---- crash: the effect's render writes to address 0
    let render = |n: &str| -> usize {
        let b = info_of(n);
        let l = &get(n).lib;
        let list = rd64(b, 56) as usize;
        let fx = lib_read(l, list, 8).map_or(0, |v| rd64(&v, 0) as usize);
        let r = lib_read(l, fx, 64).map_or(0, |v| rd64(&v, FXD_RENDER) as usize);
        if lib_seg(l, r).1 & LIBA_X != 0 { r } else { 0 }
    };
    let rc = render("crash");
    let mut gi = GuardInfo::default();
    let (r, _) = guard_call(rc, 0, 0, 0, 0, Some(&mut gi));
    t.check(rc != 0 && r == FAULT && gi.kind == GK_ACCESS && gi.addr == 0 && gi.pc == rc as u64 + 2,
            "lib: crash: render's write to address 0 is caught");
    // ---- crashpf: PNAPI parallel_for, item 11 divides by zero on some thread
    let rp = render("crashpf");
    let (r, _) = guard_call(rp, 0, 0, 0, 0, Some(&mut gi));
    t.info(&format!("  (crashpf: render -> {}, item {} kind {})\n", r as u32 as i32, PF_FAULT_ITEM.load(SeqCst),
                    PF_FAULT_KIND.load(SeqCst)));
    t.check(rp != 0 && r as u32 as i32 == ERR_FAULT && gi.kind == 0 && PF_FAULT_ITEM.load(SeqCst) == 11
            && PF_FAULT_KIND.load(SeqCst) == GK_ARITH, "lib: crashpf: the parallel item's fault stays in its guard, render returns ERR_FAULT");
    // ---- swapchan: render on a 16 x 8 surface, ROI 2..14 x 1..7, every mode
    let rs = render("swapchan");
    let (w, h) = (16usize, 8usize);
    let src: Vec<u8> = (0..w * h * 4).map(|i| (i * 37 + 11) as u8).collect();
    let mut all = rs != 0;
    for mode in 0..3u64 {
        let mut dst = vec![0xEEu8; w * h * 4];
        let ssf = [src.as_ptr() as u64, w as u64 | (h as u64) << 32, (w * 4) as u64 | 3 << 32, 0];
        let dsf = [dst.as_mut_ptr() as u64, w as u64 | (h as u64) << 32, (w * 4) as u64 | 3 << 32, 0];
        let param = [mode, 0, 0, 0];
        let args = [ssf.as_ptr() as u64, dsf.as_ptr() as u64, 2 | 1 << 32, 14 | 7 << 32, param.as_ptr() as u64, 0, 0, 0];
        let (r, _) = guard_call(rs, args.as_ptr() as u64, 0, 0, 0, Some(&mut gi));
        all &= r as u32 == 0 && gi.kind == 0;
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                let (b, g, r_, a) = (src[o], src[o + 1], src[o + 2], src[o + 3]);
                let want: [u8; 4] = if !(2..14).contains(&x) || !(1..7).contains(&y) {
                    [0xEE; 4]
                } else {
                    match mode {
                        0 => [r_, g, b, a], // R <-> B
                        1 => [r_, b, g, a], // R = G, G = B, B = R
                        _ => [g, r_, b, a], // R = B, G = R, B = G
                    }
                };
                all &= dst[o..o + 4] == want;
            }
        }
    }
    t.check(all, "lib: swapchan: render rotates the channels inside the ROI (3 modes)");
    // ---- loader errors
    // (Windows: LoadLibraryExW reports ERROR_MOD_NOT_FOUND, which win_lib.s maps to LIBE_FORMAT)
    let e1 = lib_open(&format!("{dir}/gibt-es-nicht{EXT}")).err();
    t.info(&format!("  (missing file -> {e1:?})\n"));
    t.check(e1 == Some(if cfg!(windows) { LIBE_FORMAT } else { LIBE_IO }), "lib: a missing file -> LIBE_IO (Windows: LIBE_FORMAT)");
    let scratch = format!("{dir}/plattest-lib.tmp{EXT}");
    plat::write_file(&scratch, b"kein Plugin, nur Text\n");
    let e2 = lib_open(&scratch).err();
    let good = plat::read_file(&format!("{dir}/swapchan{EXT}")).unwrap_or_default();
    plat::write_file(&scratch, &good[..good.len().min(600)]);
    let e3 = lib_open(&scratch).err();
    plat::delete(&scratch);
    t.info(&format!("  (text file -> {e2:?}, truncated -> {e3:?})\n"));
    t.check(e2 == Some(LIBE_FORMAT), "lib: not an object file -> LIBE_FORMAT");
    t.check(e3 == Some(LIBE_FORMAT), "lib: a truncated plugin -> LIBE_FORMAT");
    let n = plugs.len();
    for p in plugs {
        plat::lib_close(p.lib);
    }
    t.check(n == 9, "lib: plat_lib_close of every plugin");
}

// ---------------------------------------------------------------- --test libfuzz

pub fn t_libfuzz(t: &mut T) {
    let Some(dir) = plat::arg_value("--dir") else {
        plat::write_stderr(b"plattest: --test libfuzz needs --dir DIR\n");
        plat::exit(2);
    };
    let mut files: Vec<String> = Vec::new();
    plat::dir_list(&dir, &mut |name, is_dir, _| {
        if !is_dir {
            files.push(String::from(name));
        }
    });
    let (mut loaded, mut rejected, mut faults) = (0u32, 0u32, 0u32);
    for f in &files {
        let path = format!("{dir}/{f}");
        let mut gi = GuardInfo::default();
        match guard_run(|| lib_open(&path), Some(&mut gi)) {
            Ok(Ok(lib)) => {
                loaded += 1;
                let r = guard_run(|| lib_sym(&lib, "pn_plugin_query"), Some(&mut gi));
                if r.is_err() {
                    faults += 1;
                    report(f, &gi);
                }
                plat::lib_close(lib);
            }
            Ok(Err(_)) => rejected += 1,
            Err(_) => {
                faults += 1;
                report(f, &gi);
            }
        }
    }
    crate::out(&format!("plugin-fuzz: files,loaded,rejected,faults = {},{},{},{}\n", files.len(), loaded, rejected, faults));
    t.check(faults == 0, "libfuzz: the loader never faulted");
}

fn report(f: &str, gi: &GuardInfo) {
    crate::out(&format!("plugin-fuzz: fault in {f}: kind {} code 0x{:x} at 0x{:x}, address 0x{:x}\n", gi.kind, gi.code, gi.pc,
                        gi.addr));
}

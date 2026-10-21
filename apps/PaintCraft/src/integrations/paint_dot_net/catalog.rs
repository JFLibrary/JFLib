//! Discovery and codec adapters for the optional original Paint.NET DLL host.
//! Native PNAPI DLLs and managed assemblies may share Effects/FileTypes folders.
use crate::app::Pn;
use alloc::{format, string::String, vec::Vec};
use rt::sync::SpinLock;
use ui::*;

pub(crate) const FIRST: u32 = 0x92000;
pub(crate) const LAST: u32 = FIRST + 127;
#[derive(Clone)]
struct Entry {
    path: String,
    kind: String,
    name: String,
    caps: u32,
}
static EFFECTS: SpinLock<Vec<Entry>> = SpinLock::new(Vec::new());
static FILES: SpinLock<Vec<Entry>> = SpinLock::new(Vec::new());
static MESSAGES: SpinLock<Vec<String>> = SpinLock::new(Vec::new());
static SHAPES: SpinLock<Vec<(String, String)>> = SpinLock::new(Vec::new());

pub(crate) fn messages() -> Vec<String> {
    MESSAGES.lock().clone()
}
pub(crate) fn managed_found(path: &str) {
    #[cfg(any(windows, target_os = "linux"))]
    {
        if !super::transport::configured() {
            error(format!(
                "{path}: Die optionale Linux/Wine-DLL-Brücke ist noch nicht konfiguriert (--pdn-wine)."
            ));
        } else if plat::file_exists(&super::transport::helper_path()) == 0 {
            error(format!(
                "{path}: Der optionale .NET-Host fehlt (pdn-host/PaintCraft.PdnHost.dll)."
            ));
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    error(format!(
        "{path}: Originale Paint.NET-DLLs sind auf dieser Plattform nicht verfügbar."
    ));
}
fn error(message: String) {
    plat::write_stderr(format!("Paint.NET: {message}\n").as_bytes());
    let mut messages = MESSAGES.lock();
    if messages.len() < 256 && !messages.contains(&message) {
        messages.push(message);
    }
}
pub(crate) fn states(p: &mut Pn, ui: &mut Ui) {
    for (i, _) in EFFECTS.lock().iter().enumerate() {
        crate::app::commands::pn_cmd_set(
            ui,
            FIRST + i as u32,
            if p.doc.active().is_some()
                && !crate::integrations::paint_dot_net::host::BUSY
                    .load(core::sync::atomic::Ordering::Acquire)
            {
                0
            } else {
                CF_DISABLED
            },
        );
    }
}
pub(crate) fn command(p: &mut Pn, ui: &mut Ui, id: u32, _: u32) -> bool {
    if !(FIRST..=LAST).contains(&id) {
        return false;
    }
    let entry = EFFECTS.lock().get((id - FIRST) as usize).cloned();
    if let Some(e) = entry {
        crate::integrations::paint_dot_net::host::open_effect(p, ui, &e.path, &e.kind);
    }
    true
}
pub(crate) fn summary() -> String {
    format!(
        "Originale Paint.NET-Plugins: {} Effekte, {} Dateiformate",
        EFFECTS.lock().len(),
        FILES.lock().len()
    )
}

// Read only the PE headers. The native loader must never try to execute CLR DLLs.
pub(crate) fn is_managed(path: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(file) = plat::File::open(path, plat::FO_READ) else {
            return false;
        };
        let mut dos = [0u8; 64];
        if file.read_at(0, &mut dos) != Some(dos.len()) || &dos[..2] != b"MZ" {
            return false;
        }
        let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
        let mut pe = [0u8; 264];
        if file.read_at(offset, &mut pe) != Some(pe.len()) || &pe[..4] != b"PE\0\0" {
            return false;
        }
        let base = match u16::from_le_bytes([pe[24], pe[25]]) {
            0x10b => 24 + 96,
            0x20b => 24 + 112,
            _ => return false,
        };
        return pe[base + 14 * 8..base + 15 * 8].iter().any(|v| *v != 0);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        false
    }
}

pub(crate) fn init(p: &mut Pn, ui: &mut Ui) {
    #[cfg(any(windows, target_os = "linux"))]
    init_desktop(p, ui);
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = (p, ui);
}
#[cfg(any(windows, target_os = "linux"))]
fn init_desktop(p: &mut Pn, ui: &mut Ui) {
    if !super::transport::configured() || plat::args().iter().any(|s| s == "--no-plugins") {
        return;
    }
    let helper = crate::integrations::paint_dot_net::host::helper_path();
    if plat::file_exists(&helper) == 0 {
        return;
    }
    let root = crate::integrations::plugins::root(p);
    let explicit = plat::arg_value("--pdn-plugins");
    let mut args = alloc::vec!["--catalog", "--root", root];
    if let Some(root) = explicit.as_deref() {
        args.extend_from_slice(&["--root", root]);
    }
    if plat::args().iter().any(|s| s == "--pdn-isolated") {
        args.push("--isolated");
    }
    let data = match crate::integrations::paint_dot_net::host::exchange(
        &helper,
        &args,
        &[],
        4_000_000,
        60_000,
        || false,
    ) {
        Ok(data) => data,
        Err(e) => {
            error(e);
            return;
        }
    };
    let text = String::from_utf8_lossy(&data);
    for row in text.lines() {
        let fields: Vec<_> = row.split('\t').collect();
        if fields.len() != 6 {
            error(String::from("Ungültiger Katalogeintrag vom .NET-Host"));
            continue;
        }
        let (Some(path), Some(kind), Some(name), Some(exts)) = (
            unhex(fields[1]),
            unhex(fields[2]),
            unhex(fields[3]),
            unhex(fields[5]),
        ) else {
            continue;
        };
        let path = match super::transport::catalog_path(&path) {
            Ok(path) => path,
            Err(e) => {
                error(format!("{path}: {e}"));
                continue;
            }
        };
        let caps = fields[4].parse::<u32>().unwrap_or(0);
        let entry = Entry {
            path,
            kind,
            name,
            caps,
        };
        match fields[0] {
            "E" => {
                let mut effects = EFFECTS.lock();
                if effects.len() >= 128 {
                    error(String::from("Maximal 128 Original-Effekte"));
                    continue;
                }
                let id = FIRST + effects.len() as u32;
                let name = entry.name.replace('&', "&&").leak();
                p.cmds.push(Command::new(id, name, 0, 0, 0, CF_DISABLED));
                p.cmds_dirty = true;
                effects.push(entry);
            }
            "F" => register_file(entry, &exts),
            "S" => {
                if SHAPES.lock().len() < 128 {
                    SHAPES.lock().push((entry.path, entry.name));
                }
            }
            "U" | "X" => error(format!("{}: {}", entry.path, entry.name)),
            _ => error(String::from("Unbekannte Plugin-Kategorie")),
        }
    }
    let names: Vec<String> = EFFECTS.lock().iter().map(|e| e.name.clone()).collect();
    ui.menu_list(FIRST, &names);
}

pub(crate) fn shapes_feed(p: &mut Pn, primary_dir: &str) {
    let canonical = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
    let primary = canonical(&if primary_dir.as_bytes().get(1) == Some(&b':')
        || primary_dir.starts_with('/')
    {
        String::from(primary_dir)
    } else {
        format!("{}/{primary_dir}", plat::cwd())
    });
    for (path, name) in SHAPES.lock().clone() {
        // The ordinary shape loader already read the portable/native plugin root.
        if canonical(path.rsplit_once(['/', '\\']).map_or("", |x| x.0)) == primary {
            continue;
        }
        if let Some(data) = plat::read_file(&path).filter(|b| b.len() <= 65536) {
            if crate::tools::shapes_library::shl_add_xaml(p, &data, &name) == 0 {
                error(format!("{path}: Ungültige Form"));
            }
        } else {
            error(format!("{path}: Form nicht lesbar oder größer als 64 KiB"));
        }
    }
}
#[cfg(any(windows, target_os = "linux"))]
fn unhex(text: &str) -> Option<String> {
    if text.len() % 2 != 0 || text.len() > 131072 {
        return None;
    }
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    let bytes: Option<Vec<_>> = text
        .as_bytes()
        .chunks_exact(2)
        .map(|c| Some(nibble(c[0])? * 16 + nibble(c[1])?))
        .collect();
    String::from_utf8(bytes?).ok()
}

#[cfg(any(windows, target_os = "linux"))]
fn register_file(entry: Entry, exts: &str) {
    use img_core::*;
    if entry.caps & 3 == 0
        || exts.is_empty()
        || !exts
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b";_-".contains(&c))
    {
        return;
    }
    let mut files = FILES.lock();
    let i = files.len();
    if i >= 32 {
        error(format!("{}: Maximal 32 externe Dateiformate", entry.path));
        return;
    }
    let name: &'static str = format!(
        "{} (Paint.NET)",
        entry.name.replace(['=', ';', '\n', '\r'], " ")
    )
    .leak();
    let extensions: &'static str = format!("{exts}={name}").leak();
    let codec = alloc::boxed::Box::leak(alloc::boxed::Box::new(Codec {
        decode: if entry.caps & 1 != 0 {
            Some(DECODERS[i])
        } else {
            None
        },
        encode: if entry.caps & 2 != 0 {
            Some(ENCODERS[i])
        } else {
            None
        },
        caps: if entry.caps & 1 != 0 { IFC_DECODE } else { 0 }
            | if entry.caps & 2 != 0 { IFC_ENCODE } else { 0 },
        ..Codec::EMPTY
    }));
    if io::register_external(name, extensions, IFC_ALPHA, codec) == 0 {
        error(format!("{}: Kein Dateiformatplatz frei", entry.path));
        return;
    }
    files.push(entry);
}

#[cfg(any(windows, target_os = "linux"))]
fn decode<const I: usize>(
    data: &[u8],
    sf: &mut gfx::Surface,
    dec: &img_core::ImgDec,
    meta: &mut img_core::ImgMeta,
) -> i32 {
    use img_core::*;
    if data.len() > 500_000_000 {
        return ERR_LIMIT;
    }
    let entry = FILES.lock()[I].clone();
    let result = crate::integrations::paint_dot_net::host::exchange(
        &crate::integrations::paint_dot_net::host::helper_path(),
        &[&entry.path, &entry.kind, "--decode"],
        data,
        400_000_024,
        120_000,
        || dec.cancelled(),
    );
    let bytes = match result {
        Ok(b) => b,
        Err(e) => {
            if dec.cancelled() {
                return ERR_CANCEL;
            }
            error(format!("{}: {e}", entry.name));
            return ERR_FAIL;
        }
    };
    if bytes.len() < 24 || &bytes[..4] != b"PCIM" {
        return ERR_HDR;
    }
    let u = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let (w, h) = (u(4), u(8));
    let e = meta::dims_check(w, h, dec);
    if e != 0 {
        return e;
    }
    if bytes.len() as u64 != 24 + w as u64 * h as u64 * 4 {
        return ERR_TRUNC;
    }
    let mut image = match meta::surf_alloc(w, h, gfx::FMT_ARGB, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    for y in 0..h as usize {
        if dec.cancelled() {
            return ERR_CANCEL;
        }
        image.px[y * image.stride as usize..y * image.stride as usize + w as usize * 4]
            .copy_from_slice(&bytes[24 + y * w as usize * 4..24 + (y + 1) * w as usize * 4]);
    }
    meta.w = w;
    meta.h = h;
    meta.bits = 8;
    meta.channels = 4;
    meta.model = IMM_RGB;
    meta.flags = IMF_ALPHA;
    meta.orient = 1;
    meta.frames = 1;
    meta.dpix = u(12);
    meta.dpiy = u(16);
    meta.resunit = IMR_INCH;
    *sf = image;
    0
}
#[cfg(any(windows, target_os = "linux"))]
fn encode<const I: usize>(sf: &gfx::Surface, opt: &img_core::ImgOpt, out: &mut Vec<u8>) -> i32 {
    use img_core::*;
    if sf.w < 1
        || sf.h < 1
        || sf.w > 65535
        || sf.h > 65535
        || sf.w as u64 * sf.h as u64 > 100_000_000
    {
        return ERR_DIMS;
    }
    let entry = FILES.lock()[I].clone();
    let mut data = Vec::with_capacity(24 + sf.w as usize * sf.h as usize * 4);
    for n in [
        0x4D494350,
        sf.w as u32,
        sf.h as u32,
        opt.meta.map_or(96 << 16, |m| m.dpix),
        opt.meta.map_or(96 << 16, |m| m.dpiy),
        1,
    ] {
        data.extend_from_slice(&n.to_le_bytes());
    }
    let mut row = alloc::vec![0;sf.w as usize*4];
    for y in 0..sf.h as usize {
        if opt.cancelled() {
            return ERR_CANCEL;
        }
        meta::get_row(sf, y, &mut row, gfx::FMT_ARGB);
        data.extend_from_slice(&row);
    }
    match crate::integrations::paint_dot_net::host::exchange(
        &crate::integrations::paint_dot_net::host::helper_path(),
        &[&entry.path, &entry.kind, "--encode"],
        &data,
        500_000_000,
        120_000,
        || opt.cancelled(),
    ) {
        Ok(data) => {
            *out = data;
            0
        }
        Err(e) => {
            if opt.cancelled() {
                return ERR_CANCEL;
            }
            error(format!("{}: {e}", entry.name));
            ERR_FAIL
        }
    }
}
#[cfg(any(windows, target_os = "linux"))]
macro_rules! slots {
    ($f:ident) => {
        [
            $f::<0>, $f::<1>, $f::<2>, $f::<3>, $f::<4>, $f::<5>, $f::<6>, $f::<7>, $f::<8>,
            $f::<9>, $f::<10>, $f::<11>, $f::<12>, $f::<13>, $f::<14>, $f::<15>, $f::<16>,
            $f::<17>, $f::<18>, $f::<19>, $f::<20>, $f::<21>, $f::<22>, $f::<23>, $f::<24>,
            $f::<25>, $f::<26>, $f::<27>, $f::<28>, $f::<29>, $f::<30>, $f::<31>,
        ]
    };
}
#[cfg(any(windows, target_os = "linux"))]
static DECODERS: [img_core::DecodeFn; 32] = slots!(decode);
#[cfg(any(windows, target_os = "linux"))]
static ENCODERS: [img_core::EncodeFn; 32] = slots!(encode);

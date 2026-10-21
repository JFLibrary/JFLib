# PaintCraft plugins in Rust

Effects, adjustments, file types and shapes for PaintCraft can be written in Rust. A plugin is a
`no_std` library (a `cdylib`) that exports one function, `pn_plugin_query`, and links against
nothing: no `rt`, no `plat`, no system library. The host's functions come through the PNAPI
table.

The binaries are the same kind the assembly plugins are:
- They load in the Rust PaintCraft and in the assembly PaintCraft, on Windows and Linux.
- The ABI is version 1 of `legacy/ASMLIB/inc/plugin.inc`.
- `apps/PaintCraft/docs/PLUGINS.md` describes it for both languages: folders, PNAPI, PLUGININFO, validation, threading rules, crash guard.

A plugin depends only on the crate `lib/plugin` (+ `core`; `alloc` if it wants). That
crate holds:
- the ABI records as `#[repr(C)]` types, with layouts asserted against the include files;
- the trampolines the host calls;
- small safe views, so that the plugin's own code stays safe.

## 1. Build

```
python build_rs.py plugin NAME [--target win|linux|both] [--out DIR]   # plugins/NAME/lib.rs
python build_rs.py plugin                                              # every plugins/*
```

Output: `<out>/<target>/plugins/NAME.dll` (Windows) and `NAME.so` (Linux). `<out>` defaults to `build`.

Copy the file into `<PaintCraft folder>/Effects`, `FileTypes` or `Shapes`. Any plugin may bring
any kind of descriptor; the folders only sort them. For tests, use `PaintCraft --plugins=DIR`.

What the build does (`build_rs.py`, section `rust-plugin`):
- **The crate `plugin`:** built once per target as an rlib (`<out>/<t>/plugin_lib`).
- **The plugin:** a `cdylib` with:
  - `-C panic=abort`, and its own `#[panic_handler]`;
  - `opt-level=3`, one codegen unit;
  - fat LTO, so only the parts of `core` it uses end up in the binary.
- **Windows linking:** `lld-link /NOENTRY /NODEFAULTLIB /Brepro`:
  - no entry point, no imports;
  - `pn_plugin_query` is the only export.
- **Linux linking:** `ld.lld -shared -Bsymbolic --no-undefined --hash-style=both -z now`:
  - position independent;
  - no `DT_NEEDED`;
  - only `R_X86_64_RELATIVE` relocations;
  - no TLS, no init arrays.
- **A check of the result:**
  - It is checked the way PaintCraft's loader checks it (`plugin_verify`: `lib/plat/win_lib.rs`, `linux_elfload.rs`).
  - A plugin the loader would reject fails the build. The check covers:
    - the PE / ELF headers and size;
    - the sections and segments (W^X, page-disjoint);
    - the import, TLS and dynamic tags;
    - the relocation types;
    - the export list.
- **No C runtime:** `memcpy`, `memset` and the other C symbols `core` needs come from `lib/plugin/mem.rs`. They are written in assembly, so they never become exports.

`python build_rs.py check` lints the plugin crates:
- `#![no_std]`;
- `#![deny(unsafe_code)]`;
- only `alloc` / `core`.

## 2. A plugin

`plugins/NAME/lib.rs`:

```rust
#![no_std]
#![deny(unsafe_code)]

use plugin::*;

struct Invert;

impl Effect for Invert {
    type Aux = ();                                   // no prepare step

    fn render(fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        for y in fx.y0..fx.y1 {
            if fx.cancelled() {
                return Err(ERR_CANCEL);              // poll once per row
            }
            let (src, dst) = fx.rows(y);             // the ROI parts of row y
            for (d, &p) in dst.iter_mut().zip(src) {
                *d = p ^ 0x00FF_FFFF;                // straight 0xAARRGGBB
            }
        }
        Ok(())
    }
}

static PROPS: [FxProp; 0] = [];
static FX: FxDesc = FxDesc::new::<Invert>(c"Umkehren (Rust)", FXF_NO_DIALOG, FXC_COLOR, &PROPS);
static FX_LIST: [&FxDesc; 1] = [&FX];
static INFO: PluginInfo = PluginInfo::new(c"Mein Plugin", c"1.0").author(c"Ich").effects(&FX_LIST);

fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)                                      // None refuses loading
}

plugin::entry!(query);                               // exports pn_plugin_query

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)                              // "panic at FILE:LINE" into the log, then ud2
}
```

- `entry!` is the plugin's one `unsafe` item: the `#[unsafe(no_mangle)]` export.
  - It keeps the host's PNAPI table for `api()` when the table is a complete version 1.
- Every descriptor is a `static` built with `const` constructors. Strings are `c"..."` literals (UTF-8, NUL-terminated). Lists are `static` arrays.
  - Put a list into its own `static` (`FX_LIST` above). A temporary `&[&FX]` inside an initializer is not promoted.
- **A panic** is a crash: the host's guard catches the `ud2` and disables the plugin. The message names the function: "Absturz in render (ungültiger Befehl …)".
  - `plugin::panic` logs only the location. A handler of your own can log the message with `log!("{}", info.message())`, at the price of `core::fmt` (about +6 KB).

### Descriptors (`abi.rs`)

| Constructor | Record |
|---|---|
| `PluginInfo::new(name, version)` `.author(..)` `.url(..)` `.api_min(v)` `.effects(&LIST)` `.file_types(&LIST)` `.shapes(&LIST)` | PLUGININFO |
| `FxDesc::new::<E>(menu name, FXF_*, FXC_*, &PROPS)` | FXDESC: render of E; prepare / free (and FXF_PREPARE) when `E::PREPARE` |
| `FxProp::int / dec / angle / boolean / choice / pan / color / seed` (`.log()`) | FXPROP; slot = the parameter slot |
| `FtDesc::new::<T>(name, c"ext1;ext2", Some(c"mime"), IFC_*)` | FTDESC: probe / decode / encode as `T` says |
| `ShapeDesc::new(name, c"M 0,0 L 100,0 …")` | SHAPEDESC |

- The fields are public `#[repr(C)]` records. A test plugin can also write them by hand (see `badinfo`).
- Choice lists are byte strings: `"a\0b\0c\0\0".as_bytes()`. A malformed list does not compile.

### Effects (`fx.rs`)

`impl Effect for E`:
- `type Aux` — what prepare computes for render.
- `const PREPARE: bool` — whether E has a prepare step.
- `fn render(fx, aux)` — writes the destination.
- `fn prepare(fx) -> Result<Option<Aux>, i32>` — once per run, on one worker.
  - The aux lives in a PA_ALLOC block until the host frees it; the trampoline drops and frees it.
  - The assembly ABI returns `rax` = aux and `edx` = the error. A naked trampoline handles that.

`Fx` (one call):

| Method | Gives |
|---|---|
| `width()`, `height()` | the image size |
| `x0`, `y0`, `x1`, `y1` | the ROI |
| `src_row(y)`, `src_px(x, y)` | the whole source (read only) |
| `rows(y)` | the ROI parts of a source and a destination row |
| `dst_row(y)` | the destination row inside the ROI only |
| `param_int`, `param_f64`, `param_bool`, `param_color`, `param(slot)` | the parameters |
| `cancelled()`, `job()` | the cancel state |

- The destination of the Rust host is backed only inside the ROI. `Fx` never hands out anything outside it.
- An index outside the image or the ROI panics, and the plugin is disabled.

### File types (`ft.rs`)

`impl FileType for T`:
- `const PROBE`, `DECODE`, `ENCODE` — which functions T has.
- `fn probe(data, meta) -> bool` — a magic check; fill `meta` when given.
- `fn decode(data, &mut Decoder)`:
  - `d.alloc(w, h)` is PA_SURFALLOC: a zeroed straight FMT_ARGB surface the host owns, with the limits of the decode options.
  - Then fill `d.row_mut(y)`.
  - `d.flags()` (IMGD_*), `d.meta()` and `d.cancelled()` are there as well.
- `fn encode(&Image, &ImgOpt) -> Result<Block, i32>`:
  - `img.row(y)` gives straight ARGB rows.
  - Return the file's bytes in a `Block` (PA_ALLOC). The host frees it.

### The host (`api.rs`)

- `api()` gives `Option<&Api>`: the table after `pn_plugin_query`.
- Free functions:
  - `parallel_for(count, &|index, job| ..)` — on the host's workers; every item is guarded, and nesting is allowed.
  - `hash32(x, y, seed, stream)` — the per-pixel random bits of the built-in effects.
  - `nthreads()`;
  - `log(msg)` and `log!(..)` — lines in the plugin log (stderr: `PaintCraft: Plugin <file>: …`).
- `Block` is a PA_ALLOC byte buffer.
- `PnAlloc` is an opt-in `#[global_allocator]` on PA_ALLOC / PA_FREE / PA_REALLOC, for plugins that use `alloc`:

```rust
extern crate alloc;
#[global_allocator]
static ALLOC: plugin::PnAlloc = plugin::PnAlloc;
```

## 3. Rules (as for the assembly plugins, PLUGINS.md §4)

- **Threads:**
  - render runs on many worker threads at once.
  - Codec functions run on any thread, possibly concurrently.
  - No writable globals. Memory comes only through PNAPI: `Block`, `PnAlloc`.
- **Determinism:** the result of a pixel depends only on the source, the parameters and aux.
  - It never depends on the ROI split or on the thread count.
  - Random effects use `hash32`.
- **Pixels:** straight 0xAARRGGBB (`u32`; bytes B, G, R, A).
- **No `unsafe`** in a plugin except `entry!`. The crash tests write to address 0 and divide by zero in inline assembly, behind a local `#[allow(unsafe_code)]`.

## 4. The plugins here

| Plugin | Port of | What |
|---|---|---|
| `sepiastr` | `apps/PaintCraft/plugins/sepiastr` | adjustment "Sepia-Stärke", FP_INT strength |
| `swapchan` | `swapchan` | effect "Kanäle tauschen", FP_CHOICE order |
| `qoi` | `qoi` | file type QOI: probe, decode, encode |
| `pshapes` | `pshapes` | two shapes |
| `crash`, `crashpf`, `crashq`, `toonew`, `badinfo` | the asm test plugins | crash in render / in a parallel_for item / in pn_plugin_query, too new an API, a bad pointer (tests only) |
| `rsdemo` | (none) | "Kontrast strecken" and "Körnung" (see below) |
| `prepcheck` | (none) | test only: a prepare step that returns an aux, no aux or an error (the `rax` / `edx` return) |

`rsdemo` shows what the ports do not use:
- "Kontrast strecken" has a prepare step with an aux, run with `parallel_for` and `PnAlloc`.
- "Körnung" uses `hash32` and a seed property.

The ports keep the names, infos and math of the assembly plugins:
- Their results are byte-identical in both hosts on both OSes.
- That covers the effect results for every parameter, the QOI files written, and the QOI decodes of valid, truncated and damaged files.

One intended difference: the assembly QOI decoder needs rows written back to back. It returns
ERR_UNSUPPORTED unless the width is a multiple of 16. The Rust one writes each row at the
surface's stride, so every width decodes.

## 5. Tests

- **`python build_rs.py test`** (also `--quick`) builds the plugins and runs the plugin scripts on the Rust PaintCraft:
  - `apps/PaintCraft/tests/plugins.uis` and `plugins_crash.uis`, with the Rust ports in `<out>/<t>/rsptest`;
  - `plugins/tests/*.uis`, with the other Rust plugins in `<out>/<t>/rsplug/Effects`.
  - The expected hashes of `rsdemo.uis` and `prepcheck.uis` come from `plugins/tests/rsdemo_model.py`. `--fixture` writes `rsdemo.png`.
- **`python tools/plugin_check.py --no-build --out build`** runs the asm plugin check on the Rust PaintCraft with the Rust plugins:
  - the scripts;
  - the QOI check;
  - the loader fuzz with 1200 mutants of the Rust binaries;
  - data mutants through the full load path.

# PaintCraft plugins

This document describes the native PNAPI ABI. Original managed Paint.NET DLLs use
the separate [Windows host](../../../tools/paintcraft-pdn-host/README.md), including
folder discovery, effects menus, FileType factories and diagnostics. Managed DLLs
may share Effects/FileTypes folders with PNAPI DLLs; the PE CLR header selects
the loader before execution.


Effects, adjustments, file types and shapes can come from plugin libraries.  Spec: PLUGIN-001..020
(booksAndContext/PaintCraft/paintdotnet_feature_specification.md).  Loader and crash guard:
`legacy/ASMLIB/docs/modules/PLUGIN_LOADER.md`.  Include file: `legacy/ASMLIB/inc/plugin.inc`.

Files: `src/plugins.s` (loading, validation, registration, thunks, PNAPI, error list, hooks),
`legacy/ASMLIB/src/plat/plib.s` + `plat/win/win_lib.s` + `plat/linux/elfload.s` + `plat/linux/lx_guard.s`
(loader, guard), examples in `apps/PaintCraft/plugins/`, `tools/plugin_check.py`, tests
`tests/plugins.uis`, `tests/plugins_crash.uis`.

## 1. Folders, build

At start-up (`plug_init`, before `fio_init` / `fx_init`) PaintCraft loads every `*.dll` (Windows) /
`*.so` (Linux) of `<root>/Effects`, `<root>/FileTypes`, `<root>/Shapes` in name order (at most 32
files).  `<root>` = the folder of PaintCraft's executable; `--plugins=DIR` (or `$PDN_PLUGIN_DIR`)
uses DIR instead, `--plugins=@name` the folder `name` next to the executable (tests),
`--no-plugins` loads nothing.  `--plugins DIR` (two arguments) works too, but fileio.s still
takes DIR for a file to open ("Datei nicht gefunden.") until "--plugins" and
"--plugin-fuzz" are added to its `value_opts` table.  Any plugin may bring any kind of descriptor; the folders only sort
them.  The XAML shapes of `<exe>/Shapes/*.xaml` (SHAPES.md) are unaffected.

```
python build.py plugin NAME [--target win|linux|both] [--out build/x]   # apps/PaintCraft/plugins/NAME/*.s
python build.py plugin                                                  # every apps/PaintCraft/plugins/*
```
Output `build/x/win/plugins/NAME.dll` (PE DLL, no entry point, no imports, export
`pn_plugin_query`) and `build/x/linux/plugins/NAME.so` (ET_DYN, `-Bsymbolic`, no DT_NEEDED, only
RELATIVE relocations).  Plugins are **not** linked against libjfgui (no second copy of global
state): they include `jfgui.inc` + `plugin.inc` for constants and macros only and call the host
through the PNAPI table.  Put every table that holds pointers (PLUGININFO, FXDESC, FXPROP,
FTDESC, shape list) into `.data` -- the loader relocates it; `.rodata` must not contain pointers.

Examples: `swapchan` (effect "Kanäle tauschen", choice property), `sepiastr` (adjustment
"Sepia-Stärke", integer property), `qoi` (file type QOI, decoder + encoder), `pshapes` (two
shapes).  Test-only (never copy them into a real folder): `crash` (render writes to address 0),
`crashpf` (division by zero in a parallel_for item), `crashq` (ud2 in pn_plugin_query),
`toonew` (needs API version 2), `badinfo` (effect list outside the image).

## 2. Entry point and versioning

```
pn_plugin_query(PNAPI* api) -> rax PLUGININFO* | 0      (Win64 ABI on both OSes)
```
Called once on the UI thread.  Keep `api` if later calls need it (a plugin global written here
is fine; render must not write globals).  Return 0 to refuse loading.

* **PNAPI_VERSION = 1** (host).  `PI_APIMIN` = the lowest host version the plugin needs; a
  plugin with `PI_APIMIN > PNAPI_VERSION` (or 0) is rejected ("Das Plugin benötigt die
  Plugin-API-Version N, vorhanden ist M.").
* The PNAPI table starts with `PA_SIZE` (bytes) and `PA_VERSION`; later host versions only
  append entries -- test `PA_SIZE` before using an entry beyond `PA_SIZE_V1`.
* `PI_SIZE` = size of the plugin's PLUGININFO (>= `PI_SIZE_V1` = 88); fields added later are
  read only when `PI_SIZE` covers them.

### PNAPI (host functions, all Win64 ABI)

| offset | function |
|---|---|
| PA_ALLOC 8 | `(size) -> rax` zeroed, 64-byte aligned block or 0 (mem_alloc; thread safe) |
| PA_FREE 16 | `(ptr)` |
| PA_REALLOC 24 | `(ptr, size) -> rax` |
| PA_PARFOR 32 | `(fn, ctx, count) -> eax 0 / ERR_CANCEL / ERR_FAULT`: `fn(ctx, index, job)` for 0..count-1 on the worker threads and the caller; every item runs guarded |
| PA_HASH32 40 | `(x, y, seed, stream) -> eax` deterministic per-pixel random bits (fx_hash32) |
| PA_LOG 48 | `(msg)` UTF-8 line to stderr ("PaintCraft: Plugin <file>: msg"), any thread |
| PA_ROW 56 | `(sf, y) -> rax` pointer to row y of a SURFACE or 0 |
| PA_SURFALLOC 64 | `(sf, w, h, dec) -> eax 0 / ERR_*`: straight FMT_ARGB surface for decoders (limits of the IMGDEC, may be 0) |
| PA_CANCELLED 72 | `(job) -> eax` 1 when the job's cancel flag is set |
| PA_FMT 80 | `(buf, cap, fmt, ...) -> rax` printf subset (fmt_buf) |
| PA_NTHREADS 88 | `() -> eax` worker threads |

The SURFACE record (`jfgui.inc` SF_PX / SF_W / SF_H / SF_STRIDE / SF_FMT) and the FXARGS /
IMGDEC / IMGMETA / IMGOPT / JOB records are part of the ABI of version 1.

## 3. PLUGININFO and descriptors (`plugin.inc`)

```
PLUGININFO name, version, author, url, apimin, nfx, fx, nft, ft, nshape, shapes   (macro)
  PI_MAGIC "PNPI", PI_SIZE, PI_APIMIN, PI_FLAGS 0, PI_NAME (1..63 bytes), PI_VERSION (1..31),
  PI_AUTHOR / PI_URL (or 0), PI_NFX + PI_FX -> FXDESC*[n], PI_NFT + PI_FT -> FTDESC[n],
  PI_NSHAPE + PI_SHAPES -> SHAPEDESC[n]
```
**Effects / adjustments**: `FXDESC` / `FXPROP` exactly as `legacy/ASMLIB/inc/fx.inc` (same contract as
the built-in kernels: pure render over ROIs, deterministic, poll `JOB_CANCEL`, straight ARGB).
Plugin specifics: `FXD_ID` is ignored (the host assigns 0x10000 + slot); `FXD_NAME` is the menu
name shown as is (UTF-8, 1..47 bytes, no '=' and no leading ';' '#' '[' or blank; it is also the
settings key, so it must be unique -- duplicates are rejected); `FXF_ADJ` puts it into
Korrekturen, else into Effekte; `FXP_NAME` is the label shown in the generic dialog (common keys
like "radius" are translated); `FP_BLOB` is not available; `FP_CHOICE` needs 1..32 choices and
`0 <= min <= default <= max < choices`; numbers must be finite with `min <= default <= max`.
Up to 16 effects per plugin, 40 in total.

**File types** (`FTDESC name, exts, mime, probe, decode, encode, caps`): same function contract
as `IMGCODEC` in `img.inc` -- `probe(data, len, meta) -> 1/0` (magic check; 0 = only by the
file extension), `decode(data, len, sf, dec) -> 0 / ERR_*` (allocate the result with
`PA_SURFALLOC`, straight or premultiplied ARGB; with `IMGD_PROBE` pixels are optional),
`encode(sf, opt, &buf, &len) -> 0 / ERR_*` (`sf` is always straight FMT_ARGB; the buffer must
come from `PA_ALLOC`, the host frees it).  At least a decoder or an encoder.  `exts` = 1..8
extensions of 1..15 characters [a-z0-9] separated by ';' (no dots); an extension PaintCraft or an
earlier plugin already knows is rejected.  `caps`: IFC_ALPHA / LOSSY / LOSSLESS / ANIM / PAGES /
HDR.  The type appears in Öffnen ("Alle Bildtypen" + its own entry) and Speichern unter (with an
encoder) -- IMGF 14..21 (`img_register_dyn`, at most 8).  No save-options dialog (IOM 0).

**Shapes** (`SHAPEDESC {name, pathdata}`): 1..63-byte name, path data in the Geometry syntax of
paint.net ShapeKit XAML (`F1 M 0,0 L 100,0 ...`, letters, digits, blanks, `,.+-` only).  They are
fed to the XAML loader of the Formen tool (one XAML document per plugin) and appear after the
built-in shapes; up to 32 per plugin, 64 in total.

## 4. Threading rules

* `pn_plugin_query`: UI thread, once.
* render: many worker threads at once (and the UI thread); prepare: one worker; free: any
  thread.  No writable globals, memory only through PNAPI, results independent of the ROI split.
* Codec functions: any thread, possibly concurrently (thumbnails, open).  Thread safe, no globals.
* PNAPI functions are thread safe; `PA_PARFOR` may be nested.

## 5. Validation and errors (PLUGIN-013/014/015/019/020)

Every pointer in the plugin's data must lie inside one readable segment of its image, every
function pointer in an executable one, strings are NUL-terminated inside the segment, without
control characters.  Anything else rejects the plugin with a message, nothing of it is
registered and the library is unloaded.  A fault anywhere in plugin code (or in host code the
plugin called) is caught: the plugin is **disabled** for the rest of the session (its effects
report "Das Plugin ist abgestürzt und wurde deaktiviert …", its menu rows only show a message,
its codecs return errors), the message ("Absturz in render (Zugriffsverletzung bei +0x100a,
Adresse 0x0). …") goes to stderr and the error list.  A broken plugin never prevents the start;
removing a plugin file only removes its entries.

UI: Effekte ▸ "Plugin-Fehler…" (visible while errors exist) opens Einstellungen ▸ Plugin-Fehler:
the error lines, then "Geladene Plugins" (name, version, author, file, counts).  Diagnose lists
"Plugins: N geladen".  Getters: `plug_count`, `plug_errors`, `plug_loaded`, `plug_text(i, buf,
cap) -> PS_*`.

## 6. Minimal effect plugin

```asm
        .intel_syntax noprefix
        .include "jfgui.inc"
        .include "plugin.inc"
        .text
        .globl pn_plugin_query
pn_plugin_query:                                # (PNAPI*) -> PLUGININFO*
        lea     rax, [rip + info]
        ret
render:                                         # (FXARGS*) -> eax 0 / ERR_CANCEL
        ...                                     # dst = f(src) inside [x0,x1) x [y0,y1)
        .data
        .p2align 3
info:   PLUGININFO s_name, s_ver, s_author, 0, 1, 1, fx_list, 0, 0, 0, 0
fx_list: .quad  fx_desc
fx_desc: FXDESC 0, 0, s_fx, 0, render, 0, 1, props, FXC_COLOR
props:  FXPROP  FP_INT, 0, 0, 0, s_amount, 0.0, 100.0, 50.0
        .section .rodata
s_name: .asciz  "Mein Plugin"
...
```
See `apps/PaintCraft/plugins/swapchan/swapchan.s` (effect), `apps/PaintCraft/plugins/qoi/qoi.s` (file type).

## 7. Tests / hooks

`python tools/plugin_check.py [--target ...] [--out build/p1] [--fuzz N]`: builds, prepares
`<out>/<target>/ptest`, runs `tests/plugins*.uis` (+ `--verify-dirty`), decodes the QOI file the
script wrote with an independent Python codec, fuzzes the loader (1200 ELF mutants on Linux,
1200 DLL mutants on Windows through `--plugin-fuzz DIR`) and loads 120 data mutants through the
full path.  `--fixtures` rewrites `tests/fixtures/p1_plugin.png` / `.qoi` and prints the
expected hashes.  `build.py test` fills `<out>/<target>/ptest` itself (scripts use
`--plugins=@ptest`).

Hooks: cmd 145 i -> #0x82401 text `state|where|kind|effects|filetypes|shapes|name` (PS_: 1
loaded, 2 rejected, 3 crashed; FW_: 0 loader, 1 query, 2 import, 3 prepare, 4 render, 5 free,
6 probe, 7 decode, 8 encode, 9 parallel_for; GK_ of plugin.inc), value = state;
#0x82400 text `loaded,errors,effects,filetypes,shapes`, value = records; #0x82402 fuzz run
`files,loaded,rejected,faults`; cmd 146 need -> #0x82403 text = `img_filter_list(need)` with
'|' between the entries (1 = decoders / Öffnen, 2 = encoders / Speichern unter).  Commands
147..149 are reserved (P1).

## 8. Not supported / limits

* GPU / shader effects (PLUGIN-017/018): no GPU API in the library; PNAPI can grow later.
* No custom dialogs / UI for plugins (generic property dialog only), no plugin settings pages.
* No unloading at run time; a disabled plugin stays mapped (worker threads may still be inside).
* Windows: plugins must not import anything (not even kernel32) and have no DllMain / TLS.
* Linux: no DT_NEEDED, no initialisers, no TLS, RELA relocations RELATIVE / 64 / GLOB_DAT /
  JUMP_SLOT against own symbols only.
* The crash guard is not a sandbox: a plugin runs with the full rights of PaintCraft (see
  PLUGIN_LOADER.md §3).

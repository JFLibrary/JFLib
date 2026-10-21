# jfguiLib documentation

jfguiLib is a general-purpose GUI library with its own fonts, codecs, effects and platform
layer (Win32 with kernel32 / user32 / gdi32 only; Linux with raw syscalls, Wayland and X11),
plus the apps built on it: a drawing program (Kritzel), a paint.net clone, a widget gallery, an
installer and test apps. It was written in x86-64 assembly first; the Rust rebuild in
`RustLib/` is the library now, and the assembly is the frozen reference in `legacy/` next to
it. Every Rust part is bit-identical to the assembly (same pixels, dumps, codec bytes, script
results; the few intended differences are listed in `rewrite/PLAN.md`).

Paths in these documents are relative to `RustLib/` (the working directory of builds, tools
and tests); `../legacy`, `../rewrite` and `../booksAndContext` lie next to it.

## Start here

| Document | Content |
|---|---|
| [GUIDE.md](GUIDE.md) | writing an app: the model, layout, every widget family, menus, dialogs, windows, files, codecs, threads, effects, the test script language, testing, pitfalls |
| [API.md](API.md) | the API reference, generated from the doc comments (`python tools/gendoc.py`, `--check`) |
| [AGENT_WEB.md](AGENT_WEB.md) | modular source layout, opt-in CLI/JSON/MCP control, shared browser host, capabilities and measured platform coverage |
| [jfWord implementation](../apps/jfWord/docs/IMPLEMENTATION.md) | jfOffice, scientific editing, completed checks and remaining Office/Word compatibility work |
| [STYLES.md](STYLES.md) | the style model (`ui::Style` + sub-traits), the four styles, writing and checking a style |
| [PACKAGING.md](PACKAGING.md) | `tools/package.py`: setup.exe with a choice of style, the .deb, AppImage (runtime `apps/appimagert`), RPM .spec, Flatpak, MSIX, icons, `apps/NAME/PACKAGE`, signing (Authenticode, MSIX, apt repository, Android release key), `pkg_check.py` |
| [VERSIONING.md](VERSIONING.md) | the library's version and the apps' own (`rt::app_version!()`), what the public API is, the SemVer rules, deprecations, `build_rs.py api-diff` / `release`, the [changelog](../CHANGELOG.md) |
| [UPDATES.md](UPDATES.md) | `lib/update`: signed update channels, background download, putting in place at the next start, health marker, rollback |
| [VFS.md](VFS.md) | one file namespace: `app://` (bundled `apps/NAME/assets`), `config://`, `data://`, `cache://`, `mem://`, your own mounts (folders, overlays, ZIP) |
| [PERMISSIONS.md](PERMISSIONS.md) | `apps/NAME/PERMISSIONS`: the files, programs, URL schemes, plugins, printing and network addresses an app may use, and how `plat` enforces them |
| [SECRETS.md](SECRETS.md) | `plat::secret`: passwords, tokens and keys in the Credential Manager (DPAPI files for large values) and the Secret Service, the `secrets` permission, limits |
| [`../README.md`](../README.md) | building and testing the Rust code, the rules (`no_std`, no crates, `unsafe` only in `rt` / `simd` / `plat` / `plugin` and the module runtimes), the crate layout, SIMD kernels |
| [`../plugins/README.md`](../plugins/README.md) | PaintCraft plugins in Rust |

## What is where

| Path (in `RustLib/`) | Content |
|---|---|
| `build_rs.py` | build and test (`lib`, `app NAME [--style S]`, `all`, `run`, `test [--quick]`, `check`, `plugin`, `ide`) |
| `lib/` | the library crates: `rt`, `simd`, `plat`, `base`, `gfx`, `text`, `img_core` + one crate per codec family + `img_all`, `img_jxl_enc`, `fx`, `ui`, `plugin`; the network and data crates `netio`, `crypto`, `datastore`, `cas`, `update` |
| `styles/` | the style crates `winui`, `classic`, `macos`, `kde` (assets and `INFO` included) |
| `apps/NAME/` | one folder per app: its Rust code (`main.rs` ...), `tests/*.uis` + fixtures, `res/` (icons), `STYLE`, `TARGETS`, `PACKAGE`, `PERMISSIONS`, `assets/` (app://), `parity.txt`; `apps/PaintCraft/docs/` (the PaintCraft specification: document, layers, tools, effects, plugins, settings, diagnostics, ...) |
| `plugins/` | PaintCraft plugins written in Rust |
| `modules/` | loadable modules (`jxlenc`, the JPEG XL writer) and their runtime crates `modrt` / `modplat` |
| `tools/` | `parity.py` (assembly against Rust: apps), `diffcheck.py` (assembly against Rust: check tools), `gendoc.py` (API.md), the check tools (`*_check.py`: codecs, kernels, effects, fonts, plugins), reference models (`*_ref.py`), `package.py` (with `pkg_linux.py`, `pkg_msix.py`, `pkgmeta.py`, `codesign.py`, `sign_check.py`, `pkg_check.py`), `update_manifest.py` / `update_server.py` (update channels), `mkicon.py`, `ttf2bin.py` / `fontpreview.py` / `refrender.py` (fonts), `wincap.py` / `wslcap.py` (real-window captures), `wincheck.py` / `x11check.py` / `pencheck.py` (real windows, pen input), `testenv.py`, table generators (`gen_*.py`), `measure.py`, `platshared/` and `wlproxy/` (platform acceptance) |
| `docs/` | these documents |
| `build/` | outputs: `win/`, `linux/` (development), `release/`, the assembly reference builds `rwref*/` for parity, tool inputs (`jxl/`, `ext/`, ...) |
| `../rewrite/` | the rebuild: `PLAN.md` (plan and phases R0-R13, German), `AGENTS.md` (briefs of the agent waves), `REPORT.md` (measurements), `README.md` (the language spike) |
| `../legacy/` | the frozen assembly reference (tag `asm-reference`): `ASMLIB/` (src, inc, styles, docs), `build.py` (outputs in `legacy/build`), `apps/*/src`, `inc`, `plugins`, and the archived tools `asmcheck`, `asmcore`, P-ASM (`pasm`), `implib`, the old `gendoc.py` |
| `../booksAndContext/` | background material: books, specifications, the P-ASM design notes, the paint.net feature specification |

## The assembly reference and its specifications

`legacy/` is frozen: it is only the reference the Rust code is compared against
(`tools/parity.py`, `tools/diffcheck.py`; reference binaries in `build/rwref`). Its
documents describe the assembly API (`legacy/ASMLIB/docs/API.md`, `GUIDE.md`, `STYLES.md`,
`PACKAGING.md` -- superseded by the documents above), but the **specifications stay valid**:
the Rust code implements the same algorithms, formats and protocols, and its module headers
name the assembly source they were ported from (`[src/gfx/raster.s]`).

| Document | Content |
|---|---|
| `legacy/ASMLIB/docs/DESIGN.md` | the original design specification: platform layer (Win32 specifics, the Wayland and X11 protocols as used), surfaces and kernels, the rasterizer, SGF1 fonts, codecs, the UI core (model, data, input, dirty regions, theme tokens, automation), test strategy |
| `legacy/ASMLIB/docs/FONTS.md` | the SGF1 font format, `ttf2bin.py`, rendering conventions, licences |
| `legacy/ASMLIB/docs/modules/KERNELS.md` | straight alpha, paint.net blend modes, tiled masks, flood fill, AVX2 / SSE2 rules |
| `legacy/ASMLIB/docs/modules/RESAMPLE.md` | resampling, affine transforms, display mips, gradients |
| `legacy/ASMLIB/docs/modules/DASH_PATHDATA.md` | dashed strokes and SVG / XAML path data |
| `legacy/ASMLIB/docs/modules/FONTS_RUNTIME.md` | runtime fonts: TrueType / OpenType / CFF / COLR, the font database, synthetic styles |
| `legacy/ASMLIB/docs/modules/CODECS.md`, `CODECS_CORE.md`, `CODECS_ZLIB_STATE.md` | codec infrastructure, PNG / JPEG / BMP / GIF, deflate / inflate state |
| `legacy/ASMLIB/docs/modules/CODECS_TIFF.md`, `CODECS_TGA_DDS.md`, `CODECS_BC_ENC.md`, `CODECS_WEBP.md`, `CODECS_WEBP_ENC.md`, `CODECS_AVIF.md`, `CODECS_HEIC.md`, `CODECS_JXL.md`, `CODECS_JXR.md`, `CODECS_PDN.md` | one per codec family (the JPEG XL writer beyond the assembly is described in `lib/img_jxl/lib.rs`) |
| `legacy/ASMLIB/docs/modules/COLOR_MANAGEMENT.md` | ICC profiles, colour transforms, monitor profiles |
| `legacy/ASMLIB/docs/modules/EFFECTS_ADJUST.md`, `EFFECTS_BLUR.md`, `EFFECTS_DISTORT.md` | the effect runner contract and every effect |
| `legacy/ASMLIB/docs/modules/WIDGETS.md` | decimal values, colour tracks, angle, pan, colour wheel, reorderable lists, menu lists, multi-line text |
| `legacy/ASMLIB/docs/modules/WINDOWS.md`, `WINDOWS_PLATFORM.md` | the multi-window runtime and its real OS windows |
| `legacy/ASMLIB/docs/modules/CLIPBOARD_DND.md` | image clipboard and file drops |
| `legacy/ASMLIB/docs/modules/PEN_INPUT.md` | pen pressure, pointer type, tilt |
| `legacy/ASMLIB/docs/modules/THREADS.md` | threads, the worker pool, platform basics |
| `legacy/ASMLIB/docs/modules/CRASH.md` | crash reports, restarting, wall-clock time |
| `legacy/ASMLIB/docs/modules/PLUGIN_LOADER.md` | the plugin loader and crash guard |
| `legacy/ASMLIB/docs/modules/PRINT.md` | printing |

Where the Rust version goes beyond the assembly (the JPEG XL writer, exact crash call chains,
Rust plugins, the allocator's block cache), the crate and module headers describe it;
`rewrite/PLAN.md` lists every such addition.

# Third-party notices

Parts of JFLib are translations of third-party open-source code, follow its algorithms
and constants closely, or embed third-party data, fonts and artwork. Those parts remain
under their original licenses, independently of JFLib's own license. This file lists them
with their copyright holders. The license texts are in `licenses/`, unmodified as published
upstream, except where an entry says the text was taken from a source-file header.

Every build ships this file and `licenses/`. `build_rs.py` copies both next to each app
(the style notices go to `licenses/<app>/<style>/`), `tools/package.py` puts them into the
Windows setup, the MSIX, `.deb`, `.rpm`, AppImage and Flatpak packages (on Linux in
`/usr/share/doc/<package>/`), and the Android build packs them below `assets/` in the APK.

Many Rust modules are ports of JFLib's earlier x86 assembly, named in their file headers
(for example `[src/img/webp_vp8.s]`). That assembly was itself written from the upstream
sources, so the upstream licenses below apply to the Rust code in the same way.

## Required statements

This software is based in part on the work of the Independent JPEG Group.

Portions of this software are copyright © 1996-2026 The FreeType Project
(https://freetype.org). All rights reserved.

## Overview

| Component | Upstream version | License | Used in | License files |
| --- | --- | --- | --- | --- |
| libjxl | 0.11.1, 0.7 | BSD-3-Clause, Google patent grant | `lib/img_jxl`, `lib/img_jxl_enc`, `tools/jxl_ref.py` | `libjxl-LICENSE.txt`, `libjxl-PATENTS.txt` |
| libwebp | 1.3 | BSD-3-Clause, Google patent grant | `lib/img_webp` | `libwebp-COPYING.txt`, `libwebp-PATENTS.txt` |
| IJG libjpeg, libjpeg-turbo | libjpeg-turbo 3.x | IJG License, BSD-3-Clause, Zlib (SIMD code) | `lib/img_jpeg` | `libjpeg-turbo-LICENSE.md`, `libjpeg-turbo-README.ijg`, `libjpeg-turbo-SIMD-zlib.txt` |
| stb_image | – | MIT or public domain (Unlicense) | `lib/img_jpeg/dec.rs`, `lib/img_core/inflate.rs` | `stb-LICENSE.txt` |
| zlib | – | Zlib | `lib/img_core/deflate.rs` | `zlib-LICENSE.txt` |
| miniz | – | MIT | `lib/img_core/deflate.rs` | `miniz-LICENSE.txt` |
| DirectXTex | – | MIT | `lib/img_dds` | `DirectXTex-LICENSE.txt` |
| libtiff | 4.x | libtiff License | `lib/img_tiff/dec.rs` | `libtiff-LICENSE.md` |
| FreeType | 2.x | FreeType License (FTL) | `lib/text/outline.rs` | `FreeType-FTL.txt` |
| paint.net | 3.36 | MIT with exceptions | `lib/fx`, `lib/gfx` | `paint.net-3.36-License.txt` |
| Pinta | – | MIT | `lib/fx`, `lib/gfx` | `Pinta-license-mit.txt`, `Pinta-license-pdn.txt` |
| jxrlib | 1.1 | BSD-2-Clause | `lib/img_jxr` | `jxrlib-LICENSE.txt` |
| libaom (AV1) | – | BSD-2-Clause, AOMedia Patent License 1.0 | `lib/img_av1` | `libaom-LICENSE.txt`, `AOM-PATENTS.txt` |
| Brotli | RFC 7932 | MIT | `lib/img_jxl` | `Brotli-LICENSE.txt` |
| fdlibm | 5.3 | Sun notice | `lib/fx/warp.rs` | `fdlibm-NOTICE.txt` |
| Unicode Character Database, CLDR | UCD 15.0 / 15.1, CLDR | Unicode License v3 | `lib/ui`, `lib/text`, `lib/datetime` | `Unicode-3.0.txt` |
| tz database | – | public domain | `lib/datetime/zones.rs` | none needed |
| DejaVu fonts | 2.37 | Bitstream Vera / Arev license, public domain changes | `lib/doc_view`, `apps/termtest`, styles `kde`, `classic` | `DejaVu-LICENSE.txt` |
| Inter font | – | SIL Open Font License 1.1 | styles `jfoffice`, `backupcraft`, `netlab`, `apps/officeicontest` | `styles/*/assets/OFL-Inter.txt` |
| WordCraft icon motifs | 2026 | MIT | style `jfoffice` (`icons.bin`, also used by `backupcraft` and `netlab`) | `styles/jfoffice/LICENSE-icons.txt` |
| Dynamic Draw brush tips | v4.0 (`722ccb2`) | MIT | `apps/PaintCraft/resources/brush-factory` | `DynamicDraw-LICENSE.txt` |
| HarfRust and its crates | see notice | MIT, Apache-2.0, Zlib, Unicode-3.0 | `lib/text_shape` | `lib/text_shape/NOTICE.md` |

## libjxl

Copyright (c) the JPEG XL Project Authors. BSD-3-Clause, with Google's additional IP
rights grant (patents). Versions 0.11.1 and 0.7 carry the same license and patent texts.

- `lib/img_jxl_enc/*`: a port of libjxl 0.11.1's encoder (`enc_ac_strategy.cc`,
  `enc_adaptive_quantization.cc`, `enc_ans.cc`, `enc_modular.cc`, `enc_ma.cc`,
  `enc_patch_dictionary.cc`, `enc_detect_dots.cc`, `enc_chroma_from_luma.cc`,
  `enc_photon_noise.cc`, `enc_icc_codec.cc`, `enc_transforms-inl.h`, `enc_splines.cc` and
  others, named in each file's header). `ebutter.rs` ports libjxl 0.7's Butteraugli
  (`lib/jxl/butteraugli/butteraugli.cc`).
- `lib/img_jxl/spline.rs`: libjxl 0.7's spline decoding and rendering (`splines.cc`).
- `lib/img_jxl/emath.rs`: libjxl's fast math (`base/fast_math-inl.h`,
  `rational_polynomial-inl.h`).
- `lib/img_jxl/jpeg.rs`: libjxl 0.11.1's JPEG reconstruction from the `jbrd` box.
- The decoder in `lib/img_jxl` (`dec.rs`, `ent.rs`, `vdc.rs`, `epf.rs`, `noise.rs`, `rnd.rs`
  and the generated `tab.rs`) follows the model `tools/jxl_ref.py`, which was written from
  libjxl 0.11.1's decoder sources (`dec_frame.cc`, `dec_modular.cc`, `dec_ans.cc`,
  `dec_huffman.cc`, `dec_context_map.cc`, `quantizer.cc`, `stage_epf.cc`, `dec_noise.cc`,
  `render_pipeline/stage_*.cc` and others, named in each file's header). The other files
  of `lib/img_jxl` cite libjxl for behaviour only.
- `modules/jxlenc` ships `lib/img_jxl_enc` as a separate module next to the apps.

## libwebp

Copyright (c) 2010, Google Inc. BSD-3-Clause, with Google's additional IP rights grant for
the WebM codecs (patents). JFLib follows libwebp 1.3.

- `lib/img_webp/vp8.rs` (translated): `VP8BitReader`, `ParseIntraMode`, `ParseResiduals`,
  `GetCoeffs`, the inverse transforms, the loop filters in libwebp's SSE2 form, the fancy
  upsampler and the YUV-to-RGB constants of `yuv.h`. The intra predictors are a
  reimplementation from RFC 6386.
- `lib/img_webp/vp8_enc.rs` (translated): libwebp's lossy encoder at method 4, including the
  tables `kLinearToGammaTab`, `kWeightY`, `kBiasMatrices` and `kFreqSharpening`.
- `lib/img_webp/vp8l.rs` (mixed): the bitstream and transforms are reimplemented from
  RFC 9649; `BuildHuffmanTable`, the `VP8LBitReader` model and `kCodeToPlane` follow libwebp.
- `lib/img_webp/vp8l_enc.rs` (mixed): libwebp's lossless heuristics and constants (entropy
  analysis, `GetBestGreenToRed` / `GetBestGreenToBlue`, palette sorting, hash chain, LZ77
  and RLE references); the data layout, bit writer, Huffman builder and parser are JFLib's.
- `lib/img_webp/tab.rs`: the VP8 probability, quantizer and zigzag tables (identical to
  RFC 6386, whose reference code is also BSD-licensed by Google), `kBModesProba` in
  libwebp's mode order, and from line 283 the gamma table computed with libwebp's formula
  (`picture_csp_enc.c`).
- `lib/img_webp/container.rs` reimplements the RIFF container from RFC 9649 and cites
  libwebp only for acceptance rules.

## IJG libjpeg and libjpeg-turbo

The IJG code is copyright (C) 1991-2020 Thomas G. Lane, Guido Vollbeding and the
Independent JPEG Group; `libjpeg-turbo-README.ijg` is the unmodified IJG README with its
license terms. The libjpeg-turbo modifications are copyright D. R. Commander and others
(BSD-3-Clause, `libjpeg-turbo-LICENSE.md`). libjpeg-turbo's x86 SIMD code is copyright
2009 Pierre Ossman for Cendio AB, D. R. Commander, MIYASAKA Masaru and others under the
Zlib license; `libjpeg-turbo-SIMD-zlib.txt` reproduces that notice from
`simd/nasm/jsimdext.inc` of libjpeg-turbo 3.2.0.

- `lib/img_jpeg/dsp.rs` (translated): the accurate integer DCT and IDCT (`jfdctint`,
  `jidctint` in the form of libjpeg-turbo's `jfdctint-sse2` / `jidctint-sse2`), the
  quantization of `jquanti-sse2`, the colour conversions of `jccolext-sse2` /
  `jdcolext-sse2`, IJG's fancy upsampling (h2v1, h2v2, and libjpeg-turbo's h1v2),
  `jcsample`'s bias pattern and IJG's `jpeg_natural_order`. The Huffman and quantization
  tables are those of ITU-T T.81 Annex K.
- `lib/img_jpeg/enc.rs` (mixed): IJG's `jpeg_quality_scaling` and `jpeg_add_quant_table`,
  libjpeg-turbo's `compute_reciprocal` (`jcdctmgr.c`) and IJG's `jpeg_gen_optimal_table`.
- `lib/img_jpeg/dec.rs` reimplements the decoder; it matches libjpeg's behaviour
  (`jpeg_resync_to_restart`, progressive refinement) without its code.

## stb_image

Copyright (c) 2017 Sean Barrett; MIT, or public domain (Unlicense), at the user's choice.
The Huffman lookup of `lib/img_jpeg/dec.rs` (9-bit fast table with combined AC entries, the
`maxcode` sentinel and the sign-extension bias table) closely resembles `stb_image.h`, and
`lib/img_core/inflate.rs` builds its Huffman tables like stb_image's zlib decoder (10-bit
fast table, canonical slow path). No attribution is required under the public-domain
alternative; the license is included for transparency.

## libtiff

Copyright (c) 1988-1997 Sam Leffler, Copyright (c) 1991-1997 Silicon Graphics, Inc.

- `lib/img_tiff/dec.rs`: the YCbCr-to-RGB tables and conversion are a translation of
  libtiff's `tif_color.c` (`TIFFYCbCrToRGBInit`, `TIFFYCbCrtoRGB`, `Code2V`, `CLAMPw`).
- `lib/img_tiff/ccitt.rs` reimplements CCITT Group 3 / 4 decoding from ITU-T T.4 and T.6;
  it follows libtiff only in behaviour (EOL synchronisation, bit polarity) and uses none of
  libtiff's code or state tables.
- `lib/img_tiff/lzw.rs` and the LZW writer in `enc.rs` are JFLib's own; they read and write
  libtiff-compatible streams. The Berkeley `compress` notice in `libtiff-LICENSE.md`
  therefore does not apply to JFLib.

## zlib and miniz

zlib: (C) 1995-2026 Jean-loup Gailly and Mark Adler, Zlib license. miniz: Copyright
2013-2014 RAD Game Tools and Valve Software, Copyright 2010-2014 Rich Geldreich and
Tenacious Software LLC, MIT License. `lib/img_core/deflate.rs` uses zlib's compression
levels (`configuration_table` of `deflate.c`: good, lazy, nice and chain limits) and its
`TOO_FAR` rule, and limits Huffman code lengths with miniz's Kraft-sum repair
(`tdefl_huffman_enforce_max_code_size`). The rest of the compressor and the decoder
`lib/img_core/inflate.rs` are JFLib's own.

## DirectXTex

Copyright (c) Microsoft Corporation, MIT License. `lib/img_dds` follows DirectXTex for the
BC7 speed levels (`bc7.rs`), the perceptual BC1-BC3 channel weights (`enc.rs`), the order
in which legacy DDS pixel formats are recognised (`read.rs`) and the `DDSPF_*` header values
it writes (`write.rs`). The block encoders and decoders are JFLib's own.

## FreeType

FreeType is copyright 1996-2026 David Turner, Robert Wilhelm, Werner Lemberg and the
FreeType Project, used under the FreeType License (FTL); see the credit under "Required
statements". `lib/text/outline.rs` (`Outline::embolden`, lines 194-294) is a
translation of `FT_Outline_EmboldenXY` from `src/base/ftoutln.c`.

## paint.net 3.36 and Pinta

paint.net 3.36: Copyright (C) dotPDN LLC, Rick Brewster, Chris Crosetto, Tom Jackson,
Michael Kelsey, Brandon Ortiz, Craig Taylor, Chris Trevino, and Luke Walker. Portions
Copyright (C) Microsoft Corporation. MIT License with three exceptions (see
`paint.net-3.36-License.txt`): the code in `lib/fx` and `lib/gfx` uses none of paint.net's
logo or icon artwork and none of its "resource assets" (`.resources`, `.resx`, `.png`
files), and JFLib contains no GPC code.

Pinta: Copyright (c) 2010 Jonathan Pobst and the Pinta contributors, MIT License. Pinta's
C# port of the paint.net 3.36 effects keeps paint.net's license (`Pinta-license-pdn.txt`).

The following files implement paint.net 3.x algorithms with their integer arithmetic,
tables and edge handling: `lib/fx/adj.rs` (Brightness / Contrast, Hue / Saturation, Levels,
Posterize, Auto Level), `blur.rs`, `conv.rs`, `hist.rs`, `lochist.rs`, `lut.rs`,
`stylize.rs`, `noise.rs`, `photo.rs`, `render.rs`, `artistic.rs`, `warp.rs`, and
`lib/gfx/blendmode.rs` (`UserBlendOps`), `flood.rs` (`CheckColor`) and `gradient.rs`.
The adjustments that paint.net added after 3.36 (Curves, Exposure, Highlights / Shadows,
Temperature / Tint) are JFLib's own implementations of the features; paint.net 4 and later
are not open source and no code of theirs is used. The Curves table (`spline_lut` in
`lut.rs`) is JFLib's own natural cubic spline (a tridiagonal system solved with the Thomas
algorithm); it gives the same values as paint.net's `SplineInterpolator`, whose code comes
from Numerical Recipes and is not used.

## jxrlib

Copyright © Microsoft Corp. BSD-2-Clause. `lib/img_jxr/tab.rs` contains jxrlib's
`DPK_QPS_8` and `DPK_QPS_420` quality tables (lines 750 and 758), which `lib/img_jxr/enc.rs`
uses to map the quality setting to quantizers. The rest of `lib/img_jxr` implements
JPEG XR from ITU-T T.832.

## libaom and the AV1 specification

libaom: Copyright (c) 2016, Alliance for Open Media. BSD-2-Clause, with the Alliance for
Open Media Patent License 1.0 (`AOM-PATENTS.txt`), which also covers implementations of
the AV1 specification.

- `lib/img_av1/eec.rs`: the range encoder follows libaom's `od_ec_enc`.
- `lib/img_av1/etx.rs`: the lossless forward Walsh-Hadamard transform follows libaom's
  `av1_fwht4x4`; `lib/img_av1/enc.rs` uses libaom's loop filter level choice for key
  frames (`LPF_PICK_FROM_Q`).
- The decoder (`lib/img_av1`) follows the AV1 Bitstream and Decoding Process
  Specification (copyright 2018 The Alliance for Open Media); its constant tables were
  extracted from the specification by `tools/av1_spec_tables.py`.

## Brotli

Copyright (c) 2009, 2010, 2013-2016 by the Brotli Authors. MIT License.
`lib/img_jxl/brotli_dict.bin` is Brotli's static dictionary and
`lib/img_jxl/brotli_tab.rs` its layout and word transforms (RFC 7932 appendices A and B),
generated by `tools/gen_brotli_dict.py`.

## fdlibm

Copyright (C) 1993 by Sun Microsystems, Inc. `lib/fx/warp.rs` (about lines 422-500,
`sincos`) uses fdlibm's sine and cosine kernel polynomials (`k_sin.c`, `k_cos.c`).
`fdlibm-NOTICE.txt` reproduces the notice from fdlibm's source files.

## Unicode data

Copyright © 1991-2026 Unicode, Inc. Unicode License v3.

- `lib/ui/i18n_unicode.rs`: generated by `tools/gen_unicode.py` from `UnicodeData.txt`
  (Unicode 15.1.0).
- `lib/text/segment_data.rs`: generated by `tools/gen_segment.py` from the Unicode
  Character Database (15.0.0) and Python's `unicodedata` (15.1.0).
- `lib/ui/i18n_locale.rs`: regional formats and plural rules, a compact subset of CLDR.
- `lib/datetime/zones.rs`: the Windows-to-IANA zone mapping of CLDR's `windowsZones.xml`.

## tz database

`lib/datetime/zones.rs` is generated by `tools/gen_tz.py` from the IANA tz database, which
is in the public domain.

## Fonts and artwork

- DejaVu fonts (DejaVu Sans, Sans Condensed, Sans Mono, Serif), version 2.37: Bitstream Vera
  fonts copyright (c) 2003 Bitstream, Inc.; Arev glyphs copyright (c) 2006 Tavmjong Bah;
  DejaVu changes in the public domain. Embedded by `lib/doc_view` (`assets/`), `apps/termtest`
  (DejaVu Sans Mono from `lib/doc_view/assets`) and the styles `kde` (DejaVu Sans as SGF1) and
  `classic` (DejaVu Sans Condensed as SGF1). `licenses/DejaVu-LICENSE.txt`, and with each
  style its own copy in `styles/kde/assets/` and `styles/classic/assets/`.
  (`styles/jfoffice/assets/` also holds a copy, although jfoffice now embeds Inter.)
- Inter (SIL Open Font License 1.1, copyright (c) 2016 The Inter Project Authors): embedded
  by the styles `jfoffice`, `backupcraft` and `netlab` and by `apps/officeicontest` (which
  uses the style jfoffice); the license is in each style's `assets/OFL-Inter.txt`.
- WordCraft icon motifs (MIT, copyright (c) 2026 ArtCraft Team and the WordCraft
  contributors): the office icon artwork of `styles/jfoffice/icon_art.py` and
  `styles/jfoffice/assets/icons.bin`, which the styles `backupcraft` and `netlab` embed too;
  `styles/jfoffice/LICENSE-icons.txt`.
- Dynamic Draw brush tips (MIT, copyright (c) 2017 Joshua Lamusga): the six
  `apps/PaintCraft/resources/brush-factory/BrCircle*.png` images from
  NinthDesertDude/Dynamic-Draw, embedded unchanged by PaintCraft (see `NOTICE.md` there;
  PaintCraft also shows the license in its brush dialog).
- The fonts and icons of the other styles: see each style's license files in
  `styles/<style>/` and `styles/<style>/assets/` and `styles/NOTICE.md`.

A style that embeds another style's fonts or icons (`include_bytes!("../<other>/...")`)
ships that style's license files too, in `licenses/<other>/`.

## References without copied code

These projects are cited in comments for compatible behaviour, file formats or published
algorithms; the review for this file found no code or tables copied from them, so they need
no notice: Little CMS (tag choice and media white in `lib/img_core/icc_xform.rs`,
`icc_eval.rs`), PCG and SplitMix64 (`lib/base/rng.rs`, same output as the reference
generators), Hunspell (suggestion order and the `.aff` / `.dic` format in `lib/spell`),
libavif (`lib/img_avif/enc.rs`), the HEVC test model HM (`lib/img_hevc/enc.rs`), giflib /
Wuffs (`lib/img_gif`), Wu's colour quantizer (`lib/img_core/quantize.rs`), BLAKE3
(`lib/crypto/blake3.rs`), FastCDC (`lib/cas/chunker.rs`), the SquashFS format and the
AppImage runtime (`apps/appimagert`), libtiff's LZW and CCITT behaviour (`lib/img_tiff`),
the WebP container (`lib/img_webp/container.rs`), the paint.net `.pdn` format
(`lib/img_pdn`), and paint.net's default settings and clipboard conventions (`lib/img_core`,
`lib/img_jpeg`, `lib/img_webp`, `lib/plat`). `lib/img_pdn/template.rs` holds the .NET
serialization headers (class and field names) of a `.pdn` file written by paint.net 5.1,
which paint.net needs to read JFLib's files, and `lib/plat/linux_xkb.rs` a subset of the X11
keysym names and values, which XKB keymaps use; both are interoperability data, not code of
paint.net or X.Org.

## Tools and test data

These files are not part of any build or package:

- `tools/wlproxy/keymap.xkb` is the keymap WSLg's compositor sends, compiled from
  xkeyboard-config data (copyright Joseph Moss, the Free Software Foundation, Digital
  Equipment Corporation, Silicon Graphics and many others; MIT / X11-style permissions).
  The full notices are in `tools/wlproxy/xkeyboard-config-COPYING.txt`.
- `tools/update_manifest.py` implements Ed25519 from the algorithm of RFC 8032 section 5.1
  and the Explicit-Formulas Database. RFC 8032 is an IRTF document, whose example code is
  not licensed for modified use (IETF Trust Legal Provisions, section 8.g), so that code is
  not used.

## Rust crates of lib/text_shape

HarfRust, Unicode Bidi, Unicode Script and their dependencies are listed with their
licenses in `lib/text_shape/NOTICE.md`; the license files are in
`lib/text_shape/licenses/`. Builds ship them as `licenses/text_shape/`.

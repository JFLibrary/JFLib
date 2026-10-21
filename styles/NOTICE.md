# Font licences of the styles

Every font and icon set the styles embed is openly licensed, so all styles may be
redistributed. The licence files named below must ship with every copy; `build_rs.py` copies
them next to each app it builds (`licenses/<app>/<style>/`).

- winui and macos text: "JFLib Sans" (`winui/assets/font_reg.bin`, `font_semi.bin`,
  `font_bold.bin`) is Selawik 1.01, Copyright 2015 Microsoft Corporation, with Reserved Font Name
  "Selawik", under the SIL Open Font License 1.1 (`winui/assets/OFL-Selawik.txt`, copied to
  `macos/assets/`). The SGF1 files are subsets (text charset, no hinting), i.e. a Modified
  Version under the OFL, and therefore do not use the name Selawik. Code points Selawik lacks
  are built from its own glyphs (`tools/ttf2bin.py --compose`), except ∞ ● №, which are taken
  from Inter, Copyright 2016 The Inter Project Authors, SIL Open Font License 1.1
  (`winui/assets/OFL-Inter.txt`, copied to `macos/assets/`). The unmodified Selawik source files
  (`winui/assets/selawk.ttf`, `selawksb.ttf`, `selawkb.ttf`) are not built into any program;
  the font check tools compare the subsets with them.
- kde text: DejaVu Sans regular / bold (`kde/assets/font_reg.bin`, `font_bold.bin`). Bitstream
  Vera Fonts Copyright 2003 Bitstream, Inc., glyphs from the Arev fonts Copyright 2006 Tavmjong
  Bah, DejaVu changes in the public domain (`kde/assets/DejaVu-LICENSE.txt`, the upstream DejaVu
  `LICENSE`). The notices must ship with every copy; a modified version must not use the names
  "Bitstream", "Vera" or "Arev", which "DejaVu Sans" does not.
- classic text: DejaVu Sans Condensed 2.37 regular / bold (`classic/assets/font_reg.bin`,
  `font_bold.bin`), under the same licence as kde's DejaVu Sans (`classic/assets/DejaVu-LICENSE.txt`,
  a copy of kde's). The SGF1 files are subsets with Tahoma's line box (ascender / descender
  2049 / -423 font units instead of 1901 / -483), so the style keeps the baselines it had with
  Tahoma, its former font; the name "DejaVu Sans Condensed" contains none of the reserved names.
- Icons: the shared icon font `winui/assets/icons.bin` (styles winui, macos, classic and kde)
  takes its glyphs from Fluent UI System Icons, Copyright Microsoft Corporation, under the MIT
  licence (https://github.com/microsoft/fluentui-system-icons). Every style that embeds it keeps
  a copy of `LICENSE-FluentSystemIcons.txt` in its `assets/`.

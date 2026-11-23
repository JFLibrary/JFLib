# diagramiotest

The tests of the diagram codecs (`lib/diagram_io`), after the pattern of `codectest`: a console
program that builds VSDX packages itself and reads them back. Made with
`python build_rs.py new diagramiotest --kind console` on 2026-11-16 for stage JD3 of
[apps/JFDiagrammer/PLAN.md](../JFDiagrammer/PLAN.md).

```sh
python build_rs.py test diagramiotest                    # build it, run tests/vsdx.uis (Windows and Linux)
python build_rs.py run diagramiotest -- --check --only xml    # one group
python build_rs.py run diagramiotest -- --corpus DIR     # the local load tests with the files in DIR
python build_rs.py run diagramiotest -- --write-samples DIR  # the self-built files, to open elsewhere
python build_rs.py run diagramiotest -- --check --only roundtrip  # writing: all four roundtrip groups
python build_rs.py run diagramiotest -- --write-probe DIR [--corpus DIR]  # files for a Visio check
python build_rs.py run diagramiotest -- --check --only vdx    # (JD16) the XML formats of 2003-2010, macros, properties
python build_rs.py run diagramiotest -- --write-vdx DIR  # (JD16) the self-written .vdx / .vsx and the .vsdx written from them
```

`--write-probe` writes a new empty drawing, a new drawing with own shapes and connectors, a new
stencil and a round-trip copy of each local file to DIR, to open them in Visio. DIR must lie
outside the repository: the copies are the user's files.

## Groups

| Group | What it checks |
|---|---|
| `samples` | self-built files of all six formats (`.vsdx` `.vstx` `.vssx` `.vsdm` `.vstm` `.vssm`): every field, extra, part and relationship where it belongs; UTF-16, BOM, a prefixed namespace |
| `xml` | broken syntax, entities and DOCTYPE, encodings, wrong roots, depth (also 5000 nested groups: the reader does not recurse), attributes |
| `limits` | the counts of `vsdx::Limits` (shapes, cells, rows, sections, formula and text length, warnings); large drawings in time |
| `refs` | cycles of styles, masters and background pages; references that name nothing (warnings) |
| `rels` | wrong relationships and content types, parts used twice, macros, malformed IDs |
| `zip` | ZIP bombs and lying archives through `lib/archive` (a trimmed copy of codectest's ZIP builder) |
| `corpus` | `Zeichnung2.vsdx`, `Zeichnung 3.vsdx`, `Favoriten.vssx` from `apps/JFDiagrammer/context/files` (gitignored, never copied): counts against FEATURES.md 1.6; skipped when the folder is not found above the current folder |
| `roundtrip/samples` | (JD5) writing: the self-built files of the six formats read, written, read and written again: the same model both times, the same bytes on the second write |
| `roundtrip/new` | `vsdx::new_document` for drawing, template and stencil in both measures, written in every format: deterministic, the same model and bytes after a round trip, `visio/windows.xml` always there, only own content (no Office theme, no Calibri); a drawing with own shapes, connectors, a master instance, a group and a picture; a stencil with own masters |
| `roundtrip/hostile` | models the reader never makes (extras at places that do not exist, broken raw XML, characters XML cannot carry, parts named twice or not at all, macros, missing pictures, odd numbers, 3000 nested groups, stale relationships, tight ZIP limits): no crash, a readable file or a `WriteError` |
| `roundtrip/corpus` | the local files of `corpus` read, written and read again: the same model, the same bytes on the second write; only sizes and times are printed, nothing is written to disk; skipped like `corpus` |

| `vdx` | (JD16) Visio's XML formats of 2003-2010 (`diagram_io::vdx`): a self-written drawing, stencil and bitmap (`src/vdx_sample.rs`, after the public schema reference) read into the model -- cells with formula and value, sections, rows by name, geometry, font IDs as names, text marks and fields, a group, glue, pictures as parts (a DIB gets its file header), masters with icons, background page, properties, windows, header and footer -- and written as `.vsdx` / `.vssx` / `.vsdm` and read back with the same content; hostile files (roots, encodings, IDs, cycles, what is left out) and every limit; macro projects of `.vsdm`, `.vssm`, `.vstm` and a `.vdx` passed through byte for byte, refused in formats without macros, `vsdx::remove_macros`; `props` (core, app and custom properties) and `windows` (the drawing window) |

Only self-built files belong in the repository; the packages are made in `src/pkg.rs` and
`src/build.rs`.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the runner and its check harness |
| `src/pkg.rs` | a builder of VSDX packages and the sample drawing |
| `src/samples.rs`, `src/hostile.rs`, `src/corpus.rs` | the groups |
| `src/zipkit.rs` | crafted ZIP archives |
| `src/roundtrip.rs`, `src/hostile_write.rs` | the writing groups and `--write-probe` |
| `src/build.rs` | new documents with own shapes, connectors, masters and a picture, through the model |
| `src/dump.rs` | the whole model as text, every string resolved, to compare two reads |
| `src/vdx.rs`, `src/vdx_sample.rs` | (JD16) the group `vdx`, `--write-vdx`; the self-written `.vdx` / `.vsx` files and their pictures |
| `tests/vsdx.uis` | what `build_rs.py test` runs (`--check`) |

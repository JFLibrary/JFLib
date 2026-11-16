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
```

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

Only self-built files belong in the repository; the packages are made in `src/pkg.rs`.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the runner and its check harness |
| `src/pkg.rs` | a builder of VSDX packages and the sample drawing |
| `src/samples.rs`, `src/hostile.rs`, `src/corpus.rs` | the groups |
| `src/zipkit.rs` | crafted ZIP archives |
| `tests/vsdx.uis` | what `build_rs.py test` runs (`--check`) |

# PaintCraft source structure

PaintCraft is one `no_std` application crate with domain modules under `src/`.
`build_rs.py` discovers `src/main.rs` and all nested Rust modules automatically.
There are no flat compatibility aliases at the crate root. Cross-domain imports
name their owner explicitly, for example `crate::document::model::DocId`.

```text
apps/PaintCraft/
  src/
    main.rs                 Entry macro and domain declarations
    app/
      mod.rs                App descriptor, JFLib adapter and launch
      state.rs              Pn: shared editor state and deferred UI updates
      lifecycle.rs          Startup, frame passes and command routing
      commands.rs           Command/menu tables and command-state helpers
      constants.rs          Command, widget, window and tool identifiers
      formatting.rs         Bounded UTF-8 UI formatting
    document/
      model.rs              Documents, layer records and document identity
      layers.rs             Layer operations
      composite.rs          Canvas compositing and preview hooks
      history/              Undo/redo, memory accounting and diagnostics
      selection.rs          Selection geometry and masks
      clipboard.rs          Clipboard editing
      operations.rs         Resize, crop and other image operations
      color.rs              Color management
    tools/
      mod.rs                Tool dispatch, shared options and live edits
      brush.rs, text.rs, …   Individual interactive tools
      brushes/              Stroke engine, bitmap tips, preset parser and preset UI
      shapes_library.rs     Built-in and imported shape definitions
    effects/                Effect execution, preview controls, curves/levels
    workspace/
      chrome.rs, view.rs    Main window and canvas presentation
      tabs.rs, palette.rs   Document tabs and palette controls
      icons/                Icon renderer and generated artwork
      dialogs/              Image, layer, save and settings dialogs
      panels/               Tools, colors, history, layers and docking
    integrations/
      file_io.rs            Open/save workflow and codec integration
      plugins/              Native loader and browser implementation
      paint_dot_net/        Managed plugin catalog and external host bridge
      gmic.rs               Standalone G'MIC-Qt bridge
      printing.rs           Printing workflow
      services.rs           Acquisition and update services
    preferences/            Persistent settings and localization
    diagnostics/            Script diagnostics and crash reporting
  docs/                     User/developer documentation and embedded help
  locales/                  Translation catalogs
  res/                      Platform resources
  resources/brush-factory/   Original MIT-licensed brush images and license
  tests/                    UI regression scripts
  update_check.py            Embedded update helper
```

## Responsibilities and dependencies

`main.rs` only declares the application modules and invokes the entry macro.
`app::run` owns `Rc<RefCell<Pn>>`, implements the JFLib `App` callbacks and
flushes requested layout/render changes after each callback. `app::lifecycle`
retains the startup order, command ranges and broadcast rules. The refactor does
not change command IDs, settings keys, file formats or the native plugin ABI.

Each domain keeps its own state and operations. `Pn` assembles those state
objects, so existing editing workflows can still coordinate document changes,
history and rendering in one transaction. The domains remain coupled through
that application state; they are not independent reusable crates. New pure
algorithms should take the narrow state they need instead of `&mut Pn`.

`integrations::plugins` selects its private `native` or `web` implementation with
`cfg` and exports the same internal API. Operating-system implementations remain
in the shared `plat` crate. Shared text rendering and codecs stay in their
existing library crates:

- `lib/text_shape/Cargo.toml` is the standalone Cargo crate for pinned OpenType
  and Unicode dependencies, with its implementation in `src/lib.rs`, integration
  tests in `tests/`, and dependency notices in `NOTICE.md` and `licenses/`.
- `lib/text/shaping.rs` adapts shaping to JFLib text rendering.
- `lib/img_all/psd/mod.rs` provides the composite codec;
  `lib/img_all/psd/document.rs` handles raster-layer PSD/PSB documents. The public
  `img_all::psd::document` API is unchanged.

The application keeps the repository's direct-rustc build. A Cargo workspace
containing artificial manifests for every shared JFLib library is not needed
for this source layout. Run `python build_rs.py ide` from the repository root to
regenerate local rust-analyzer paths after moving sources.

## Build and validation

Run from the JFLib root:

```sh
python build_rs.py check
python -m unittest discover -s tools/tests -p test_project_sources.py
python build_rs.py app PaintCraft --target both
python build_rs.py app PaintCraft --target web --web-mode scalar
cargo test --locked --manifest-path lib/text_shape/Cargo.toml --target-dir build/shaping-tests
```

For the shaping tests, set `PAINTCRAFT_SHAPING_FONT` to an installed font with
Latin, Arabic and Hebrew coverage (for example `C:\Windows\Fonts\arial.ttf`
on Windows).

The desktop UI scripts remain in `tests/`. The PSD, creative-tool and
text/language runners remain under `tools/paintcraft_*_check.py`; their inputs and
commands are documented in [MANUAL_PARITY.md](MANUAL_PARITY.md). Source discovery
also fingerprints the embedded help, language catalog and update helper even
though these resources live outside `src/`.

`SKELETON.md` and older milestone/security reports describe historical snapshots.
Their assembly paths and line numbers are historical references; use this guide
and current source definitions to locate today's implementation.

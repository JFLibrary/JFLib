# PaintCraft host for original Paint.NET plugins

This Windows helper executes **unmodified Paint.NET effect plugins** of both
generations (the classic `Effect` and the v5 `BitmapEffect` / `GpuEffect` with
their PropertyBased forms) and `FileType` / `IFileTypeFactory` / `IFileTypeFactory2` plugins. PaintCraft
distinguishes managed PE assemblies from native PNAPI DLLs before choosing the
loader. PropertyBased effects use a typed PropertyGrid editor backed by their
original PropertySystem values and rules. Other effects may provide their own
`EffectConfigDialog` or `EffectConfigForm`. There is no live preview in this helper.
Native Linux PaintCraft can also invoke it through a configured Wine installation.

## Use

Install Paint.NET 5.1 and PaintCraft from its Windows setup; nothing else is needed.
The setup ships the helper in `pdn-host` beside the program. It is our own assembly:
Paint.NET's assemblies remain in the user's installation, and the project uses
references with `Private=false`, so none are redistributed.

PaintCraft runs the helper on the runtime Paint.NET ships (`PaintCraft.exe --pdn-clr
<paint.net folder> <helper dll> ...` loads its `coreclr.dll`, `lib/plat/win_clr.rs`), so
the machine needs no .NET installation. The folder comes from `--pdn-install` or
`PAINTDOTNET_DIR`, then the installer's `HKLM\SOFTWARE\paint.net\TARGETDIR`, then
`%ProgramFiles%/paint.net`. `--pdn-dotnet` (or a helper `.exe` in `--pdn-host`) selects
an installed .NET 9 Desktop Runtime instead; that route sets `DOTNET_ReadyToRun=0`.

When plugins cannot run, the empty **Effekte > Paint.NET-Plugins** menu says why:
Paint.NET missing, helper missing, Linux without the Wine bridge, plugins turned off,
the search still running or failed, or only load errors. A normal start searches in
the background; `--pdn-plugins` and `--pdn-isolated` search before the window opens, so
a file given to that start can use a plugin's format.

Extract plugin archives first. Copy effect DLLs to `Effects`, file-type DLLs to
`FileTypes` (a plugin with dependencies in a subfolder of its own), and XAML shapes
to `Shapes`. Restart PaintCraft afterwards, matching the
[Paint.NET installation workflow](https://paint.net/doc/latest/InstallPlugins.html).

## Build

`python build_rs.py app PaintCraft --target win` and `tools/package.py` build the helper
into `<out>/win/pdn-host` when `dotnet` (the SDK) and Paint.NET exist on the machine;
otherwise they print a note and go on without it. `PaintDotNetDir` names another
Paint.NET folder. Intermediate SDK files go under the root's `build/paintcraft-pdn-host`.
Update bundles carry it with the program (`tools/package.py`); a setup built on a machine
without Paint.NET ships no helper. Linux packages never contain it.

## Where plugins are found

On Windows the host discovers these folders under the portable/native plugin
root (`--plugins`, `PDN_PLUGIN_DIR`, or beside the executable), Documents / PaintCraft
App Files, Documents / Paint.NET App Files, and the local Paint.NET installation.
It also honours Paint.NET's enabled AdditionalPluginDirectoryRoots registry setting,
including environment expansion. `--pdn-plugins DIR` adds a root; `--pdn-isolated`
suppresses Documents, installation and registry roots for reproducible tests.
`--no-plugins` suppresses discovery. Nothing is copied into the Paint.NET installation.

Compatible effects appear in **Effekte > Paint.NET-Plugins**. The existing
**Paint.NET-Effekt öffnen…** picker can also run an individual DLL. File types add
their loading and saving extensions to the file dialogs; extension capabilities
are kept separate. XAML shapes join the gallery. **Einstellungen > Plugin-Fehler**
lists assembly loading errors.

`Effects` and `FileTypes` are searched four folders deep; hidden, system and linked
folders are skipped. Only assemblies that reference a `PaintDotNet*` assembly count, so
a plugin's dependencies are not treated as plugins. Each candidate is inspected in its own
process (15-second limit, four at a time). A catalog has a
60-second overall limit, at most 256 DLLs, 128 effects and 32 external codec entries.
These codec slots are separate from the eight native PNAPI format slots. Native
formats keep priority when a plugin advertises an already built-in extension.

PaintCraft supplies the active layer, selection, primary/secondary colors and
brush width. It validates the returned size and document version, blends through
the original selection coverage, and creates one undo step. Cancel terminates
the child process tree; a failed or malformed result does not change pixels.
Process isolation is crash isolation, not a security sandbox for untrusted code.

## Compatibility boundaries

Supported and tested: the classic Effect API, PropertyBasedEffect parameters,
region selection, alpha, cancel, undo/redo. Tested with installed Paint.NET's
`InvertColorsEffect` and BoltBait's original `Pastel.dll` / `OilPaintingPlus.dll`
from the author's 6.11 package. The latter two also pass parameter-editor creation.

The v5 API is tested with our own `tests/EffectFixture.cs` through the menu
(`apps/PaintCraft/tests/integration/pdn_modern.uis`): `PropertyBasedBitmapEffect` with
integer and floating-point output, `BitmapEffect<TToken>` with its own form, a plain
`BitmapEffect`, `PropertyBasedGpuImageEffect` (a built-in Direct2D effect and Paint.NET's
own `PdnBrightnessContrastEffect`, a registered pixel shader) and
`PropertyBasedGpuDrawingEffect`, on the default GPU and on WARP, with selection,
undo/redo and cancel. For the same path under Wine see [Linux/Wine setup](#linuxwine-setup).

Real plugins, run once by hand on Paint.NET 5.1.12 (2026-11-03; downloads are not part of the
test suite):

- **BoltBait's pack 6.11:** all 47 effects (19 classic, 28 GPU) render with default settings on the
  whole image and in a selection, none changes pixels outside it, and all GPU effects also run
  on WARP (rounding differs by a step in some pixels). All editors build; a full catalog takes 3 s
  on an idle machine. 11 of them also ran through the menu with undo, and one with a
  selection and redo. Paste Alpha and From Clipboard read a real clipboard image.
- **PdnGpuEffectSamples:** all 20 samples render, including those with their own ComputeSharp pixel
  shaders. They target Paint.NET 5.2 and were built for 5.1 with `ComputeSharp.D2D1` 3.1.0, the
  library beside the plugin DLL.
- **WebPFileType 1.6.0:** catalog, encode and decode with alpha through the helper. PaintCraft's own
  WebP codec keeps priority in its dialogs, so the plugin is not used for that extension there.
- **PSFilterPdn 2.0.11:** found in its own folder with its dependencies skipped; its dialog opens,
  searches its folder and cancels. **No 8bf filter has been run**: none was available, so its filter
  process `PSFilterShim.exe`, previews and the checkerboard stayed unused. Rendering without a chosen
  filter ends in a plugin error, as seen with `--defaults`.

These runs check defaults and that results look right, not that pixels equal Paint.NET's own
output. Parameter values other than defaults were not swept.

The FileType bridge is tested with a separately compiled factory fixture through
the original Paint.NET API, including distinct read/write extensions, alpha, DPI,
round trips, malformed packets and a broken neighbouring DLL. It imports a
flattened image and resolution as a new, unsaved document. Saving uses the
plugin's default token; custom save dialogs, layers, ICC/EXIF/XMP transport through
this bridge and application-specific host services are not implemented. This
fixture does not establish compatibility with every third-party file-type DLL.

Boundaries of the v5 path:

- **Parameters without IndirectUI's controls.** The host reads a plugin's `OnCreateConfigUI`
  through the internal `ControlInfo.Initialize` and shows its labels, descriptions, value names
  and order in the plain PropertyGrid, with the range as text, colors as `#RRGGBB` with a color
  dialog, and coordinates as `x; y`. Paint.NET's own controls stay out of reach (they need
  `PdnSynchronizationContext` and more of the application), so there are no sliders, color
  wheels, sections or tabs, and links are left out. A label the plugin leaves empty shows the
  internal name; so do all of them when its UI code fails. An effect whose own
  `OnCreateConfigForm` builds on those controls fails with that error.
- **Application services.** Classic and v5 effects get `IDirect2DFactory`, `IImagingFactory`,
  `IDirectWriteFactory`, `IEnumLocalizerFactory` (a plugin's enum values keep their names),
  `IClipboardService` (Paint.NET's own, given a window it needs), `IPalettesService` (a fixed palette
  of 96 colors, not the user's), `IUserFilesService` (a folder of ours under
  `%LOCALAPPDATA%/PaintCraft/pdn-host`, so a plugin's settings there are not the ones it has in
  Paint.NET), `IFileSystemService` (the plugin folders above), `IExceptionDialogService` (a message
  box), `IShellService` (opens links and folders), `IVisualStylingService` (a fixed light
  checkerboard) and the settings below. A plugin requiring another service (file types) gets `null`
  or Paint.NET's "service not found" error, shown as a plugin error. An exception in a plugin's own
  dialog shows the framework's "Unhandled exception" box with Continue.
- **A plugin's own libraries.** Each plugin gets its own load context in which a library beside its
  DLL wins, except `PaintDotNet*`, `System*`, `Microsoft*` and WPF, which stay shared. This matters:
  Paint.NET's `ComputeSharp.D2D1.dll` is trimmed and lacks what plugin shaders need, so a shader
  plugin must bring or merge its own copy. Such a copy must fit Paint.NET's public types.
- **Parts that are not public API.** The GPU renderer asks `ISettingsService` for
  `Graphics/RenderingGpuChoice` and needs the internal `GpuChoice` and `GpuInfoCacheService`.
  These interfaces are closed with an internal member, so `ModernEffects.cs` generates their
  implementation with `IgnoresAccessChecksTo`. A Paint.NET update can break this: GPU effects then
  report "Diese paint.net-Version passt nicht zum Host", CPU effects do not depend on it. The
  cache file `GpuInfoCache.ini` goes to `%LOCALAPPDATA%/PaintCraft/pdn-host`; Paint.NET's own
  settings are neither read nor changed.
- **GPU choice.** The default GPU, as Paint.NET defines it, regardless of the choice made in
  Paint.NET's settings; `--pdn-gpu software` forces WARP, and a failing GPU falls back to it.
- **Selection.** Effects get a pixelated selection (every pixel with coverage), PaintCraft
  applies the fractional coverage to the result. `DisableSelectionClipping` gains nothing,
  because PaintCraft masks the result anyway. Tiles cover the selection's bounding box.
- **One layer.** The document holds only the active layer: effects reading other layers or
  a composite see this one. No metadata, sRGB assumed.
- **No preview, no progress.** The dialog comes first, then the result.

Process isolation is crash isolation only: a plugin runs with the user's rights, on the
GPU as on the CPU, and can do what any program can.

The current API dependency was tested with assembly version `5.112.9563.32325`
(Paint.NET 5.1.12, .NET 9). Paint.NET updates can change these APIs; rebuild and retest
the host after such an update. Only a Paint.NET folder with `coreclr.dll` runs without an
installed .NET; the Microsoft Store edition has not been tried. Under Wine the helper also runs
on the 5.2 Beta in the Paint.NET-on-Wine package; 5.2 under Windows has not been tried.

## Can Windows DLLs run on Linux?

The `.dll` suffix alone does not decide portability: a managed .NET assembly can
contain portable code. Paint.NET plugins also rely on Paint.NET's Windows APIs,
WinForms/COM or GPU services, however. They cannot be loaded by the native Linux
PNAPI `.so` loader or made portable by renaming the file.

PaintCraft now implements and tests that external Wine-helper architecture.
An unmodified BoltBait `Pastel.dll` produced identical changed pixels in direct
Windows and Linux/Wine tests. The native Linux editor also passed catalog/menu,
effect undo/redo, FileType alpha/DPI round-trip, cancellation and XAML shape tests.
Dynamic Draw v4.0 passed original DLL loading, dialog construction and the render
entry point on both platforms. These checks do not test interactive painting,
tablet input, or every plugin-owned dialog and application service.

## Linux/Wine setup

Use a private Wine prefix and the experimental
[Paint.NET-on-Wine package](https://github.com/paintdotnet/Paint.NET-on-Wine).
The package replaces what Wine lacks with managed implementations. `paintdotnet.exe` installs them at
its start when `PaintDotNet.Configuration.json` says `EnableWineMode`; the helper starts instead of that
program and does the same (`WineMode.cs`: managed imaging and Direct2D, DWriteCore, the alternate WARP
lookup). As the package's `paintdotnet.sh` does, PaintCraft sets `WINEDLLOVERRIDES=mshtml=;d3dcompiler_47=n`
unless the variable is already set: Wine's own d3dcompiler cannot reflect the shaders of GPU effects.
The regular Windows Paint.NET 5.1 package failed its Direct2D color-profile initialization in our Wine
test. Merely installing Wine around that package is therefore insufficient. The package's minimum of
Wine 11.20 is checked by `paintdotnet.exe`, not by the helper; a prefix set up with
`wineboot -i` and `winecfg -v win11` is enough for it, DXVK is not needed (one GPU effect gave the
same pixels with and without it).

Tested on 2026-11-03 with the helper built against the Windows 5.1 API, the package's bundled
.NET 11 runtime and default settings, against the same plugins on Windows (WARP for GPU effects):

- **Fedora 44, WineHQ 11.21, Paint.NET-on-Wine 5.200.9793.3533: works.** Same catalog as on Windows
  (19 classic, 28 GPU, PSFilterPdn, WebP). No effect changes pixels outside a selection.
  - **Classic:** 15 byte-identical. `Complementary Colors` and `Polygon / Stars` differ in what they
    draw as text and lines. `Paste Alpha` runs with an empty clipboard; a real clipboard image was not
    tried. `Dimensions` fails with `Font 'Arial' cannot be found`, the prefix has no such font.
  - **GPU:** 20 byte-identical or within one step in some pixels, as on WARP against a GPU. `Flames`
    differs by up to 5 steps, `Feather Object` and `Feather Selection` by more in about 2000 pixels
    without a difference to see. `Level Horizon` differs in the glyphs it draws. `Calendar`,
    `Meme Maker` and `Text Fun Factory` use other fonts and break their text differently, which shows.
    `From Clipboard` runs with an empty clipboard.
  - **PaintCraft:** through the menu Drop Shadow+, Transparency, Vignette+ (undo, redo), Pastel (the
    same checksum as on Windows), Flip Horizontal, and cancelling a GPU effect without leftover
    processes. A catalog takes about 30 s on a Pentium G4560T, in the background on a normal start.
  - **Dialogs**, on a KWin without a screen: Vignette+, Drop Shadow+, Pastel and BBChart open and apply;
    PSFilterPdn opens and cancels, no 8bf filter was run. WebP encodes and decodes through the helper.
- **Ubuntu 26.04, its Wine 10.0: works** for Pastel (byte-identical), Vignette+ and Transparency
  (within one step); the host alone, without windows or PaintCraft.
- **Ubuntu 24.04 / WSL, Wine 11.19, Paint.NET-on-Wine 5.200.9775.2252: works** for classic effects
  (an earlier test, before the v5 path).
- **Fedora's package wine 11.20 Staging: does not work.** It stops with `Cannot get symbol
  u_charsToUChars from libicuuc` (its prefix has `icuuc.dll` and `icuin.dll` but no `icu.dll`, unlike
  WineHQ 11.21). With `DOTNET_SYSTEM_GLOBALIZATION_USENLS=1` it fails later: `Bgra32 is not a
  supported pixel format` or `Unable to load DLL 'd2d1'`. Whether this is Fedora's build or 11.20
  itself is open.

The experimental Paint.NET builds expire 12 weeks after they were built;
follow their release instructions and retest after updating. Neither Wine nor
Paint.NET is bundled with PaintCraft. Linux packages do not contain the helper either: it is built on
Windows against a Paint.NET installation and deployed as below.

After building the Windows helper, deploy its files into your **private unpacked**
Paint.NET-on-Wine directory. This copies only `PaintCraft.PdnHost.dll`, its optional
Windows `.exe` launcher, and two helper-named runtime/dependency manifests derived
from that exact Paint.NET package. Existing Paint.NET binaries and settings are
left intact.

```sh
python3 tools/paintcraft-pdn-host/deploy_wine.py \
  --host /path/to/built/PaintCraft.PdnHost.dll \
  --paintnet "$HOME/opt/paintnet-wine"

./build/linux/PaintCraft \
  --pdn-wine /usr/bin/wine \
  --pdn-wine-prefix "$HOME/.wine-paintcraft" \
  --pdn-host "$HOME/opt/paintnet-wine/PaintCraft.PdnHost.exe" \
  --pdn-install "$HOME/opt/paintnet-wine" \
  --plugins "$HOME/PaintCraft Plugins"
```

The `.exe` launcher uses the package's bundled runtime and needs no separate
`dotnet.exe`. If deploying only the helper DLL, use `--pdn-host .../PaintCraft.PdnHost.dll`
and `--pdn-dotnet 'C:/Program Files/dotnet/dotnet.exe'` for an installed Windows
.NET launcher inside the prefix. The native Linux `dotnet` executable cannot run
this Windows/WinForms host.

Environment alternatives: `PAINTCRAFT_PDN_WINE`, `WINEPREFIX`,
`PAINTCRAFT_PDN_HOST`, `PAINTCRAFT_PDN_DOTNET`, and `PAINTDOTNET_DIR`.
Command-line options take precedence. Wine is only invoked when explicitly
configured. Paths are passed as individual arguments, preserving spaces and
Unicode. The mapping assumes the standard Wine `Z:` → `/` and `C:` →
`$WINEPREFIX/drive_c`; custom drive mappings and UNC shares are not supported.

The transport pumps stdin/stdout/stderr concurrently, bounds responses and
diagnostics, and applies time limits. Besides terminating the native child
process group, it updates a private heartbeat file. Host and catalog workers
exit when it stops changing for five seconds or disappears, including Wine
workers started through wineserver. A stalled codec survived continuously updated
heartbeats and exited about five seconds after they stopped in Windows and Wine
tests, without publishing image bytes. Isolation is not a security sandbox.

The native G’MIC integration works independently of this Windows host; see
[its setup guide](../../apps/PaintCraft/docs/CREATIVE_TOOLS.md).

## Tests and protocol

```powershell
python tools/paintcraft_manual_check.py --out build/paintcraft-parity --target win --pdn-host
```

This builds `tests/EffectFixture.csproj` and `tests/Fixture.csproj` itself and runs
`pdn_host.uis`, `pdn_modern.uis`, the menu's reasons, the background search (a named folder
searched like a normal start with `--pdn-background`), and the host directly with no .NET on
the PATH. Scripts otherwise skip the search of a normal start, so they do not depend on what
the machine has installed.

For direct diagnostics, the helper takes `plugin.dll [fully.qualified.Type]`,
`--list` enumerates effect types, `--defaults` skips editing, `--gpu software` forces
WARP, and `--validate-dialog` constructs the editor without showing it and without
reading an image. Normal runs read
PCFX from stdin: six little-endian 32-bit values (magic `0x58464350`, width, height,
primary, secondary, brush-width float bits), straight BGRA pixels, then A8 selection
coverage. Output is magic, width, height and straight BGRA. Exit 2 means cancelled;
other nonzero exits report the error on stderr. Dimensions are limited to 100M
pixels and 65535 per axis. Successful output is emitted only after rendering.

Catalog diagnostics: `--catalog --root DIR [--root DIR] [--isolated]`; `DLL --inspect`
inspects one assembly. Catalog lines have tab-separated kind, hex UTF-8 path/type/name,
capability flags, and hex UTF-8 extensions. Kinds E/F/S/U/X mean effect (flags: 0 classic,
1 v5 CPU, 2 GPU), file type, shape, unsupported effect, and error. `DLL FactoryType#0 --decode`
reads encoded bytes and returns PCIM: six little-endian UInt32 values (magic
0x4D494350, width, height, horizontal/vertical DPI in 16.16, original layer count),
then straight BGRA. `--encode` consumes PCIM and returns encoded bytes. Both paths
publish results only after success; the PaintCraft file layer performs the atomic
write. Codec calls have a two-minute limit and honour save/decode cancellation.

Additional checks:

```powershell
python tools/paintcraft_remaining_check.py --out build/paintcraft-parity --target both --plugins
```

The Linux suite takes explicit paths and never downloads dependencies:

```sh
python3 tools/paintcraft_wine_check.py --out build/paintcraft-next \
  --wine /usr/bin/wine --prefix "$HOME/.wine-paintcraft" \
  --host "$HOME/opt/paintnet-wine/PaintCraft.PdnHost.exe" \
  --paintnet "$HOME/opt/paintnet-wine" \
  --pastel /path/to/original/Pastel.dll \
  --fixture /path/to/built/Fixture.dll
```

Build the FileType fixture with `dotnet build tests/Fixture.csproj` from this
directory. `tools/paintcraft_pdn_watchdog_check.py --help` describes the separate
heartbeat test against its blocking decoder. `tests/PresetsFixture.csproj`
exports JSON and DataContract XML with an original Dynamic Draw DLL's own types;
the optional `--original-plugin` argument to `tools/paintcraft_brush_preset_check.py`
then imports and paints with both files on Windows and Linux.

# GPU -- explizite GPU-Schicht für UI, 2D-CAD und 3D-Spiele

`lib/gpu` gibt Programmen direkten Zugriff auf die Grafikkarte: Rechen-Shader, Rendern mit
Tiefentest, indirekte Aufrufe, Zeitstempel und Fenster-Swapchains. Die API ist explizit wie
Vulkan und WebGPU (Instanz, Adapter, Gerät, Warteschlange, Ressourcen, Pipelines,
Befehlspuffer, Barrieren) und neutral gegenüber dem Backend: das Backend wird pro Ziel beim
Übersetzen gewählt, es gibt keine dynamische Verteilung.

Stand (G1a): Vulkan unter Windows. `vulkan-1.dll` wird zur Laufzeit geladen, das Programm
startet also auch auf Rechnern ohne Vulkan und bekommt dort einen sauberen Fehler. Geplant:
Vulkan unter Linux und Android, WebGPU im Browser, später Metal -- mit denselben Typen und
Methoden. Die Oberfläche rastert weiter auf der CPU; ihre Darstellung über die GPU folgt mit G3.

| Teil | Wo |
| --- | --- |
| Öffentliche Typen: Beschreibungen, Formate, Fehler, Features, Limits, `Pod` | [`lib/gpu/lib.rs`](../lib/gpu/lib.rs) |
| Instanz, Adapter, Validierung | [`lib/gpu/vk_inst.rs`](../lib/gpu/vk_inst.rs), Laden der DLL: [`vk_load.rs`](../lib/gpu/vk_load.rs) |
| Gerät, Warteschlange, Zeitleiste, verzögertes Freigeben, Uploads | [`lib/gpu/vk_dev.rs`](../lib/gpu/vk_dev.rs) |
| Speicher-Unterverteilung | [`lib/gpu/vk_mem.rs`](../lib/gpu/vk_mem.rs) |
| Puffer, Texturen, Sampler, Shader, Bindungen, Bindless, Pipelines, Zeitstempel | [`lib/gpu/vk_res.rs`](../lib/gpu/vk_res.rs) |
| Befehle aufzeichnen | [`lib/gpu/vk_cmd.rs`](../lib/gpu/vk_cmd.rs) |
| Fensterflächen und Swapchains | [`lib/gpu/vk_swap.rs`](../lib/gpu/vk_swap.rs) |
| Vulkan-Deklarationen (von Hand, mit Layout-Prüfungen gegen die Header) | [`lib/gpu/vk_ffi.rs`](../lib/gpu/vk_ffi.rs) |
| Fenster-Handles für die GPU | `plat::win_native` ([`lib/plat/win_api.rs`](../lib/plat/win_api.rs)) |
| Tests und Messungen | [`apps/gputest`](../apps/gputest/main.rs) |

## Ein Beispiel

```rust
let inst = gpu::Instance::new()?;                 // Err(NoLoader) ohne Vulkan, Err(Disabled) mit jfgui_GPU=0
let adapter = inst.adapter()?;                    // diskrete GPU zuerst, jfgui_GPU_ADAPTER wählt eine andere
let dev = adapter.create_device(&gpu::DeviceDesc::DEFAULT)?;   // Stufe Voll, wenn vorhanden
let shader = gpu::ShaderModule::from_spirv(&dev, CS_FILL)?;
let bgl = dev.create_bind_group_layout(&[gpu::Binding {
    binding: 0, kind: gpu::BindingKind::StorageBuffer, stages: gpu::Stages::COMPUTE }])?;
let layout = dev.create_pipeline_layout(&gpu::PipelineLayoutDesc { groups: &[&bgl], push_constants: 12 })?;
let pipe = dev.create_compute_pipeline(&gpu::ComputePipelineDesc {
    layout: &layout, shader: gpu::ShaderStage { module: &shader, entry: "main" } })?;
let buf = dev.create_buffer(&gpu::BufferDesc { size: 4 << 20, usage: gpu::BufferUsage::STORAGE,
                                               memory: gpu::MemoryKind::Device })?;
let rb = dev.create_buffer(&gpu::BufferDesc { size: 4 << 20, usage: gpu::BufferUsage::NONE,
                                              memory: gpu::MemoryKind::Readback })?;
let group = dev.create_bind_group(&gpu::BindGroupDesc { layout: &bgl, entries: &[gpu::BindEntry {
    binding: 0, resource: gpu::Resource::Buffer { buffer: &buf, offset: 0, size: 0 } }] })?;

let mut e = dev.encoder()?;
e.set_compute_pipeline(&pipe);
e.set_bind_group(0, &group);
e.push_constants(0, &[1u32 << 20, 3, 7]);
e.dispatch(1 << 14, 1, 1);
e.barrier(gpu::Access::COMPUTE_WRITE, gpu::Access::COPY_READ);
e.copy_buffer(&buf, 0, &rb, 0, 4 << 20);
e.barrier(gpu::Access::COPY_WRITE, gpu::Access::HOST_READ);
let id = dev.queue().submit([e.finish()?])?;      // Wert auf der Zeitleiste des Geräts
dev.wait(id, 1_000_000_000)?;
let mut out = vec![0u32; 1 << 20];
rb.read(0, &mut out)?;
```

## API-Überblick

- **Instanz und Adapter**: `Instance::new()` / `with(&InstanceDesc)`, `adapters()` (alle GPUs,
  diskrete zuerst; `AdapterInfo::missing` nennt, warum eine unbrauchbar ist), `adapter()`.
  `AdapterInfo`: Name, Hersteller, Gerätekennung, Art, Treibername und -version, API-Version.
  `adapter.features()`, `limits()`, `level()`, `create_device(&DeviceDesc)`.
- **Gerät**: `queue()`, `features()`, `limits()`, `level()`, `wait(id, timeout_ns)`,
  `wait_idle()`, `poll()`, `completed()`, `is_lost()`, `memory_reserved()`, `test_lose()`
  (Testhilfe).
- **Ressourcen**: `create_buffer` (`MemoryKind::Device` / `Upload` / `Readback`; die beiden
  letzten sind dauerhaft abgebildet: `write`, `read`), `create_texture` (2D, 2D-Array, Würfel,
  Würfel-Array, 3D, Mip-Stufen, Tiefenformate, BC1..BC7 wenn vorhanden; `view()`,
  `create_view(&ViewDesc)`), `create_sampler`, `ShaderModule::from_spirv`.
- **Bindungen**: `create_bind_group_layout(&[Binding])`, `create_pipeline_layout` (höchstens
  4 Gruppen, höchstens 128 Byte Push-Konstanten), `create_bind_group`; auf Stufe Voll
  `create_bindless` (eine große Tabelle: Bindung 0 Texturen, 1 Sampler, 2 Speicherpuffer,
  3 Speichertexturen; `set_texture`, `set_sampler`, `set_buffer`, `set_storage_texture`,
  `group()`, `layout()`).
- **Pipelines**: `create_compute_pipeline`, `create_render_pipeline` (Vertex + Fragment,
  dynamisches Rendern, Tiefentest, Mischen, Culling; Vertex-Eingaben sind optional,
  Vertex-Pulling aus Speicherpuffern geht ohne sie).
- **Befehle**: `device.encoder()`; Kopien (`copy_buffer`, `fill_buffer`,
  `copy_buffer_to_texture`, `copy_texture_to_buffer`), Barrieren (`barrier`,
  `texture_barrier`, `texture_barrier_range`), Rechnen (`set_compute_pipeline`,
  `set_bind_group`, `push_constants`, `dispatch`, `dispatch_indirect`), Render-Durchgänge
  (`begin_render` .. `end_render`, `set_render_pipeline`, `set_viewport`, `set_scissor`,
  `set_vertex_buffer`, `set_index_buffer`, `draw`, `draw_indexed`, `draw_indirect`,
  `draw_indexed_indirect`; auf Voll `draw_indexed_indirect_multi` und
  `draw_indexed_indirect_count`), Zeitstempel (`reset_timestamps`, `timestamp`); `finish()`.
- **Warteschlange**: `submit([cmd, ..])` gibt eine `SubmitId` (Zeitleisten-Semaphor);
  `write_buffer` und `write_texture` werden über Staging-Speicher vor der nächsten Übergabe
  ausgeführt; `read_buffer` (kopiert, übergibt und wartet: der langsame Weg für Werkzeuge und
  Tests); `flush()`.
- **Zeitstempel**: `create_timestamps(n)`, `read_ticks`, `read_ms` (Millisekunden seit dem
  ersten gelesenen Zeitstempel des Geräts, damit `f64` jeden Takt behält); `Err(NotReady)`,
  solange nicht alle geschrieben und fertig sind.
- **Fenster**: `Surface::new(&instance, plat_fenster)`, `create_swapchain(&surface,
  &SwapchainDesc)`, `acquire()` (`None`, solange das Fenster minimiert ist oder keine Fläche
  hat), `view(&frame)`, `texture(&frame)`, `present(frame, [cmd, ..])`, `resize()`,
  `set_present_mode(m)`, `size()`, `format()`, `present_mode()`.

## Konventionen

Sie gelten für jedes Backend und für den WGSL-Übersetzer:

- **Koordinaten** wie WebGPU: normierte Gerätekoordinaten mit y nach oben, Tiefe 0 bis 1,
  Ursprung von Framebuffer und Texturen oben links. Das Vulkan-Backend kippt dafür den Viewport
  (negative Höhe); Shader kippen nichts. Vorderseiten sind gegen den Uhrzeigersinn
  (`front_cw` dreht das um).
- **Bindungen**: Gruppe g, Bindung b einer Pipeline-Struktur = WGSL `@group(g) @binding(b)` =
  SPIR-V `DescriptorSet g`, `Binding b`. Höchstens 4 Gruppen.
- **Push-Konstanten**: ein Block ab Offset 0, für alle Stufen sichtbar, höchstens
  `PUSH_CONSTANTS_MAX` = 128 Byte. Ein späteres WebGPU-Backend bildet sie über einen kleinen
  Uniform-Puffer nach.
- **Zustände von Texturen** (`Layout`): `Undefined`, `General`, `CopySrc`, `CopyDst`,
  `Sampled`, `ColorTarget`, `DepthTarget`, `DepthRead`, `Present`. Der Aufrufer sagt, in
  welchem Zustand eine Textur ist und in welchen sie soll (`texture_barrier(t, vorher,
  nachher)`); Anhänge eines Render-Durchgangs geben `before` und `after` an, die Übergänge macht
  `begin_render` / `end_render`. Neue Texturen und Swapchain-Bilder beginnen in `Undefined`;
  ein Swapchain-Bild endet in `Present`.
- **Barrieren für Puffer** sind globale Speicherbarrieren: `barrier(Access::COMPUTE_WRITE,
  Access::COPY_READ)` -- was vorher fertig sein muss, was danach wartet.
- **Gruppen und Push-Konstanten** gehen an die zuletzt gesetzte Pipeline: außerhalb von
  Render-Durchgängen an die Rechen-Pipeline, innerhalb an die Render-Pipeline (Pipeline zuerst
  setzen).

## Feature-Stufen

| Stufe | Bedeutung | Vulkan-Voraussetzungen |
| --- | --- | --- |
| Basis | was WebGPU und Vulkan 1.1 können: Rechen-Shader, Speicherpuffer, einzelne indirekte Aufrufe, Textur-Arrays, Zeitstempel wo vorhanden | Vulkan 1.1, eine Warteschlange für Grafik und Rechnen, Zeitleisten-Semaphoren, dynamisches Rendern (Kern in 1.3 oder `VK_KHR_dynamic_rendering`) |
| Voll | zusätzlich Bindless-Tabellen, `draw_indexed_indirect_count`, mehrere indirekte Aufrufe, Draw-Parameter, Subgruppen-Operationen | Vulkan 1.3 mit Descriptor-Indexing (update-after-bind, partially bound, non-uniform), drawIndirectCount, multiDrawIndirect, drawIndirectFirstInstance, shaderDrawParameters, Subgruppen (basic, vote, ballot, arithmetic, shuffle) im Rechen-Shader |

`device.level()` und `device.features()` sagen, was da ist. Mesh-Shader und Raytracing werden
in `Features` gemeldet (`mesh_shader`, `ray_query`, `ray_pipeline`), aber noch nicht
eingeschaltet. `DeviceDesc { max_level: Level::Basis }` erzwingt ein Basis-Gerät (zum Testen
der Basis-Pfade auf großer Hardware).

## Fehler und verlorene Geräte

Jeder Aufruf gibt `Result` zurück, nichts löst eine Panik aus. `GpuError`: `Disabled`,
`NoLoader`, `NoAdapter`, `Unsupported`, `Invalid`, `OutOfMemory`, `DeviceLost`, `SurfaceLost`,
`Timeout`, `NotReady`, `Failed(VkResult)`. Fehlbenutzung, die das Backend billig erkennen kann
(Größen, Bereiche, Ausrichtungen, Zustände, ein Draw ohne Pipeline, eine Gruppe mit falscher
Struktur, ein Voll-Befehl auf Basis), ergibt `Invalid`; aufgezeichnete Befehle melden den ersten
Fehler bei `finish()`. Ein verlorenes Gerät (Treiber-Reset, GPU entfernt) ergibt bei jedem
späteren Aufruf `DeviceLost`; dann das `Device` fallen lassen und vom Adapter ein neues anlegen.

## Speicher und Lebensdauer

Ein einfacher Unterverteiler: pro Speichertyp und Belegungsart (Puffer / optimale Texturen)
Blöcke von 64 MiB Gerätespeicher oder 16 MiB Host-Speicher, Anforderungen über einem halben
Block bekommen einen eigenen Block; freie Bereiche werden zusammengelegt. Ressourcen sind
referenzgezählte Griffe. Ein Befehlskodierer leiht, was er aufzeichnet; eine fallen gelassene
Ressource wird zerstört, sobald die GPU alle Übergaben vor dem Fallenlassen erledigt hat (die
Zeitleiste des Geräts). Die CPU wartet nur, wenn man es verlangt (`wait`, `read_buffer`,
`Timestamps::read_*`, Swapchain-Neubau).

## Fenster und Darstellung

Die Swapchain zeigt auf ein gewöhnliches plat-Fenster (`plat::win_create`), das sein Programm
mit eigener Ereignisschleife (`plat::wait`, `plat::evq_pop`) führt und **nie an
`plat::win_present` gibt**. So malt die CPU-Darstellung nichts darüber: ohne eine behaltene Kopie
malt `WM_PAINT` nichts, `WM_ERASEBKGND` löscht nichts, die Fensterklasse hat keinen Pinsel.
Dafür war nur ein Zugriff auf die Fenster-Handles nötig: `plat::win_native(w)` liefert HWND und
HINSTANCE für `VK_KHR_win32_surface`. Die UI-Fenster von `lib/ui` stellen weiter über die CPU
dar; das ändert G3.

Darstellungsmodi: `Fifo` (V-Sync, immer vorhanden, Standard), `Mailbox`, `Immediate` (für
Messungen); fehlt einer, wird `Fifo` genommen. Die Swapchain wird beim nächsten `acquire()` neu
gebaut, wenn sich die Fenstergröße geändert hat, die letzte Darstellung veraltet oder
suboptimal war, `resize()` oder `set_present_mode()` es verlangt hat oder ein Bild nie
dargestellt wurde; vorher wartet sie, bis die Warteschlange leer ist. Minimiert gibt
`acquire()` `None`. Mit `SwapchainDesc::copy_src` lassen sich die dargestellten Bilder
zurücklesen.

## Umgebungsvariablen

| Variable | Wirkung |
| --- | --- |
| `jfgui_GPU=0` (oder `off`) | GPU aus: `Instance::new` gibt `Err(Disabled)` |
| `jfgui_GPU_DLL=name` | anderer Loader statt `vulkan-1.dll` (Name: nur aus System32; Pfad: aus seinem Ordner) |
| `jfgui_GPU_ADAPTER=n` / `=name` | Adapter n von `adapters()` oder der erste, dessen Name das Wort enthält |
| `jfgui_GPU_PRESENT=fifo\|mailbox\|immediate` | Darstellungsmodus jeder neuen Swapchain |
| `jfgui_GPU_VALIDATE=1` | Khronos-Validierungsschicht mit Synchronisationsprüfung, wenn installiert |
| `jfgui_GPU_DEBUG=1` | die Debug-Meldungen des Treibers (ab Info-Stufe) auf stderr |

## Validierung

Das Vulkan SDK ist nur ein Entwicklungswerkzeug; zur Laufzeit braucht `lib/gpu` nichts davon.
Mit installiertem SDK (Schicht `VK_LAYER_KHRONOS_validation`) schaltet `jfgui_GPU_VALIDATE=1`
oder `InstanceDesc { validate: true, .. }` die Validierung ein, samt Synchronisationsprüfung
(`VK_EXT_layer_settings` „validate_sync“, sonst `VK_EXT_validation_features`). Meldungen gehen
über `VK_EXT_debug_utils` als `gpu: validation error: ...` auf stderr;
`gpu::validation_messages()` zählt Fehler und Warnungen. Ohne die Schicht erscheint einmal ein
Hinweis und alles läuft wie ohne die Variable.

```text
set jfgui_GPU_VALIDATE=1
build\win\gputest.exe --test all          (oder: gputest --test all --validate)
```

`tests/validate.uis` lässt jeden Test einmal mit Validierung laufen; ein Validierungsfehler
lässt ihn scheitern. Der Test `syncval` baut absichtlich eine fehlende Barriere ein und erwartet,
dass die Synchronisationsprüfung sie meldet -- so ist sicher, dass sie wirklich an ist.
Die selbst geschriebenen SPIR-V-Module von gputest sind mit `spirv-val --target-env vulkan1.1`
und `vulkan1.3` geprüft.

Achtung bei Messungen: Der Vulkan Configurator (`vkconfig`) kann Schichten für alle Programme
erzwingen (eine Loader-Einstellungsdatei unter `%LOCALAPPDATA%\LunarG\vulkan`), etwa
GFXReconstruct, das jeden Aufruf aufzeichnet. `VK_LOADER_LAYERS_DISABLE` hebt das nicht auf;
für Messungen den Configurator ausschalten.

## Tests

`python -u build_rs.py test gputest` (nur Windows: `apps/gputest/TARGETS`) führt
`apps/gputest/tests/*.uis` aus:

| Skript | Prüft |
| --- | --- |
| `info.uis` | Adapter, Treiber, API-Version, Stufe, Features, Limits; ein auf Basis begrenztes Gerät |
| `compute.uis` | Rechen-Shader schreibt f(i) in einen Speicherpuffer, 1000 und 1M Elemente, die Elemente dahinter unberührt; `write_buffer` / `read_buffer` |
| `copies.uis` | Puffer → Textur → Puffer mit aufgefüllten Zeilen; `write_texture` in jede Mip-Stufe und Schicht eines Arrays, Rücklesen pro Mip |
| `graphics.uis` | zwei Dreiecke (Vertex-Pulling, Indexpuffer, Rückseiten-Culling, Tiefentest) in eine Textur; Pixel geprüft |
| `indirect.uis` | Argumente von einem Rechen-Shader: `dispatch_indirect`, `draw_indexed_indirect`, auf Voll `_multi` und `_count` |
| `timestamps.uis` | monoton über zwei Übergaben, plausibel gegen die CPU-Uhr |
| `bindless.uis` | (Voll) Puffer und Texturen über eine Tabelle; ein Platz ersetzt |
| `window.uis` | Fenster mit Swapchain: FIFO-Bilder, dargestelltes Bild zurückgelesen, Größe geändert, minimiert / wiederhergestellt, IMMEDIATE und MAILBOX |
| `errors.uis` | `jfgui_GPU=0`, fehlender Loader, `jfgui_GPU_ADAPTER` ohne Treffer, ungültige Aufrufe (Fehler, kein Absturz), verlorenes Gerät und ein neues danach |
| `validate.uis` | alles einmal mit Validierungsschicht (siehe oben) |

Ohne Vulkan oder brauchbare GPU melden die Tests `SKIP` und gelten als bestanden.
`gputest --bench` misst die Zahlen unten.

## Messwerte

GeForce RTX 2070 SUPER, Treiber 610.88 (Vulkan 1.4.341), Ryzen 5 3600, Windows 11, 165-Hz-Monitor;
`gputest --bench`, Entwicklungs-Build, ohne Validierung (Mittelwerte mehrerer Läufe):

| Messung | Wert |
| --- | --- |
| Instanz anlegen | erstes Mal 440 .. 880 ms (Laden von Loader, Treiber und Schichten), danach 23 .. 32 ms |
| Adapter aufzählen | 1,5 .. 3 ms |
| Gerät anlegen | erstes Mal 70 .. 110 ms, danach 65 .. 70 ms |
| Swapchain anlegen (320 × 240) | erstes Mal 430 .. 700 ms, danach 58 .. 62 ms |
| Löschen und Darstellen, FIFO | 6,06 ms pro Bild (165 Bilder/s = Bildwiederholrate) |
| Löschen und Darstellen, MAILBOX | 3,1 ms pro Bild (320 Bilder/s) |
| Löschen und Darstellen, IMMEDIATE | 0,49 .. 0,57 ms pro Bild (1750 .. 2050 Bilder/s), Median 0,39 ms |
| 1M-Rechen-Rundreise (aufzeichnen, übergeben, warten, 4 MiB lesen) | 1,0 .. 1,5 ms (Median), davon GPU: Füllen 0,017 ms, Kopie 0,32 ms; Lesen auf der CPU 0,5 ms |

## Bekannte Lücken

- Nur Windows. Linux und Android brauchen einen Weg zu `libvulkan.so` (JFLib lädt unter Linux
  ohne libc) und ihre Fensterflächen; das Backend übersetzt dort, `Instance::new` gibt
  `NoLoader`.
- Eine Warteschlange; keine eigene Rechen- oder Kopierwarteschlange.
- Keine Spezialisierungskonstanten, kein Pipeline-Cache, kein MSAA, keine Stencil-Operationen,
  keine Textur-zu-Textur-Kopien und kein Blit (Mip-Erzeugung).
- Bindungen ohne Arrays (`count` 1); Arrays gibt es über die Bindless-Tabelle auf Stufe Voll.
- Uploads legen pro Übergabe mindestens einen 4-MiB-Staging-Bereich an; ein Ringpuffer für
  viele kleine Schreibvorgänge pro Bild fehlt.
- Kopien von Tiefen-Stencil-Formaten werden abgelehnt.
- Zeilenlängen für Kopien im Kodierer prüft Vulkan nicht auf 256 Byte; WebGPU verlangt das --
  für Portabilität `bytes_per_row` als Vielfaches von 256 wählen.
- Ohne `hostQueryReset` (Vulkan 1.1) werden neue Zeitstempel erst in der nächsten Übergabe
  zurückgesetzt; vorher `read_*` aufzurufen ist dort nicht erlaubt.
- Ein `Frame`, der nie dargestellt wird, lässt die Swapchain beim nächsten `acquire()` neu bauen.
- Die Pfade für Vulkan 1.1 und 1.2 (Erweiterungen statt Kern) sind geschrieben, aber mangels
  solcher Hardware hier nicht getestet.

## Für den WGSL-Übersetzer (`lib/shader`)

- SPIR-V 1.3 für Stufe Basis (Vulkan 1.1); bis 1.5 auf Vulkan 1.2, bis 1.6 auf 1.3.
  `ShaderModule::from_spirv` lehnt höhere Versionen ab. In 1.3 nennt `OpEntryPoint` nur
  Input- und Output-Variablen; Speicherpuffer sind `StorageBuffer` mit `Block`.
- Gruppen und Bindungen 1:1 (`DescriptorSet` g, `Binding` b). Arten: Uniform-Puffer
  (`Uniform` + `Block`), Speicherpuffer (`StorageBuffer` + `Block`, nur lesend mit
  `NonWritable`), Textur (`OpTypeImage` Sampled 1, `UniformConstant`), Speichertextur
  (Sampled 2), Sampler (`OpTypeSampler`); keine kombinierten Bild-Sampler, der Shader bildet
  `OpSampledImage` selbst.
- Push-Konstanten: ein `Block` in `PushConstant` mit `Offset`-Dekorationen ab 0, höchstens
  128 Byte.
- Bindless-Tabelle: in der Gruppe, in die das Programm sie setzt; Bindung 0 Laufzeit-Array von
  Texturen, 1 Sampler, 2 Speicherpuffer (Laufzeit-Array von `Block`-Strukturen, ohne
  `ArrayStride` am äußeren Array), 3 Speichertexturen. `RuntimeDescriptorArray` (unter 1.5 mit
  `SPV_EXT_descriptor_indexing`); nicht einheitliche Indizes mit `NonUniform`.
- Kein Kippen von y: das Backend kippt den Viewport. Fragment-Shader mit `OriginUpperLeft`.
- Vertex-Pulling über `VertexIndex` / `InstanceIndex` (Vulkan-Bedeutung: einschließlich
  Basis-Vertex und erster Instanz); `BaseVertex`, `BaseInstance`, `DrawIndex` mit
  `Features::draw_parameters`.

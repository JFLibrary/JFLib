# GPU -- explizite GPU-Schicht für UI, 2D-CAD und 3D-Spiele

`lib/gpu` gibt Programmen direkten Zugriff auf die Grafikkarte: Rechen-Shader, Rendern mit
Tiefentest, indirekte Aufrufe, Zeitstempel und Fenster-Swapchains. Die API ist explizit wie
Vulkan und WebGPU (Instanz, Adapter, Gerät, Warteschlange, Ressourcen, Pipelines,
Befehlspuffer, Barrieren) und neutral gegenüber dem Backend: das Backend wird pro Ziel beim
Übersetzen gewählt, es gibt keine dynamische Verteilung.

Stand (G1a): Vulkan unter Windows. `vulkan-1.dll` wird zur Laufzeit geladen, das Programm
startet also auch auf Rechnern ohne Vulkan und bekommt dort einen sauberen Fehler. Geplant:
Vulkan unter Linux und Android, WebGPU im Browser, später Metal -- mit denselben Typen und
Methoden. Die Oberfläche rastert weiter auf der CPU; unter Windows zeigt die GPU das Ergebnis
(G3a, [Ausgabe der UI](#ausgabe-der-ui), `jfgui_GPU=0` schaltet das ab), die Ebenen über dem Grund
und große Bilder als eigene Texturen (G3b, [Ebenen und Bilder](#ebenen-und-bilder)).

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
| Fenster-Handles für die GPU | `plat::win_native` ([`lib/plat/win_api.rs`](../lib/plat/win_api.rs)), das Kindfenster der UI-Ausgabe: `plat::win_gpu_child` ([`lib/plat/win_gpuout.rs`](../lib/plat/win_gpuout.rs)) |
| Ausgabe der UI über die Swapchain | [`lib/ui/gpu_out.rs`](../lib/ui/gpu_out.rs), Shader: [`gpu_out.wgsl`](../lib/ui/gpu_out.wgsl), [`gpu_out_spv.rs`](../lib/ui/gpu_out_spv.rs) |
| Tests und Messungen | [`apps/gputest`](../apps/gputest/main.rs), die UI-Ausgabe: [`apps/uigputest`](../apps/uigputest/main.rs), [`apps/gallery/tests/gpu_output.uis`](../apps/gallery/tests/gpu_output.uis), [`gpu_layers.uis`](../apps/gallery/tests/gpu_layers.uis) |

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
    layout: &layout, shader: gpu::ShaderStage::new(&shader, "main") })?;
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
  diskrete zuerst; `AdapterInfo::missing` nennt, warum eine unbrauchbar ist), `adapter()`,
  `adapters_low_power(display)` (für die Ausgabe der UI; `low_power_rank`).
  `AdapterInfo`: Name, Hersteller, Gerätekennung, Art, Treibername und -version, API-Version.
  `adapter.features()`, `limits()`, `level()`, `can_present(&surface)`,
  `create_device(&DeviceDesc)`.
- **Gerät**: `queue()`, `features()`, `limits()`, `level()`, `wait(id, timeout_ns)`,
  `wait_idle()`, `poll()`, `completed()`, `is_done(id)`, `is_lost()`, `memory_reserved()`, `test_lose()`
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
- **Pipelines**: `create_compute_pipeline`, `create_compute_pipeline_sized(&desc, größe)` (eine
  verlangte Subgroup-Größe, WGSLs `@subgroup_size`: `Features::subgroup_size_control`, eine
  Zweierpotenz in `Limits::subgroup_min_size` … `subgroup_max_size`, die Workgroup-Größe in x ein
  Vielfaches), `create_render_pipeline` (Vertex + Fragment,
  dynamisches Rendern, Tiefentest, Mischen, Culling; Vertex-Eingaben sind optional,
  Vertex-Pulling aus Speicherpuffern geht ohne sie). Jede Stufe ist ein `ShaderStage { module,
  entry, constants }` (`ShaderStage::new(module, entry)` ohne Konstanten) mit ihren
  Pipeline-Konstanten (unten). `ShaderModule::entry_points()` und `constants()` sagen, was ein
  Modul enthält.
- **Befehle**: `device.encoder()`; Kopien (`copy_buffer`, `fill_buffer`,
  `copy_buffer_to_texture`, `copy_texture_to_buffer`), Barrieren (`barrier`,
  `texture_barrier`, `texture_barrier_range`), Rechnen (`set_compute_pipeline`,
  `set_bind_group`, `push_constants`, `dispatch`, `dispatch_indirect`), Render-Durchgänge
  (`begin_render` .. `end_render`, `set_render_pipeline`, `set_viewport`, `set_scissor`,
  `set_vertex_buffer`, `set_index_buffer`, `draw`, `draw_indexed`, `draw_indirect`,
  `draw_indexed_indirect`; auf Voll `draw_indexed_indirect_multi` und
  `draw_indexed_indirect_count`), Zeitstempel (`reset_timestamps`, `timestamp`); `finish()`.
- **Warteschlange**: `submit([cmd, ..])` gibt eine `SubmitId` (Zeitleisten-Semaphor);
  `write_buffer`, `write_texture`, `write_texture_rects` (Rechtecke eines Bildes mit eigener
  Zeilenlänge, ein Übergang für alle) und `write_texture_rects_at` (dasselbe aus einem größeren
  Bild: die Textur hält einen Teil davon) werden über den Staging-Ring vor der nächsten Übergabe
  ausgeführt; `read_buffer` (kopiert, übergibt und wartet: der langsame Weg für Werkzeuge und
  Tests); `flush()`.
- **Zeitstempel**: `create_timestamps(n)`, `read_ticks`, `read_ms` (Millisekunden seit dem
  ersten gelesenen Zeitstempel des Geräts, damit `f64` jeden Takt behält); `Err(NotReady)`,
  solange nicht alle geschrieben und fertig sind.
- **Fenster**: `Surface::new(&instance, plat_fenster)`, `Surface::native(&instance, griffe,
  plat_fenster)` (ein Kindfenster), `create_swapchain(&surface, &SwapchainDesc)`, `acquire()`
  (`None`, solange das Fenster minimiert ist oder keine Fläche hat), `try_acquire()` (wartet nie,
  baut nie neu: `Frame`, `NotReady`, `Stale`, `NoArea`), `rebuild()` (auch auf einem anderen
  Thread), `view(&frame)`, `texture(&frame)`, `present(frame, [cmd, ..])`, `resize()`,
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

## Pipeline-Konstanten (WGSL `override`)

Ein Override eines Shaders (WGSL `@id(n) override x: u32 = 64;`, SPIR-V eine
Spezialisierungskonstante mit `SpecId n`, docs/SHADER.md) bekommt seinen Wert beim Anlegen der
Pipeline, je Stufe:

```rust
let c = [gpu::Constant { id: 0, value: 128.0 }, gpu::Constant { id: 3, value: -1.25 }];
let pipe = dev.create_compute_pipeline(&gpu::ComputePipelineDesc {
    layout: &layout, shader: gpu::ShaderStage { module: &m, entry: "main", constants: &c } })?;
```

- **Schlüssel ist die Nummer** (`SpecId`). WebGPU nimmt `constants: Record<string, number>`
  mit der Nummer als Text (`"0"`) oder, bei Overrides ohne `@id`, ihrem Namen. Der
  WGSL-Übersetzer (`lib/wgsl`) vergibt deshalb auch Overrides ohne `@id` eine Nummer
  (0x10000 + Index, außerhalb von 0 … 65535, die WGSL für `@id` erlaubt), und
  `wgsl::Output::constants` macht aus WebGPU-artigen Schlüsseln (Name oder Zahl) die Liste
  `(Nummer, Wert)` für jede Spezialisierungskonstante des Moduls, abgeleitete Overrides fertig
  ausgerechnet (docs/WGSL.md „Overrides“); ein späteres WebGPU-Backend setzt den Schlüssel
  selbst.
- **Der Wert ist ein `f64`** wie in WebGPU und wird so umgewandelt wie dort: `bool` ist wahr außer
  bei 0 und NaN; `i32` / `u32` (und die anderen Ganzzahlen) werden zur Null hin abgeschnitten und
  müssen endlich und im Bereich liegen; `f32` / `f16` / `f64` werden gerundet und müssen endlich
  bleiben. Sonst `Invalid`.
- **Geprüft gegen das Modul** (`ShaderModule::from_spirv` liest die Spezialisierungskonstanten
  mit `shader::reflect`): eine Nummer, die das Modul nicht hat, oder eine Nummer zweimal ist
  `Invalid` (Vulkan würde sie übergehen, WebGPU nicht); ein Modul mit einer `SpecId` zweimal
  ebenso. Overrides, die die Pipeline nicht setzt, behalten den Standardwert im Modul. Ob ein
  WGSL-Override ohne Initialisierer gesetzt ist, weiß nur der Übersetzer (SPIR-V hat immer einen
  Standardwert): das prüft `wgsl::Output::constants`.
- **Workgroup-Größe**: nach dem Einsetzen der Konstanten (`WorkgroupSize`-Built-in vor
  `LocalSizeId` vor `LocalSize`) muss jede Achse ≥ 1 und innerhalb von
  `Limits::max_compute_workgroup_size`, das Produkt innerhalb von `max_compute_invocations` sein,
  sonst `Invalid` (WebGPU prüft das ebenfalls bei der Pipeline). Ein Modul mit `LocalSizeId`
  braucht `maintenance4` (Vulkan 1.3; sonst `Unsupported`).
- Auch der Entry Point wird geprüft: das Modul muss ihn für die Stufe haben, sonst `Invalid`.

Vulkan bekommt ein `VkSpecializationInfo` je Stufe (bool als `VkBool32`, sonst die Breite des
Typs). Getestet von `gputest --test spec` (unten).

## Feature-Stufen

| Stufe | Bedeutung | Vulkan-Voraussetzungen |
| --- | --- | --- |
| Basis | was WebGPU und Vulkan 1.1 können: Rechen-Shader, Speicherpuffer, einzelne indirekte Aufrufe, Textur-Arrays, Zeitstempel wo vorhanden | Vulkan 1.1, eine Warteschlange für Grafik und Rechnen, Zeitleisten-Semaphoren, dynamisches Rendern (Kern in 1.3 oder `VK_KHR_dynamic_rendering`) |
| Voll | zusätzlich Bindless-Tabellen, `draw_indexed_indirect_count`, mehrere indirekte Aufrufe, Draw-Parameter, Subgruppen-Operationen | Vulkan 1.3 mit Descriptor-Indexing (update-after-bind, partially bound, non-uniform), drawIndirectCount, multiDrawIndirect, drawIndirectFirstInstance, shaderDrawParameters, Subgruppen (basic, vote, ballot, arithmetic, shuffle) im Rechen-Shader |

`device.level()` und `device.features()` sagen, was da ist. Subgroups: `Features::subgroups` (die
Operationen, die die Stufe Voll braucht, in Compute-Shadern), `subgroups_webgpu` (was WebGPUs Feature
`subgroups` und WGSLs `enable subgroups` brauchen: dazu relative Shuffles und Quads, auch in
Fragment-Shadern; so bindet `wgsl::feature::for_device` die Erweiterung ans Gerät),
`subgroup_stages` und `subgroup_ops` wie gemeldet, `subgroup_size_control` (Vulkan 1.3),
`subgroup_extended_types` (f16; beide werden eingeschaltet, wo vorhanden), `Limits::subgroup_size`,
`subgroup_min_size`, `subgroup_max_size`. Mesh-Shader und Raytracing werden
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
Blöcke, die wachsen (Gerätespeicher 4 MiB, dann das Doppelte bis 64 MiB; Host-Speicher 1 MiB bis
16 MiB), Anforderungen über einem Achtel des größten Blocks bekommen einen eigenen Block, der mit
ihnen wieder geht; freie Bereiche werden zusammengelegt. Ein Programm mit einem kleinen Fenster
reserviert so wenige MiB statt 80. Uploads laufen über einen Staging-Ring: Stücke von 256 KiB,
von vorn nach hinten gefüllt, über Übergaben hinweg; ein volles Stück ist wieder frei, sobald die
GPU die letzte Übergabe erledigt hat, die daraus liest (zwei bleiben frei liegen). Ein Upload über
einem halben Stück bekommt einen eigenen Puffer, den der nächste passende wieder nimmt und der eine
Sekunde nach dem letzten zurückgeht (`Device::poll`). Ressourcen sind
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
HINSTANCE für `VK_KHR_win32_surface`. Die UI-Fenster von `lib/ui` stellen über beide Wege dar: ihre
Swapchain liegt auf einem Kindfenster ([Ausgabe der UI](#ausgabe-der-ui)).

Darstellungsmodi: `Fifo` (V-Sync, immer vorhanden, Standard), `Mailbox`, `Immediate` (für
Messungen); fehlt einer, wird `Fifo` genommen. Die Swapchain wird beim nächsten `acquire()` neu
gebaut, wenn sich die Fenstergröße geändert hat, die letzte Darstellung veraltet oder
suboptimal war, `resize()` oder `set_present_mode()` es verlangt hat oder ein Bild nie
dargestellt wurde; vorher wartet sie, bis die Warteschlange leer ist. Minimiert gibt
`acquire()` `None`. Mit `SwapchainDesc::copy_src` lassen sich die dargestellten Bilder
zurücklesen.

## Ausgabe der UI

Die UI rastert weiter auf der CPU und bleibt pixelgleich; der Framebuffer ist die Wahrheit. Unter
Windows zeigt die GPU ihn ([`lib/ui/gpu_out.rs`](../lib/ui/gpu_out.rs), G3a): die
Änderungsrechtecke, die die Laufzeit ohnehin berechnet, gehen in eine Textur je Fenster, ein
Dreieck zeichnet die Textur ins Swapchain-Bild, das Bild wird dargestellt. Alle Fenster teilen ein
Gerät. **Das ist der Standard** (unten: was es kostet). `jfgui_GPU=0`
schaltet die GPU überall ab. Skripte und Läufe ohne Fenster bleiben auf dem CPU-Pfad, außer ein
Skript verlangt mit `gpu on` anderes. Andere Ziele linken nichts davon
(`lib/ui/gpu_out_off.rs`, `TARGET_DEPS` in `build_rs.py`).

**Der Weg eines Fensters.**

| Stufe | Was geschieht |
| --- | --- |
| CPU | `plat::win_present` wie ohne GPU. Das erste Bild jedes Programms geht so |
| Job | Instanz, Gerät, Pipeline und Swapchain entstehen auf einem Hilfsthread, der erst nach dem ersten Bild startet und danach endet. Die CPU zeichnet weiter |
| Wechsel | der ganze Framebuffer in die Textur, ein `present` auf dem verborgenen Kindfenster, dann das Kindfenster zeigen und die Kopie für `WM_PAINT` freigeben |
| GPU | je Bild: Rechtecke hochladen, `try_acquire`, zeichnen, `present` |
| zurück | bei neuer Größe oder einem Fehler erst das Bild über den CPU-Pfad zeigen, dann das Kindfenster verbergen; keine Lücke, kein falsches Bild |

**Das Kindfenster.** Hat eine Swapchain auf einem Fenster dargestellt, bleibt GDI dort unsichtbar,
auch nachdem Swapchain und Fläche zerstört sind, nach `WM_PAINT`, neuer Größe und Aus- und
Einblenden: DWM zeigt weiter das letzte Bild (hier gemessen, `uigputest --test hwnd` druckt es).
Von der GPU zurück auf denselben HWND geht also nicht. Deshalb lebt die Swapchain auf einem
Kindfenster, das den Client-Bereich bedeckt (`plat::win_gpu_child`, `lib/plat/win_gpuout.rs`:
`WS_CHILD | WS_DISABLED`, `HTTRANSPARENT`, zeichnet nie selbst, nimmt keine Eingaben; der
Hit-Test mit dem Größenrand des Fensters läuft weiter im Elternfenster). Das Hauptfenster bleibt
sein Leben lang der CPU-Pfad: Kindfenster verborgen = sofort, was GDI zuletzt gezeichnet hat.
Ein Kindfenster, das gezeigt wird, bevor sein `present` den Bildschirm erreicht, zeigt das
Elternfenster durch: dieselben Pixel, kein Blitzen. `gpu expect screen` und die Prüfungen von
`uigputest` lesen dafür den zusammengesetzten Bildschirm.

**Größe ändern.** Jedes Bild einer Größenänderung ist ein CPU-Bild, auch in der modalen Schleife:
GDI zeichnet, bevor `SetWindowPos` zurückkehrt, also hinkt nichts dem Rahmen hinterher (eine
Swapchain käme ein bis zwei Bilder später: gestreckte Bilder oder Lücken). Sobald die Größe 100 ms
steht und die modale Schleife zu Ende ist, baut der Hilfsthread die Swapchain neu (etwa 10 ms, das Fenster bleibt so lange auf dem CPU-Pfad) und
das Fenster wechselt zurück. Dasselbe bei Maximieren, Einrasten, Vollbild und DPI-Wechsel. Beim
Verschieben, auch auf einen anderen Monitor, geschieht nichts: das Kindfenster fährt mit.

**Warten.** Ein `present` wartet nie: `Swapchain::try_acquire` gibt `NotReady`, wenn kein Bild frei
ist; das Bild bleibt in der Textur und kommt mit `gp_tick` (die Fensterschleife schaut nach 2 ms)
oder dem nächsten Bild. Bilder im Abstand unter 4 ms warten ebenso (zu sehen ist davon nichts, die
GPU muss es nicht zeichnen). Darstellungsmodus: MAILBOX (fehlt er: FIFO), zwei Bilder.

**Leerlauf.** Kein Thread von uns, kein Timer, kein Bild. Minimierte Fenster laden nichts hoch und
bekommen beim Wiederherstellen das ganze Bild. Ein blinkender Cursor kostet ein kleines Hochladen
(4 KiB je Blinken) und einmal den Fensterinhalt zeichnen.

**Hochladen.** Der Staging-Ring von `lib/gpu` (Kapitel „Speicher und Lebensdauer“): ein Bild kleiner
Rechtecke kostet, was sie enthalten, und keinen neuen Puffer; ein ganzes Bild bekommt einen
eigenen, der eine Sekunde nach dem letzten zurückgeht.

**Pixelgleichheit.** UNORM-Format, `COMPOSITE_ALPHA_OPAQUE`, keine sRGB-Umwandlung, der
Fragment-Shader holt das Texel mit `textureLoad` (kein Filter) und setzt Alpha auf 1 (das vierte Byte
des XRGB-Framebuffers bedeutet nichts). `jfgui_GPU_VERIFY=1` liest jedes dargestellte Bild zurück
(`SwapchainDesc::copy_src`) und vergleicht es Byte für Byte; `apps/gallery/tests/gpu_output.uis` tut
das auf Galerie-Seiten nach Teil-Updates, Scroll-Kopien, neuen Größen, DPI- und Themenwechsel, mit
einem zweiten Fenster und nach einem verlorenen Gerät, und vergleicht den zusammengesetzten
Bildschirm mit dem Framebuffer (gleich bis auf die vom System gerundeten Fensterecken). Maximiert
sind Bildschirm mit CPU- und GPU-Pfad gleich bis auf die Uhr der Taskleiste.

**Adapter.** `instance.adapters_low_power(display)`: nur integrierte und diskrete GPUs (eine
CPU- oder virtuelle ist kein Gewinn gegenüber dem CPU-Pfad), der Adapter, der den Monitor des Fensters
betreibt, zuerst (`plat::win_display_adapter`: `EnumDisplayDevices`, PCI-Kennung; er ist ohnehin wach und
braucht keine Kopie zwischen GPUs), integrierte vor diskreten. `jfgui_GPU_ADAPTER` gilt auch hier.
Programme, die eine GPU für sich wollen, nehmen weiter `instance.adapter()` (diskrete zuerst). Hier
gibt es nur eine GPU; die Rangfolge prüft `uigputest --test adapter` als Tabelle. Ob schon das Aufzählen
der Adapter eine schlafende diskrete GPU weckt (der Treiber wird geladen), ist auf einem Laptop noch zu messen.

**Fehler.** Kein Vulkan, kein brauchbarer Adapter oder keine Fläche, ein Start in einer
Remote-Desktop-Sitzung: CPU-Pfad für den ganzen Lauf, ohne Meldung. Gerät verloren oder ein anderer
Fehler: alle Fenster mit dem Framebuffer zurück auf den CPU-Pfad, ein neues Gerät nach 2 s, 4 s, ...
5 min (ist die Sitzung inzwischen eine Remote-Desktop-Sitzung, wird alle 10 s wieder geprüft).

**Fensterarten.**

| Fenster | Weg |
| --- | --- |
| Hauptfenster, auch mit eigener Titelleiste (`AF_CUSTOM_TITLEBAR`), maximiert, Vollbild | Swapchain |
| Zweitfenster (`window_begin`), Dialoge, Werkzeugfenster | Swapchain, gleiches Gerät |
| Popups und Tooltips außerhalb des Fensters (`plat::popup`: `WS_POPUP`, Fensterregion, `CS_DROPSHADOW`, oft nur für Augenblicke) | bleiben CPU: eine Swapchain dauert länger als viele von ihnen leben; sie liegen über dem Elternfenster und seinem Kind |
| Popups und Menüs im Fenster | Teil des Framebuffers |
| Überlagerung des Farbwählers, der Bildschirmaufnahme | bleiben CPU |

Schatten und runde Ecken macht DWM am Hauptfenster, sie gelten auch für das Kind (auf dem Bildschirm
geprüft).

**Shader.** WGSL in [`lib/ui/gpu_out.wgsl`](../lib/ui/gpu_out.wgsl), vorübersetzt in
[`lib/ui/gpu_out_spv.rs`](../lib/ui/gpu_out_spv.rs) (204 + 373 + 513 + 659 Wörter): UI-Programme linken `lib/wgsl`
nicht. `uigputest --test shaders` vergleicht die Wörter mit `wgsl::compile`, `uigputest --gen-shaders
lib/ui/gpu_out_spv.rs` schreibt sie neu.

**Tests.** `apps/gallery/tests/gpu_output.uis` (das Skriptkommando `gpu`, GUIDE §12) und
`apps/uigputest` (nur Windows; echtes Fenster, echte Fensterschleife, jeder Test ein Skript):
`switch`, `resize`, `minimize`, `dpi`, `windows`, `off`, `noloader`, `lose`, `idle`, `csd`, `monitors`, `shaders`,
`adapter`, `hwnd`, `layers`, `images`, `imgplain`, `canvas`, `canvasplain`, `canvasoff`; dazu `apps/gallery/tests/gpu_layers.uis` und
`gpu_layers_rtl.uis`, und `gpu_canvas.uis` von `svgtest`, `diagramtest` (auch `gpu_canvas_rtl.uis`), Kritzel und PaintCraft. Alle unter der Validierungsschicht ohne Meldung, jedes dargestellte Bild
zurückgelesen; ohne GPU `SKIP`; liegt ein anderes Fenster über dem Testfenster, ist die Prüfung des
Bildschirms ein Hinweis statt eines Fehlers. Maximieren und Vollbild aktivieren das Fenster, deshalb tut das
kein Test mehr; eine frühere Fassung von `csd` hat beides einmal geprüft. `gputest --test upload` prüft Ring, `write_texture_rects`, `write_texture_rects_at` und
`try_acquire`.

### Ebenen und Bilder

Der Framebuffer bleibt die Wahrheit: er hält das ganze Bild, Byte für Byte wie auf dem CPU-Pfad. Nur
was die GPU davon bekommt, ist getrennt ([`lib/ui/gpu_out.rs`](../lib/ui/gpu_out.rs), G3b).

**Warum die GPU nicht mischt.** Der naheliegende Weg, eine Ebene als vormultiplizierte Textur mit
durchsichtigem Grund über den Rest zu blenden, ist nicht pixelgleich. Versuch auf der Galerie (Wegwerf-Code,
1000 × 1500, Menü und Untermenü): die Ebene einzeln gezeichnet, mit dem **genauen** ganzzahligen „über“
aufs darunter gelegt, wie es die beste GPU könnte, gegen direkt gezeichnet. Pixel mit Alpha 0 und 255 stimmen immer (1 431 680 und
61 820, 0 anders). Von den durchscheinenden (Schatten, Ecken; 6 500 und 8 282) weichen 808 und 1 090 ab,
um 1 bis 2 Stufen: ein Schatten sind zwölf Mischungen nacheinander, gerundet wird jedes Mal. Dazu
Subpixel-Text: der geht nur auf einem deckenden Ziel (`Text`: FMT_XRGB), auf einer durchsichtigen
Ebene wird er grau. Also mischt die GPU nichts.

**Ebenen.** Die Textur des Fensters hält nur noch die Grundebene. Jede Ebene darüber (Dialog, Popup,
Tooltip) mit Befehlen hat eine eigene Textur; sie hält in den 32 × 32-Zellen, die die Ebene berührt,
die Pixel **bis einschließlich dieser Ebene**, auf der CPU über das gezeichnet, was wirklich darunter
liegt: dieselben Kerne auf denselben Pixeln, also nichts, was abweichen könnte, ob Schatten,
durchscheinende Widgets, Subpixel-Text, Klipp oder RTL (die Befehle sind schon gespiegelt). Der
Render-Durchgang zeichnet den Grund, dann diese Zellen deckend in Ebenenfolge (Scissor je Rechteck, kein Blending).
Neben jeder Textur bleiben die Pixel unter der Ebene zellenweise im Speicher (`Store`).
Der Hash der Schmutz-Zellen wird dafür nach jeder Ebene festgehalten (`dirty::W::through`, nur wenn es Ebenen gibt), so ist je Zelle bekannt,
welche Ebene sich als unterste geändert hat und welche Ebenen sie berühren. Für jede
Ebene von unten nach oben (`gp_split_draw`): die Zellen, in denen sie die unterste geänderte ist, bekommen ihre
Pixel von darunter zurück in den Framebuffer; die Zellen, die sie berührt, werden gemerkt, sie wird dort
gezeichnet und in ihre Textur geladen.

| Vorgang | CPU | hochgeladen |
| --- | --- | --- |
| Popup öffnen | nur das Popup (Pixel darunter gemerkt) | seine Zellen |
| Popup schließen | die gemerkten Pixel zurück in den Framebuffer | nichts |
| Popup verschieben | nur das Popup an der neuen Stelle, die alte Stelle aus dem Speicher | seine neuen Zellen |
| Hover in einem Menü | nur die Zellen dieser Ebene | nur diese Zellen |
| Unter einer offenen Ebene ändert sich etwas | wie immer: Grund und alles darüber in diesen Zellen | diese Zellen in jede betroffene Textur |

Texturen und Speicher gibt es nur, solange die Ebene Inhalt hat, und eine Sekunde länger (ein Menü zu, das
nächste auf: kein neuer Speicher); frei werden sie ohne Bild, die Fensterschleife schaut dafür
einmal nach. Eine Textur ist so groß wie das, was die Ebene umschließt, auf 256 px gerundet, mit Platz zum Verschieben.
Solange Ebenen getrennt sind, gibt es keine Scroll-Kopien: sie verschieben Pixel des Framebuffers.

**Wann alles zurück in eine Textur geht** (`gp_flatten`, je Fenster): neue Größe, minimiert, der Inspektor,
eine Reparatur von `--verify-dirty`, jeder Fehler. Dann steht der ganze Framebuffer in der Grundtextur, das Bild stimmt.
Hat eine Ebene dabei noch Inhalt, zeichnet die nächste Änderung einmal alles und trennt wieder. Gerät verloren:
Texturen und Speicher weg, CPU-Pfad, nach dem neuen Gerät kommen Ebenen und Bilder mit der nächsten Änderung zurück.

**Bilder.** `Cmd::Image` der Grundebene ab 256 × 256 sichtbaren Pixeln werden zu Texturen, wenn sie deckend sind (FMT_XRGB, oder Alpha überall 255: einmal geprüft).
Schlüssel: Identität der Fläche (`Rc`; ein `Weak` hält die Adresse fest, keine andere Fläche kommt dorthin) und `ver`.
Hochgeladen wird erst, wenn die Neuzeichnungen des Bildes so viel gekostet haben wie das Hochladen (Mieten oder Kaufen):
ein Bild, das still steht, belegt nie GPU-Speicher, eines mit neuer `ver` in jedem Bild ebenso nie. Es geht in Teilen von 4 MiB hoch, je Bild, das es zeichnet; bis dahin zeichnet die CPU weiter (in einem Stück kostete ein Bild von 4000 × 3000 einmal 61 ms). Budget 256 MiB (`jfgui_GPU_IMAGE_MB`), verdrängt wird, was am längsten nicht gezeichnet wurde;
30 s nicht gezeichnet: weg mit dem nächsten Bild. Größer als das Budget, durchscheinend, in einer höheren Ebene oder unter der Mindestgröße: die CPU zeichnet es wie bisher.

Die GPU zeichnet ein Bild nur in den Zellen, die es ganz füllt und in denen es der letzte Befehl der Grundebene ist; dort zeichnet und lädt die
CPU nichts. Der Shader rechnet die Abbildung von `Dc::blit_scaled` in ganzen Zahlen nach
(`(d * step + step / 2) >> 16`, `textureLoad`, kein Filter), 1:1 ist derselbe Fall mit Schritt 65536.
Randzellen und Zellen mit etwas darüber laufen wie immer. Dem Framebuffer fehlen in diesen Zellen dann die
Pixel des Bildes (`owed`); `gp_settle` zeichnet sie nach, bevor ihn jemand liest: Rückweg zum CPU-Pfad, jede
Ebene darüber, `jfgui_GPU_VERIFY`, `--verify-dirty`, `shot`, `expect pixel`, `expect same`, der Agent; ein Programm, das selbst `Ui::fb()` liest,
ruft vorher `Ui::gpu_settle()`. Mit Bildern auf der GPU gibt es ebenfalls keine Scroll-Kopien.

Was nicht `Cmd::Image` ist, geht nicht diesen Weg: die Bildansicht `image_view` und die Leinwände zeichnen über `Cmd::Custom` ([Leinwände](#leinwände)).

### Messungen

GeForce RTX 2070 SUPER, Treiber 610.88, Ryzen 5 3600, zwei 1080p-Monitore mit 165 Hz, Windows 11,
Release-Builds, Konfigurationen abwechselnd, Mediane der Läufe. Parallel bauten andere Sitzungen. „Vorher“ ist der Stand vor G3a, „aus“ dieser Stand mit `jfgui_GPU=0`, „an“ mit
`jfgui_GPU=1`. Die Werkzeuge waren Python-Skripte (`ctypes`, kein Teil des Repos), die echte Fenster starten und den
Bildschirm mit `BitBlt` lesen; `uigputest --bench latency|idle|caret` ist der Teil im Repo.

| Messung | vorher / CPU-Pfad | GPU-Pfad | wie |
| --- | ---: | ---: | --- |
| Prozessstart → das Bild steht auf dem Bildschirm (gallery 1200 × 800, mit DWMs Einblenden) | vorher 170 ms (153–304), aus 165 ms (144–400) | 170 ms (149–434) | 15 Läufe je Variante, Bildschirm gelesen; kein Unterschied |
| Erstes Bild → Swapchain zeigt | – | 510–580 ms; Instanz 360–430, Adapter 3–5, Gerät 78–90, Swapchain + Textur 38–42, Pipeline 2 (Hilfsthread); der Wechsel auf dem UI-Thread 5–7 ms | 3 Läufe, `jfgui_GPU_TRACE=1` |
| Blitzen beim Wechsel | – | 0 falsche von 127–154 Bildschirmstreifen | `uigputest --test switch` |
| `WM_CLOSE` → Fenster weg / Prozess weg | vorher 1,9 / 12 ms, aus 1,9 / 14 ms | 0,4 / 114 ms (der Treiber beim Prozessende; das Fenster geht vorher) | 7 Läufe |
| Ereignis → `present` zurück, kleiner Fleck (200 × 40), Median (p95) | 0,29 ms (1,06) | MAILBOX 0,74 ms (1,50), FIFO 0,44 ms (0,98) | 3 Läufe zu je 120 Tastendrücken, 15–30 ms Abstand |
| Ereignis → `present` zurück, ganzes Bild (1280 × 800) | 1,88 ms (2,99) | MAILBOX 1,67 ms (2,56), FIFO 1,37 ms (1,87) | ebenso |
| Ereignis → Bildschirm, kleiner Fleck | 6,5 ms (12,2) | MAILBOX 9,7 ms (12,7), FIFO 7,0 ms (11,9) | ebenso; ein Thread liest den Bildschirm |
| Ereignis → Bildschirm, ganzes Bild | 9,8 ms (16,1) | MAILBOX 7,7 ms (12,9), FIFO 10,2 ms (17,5) | ebenso |
| Dauerfeuer: Alter des gezeigten Bildes, Median (p95) | 4,5 ms (6,1) | MAILBOX 3,2 ms (5,6), IMMEDIATE 3,2 ms (5,3), **FIFO 33,5 ms (34,3)** | 3 Läufe zu je 6000 Tastendrücken ohne Pause; die Farbe des Flecks ist seine Nummer |
| Dauerfeuer: Ereignis → `present` zurück | 0,14 ms | 0,08 ms (die meisten Bilder warten auf das nächste) | ebenso |
| Größenänderung, modale Schleife | keine schwarzen Flächen | keine schwarzen Flächen; die Bilder der Schleife sind CPU-Bilder. Der einmalige Wechsel zurück 2,5–3,7 ms | je 1 Lauf mit `SC_SIZE` und Pfeiltasten, 39 und 48 Bildschirmaufnahmen, angesehen |
| Maximiert, fertiges Bild | | gleich dem CPU-Pfad bis auf 131 Pixel der Taskleistenuhr | Bildschirmaufnahmen verglichen |
| Leerlauf, CPU-Zeit | 0 ms in 30 s, 5 Threads | 47 ms in 30 s (1,6 ms/s), 19 Threads, alles in `nvoglv64.dll`; ein zweiter Lauf 5 ms/s. Ein Treiberthread wacht rund 150-mal je Sekunde auf | je 1 Lauf; `GetThreadTimes` zählt in Schritten von 15,6 ms |
| Leerlauf, GPU-Zeit und Bilder | – | 0 ms, kein Bild | Leistungszähler `GPU Engine`, `Ui::gpu_output` |
| Blinkender Cursor (1280 × 800) | – | 4 KiB je Blinken; GPU 159 ms in 15 s bei 28 Bildern (Zähler von Windows, grob); ein Bild für jedes Blinken, nichts dazwischen | je 1 Lauf |
| Speicher, gallery 1200 × 800: Working Set / privat | vorher 20,5 / 10,5 MiB, aus 20,6 / 10,5 MiB | 67,7 / 68,1 MiB | 3 Läufe; der Anteil des Treibers: +47 / +58 MiB |
| GPU-Speicher | 0 | 27,0 MiB dediziert + 14,9 MiB geteilt; davon 3 × 3,7 MiB die Bilder (zwei der Swapchain, die Textur), `memory_reserved()` 5–6 MiB | 3 Läufe, Zähler `GPU Process Memory` |
| Größe der Programme, Release | gallery 3 877 376, Kritzel 4 038 144, PaintCraft 8 348 160 Byte | +186 880, +186 880, +186 368 Byte (`jfgui_GPU` ändert daran nichts) | |
| Andere Ziele | gallery Linux 4 398 528 Byte, Browser (threads) 3 464 579 Byte | +240 und +465 Byte (das Skriptkommando), keine Vulkan-Zeichenkette im Programm | |

Die Läufe zu Start, Ende, Latenz und Dauerfeuer liefen vor der Regel „höchstens drei Runden“ und mit
Prozesspriorität hoch; sie sind nicht wiederholt worden. Beim Ereignis → Bildschirm streuen die
Mediane zwischen zwei Messreihen um 1–3 ms bei 6 ms Bildabstand, und andere Sitzungen belasteten den
Rechner. Das Ergebnis lautet deshalb: **kein erkennbarer Unterschied** zwischen CPU-Pfad und MAILBOX, weder
beim Fleck (dort CPU vorn) noch beim ganzen Bild (dort GPU vorn). Das Lesen des Bildschirms wartet selbst
auf DWM und misst nicht bis zum Licht. Unter Dauerfeuer entscheidet die Messung: FIFO läuft fünf
Bildwechsel hinterher, deshalb MAILBOX. Ein 60-Hz-Monitor war nicht zur Hand.

**Was der Standard kostet.** Seit dem 01.11.2026 ist der Weg standardmäßig an. Startzeit, Latenz und
Größenänderung sind so gut wie auf dem CPU-Pfad,
der Leerlauf nicht: die Treiberthreads kosten 1,6–5 ms CPU je Sekunde und wecken den Rechner, der CPU-Pfad nichts. Von uns läuft dabei nichts, abstellen ließe es sich nur, indem das Gerät im Leerlauf wegginge (ein neues dauert eine halbe Sekunde). Dazu kommen 47 MiB Working Set, 58 MiB privater und 42 MiB GPU-Speicher je Programm sowie 100 ms beim Prozessende. Sichtbar bringt der Weg erst mit Ebenen und Bildern als GPU-Texturen (G3b)
und Leinwänden (G3c) etwas. Die Größe der Programme wächst in jedem Fall.

**G3b, vorher und nachher.** `uigputest --bench layers`: ein Fenster 1920 × 1000, eine volle Seite (rund
2 200 Felder aus abgerundetem Rechteck und Text), 200 Bilder je Szene im selben Prozess, Release; „vorher“ ist
`jfgui_GPU_LAYERS=0` (der Weg von G3a), „nachher“ der Standard, je 3 Läufe im Wechsel, parallel bauten andere Sitzungen.
Median der drei Mediane; Render ist `render_live`, Present `gp_present` (nach dem Hochladen in Render bzw. mit ihm).
Das Hochladen steht in Byte je Bild.

| Szene | Render vorher | nachher | Present vorher | nachher | hochgeladen vorher | nachher | Grund-Pixel vorher | nachher |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Popup 320 × 420 öffnen | 2 306 µs | 1 775 µs | 668 µs | 563 µs | 737 KB | 737 KB | 184 320 | 0 |
| Popup schließen | 1 141 µs | 292 µs | 681 µs | 534 µs | 737 KB | 0 | 184 320 | 0 |
| Popup verschieben | 2 226 µs | 1 638 µs | 639 µs | 568 µs | 737 KB | 737 KB | 184 320 | 0 |
| Tooltip 200 × 72 öffnen (vorher 2 Läufe) | 602–607 µs | 531 µs | 638–677 µs | 561 µs | 164 KB | 164 KB | 40 960 | 0 |
| Tooltip schließen (vorher 2 Läufe) | 358–378 µs | 178 µs | 594–652 µs | 592 µs | 164 KB | 0 | 40 960 | 0 |
| Dialog 560 × 380 ziehen | 2 980 µs | 2 148 µs | 700 µs | 527 µs | 1 090 KB | 1 090 KB | 272 384 | 0 |
| Dialog mit Dunst ziehen | 3 059 µs | 2 257 µs | 711 µs | 529 µs | 1 090 KB | 1 090 KB | 272 384 | 0 |
| Bild 4000 × 3000 auf 1200 × 900, daneben ändert sich wenig | 193 µs | 200 µs | 612 µs | 616 µs | 33 KB | 33 KB | 8 192 | 8 192 |
| dasselbe, die ganze Seite ändert sich | 13 482 µs | 5 785 µs | 1 336 µs | 526 µs | 7 680 KB | 3 437 KB | 1 920 000 | 859 136 |
| dasselbe, das Bild wird verschoben | 9 238 µs | 1 566 µs | 1 120 µs | 525 µs | 4 633 KB | 389 KB | 1 158 144 | 97 280 |
| Popup über dem Bild, auf und zu | 2 072 µs | 1 233 µs | 639 µs | 538 µs | 737 KB | 737 / 0 KB | 184 320 | 0 |

Beim Öffnen und Verschieben bleibt die Menge gleich und der größte Teil der Zeit: Schatten (zwölf Runden über die ganze
Fläche) und Inhalt der Ebene selbst. Was wegfällt, ist alles darunter; je teurer der Grund (eigene Zeichnungen, Leinwände),
desto mehr bringt das. Am meisten gewinnen Schließen und Bilder. Ein stillstehendes Bild kostet und bringt nichts.

Ändert sich die ganze Seite unter einem offenen Dialog, zeichnet dieser Weg auf einem Thread statt in Bändern. Mit derselben
Arbeit auf der CPU (`jfgui_GPU_IMAGES=0`) sind das 13–15 ms statt 12 ms, dafür 0,5 statt 1,2 ms beim Darstellen, bei 14 % mehr
Upload (zwei Läufe gegen einen, der Rechner unter Last).

Echte Galerie (1900 × 1000, ein Skript, ganze Zahlen aus den Zählern, je 1 Lauf; hochgeladen, in Klammern davon in die Grundtextur, und Grund-Pixel):

| Vorgang | vorher | nachher |
| --- | ---: | ---: |
| Menü „Datei“ öffnen | 360 KB (360 KB), 90 112 | 360 KB (16 KB), 4 096 |
| Hover im Menü | 221 KB (221 KB), 55 296 | 221 KB (0), 0 |
| Menü schließen | 352 KB (352 KB), 88 064 | 16 KB (16 KB), 4 096 |
| Inhaltsdialog mit Dunst öffnen | 7 723 KB (7 723 KB), 1 930 720 | 7 776 KB (176 KB), 44 032 |
| Inhaltsdialog schließen | 7 600 KB (7 600 KB), 1 900 000 | 49 KB (49 KB), 12 288 |

Das Bild geht in Teilen hoch: der teuerste Rahmen während des Mietens und Ladens 23 ms (`--n 40`, 1 Lauf; das Bild ganz auf der CPU 15 ms je Rahmen),
vorher in einem Stück einmal 61 ms. Eine vierte Runde nachher lag unter fremder Last (schlechtester Rahmen 117 ms in Szenen ohne Bild) und zählt in der Tabelle nicht.

Größe, Release: gallery Windows 4 076 544 → 4 126 720 Byte (+50 176), Linux 4 412 248 → 4 413 848 (+1 600), Browser
3 518 288 → 3 521 061 Byte (simd, threads je −1 728), dort keine Vulkan-Zeichenkette.

Speicher (`memory_reserved`, Blöcke der GPU und Staging): vorher 10–13 MiB mit Popups, nachher 16–20 MiB; davon eine Popup-Textur 1,5 MiB
plus 0,7 MiB CPU-Speicher für die Pixel darunter, ein Dialog 2,3 + 1,1 MiB, Dunst über dem ganzen Fenster 7,5 + 7,7 MiB. Das Bild: 45,8 MiB Textur,
`memory_reserved` 67–78 MiB (vorher 13–28 MiB), kurz dazu 48 MiB Staging, der eine Sekunde später zurückgeht. Ohne Ebenen und ohne geänderte Bilder: kein Unterschied zu vorher.

### Leinwände

Zoomen und Verschieben einer Leinwand kostete je Bild ein ganzes Rastern. Solange sich nur die Ansicht bewegt, kommt ihr Inhalt jetzt aus
GPU-Texturen, nur die Transformation ändert sich ([`lib/ui/canvas.rs`](../lib/ui/canvas.rs), [`lib/ui/gpu_out.rs`](../lib/ui/gpu_out.rs), G3c). Die CPU bleibt die Wahrheit:
**in Ruhe ist das Bild Byte für Byte das des CPU-Pfads.**

**Die API.** Eine Leinwand zeichnet wie immer über `ui.canvas(...)` oder `cmd_custom` mit ihrem `Draw`. Direkt danach sagt sie, woraus das besteht:

```rust
let inp = ui.canvas(ID, Some(draw.clone()), 0, ver).clone();
ui.canvas_gpu(CanvasDesc {
    id: ID,
    name: "bild",                        // für die Umgebungsvariablen unten
    rect: [x, y, w, h],                  // die Fläche im Fenster, die der Inhalt deckend füllt
    view: CanvasView { scale, ox, oy },  // Fensterpixel = o + Inhalt * scale
    ver: inhalt,                         // steht für alles, was der Inhalt zeigt; neu: seine Kacheln gehen
    content: draw,                       // Rc<dyn CanvasContent>: draw_view(g, view, rect)
    gpu: CanvasGpu::Pan,                 // Cpu | Pan | Zoom
    exact_pan: true,                     // das Rastern hängt nicht vom Ort ab: jfgui_GPU_VERIFY besteht darauf
});
ui.canvas_gpu_damage(ID, [x0, y0, x1, y1]);  // Inhalt geändert: die Kacheln darüber gehen
```

`ui.canvas_gpu_wanted()` sagt, ob sich das Bauen lohnt. **Auf jedem Ziel derselbe Code:** ohne GPU-Ausgabe, unter Linux, Android und im Browser, in Skripten ohne `gpu on`, mit `jfgui_GPU=0` tut `canvas_gpu` nichts, das `Draw` malt jedes Bild wie bisher.
`image_view` (`ImageViewState::gpu`) und `diagram` (`DiagramState::gpu`, also auch `diagramtest` und beide Leinwände von JFNetworker) tun das selbst.

**Wie es läuft.** Die Kacheln sind 256 × 256 Texel plus ein Texel Rand, zu einer Stufe (Maßstab und Phase) gehörig, gezeichnet von `draw_view` auf dem UI-Thread, höchstens 5 ms je Bild, was übrig bleibt geht in diesem Bild den Weg der CPU.

| Vorgang | CPU | GPU |
| --- | --- | --- |
| Ansicht steht still, Inhalt unverändert | nichts | nichts: keine Textur vor der ersten Bewegung, kein Bild, kein Thread |
| Verschieben um ganze Pixel | die Kacheln, die neu ins Bild kommen (einmal); Zellen mit etwas darüber: ihr `Draw`, das Darüber | die Kacheln kopiert (`fs`) |
| Zoom, `CanvasGpu::Zoom` | dasselbe, eine neue Stufe, wenn der Faktor 0,55 .. 1,9 verlässt (die alte bleibt, zurück kostet nichts) | die Kacheln bilinear (`fs_tile`) |
| Zoom, `CanvasGpu::Pan` | das Bild, sofort, wie ohne GPU | wie immer |
| 180 ms nach der letzten Bewegung | ein Bild wie ohne GPU, hochgeladen | wie immer: jetzt stimmt alles, und es kommt kein Bild mehr |
| Strich, verschobener Knoten, neue `ver` | wie ohne GPU, die Bewegung endet sofort; `canvas_gpu_damage` gibt nur die betroffenen Kacheln frei | wie immer |

Der Befehl einer Leinwand, die aus Kacheln kommt, geht mit seinem Ort statt seines Inhalts in den Hash der Zellen (`dirty.rs`), sonst wäre jede Zelle jedes Bild schmutzig. Sie wird genau wie ein Bild aus G3b vermerkt: in den Zellen, in denen sie als Letzte der Grundebene liegt, zeichnet die GPU, die CPU zeichnet und lädt nichts (`Split::owed`, `Split::stale`); auch in Randzellen, die sie nur teilweise füllt, dort mit dem Scissor an ihrem Rand. Zellen mit etwas darüber (Griffe, Bildlaufleisten, Auswahl) und Zellen ohne ihre Kachel laufen über ihr `Draw` und werden hochgeladen; unter einer offenen Ebene ebenso, damit die Ebene das Richtige über sich hält. `gp_settle` ruft das `Draw`, bevor jemand den Framebuffer liest: `Ui::gpu_settle()` außerhalb eines Durchgangs, `shot`, `expect pixel`, der Agent, `--verify-dirty` und die Wege zurück zum CPU-Pfad geben also immer die richtigen Pixel, auch mitten in einer Geste. PaintCrafts Drucken und Zwischenablage und Kritzels Export lesen den Framebuffer nicht, sie zeichnen in eigene Flächen.

**Budget und Lebensdauer.** 128 MiB (`jfgui_GPU_CANVAS_MB`), verdrängt wird die am längsten nicht gezeichnete Kachel, nie eine des laufenden Bildes; passt nichts mehr hinein, läuft der Rest über die CPU. 30 s nach der letzten Bewegung gehen alle Kacheln, ohne ein Bild. Neue Größe, Inspektor, minimiert: das Fenster zeichnet über die CPU, die Kacheln bleiben. Gerät verloren: alle Kacheln weg, die nächste Geste füllt sie auf dem neuen Gerät neu. Ein gespiegeltes Fenster (RTL) zeichnet Leinwände auf der CPU.

**Was während einer Geste anders aussieht.** Gemessen mit `gpu shot` (das zurückgelesene dargestellte Bild) gegen `shot` (die Pixel der CPU), 1920 × 1000; `jfgui_GPU_VERIFY` zählt dasselbe je Bild in `GpuOutput::canvas_diff_*`.

| Leinwand | Verschieben um ganze Pixel | Zoom um den Faktor 1,25, bis zu 180 ms nach dem letzten Rad | Standard |
| --- | --- | --- | --- |
| Kritzel | **gleich**, 0 Pixel (`exact_pan`: `blit_scaled` rechnet von der Bildecke aus, ein Unterschied wäre ein Fehler) | nur mit dem Schalter: 1,41 Mio. Pixel, im Mittel 34 Stufen (rauschiges Testbild) | `Pan`: der Zoom-Schritt läuft über die CPU, kein Unterschied zu keiner Zeit |
| PaintCraft | **gleich**, 0 Pixel (`exact_pan`: die Tabellen von `render_image` rechnen von der Bildecke aus) | nur mit dem Schalter: 1,58 Mio. Pixel, im Mittel 20 Stufen | `Pan`, ebenso. Mit Pixelraster, Auswahl oder einem Overlay des Werkzeugs bleibt die Leinwand auf der CPU: das `Draw` malt sie mit dem Bild |
| `image_view`, Bitmap | gleich, 0 Pixel (200 %, nicht bewiesen: `affine` rechnet in Fließkomma) | Foto: 734 000 Pixel, im Mittel 5, höchstens 20 Stufen, kaum zu sehen. 32 × 24 Pixel groß: Blockkanten weich, das Raster bleibt, im Mittel 32 Stufen | `Zoom` |
| `image_view`, SVG | fast: 97 von 1,6 Mio. Pixeln, um 1 Stufe | 973 000 Pixel, im Mittel 11, höchstens 86: etwas weich | `Zoom` |
| `diagram` | fast: 0 bis 1 Pixel je Bild, um 1 Stufe | 269 000 Pixel, im Mittel 7, höchstens 224 (Text- und Linienkanten): weicher, die Geometrie stimmt; beim Stehenbleiben wird das Bild scharf | `Zoom` |

**Ganzzahliges Verschieben.** `scrollcopy.rs` warnt zu Recht: der Rasterer rechnet in absoluten f32-Koordinaten. Das trifft nur Vektorinhalt: eine Kachel hat andere absolute Koordinaten als das Fenster, und ein Diagramm rechnet Welt nach Fenster mit etwas anderen Rundungen. Gemessen sind das 0 bis 1 Pixel je Bild um eine Stufe (Diagramm) und 97 Pixel um eine Stufe (SVG, viele Kurven). Rasterinhalt ist gleich, weil seine Abbildung nur vom Abstand zur Bildecke abhängt. Deshalb bleibt es auch beim Verschieben dabei: 180 ms danach zeichnet die CPU.

**Der Schalter je Leinwand.** Im Code `CanvasDesc::gpu`, `ImageViewState::gpu`, `DiagramState::gpu`, `CANVAS_GPU` in `apps/Kritzel/view.rs` und `apps/PaintCraft/src/workspace/view.rs`. Ohne Bauen: `jfgui_GPU_CANVAS_CPU=image,diagram,kritzel,paintcraft` hält die genannten auf der CPU, `jfgui_GPU_CANVAS_PAN=...` lässt sie nur verschieben, `jfgui_GPU_CANVAS_ZOOM=...` lässt sie auch skalieren, `jfgui_GPU_CANVAS=0` schaltet alles ab, `jfgui_GPU_CANVAS_SETTLE_MS=n` ändert die 180 ms.

**Warum Kritzel und PaintCraft nicht skalieren.** Ein Zoom-Schritt kostet dort auf der CPU 6 ms, gleich groß wie das Bild im Fenster, nicht wie das Dokument. Das merkt niemand. Skaliert wären es 1 ms, dafür zeigt jedes Rad für 180 ms ein weicheres Bild, das dann wieder scharf springt, und am Ende kostet es trotzdem die 6 ms. Nächster Nachbar exakt aus den Pixeln des Dokuments ginge auch (das tut `fs_image` für Bilder), kostet aber den Speicher des ganzen Dokuments und bringt dieselben 5 ms. Nicht gebaut.

**Messungen.** Nur `lib/ui` im selben Programm, `jfgui_GPU_CANVAS=0` gegen Standard, Release, 1920 × 1000, GeForce RTX 2070 SUPER, Ryzen 5 3600, je 3 Läufe im Wechsel in einem Fenster je Lauf (`gpu on fast`, `bench`), parallel bauten andere Sitzungen. Median der Läufe, je Bild; Render ist `render_live`, Present `gp_present`, hochgeladen in KB. Die Skripte waren Wegwerf-Code: Szene laden, `bench 120 drag A B`, `gpu print`.

| Leinwand, Inhalt | Vorgang | Render vorher | nachher | Present vorher | nachher | hochgeladen vorher | nachher |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `image_view`, Foto 1600 × 1200 bei 200 % | verschieben | 30 044 µs | 1 983 µs | 1 375 µs | 435 µs | 7 617 | 732 |
| | Rad, ±1 Schritt | 32 472 µs | 1 713 µs | 1 358 µs | 388 µs | 7 680 | 680 |
| | Rad, 8 hinauf, 8 hinab | 21 838 µs | 6 069 µs | 1 306 µs | 494 µs | 7 680 | 2 490 |
| | das Bild danach, einmal | | 28 410–30 784 µs | | 483–656 µs | | 6 636 |
| `image_view`, SVG bei 612 % | verschieben | 9 128 µs | 603 µs | 1 314 µs | 375 µs | 7 617 | 569 |
| | Rad, ±1 Schritt | 9 862 µs | 1 169 µs | 1 327 µs | 407 µs | 7 680 | 769 |
| | das Bild danach, einmal | | 9 655–9 981 µs | | 446–500 µs | | 6 636 |
| `diagram`, 3 630 Knoten (`diagramtest --grid 60`) | verschieben | 15 671 µs | 1 029 µs | 1 421 µs | 427 µs | 7 555 | 373 |
| | Rad, ±1 Schritt | 13 330 µs | 572 µs | 1 372 µs | 438 µs | 7 680 | 354 |
| | Rad, 8 hinauf, 8 hinab | 7 373 µs | 3 459 µs | 1 348 µs | 641 µs | 7 680 | 3 079 |
| | das Bild danach, einmal | | 3 135–17 043 µs | | 495–508 µs | | 7 373 |
| Kritzel, 4000 × 3000 | verschieben | 6 367 µs | 553 µs | 1 563 µs | 350 µs | 7 680 | 478 |
| | Zoom-Schritt (CPU) | 6 216 µs | 5 667 µs | 1 553 µs | 1 515 µs | 7 680 | 7 680 |
| | Zoom-Schritt, Schalter `ZOOM` | | 824 µs | | 439 µs | | 660 |
| | **Strich** | 82 µs | 83 µs | 366 µs | 342 µs | 27 | 27 |
| | das Bild danach, einmal | | 5 712 µs | | 399 µs | | 6 390 |
| PaintCraft, 4000 × 3000 bei 100 % | verschieben | 5 972 µs | 1 285 µs | 1 436 µs | 502 µs | 7 617 | 595 |
| | Zoom-Schritt (CPU) | 6 098 µs | 5 911 µs | 1 385 µs | 1 427 µs | 7 680 | 7 680 |
| | Zoom-Schritt, Schalter `ZOOM` | | 1 088 µs | | 426 µs | | 524 |
| | **Strich** | 274 µs | 295 µs | 369 µs | 340 µs | 29 | 29 |
| | das Bild danach, einmal | | 6 234 µs | | 439 µs | | 6 636 |

Beim Strich kein Unterschied (Streuung der Läufe 262–334 und 273–314 µs); ein Strich berührt keine Kachel außer durch `canvas_gpu_damage`, die in Mikrosekunden frei wird, und wartet auf nichts. Ein gezogener Knoten im `diagram` läuft wie vorher über die CPU: sein Inhalt ändert sich, also keine Kacheln (in `diagramtest` 0,4 ms je Bild vorher und nachher, das Ziehen zeichnet dort nur Überlagerungen).

Wo der Gewinn kleiner ist: **unter 1 ms** bleibt nur, was keine Zellen über der Leinwand hat. Das Foto hat Bildlaufleisten und eine Statuszeile, deren Zellen je Bild `IvDraw` kosten (170 000 Pixel, 1,4 ms von 2 ms), PaintCraft Lineale und Leisten. Beim Rad in einem Zug zählen die neuen Stufen: das Foto 113 Kacheln in 16 Bildern (5 ms je Bild), das Diagramm 163. Das erste Bild einer Geste füllt seine Kacheln: das längste Bild war beim Foto 14,7 ms (vorher 39), Kritzel 11,9 (15), PaintCraft 12–15 (14,5), Diagramm 6,3 (26), hier ohne die 5-ms-Grenze, die für Skripte nicht gilt. Das Bild danach kostet genau ein Bild wie vorher, wer also alle 200 ms ruckelt, zahlt fast so viel wie zuvor. Bei einem kleinen Fenster oder einer kleinen Zeichnung bringt das alles entsprechend weniger.

Speicher (`memory_reserved`, Blöcke der GPU und Staging): vorher 16–23 MiB je nach Zustand, mit Leinwänden je nach Zahl der Kacheln: 50 MiB (Kritzel, PaintCraft: 32 Kacheln, 8 MiB), 82 MiB (Foto 75 Kacheln, 19 MiB; Diagramm 82, 21 MiB), 146 MiB nach einem Zoom über viele Stufen (188–245 Kacheln, 48–62 MiB). Der Rest ist Staging des Hochladens (ein ganzes Bild im Ring) und Blockreste. Nach 30 s ohne Bewegung gehen die Kacheln. Eine Leinwand, die nie bewegt wird, kostet nichts.

Größe, Release, gegen den Stand davor: gallery Windows 4 153 856 → 4 201 984 Byte (+48 128), PaintCraft 8 624 128 → 8 677 376 (+53 248), gallery Linux 4 441 480 → 4 442 808 (+1 328), Browser 3 542 127 → 3 584 472 (+42 345), simd 3 481 482 → 3 531 406 (+49 924), threads 3 490 369 → 3 496 433 (+6 064). Unter Linux kommt fast nichts dazu, im Browser ist die Spanne so groß, dass das nach Inlining-Schwankung aussieht; nachgegangen bin ich dem nicht. Keine Vulkan-Zeichenkette in beiden.

### Was fehlt

- Kacheln entstehen auf dem UI-Thread, in dem Bild, das sie braucht, ohne Vorrat um die Ansicht: das erste Bild einer Geste bleibt teuer. Ein Hilfsthread ginge nur für Inhalte, deren `draw_view` keine `Rc`-Welt braucht.
- Beim Verschieben über eine Geste hinweg wäre die Kachel oft schon das richtige Bild; dass das Bild danach trotzdem gezeichnet wird, kostet ein Bild, ist aber der Beweis in Ruhe.
- Eine verschobene Ebene wird neu gezeichnet und ganz hochgeladen. Ihr deckendes Inneres könnte in der Textur bleiben, nur der
  durchscheinende Rand müsste neu. Das setzt voraus, dass Pixel beim Verschieben gleich bleiben: der Rasterer rechnet in
  absoluten f32-Koordinaten, abgerundete Ecken, Kreise und Linien ändern sich (siehe `scrollcopy.rs`). Nicht gebaut.
- Durchscheinende Bilder, Bilder in höheren Ebenen und unter 256 × 256 bleiben auf der CPU. Die Bildränder (eine Zelle breit) laufen weiter über den Grund.
- Andere `Cmd::Custom` (Diagramme von `chart`, Timeline, Terminal) bleiben auf der CPU; `ui.canvas_gpu` gibt es auch für sie, wenn es sich lohnt.
- Kein teilweises Zeichnen in die Swapchain (`VK_KHR_incremental_present`): jedes Bild zeichnet das
  ganze Fenster, auch für ein Blinken. Billig, aber nicht nichts auf einer schwachen GPU.
- Scroll-Kopien laufen im Framebuffer; die Textur bekommt die verschobenen Zellen neu (die GPU könnte sie
  selbst verschieben).
- Der Framebuffer wird einmal in den Staging-Puffer kopiert (der Weg ohne Kopie:
  `VK_EXT_external_memory_host`, verlangt Warten auf die GPU).
- Nicht getestet: ein Laptop mit zwei GPUs, AMD und Intel, Monitore an verschiedenen GPUs (die Swapchain
  bleibt auf ihrem Adapter, DWM kopiert), ein echter Treiber-Reset (`test_lose` spielt ihn nur vor),
  Bildschirmsperre und Remote Desktop im laufenden Betrieb, HDR, Bildschirmleser mit Treffertest auf dem Kindfenster.
- Was das Fenster-DC liest (`plat::win_test_image`, Werkzeuge wie `tools/wincap.py`), sieht im GPU-Betrieb
  das letzte CPU-Bild; Aufnahmen über den Bildschirm oder Windows Graphics Capture sehen das richtige.
- `VK_KHR_present_wait` fehlt in `lib/gpu`; „bis auf dem Bildschirm“ ist deshalb mit dem Lesen des
  Bildschirms gemessen, das selbst auf DWM wartet.

## Umgebungsvariablen

| Variable | Wirkung |
| --- | --- |
| `jfgui_GPU=0` (oder `off`) | GPU aus: `Instance::new` gibt `Err(Disabled)`, die UI bleibt auf dem CPU-Pfad |
| nicht gesetzt, `jfgui_GPU=1` (oder `on`) | die UI-Fenster zeigen ihre Bilder über die Swapchain ([Ausgabe der UI](#ausgabe-der-ui)) |
| `jfgui_GPU_VERIFY=1` | die UI-Ausgabe liest jedes dargestellte Bild zurück und vergleicht es mit dem Framebuffer (langsam, zum Testen) |
| `jfgui_GPU_TRACE=1` | die UI-Ausgabe schreibt ihre Schritte und Zeiten auf stderr |
| `jfgui_GPU_LAYERS=0` | die UI-Ausgabe hält eine Textur je Fenster: keine Ebenen, keine Bilder ([Ebenen und Bilder](#ebenen-und-bilder)) |
| `jfgui_GPU_IMAGES=0` | keine Bilder als Texturen, Ebenen bleiben |
| `jfgui_GPU_IMAGE_MB=n` | Budget der Bildtexturen (Vorgabe 256) |
| `jfgui_GPU_CANVAS=0` | keine Leinwand nutzt Kacheln ([Leinwände](#leinwände)) |
| `jfgui_GPU_CANVAS_CPU=a,b` | die so benannten Leinwände (`image`, `diagram`, `kritzel`, `paintcraft`) zeichnen auf der CPU |
| `jfgui_GPU_CANVAS_PAN=a,b` | sie verschieben nur aus Kacheln, ein Zoom läuft über die CPU |
| `jfgui_GPU_CANVAS_ZOOM=a,b` | sie zoomen auch aus Kacheln, was immer das Widget gewählt hat |
| `jfgui_GPU_CANVAS_MB=n` | Budget der Kacheln (Vorgabe 128) |
| `jfgui_GPU_CANVAS_SETTLE_MS=n` | wann das Bild der CPU der letzten Bewegung folgt (Vorgabe 180) |
| `jfgui_GPU_RETRY_MS=n` | Wartezeit bis zum ersten neuen Gerät nach einem Fehler (Vorgabe 2000, dann doppelt so lang, bis 5 min) |
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
| `upload.uis` | der Staging-Ring: 2000 kleine Uploads bleiben im Ring, ein ganzes Bild nimmt einen eigenen Puffer, der eine Sekunde später weg ist; Blöcke wachsen; `write_texture_rects` und `write_texture_rects_at` (Zeilenlänge, Klippen, Fehler); `write_buffer` über Stückgrenzen; `try_acquire` wartet nicht auf die Anzeige, `Stale` nach neuer Größe, `rebuild`, alles auf einem Kindfenster |
| `graphics.uis` | zwei Dreiecke (Vertex-Pulling, Indexpuffer, Rückseiten-Culling, Tiefentest) in eine Textur; Pixel geprüft |
| `indirect.uis` | Argumente von einem Rechen-Shader: `dispatch_indirect`, `draw_indexed_indirect`, auf Voll `_multi` und `_count` |
| `timestamps.uis` | monoton über zwei Übergaben, plausibel gegen die CPU-Uhr |
| `bindless.uis` | (Voll) Puffer und Texturen über eine Tabelle; ein Platz ersetzt |
| `window.uis` | Fenster mit Swapchain: FIFO-Bilder, dargestelltes Bild zurückgelesen, Größe geändert, minimiert / wiederhergestellt, IMMEDIATE und MAILBOX |
| `errors.uis` | `jfgui_GPU=0`, fehlender Loader, `jfgui_GPU_ADAPTER` ohne Treffer, ungültige Aufrufe (Fehler, kein Absturz), verlorenes Gerät und ein neues danach |
| `scene.uis` | die Messszene (unten) mit den handgeschriebenen SPIR-V-Shadern und mit den von `lib/wgsl` übersetzten WGSL-Shadern: `move` = `move_all` Bit für Bit, `cull` = `cull_ref`, ein Bild bekannter Würfel (Lage, Farbe, Tiefe, Rückseiten), indirekter und Einzel-Pfad gleich, die WGSL-Bilder = die SPIR-V-Bilder Pixel für Pixel; WGSL-`move` ohne Robustheit bitgleich, mit erlaubter Kontraktion gemeldet (der Treiber fusioniert: 1,33 Mio. Wörter weichen ab) |
| `wgsl.uis` | das WGSL-Back-End auf der GPU, auf SPIR-V 1.6 und auf einem Basis-Gerät mit 1.3 ([`t_wgsl.rs`](../apps/gputest/t_wgsl.rs), [`t_wgsl2.rs`](../apps/gputest/t_wgsl2.rs), [`t_wgsl3.rs`](../apps/gputest/t_wgsl3.rs)): die Subgroup- und Quad-Funktionen auf Laufzeitwerten gegen eine CPU-Referenz, die Subgroup-Größe und aktive Lanes kennt (Workgroups von 1 bis 256 Aufrufen, ein Zweig auf dem Aufruf), f16, `@subgroup_size`, Quad-Swaps im Fragment-Shader; jeder Operator und jede numerische eingebaute Funktion auf Laufzeitwerten (150 Fälle, 13 469 Ergebnisse, neun davon mit f16) gegen den Konstantenauswerter von `lib/wgsl` mit den Genauigkeiten der Spezifikation und WGSLs Laufzeitregeln (Division durch 0, `MIN / -1`, Shifts ab der Bitbreite); NaN bei `min` / `max` / `clamp` / Vergleichen (gemeldet: 7 von 7 wie WGSL); Overrides als Spezialisierungskonstanten und eingesetzt, mit Standardwerten, Fehlerfällen; Robustheit (Schreiben hinter ein Laufzeit-Array und ein festes Array, Lesen weit außerhalb); Workgroup-Speicher genullt, Atomics dort, `workgroupUniformLoad`; `discard` (Farbe, Atomic danach, `dpdx` danach); `textureStore` außerhalb übersprungen, `textureLoad` außerhalb geklemmt. `--dump DATEI` schreibt die Fälle für [`tools/wgsl_run.mjs`](../tools/wgsl_run.mjs) (Chrome rechnet dieselben Shader), `--chrome ERGEBNIS` vergleicht dessen Ergebnisse mit denselben Referenzen |
| `bench_smoke.uis` | kurze Läufe der Bildschleife der Messszene mit `--verify`: indirekt und einzeln ohne Fenster, Fenster mit FIFO und IMMEDIATE; indirekt und einzeln mit den WGSL-Shadern |
| `spec.uis` | Pipeline-Konstanten: `shader::samples::spec_fill` als SPIR-V 1.3 (`WorkgroupSize`-Built-in) und 1.6 (`LocalSizeId`) mit den Standardwerten und mit Konstanten für alle fünf Overrides, Workgroup-Größen 1, 32, 256 und das Gerätelimit (1024; sie bemisst auch ein Workgroup-Array) = `spec_fill_ref` Wort für Wort; `constants()`; die Umwandlungen wie WebGPU (u32 1,9 = 1, bool 0,5 = wahr, i32 −2,7 = −2); `Invalid` für fremde und doppelte Nummern, Werte außerhalb des Typs, NaN, unendlich, Workgroup-Größe 0 und über dem Limit, einen fehlenden Entry Point |
| `textures.uis` | Texturen in Shadern aus `lib/shader` gegen CPU-Referenzen (unten) |
| `validate.uis` | alles einmal mit Validierungsschicht (siehe oben), auch `scene`, `bench_smoke`, `spec`, `textures` und `wgsl` (ohne Meldung) |

Ohne Vulkan oder brauchbare GPU melden die Tests `SKIP` und gelten als bestanden.
`gputest --bench` misst die Zahlen unten, `gputest --bench N` die Messszene.

**Texturen in Shadern** (`gputest --test textures`, [`t_texture.rs`](../apps/gputest/t_texture.rs);
die Shader sind `shader::samples`, vor dem Hochladen von `shader::val` geprüft). Exakt heißt:
gleich der CPU-Referenz Bit für Bit (unorm-Texel als `k / 255` in f32).

| Teil | was | Ergebnis auf der RTX 2070 SUPER |
| --- | --- | --- |
| fetch | `OpImageFetch` jedes Texels jeder Mip-Stufe und Schicht im Rechen-Shader: rgba8unorm (5 Mips), r32float (4 Mips), r32float-2D-Array (3 Schichten, 2 Mips), r32uint, r32sint, rgba32uint, depth32float (die Tiefe in x) | exakt; fehlende Kanäle 0, 0, 1 |
| sample | `OpImageSampleExplicitLod` (nearest) im Rechen-Shader: 2D-Array (Schicht = RNE(z) begrenzt auf 0 … 3, auch die Halbfälle 0,5 / 1,5 / 2,5), Würfel 4 × 4 (jedes Texel jeder Seite über seine Richtung, Länge 1, 2,5 und 0,125; die CPU-Seitenwahl nach Vulkans Tabelle), Würfel-Array (2 Würfel) | exakt |
| storage | `OpImageWrite`, dann `OpImageRead` und `OpImageQuerySize` von rgba8unorm, rgba16float, r32float, rgba32float, r32uint, rgba32uint, r32sint (16 × 12); die Bytes der Textur zurückgelesen; lesen-ändern-schreiben in einem Shader für r32float / r32uint / r32sint | exakt (rgba8unorm: die Bytes und `k / 255`) |
| fragment | Textur + Sampler im Fragment-Shader (Rgba32Float-Ziel): nearest mit expliziter LOD 0 … 4 und mit impliziter LOD (Mip-Auswahl aus den Ableitungen, Ziel in Mip-Größe); bilinear bei LOD 0 und trilinear bei LOD 1,5 an Viertel-Texel-Positionen | nearest exakt; linear max. Fehler 5,8·10⁻⁶ / 7,6·10⁻⁶ bei einer Toleranz von 1/256 (8 Bit Unterteilung der Filtergewichte; an Viertel-Texeln sind die Gewichte exakt) |
| compare | Tiefentextur + Vergleichssampler (`less`, `greater-equal`), `OpImageSampleDrefExplicitLod` und `…ImplicitLod`, fünf Referenzwerte einschließlich eines Texelwerts; PCF (linearer Vergleich) | exakt; PCF Fehler 0 (Toleranz 1/256) |
| queries | `OpImageQuerySizeLod` bei Mip 0 und 1 und `OpImageQueryLevels` von 2D, 2D-Array, Würfel, Würfel-Array (Anzahl Würfel), Tiefe und einer Ansicht der Mips 1 … 3; `OpImageQuerySize` einer Speichertextur | exakt |
| bindless | (Voll) Texturen und Sampler einer Bindless-Tabelle, je Aufruf ein anderer Index (`NonUniform`), nearest und linear abgetastet | max. Fehler 3·10⁻⁸ |

Alles läuft auch mit `--validate` ohne Meldung der Validierungsschicht.

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

## Messszene

`gputest --bench N` lässt die Benchmark-Szene aus `lib/shader` ([SHADER.md](SHADER.md),
„Benchmark-Vertrag“) auf `lib/gpu` laufen und misst sie
([`apps/gputest/scene.rs`](../apps/gputest/scene.rs), [`measure.rs`](../apps/gputest/measure.rs);
die Tests in [`t_scene.rs`](../apps/gputest/t_scene.rs)):

```text
gputest --bench N [--res 1080p|4k|720p|BxH] [--mode indirect|single] [--window | --fullscreen]
                  [--present immediate|fifo|mailbox] [--frames F] [--warmup W] [--inflight K]
                  [--verify] [--shot DATEI.png] [--validate] [--shaders spirv|wgsl] [--robust 0]
                  [--fused]
```

`--shaders wgsl` nimmt statt der handgeschriebenen SPIR-V-Shader aus `lib/shader/bench.rs` die
WGSL-Shader aus `lib/wgsl/bench`, beim Anlegen der Szene von `lib/wgsl` für die Stufe des Geräts
übersetzt (robust, `move` und `cull` mit `exact`); `--robust 0` ohne Robustheit, `--fused` ohne
`NoContraction` (dann ist `move` nicht mehr bitgleich).

Voreinstellung: 1080p, `indirect`, ohne Fenster, 300 gemessene Bilder nach 60 zum Aufwärmen,
3 Bilder in Flug. Ohne Fenster zeichnet die Szene in eine Rgba8Unorm- und eine
Depth32Float-Textur der Größe, ohne Darstellung. `--window` öffnet ein plat-Fenster dieser
Client-Größe (höchstens der Arbeitsbereich des Monitors) mit Swapchain, IMMEDIATE zum Messen,
`--present fifo` zum Anschauen (Esc oder Schließen beendet); `--fullscreen` legt das Fenster
rahmenlos über den ganzen Monitor (`WCMD_FULLSCREEN`). Die Animationszeit läuft 1/60 s je Bild
weiter, nicht mit der Uhr: jeder Lauf zeichnet dieselben Bilder. `--verify` vergleicht danach
die Objekte und die sichtbare Liste des letzten Bildes Bit für Bit mit `move_all` /
`cull_ref`, `--shot` speichert das letzte Bild als PNG. Die letzte Zeile (`RESULT n=… res=…`)
ist für Skripte.

**Szene.** N Würfel bewegt `move` auf Bahnen in einer flachen Scheibe (Radius `extent_for(N)`
= 3 · ∛N, Höhe ± extent/8). Die Kamera fährt in der Scheibe auf einem Kreis mit halbem Radius
knapp über der Mitte (0,08 · extent) einmal in 50 s herum und blickt 30° neben die Mitte;
vertikales Sichtfeld 60°, near 0,1, far 3 · extent. So sind bei 16:9 rund 53 % der Würfel im
Sichtkegel (Bounding-Spheres gegen sechs Ebenen), bei jedem N und über die ganze Fahrt.
Die Objekte entstehen einmal auf der GPU (`move` zur Zeit 0) und werden nie hochgeladen; je
Bild gehen nur 176 Byte Kamera über den Bus.

**Ablauf je Bild** (`indirect`): Zeitstempel 0; eine Barriere gegen alle Zugriffe des Bildes
davor; Kamera aus dem Staging-Puffer des Bild-Slots in den Uniform-Puffer kopieren; `move`
(⌈N/256⌉ Gruppen); Zeitstempel 1; `instance_count` = 0 (`fill_buffer`), Barriere, `cull`;
Zeitstempel 2; Barriere zu Indirect-, Vertex- und Kopier-Lesen; den `DrawCmd` in den
Rückleseplatz des Slots kopieren (sichtbare Anzahl); Render-Durchgang mit einem
`draw_indexed_indirect` (`vs_instanced` + `fs_lambert`, Rückseiten-Culling, Tiefentest);
Zeitstempel 3. `single`: kein `cull`; im Durchgang je Objekt `push_constants(16, [i])` und
`draw_indexed(36, 1, 0, 0, 0)` mit `vs_single`. Zwei Bindungs-Layouts mit den Nummern des
Vertrags: eines für die Rechen-Pipelines (beschreibbar), eines für die Render-Pipelines (nur
lesend), damit kein Vertex-Shader eine beschreibbare Bindung braucht.

**Bilder in Flug.** Die Szene hat K Slots (Kamera-Staging, Rücklesen des `DrawCmd`, vier
Zeitstempel). Bevor Bild f in Slot f mod K aufgezeichnet wird, wartet die Schleife auf die
Übergabe von Bild f − K (ohne Fenster `device.wait`, im Fenster `swapchain.acquire` mit
`frames_in_flight` K), liest dessen Zeitstempel und sichtbare Anzahl und zeichnet dann auf. Die
CPU läuft also bis zu K Bilder voraus und wartet nur, wenn die GPU zurückliegt. Objekte,
sichtbare Liste und `DrawCmd` gibt es nur einmal: die GPU arbeitet die Bilder nacheinander ab
(die Barriere am Bildanfang), nur die Aufzeichnung überlappt.

**Messgrößen.** CPU: Kamera schreiben + aufzeichnen + `finish` („Aufz.“) und dazu `submit`
bzw. `present` („CPU“), ohne Warten. GPU aus den Zeitstempeln: `move` = 1 − 0, `cull` = 2 − 1,
`draw` = 3 − 2, gesamt = 3 − 0; GPU-Bildabstand = Abstand der Zeitstempel 3 zweier Bilder,
Belegung = Summe der Gesamtzeiten durch Summe dieser Abstände. Bildzeit = Abstand zweier
`submit` / `present` auf der CPU-Uhr. Je Lauf Median und 99. Perzentil über die gemessenen
Bilder.

**Messbedingungen.** GeForce RTX 2070 SUPER (448 GB/s), Treiber 610.88, Ryzen 5 3600, Windows
11, Monitor 1920 × 1080 mit 165 Hz; Release-Build (`python build_rs.py app gputest --release`),
ohne Validierung, die impliziten Schichten des Systems an (OBS-Hook, NV-Optimus/Present). Parallel
liefen in anderen Worktrees die volle Testsuite und Builds (CPU-Last 97–100 %); jede Messung lief
deshalb mit Prozesspriorität „hoch“. Mit normaler Priorität verlor der Prozess Zeitscheiben von
30–100 ms, die GPU stand bis zu 87 % der Zeit still (Bildzeit-p99 bis 115 ms); solche Läufe sind
verworfen. Je Konfiguration drei Läufe, abwechselnd (alle Konfigurationen nacheinander, die zweite
Runde rückwärts); die Tabellen zeigen den Median der drei Läufe, in Klammern die Spanne der drei
Läufe (bei „Spanne“ die der Mediane). Alle Zeiten in ms, 300 Bilder nach 60 zum Aufwärmen, 3
Bilder in Flug; Bilder/s aus der mittleren Bildzeit.

`indirect` (sichtbar: 53,4 % bei 100k, 52,9 % bei 1M; Belegung = Anteil der Zeit, in der die GPU
an der Szene arbeitet):

| N | Ziel | CPU Med. (p99) | GPU move / cull / draw | GPU gesamt Med. (p99), Spanne | Bildzeit Med. (p99) | Bilder/s (Spanne) | GPU belegt | Engpass |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 100k | 1080p | 0,064 (0,16) | 0,033 / 0,016 / 0,235 | 0,284 (0,81), 0,283–0,305 | 0,300 (0,90) | 3055 (2825–3081) | 95 % | Rasterung + Geometrie |
| 250k | 1080p | 0,068 (0,19) | 0,071 / 0,041 / 0,406 | 0,518 (1,07), 0,518–0,530 | 0,535 (1,17) | 1695 (1544–1704) | 97 % | Geometrie |
| 500k | 1080p | 0,089 (0,17) | 0,133 / 0,072 / 0,682 | 0,888 (1,59), 0,873–0,892 | 0,912 (1,66) | 1003 (993–1019) | 98 % | Geometrie |
| 1M | 1080p | 0,139 (0,19) | 0,259 / 0,133 / 1,256 | 1,650 (2,38), 1,640–1,675 | 1,710 (2,50) | 545 (531–557) | 99 % | Geometrie |
| 100k | 4K | 0,078 (0,23) | 0,033 / 0,016 / 0,643 | 0,693 (1,26), 0,689–0,698 | 0,721 (1,29) | 1289 (1166–1320) | 97 % | Rasterung / Fragmente |
| 250k | 4K | 0,080 (0,18) | 0,071 / 0,041 / 0,928 | 1,040 (1,63), 1,038–1,054 | 1,064 (1,73) | 878 (874–882) | 98 % | Rasterung / Fragmente |
| 500k | 4K | 0,124 (0,22) | 0,134 / 0,072 / 1,327 | 1,538 (2,26), 1,531–1,563 | 1,574 (2,31) | 594 (591–597) | 99 % | Geometrie + Rasterung |
| 1M | 4K | 0,141 (0,22) | 0,259 / 0,133 / 2,012 | 2,568 (3,77), 2,397–2,603 | 2,543 (3,68) | 380 (369–386) | 99 % | Geometrie + Rasterung |
| 100k | Vollbild 1080p | 0,294 (0,97) | 0,033 / 0,016 / 0,281 | 0,331 (0,74), 0,320–0,331 | 0,352 (1,17) | 2192 (2111–2508) | 77 % | `present`, Bildwechsel |
| 250k | Vollbild 1080p | 0,288 (1,20) | 0,071 / 0,042 / 0,452 | 0,564 (1,02), 0,559–0,565 | 0,588 (1,77) | 1482 (1405–1558) | 92 % | Geometrie |
| 500k | Vollbild 1080p | 0,321 (1,20) | 0,139 / 0,072 / 0,727 | 0,939 (1,37), 0,932–0,940 | 1,002 (1,85) | 921 (902–946) | 95 % | Geometrie |
| 1M | Vollbild 1080p | 0,386 (0,80) | 0,264 / 0,134 / 1,290 | 1,700 (2,43), 1,690–1,704 | 1,866 (2,61) | 526 (523–526) | 99 % | Geometrie |

`single` (kein Culling, alle N Würfel, je ein Draw):

| N | Ziel | CPU Med. (p99), Spanne | GPU move / draw | GPU gesamt Med. (p99) | Bildzeit Med. (p99) | Bilder/s (Spanne) | GPU belegt | Engpass |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 100k | 1080p | 8,48 (10,9), 8,40–8,67 | 0,042 / 0,761 | 0,803 (1,64) | 8,49 (10,9) | 117 (115–118) | 11 % | CPU-Aufzeichnung |
| 250k | 1080p | 22,1 (26,2), 20,5–22,6 | 0,113 / 2,421 | 2,534 (3,90) | 22,1 (26,2) | 45 (43–49) | 13 % | CPU-Aufzeichnung |
| 500k | 1080p | 44,0 (52,0), 43,6–44,4 | 0,211 / 5,621 | 5,832 (8,28) | 44,0 (52,0) | 23 (22–23) | 14 % | CPU-Aufzeichnung |
| 1M | 1080p | 89,9 (104), 80,3–90,1 | 0,407 / 10,71 | 11,25 (12,6) | 89,9 (104) | 11 (11–12) | 12 % | CPU-Aufzeichnung |
| 100k | 4K | 9,14 (10,5), 7,42–9,17 | 0,042 / 0,960 | 1,002 (2,13) | 9,15 (10,5) | 109 (107–130) | 13 % | CPU-Aufzeichnung |
| 250k | 4K | 21,8 (26,3), 18,1–22,7 | 0,113 / 2,631 | 2,745 (4,04) | 21,8 (26,4) | 46 (44–54) | 14 % | CPU-Aufzeichnung |
| 500k | 4K | 40,8 (48,3), 39,7–45,3 | 0,210 / 5,848 | 6,059 (6,88) | 40,8 (48,3) | 25 (22–25) | 15 % | CPU-Aufzeichnung |
| 1M | 4K | 90,1 (102), 81,7–90,5 | 0,407 / 11,16 | 11,60 (13,0) | 90,2 (102) | 11 (11–12) | 13 % | CPU-Aufzeichnung |
| 100k | Vollbild 1080p | 8,99 (10,1), 7,26–9,27 | 0,042 / 0,786 | 0,828 (1,66) | 9,00 (10,1) | 109 (107–134) | 10 % | CPU-Aufzeichnung |
| 250k | Vollbild 1080p | 21,7 (24,1), 18,6–22,0 | 0,113 / 2,441 | 2,554 (3,18) | 21,7 (24,1) | 46 (45–53) | 12 % | CPU-Aufzeichnung |
| 500k | Vollbild 1080p | 41,0 (45,7), 35,5–43,5 | 0,210 / 4,812 | 5,023 (5,95) | 41,0 (45,7) | 25 (23–28) | 13 % | CPU-Aufzeichnung |
| 1M | Vollbild 1080p | 87,0 (125), 86,9–90,8 | 0,406 / 10,10 | 10,50 (11,0) | 87,0 (125) | 11 (11–12) | 12 % | CPU-Aufzeichnung |

Die Spannen der GPU-Zeiten liegen meist unter 3 %. Verzerrt hat die Last vor allem die
CPU-gebundenen `single`-Läufe: die dritte Runde lief bei geringerer Last bis zu 20 % schneller
(etwa 1M bei 1080p 80,3 statt 89,7–90,1 ms); bei 4K 1M streut auch die GPU-Gesamtzeit (2,40–2,60).

**Dreiecke je Sekunde, größter Fall** (1M, `indirect`, 529 000 Würfel × 12 = 6,35 M Dreiecke je
Bild, die Hälfte davon Rückseiten): 1080p 545 Bilder/s = 3,5 G Dreiecke/s, im Zeichendurchgang
selbst 6,35 M / 1,256 ms = 5,1 G/s (12,7 M Vertex-Aufrufe = 10 G Vertices/s); 4K 380 Bilder/s =
2,4 G/s, im Durchgang 3,2 G/s. Zum Vergleich `single` 1M: 12 M Dreiecke × 11 Bilder/s =
0,13 G/s (die GPU allein schaffte 1,1 G/s).

**Weitere Reihen** (je drei Läufe, Median):

| Reihe | Werte |
| --- | --- |
| Bildgröße, 1M `indirect`: draw | 320 × 180: 1,185 · 1280 × 720: 1,193 · 1080p: 1,257 · 2560 × 1440: 1,401 · 4K: 1,987 |
| Bildgröße, 100k `indirect`: draw | 320 × 180: 0,127 · 1280 × 720: 0,137 · 1080p: 0,234 · 2560 × 1440: 0,342 · 4K: 0,641 |
| Bilder in Flug K, 100k 1080p | K = 1: Bildzeit 0,427, GPU belegt 62 % · K = 2: 0,301, 94 % · K = 3: 0,302, 95 % |
| Bilder in Flug K, 1M 1080p | K = 1: 1,870, 89 % · K = 2: 1,703, 99 % · K = 3: 1,729, 99 % |
| Fenster 1904 × 984 (größte Client-Fläche des Monitors), IMMEDIATE, 1M | Bildzeit 2,07 (Mittel 2,11 = 473 Bilder/s), draw 1,62: der Desktop-Compositor setzt jedes Bild zusammen und teilt sich die GPU |
| Vollbild FIFO, 1M | Mittel 6,22 ms = 161 Bilder/s (165 Hz), GPU 1,88 ms je Bild, belegt 32 % |
| `--verify`, 1M nach 70 Bildern | 12 M Gleitkommazahlen = `move_all` Bit für Bit, keine Denormalen gekappt; sichtbare Liste = `cull_ref` (528 797) |

Ein Fenster in 4K ist auf diesem 1080p-Monitor nicht möglich; 4K wurde ohne Fenster gemessen.

**Wo der Engpass liegt.**

- `indirect`, CPU: kein Engpass. Aufzeichnen 0,03–0,08 ms, `vkQueueSubmit` 0,03–0,06 ms, im
  Vollbild `present` weitere ~0,25 ms (`vkQueuePresentKHR`), unabhängig von N bis auf Rauschen.
  Die CPU wartet 80–95 % der Bildzeit auf die GPU. Nur bei sehr kurzen Bildern (100k im Vollbild,
  0,35 ms) begrenzen `present` und der Bildwechsel: die GPU ist dort nur zu 77 % belegt.
- `move`: Schreibbandbreite. 48 Byte je Objekt in fünf Speicherbefehlen (vec3, float, vec4, vec3,
  float) mit 48 Byte Abstand: 1M = 48 MB in 0,26 ms ≈ 185 GB/s, 41 % der Spitze von 448 GB/s;
  die Rechnung (261 Befehle je Objekt laut Interpreter) allein wäre mehrfach schneller. Linear in
  N: 0,26 ms je Million.
- `cull`: Lesebandbreite. Es braucht nur Position und Radius (16 Byte), liest wegen der
  32-Byte-Sektoren aber jeden Datensatz ganz: 48 MB in 0,133 ms ≈ 360 GB/s, 80 % der Spitze.
- `draw` bis 1440p bei großem N: Geometrie-Durchsatz, also Vertex-Aufrufe und die
  Primitiv-Verarbeitung der festen Funktion. Die Zeichenzeit für 1M ist bei 320 × 180 und bei
  1080p fast gleich (1,19 bzw. 1,26 ms); Speicher ist es nicht (25 MB Objektdaten je Bild
  ≈ 20 GB/s), Rechenleistung auch nicht (der Vertex-Shader ist kurz). Je Würfel im Sichtkegel
  ~2,25 ns: 24 Vertex-Aufrufe und 12 Dreiecke, die Hälfte davon nach dem Vertex-Shader als
  Rückseite verworfen ≈ 10 G Vertices/s und 5 G Dreiecke/s.
- `draw` bei 4K und bei kleinem N: Rasterung, früher Tiefentest, Fragment-Shader und Schreiben
  von Farbe und Tiefe. Über der Geometrie-Grundlast kommen bei 1M in 4K ~0,8 ms hinzu, bei 100k
  wächst die Zeit oberhalb von 720p fast mit der Pixelzahl (0,23 → 0,64 ms von 1080p zu 4K;
  bei kleinem N sind die Würfel nah und groß).
- `single`: die CPU-Aufzeichnung. ~87–90 ns je Objekt, getrennt gemessen ~37 ns für
  `vkCmdPushConstants` und ~55 ns für `vkCmdDrawIndexed`; `lib/gpu` legt je Aufruf nur ein paar
  Vergleiche darüber. Die GPU verarbeitet einen Einzel-Draw in ~8–11 ns (≈ 100 M Draws/s) und ist
  zu 10–15 % belegt. Auch mit freier CPU wäre der Pfad auf der GPU langsamer als `indirect` mit
  Culling (1M: 10,7 statt 1,26 ms draw), weil er alle N und jeden Würfel als eigenen Draw zeichnet.
- Bilder in Flug: mit K = 1 wartet die GPU auf jede Aufzeichnung (100k: 62 % belegt, Bildzeit
  0,43 statt 0,30 ms); ab K = 2 ist sie zu 94–99 % belegt, K = 3 bringt hier nichts mehr, puffert
  aber Aussetzer der CPU besser ab.

**Was weitere Gewinne begrenzt, und was G5 daraus mitnimmt.**

- Die Zeichenzeit ist Geometrie: jeder sichtbare Würfel kostet 24 Vertex-Aufrufe und 12 Dreiecke,
  von denen die Hälfte erst nach dem Vertex-Shader als Rückseite wegfällt, und der Vertex-Shader
  rechnet die Drehmatrix aus dem Quaternion 24-mal je Würfel. Viele der ~530 000 Würfel im
  Sichtkegel verdecken sich gegenseitig (das Bild ist eine dichte Wand aus Würfeln), und die
  fernen sind nur wenige Pixel groß. Größter Hebel ist deshalb Verdeckungs-Culling (Hi-Z-Pyramide
  aus dem Tiefenpuffer des Vorbilds, zweistufig), dann Detailstufen (ferne Würfel als wenige
  Dreiecke oder Punkte) und auf Stufe Voll Mesh-Shader, die Rückseiten je Objekt vor der
  Rasterung verwerfen (3 statt 6 Seiten). Die Transformation gehört einmal je Instanz in den
  Rechen-Shader (kompakte Instanzdaten mit fertiger Matrix), nicht in jeden Vertex.
- Rechnen ist reine Bandbreite: Bewegung und Culling sollten auf kompakten Daten laufen (Struktur
  aus Arrays: 16-Byte-Bounding-Spheres fürs Culling, Transformationen getrennt, wo sinnvoll
  halbe Genauigkeit), nur Geändertes schreiben und das Culling nicht 48 Byte je Objekt lesen
  lassen.
- Eine Warteschlange: `move` und `cull` des nächsten Bildes können nicht neben dem Zeichnen des
  laufenden laufen (die Objekte gibt es einmal, die Barriere am Bildanfang serialisiert). Eine
  eigene Rechen-Warteschlange in `lib/gpu` und doppelt gepufferte Objektdaten würden bei 1M
  ~0,4 ms je Bild verstecken.
- Die CPU ist mit dem GPU-getriebenen Pfad aus dem Spiel (0,1 ms je Bild für 1M Objekte, auch
  `vkQueueSubmit` und `present` kosten mehr als das Aufzeichnen). Ein Draw je Objekt dagegen
  begrenzt auf ~10 000 Objekte je Millisekunde CPU, und auch die GPU schafft nur ~100 M Draws/s.
  G5 soll Objekte nie einzeln zeichnen: Instanzlisten je Mesh, gefüllt vom Culling,
  `draw_indexed_indirect_count` / `_multi` (Stufe Voll) für mehrere Meshes und Materialien in
  einem Aufruf, Basis mit einem indirekten Draw je Mesh.
- Bilder in Flug: 2–3 genügen. Bei sehr kurzen Bildern (0,3 ms) puffern drei Bilder aber nur
  ~1 ms, und Aussetzer der CPU (Zeitscheiben von Windows unter Last) lassen die GPU leerlaufen.
  Je Bild in Flug eigene Kamera- und Uniform-Daten (hier ein Staging-Slot und eine Kopie am
  Bildanfang), Zeitstempel je Durchgang zum Profilieren behalten; für die Analyse fehlen in
  `lib/gpu` noch Pipeline-Statistiken (Vertex-, Primitiv- und Fragment-Zähler).
- Für große Szenen Reverse-Z (Tiefe 1 → 0 mit Float-Tiefenpuffer) statt des Vertrags mit Tiefe
  0 = near; die Kamera hier (near 0,1, far 900) reicht nur, weil die Würfel weit auseinander
  liegen.

**WGSL gegen SPIR-V** (G2c: die sechs Shader aus `lib/wgsl/bench`, von `lib/wgsl` übersetzt,
gegen die handgeschriebenen; dieselben Messbedingungen, gleichzeitig ein Release-Build in einem
anderen Prozess; Median aus drei Läufen je Konfiguration, abwechselnd, Spanne der GPU-Gesamtzeit
in Klammern; ms):

| N, Ziel, Modus | Shader | GPU move / cull / draw | GPU gesamt (Spanne) | Bildzeit Mittel |
| --- | --- | ---: | ---: | ---: |
| 1M, 1080p, indirekt | SPIR-V | 0,257 / 0,133 / 1,242 | 1,633 (1,623–1,635) | 1,670 |
| | WGSL | 0,251 / 0,133 / 1,234 | 1,617 (1,614–1,628) | 1,647 |
| | WGSL ohne Robustheit | 0,244 / 0,133 / 1,236 | 1,613 (1,609–1,617) | 1,649 |
| 1M, 4K, indirekt | SPIR-V | 0,256 / 0,133 / 1,937 | 2,326 (2,318–2,331) | 2,370 |
| | WGSL | 0,251 / 0,133 / 1,927 | 2,313 (2,309–2,319) | 2,355 |
| | WGSL ohne Robustheit | 0,245 / 0,133 / 1,929 | 2,309 (2,304–2,314) | 2,345 |
| 100k, 1080p, einzeln | SPIR-V | 0,036 / – / 0,660 | 0,695 (0,693–0,798) | 6,390 |
| | WGSL | 0,041 / – / 0,670 | 0,711 (0,710–0,712) | 6,434 |
| | WGSL ohne Robustheit | 0,041 / – / 0,661 | 0,701 (0,701–0,703) | 7,508 |

Die übersetzten Shader sind so schnell wie die handgeschriebenen (`move` sogar 2 % schneller);
die Robustheit kostet in `move` 0,007 ms (+2,8 %), sonst nichts Messbares. Der Einzel-Pfad ist
CPU-gebunden (100 000 Draws je Bild); seine Bildzeiten schwanken mit der Last des Rechners.

## Bekannte Lücken

- Nur Windows. Linux und Android brauchen einen Weg zu `libvulkan.so` (JFLib lädt unter Linux
  ohne libc) und ihre Fensterflächen; das Backend übersetzt dort, `Instance::new` gibt
  `NoLoader`.
- Eine Warteschlange; keine eigene Rechen- oder Kopierwarteschlange (die Messszene könnte so
  `move` und `cull` neben dem Zeichnen laufen lassen).
- Keine Pipeline-Statistik-Abfragen (Vertex-, Primitiv-, Fragment-Zähler); nur Zeitstempel.
- Kein Pipeline-Cache, kein MSAA, keine Stencil-Operationen, keine Textur-zu-Textur-Kopien und
  kein Blit (Mip-Erzeugung).
- `Format` hat kein `Rgba32Sint` (und keine anderen vierkanaligen 8-/16-Bit-Ganzzahlformate);
  gputest überspringt diese Speichertextur.
- Bindungen ohne Arrays (`count` 1); Arrays gibt es über die Bindless-Tabelle auf Stufe Voll.
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
- Overrides: skalare Spezialisierungskonstanten, `SpecId` = Nummer der Pipeline-Konstante (auch
  für Overrides ohne `@id`, siehe „Pipeline-Konstanten“). Abgeleitete Overrides mit
  Gleitkomma-Arithmetik oder eingebauten Funktionen kann SPIR-V nicht ausdrücken
  (`OpSpecConstantOp` nur ganzzahlig): `lib/wgsl` rechnet beim Anlegen der Pipeline jedes
  Override auf der CPU aus und gibt jeder Spezialisierungskonstante ihren fertigen Wert
  (`wgsl::Output::constants`, docs/WGSL.md „Overrides“). `@workgroup_size` mit Overrides: auf
  Basis das `WorkgroupSize`-Built-in (ein Modul je Entry Point), auf Voll `LocalSizeId`.
- Umgesetzt in [`lib/wgsl`](WGSL.md) (G2c); `lib/gpu` schaltet dafür zusätzlich ein, wo das Gerät
  es hat: 16-Bit-Werte in Uniform-Puffern und Push-Konstanten, uniformBufferStandardLayout,
  shaderDemoteToHelperInvocation, shaderClipDistance, geometryShader (primitive_index),
  sampleRateShading, dualSrcBlend, shaderStorageImageExtendedFormats,
  shaderStorageImageReadWithoutFormat (`Features::uniform_16bit`, `push_constant_16bit`,
  `uniform_standard_layout`, `demote`, `clip_distances`, `primitive_index`, `sample_shading`).
- Bindless-Tabelle: in der Gruppe, in die das Programm sie setzt; Bindung 0 Laufzeit-Array von
  Texturen, 1 Sampler, 2 Speicherpuffer (Laufzeit-Array von `Block`-Strukturen, ohne
  `ArrayStride` am äußeren Array), 3 Speichertexturen. `RuntimeDescriptorArray` (unter 1.5 mit
  `SPV_EXT_descriptor_indexing`); nicht einheitliche Indizes mit `NonUniform`.
- Kein Kippen von y: das Backend kippt den Viewport. Fragment-Shader mit `OriginUpperLeft`.
- Vertex-Pulling über `VertexIndex` / `InstanceIndex` (Vulkan-Bedeutung: einschließlich
  Basis-Vertex und erster Instanz); `BaseVertex`, `BaseInstance`, `DrawIndex` mit
  `Features::draw_parameters`.

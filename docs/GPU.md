# GPU -- explizite GPU-Schicht für UI, 2D-CAD und 3D-Spiele

`lib/gpu` gibt Programmen direkten Zugriff auf die Grafikkarte: Rechen-Shader, Rendern mit
Tiefentest, indirekte Aufrufe, Zeitstempel und Fenster-Swapchains. Die API ist explizit wie
Vulkan und WebGPU (Instanz, Adapter, Gerät, Warteschlange, Ressourcen, Pipelines,
Befehlspuffer, Barrieren) und neutral gegenüber dem Backend: das Backend wird pro Ziel beim
Übersetzen gewählt, es gibt keine dynamische Verteilung.

Stand: Vulkan unter Windows, unter Linux ([Linux](#linux)) und unter Android ([Ausgabe unter Android](#ausgabe-unter-android)),
WebGPU im Browser ([Ausgabe im Browser](#ausgabe-im-browser)).
`vulkan-1.dll`, `libvulkan.so.1` bzw. `libvulkan.so` wird zur Laufzeit geladen, das Programm startet also auch auf
Rechnern ohne Vulkan und bekommt dort einen sauberen Fehler. Geplant:
später Metal -- mit denselben Typen und Methoden. Die Oberfläche rastert weiter auf der CPU; unter
Windows, Linux, Android und im Browser zeigt die GPU das Ergebnis
(G3a, [Ausgabe der UI](#ausgabe-der-ui), unter Linux als dmabuf: G4b, [Ausgabe unter Linux](#ausgabe-unter-linux),
unter Android auf einem `SurfaceView`: G4c, [Ausgabe unter Android](#ausgabe-unter-android),
im Browser auf einer eigenen Leinwand über `#screen`: G4d, [Ausgabe im Browser](#ausgabe-im-browser);
`jfgui_GPU=0` schaltet das ab), die Ebenen über dem Grund
und große Bilder als eigene Texturen (G3b, [Ebenen und Bilder](#ebenen-und-bilder)).

| Teil | Wo |
| --- | --- |
| Öffentliche Typen: Beschreibungen, Formate, Fehler, Features, Limits, `Pod` | [`lib/gpu/lib.rs`](../lib/gpu/lib.rs) |
| Instanz, Adapter, Validierung | [`lib/gpu/vk_inst.rs`](../lib/gpu/vk_inst.rs), Laden der Bibliothek: [`vk_load.rs`](../lib/gpu/vk_load.rs) |
| Programme gegen glibc (Linux) | `build_rs.py` Abschnitt „glibc“, [`lib/plat/linux_glibc.rs`](../lib/plat/linux_glibc.rs), [Linux](#linux) |
| Gerät, Warteschlange, Zeitleiste, verzögertes Freigeben, Uploads | [`lib/gpu/vk_dev.rs`](../lib/gpu/vk_dev.rs) |
| Speicher-Unterverteilung | [`lib/gpu/vk_mem.rs`](../lib/gpu/vk_mem.rs) |
| Puffer, Texturen, Sampler, Shader, Bindungen, Bindless, Pipelines, Zeitstempel | [`lib/gpu/vk_res.rs`](../lib/gpu/vk_res.rs) |
| Befehle aufzeichnen | [`lib/gpu/vk_cmd.rs`](../lib/gpu/vk_cmd.rs) |
| Fensterflächen und Swapchains | [`lib/gpu/vk_swap.rs`](../lib/gpu/vk_swap.rs); unter Linux dieselben Typen über dmabuf: [`vk_dmabuf.rs`](../lib/gpu/vk_dmabuf.rs) |
| Fläche, Thread und `libvulkan.so` unter Android | [`lib/plat/android_gpu.rs`](../lib/plat/android_gpu.rs), [`android/GalleryActivity.java`](../android/GalleryActivity.java), [`android/bridge.c`](../android/bridge.c) |
| Puffer der GPU am Fenster (Linux) | [`lib/plat/linux_gpuout.rs`](../lib/plat/linux_gpuout.rs), [`linux_wl_gpu.rs`](../lib/plat/linux_wl_gpu.rs) (zwp_linux_dmabuf_v1), [`linux_x11_gpu.rs`](../lib/plat/linux_x11_gpu.rs) (DRI3, Present) |
| Vulkan-Deklarationen (von Hand, mit Layout-Prüfungen gegen die Header) | [`lib/gpu/vk_ffi.rs`](../lib/gpu/vk_ffi.rs) |
| WebGPU im Browser | [`lib/gpu/web_dev.rs`](../lib/gpu/web_dev.rs), [`web_res.rs`](../lib/gpu/web_res.rs), [`web_cmd.rs`](../lib/gpu/web_cmd.rs), [`web_swap.rs`](../lib/gpu/web_swap.rs), die Seite des Browsers: [`web/gpu.js`](../web/gpu.js); die Leinwand: [`lib/plat/web_gpu.rs`](../lib/plat/web_gpu.rs), [`web/loader.js`](../web/loader.js) |
| Fenster-Handles für die GPU | `plat::win_native` ([`lib/plat/win_api.rs`](../lib/plat/win_api.rs)), das Kindfenster der UI-Ausgabe: `plat::win_gpu_child` ([`lib/plat/win_gpuout.rs`](../lib/plat/win_gpuout.rs)) |
| Ausgabe der UI über die Swapchain | [`lib/ui/gpu_out.rs`](../lib/ui/gpu_out.rs), Shader: [`gpu_out.wgsl`](../lib/ui/gpu_out.wgsl), [`gpu_out_spv.rs`](../lib/ui/gpu_out_spv.rs) |
| Tests und Messungen | [`apps/gputest`](../apps/gputest/main.rs), die UI-Ausgabe: [`apps/uigputest`](../apps/uigputest/main.rs), [`apps/gallery/tests/gpu_output.uis`](../apps/gallery/tests/gpu_output.uis), [`gpu_layers.uis`](../apps/gallery/tests/gpu_layers.uis); Android: [`apps/androidGallery/tests/android_gpu.py`](../apps/androidGallery/tests/android_gpu.py), [`apps/gputest/tests/android.py`](../apps/gputest/tests/android.py); Browser: [`apps/gallery/tests/gpu_web.mjs`](../apps/gallery/tests/gpu_web.mjs), [`apps/webgputest`](../apps/webgputest/main.rs), [`tools/tests/web_gpu_bench.mjs`](../tools/tests/web_gpu_bench.mjs) |

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
  `adapters_low_power(display)` (für die Ausgabe der UI; `low_power_rank`), `prepare()` (im Browser
  kommen Adapter und Gerät später, anderswo sofort `true`).
  `AdapterInfo`: Name, Hersteller, Gerätekennung, Art, Treibername und -version, API-Version.
  `adapter.features()`, `limits()`, `level()`, `can_present(&surface)`,
  `create_device(&DeviceDesc)`.
- **Gerät**: `queue()`, `features()`, `limits()`, `level()`, `wait(id, timeout_ns)`,
  `wait_idle()`, `poll()`, `completed()`, `is_done(id)`, `is_lost()`, `memory_reserved()`, `test_lose()`
  (Testhilfe).
- **Ressourcen**: `create_buffer` (`MemoryKind::Device` / `Upload` / `Readback`; die beiden
  letzten sind dauerhaft abgebildet: `write`, `read`), `create_texture` (2D, 2D-Array, Würfel,
  Würfel-Array, 3D, Mip-Stufen, Tiefenformate, BC1..BC7 wenn vorhanden; `view()`,
  `create_view(&ViewDesc)`), `create_sampler`, `ShaderModule::from_spirv`, im Browser
  `ShaderModule::from_wgsl`.
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
- **Rechen-Warteschlange** (G5d, [unten](#rechen-warteschlange-g5d)): `device.compute_queue()` gibt
  `Option<&ComputeQueue>` (`encoder()`, `submit`, `submit_after(&[id], ..)`); `queue.submit_after(&[id], ..)`
  wartet auf Übergaben der anderen.
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
  plat_fenster)` (ein Kindfenster; Android: das `ANativeWindow` des `SurfaceView`), `create_swapchain(&surface, &SwapchainDesc)`, `acquire()`
  (`None`, solange das Fenster minimiert ist oder keine Fläche hat), `try_acquire()` (wartet nie,
  baut nie neu: `Frame`, `NotReady`, `Stale`, `NoArea`), `rebuild()` (auch auf einem anderen
  Thread), `view(&frame)`, `texture(&frame)`, `present(frame, [cmd, ..])`, `resize()`,
  `set_present_mode(m)`, `size()`, `format()`, `present_mode()`. Für denselben Code auf jedem Ziel
  außerdem `prepare()`, `flush()`, `busy()`, `next_look(retry)`, `damage(rects)` und
  `Surface::track_size()`: unter Windows wirkungslos, unter Linux [Ausgabe unter Linux](#ausgabe-unter-linux).

## Konventionen

Sie gelten für jedes Backend und für den WGSL-Übersetzer:

- **Koordinaten** wie WebGPU: normierte Gerätekoordinaten mit y nach oben, Tiefe 0 bis 1,
  Ursprung von Framebuffer und Texturen oben links. Das Vulkan-Backend kippt dafür den Viewport
  (negative Höhe); Shader kippen nichts. Vorderseiten sind gegen den Uhrzeigersinn
  (`front_cw` dreht das um).
- **Bindungen**: Gruppe g, Bindung b einer Pipeline-Struktur = WGSL `@group(g) @binding(b)` =
  SPIR-V `DescriptorSet g`, `Binding b`. Höchstens 4 Gruppen.
- **Push-Konstanten**: ein Block ab Offset 0, für alle Stufen sichtbar, höchstens
  `PUSH_CONSTANTS_MAX` = 128 Byte. Das WebGPU-Backend bildet sie über einen kleinen
  Uniform-Puffer nach ([Ausgabe im Browser](#ausgabe-im-browser)).
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
  ausgerechnet (docs/WGSL.md „Overrides“); das WebGPU-Backend setzt den Schlüssel selbst (die
  Nummer als Text, ab 0x10000 den Namen, den es im WGSL gelesen hat).
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

## Rechen-Warteschlange (G5d)

**Eine zweite Warteschlange für Rechen-Shader und Kopien, neben der des Geräts.** `Device::compute_queue()`
gibt sie, wo das Gerät eine Familie mit Compute und ohne Grafik hat (die RTX 2070 SUPER: Familie 2), sonst
`None`; im Browser immer `None` (WebGPU hat eine Warteschlange), und `jfgui_GPU_COMPUTE=0` schaltet sie ab.
Ohne sie läuft alles über `device.queue()`: Wer `Option` prüft, hat beide Wege mit demselben Code.

```rust
let cq = dev.compute_queue();                       // Option<&ComputeQueue>
let mut e = match cq { Some(q) => q.encoder()?, None => dev.encoder()? };
e.set_compute_pipeline(&p); e.set_bind_group(0, &g); e.dispatch(n, 1, 1);
let id = match cq { Some(q) => q.submit([e.finish()?])?, None => dev.queue().submit([e.finish()?])? };
dev.queue().submit_after(&[id], [weiter.finish()?])?;   // wartet über die Zeitleiste, an allen Befehlen
dev.wait(id, timeout)?;                             // Device::wait, is_done, wait_idle nehmen beide
```

- **Eine Zeitleiste je Warteschlange.** Zwei Warteschlangen, die dasselbe Zeitleisten-Semaphor signalisieren,
  könnten seine Werte in falscher Reihenfolge erreichen (ein Signal muss größer sein als der Stand). Die
  `SubmitId` der Rechen-Warteschlange trägt deshalb Bit 63 (`SubmitId` ist ein öffentliches Tupel, ein Feld mehr
  ginge nicht additiv); `Device::wait`, `is_done` und `submit_after` erkennen es. `submit_after` wartet an
  allen Befehlen (`ALL_COMMANDS`); feinere Wartestufen sind nicht gebaut.
- **Gemeinsame Ressourcen: concurrent sharing, keine Übergabe des Besitzes.** Jeder Puffer und jede Textur, die
  kein Renderziel ist, wird mit beiden Familien angelegt (`VK_SHARING_MODE_CONCURRENT`). Eine Übergabe des Besitzes
  bräuchte für jede Ressource bei jedem Wechsel eine Freigabe-Barriere auf der einen und eine Übernahme-Barriere auf
  der anderen Warteschlange, im Code des Aufrufers, in derselben Reihenfolge; im Browser und ohne zweite Familie
  wären es leere Befehle. Concurrent kostet bei Puffern nach allem Bekannten nichts (gemessen: unten); bei Texturen
  kann es auf manchen GPUs die Kompression von Renderzielen kosten, deshalb bleiben Farb- und Tiefenziele exklusiv
  bei der Familie des Geräts (Render-Durchgänge laufen nur dort). Eine Textur, die die Rechen-Warteschlange
  benutzt, darf also kein Renderziel sein.
- **Kodierer je Familie.** `ComputeQueue::encoder()` gibt einen Kodierer aus einem Befehlspool ihrer Familie: Kopien,
  Füllen, Barrieren, Rechen-Shader, Zeitstempel (wo die Familie welche hat; die RTX hat sie). Ein Render-Durchgang
  ist `Invalid` (aus `finish`). Seine Barrieren behalten nur Stufen und Zugriffe, die eine Rechen-Familie kennt
  (Compute, Kopie, indirekte Argumente, Host); was Grafik betrifft (in `Access` oder im Layout einer Textur), fällt
  weg, statt dass die Validierung es meldet. Ein Befehlspuffer der einen Familie auf der anderen Warteschlange ist
  `Invalid`.
- **Uploads** (`queue.write_buffer`, `write_texture`) laufen weiter vor der nächsten Übergabe der Warteschlange
  des Geräts. Eine Übergabe an die Rechen-Warteschlange schickt sie, wenn welche warten, zuerst dort ab und wartet
  darauf.
- **Lebensdauer.** Eine fallen gelassene Ressource wartet auf die jeweils letzte Übergabe beider Warteschlangen.

**Getestet** (`gputest --test queues`, unter Validierung): acht Runden `b = 3b + k` auf einem Puffer, abwechselnd
auf beiden Warteschlangen, jede nach der davor (`submit_after`), alle abgeschickt, bevor die CPU einmal wartet, die
erste liest, was `write_buffer` bereitgelegt hat: 0 von 65 536 Werten falsch; eine Speichertextur, auf der
Rechen-Warteschlange geschrieben (ihre Layouts dort gewechselt), auf der des Geräts gelesen: 0 von 960 Texeln falsch;
`wait` und `is_done` mit der `SubmitId` der Rechen-Warteschlange; ein Puffer, der fällt, während sie ihn noch
benutzt; Zeitstempel dort; verweigert: Befehlspuffer der anderen Familie in beide Richtungen, ein Render-Durchgang,
ein Warten auf eine Übergabe, die es nicht gibt; ohne Rechen-Warteschlange (`jfgui_GPU_COMPUTE=0`) dieselben acht
Runden auf einer. **Absichtlich fehlende Synchronisation:** zwei Dispatches auf demselben Puffer ohne Barriere in
einem Befehlspuffer der Rechen-Warteschlange meldet die Validierung (1 Fehler). Ein fehlendes Warten *zwischen*
den Warteschlangen meldet die Schicht (1.4.363) nicht, und auch zwei Übergaben an *eine* Warteschlange ohne
Synchronisation dazwischen meldet sie nicht (geprüft mit einer ersten Übergabe, die noch lief, als die zweite kam:
0 Meldungen). Deshalb prüft der Test das Warten an den Werten: ein langer Dispatch (1 Mio. × 20 000 Schritte) auf
der Warteschlange des Geräts, danach die Rechen-Warteschlange auf seinem Puffer. Mit `submit_after`: 0 von 512
Stichproben falsch; ohne: 260 von 512 falsch.

**Ein Absturz unter Validierung, nicht aufgeklärt.** Zweimal stürzte `gputest --test all --validate` mit Rechen-Warteschlange
ab (`0xC0000005` in einer DLL, nicht im Programm), beide Male beim Abbau des Tests `bindless`, nachdem `upload` eine
Swapchain benutzt hatte; ohne Validierung, mit `jfgui_GPU_COMPUTE=0`, ohne den Swapchain-Teil und mit denselben Tests
in Teilfolgen nicht, auch nicht in der genauen Folge bis `bindless`, die beim ersten Mal abstürzte. Danach fragt
`lib/gpu` die Zeitleiste der Rechen-Warteschlange nur noch, wenn auf ihr etwas aussteht (vorher bei jedem Aufräumen),
und die ganze Reihe lief grün; ob das die Ursache war, ist nicht belegt (vermutet: die Schicht).

**Läuft etwas nebeneinander?** Zwei lange Dispatches (1 Mio. × 2000 Schritte, je ein Puffer) nacheinander auf der
Warteschlange des Geräts: 1,75 ms; je einer auf jeder: 1,46 ms (ein Lauf, `gputest --test queues`). Die
Messungen in render3d stehen in [RENDER3D.md](RENDER3D.md#zweite-warteschlange-g5d).

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
Windows zeigt die GPU ihn ([`lib/ui/gpu_out.rs`](../lib/ui/gpu_out.rs), G3a), unter Linux und Android derselbe Code
([Ausgabe unter Linux](#ausgabe-unter-linux), G4b; [Ausgabe unter Android](#ausgabe-unter-android), G4c; dort auch, was anders ist): die
Änderungsrechtecke, die die Laufzeit ohnehin berechnet, gehen in eine Textur je Fenster, ein
Dreieck zeichnet die Textur ins Swapchain-Bild, das Bild wird dargestellt. Alle Fenster teilen ein
Gerät. **Das ist der Standard** (unten: was es kostet). `jfgui_GPU=0`
schaltet die GPU überall ab. Skripte und Läufe ohne Fenster bleiben auf dem CPU-Pfad, außer ein
Skript verlangt mit `gpu on` anderes. Statische Linux-Programme linken nichts davon
(`lib/ui/gpu_out_off.rs`, the `[target]` tables in `lib/ui/Cargo.toml`).

**Der Weg eines Fensters.**

| Stufe | Was geschieht |
| --- | --- |
| CPU | `plat::win_present` wie ohne GPU. Das erste Bild jedes Programms geht so |
| Job | Instanz, Gerät, Pipeline und Swapchain entstehen auf einem Hilfsthread, der erst nach dem ersten Bild startet und danach endet. Die CPU zeichnet weiter |
| Wechsel | der ganze Framebuffer in die Textur, ein `present` auf dem verborgenen Kindfenster, dann das Kindfenster zeigen und die Kopie für `WM_PAINT` freigeben |
| GPU | je Bild: Rechtecke hochladen, `try_acquire`, zeichnen, `present` |
| zurück | bei neuer Größe, einem Fehler oder vor dem Verbergen erst das Bild über den CPU-Pfad zeigen, dann das Kindfenster verbergen; keine Lücke, kein falsches Bild |

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

**Aus- und Einblenden.** Das Bild einer Swapchain kommt mit ihrem Fenster nicht zurück: Ein verborgenes
und wieder gezeigtes Fenster blieb weiß, bis ein neues Bild kam, und ein unverändertes Fenster stellt keines
dar (gemessen auf einer RTX 2070 und auf einem Surface; ein Fokuswechsel half, weil er neu zeichnet). Deshalb
geht ein Fenster vor dem Verbergen auf den CPU-Pfad zurück (`gp_window_show`), für ein verborgenes Fenster
wird nichts gebaut oder gewechselt, und beim Zeigen steht das Bild aus der Kopie sofort da (`UpdateWindow`
nach `ShowWindow`: `WM_PAINT` käme sonst erst, wenn die Nachrichtenschlange leer ist, und ein schwebender
Stift hält sie voll; bis dahin verdeckt, `DWMWA_CLOAK`, sonst wären die ersten Zusammensetzungen weiß). Die Swapchain folgt wie nach einer Größenänderung. Dasselbe gilt, wenn das System ein
Werkzeugfenster mit seinem minimierten Besitzer verbirgt (`WST_HIDDEN`). `uigputest --test windows` liest das
wiedergekommene Fenster vom Bildschirm.

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
`apps/uigputest` (Windows und Linux; echtes Fenster, echte Fensterschleife, jeder Test ein Skript):
`switch`, `resize`, `minimize`, `dpi`, `windows`, `off`, `noloader`, `lose`, `idle`, `csd`, `monitors`, `shaders`,
`adapter`, `hwnd`, `layers`, `images`, `imgplain`, `canvas`, `canvasplain`, `canvasoff`, `nodmabuf`, `refused`; dazu `apps/gallery/tests/gpu_layers.uis` und
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

`ui.canvas_gpu_wanted()` sagt, ob sich das Bauen lohnt. **Auf jedem Ziel derselbe Code:** ohne GPU-Ausgabe, in statischen Linux-Programmen, in Skripten ohne `gpu on`, mit `jfgui_GPU=0` tut `canvas_gpu` nichts, das `Draw` malt jedes Bild wie bisher.
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

Beim Strich kein Unterschied (Streuung der Läufe 262–334 und 273–314 µs); ein Strich berührt keine Kachel außer durch `canvas_gpu_damage`, die in Mikrosekunden frei wird, und wartet auf nichts. Ein gezogener Knoten im `diagram` läuft wie vorher über die CPU: sein Inhalt ändert sich, also keine Kacheln (in `diagramtest` je Bild im Median 442 µs vorher, 389 µs nachher, 3 Läufe; das Ziehen zeichnet dort nur Überlagerungen, das Loslassen einmal alles).

Wo der Gewinn kleiner ist: **unter 1 ms** bleibt nur, was keine Zellen über der Leinwand hat. Das Foto hat Bildlaufleisten und eine Statuszeile, deren Zellen je Bild `IvDraw` kosten (170 000 Pixel, 1,4 ms von 2 ms), PaintCraft Lineale und Leisten. Beim Rad in einem Zug zählen die neuen Stufen: das Foto 113 Kacheln in 16 Bildern (5 ms je Bild), das Diagramm 163. Das erste Bild einer Geste füllt seine Kacheln: das längste Bild war beim Foto 14,7 ms (vorher 39), Kritzel 11,9 (15), PaintCraft 12–15 (14,5), Diagramm 6,3 (26), hier ohne die 5-ms-Grenze, die für Skripte nicht gilt. Das Bild danach kostet genau ein Bild wie vorher, wer also alle 200 ms ruckelt, zahlt fast so viel wie zuvor. Bei einem kleinen Fenster oder einer kleinen Zeichnung bringt das alles entsprechend weniger.

Speicher (`memory_reserved`, Blöcke der GPU und Staging): vorher 16–23 MiB je nach Zustand, mit Leinwänden je nach Zahl der Kacheln: 50 MiB (Kritzel, PaintCraft: 32 Kacheln, 8 MiB), 82 MiB (Foto 75 Kacheln, 19 MiB; Diagramm 82, 21 MiB), 146 MiB nach einem Zoom über viele Stufen (188–245 Kacheln, 48–62 MiB). Der Rest ist Staging des Hochladens (ein ganzes Bild im Ring) und Blockreste. Nach 30 s ohne Bewegung gehen die Kacheln ohne ein Bild (`uigputest --test canvas`, kleines Fenster: 21 → 17 MiB). Eine Leinwand, die nie bewegt wird, kostet nichts.

Größe, Release, gegen den Stand davor: gallery Windows 4 153 856 → 4 201 984 Byte (+48 128), PaintCraft 8 624 128 → 8 677 376 (+53 248), gallery Linux 4 441 480 → 4 442 808 (+1 328), Browser 3 542 127 → 3 584 472 (+42 345), simd 3 481 482 → 3 531 406 (+49 924), threads 3 490 369 → 3 496 433 (+6 064). Unter Linux kommt fast nichts dazu, im Browser ist die Spanne so groß, dass das nach Inlining-Schwankung aussieht; nachgegangen bin ich dem nicht. Keine Vulkan-Zeichenkette in beiden.

### Ausgabe unter Linux

Derselbe Presenter wie unter Windows (`lib/ui/gpu_out.rs`), mit denselben Stufen, Rückfällen, Ebenen, Bildern und
Leinwänden. Anders ist, was unter ihm liegt. Vulkans Wayland-Fläche will einen Zeiger aus `libwayland-client`, die für X11
einen aus `libxcb`; JFLib spricht beide Protokolle selbst und hat beide nicht. Also zeichnet die GPU in eigene Bilder
(`Swapchain` in [`vk_dmabuf.rs`](../lib/gpu/vk_dmabuf.rs), keine `VkSurface`), exportiert sie als dma-buf, und der eigene
Protokollcode hängt sie ans Fenster.

**Der Weg eines Bildes.**

| Schritt | Wayland | X11 |
| --- | --- | --- |
| Was der Bildschirm nimmt | `zwp_linux_dmabuf_v1` ab Version 4: Default-Feedback (Formattabelle, Hauptgerät, Tranchen des Hauptgeräts); Version 3: Modifier-Events; darunter CPU-Pfad. Erst beim ersten Bedarf gefragt | DRI3 1.2 und Present 1.0, sonst CPU-Pfad; `DRI3Open` nennt das Gerät (`st_rdev`), `GetSupportedModifiers` für Tiefe 24, 32 Bit |
| Bilder | XRGB8888 (sonst XBGR, ARGB, ABGR), Modifier mit einer Ebene, die Gerät und Bildschirm beide können; der Treiber wählt (`VkImageDrmFormatModifierListCreateInfoEXT`), eigene Speicherbelegung, `vkGetMemoryFdKHR` | ebenso, nur XRGB8888 |
| Anmelden | `zwp_linux_buffer_params_v1.create`, nicht `create_immed`: ein Import, der scheitert, gibt `failed` und den CPU-Pfad, `create_immed` darf die Verbindung beenden | `PixmapFromBuffers`, ein X-Fehler gibt den CPU-Pfad |
| Zeichnen | Render-Durchgang wie unter Windows; davor holt ein Barrier das Bild von `VK_QUEUE_FAMILY_FOREIGN_EXT`, danach gibt einer es zurück, Layout `GENERAL` | ebenso |
| Zeigen | erst wenn die Zeitleiste die Übergabe erreicht hat (`Swapchain::flush`, kein Warten): attach, damage_buffer, frame, commit | ebenso: `PresentPixmap`, die Rechtecke als Update-Region (XFixes) |
| Nächstes Bild | nach `wl_callback.done` (höchstens 1 s) | nach `PresentCompleteNotify` (höchstens 1 s) |
| Bild wieder frei | nach `wl_buffer.release` und wenn `poll(POLLOUT)` auf dem dma-buf keine Leser-Fence mehr meldet | nach `PresentIdleNotify`, ebenso |

Ein Wechsel zwischen Shared Memory und dmabuf auf einem `wl_surface` oder X-Fenster braucht kein Kindfenster: ein `wl_buffer`
ist ein `wl_buffer`. `win_gpu_child` heißt nur so wie unter Windows und sagt, was der Bildschirm nimmt. Viewport, Fractional
Scale, Bestätigen des Configure und Fenstergeometrie laufen durch denselben Code wie beim Shared Memory
(`surface_state`, `commit_buffer` in [`linux_wl_present.rs`](../lib/plat/linux_wl_present.rs)).

**Warum nie ein falsches Bild zu sehen ist.** Implicit Sync setzt voraus, dass Treiber und Kernel Fences an den dma-buf hängen
(ohne WSI nur mit `DMA_BUF_IOCTL_IMPORT_SYNC_FILE`, Linux ab 6.0) und der Compositor sie beachtet. Darauf baut hier nichts:

- Ein Bild geht erst ans Fenster, wenn die GPU fertig ist. Beim attach hängt also keine Fence daran, auf keinem Compositor, Kernel oder Treiber.
- In ein Bild wird erst gezeichnet, wenn der Compositor es zurückgegeben hat, keine Leser-Fence mehr daran hängt (seine GPU kann beim `release` noch lesen) und es nicht das neueste ist (ein erneut gezeigtes Fenster bekommt es wieder).
- `plat::win_present` beendet den Weg über die Puffer: ein Bild unterwegs wird verworfen, das Shared Memory zeigt das ganze Bild. Was das Shared Memory noch zeigen wollte, verfällt beim ersten Puffer; die Wiederholungen nach dem Mappen (WSLg) und `Expose` laufen unter den Puffern nicht übers Shared Memory.

Der Preis: das Bild steht ein bis drei Millisekunden später am Fenster als mit einer Fence. Mehr als die GPU-Zeit und die letzte Millisekunde
des Nachschauens ist es nicht (der Compositor müsste ohnehin warten). Geht ein Bild unterwegs, schaut die Fensterschleife jede Millisekunde nach; ein
`sync_file` im `poll` der Schleife spart diese Millisekunde, ist aber nicht gebaut.

**Takt.** Wie beim Shared Memory ein Commit je Antwort des Bildschirms. Solange die fehlt, gibt `try_acquire` `NotReady`, das Bild bleibt in der
Textur, sein neuester Stand wird gezeichnet, wenn die Antwort kommt: es staut sich nichts, die GPU zeichnet keine Bilder, die niemand sieht, also
was MAILBOX für ein Fenster bedeutet (`present_mode()` sagt `Mailbox`). Die Antwort beendet das Warten der Schleife (`wait` kehrt zurück, kein Event, kein neues Bild der UI);
vorher schaut nichts nach. Nur das erste Bild nach dem Anlegen wartet nicht: bei einem Fenster, das sich bewegt, ist immer eine Antwort auf das letzte CPU-Bild unterwegs.

**Damage.** Der Compositor bekommt die Rechtecke, die die Laufzeit ohnehin kennt (`Swapchain::damage`), außer bei Bildern mit Ebenen, Bildern und Leinwänden und dem Bild danach: dann das Ganze.

**Threads.** Der Fenster-Code darf nur vom Thread der Fensterschleife benutzt werden. Der Arbeitsthread baut also nur Vulkan-Objekte: die Größe kommt über `Surface::track_size`, das Anmelden
der Puffer (`prepare`) und das Freigeben alter (`RETIRED`) geschieht auf dem UI-Thread. Stapel des Arbeitsthreads: 8 MiB wie die Vorgabe von glibc, nur belegt, was benutzt wird; anv kommt
ohne Shader-Cache mit 128 KiB aus (`jfgui_GPU_WORKER_KB`, je ein Lauf mit 128, 256, 512, 1024, 2048 KiB), andere Treiber sind nicht gemessen.

**Rückfälle.** Kein Vulkan, kein passender Adapter (CPU-Adapter nie: `adapters_low_power`, `can_present`; ein anderes Gerät als das des Compositors auch nicht), kein dmabuf im Compositor, kein DRI3 oder Present, kein gemeinsames Format, ein
abgelehnter Puffer: CPU-Pfad für den ganzen Lauf, ohne Meldung. Ein statisches Programm hat das alles nicht (`gpu_out_off.rs`).

**Was man zurücklesen kann.** `jfgui_GPU_VERIFY=1` liest jedes gezeigte Bild aus der GPU zurück. Ein Wayland-Client kann sein Fenster nicht lesen: `gpu expect screen` und die Prüfungen von `uigputest` sind dort Hinweise.
Unter X11 liest `GetImage`, was der Server hat; bei einem geflippten Fenster unter Xwayland ist das das Bild davor (gesehen mit KWin 6.6, die Komposition zeigt das letzte), deshalb zeigt `gpu expect screen` dort einmal mehr dasselbe.
Der echte Beweis war einmalig von Hand: ein eigener KWin ohne Bildschirm (`kwin_wayland --virtual --xwayland`, eigener Sitzungsbus, `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1`, unsichtbar für jeden), `spectacle -b -n -f` darin, Ausschnitt mit `shot` verglichen: 0 Pixel anders, mit offenem Menü, nach dem Schließen, nach Teil-Updates, auf Wayland und X11, auf Wayland auch bei Maßstab 1,5 (Viewport).

**Messungen.** Intel HD Graphics 610 (KBL GT1), Mesa 26.0.3, Pentium G4560T, Fedora 44, KWin 6.6; Release-Build, Fenster 1280 × 800. Wenn nicht anders gesagt auf dem eigenen KWin ohne Bildschirm (virtuelle Ausgabe, 60 Hz, echte GPU, Wayland);
drei Läufe je Variante, gezeigt die Spanne der Läufe. Der Rechner ist geteilt (Last um 1). `uigputest --bench stages|latency|idle|caret`.

| Messung | CPU-Pfad | GPU-Pfad | wie |
| --- | ---: | ---: | --- |
| Prozessstart bis Prozessende über 300 ms (gallery `--auto-close 300`) | statisch 22–28 ms, glibc 19–35 ms | 28–42 ms | von außen; streut stark |
| Erstes Bild → Swapchain | – | 74–78 ms; Instanz 40–45, Adapter 6–7, Gerät 2,5–3,4, Bilder + Textur 1, Pipeline 1–1,6 (ohne Shader-Cache 15–27); der Wechsel auf dem UI-Thread 5,6–6,4 ms | `jfgui_GPU_TRACE=1` |
| Taste → Commit, kleiner Fleck, Median | 0,13–0,14 ms | 2,8–3,0 ms | wp_presentation, 120 Tasten je Fenster |
| Taste → Bildschirm, kleiner Fleck, Median (p95) | 9,2–9,5 ms (17) | 12,5–13,3 ms (20) | ebenso |
| Taste → Commit, ganzes Bild, Median | 1,6–2,0 ms | 6,1–6,3 ms | ebenso |
| Taste → Bildschirm, ganzes Bild, Median (p95) | 11,8–13,2 ms (20) | 14,3–15,2 ms (23) | ebenso |
| **Echter KWin, Taste → Bildschirm**, kleiner Fleck | 16,6 / 24,7 / 17,6 ms | 24,6 / 18,9 / 24,8 ms | 3 Läufe; zweigipflig, in beiden Pfaden: kein Unterschied zu sehen |
| **Echter KWin**, ganzes Bild | 26,4 ms | 26,0 ms | 1 Lauf |
| Leerlauf: CPU, Kontextwechsel, Threads | 0 ms, 0 in 10 s, 3 Threads | 0 ms, 0 in 10 s, 5 Threads | von außen, `/proc` (Schritte von 10 ms) |
| Blinkender Cursor: CPU, Kontextwechsel | 0–10 ms, 40–61 in 10 s | 10 ms, 84–102 in 10 s | ebenso |
| Speicher im Leerlauf: RSS / anonym | 13,1 / 4,4 MiB | 54,0 / 10,5 MiB | das meiste davon sind Seiten des Treibers |
| Speicher nach den Szenen: RSS / anonym | 21,4 / 4,6 MiB | 55,4 / 11,9 MiB | ebenso |
| GPU-Speicher des Prozesses (fdinfo `drm-total`) | 0 | Leerlauf 24,5 MiB, nach den Szenen 49–53 MiB; `memory_reserved()` 15–21 MiB | |
| Ganze Seite, 60 Bilder/s, je Bild: CPU des Programms | 6,1–8,8 ms | 6,1–6,7 ms | `--bench stages`, 120 Bilder |
| … GPU des Programms / CPU des Compositors | 0 / 1,4–1,75 ms | 2,64 ms / 0,58–0,67 ms | KWins GPU-Zeit nicht lesbar (Datei-Capability) |
| Kleiner Fleck: CPU / GPU / Compositor | 1,03–1,12 / 0 / 0,83–0,92 ms | 1,05–1,47 / 1,71 / 0,67–0,83 ms | das ganze Fenster wird gezeichnet |
| Popup auf, zu: CPU / GPU / Compositor | 3,2–3,8 / 0 / 2,0–2,1 ms | 2,3–3,2 / 1,83 / 0,58–0,75 ms | |
| Leinwand verschieben: CPU / GPU / Compositor | 1,19–1,49 / 0 / 1,17–1,42 ms | 0,68–0,95 / 1,96 / 0,58–0,92 ms | einfacher Inhalt; teure Inhalte gewinnen mehr |
| Größe gallery, Release | statisch 4 472 664 Byte | glibc mit GPU 4 787 544 Byte (+314 880) | |

Unter X11 (Xwayland desselben KWin, je 3 Läufe) ist es dasselbe Bild: ganze Seite CPU 6,1–8,2 gegen 6,1–6,3 ms, kleiner Fleck 0,91–1,18 gegen 1,26–1,48 ms, GPU 2,64 und 1,71 ms; RSS 17–21 gegen 55 MiB.

Was das heißt: Eine schwache GPU reicht, 1,7 bis 2,6 ms je Bild bei diesem Fenster. Die CPU des Programms ist gleich oder kleiner, die des Compositors kleiner (er muss nichts kopieren). Im Leerlauf kostet der Weg nichts. Der Preis ist
Speicher, rund 40 MiB RSS (meist geteilte Seiten des Treibers) und 25 bis 50 MiB im Treiber, und der Commit kommt um die GPU-Zeit später; auf dem Bildschirm bleibt davon wenig. Empfehlung: an, wie unter Windows (`DEFAULT_ON_LINUX` in `gpu_out.rs`, eine Zeile), solange kein Treiber anderes zeigt.

**Gegenprobe auf einem Laptop mit zwei GPUs** (ThinkPad P14s Gen 2i, Ubuntu 26.04, Intel Iris Xe mit Mesa 26.0.8, NVIDIA T500 mit dem proprietären Treiber 595.99). Der echte Desktop läuft auf der Intel-GPU, und die UI wählt sie von selbst, unter Wayland und X11 (`uigputest --test switch`: 23 gezeigte Bilder zurückgelesen, keins anders). Eine erzwungene NVIDIA (`jfgui_GPU_ADAPTER=NVIDIA`) zeigt dort nichts: Die Bilder laufen nur auf der GPU, auf der der Compositor läuft, das Fenster bleibt auf dem CPU-Pfad. Bilder von einer GPU zur anderen (PRIME) sind nicht gebaut. Auf einem eigenen KWin ohne Bildschirm, der selbst auf der NVIDIA lief, bestehen `gputest --test all`, die GPU-Tests von `uigputest` und die Skripte `gpu_output`, `gpu_layers` und `gpu_layers_rtl` der Galerie, unter Wayland und X11: Der proprietäre Treiber kommt mit dmabuf zurecht. Ob das Auflisten der Adapter eine schlafende zweite GPU weckt, bleibt offen: Die T500 schläft auf diesem Rechner nie.

| Messszene, 1080p, Entwicklungs-Build, je drei Läufe | 10 000 Würfel | 100 000 | 1 Mio. |
| --- | ---: | ---: | ---: |
| Intel Iris Xe (TGL GT2) | 0,53 ms | 2,48 ms | 19,4 ms |
| NVIDIA T500 | 0,68 ms | 1,70 ms | 8,4 ms |

**Gegenprobe auf Mutter** (50.1, derselbe Laptop, ein eigener Mutter ohne Bildschirm, `mutter --headless --wayland --virtual-monitor 1920x1080`; er läuft auf der Iris Xe und bietet keine Rahmen vom Server): Die Tests von `uigputest`, `gputest --test window` und die drei Skripte der Galerie bestehen unter Wayland und über Xwayland, 0 zurückgelesene Bilder anders. Der Lauf fand einen Fehler unter X11: `WCMD_RESTORE` schickte nur `MapWindow`, und das bewirkt bei einem Fenstermanager nichts, der minimierte Fenster abgebildet lässt. Jetzt geht dazu `_NET_ACTIVE_WINDOW` hinaus.

**Nicht getestet:** AMD, die ganze GNOME-Oberfläche und andere Compositors, X11 ohne Xwayland, eine zweite GPU, die schläft, ein echter Treiber-Reset, die Validierungsschicht (auf keinem der Linux-Rechner installiert), `Expose` unter den Puffern (trat nicht auf), Vulkan 1.1.

### Ausgabe unter Android

Derselbe Presenter wie unter Windows und Linux (`lib/ui/gpu_out.rs`, G4c), mit denselben Stufen, Rückfällen, Ebenen, Bildern und Leinwänden; `cfg` nur beim Laden von `libvulkan.so` und beim Anlegen der Fläche. Die Oberfläche rastert weiter auf der CPU, Windows- und Linux-Verhalten bleibt, wie es war. Die Plattform-Seite: [`lib/plat/android_gpu.rs`](../lib/plat/android_gpu.rs), [`android/GalleryActivity.java`](../android/GalleryActivity.java), [`android/bridge.c`](../android/bridge.c).

**`lib/gpu` auf Android.** `libvulkan.so` wird mit `dlopen` geladen, nicht gelinkt: Ein `DT_NEEDED` beendete auf einem Gerät ohne die Bibliothek das ganze Programm beim Laden, so gibt es `NoLoader`. Android 8 hat nur den Loader 1.0, das Programm sieht das vor jedem Treiberaufruf (`a Vulkan 1.1 loader`). Instanz, Adapter und Gerät wie überall, Stufe Basis, wo nur das da ist; `VK_KHR_android_surface` und ein echter `VkSwapchainKHR` ([`vk_swap.rs`](../lib/gpu/vk_swap.rs)).

**Zwei Wege in einem Fenster.** Die View mit ihrem Bitmap (`GalleryView`) bleibt der CPU-Pfad und nimmt alle Eingaben, die Tastatur und die Barrierefreiheit. Die Swapchain liegt auf einem `SurfaceView` darüber, dem Gegenstück zum Kindfenster unter Windows:

- `setZOrderOnTop(true)`: über dem Inhalt des Fensters. Eine Fläche ohne Puffer zeigt der Compositor nicht: Bis zum ersten `present` sieht man die View darunter, dieselben Pixel, ohne Lücke und ohne Schwarz (unter dem Fenster läge eine schwarze Hintergrundebene, und die View müsste durchsichtig werden, bevor man weiß, ob der Puffer angekommen ist).
- Im Baum liegt der `SurfaceView` **unter** der View, also bekommt die View die Finger; für die Barrierefreiheit ist er unwichtig.
- Deckend (`RGBX_8888`): Der Compositor verdeckt das Fenster, statt zu mischen.
- Die Fläche existiert, solange der `SurfaceView` sichtbar ist. Zurück zum CPU-Pfad heißt deshalb: das Bild ins Bitmap, `INVISIBLE`, Fläche und Swapchain gehen mit. Die Swapchain wird unter Android nie neu gebaut, sondern neu angelegt.

| Stufe | Was geschieht |
| --- | --- |
| CPU | `plat::win_present`, Kopie ins Bitmap (`jfgui_host_present`), `invalidate`, HWUI zeichnet. So geht das erste Bild jedes Laufs; der Worker startet danach |
| Job | Instanz, Gerät, Pipeline, Swapchain auf einem Thread (`plat::gpu_thread_spawn`, ein echter pthread; `plat::thread_spawn` bleibt hier `None`, Pools und Tasks dieses Ziels sind kooperativ). Der Thread endet danach. Xclipse 540: Instanz 7–10 ms, Adapter 0,5–1, Gerät 3–10, Swapchain und Textur 17–37, Pipeline 31–63 |
| Wechsel | das ganze Bild in die Textur, ein `present`, die Fläche zeigt sich von selbst; die Kopie von `Window.pixels` geht (`win_release_kept`) |
| GPU | wie überall. `nativeRender` gibt 2 zurück, das Bitmap ruht, kein `invalidate`. Hat HWUI nichts zu tun, wacht sein Thread nicht auf |
| zurück | das ganze Bild ins Bitmap (`gp_leave`, dann `flush` in demselben `jfgui_tick`), `INVISIBLE` per `post` (`setVisibility` ruft `surfaceDestroyed` sofort, während Rust die App hält) |

**Lebenszyklus.**

| Ereignis | Was geschieht |
| --- | --- |
| Pause ohne Stopp (ein Dialog, ein zweites Fenster im Vordergrund) | die Fläche bleibt, keine Bilder, nichts wird gebaut |
| Stopp (`Home`, eine andere App): `surfaceDestroyed` | noch im Callback: das Bild ins Bitmap (das Fenster kann vor dem nächsten Bild wieder zu sehen sein), Swapchain weg (`gp_surface_gone`, wartet auf die Warteschlange). Der `SurfaceView` bleibt sichtbar: Die Fläche kommt beim Zurückkehren von selbst, Stufe Gpu rund 200 ms später |
| Ein Job läuft, während die Fläche geht | `ANativeWindow_acquire` hält das Fenster, `Win::lost` wirft das Ergebnis weg; der Treiber bekommt Fehler, keinen hängenden Zeiger |
| Größe (Tastatur, Drehen, mehrere Fenster, Einklappen) | jedes Bild der Änderung ein CPU-Bild, 100 ms nach dem letzten neue Fläche, neue Swapchain: auf dem Telefon rund 200 ms bis zur Stufe Gpu. Bis der `SurfaceView` weg ist, kann die alte Fläche ein Bild lang gestreckt über dem neuen Bitmap liegen (auf API 26–30 ohne BLAST umgekehrt das alte Bitmap zu sehen sein); nicht einzeln gemessen |
| Drehen | das Bild bleibt aufrecht (`VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR`), der Compositor dreht. `SUBOPTIMAL` bei jedem `present` heißt dann nur das und führt zu nichts; eine neue Größe sieht `window_size` (Android meldet dafür nie `OUT_OF_DATE`). Preis unten |
| Werkzeugfenster, Popups aus einem Fenster | werden auf der Leinwand der Plattform zusammengesetzt, also unter der Fläche: solange eins zu sehen ist, ist Fenster 0 auf dem CPU-Pfad (`win_gpu_child_alone`). Danach zurück |
| Gerät verloren | wie überall: alles zurück, neues Gerät nach 2 s, 4 s, … (auf dem Xclipse in 20 ms gebaut) |

**Rückfälle.** Eine App darf Vulkan nicht im Manifest verlangen (`tools/android.py` bricht dann ab, `android_gpu.py` prüft die APK). Ohne Vulkan 1.1 in den Features des Systems (`FEATURE_VULKAN_HARDWARE_VERSION`) wird der Treiber gar nicht erst geladen. Kein Loader, kein brauchbarer Adapter (auch: Vulkan 1.2 ohne dynamisches Rendern, so `-gpu swiftshader_indirect` im Emulator): CPU-Pfad für den ganzen Lauf, ohne Meldung, mit der Auflösung, die der CPU-Pfad hat.

**Ein kaputter Treiber.** Absturz im Treiber lässt sich nicht abfangen. Deshalb steht in `files/gpu-state`, was das System gezeigt hat (`Build.FINGERPRINT`, dann `ok`, `never` oder `trying N`): vor dem ersten Treiberaufruf `trying N+1`, nach dem ersten gezeigten Bild `ok`. Zwei Läufe, die in dieser Spanne enden, und der dritte bleibt auf dem CPU-Pfad; `never` heißt: Dieser Stand des Systems kann es nicht, der Treiber wird nicht mehr angefasst. Ein neues System versucht es neu. Also höchstens zwei Abstürze statt eines kaputten Programms. Geprüft mit `jfgui_GPU_TEST_DIE=1`.

**Was die Auflösung begrenzte.** `GalleryView.onSizeChanged` hatte zwei Grenzen: höchstens 2 Pixel je DIP (`2 / density`) und 2048 Pixel auf der längeren Seite; auf dem A56 (450 dpi) zählt die erste: 768 × 1496 bei 192 dpi statt 1080 × 2104 bei 270 dpi, HWUI skaliert hoch. Dazu 4096 Pixel in `bridge.c` für das Bitmap. **Durch die GPU gelten nur noch die 4096** (`scale()`, auch ein Bild auf dem CPU-Pfad wird währenddessen so groß); der CPU-Pfad behält beide, die Messung unten sagt warum. Die Auflösung steht vor dem ersten Bild fest und wechselt nur, wenn die GPU sich als unbrauchbar herausstellt (`GPU_STATE_NEVER`: neues Bitmap, einmal neu gelegt); über das Ende des Laufs hinaus sagt das `gpu-state`. `jfgui_GPU_RES=cap` gibt der GPU dieselbe Begrenzung wie dem CPU-Pfad (`setFixedSize`, der Compositor skaliert).

**Die Vorgabe** steht in zwei Zeilen in `GalleryActivity`: `GPU_DEFAULT` und `GPU_FULL_RESOLUTION`. Die Java-Seite muss es vor dem ersten Bild wissen und setzt `jfgui_GPU` immer selbst.

**Messungen.** Samsung SM-A566B (Exynos 1580, Xclipse 540, Treiber 24.0.560, Vulkan 1.3.279, Stufe Voll), Android 16, 120 Hz, View 1080 × 2104; Release-Build; je **drei Läufe**, gezeigt die Spanne der Läufe, Reihenfolge wechselnd. **Android Studios Spiegelung lief die ganze Zeit** (`studio.screen.sharing`, ein eigener Bildschirm, dessen Komposition auf der GPU läuft): Alles, was auf dem Bildschirm endet, ist davon betroffen, in allen Spalten; die Werte des UI-Threads kaum. Die Akkulaufzeit lässt sich hier nicht messen. Werkzeug: `am instrument -e phase bench` (`GalleryGpuInstrumentation`). „Bis zum Bildschirm“ kommt aus `dumpsys SurfaceFlinger --frametimeline` (`--latency` antwortet unter Android 16 nicht), an die Uhr gebunden über das Token eines Bildes in `gfxinfo framestats`.

| Messung | CPU, begrenzt (bisher) | CPU, volle Auflösung | GPU, begrenzt | GPU, volle Auflösung |
| --- | ---: | ---: | ---: | ---: |
| Start (`am start -W`, ms) | 291–341 | 314–337 | 297–335 | 293–316 |
| Start der Ausgabe → Swapchain (ms) | – | – | 91–92 | 113–127 |
| Leerlauf 10 s: Bilder | 0 | 0 | 0 | 0 |
| Leerlauf 10 s: CPU (ms) | 120–160 | 140–170 | 160–180 | 160–170 |
| Leerlauf 10 s: Kontextwechsel gesamt / UI-Thread | 88–101 / 85–87 | 91–96 / 80–81 | 88–96 / 79–85 | 85–89 / 79–83 |
| Threads | 26–27 | 26–27 | 27–28 | 27–28 |
| Speicher, PSS (MiB) | 84–104 | 113–127 | 133–139 | 199–203 |
| davon Graphics (MiB) | 37–43 | 52–64 | 66–72 | 119 |
| `memory_reserved()` im Leerlauf / nach einer Leinwand (MiB) | 0 | 0 | 4,4 / 23–26 | 8,8 / 34 |
| ganzes Bild: UI-Thread, Median (ms) | 17,5–17,8 | 24,1–25,6 | 19,1–23,9 | 26,2–26,4 |
| ganzes Bild: bis zum Bildschirm, Median (ms) | 38–39 | 47–49 | 32–36 | 39–41 |
| kleine Änderung (Checkbox): UI-Thread, Median (ms) | 0,9–1,3 | 0,7–1,6 | 9,0–9,2 | 9,0–9,5 |
| kleine Änderung: bis zum Bildschirm, Median (p95) (ms) | 45–48 (58–75) | 45–50 (57–72) | 36–38 (49–58) | 38–41 (49–50) |
| Finger rollt: UI-Thread, Median (p95) (ms) | 5,7–5,9 (8,1–9,8) | 8,9–10,8 (12–16) | 5,3–9,3 (9,8–13) | 7,7–8,1 (11–13) |
| Finger rollt: je Bild (ms) | 9,6–9,7 | 13,3–17,9 | 9,3–15,8 | 9,8–10,2 |
| Finger rollt: gezeigt von 120, verspätet | 120, 0–2 | 120, 3–17 | 120, 0–2 | 120, 0–2 |
| Leinwand ziehen: UI-Thread, Median (p95) (ms) | 3,2–3,3 (5,3–6,7) | 6,6–7,2 (8,9–11) | 1,5–1,6 (2,9–4,4) | 1,7–1,8 (2,9–3,4) |
| Leinwand ziehen: je Bild (ms) | 9,1–9,7 | 13,1–14,8 | 9,2–9,4 | 9,2–9,4 |
| Leinwand ziehen: verspätet von 120 | 3–7 | 51–77 | 4–7 | 0–3 |

Die Spalte „GPU, begrenzt“ rollte in einem Lauf mit 120 Hz und in zweien mit 60 Hz (Samsungs Bildrate, eine Stimme des Compositors, Ursache nicht geklärt); das ist die Streuung, kein Unterschied in der Arbeit. Ein Lauf quer (nur je einer): Der Compositor dreht die aufrechte Fläche nicht in der Hardware (`dumpsys SurfaceFlinger`: `CLIENT`, `ROT_90`), er komponiert auf der GPU. GPU, volle Auflösung, quer: ganzes Bild 45,6 ms bis zum Bildschirm, gezeigt beim Rollen 98 von 120; CPU, begrenzt, quer: 40,1 ms, 118 von 120.

Was das heißt:

- **Der CPU-Pfad behält seine Grenze.** Volle Auflösung über das Bitmap verliert fast alles: Rollen im Schnitt 13–18 ms je Bild statt 9,7, die Leinwand 51–77 verspätete von 120, denn jede Änderung lädt in HWUI das ganze Bitmap neu hoch (der Kommentar in `bridge.c`).
- **Die GPU macht Eingaben nicht langsamer**, kleine Änderungen sogar rund 8 ms früher auf dem Bildschirm (keine Runde über View und `RenderThread`), obwohl der UI-Thread dafür 9 ms statt 1 ms zahlt: 8 ms davon stecken in `gp_present` (Hochladen, `acquire`, Übergabe, `present`, nach einer Ruhepause; die Ursache im Treiber ist nicht gesucht). Solange die Bilder dicht folgen, verschwindet das: gleiche Rollzeiten bei gleicher Auflösung und 120 Hz (bei 60 Hz, in zwei Läufen, kam es zurück: 9 ms). Die Leinwand kostet die Hälfte.
- **Volle Auflösung kostet CPU, wie vorhergesagt, aber sie reicht auf diesem Telefon.** Die CPU rastert doppelt so viele Pixel: Ein ganzes Bild 26 statt 18 ms auf dem UI-Thread, Rollen 7,7–8,1 statt 5,7–5,9 ms. 120 Hz hält es noch, mit wenig Luft (8,3 ms Budget, p95 11–13 ms; verspätet wie vorher 0–2 von 120). Ein schwächeres Telefon rollt mit voller Auflösung langsamer als jetzt: Das ist nicht gemessen und eine echte Frage.
- **Der Preis ist Speicher.** Volle Auflösung durch die GPU braucht 199–203 MiB PSS statt 84–104, davon 119 statt 37–43 MiB Graphics. Darin stecken, aus den Größen geschätzt und nicht einzeln gemessen: die drei Puffer des Fensters, das nichts mehr zeigt (30 MiB), das ruhende Bitmap samt HWUIs Textur (18 MiB), Framebuffer, Textur, mindestens drei Bilder der Swapchain. Mit Begrenzung sind es 133–139 MiB.
- **Im Leerlauf** kostet es nichts Messbares: 0 Bilder, gleiche Kontextwechsel. Die etwa 8 Wechsel je Sekunde auf dem UI-Thread sind der alte Blick alle 250 ms (`jfgui_frame_delay`) in allen Spalten; kein Thread von uns (der eine zusätzliche gehört dem Treiber).
- **Empfehlung: GPU an, volle Auflösung an** (`GPU_DEFAULT`, `GPU_FULL_RESOLUTION`). Das Bild ist schärfer (270 statt 192 dpi), Eingaben kommen früher an, das Rollen hält 120 Hz. Wer den Speicher scheut oder schwache Telefone zuerst sieht: `GPU_FULL_RESOLUTION = false` kostet 65 MiB weniger und rollt, wie es heute rollt.

| Größe, arm64-v8a, Release (`jetzt` mit der neuen Seite der Galerie) | vor G4c | jetzt |
| --- | ---: | ---: |
| APK (Bytes) | 3 845 324 | 4 103 372 |
| `libjfgui_android.so` (Bytes) | 3 631 888 | 3 873 080 |

**Auf dem Telefon gefunden:** `gputest --test indirect` schlug auf dem Xclipse fehl, die Dreiecke fehlten. Der Rechen-Shader, der die Argumente schrieb, las `w[i]` mit veränderlichem `i` aus einem Array in Push-Konstanten, und der Treiber (24.0.560) liefert dafür immer `w[0]`, auf gültigem SPIR-V; der Shader schreibt jetzt ein Wort je Aufruf. `lib/wgsl` und `gpu_out.wgsl` indizieren so nichts; ein WGSL-Programm könnte es, das Back-End weiß davon nichts. Und zwei Sperren im Presenter, die nur Windows und Linux nannten (`dirty.rs`): Ebenen wären still in der Grundtextur gelandet; `android_gpu.py` hat sie gefunden.

**Tests.** `apps/androidGallery/tests/android_gpu.py` (Emulator oder Telefon, `--package` neben einer installierten Galerie; `GalleryGpuInstrumentation`, die Seite „Bild & Leinwand“) und `apps/gputest/tests/android.py` (`gputest` ist hier ein Programm für `adb shell`, ohne Fläche: Was ein Fenster braucht, ist `SKIP`, die Swapchain testet `android_gpu.py`). Beide laufen in `build_rs.py test`, wenn `jfgui_TEST_ANDROID=SERIAL` gesetzt ist, und ersetzen dort eine installierte Galerie (ihre Daten bleiben bis auf `files/gpu-state`); sonst nennt die Suite sie nur. Ohne Gerät `SKIP`. Der Emulator bietet Vulkan nur mit `-gpu host` (Vulkan 1.3 über gfxstream, Adapter der RTX 2070); `-gpu swiftshader_indirect` hat Vulkan 1.2 ohne dynamisches Rendern, also den Rückfall, und auch das ist ein Testlauf wert.

**Nicht getestet:** ein Telefon mit Vulkan 1.1 oder Android 8 und 9, Adreno und Mali, ein echter Treiber-Reset (`test_lose` spielt ihn vor), ein Treiber, der wirklich abstürzt (das Beenden ist nachgestellt), die Pause ohne Stopp für sich, der geteilte Bildschirm und Faltgeräte (der Weg über die neue Größe ist geprüft: Tastatur und Drehen auf dem Telefon, ein freies Fenster mit `am task resize` im Emulator), Bildschirmleser mit echtem TalkBack auf dem GPU-Pfad, ein Lauf ohne die Spiegelung von Android Studio, die Akkulaufzeit.

**Nicht gebaut:** Vorab-Drehen der Bilder (`preTransform`, die Rechtecke und der Shader gedreht; das nähme dem Compositor die GPU-Komposition quer ab), Freigeben des Bitmaps auf dem GPU-Pfad (18 MiB, kostet einen Rückweg, der es erst wieder anlegt), eine Grenze für schwache Telefone (`isLowRamDevice`), GPU-Komposition für Werkzeugfenster.

### Ausgabe im Browser

Derselbe Presenter wie überall (`lib/ui/gpu_out.rs`, G4d), mit denselben Stufen, Rückfällen, Ebenen, Bildern und Leinwänden; darunter liegt WebGPU statt Vulkan. Die Oberfläche rastert weiter auf der CPU, im selben Worker wie bisher. Die Seite des Browsers: [`web/gpu.js`](../web/gpu.js), [`lib/gpu/web_dev.rs`](../lib/gpu/web_dev.rs), [`web_res.rs`](../lib/gpu/web_res.rs), [`web_cmd.rs`](../lib/gpu/web_cmd.rs), [`web_swap.rs`](../lib/gpu/web_swap.rs), [`lib/plat/web_gpu.rs`](../lib/plat/web_gpu.rs).

**Wo das Programm läuft.** In allen drei Bündeln (`scalar`, `simd`, `threads`) läuft die App in einem Worker (`web/worker.js`), nicht auf der Seite. Die Seite (`web/loader.js`) nimmt Eingaben und malt, was der Worker schickt, mit `putImageData` in `#screen`. Im Bündel `threads` kommen Rechen-Worker dazu, dieselbe WASM-Datei im selben geteilten Speicher: Dort rastern Bänder, die UI betritt nur der App-Worker. `navigator.gpu` und `OffscreenCanvas` gibt es im Worker. WebGPU-Objekte gehören aber dem Thread, der sie angelegt hat, und lassen sich nicht weiterreichen. Also liegt alles auf dem App-Worker, es gibt in keinem Bündel einen Job-Thread für die GPU; die Typen des Backends halten `Rc`, der Compiler lässt sie nicht in die Rechen-Worker. Was „Job“ heißt, geschieht auf demselben Thread, sobald der Browser geantwortet hat. Warten geht dort nicht: Die Antwort des Browsers braucht die Ereignisschleife des Workers.

**Was die Bündel unterscheidet**, wenn es um die GPU geht, ist nur der Speicher: Im Bündel `threads` ist er ein `SharedArrayBuffer`. `queue.writeTexture` und `writeBuffer` nehmen ihn in Chrome 156 (eigens geprüft); wirft ein Browser dort `TypeError`, gehen die Bytes ab da über eine Kopie (`copyShared` in `gpu.js`). In Firefox 159 läuft das Bündel `threads` ebenfalls (ein Lauf, jedes Bild zurückgelesen); welchen der beiden Wege es nimmt, ist nicht festgehalten. `TextDecoder` nimmt ihn nicht, Texte werden vorher kopiert.

**`lib/gpu` im Browser.** Dieselben Typen und Methoden.

| Die API | im Browser |
| --- | --- |
| Instanz, Adapter, Gerät | `requestAdapter` und `requestDevice` antworten später: `Instance::new()` fragt, `prepare()` sagt, ob die Antwort da ist (anderswo sofort `true`), bis dahin `NotReady`. Ein Adapter, `low-power` erbeten; der Browser sagt nicht, ob integriert oder diskret; ein Software-Adapter (`isFallbackAdapter`) gilt als `Cpu` und fehlt in `adapters_low_power`. WebGPU gibt je Adapter ein Gerät: ein neues Gerät braucht eine neue `Instance` |
| Stufe | immer Basis. Bindless, `draw_indexed_indirect_multi`, `_count`: `Unsupported` bzw. `Invalid` |
| Shader | `ShaderModule::from_wgsl`: Der Browser übersetzt und prüft. `from_spirv` ist `Unsupported`. `entry_points()` und `constants()` liest ein kleiner Scanner aus dem Text (`scan` in `web_res.rs`), er urteilt über nichts |
| Push-Konstanten | ein Uniform-Puffer hinter einem dynamischen Offset, Gruppe 3 jeder Pipeline mit Push-Konstanten; `var<immediate> x: T;` wird beim Anlegen zu `@group(3) @binding(0) var<uniform> x: T;`. Also ein Block mit Uniform-Layout und höchstens drei eigene Gruppen (vier und Push-Konstanten: `Unsupported`). Je Draw mit neuen Werten ein Platz von 256 Byte (`minUniformBufferOffsetAlignment`), der Ring wächst von selbst. Chrome 156 nimmt `var<immediate>` schon selbst an; das ist hier nicht benutzt, die anderen Browser zählen |
| Pipeline-Konstanten | `constants: Record<string, number>`, Schlüssel die Nummer als Text, ab 0x10000 der Name. Fremde und doppelte Nummern `Invalid` wie überall; Wertebereiche prüft der Browser, später |
| Kopien | `bytes_per_row` muss ein Vielfaches von 256 sein (außer bei einer einzigen Zeile), sonst `Invalid` aus `finish()`. `write_texture_rects` kennt die Regel nicht: `queue.writeTexture` nimmt jede Zeilenlänge und liest direkt aus dem WASM-Speicher, ohne Staging-Ring |
| Bindungen | WebGPU will mehr wissen, als `Binding` sagt (Dimension und Art einer Textur, Format einer Speichertextur, Art eines Samplers). Das Layout entsteht deshalb spät: aus den Ressourcen der ersten Bindungsgruppe oder aus dem WGSL der ersten Pipeline |
| Barrieren, Layouts | wirkungslos, WebGPUs Regeln tun das; die Aufrufe bleiben für denselben Code |
| Aufzeichnen | Wörter in einem `Vec<u32>`, erst `Queue::submit` macht in einem einzigen Aufruf nach JavaScript daraus einen `GPUCommandEncoder` (`submit` in `gpu.js`) |
| Warten | geht nicht. `wait(id, _)` antwortet sofort (`false`: noch nicht), `wait_idle` und `Queue::read_buffer` sind `Unsupported`. `Buffer::read` eines Readback-Puffers sagt `NotReady`, bis die Übergabe fertig und der Puffer abgebildet ist; `is_done` fragt |
| Puffer | Upload: `write` ist `queue.writeBuffer`. Readback: nur Kopien dürfen hinein (`MAP_READ` geht nur mit `COPY_DST`), eine andere Verwendung ist `Unsupported` |
| Fehler | Was hier zu erkennen ist, ist sofort `Invalid`. Was der Browser findet, kommt später als `uncapturederror` und wird von `submit`, `try_acquire`, `poll` zurückgegeben: Validierung `Invalid`, Speicher `OutOfMemory` |
| Swapchain | `getContext("webgpu")` auf einem `OffscreenCanvas`: das bevorzugte Format, deckend, sRGB. Darstellungsmodus und Bilderzahl gehören dem Browser, `present_mode()` sagt `Mailbox`. `try_acquire` ist `getCurrentTexture`, nie `NotReady`. Ein `present` gibt es nicht: Das Bild geht los, wenn der Worker zur Ereignisschleife zurückkehrt, ohne die Seite. Deshalb zeichnet `gp_tick` je Runde des Workers einmal |
| Zeitstempel, Subgroup-Größe | nicht gebaut (`Features::timestamps` ist `false`, WebGPU kennt sie nur am Anfang und Ende eines Durchgangs) |

**Ein WGSL-Prüfer im Bündel?** Nein, der des Browsers reicht: Er urteilt ohnehin, und unser Urteil wäre nur eine zweite Meinung über Texte, die im Programm stehen. `lib/wgsl` kostete im Bündel `simd` 576 386 Byte mehr (+15 %; gzip +203 480) nur für `wgsl::check`, mit dem Weg nach SPIR-V 800 743 Byte. Einmal gemessen, Wegwerf-Code. Es ist eine eigene Frage, sobald Programme zur Laufzeit fremdes WGSL laden; heute steht jede Zeile WGSL im Programm und wird von `uigputest --test shaders` gegen `lib/wgsl` geprüft.

**Zwei Leinwände.** `#screen` bleibt der CPU-Pfad und nimmt alle Eingaben. Die Swapchain liegt auf einer zweiten Leinwand `#gpu` darüber (`pointer-events: none`), als `OffscreenCanvas` im Worker, das Gegenstück zum `SurfaceView` unter Android. Eine Leinwand ohne Bild zeigt nichts: Bis ihr erstes Bild beim Compositor ist, sieht man `#screen` darunter, dieselben Pixel, ohne Nachricht und ohne Zeitabstimmung. Jede Zeit auf der GPU hat eine neue Leinwand (`transferControlToOffscreen` geht einmal, und so kann keine ein altes Bild zeigen). `#input` und der Baum der Barrierefreiheit liegen darüber wie bisher.

| Stufe | Was geschieht |
| --- | --- |
| CPU | wie bisher: Worker kopiert, `postMessage`, `putImageData`. So geht das erste Bild jedes Laufs |
| Job | `Instance::new` (ohne `navigator.gpu`: Schluss, nichts am DOM geändert), `gpu.js` wird geladen, `requestAdapter` und `requestDevice` laufen; das Programm läuft weiter über den CPU-Pfad. Hier 250–420 ms nach dem ersten Bild, der größte Teil `requestAdapter` (in einem Lauf für sich: 336 ms, `requestDevice` 55 ms). Dann die Leinwand von der Seite (`REQUEST_GPU_CANVAS`), dann Pipelines und Swapchain auf diesem Thread |
| kleines Bild | 8 × 8 Pixel durch alle drei Pipelines und die Push-Konstanten, zurückgelesen und verglichen (`probe_start`). Ein Browser, dessen WebGPU anders zeichnet, zeigt nie ein Bild |
| Wechsel | das ganze Bild in die Textur, ein Bild; die Kopie für `#screen` und die Pixelpuffer des Workers gehen. `#screen` ruht mit seinem letzten Bild |
| GPU | wie überall, eine Runde des Workers ein Bild. Im Leerlauf nichts |
| zurück | das ganze Bild an `#screen` (`gp_leave`), danach erst nimmt die Seite `#gpu` weg (`after_flush`): Bis dahin liegt das alte Bild darüber, nie eine Lücke |

Größe: wie überall jedes Bild über die CPU, 100 ms nach dem letzten eine neue Leinwand. Bis dahin streckt der Browser beide gleich. Werkzeugfenster und Popups aus einem Fenster setzt `web_windows.rs` auf `#screen` zusammen, also darunter: Solange eins zu sehen ist, ist Fenster 0 auf dem CPU-Pfad (`win_gpu_child_alone`), wie unter Android.

**Rückfälle.** Keins davon ändert etwas am Bild, alle sind in `gpu_web.mjs` geprüft.

| Was | Was geschieht |
| --- | --- |
| kein `navigator.gpu`, `gpu.js` nicht zu laden, kein Adapter, ein Software-Adapter | CPU-Pfad für den ganzen Lauf, ohne Meldung. Keine Leinwand |
| `requestDevice` scheitert | CPU-Pfad, neuer Versuch nach 2 s, 4 s, … |
| der Browser verwirft einen Shader, das kleine Bild stimmt nicht | CPU-Pfad für den ganzen Lauf; die leere Leinwand geht |
| Gerät verloren (`device.lost`, auch zwischen zwei Bildern: `gp_tick` schaut) | das Bild über `#screen`, die Leinwand weg, neue Instanz nach 2 s, 4 s, …, neue Leinwand. Chrome zeigt nach `device.destroy()` weiter das letzte Bild, also keine Lücke; ob das bei einem echten Treiber-Reset so ist, ist nicht geprüft |
| ein Aufruf wirft in `gpu.js` | gefangen (sonst stünde der Worker), gilt als interner Fehler des Geräts |
| die Leinwand bekommt nichts, obwohl alles richtig rechnet | der Wächter, unten: das Bild über `#screen`, die Leinwand weg, CPU-Pfad für den ganzen Lauf |

**Der Wächter.** Alle Prüfungen oben sitzen im Worker. Kommt bei der Leinwand trotzdem nichts an, scheint `#screen` mit seinem letzten Bild durch sie hindurch, und das Programm wirkt eingefroren, während alles richtig rechnet. Deshalb schaut die Seite nach jedem Wechsel auf eine neue Leinwand nach (`watchGpuCanvas` in [`web/loader.js`](../web/loader.js), ausgelöst von `REQUEST_GPU_SHOWN`): `drawImage` von `#gpu` in ein Raster von 32 × 18 Proben ohne Glättung.

- **Mehrere Farben:** in Ordnung, der Wächter ist fertig, für diese Leinwand schaut er nicht wieder.
- **Eine Farbe, und `#screen` zeigt dieselbe:** ebenfalls in Ordnung, das Programm zeigt gerade eine Farbe.
- **Eine Farbe, und `#screen` zeigt etwas anderes:** Er schaut alle 100 ms wieder. Bleibt das 1,5 s so, meldet die Seite es dem Worker (`gpu-blank`), und `gp_tick` geht für den ganzen Lauf auf den CPU-Pfad zurück (`Unsupported`, im Log `the canvas shows nothing`). Der nächste Seitenaufruf prüft von vorn, gespeichert wird nichts.
- **`drawImage` wirft:** Das zählt wie eine Farbe. Wo niemand für die Leinwand bürgen kann, bleibt der alte Weg.

Es heißt „eine Farbe“ und nicht „durchsichtig“, weil eine Leinwand ohne Bild sich in Chrome durchsichtig und in Firefox deckend schwarz liest. Die Zeit zählt über Animationsbilder, je Bild höchstens 100 ms: Ein minimiertes Fenster hat keine und bekommt kein Urteil, eine hängende Seite wird nicht der Leinwand angelastet.

| Der Wächter, je Fall ein Lauf (die leere Leinwand nachgestellt: Die Ausgabe zeichnet in eine Textur, die niemand zeigt) | Chrome 156 | Firefox 159 |
| --- | --- | --- |
| echtes Fenster, gesunde Leinwand | bleibt auf der GPU, einmal gelesen | ebenso |
| echtes Fenster, leere Leinwand | alter Weg | alter Weg |
| minimiertes Fenster, leere Leinwand | kein Blick und kein Urteil (letzter Bericht nach 6 s), nach dem Wiederherstellen alter Weg | nicht geprüft |

Kopflos in `gpu_web.mjs`, alle drei Bündel: Der alte Weg kommt nach 1,4–1,6 s, das Bild auf dem Schirm gleicht dann dem Framebuffer auf 0 Pixel genau, auch nach einem neuen Bild.

**Was er kostet.** Einmal lesen je Leinwand, also nach jeder neuen Größe wieder. In Chrome 1–3 ms, das erste Mal je Browser-Prozess 55–68 ms im Haupt-Thread (`getImageData`; kopflos wie im echten Fenster), kurz nach dem Start und einmalig; eine Umgehung ist nicht gesucht. In Firefox 5–15 ms. Nur bei einer verdächtigen Leinwand liest er öfter, dann alle 100 ms für 1,5 s.

**Was er nicht sieht.**

- **Den Bildschirm.** Er liest, was bei der Leinwand angekommen ist, nicht, was der Compositor daraus macht. Ein Treiber, der ein richtiges Bild schwarz anzeigt, bleibt unbemerkt; eine Seite kann den Bildschirm nicht lesen. Dagegen hilft nur `?gpu=0`.
- **Spätere Ausfälle.** Er schaut einmal je Leinwand. Bleiben danach die Bilder aus, ohne dass das Gerät verloren geht, merkt er nichts. Einen Grund dafür kennen wir nicht, ein Vergleich mit dem Framebuffer bei jeder Ruhepause wäre der nächste Schritt.
- **Falsche Bilder.** Ein buntes, aber falsches Bild besteht. Die Pipelines prüft das kleine Bild vor dem Wechsel, und `jfgui_GPU_VERIFY` vergleicht in den Tests jedes Bild.
- **Ein schwarzes Programm in Firefox.** Zeigt `#screen` schwarz, ist eine leere Leinwand dort nicht von diesem Bild zu unterscheiden.

Einen falschen Alarm gäbe es, wenn ein Programm genau beim Wechsel von mehreren Farben auf eine geht und 1,5 s dort bleibt. Dann läuft dieser Lauf über den alten Weg, sonst geschieht nichts.

**Die Vorgabe** steht in einer Zeile: `DEFAULT_ON_BROWSER` in `gpu_out.rs`. Telefone und Tablets bleiben beim alten Weg, bis eins getestet ist: `web/loader.js` setzt dort `jfgui_GPU=0` (`userAgentData.mobile`, `Android`, `iPhone`, `iPad`, `iPod` oder `Mobile` in der Kennung, ein iPad, das sich als Mac ausgibt). `?gpu=0` und `?gpu=1` in der Adresse überschreiben beides, `?gpuenv=jfgui_GPU_VERIFY=1,..` setzt Variablen `jfgui_GPU_*`. Läufe mit `?agent=1` bleiben ohne `?gpu=1` auf dem CPU-Pfad, wie Skripte: Die Browser-Tests lesen `#screen`.

**Messungen.** GeForce RTX 2070 SUPER, Ryzen 5 3600, Chrome 156 kopflos (mit der echten GPU: Adapter `nvidia turing`), eigenes Profil, Release-Build, Bündel `simd`, 1920 × 1000, je **drei Läufe** im Wechsel, gezeigt die Spanne der Läufe; parallel bauten andere Sitzungen. Werkzeug: [`tools/tests/web_gpu_bench.mjs`](../tools/tests/web_gpu_bench.mjs). „Worker“ ist die Zeit eines Stapels im App-Worker (Rastern, Kopieren, Hochladen, Aufzeichnen; Median), „Seite“ `TaskDuration` des Haupt-Threads je Schritt (CDP, enthält die kleinen `evaluate` des Werkzeugs), „verspätet“ ein Bild der Anzeige, das den letzten Stapel noch unfertig fand: neue Eingaben mussten warten. Das Rollen läuft mit dem Spiegel der Barrierefreiheit (dem Standard), der die Seite in beiden Spalten rund 10–15 ms je Schritt kostet. Ohne ihn (`?a11y=0`, drei Läufe, unter Last): Seite bisher 3,2–9,6 ms, GPU 1,7–2,1 ms; Worker bisher 2,0–5,9 ms, GPU 4,6–6,5 ms. „Verspätet“ steht aus einem zweiten Satz von drei Läufen mit berichtigtem Zähler; in diesem Satz bauten neun `rustc` anderer Sitzungen, die Streuung ist groß und je Weg gab es einen schlechten Lauf (Worker bis 88 ms bisher, bis 263 ms GPU).

| Messung | bisher (`?gpu=0`) | GPU (`?gpu=1`) |
| --- | ---: | ---: |
| Laden → erstes Bild (ms) | 307–661 | 313–435 |
| Laden → GPU zeigt (ms) | – | 639–800 |
| ganzes Bild: Worker, Median (p95) (ms) | 4,1–7,3 (4,9–10,8) | 3,9–5,1 (4,7–6,1) |
| ganzes Bild: Seite (ms) | 1,9–2,8 | 1,4–1,6 |
| ganzes Bild: verspätet (von Bildern der Anzeige) | 0–47 von 139–410 | 2–31 von 158–329 |
| kleine Änderung (Hover, 49 KB): Worker, Median (p95) (ms) | 1,1–1,5 (1,6–2,6) | 1,8–2,1 (2,4–3,9) |
| kleine Änderung: Seite (ms) | 1,4–1,9 | 1,4–1,6 |
| kleine Änderung: verspätet | 0–16 von 258–491 | 1–34 von 297–807 |
| Rollen: Worker, Median (p95, schlechtester) (ms) | 5,0–10,2 (10–15, 11–18) | 4,3–6,2 (9–15, 23–43) |
| Rollen: Seite (ms) | 12,8–15,7 | 16,6–21,9 |
| Rollen: Bilder in 120 Schritten | 176–238 | 262–281 |
| Rollen: verspätet | 128–491 von 420–719 | 259–760 von 553–900 |
| Leinwand ziehen (Muster, 928 × 320): Worker, Median (p95, schlechtester) (ms) | 2,1–2,6 (2,7–3,9, 3–15) | 2,5–2,6 (3,4–3,8, 16–21) |
| Leinwand ziehen: Seite (ms) | 2,7–3,8 | 2,5–2,7 |
| Leinwand ziehen: verspätet | 0 von 260–660 | 0–183 von 258–540 |
| Leinwand ziehen: zur Seite bzw. zur GPU (MB in 120 Schritten) | 316 | 39 |
| Leerlauf 10 s: Bilder | 0 | 0 |
| Leerlauf 10 s: CPU aller Prozesse des Browsers (ms; Schritte von 15,6 ms) | 578–828 | 500–891 |
| Speicher aller Prozesse, privat / Working Set (MiB) | 363–365 / 628–630 | 454–461 / 780–790 |
| davon GPU-Prozess, privat (MiB) | 89 | 175–179 |
| davon Renderer, privat (MiB) | 201–205 | 207–211 |
| `measureUserAgentSpecificMemory` (MB; ohne GPU) | 18,2–18,5 | 10,7–11,2 |
| `memory_reserved()`: was das Programm erbeten hat (MiB) | 0 | 7,3 |

Bis zum Bildschirm lässt sich kopflos nicht messen: Chrome setzt dort auf Verlangen zusammen, sein Trace hat für die meisten Bilder keinen `Display::DrawAndSwap`. Deshalb ein echtes Fenster, ohne Aktivierung, nicht zuoberst festgehalten, je eines für Chrome und Firefox; 60 Hover-Wechsel je Lauf (ausgelöst in der Seite, keine echte Maus), ein Thread liest die Fläche des Knopfs vom Bildschirm (`BitBlt`, je Lesen rund 6 ms: es wartet auf DWM und misst nicht bis zum Licht; 165 Hz). Wegwerf-Werkzeug. Die Uhr der Seite ist über die Schleife gegen die des Werkzeugs abgeglichen.

| Ereignis → Bildschirm, Median (p95) (ms) | bisher | GPU | |
| --- | ---: | ---: | --- |
| Chrome 156 | 18,9–19,3 (21–65) | 9,3–12,5 (15,5–16,9) | zwei Läufe je Weg; ein dritter GPU-Lauf lag unter einem anderen Fenster (7 von 60 gesehen), bisher: 44 von 60 in einem Lauf |
| Firefox 159 | 21,0–23,2 (57–96) | 8,7–14,4 (18–25) | drei bzw. zwei Läufe; ein dritter GPU-Lauf gestört (45 von 60 gesehen, keine Zahl) |

Das Bündel `threads` (drei Läufe, Worker Median): ganzes Bild 5,9–6,8 gegen 4,2–5,0 ms, kleine Änderung 1,4 gegen 2,0–2,1 ms, Rollen 6,1–7,1 gegen 4,8–5,8 ms; die Seite wie oben. Es verhält sich wie `simd`. `scalar` ist nicht gemessen.

Was das heißt:

- **Pixelgleich, auch auf dem echten Bildschirm.** Unten.
- **Das Bild kommt früher an**, 7–10 ms in beiden Browsern: Es geht vom Worker zum Compositor und nicht über `postMessage`, den Haupt-Thread und sein nächstes Bild. Das ist die eine Zahl, die sich Nutzern zeigt, und sie ist am wenigsten sicher gemessen (wenige Läufe, das Lesen hat 6 ms Korn).
- **Der Worker zahlt für kleine Änderungen 0,6 ms mehr** (Aufrufe nach JavaScript, ein Render-Durchgang), bei ganzen Bildern weniger (keine Kopie ins RGBA des Canvas, kein `postMessage`). Die Seite spart bei ganzen Bildern 1 ms.
- **Beim Rollen** kommen mehr Bilder durch (262–281 statt 176–238). Dafür gibt es Ausreißer: Bis 43 ms hängt der Worker in `writeTexture`, wenn der GPU-Prozess im Rückstand ist; der alte Weg hat sie nicht (11–18 ms), und unter fremder Last verspäten sich auf dem GPU-Weg mehr Bilder. Beim Rollen geht fast das ganze Bild hoch, 7–8 MB je Schritt, wie auf dem alten Weg zur Seite. Der teuerste Posten bleibt in beiden Spalten der Spiegel der Barrierefreiheit.
- **Leinwände**: Das Hochladen sinkt auf ein Achtel. Bei diesem billigen Muster gewinnt der Worker nichts; was unter Windows 30 ms kostet (ein Foto, ein Diagramm), kostet auch hier nur noch die Kacheln ([Leinwände](#leinwände)), gemessen ist das im Browser nicht.
- **Der Preis ist Speicher**: rund 95 MiB privat mehr, fast alles im GPU-Prozess (Dawn, Swapchain, Texturen), die Pixelpuffer des Workers gehen dafür. GPU-Speicher sagt der Browser nicht.
- **Im Leerlauf** kein Unterschied: kein Bild, gleiche CPU.
- **Entschieden am 03.11.2026: an auf dem Desktop** (`DEFAULT_ON_BROWSER = true`), Telefone und Tablets bleiben beim alten Weg, bis eins getestet ist. Der Rückfall ist sauber, und die beiden Browser, die hier gehen, zeigen dasselbe Bild früher. Der Preis sind 95 MiB, 200 KB WASM (70 KB gzip) und Ausreißer beim Rollen. Das Risiko liegt in dem, was nicht geprüft ist. Eine Leinwand, bei der nichts ankommt, fängt der Wächter; eine, die das richtige Bild hat und auf dem Schirm trotzdem nichts zeigt, merkt niemand.

| Größe, gallery, Release (Byte) | vor G4d | jetzt | + | gzip vor G4d | gzip jetzt |
| --- | ---: | ---: | ---: | ---: | ---: |
| `scalar` | 3 617 267 | 3 834 060 | 216 793 | 1 357 042 | 1 430 723 |
| `simd` | 3 556 661 | 3 772 009 | 215 348 | 1 343 295 | 1 412 115 |
| `threads` | 3 521 562 | 3 702 658 | 181 096 | 1 336 180 | 1 404 051 |

Dazu `gpu.js` (16 816 Byte, gzip 5 940), das nur geladen wird, wenn die Ausgabe startet. Keine Vulkan-Zeichenkette in den Bündeln (`Vulkan`, `vulkan`, `vkCreate`, `VK_`: 0).

**Pixelvergleich.** Was die Tests vergleichen, ist die Zusammensetzung des Browsers, nicht nur die Textur.

| Was | Wie | Ergebnis |
| --- | --- | --- |
| Bildschirm GPU gegen Bildschirm bisher, Schritt für Schritt (erstes Bild, Hover, Menü auf, Menü zu, neue Größe; bei `simd` auch 125, 150 und 200 %, „Bild & Leinwand“ nach Ziehen und Rollen) | `page.screenshot`, Chrome kopflos, alle drei Bündel | 0 Pixel anders |
| Bildschirm gegen Framebuffer (`ui.render`), auch auf dem Rückweg nach verlorenem Gerät und auf dem neuen Gerät | ebenso | 0 Pixel anders |
| jedes gezeigte Bild gegen den Framebuffer, Byte für Byte | `jfgui_GPU_VERIFY` im Browser: zurückgelesen, verglichen, wenn der Browser geantwortet hat | 0 anders |
| während der Umschaltung | 5–6 Aufnahmen, die erste vor dem Wechsel | 0 anders als bisher, keine schwarz |
| echtes Fenster, Chrome 156 | `BitBlt` der Leinwand vom Bildschirm, bisher gegen GPU | 0 von 672 072 Pixeln in drei von vier vergleichbaren Läufen; im vierten lag ein anderes Fenster darüber |
| echtes Fenster, Firefox 159 | ebenso | 0, 18 und 32 von 618 936 Pixeln; zwischen zwei Läufen desselben Wegs sind es ebenfalls 0 bis 32 (die Ursache ist nicht gesucht; am Weg liegt es also nicht) |

**Gefunden:** `quad_blit` zeichnete eine Leinwand beim Nachzeichnen in den Framebuffer (`gp_settle`, jede Ebene darüber) mit ihrem ganzen Rechteck, ohne den Klipp eines rollenden Elternteils: Unter einer Kopfzeile standen dann ihre Pixel, im Framebuffer, also auf dem Rückweg zum CPU-Pfad auch auf dem Schirm. Das galt auf allen Zielen; gefunden hat es der Vergleich im Browser, unter Windows und Linux prüft es jetzt `apps/androidGallery/tests/gpu_canvas_clip.uis`. Ein Treiberfehler wie beim Xclipse (Push-Konstanten-Array) kam hier nicht vor; auf einem Telefon ist nichts getestet.

**Ganze Gerätepixel.** Bei 125 % endet die 38 px hohe Host-Leiste bei 47,5 Gerätepixeln, und `clientWidth` zählt ganze CSS-Pixel. Schon der alte Weg zeigte deshalb bei 110, 125 und 175 % ein umgerechnetes Bild (17 000 bis 39 000 Pixel anders als der Framebuffer, die Schrift weicher), und `#gpu` wich davon noch einmal ab. Jetzt endet die Leiste auf einem ganzen Gerätepixel, die Größe der Leinwand kommt aus ihren Kanten, gerundet wie vom Browser, und `image-rendering: pixelated` zeigt den Puffer 1:1, auch wo CSS den Ort nicht genau sagen kann (48 Gerätepixel sind bei 125 % 38,39 px). `gpu_web.mjs` zählt bei 100, 125, 150 und 200 % 0 Pixel, beide Wege gegeneinander und gegen den Framebuffer. Bei 110 und 175 % lief nur ein Hilfsskript einmal von Hand: beide Wege gleich, gegen den Framebuffer 91 und 215 Pixel um höchstens 3, wie es dieses Skript auch bei 100 % sieht. Fenstergrößen mit gebrochenen Gerätepixeln (1001 px bei 125 %, nur unter Playwright möglich) sind ausgenommen.

**Gefunden bei der Durchsicht.** Mehr als 64 Push-Werte in einer Übergabe ließen die Aufrufe danach still den letzten Wert sehen, das Bild wäre bis zur nächsten Änderung falsch geblieben. Jetzt übernimmt mitten in der Übergabe ein größerer Ring (`webgputest`: 200 Werte, vorher ein Fehlschlag).

**Absturz des GPU-Prozesses.** Einmal von Hand: Der GPU-Prozess eines eigenen, kopflosen Chrome wurde beendet. Chrome startete einen neuen, die Seite meldete `GPU device lost`, und innerhalb von 3 s lief die Ausgabe auf einem neuen Gerät und einer neuen Leinwand (`leaves=1`, `switches=2`). Das ist kein Teil der Tests.

**Firefox.** Playwrights Firefox 153 sperrt WebGPU („disabled by blocklist“, kein Adapter), das installierte Firefox 159 kopflos ebenso: Dort prüfen die Tests den Rückfall. In einem echten Fenster hat Firefox 159 einen Adapter und zeigt die Ausgabe (die Zahlen oben, ohne Treiber in der Seite gemessen, weil Playwright es nicht steuern kann). Es nennt seinen Adapter nicht, `GpuOutput::adapter` bleibt dort leer.

**Tests.** `apps/gallery/tests/gpu_web.mjs` (die Ausgabe in allen drei Bündeln, Chrome und Firefox; ohne brauchbares WebGPU `SKIP` und nur der Rückfall; ein verlorenes Gerät über `device.destroy()`, also nachgestellt; der Wächter mit einer leeren und einer schwarzen Leinwand, ebenfalls nachgestellt), `apps/webgputest` mit `tests/api_web.mjs` (die API ohne Fenster: Fehler, Rechen-Shader, Kopien, Zeichnen, Gerät verloren, in jedem Bündel), [`tools/tests/web_gpu_bench.mjs`](../tools/tests/web_gpu_bench.mjs) (die Messungen). `.uis`-Skripte laufen im Browser nicht; das Skriptkommando `gpu` gibt es dort nicht.

**Nicht getestet:** Safari, Chrome auf Android und Linux, Firefox außerhalb dieses einen Rechners und ohne die Vorführung oben (Ebenen, Bilder, Leinwände sind in Firefox nicht durchlaufen; `threads` nur minimiert, also ohne Blick auf den Bildschirm), AMD und Intel, Monitore mit HDR oder großem Farbraum, ein echter Treiber-Reset, ein Tab im Hintergrund (nur der Wächter in einem minimierten Fenster), der Zoom der Seite in einem echten Fenster, ein Telefon.

**Nicht gebaut:** Zeitstempel, `create_compute_pipeline_sized`, Bindless; das Skriptkommando `gpu`; Freigeben von `#screen` während der GPU-Zeit (dort bleiben 8 MB für den Rückweg).

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
| nicht gesetzt, `jfgui_GPU=1` (oder `on`) | die UI-Fenster zeigen ihre Bilder über die Swapchain ([Ausgabe der UI](#ausgabe-der-ui)); ohne die Variable entscheiden `DEFAULT_ON`, `DEFAULT_ON_LINUX` und `DEFAULT_ON_BROWSER` in `gpu_out.rs`, unter Android `GPU_DEFAULT` in `GalleryActivity`. Im Browser kommen die Variablen aus der Adresse: `?gpu=0\|1` und `?gpuenv=jfgui_GPU_NAME=wert,..` (nur Namen `jfgui_GPU_*`); ohne `?gpu` setzt `web/loader.js` auf Telefonen und Tablets `jfgui_GPU=0` |
| `jfgui_GPU_RES=cap` / `=full` | (Android, Debug-Builds) die Auflösung, durch die GPU und auf dem CPU-Pfad ([Ausgabe unter Android](#ausgabe-unter-android)) |
| `jfgui_GPU_TEST_DIE=1` | (Android, Tests) der Arbeitsthread beendet das Programm, wo ein kaputter Treiber abstürzen würde |
| `jfgui_TEST_LANDSCAPE=1` | (Android, Tests) die Activity bittet um Querformat, keine Einstellung des Systems |
| `jfgui_ANDROID_PACKAGE=name` | (Bauen) die APK unter einem anderen Paket, neben einer installierten Galerie |
| `jfgui_GPU_DMABUF=0` / `=refuse` | (Linux, Tests) der Bildschirm nimmt keine GPU-Puffer / lehnt sie ab |
| `jfgui_GPU_WORKER_KB=n` | Stapel des Arbeitsthreads der UI-Ausgabe (um zu sehen, was ein Treiber braucht) |
| `jfgui_WL_FEEDBACK=1` | (Wayland, Messungen) `wp_presentation`: wann ein Commit auf dem Bildschirm war (`plat::gpu_test_presented`, `uigputest --bench latency`) |
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
| `jfgui_GPU_DLL=name` | anderer Loader statt `vulkan-1.dll` (Name: nur aus System32; Pfad: aus seinem Ordner) bzw. `libvulkan.so.1` (wie `ld.so` sucht) |
| `jfgui_GPU_ADAPTER=n` / `=name` | Adapter n von `adapters()` oder der erste, dessen Name das Wort enthält |
| `jfgui_GPU_PRESENT=fifo\|mailbox\|immediate` | Darstellungsmodus jeder neuen Swapchain |
| `jfgui_GPU_COMPUTE=0` (oder `off`) | (G5d) keine eigene Rechen-Warteschlange: `Device::compute_queue()` ist `None`, Ressourcen exklusiv |
| `jfgui_GPU_VALIDATE=1` | Khronos-Validierungsschicht mit Synchronisationsprüfung, wenn installiert |
| `jfgui_GPU_DEBUG=1` | die Debug-Meldungen des Treibers (ab Info-Stufe) auf stderr |

## Validierung

Das Vulkan SDK ist nur ein Entwicklungswerkzeug; zur Laufzeit braucht `lib/gpu` nichts davon.
Mit installiertem SDK (Schicht `VK_LAYER_KHRONOS_validation`) schaltet `jfgui_GPU_VALIDATE=1`
oder `InstanceDesc { validate: true, .. }` die Validierung ein, samt Synchronisationsprüfung
(`VK_EXT_layer_settings` „validate_sync“ und „syncval_shader_accesses_heuristic“, sonst
`VK_EXT_validation_features`, das nur das Erste kennt). Die zweite Einstellung ist in der Schicht
aus; ohne sie sieht die Prüfung nicht, was Shader über ihre Bindungen lesen und schreiben, und eine
fehlende Barriere zwischen Rechen- und Vertex-Shader bleibt unbemerkt (so geschehen bei G5a, auf der
NVIDIA-Karte ohne sichtbare Folge). Sie schließt aus dem SPIR-V auf die Zugriffe und kann Falsches
melden. Meldungen gehen
über `VK_EXT_debug_utils` als `gpu: validation error: ...` auf stderr;
`gpu::validation_messages()` zählt Fehler und Warnungen. Ohne die Schicht erscheint einmal ein
Hinweis und alles läuft wie ohne die Variable.

```text
set jfgui_GPU_VALIDATE=1
build\win\gputest.exe --test all          (oder: gputest --test all --validate)
```

`tests/validate.uis` lässt jeden Test einmal mit Validierung laufen; ein Validierungsfehler
lässt ihn scheitern. Der Test `syncval` baut absichtlich zwei fehlende Barrieren ein, eine zwischen
Kopien und eine zwischen zwei Dispatches, und erwartet, dass die Synchronisationsprüfung sie meldet
-- so ist sicher, dass sie wirklich an ist.
Die selbst geschriebenen SPIR-V-Module von gputest sind mit `spirv-val --target-env vulkan1.1`
und `vulkan1.3` geprüft.

Achtung bei Messungen: Der Vulkan Configurator (`vkconfig`) kann Schichten für alle Programme
erzwingen (eine Loader-Einstellungsdatei unter `%LOCALAPPDATA%\LunarG\vulkan`), etwa
GFXReconstruct, das jeden Aufruf aufzeichnet. `VK_LOADER_LAYERS_DISABLE` hebt das nicht auf;
für Messungen den Configurator ausschalten.

## Tests

`python -u build_rs.py test gputest` führt `apps/gputest/tests/*.uis` aus, unter Windows und Linux
(dort gegen glibc, ohne die Fenstertests: [Linux](#linux)); unter Android `apps/gputest/tests/android.py`
([Ausgabe unter Android](#ausgabe-unter-android)):

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
| `queues.uis` | (G5d) die Rechen-Warteschlange: Runden auf einem Puffer abwechselnd auf beiden Warteschlangen mit Warten über die Zeitleiste (= CPU), eine Speichertextur von der einen zur anderen, Warten und Ids beider, ein Puffer, der fällt, während er noch benutzt wird, was verweigert wird, ohne Rechen-Warteschlange (`jfgui_GPU_COMPUTE=0`); mit `--validate` eine fehlende Barriere auf der Rechen-Warteschlange |
| `validate.uis` | alles einmal mit Validierungsschicht (siehe oben), auch `scene`, `bench_smoke`, `spec`, `textures` und `wgsl` (ohne Meldung) |
| `glibc.uis` | (Linux, gegen glibc gelinkt; sonst `SKIP`) Argumente und Umgebung; Hauptthread und jeder `plat`-Thread mit eigenem Kontrollblock und TLS (FS-Basis = `pthread_self`, eigenes `errno`, eigener `dlerror`-Text); der verlangte Stapel; 300 Threads hinterlassen keine Abbildung; Vulkan auf einem `plat`-Thread von 2 MiB. In Kindprozessen: `exit` von glibc aus `main` und von einem Thread, auch mit laufenden Threads und einem noch offenen Gerät; Absturz auf dem Haupt- und einem `plat`-Thread, Stapelüberlauf und Panik mit Bericht, Code 3 und ohne `at_exit` |
| `noloader.uis` | `jfgui_GPU_DLL` nennt eine fehlende Datei (Name, Pfad) oder eine Bibliothek, die kein Vulkan-Loader ist: `Err(NoLoader)`; ein Programm ohne Loader startet, `--info` meldet `SKIP`, Code 0 |

Ohne Vulkan oder brauchbare GPU melden die Tests `SKIP` und gelten als bestanden. Im Browser: `apps/webgputest/tests/api_web.mjs`
und `apps/gallery/tests/gpu_web.mjs` ([Ausgabe im Browser](#ausgabe-im-browser)).
`gputest --bench` misst die Zahlen unten, `gputest --bench N` die Messszene.

**Texturen in Shadern** (`gputest --test textures`, [`t_texture.rs`](../apps/gputest/t_texture.rs);
die Shader sind `shader::samples`, vor dem Hochladen von `shader::val` geprüft). Exakt heißt:
gleich der CPU-Referenz Bit für Bit; ein rgba8unorm-Kanal darf von `k / 255` in f32 eine Stelle
abweichen (die RTX tut es nie, Mesa manchmal: [Linux](#linux)), der Test zählt das.

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

G5a hat davon die Hälfte umgesetzt ([RENDER3D.md](RENDER3D.md), mit Messungen): kompakte Kugeln, fertige Matrix,
Instanzlisten je Mesh, nur Geändertes hochladen, Reverse-Z. Die Zeichenzeit blieb dabei gleich, das Quaternion je Ecke war
also nicht der Engpass. Zwei Dinge fielen erst dort auf: 32-Bit-Indizes und feste Vertex-Attribute kosten auf dieser GPU viel
mehr, als man erwartet.

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

## Linux

Alle GPU-Treiber unter Linux sind glibc-Bibliotheken, ein Prozess ohne glibc kann keinen laden.
Ein Programm für die GPU wird deshalb dynamisch gegen glibc gelinkt und gegen nichts sonst;
`libvulkan.so.1` kommt zur Laufzeit per `dlopen`, ohne die Datei startet es und `Instance::new`
gibt `Err(NoLoader)`. Kommandozeilen- und Serverprogramme bleiben statisch ohne libc
([ROADMAP.md](ROADMAP.md) §8). Ein statisches Programm, das `gpu` linkt, bekommt immer `NoLoader`.

**Stand (G4b):** Alles wie unter Windows, Fenster über dmabuf ([Ausgabe unter Linux](#ausgabe-unter-linux)).
`Features::swapchain` ist auf einem CPU-Adapter `false`, die Fenstertests von `gputest` melden dort `SKIP`.

**Welche Programme.** Ein Programm, das `ui` nennt, wird gegen glibc gelinkt, auch `gui`-Programme einer App in `BINS`;
`cli`-Programme und alles ohne `ui` bleiben statisch und im Release-Build Byte für Byte, wie sie waren (`wgsltest`, `jfs`, `appimagert` geprüft; ein Entwicklungs-Build wie `coretest` kann um Füllbytes abweichen, weil `plat` sich anders in Codegen-Einheiten teilt: dieselben Funktionen in derselben Größe).
Eine Datei `apps/NAME/GLIBC` erzwingt glibc (`gputest`, `uigputest`), eine Datei `apps/NAME/STATIC` hält eine App statisch
(`plattest`, `linuxtest`: sie testen weiter, was `--static` ergibt). `build_rs.py ... --static` und `tools/package.py --static`
bauen die UI-Programme dieses Laufs statisch: kein Vulkan-Code, nur Shared Memory, für Systeme ohne glibc wie Alpine. Module und Plugins
bleiben, wie sie sind: ihr Lader braucht keine libc, sie importieren nichts (PaintCraft mit `jxlenc` und den Rust-Plugins bestanden, auch der Absturz eines Plugins).

**Bauen.** `build_rs.py`, Abschnitt „glibc“:

- Jede Crate bekommt `--cfg jfgui_glibc` und liegt in `build/linux/.glibc`, das Programm wie jedes
  andere in `build/linux`. Nennt es `ui` nicht, wird kein Stil gebaut (`gputest`: 7 Crates).
- Zum Linken ist nichts zu installieren. `Builder.glibc_stubs` schreibt aus der Tabelle `GLIBC_STUBS`
  drei Attrappen `libpthread.so.0`, `libdl.so.2` und `libc.so.6`: eine Funktion `ret` unter jedem
  Namen (`rustc --emit=obj`), dann `ld.lld -shared` mit einem Versionsskript. Sie nennen dem Linker
  nur Namen und Versionen; zur Laufzeit gelten die Bibliotheken des Systems.
- Das Programm hat eine feste Adresse wie die statischen (`ET_EXEC`, die `.map` gilt weiter), den
  Interpreter `/lib64/ld-linux-x86-64.so.2`, `DT_NEEDED` auf die drei Dateien, bindet beim Start
  (`BIND_NOW`), importiert nur Funktionen und exportiert nichts: `memcpy` aus `rt` ersetzt nie die
  der Treiber.
- Alle 15 Importe tragen die Version `GLIBC_2.2.5`, die älteste, die x86-64 hat. Bis glibc 2.33
  stehen die Funktionen so in `libdl` und `libpthread`. Seit 2.34 stehen sie in `libc`, dort bleibt
  diese Version als Kompatibilitätssymbol, und `libdl` und `libpthread` bleiben als leere Dateien.
  Dieselbe Datei startet also auf beiden Seiten; von den Symbolen her auf jeder glibc für x86-64, eine
  Grenze setzen eher die Treiber (Vulkan 1.1 gibt es ab etwa RHEL 7, glibc 2.17). **Geprüft** ist
  bisher nur die neue Seite (2.39, 2.43); auf den Linux-Umgebungen hier liegt keine ältere glibc
  ([LINUX_TESTPLAN.md](LINUX_TESTPLAN.md) Abschnitt 5).

**Was in `lib/plat` anders ist** ([`linux_glibc.rs`](../lib/plat/linux_glibc.rs), das Ende von
[`linux_thread.rs`](../lib/plat/linux_thread.rs)):

| Teil | statisch | gegen glibc |
| --- | --- | --- |
| Start | `_start` ruft `jfgui_main`, dann `exit_group` | `_start` ruft `__libc_start_main` (TLS und Initialisierung hat `ld.so` schon erledigt), die gibt den Rückgabewert an `exit` |
| Threads | roher `clone` ohne TLS | `pthread_create` auf derselben Abbildung (Schutzseite, Signalstapel, Schutzseite, Stapel); oben 16 KiB mehr für Kontrollblock und statischen TLS, der Stapel bleibt so groß wie verlangt; `pthread_join`, dann `munmap` |
| `plat::exit`, Rückkehr aus `main` | `exit_group` | `exit`: `plat::at_exit`, Destruktoren der Treiber, stdio |
| Absturz, Panik (`rt::os::exit`) | `exit_group` | `exit_group`, kein Destruktor im Signal-Handler |
| `dl_open`, `dl_sym`, `dl_close`, `dl_error`, `glibc_version` | gibt es nicht | das `dlopen` des Systems |

`rt` bleibt gleich. Signale setzt `plat` weiter mit `rt_sigaction` direkt, ohne die zwei internen von
glibc zu berühren; der Allokator (`rt::mem`, eigene Abbildungen, kein TLS, auch von Treiber-Threads
aufrufbar) und `fork` (das Kind macht bis `exec` nur Systemaufrufe) bleiben, wie sie sind. Die
Threads eines Treibers haben keinen Signalstapel: Dort läuft der Bericht auf ihrem Stapel. Mesa sperrt
auf seinen Threads alle Signale außer SIGSEGV und SIGSYS; ein SIGILL dort beendet den Prozess also
sofort durch den Kernel, ohne Bericht, ein SIGSEGV kommt an. `plat::setenv` ändert die Liste von JFLib,
die `plat::env` und Kindprozesse sehen; das `getenv` eines Treibers liest die Umgebung vom Start.

Statische Programme sind dabei Byte für Byte gleich geblieben (`plattest`, `gallery` im
Entwicklungs-Build, `wgsltest` im Release-Build, gegen den Stand davor). Deshalb stehen die
`cfg`-Attribute in `linux.rs` und `linux_thread.rs` in derselben Zeile wie das Element: die
Zeilennummern der Panik-Stellen sind Bytes im Programm.

Größe von `gputest`: Release statisch 2 569 872, gegen glibc 2 599 296 Byte (+29 424),
Entwicklungs-Build 2 782 232 und 2 817 944 (+35 712). Darin steckt der Test `glibc`, den nur dieser
Build hat (rund 12 KB); Dynamik-Tabellen und PLT sind knapp 2 KB.

**Tests.** `gputest --test glibc` (Start, Threads, Ende) und `--test noloader`, siehe
[Tests](#tests). Dabei läuft auch Vulkan auf einem `plat`-Thread mit 2 MiB Stapel, der Größe des
Arbeits-Threads von `gpu_out`. Einmal von Hand in WSL: Das Programm in einem `bwrap` mit nichts als
`ld.so` und den drei Bibliotheken startet, `--info` meldet `SKIP`, `--test glibc` besteht.

| Umgebung | Treiber | Ergebnis |
| --- | --- | --- |
| WSL Ubuntu 24.04, glibc 2.39, Ryzen 5 3600 | llvmpipe, Mesa 25.2.8, LLVM 20.1.2, 256 Bit | alles bestanden, Fenstertests `SKIP` |
| Fedora 44, glibc 2.43, Pentium G4560T | Intel HD Graphics 610 (KBL GT1), anv, Mesa 26.0.3, Vulkan 1.4.335 | alles bestanden, Fenstertests `SKIP`, in 7–13 s |
| Fedora 44, derselbe Rechner | llvmpipe, Mesa 26.0.3, LLVM 22.1, 128 Bit | Treiberfehler, unten; bestanden nur `copies`, `upload`, `bindless`, `noloader` |
| WSL mit `GALLIUM_OVERRIDE_CPU_CAPS=sse4.1 LP_NATIVE_VECTOR_WIDTH=128` | llvmpipe, Mesa 25.2.8, LLVM 20.1.2, 128 Bit | alles bis in `wgsl`, dort nach `modf` ein Absturz in `libLLVM` (`InstCombine`) |

Die Validierungsschicht ist auf beiden Rechnern nicht installiert, unter Linux lief also noch keine
Validierung (`validate.uis` läuft dort ohne sie).

**Was die erste fremde GPU und llvmpipe gefunden haben:**

- *Ein `Device`-Puffer ließ sich auf einem CPU-Adapter schreiben.* Dort ist aller Speicher
  abbildbar, `Buffer::write` prüfte nur, ob eine Adresse da ist; ein Programm liefe auf llvmpipe und
  scheiterte auf der nächsten GPU. Jetzt zählt die Art des Puffers, auf jedem Adapter `Invalid`.
- *Der Ring wuchs auf llvmpipe um 10 MiB.* 2000 Übergaben ohne Warten: Ein CPU-Gerät liegt
  Hunderte zurück, und was sie lesen, kann nicht zurück in den Ring. Das ist richtig; der Test hat
  nachgestellt, was die UI nicht tut. Er hält jetzt drei Bilder in Flug wie eine Swapchain, die Grenze
  von 2 MiB bleibt und gilt überall.
- *`k / 255` ist nicht auf jeder GPU korrekt gerundet.* Mesa landet bei manchen `k` eine Stelle
  daneben, was Vulkan erlaubt: llvmpipe überall (513 von 1364 Werten beim Laden und Abtasten, 281 von
  768 bei Speichertexturen), Intel nur bei Speichertexturen (281 von 768), die RTX nie. `gputest`
  erlaubt das nur bei rgba8unorm und zählt es; alle anderen Formate bleiben exakt.
- *Intel spült f16-Denormale auf 0.* WGSL erlaubt das, `Tol::Halves` kannte die Regel nur für f32.
- *llvmpipe auf Fedora 44 ist auf diesem Prozessor unbrauchbar* (Mesa 26.0.3 mit LLVM 22.1, Pentium
  ohne AVX und BMI): Abstürze in `libLLVM` beim Übersetzen (`simplifyMaskedScatter`,
  `SimplifyDemandedVectorElts`), falsche Ergebnisse (`i32 +` gibt für 1 + 1 die 0), `SIGILL` in
  gejittetem Code (`andn` auf einem Prozessor ohne BMI1). Dieselben SPIR-V-Module laufen auf der
  HD 610, der RTX 2070 und auf llvmpipe mit LLVM 20 und AVX2; es liegt am Treiber. Die UI nimmt ohnehin
  keinen CPU-Adapter (`adapters_low_power`), `instance.adapter()` nimmt ihn als letzten.
- *Nichts gefunden* bei den Speichertypen einer integrierten GPU (`vk_mem.rs` trifft dort den
  richtigen Typ, Uploads und Rücklesen stimmen), bei den Grenzen (Uniform-Ausrichtung 64, Speicher 4,
  Bindung bis 4095 MiB) und bei der wechselnden Subgroup-Größe (8 bis 32, gesehen 8, 16 und 32; die
  Referenz der Subgroup-Tests kennt sie). Alle 13 469 Ergebnisse des WGSL-Tests stimmen.

**Die HD 610 in Zahlen.** Release-Build, ohne Fenster, Mesa 26.0.3; je Variante drei Läufe,
abwechselnd, auf dem geteilten Rechner (Last um 1, Firefox lief mit). Median, in Klammern die
Spanne über die Läufe. Zeiten in ms. 300 Bilder nach 60 zum Aufwärmen, bei 1 Mio. und 100k einzeln
weniger.

| N | Modus, Ziel | CPU Med. | GPU move / cull / draw | GPU gesamt (Spanne) | Bildzeit Med. (p99) | Bilder/s | Dreiecke/s |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10k | indirekt, 1080p | 0,21 | 0,08 / 0,10 / 2,23 | 2,42 (2,37–2,43) | 2,51 (3,82) | 382 | 0,03 G |
| 100k | indirekt, 720p | 0,20 | 0,53 / 0,92 / 6,43 | 7,95 (7,93–7,97) | 7,95 (8,88) | 127 | 0,08 G |
| 100k | indirekt, 1080p | 0,20 | 0,52 / 0,93 / 7,46 | 9,19 (9,16–9,19) | 9,29 (10,3) | 108 | 0,07 G |
| 100k | indirekt, 1080p, WGSL | 0,19 | 0,51 / 0,85 / 7,57 | 9,19 (9,13–9,20) | 9,30 (10,4) | 108 | 0,07 G |
| 1M | indirekt, 720p | 0,34 | 5,21 / 10,2 / 53,6 | 68,8 (68,7–68,9) | 70,0 (71,3) | 14 | 0,09 G |
| 1M | indirekt, 1080p | 0,30 | 5,40 / 9,91 / 54,9 | 70,2 (69,8–70,3) | 70,5 (72,2) | 14 | 0,09 G |
| 10k | einzeln, 1080p | 4,89 | 0,06 / – / 3,55 | 3,63 (3,58–3,64) | 4,89 (6,07) | 205 | 0,03 G |
| 100k | einzeln, 1080p | 32,9 | 0,48 / – / 29,1 | 29,6 (29,5–29,6) | 32,9 (40,7) | 30 | 0,04 G |

Was eine schwache GPU also tut: 100 000 bewegte Würfel in 1080p bei 108 Bildern/s, gerechnet rund
200 000 bei 60 Bildern/s. Die RTX 2070 SUPER ([Messszene](#messszene)) ist bei 1 Mio. Würfeln gut 40-mal
schneller (70 ms gegen 1,65 ms), bei 100 000 Würfeln 32-mal. Engpass ist die Geometrie, rund 90 Mio.
Dreiecke/s gegen 3,5 Mrd.; `move` ist 21-mal langsamer, `draw` 44-mal, `cull` 75-mal (es liest die
ganzen Objektdaten, vermutlich die Speicherbandbreite). Die Pixel kosten wenig: 1080p gegen 720p bei
100 000 Würfeln 1,2 ms. Ein Zeichenbefehl je Objekt kostet die CPU 330–490 ns und bremst hier auch die
GPU (29,1 statt 7,46 ms bei 100 000). WGSL und SPIR-V sind gleich schnell. Für die UI ist das reichlich:
ein ganzes Bild 1080p hochladen 2,5–3,4 ms, kleine Uploads 41 µs, eine Million Elemente rechnen und
zurücklesen 2,8 ms; Instanz 29–37 ms, Gerät 3 ms (RTX: 23–32 ms, 65–70 ms). Zeiten mit Darstellung:
[Ausgabe unter Linux](#ausgabe-unter-linux).

**Sandbox.** Eine App mit `sandbox`-Zeile braucht für den Treiber nichts zusätzlich: Landlock erlaubt `/usr`, `/etc`, `/lib*`, `/opt`, `/sys`, `/run` zum Lesen und `/dev`, `/tmp` zum Schreiben
([`linux_sandbox.rs`](../lib/plat/linux_sandbox.rs)), die Liste von seccomp sperrt nichts, was ein Treiber zwingend braucht (`perf_event_open` gibt `EPERM`). Der Shader-Cache in `~/.cache` schlägt fehl, wenn das Manifest ihn nicht erlaubt: Dann übersetzt der Treiber jedes Mal.
`sandboxtest` prüft das mit dem echten Treiber (Fedora: HD 610 unter `sandbox strict`).

**Pakete.** `.deb` und RPM nennen glibc als Abhängigkeit und empfehlen Vulkan, Flatpak bekommt `--device=dri` ([PACKAGING.md](PACKAGING.md)); mit `--static` nichts davon.

## Bekannte Lücken

- Unter Android keine Validierung (die Schicht liegt keiner APK bei). Unter Linux lief noch keine Validierung und
  noch keine ältere glibc als 2.39 ([Linux](#linux)); nur Intel und KWin als Treiber und Compositor ([Ausgabe unter Linux](#ausgabe-unter-linux)).
- Keine eigene Kopierwarteschlange; die Rechen-Warteschlange (G5d) wartet an allen Befehlen (keine feineren
  Wartestufen), und die Validierungsschicht meldet fehlende Synchronisation zwischen Übergaben nicht.
- Keine Pipeline-Statistik-Abfragen (Vertex-, Primitiv-, Fragment-Zähler); nur Zeitstempel.
- Kein Pipeline-Cache, kein MSAA, keine Stencil-Operationen, keine Textur-zu-Textur-Kopien und
  kein Blit (Mip-Erzeugung).
- `Format` hat kein `Rgba32Sint` (und keine anderen vierkanaligen 8-/16-Bit-Ganzzahlformate);
  gputest überspringt diese Speichertextur.
- Bindungen ohne Arrays (`count` 1); Arrays gibt es über die Bindless-Tabelle auf Stufe Voll.
- Kopien von Tiefen-Stencil-Formaten werden abgelehnt.
- Zeilenlängen für Kopien im Kodierer prüft Vulkan nicht auf 256 Byte; WebGPU verlangt das --
  für Portabilität `bytes_per_row` als Vielfaches von 256 wählen (das WebGPU-Backend sagt sonst `Invalid`).
- Ohne `hostQueryReset` (Vulkan 1.1) werden neue Zeitstempel erst in der nächsten Übergabe
  zurückgesetzt; vorher `read_*` aufzurufen ist dort nicht erlaubt.
- Ein `Frame`, der nie dargestellt wird, lässt die Swapchain beim nächsten `acquire()` neu bauen.
- Die Pfade für Vulkan 1.1 und 1.2 (Erweiterungen statt Kern) sind geschrieben, aber mangels
  solcher Hardware hier nicht getestet (der Emulator mit SwiftShader meldet 1.2, ihm fehlt dynamisches Rendern).

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

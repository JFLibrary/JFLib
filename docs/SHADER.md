# lib/shader – SPIR-V erzeugen, lesen und prüfen

`lib/shader` ist das Back-End des späteren WGSL-Compilers (Schritt G2) und schon jetzt direkt aus
Rust benutzbar: ein Modul-Builder, der SPIR-V-Wörter schreibt, ein Disassembler im Stil von
`spirv-dis`, ein struktureller Validator, ein Layout-Helfer für std140 / std430 und die Shader der
GPU-Benchmark-Szene mit ihrem Datenvertrag und CPU-Referenzen. Die Crate ist `no_std` + `alloc`,
ohne Abhängigkeiten und ohne GPU testbar (`apps/shadertest`).

Zwei Feature-Stufen:

| Stufe | Vulkan | SPIR-V | Konstante |
|---|---|---|---|
| Basis | 1.1 | 1.3 | `shader::BASIS` (= 3) |
| Voll | 1.3 | 1.6 (1.4 und 1.5 gehen auch) | `shader::VOLL` (= 6) |

`Module::new(minor)` nimmt die Unterversion 3 bis 6. Was sich zwischen den Versionen ändert,
erledigt der Builder selbst: die Interface-Liste des `OpEntryPoint` (bis 1.3 nur die Input- und
Output-Variablen, ab 1.4 jede globale Variable, die der Aufrufbaum benutzt), `discard`
(`OpKill` bis 1.5, `OpTerminateInvocation` ab 1.6) und `demote` (vor 1.6 mit
`SPV_EXT_demote_to_helper_invocation`).

## Builder-API

```rust
use shader::{Module, Rule, Sc, Bi, Model, Mode};
let mut m = Module::new(shader::BASIS);
let (void, f, u) = (m.ty_void(), m.ty_f32(), m.ty_u32());
let arr = m.ty_runtime_array(f, Some(4));
let block = m.ty_struct_layout("Data", &[("values", arr)], Rule::Std430);
let data = m.buffer(Sc::StorageBuffer, block, 0, 0);
let uv3 = m.ty_vec(u, 3);
let gid = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
let (main, _) = m.begin_function(void, &[]);
let g = m.load(gid);
let i = m.extract(g, &[0]);
let c0 = m.const_u32(0);
let p = m.access(data, &[c0, i]);
let x = m.load(p);
let y = m.fmul(x, x);
m.store(p, y);
m.ret();
m.end_function();
m.entry_point(Model::GLCompute, main, "main");
m.exec_mode(main, Mode::LocalSize, &[64, 1, 1]);
let words: Vec<u32> = m.finish();          // shader::to_bytes(&words) = die .spv-Datei
```

Kompakt:

- **Modul**: `Module::new(minor)` (Capability `Shader`, Memory Model `Logical GLSL450`),
  `capability`, `extension`, `glsl_set` (Import von GLSL.std.450, beim ersten Gebrauch),
  `source(Lang::WGSL, v)`, `generator` (Header-Wort, Standard 0), `finish() -> Vec<u32>`,
  `to_bytes` / `from_bytes` (auch Big-Endian).
- **Typen** (dedupliziert, außer Structs und Arrays mit Stride): `ty_void`, `ty_bool`,
  `ty_int(width, signed)` / `ty_u32` / `ty_i32` (8, 16, 64 Bit setzen `Int8` / `Int16` /
  `Int64`), `ty_float(width)` / `ty_f32` (16 / 64 setzen `Float16` / `Float64`), `ty_vec`,
  `ty_mat(col, cols)`, `ty_array`, `ty_array_stride`, `ty_array_layout(elem, n, rule)`,
  `ty_runtime_array(elem, stride)`, `ty_struct`, `ty_struct_layout(name, members, rule)`
  (OpName / OpMemberName, `Offset` je Member, `ColMajor` + `MatrixStride` an Matrizen),
  `ty_ptr`, `ty_func`, `ty_image` (setzt `Sampled1D`, `ImageBuffer`, `SampledCubeArray` …),
  `ty_sampler`, `ty_sampled_image`; Abfragen `ty(id)`, `type_of(value)`, `bool_like`,
  `scalar_of`.
- **Konstanten** (dedupliziert; `-0.0` und `0.0` sind verschieden): `const_u32`, `const_i32`,
  `const_f32`, `const_bool`, `const_scalar(ty, words)` (auch 64 Bit), `const_composite`,
  `const_vec_f32`, `const_null`.
- **Globale Variablen**: `global(sc, ty, init)`, `input(ty, location)`, `output(ty, location)`,
  `builtin(sc, ty, Bi)` (setzt `ClipDistance`, `DrawParameters`, `SampleRateShading` …),
  `buffer(Sc::Uniform | Sc::StorageBuffer, block, set, binding)` (`Block` am Struct,
  `DescriptorSet` + `Binding` an der Variable), `push_constant(block)`, `workgroup(ty)`,
  `resource(ty, set, binding)` (Bilder und Sampler in `UniformConstant`).
- **Namen und Dekorationen**: `name`, `member_name`, `decorate`, `member_decorate` (gleiche
  Dekorationen werden einmal geschrieben); `no_contraction = true` dekoriert jedes
  Gleitkomma-Add/Sub/Mul/Div/Rem und jedes Produkt mit `NoContraction` (kein FMA).
- **Entry Points**: `entry_point(Model, func, name)`, `exec_mode(func, Mode, args)`; das
  Interface berechnet `finish()` aus den Funktionen.
- **Funktionen und Blöcke**: `begin_function(ret, params) -> (id, params)` (öffnet den
  Eintrittsblock), `begin_function_id` (für vorab vergebene Ids), `end_function`, `label`,
  `begin_block`, `block_open`, `local(ty, init)` (Function-Variable, landet am Anfang des
  Eintrittsblocks), `id()` (freie Id für Vorwärtsbezüge).
- **Kontrollfluss**: `branch`, `branch_cond`, `selection_merge`, `loop_merge`, `switch`, `ret`,
  `ret_value`, `unreachable`, `discard`, `demote`, `phi`; strukturiert: `if_then`, `if_else`,
  `loop_(cond, body, cont)` (der Body bekommt `Loop { merge, cont }` für break / continue).
- **Instruktionen** (Ergebnistyp wird aus den Operanden abgeleitet): `fadd fsub fmul fdiv fmod
  fneg`, `iadd isub imul udiv sdiv umod srem ineg`, `band bor bxor bnot shl shr sar`,
  `vec_scale mat_scale mat_vec vec_mat mat_mat transpose dot`, Vergleiche `cmp(op, a, b)` und
  `feq fne flt fle fgt fge ieq ine ult ule uge slt sge`, `land lor lnot any all select`,
  Konvertierungen `convert(op, ty, a)`, `u2f s2f f2u f2s bitcast`, `access` / `access_c`
  (Access Chains), `load`, `store`, `construct`, `extract`, `insert`, `shuffle`,
  `extract_dynamic`, GLSL.std.450 über `ext(ty, inst, args)` / `glsl(inst, args)` / `length`,
  Atomics `atomic(op, ptr, scope, semantics, value)`, `atomic_load`, `atomic_store`,
  `atomic_cas`, Barrieren `control_barrier`, `memory_barrier`, Bilder `sampled_image`, `sample`,
  `sample_lod`, `fetch`, `image_read`, `image_write` (setzen die *WithoutFormat-Capabilities bei
  `Fmt::Unknown`), `image_size`, `image_size_lod` (`ImageQuery`), Ableitungen `deriv(op, a)`,
  `dpdx`, `dpdy`, `fwidth` (Fine / Coarse setzen `DerivativeControl`), `call`; für alles andere
  `emit(opcode, ty, operands)`.

Ids werden in Aufrufreihenfolge vergeben, die Ausgabe ist deterministisch (gleiche Aufrufe,
gleiche Wörter). Der Builder prüft Typen nur so weit, wie er Ergebnistypen ableiten muss; falsche
Benutzung (Instruktion nach dem Terminator, Block ohne Terminator) endet mit einem Panic, alles
andere findet der Validator.

`spv` enthält die Opcodes (`op::FAdd` …), die Enums `Cap`, `Sc`, `Dec`, `Bi`, `Model`, `Mode`,
`Dim`, `Fmt`, `Lang` mit Namenstabellen, `scope::*`, `sem::*`, `glsl::*` und die
Operanden-Grammatik (`OPS`), die Disassembler und Validator teilen; `parse` zerlegt Wörter in
Instruktionen und Operanden.

## Layout-Helfer (`layout`)

`Rule::Std140` (Uniform-Buffer: erweiterte Ausrichtung, Arrays / Structs / Matrizen auf 16 Byte)
und `Rule::Std430` (Storage-Buffer, Push-Konstanten: Basisausrichtung) nach Vulkans „Offset and
Stride Assignment“: `align`, `size`, `stride(elem)`, `matrix_stride(mat)`, `offsets(members)` –
über jede Typtabelle (`Types`: der Builder, der Validator). Matrizen sind column-major,
Laufzeit-Arrays haben keine Größe (letztes Member eines Blocks), `bool` hat kein Layout.

| Fall | std430 | std140 |
|---|---|---|
| `vec3; float` | 0, 12 (16 Byte) | 0, 12 |
| `float; vec3` | 0, 16 | 0, 16 |
| `float[4]` | Stride 4 | Stride 16 |
| `vec3[]` | Stride 16 | Stride 16 |
| `mat4[3]` | Stride 64, 192 Byte | ebenso |
| `mat2` | MatrixStride 8, 16 Byte | MatrixStride 16, 32 Byte |
| `float; struct { vec2; float }; float` | 0, 8, 24 (32 Byte) | 0, 16, 32 (48 Byte) |

## Disassembler (`dis`)

`dis::disassemble(words) -> Result<String, String>` schreibt Text wie `spirv-dis`: Kopfzeilen
(`; SPIR-V`, Version, Generator, Bound, Schema), Ergebnis-Ids rechtsbündig vor Spalte 15,
Friendly Names wie im FriendlyNameMapper von SPIRV-Tools (OpName zuerst, dann `void`, `uint`,
`v4float`, `mat4v4float`, `_arr_float_uint_4`, `_runtimearr_X`, `_ptr_StorageBuffer_X`,
`_struct_N`, Konstanten `uint_4`, `int_n1`, `float_0_5`, `true`; doppelte Namen bekommen `_0`,
`_1` …), Enums und Masken mit Namen, Gleitkommazahlen wie `%.9g` (`dis::fmt_float`; inf / NaN
hexadezimal). Für alle zwölf Benchmark-Module ist die Ausgabe Zeile für Zeile gleich der von
`spirv-dis` aus SPIRV-Tools v2026.4 (Vulkan SDK 1.4.363.0). Unbekannte Opcodes erscheinen als
`OpUnknown(N)` mit ihren Wörtern.

## Validator (`val`)

`val::validate(words) -> Result<(), Vec<Error>>`, `Error { rule, at, msg }` (`at`: Wort-Offset
der Instruktion). Er prüft, was ein Modul-Builder falsch machen kann – kein vollständiger Ersatz
für `spirv-val`.

| Regel | Was geprüft wird |
|---|---|
| `Header` | Magic, Version 1.0 bis 1.6, Schema 0, jede Id unter dem Bound |
| `Grammar` | bekannte Opcodes, Operanden passen zur Grammatik |
| `Section` | logische Reihenfolge der Abschnitte, genau ein `OpMemoryModel`, nichts Modulweites in Funktionen, keine Funktion ohne `OpFunctionEnd` |
| `IdUnique` | jede Ergebnis-Id einmal definiert |
| `IdDefined` | jede benutzte Id definiert, vor ihrer Benutzung (außer Namen, Dekorationen, Entry Points, Execution Modes, Labels, Phis, aufgerufene Funktionen); funktionslokale Ids nur in ihrer Funktion |
| `IdKind` | Ergebnistypen sind Typen, Sprungziele Labels, Operanden Werte, Callees Funktionen, `OpExtInst`-Sets Importe |
| `Type` | Typen wohlgeformt (Vektor 2–4, Matrix aus Float-Vektoren, Array-Längen konstant ≥ 1, Laufzeit-Arrays zuletzt) und Typ-Übereinstimmung für Arithmetik, Bit-Operationen, Shifts, Produkte, Vergleiche, Logik, Select, Konvertierungen, Access Chains, Load / Store, Composites, Shuffles, GLSL.std.450, Atomics, Barrieren, Bilder, Ableitungen, Aufrufe, Phis, Returns, Konstanten, Variablen-Initialisierer, Funktionsparameter |
| `Decoration` | Ziel passt zur Dekoration (`Block` an Structs, `ArrayStride` an Arrays, `Offset` / `MatrixStride` nur an Membern, `Location` / `Flat` an Input / Output, `Binding` / `DescriptorSet` an Ressourcen, `NoContraction` an Ergebnissen), Member-Indizes |
| `Storage` | Speicherklasse der Variable = die ihres Pointers, Function-Variablen zuerst im Eintrittsblock, Vulkans Regeln: Block-Structs in Uniform / StorageBuffer / PushConstant, `DescriptorSet` + `Binding` an Ressourcen, keine an Push-Konstanten, `Location` oder `BuiltIn` an Input / Output, nur Bilder / Sampler in UniformConstant, kein Store in Input / Uniform / PushConstant / UniformConstant, Atomics nur auf Buffer-, Workgroup- und Image-Speicher, höchstens eine Push-Konstanten-Variable je Entry Point |
| `BlockLayout` | `Offset` an jedem Block-Member, ausgerichtet nach std140 (Uniform) bzw. std430 (StorageBuffer, PushConstant), keine Überlappung; `ArrayStride` / `MatrixStride` vorhanden, Vielfache der Ausrichtung und nicht kleiner als das Element; `ColMajor` / `RowMajor` an Matrizen; Laufzeit-Arrays nur zuletzt im äußeren Block; Push-Konstanten höchstens 128 Byte |
| `Cfg` | Blöcke beginnen mit `OpLabel` und enden mit genau einem Terminator; Merge-Instruktionen direkt vor ihrem Branch (`OpSelectionMerge` vor `OpBranchConditional` / `OpSwitch`, `OpLoopMerge` vor `OpBranch` / `OpBranchConditional`); Merge- und Continue-Ziele sind Blöcke der Funktion, ein Merge-Block gehört zu höchstens einem Header und steht nach ihm; bedingte Sprünge ohne Merge nur als break / continue; Rückwärtskanten nur zu Loop-Headern, jede Schleife hat eine; der Eintrittsblock ist kein Sprungziel; Phis am Blockanfang mit Vorgängern als Eltern |
| `Interface` | Entry Point ist eine void-Funktion ohne Parameter; Interface nur aus globalen Variablen, vor 1.4 nur Input / Output, ab 1.4 jede höchstens einmal; es enthält jede globale Variable des Aufrufbaums (ab 1.4) bzw. jede Input- / Output-Variable (vor 1.4); keine zwei Entry Points gleichen Namens und Modells |
| `ExecMode` | `LocalSize` (oder `LocalSizeId` / WorkgroupSize-Built-in) für GLCompute, `OriginUpperLeft` und kein `OriginLowerLeft` für Fragment, Modes nur für passende Modelle, `LocalSize` ≥ 1, `DepthReplacing` wenn `FragDepth` geschrieben wird, fragmentexklusive Instruktionen (discard, demote, Ableitungen, Sampling mit impliziter LOD) nur unter Fragment-Entry-Points, Workgroup-Variablen nur unter Compute |
| `Capability` | Capabilities, die Typen (Int8/16/64, Float16/64), Instruktionen (DerivativeControl, ImageQuery, DemoteToHelperInvocation, Int64Atomics, *WithoutFormat), Built-ins (ClipDistance, CullDistance, DrawParameters, SampleRateShading, MultiView), Bildtypen und Modelle brauchen (`Shader` impliziert `Matrix`) |
| `Version` | `OpTerminateInvocation` / `OpDemoteToHelperInvocation` vor 1.6 nur mit ihrer Extension, `OpCopyLogical` und `OpSelect` auf Composites erst ab 1.4, StorageBuffer vor 1.3 nur mit Extension |

Nicht geprüft (dafür ist `spirv-val` zuständig): Dominanz im eigentlichen Sinn (nur „vorher
definiert“ in Instruktionsreihenfolge), die vollständigen Regeln strukturierter Konstrukte
(Verschachtelung, Ausgänge außer break / continue, Switch-Fallthrough, Header-Dominanz), Anzahl
der Phi-Eltern gleich Anzahl der Vorgänger, Typregeln der Opcodes, die der Builder nicht schreibt
(Gruppen-/Subgroup-Operationen, Bitfelder, Kernel), Typ und Speicherklasse der Built-ins
(`Position` als vec4-Output …), überlappende Locations / Components und das Zusammenpassen der
Stufen, Vulkan-Memory-Model, relaxed / scalar Block Layout (der Validator verlangt die strenge
Ausrichtung), Gerätegrenzen außer den 128 Byte Push-Konstanten, Spezialisierungskonstanten,
Regeln einzelner Extensions.

## Benchmark-Vertrag (`bench`)

Eine GPU-getriebene Szene aus 100 000 bis 1 000 000 instanzierten Würfeln: `move` bewegt jedes
Objekt deterministisch aus Zeit und Seed, `cull` testet seine Bounding-Sphere gegen sechs
Ebenen und hängt sichtbare Indizes an eine Liste, `vs_instanced` zeichnet alle sichtbaren mit
einem indirekten Draw, `vs_single` ist der Vergleichspfad mit einem Draw pro Objekt,
`fs_lambert` schattiert.

Jeder Shader: `Shader::X.build(minor)` bzw. `move_spv(minor)`, `cull_spv`, `cull_reset_spv`,
`vs_instanced_spv`, `vs_single_spv`, `fs_lambert_spv`; Entry Point `"main"`; einzige Capability
`Shader`; `move` und `fs_lambert` importieren GLSL.std.450 (Floor bzw. Normalize, FMax). Beide
Stufen (1.3 und 1.6) erzeugen denselben Code, nur die Interfaces unterscheiden sich.

| Shader | Modell | Workgroup | liest | schreibt |
|---|---|---|---|---|
| `move` | GLCompute | 256 × 1 × 1 | Push | Objects |
| `cull_reset` | GLCompute | 1 × 1 × 1 | – | DrawCmd |
| `cull` | GLCompute | 256 × 1 × 1 | Camera, Objects, Push | Visible, DrawCmd.instance_count (atomar) |
| `vs_instanced` | Vertex | – | Camera, Objects, Visible, Cube | Position, Location 0/1 |
| `vs_single` | Vertex | – | Camera, Objects, Cube, Push.object_index | Position, Location 0/1 |
| `fs_lambert` | Fragment | – | Camera, Location 0/1 | Location 0 |

### Descriptor Set 0

Ein Layout für alle Pipelines, Stage-Flags `COMPUTE | VERTEX | FRAGMENT`:

| Binding | Konstante | Typ | Inhalt | Usage-Flags |
|---|---|---|---|---|
| 0 | `B_CAMERA` | UNIFORM_BUFFER | `Camera` (std140, 176 Byte) | UNIFORM, TRANSFER_DST |
| 1 | `B_OBJECTS` | STORAGE_BUFFER | `Object[count]` (std430, 48 Byte Stride) | STORAGE |
| 2 | `B_VISIBLE` | STORAGE_BUFFER | `uint[count]` | STORAGE |
| 3 | `B_DRAW` | STORAGE_BUFFER | `DrawCmd` (20 Byte) | STORAGE, INDIRECT, TRANSFER_DST |
| 4 | `B_CUBE` | STORAGE_BUFFER | `CubeVertex[24]` (32 Byte Stride) | STORAGE |

Dazu ein Index-Buffer mit `cube_indices()` (36 × `u16`, `VK_INDEX_TYPE_UINT16`). Die
Vertex-Shader lesen ihre Storage-Buffer nur (`NonWritable`), `vertexPipelineStoresAndAtomics`
wird nicht gebraucht; auch `multiDrawIndirect`, `drawIndirectFirstInstance` und
`shaderDrawParameters` nicht.

`apps/gputest` nimmt statt des einen Layouts zwei mit denselben Nummern: eines für die
Rechen-Pipelines (Bindungen 0 bis 3, 1 bis 3 beschreibbar) und eines für die Render-Pipelines
(0, 1, 2, 4, die Speicherpuffer nur lesend). So braucht kein Vertex-Shader eine beschreibbare
Bindung (`Features::vertex_stores`), wie unter WebGPU, wo ein Speicherpuffer im Vertex-Shader
nur lesend gebunden werden darf.

### Datensätze (Rust-Structs mit `bytes()` / `from_bytes()`)

`Object` – std430, 48 Byte, `OBJECT_OFF`:

| Offset | Feld | Typ |
|---|---|---|
| 0 | `pos` | vec3 (Weltposition) |
| 12 | `scale` | float (halbe Kantenlänge) |
| 16 | `rot` | vec4 (Einheitsquaternion x, y, z, w) |
| 32 | `color` | vec3 |
| 44 | `radius` | float (Bounding-Sphere = scale · √3) |

`Camera` – std140, 176 Byte, `CAMERA_OFF`:

| Offset | Feld | Typ |
|---|---|---|
| 0 | `view_proj` | mat4, column-major, MatrixStride 16; `clip = view_proj · vec4(world, 1)` |
| 64 | `planes[6]` | vec4, ArrayStride 16; (n, d) mit \|n\| = 1, innen wenn n·p + d ≥ 0 |
| 160 | `light_dir` | vec3 (zum Licht, normiert) |
| 172 | `ambient` | float |

`DrawCmd` – std430, 20 Byte = `VkDrawIndexedIndirectCommand`: `index_count` 0, `instance_count`
4, `first_index` 8, `vertex_offset` (int) 12, `first_instance` 16.

`CubeVertex` – std430, 32 Byte: `pos` vec4 (w = 1) bei 0, `normal` vec4 (w = 0) bei 16;
`cube_vertices()` liefert 24 (4 je Seite, Würfel −1 … 1), `cube_bytes()` die 768 Byte.

`Push` – Push-Konstanten, std430, 20 Byte (≤ 128), ein Bereich für alle Stufen bei Offset 0:

| Offset | Feld | benutzt von |
|---|---|---|
| 0 | `time` (float, Sekunden) | move |
| 4 | `count` (uint) | move, cull |
| 8 | `seed` (uint) | move |
| 12 | `extent` (float, Szenenradius; `extent_for(count)`) | move |
| 16 | `object_index` (uint) | vs_single |

### Ablauf je Frame (instanzierter Pfad)

1. `vkCmdPushConstants` (alle drei Stufen, 20 Byte), `move` mit `ceil(count / 256)` Gruppen.
2. Zurücksetzen: `cull_reset` (1 Gruppe) – oder `DrawCmd::reset()` einmal hochladen und je Frame
   `vkCmdFillBuffer(draw, 4, 4, 0)`.
3. Barriere Compute/Transfer-Schreiben → Compute-Lesen/Schreiben (Objects, DrawCmd).
4. `cull` mit `ceil(count / 256)` Gruppen.
5. Barriere `COMPUTE_SHADER / SHADER_WRITE` → `DRAW_INDIRECT / INDIRECT_COMMAND_READ` und
   `VERTEX_SHADER / SHADER_READ`.
6. Render-Pass: Pipeline aus `vs_instanced` + `fs_lambert` ohne Vertex-Input-State,
   `TRIANGLE_LIST`, Index-Buffer, `vkCmdDrawIndexedIndirect(draw, 0, 1, 20)`.
7. Vor dem nächsten `move` / Reset: Barriere vom Vertex-Lesen bzw. Indirect-Lesen zum
   Compute-Schreiben.

Vergleichspfad: `vs_single` + `fs_lambert`, je Objekt `vkCmdPushConstants(object_index)` und
`vkCmdDrawIndexed(36, 1, 0, 0, 0)` – entweder für alle Objekte oder CPU-getrieben für die von
`cull_ref` über `move_all` ermittelten.

Rasterizer: `Camera::look_at` / `from_view_proj` erzeugen den Clip-Space von `lib/gpu` (wie
WebGPU und Metal): NDC mit y nach oben, Tiefe 0 = near bis 1 = far. Die Matrix kippt nichts;
das Vulkan-Backend von `lib/gpu` kippt den Viewport, so dass Welt-oben auf dem Bildschirm oben
ist. Wer den Vertrag ohne `lib/gpu` direkt auf Vulkan benutzt, braucht denselben gekippten
Viewport (negative Höhe). Die sechs Ebenen sind links, rechts, unten, oben, near, far.
Würfeldreiecke sind von außen gesehen gegen den Uhrzeigersinn und bleiben es in NDC (y oben):
Vorderseiten wie die Voreinstellung von `lib/gpu` (`front_cw: false`, unter Vulkan mit dem
gekippten Viewport `frontFace = COUNTER_CLOCKWISE`), `cull: Cull::Back`, Tiefentest `LESS`,
Depth-Clear 1.0. Farbausgang Location 0 (vec4, Alpha 1). `apps/gputest` prüft Lage und Windung
auf der GPU an einem gerenderten Bild (`gputest --test scene`, docs/GPU.md „Messszene“).

### Bewegung, Culling und die CPU-Referenzen

Für Objekt i: `h1 = hash(i ^ seed)`, `h2 … h8` je `hash` des vorigen (PCG-Hash),
`f_k = unit(h_k)` (obere 24 Bit · 2⁻²⁴, exakt); Orbitradius `extent · (0.1 + 0.9 f1)`, Höhe
`(extent · 0.25) · (f2 − 0.5)`, Winkel in Umdrehungen `f4 + (f3 − 0.5) · 0.2 · time`, Größe
`0.25 + 0.5 f5`, Drehung aus zwei Halbwinkeln `f6 + (f6 − 0.5) · time` und
`f7 + (f7 − 0.5) · time` als `q = (cy·sx, sy·cx, −sy·sx, cy·cx)`, Farbe aus den Bytes von `h8`.
Sinus und Kosinus berechnet `sincos_turns` ohne transzendente Instruktionen: Reduktion
`r = u − floor(u + 0.5)`, Taylor-Polynome für sin / cos von πr, Doppelwinkelformeln.

`move` und `cull` benutzen nur Addition, Subtraktion, Multiplikation, `Floor`, Konvertierungen
ganzer Zahlen unter 2²⁴ und Ganzzahl-Operationen, jedes Gleitkommaergebnis trägt
`NoContraction`, die Reihenfolge ist die der Referenzen `move_ref` / `move_all` und `visible` /
`cull_ref`. Vulkan verlangt für Add / Sub / Mul korrekte Rundung, also müssen die GPU-Ergebnisse
**bitgleich** sein (Ausnahme: Denormale dürfen auf 0 fallen). Die sichtbare Liste kommt in der
Reihenfolge der Atomics – zum Vergleich sortieren; `instance_count` = Länge von `cull_ref`.
Der Vertex- und der Fragment-Shader sind nicht exakt (Matrixprodukte, `Normalize`).

## Tests: `apps/shadertest`

`python -u build_rs.py test shadertest` (Windows und Linux) bzw.
`build/win/shadertest.exe [--only GRUPPE]`:

- `golden`: das kleinste Modul (leerer GLCompute-Entry-Point) Wort für Wort gegen die Spezifikation;
  `.spv`-Bytes, `from_bytes` auch Big-Endian.
- `builder`: Deduplikation, automatische Capabilities und Extensions, discard / demote je
  Version, Interfaces 1.3 gegen 1.6, `NoContraction` nur in `move` / `cull`, ein Modul mit
  Bildern, Atomics, Barriere, Schleife mit continue.
- `dis`: das kleinste Modul als Text, Gleitkomma-Formatierung, alle Benchmark-Shader gegen
  `apps/shadertest/golden/*.spvasm` (neu schreiben: `shadertest --write DIR`, dann die
  `.spvasm`-Dateien nach `golden/` kopieren).
- `layout`: Offsets, Strides und Größen typischer Structs, die Benchmark-Datensätze, was
  `ty_struct_layout` dekoriert.
- `validate`: alle Benchmark-Shader beider Stufen gültig; 34 kaputte Module, mindestens eines je
  Regel, jedes von genau der erwarteten Regel abgelehnt.
- `refs`: Hash-Werte, `floor`, `sincos_turns` gegen f64, Eigenschaften von `move_ref`,
  Datensätze, Würfel-Windung, Kamera (NDC y oben, rechts / oben, Windung der Würfelseiten in NDC),
  `visible` gegen einen Clip-Space-Test, `cull_ref`.
- `interp`: ein kleiner SPIR-V-Interpreter (nur in der Test-App) führt die Benchmark-Shader auf
  der CPU aus: `move` und `cull` gleich `move_ref` / `cull_ref` Bit für Bit (3000 Objekte),
  `cull_reset`, Vertex-Shader (Position, Normale, Farbe) und `fs_lambert` gegen f64-Rechnung.
- `determinism`: jeder Shader zweimal und in anderer Reihenfolge gebaut, gleiche Wörter.
- `sdk`: wenn das Vulkan SDK da ist (`--sdk DIR`, `%VULKAN_SDK%\Bin`, das neueste
  `C:\VulkanSDK\*\Bin`, `PATH`): `spirv-val --target-env vulkan1.1` (1.3-Module) bzw.
  `vulkan1.3` (1.6-Module) akzeptiert jedes gebaute Modul und lehnt 33 der 34 kaputten ab (die
  128-Byte-Grenze für Push-Konstanten prüft `spirv-val` nicht); `spirv-dis` und
  `dis::disassemble` werden verglichen (Unterschiede werden gemeldet, nicht als Fehler gezählt).
  Ohne SDK meldet die Gruppe, dass sie übersprungen wurde. Das SDK ist nur Testwerkzeug; die
  Crate braucht es nicht.

## Was fehlt

Mesh- und Task-Shader (`MeshEXT` / `TaskEXT` sind nur als Modelle bekannt), Subgroup-Operationen,
Spezialisierungskonstanten, `OpTypeForwardPointer` / Physical Storage Buffer, Row-Major-Layouts
im Layout-Helfer, Switch-Literale breiter als 32 Bit, MSL-Ausgabe, der WGSL-Compiler selbst (G2);
der Validator deckt die oben genannten Lücken nicht ab.

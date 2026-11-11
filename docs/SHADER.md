# lib/shader – SPIR-V erzeugen, lesen und prüfen

`lib/shader` ist das Back-End des späteren WGSL-Compilers (Schritt G2) und schon jetzt direkt aus
Rust benutzbar: ein Modul-Builder, der SPIR-V-Wörter schreibt, ein Disassembler im Stil von
`spirv-dis`, ein struktureller Validator, ein Layout-Helfer für std140 / std430, `reflect`
(Entry Points, Spezialisierungskonstanten und ihre Werte, Workgroup-Größen), die Shader der
GPU-Benchmark-Szene mit ihrem Datenvertrag und CPU-Referenzen und kleine Beispiel-Shader für die
Tests (`samples`). Die Crate ist `no_std` + `alloc`, ohne Abhängigkeiten und ohne GPU testbar
(`apps/shadertest`).

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
- **Spezialisierungskonstanten** (WGSL `override`, unten mehr): `spec_u32`, `spec_i32`,
  `spec_f32`, `spec_bool`, `spec_scalar(ty, spec_id, words)` (`OpSpecConstant*` mit `SpecId`, nie
  dedupliziert), `spec_composite`, `spec_op(ty, opcode, ids, literals)`
  (`OpSpecConstantOp`), `is_spec`; Workgroup-Größe: `local_size_id(func, [x, y, z])`,
  `workgroup_size_builtin([x, y, z])`, `workgroup_size(func, [x, y, z])`; Arrays mit
  Override-Länge: `ty_array_id(elem, len)`.
- **Globale Variablen**: `global(sc, ty, init)`, `input(ty, location)`, `output(ty, location)`,
  `builtin(sc, ty, Bi)` (setzt `ClipDistance`, `DrawParameters`, `SampleRateShading` …),
  `buffer(Sc::Uniform | Sc::StorageBuffer, block, set, binding)` (`Block` am Struct,
  `DescriptorSet` + `Binding` an der Variable), `push_constant(block)`, `workgroup(ty)`,
  `resource(ty, set, binding)` (Bilder und Sampler in `UniformConstant`).
- **Namen und Dekorationen**: `name`, `member_name`, `decorate`, `member_decorate` (gleiche
  Dekorationen werden einmal geschrieben); `no_contraction = true` dekoriert jedes
  Gleitkomma-Add/Sub/Mul/Div/Rem und jedes Produkt mit `NoContraction` (kein FMA).
- **Entry Points**: `entry_point(Model, func, name)`, `exec_mode(func, Mode, args)`,
  `exec_mode_id(func, Mode, ids)` (`OpExecutionModeId`); das Interface berechnet `finish()` aus
  den Funktionen.
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
  `sample_lod`, `sample_dref` / `sample_dref_lod` (Tiefenvergleich), `fetch`, `image_read`,
  `image_write` (setzen die *WithoutFormat-Capabilities bei `Fmt::Unknown`), `image_size`,
  `image_size_lod`, `image_levels` (`ImageQuery`), `nonuniform` (`NonUniform` und
  `ShaderNonUniform`; `resource` mit Laufzeit-Array setzt `RuntimeDescriptorArray`, vor 1.5 mit
  `SPV_EXT_descriptor_indexing`), Ableitungen `deriv(op, a)`,
  `dpdx`, `dpdy`, `fwidth` (Fine / Coarse setzen `DerivativeControl`), `call`; für alles andere
  `emit(opcode, ty, operands)`.

Ids werden in Aufrufreihenfolge vergeben, die Ausgabe ist deterministisch (gleiche Aufrufe,
gleiche Wörter). Der Builder prüft Typen nur so weit, wie er Ergebnistypen ableiten muss; falsche
Benutzung (Instruktion nach dem Terminator, Block ohne Terminator) endet mit einem Panic, alles
andere findet der Validator.

`spv` enthält die Opcodes (`op::FAdd` …), die Enums `Cap` (auch die 16-Bit-Speicher-Capabilities
`StorageBuffer16BitAccess`, `UniformAndStorageBuffer16BitAccess`, `StoragePushConstant16`,
`StorageInputOutput16`), `Sc`, `Dec`, `Bi`, `Model`, `Mode`,
`Dim`, `Fmt`, `Lang` mit Namenstabellen, `scope::*`, `sem::*`, `glsl::*` und die
Operanden-Grammatik (`OPS`), die Disassembler und Validator teilen; `parse` zerlegt Wörter in
Instruktionen und Operanden.

## Spezialisierungskonstanten (WGSL `override`)

Ein Override wird eine skalare Spezialisierungskonstante mit `SpecId` = der Konstantennummer der
Pipeline (`lib/gpu`: `Constant { id, value }`, docs/GPU.md „Pipeline-Konstanten“); ihr Wert im
Modul ist der Standardwert. Abgeleitete Overrides werden `OpSpecConstantOp`, aber SPIR-V erlaubt
unter `Shader` nur ganzzahlige Arithmetik, Shifts, Bit- und Logik-Operationen, ganzzahlige
Vergleiche, `Select`, `SConvert` / `UConvert` (ab 1.4) / `FConvert`, `QuantizeToF16`,
`VectorShuffle`, `CompositeExtract` / `CompositeInsert` -- **keine Gleitkomma-Arithmetik und
keine eingebauten Funktionen**. Für solche Override-Ausdrücke muss der WGSL-Übersetzer (G2c) den
Wert beim Anlegen der Pipeline selbst ausrechnen, wie Tint es tut.

Workgroup-Größe aus Overrides (`@workgroup_size(n)` mit einem Override `n`):

| Stufe | Weg | Bedingung |
| --- | --- | --- |
| Basis (SPIR-V 1.3) | `WorkgroupSize`-Built-in an einem `OpSpecConstantComposite` uvec3 (`workgroup_size_builtin`) | gilt für **jeden** GLCompute-Entry-Point des Moduls und hat Vorrang vor `LocalSize`: ein Modul je Entry Point |
| Voll (SPIR-V 1.6) | `OpExecutionModeId … LocalSizeId %x %y %z` (`local_size_id`), je Entry Point | Vulkan verlangt `maintenance4` (Kern und Pflicht in Vulkan 1.3); `lib/gpu` meldet `Unsupported` ohne |

`workgroup_size(func, size)` nimmt den Weg der Stufe. Die Teile dürfen gewöhnliche Konstanten
und Spezialisierungskonstanten mischen. Ein Array mit Override-Länge (nur im Workgroup-Speicher)
ist `ty_array_id(elem, spec_konstante)`.

`reflect` liest das alles aus fertigen Wörtern:

- `entry_points(words)`: Modell, Name, Funktions-Id je `OpEntryPoint`.
- `spec_constants(words)`: je `SpecId` die Konstante, ihre Art (`Kind::Bool`, `Int { width,
  signed }`, `Float { width }`) und die Bits des Standardwerts.
- `constant_values(words, set)`: die Wörter jeder Konstanten auf Modulebene, wenn die Pipeline
  die Spezialisierungskonstanten in `set` (`(SpecId, Bits)`) setzt; Komposita und jede erlaubte
  `OpSpecConstantOp`-Operation werden ausgewertet. `Err` für das, was SPIR-V undefiniert lässt
  (Division durch 0, Shift um die Bitbreite oder mehr) und für Operationen, die unter `Shader`
  nicht erlaubt sind.
- `workgroup_size(words, entry, set)`: Built-in vor `LocalSizeId` vor `LocalSize`.
- `uses_local_size_id(words)`, `f16_from_f32` / `f16_to_f32` (round to nearest even).

`lib/gpu` prüft damit die Konstanten einer Pipeline gegen das Modul und die spezialisierte
Workgroup-Größe gegen die Grenzen; der Interpreter von shadertest führt spezialisierte Module aus.

## Beispiel-Shader (`samples`)

Kleine Module mit CPU-Referenzen, die shadertest prüft (Validator, `spirv-val`, `spirv-dis`,
Interpreter) und gputest auf der GPU laufen lässt (docs/GPU.md „Tests“):

| Funktion | Modell | prüft |
| --- | --- | --- |
| `spec_fill(minor)` | GLCompute | fünf Overrides (u32, u32, i32, f32, bool) mit `SpecId` 0 … 4, die Workgroup-Größe aus `SPEC_WG` (Basis: Built-in, Voll: `LocalSizeId`), ein Workgroup-Array dieser Länge mit Barriere, sieben `OpSpecConstantOp`-Arten; Referenz `spec_fill_ref(&SpecParams, n)` |
| `vs_fullscreen` | Vertex | ein Dreieck über das ganze Ziel, ohne Eingaben |
| `fs_sample` | Fragment | Textur + Sampler, explizite LOD oder implizite (Mip-Auswahl über Ableitungen) |
| `fs_compare` | Fragment | Tiefentextur + Vergleichssampler, `OpImageSampleDrefExplicitLod` / `…ImplicitLod` |
| `cs_fetch(minor, Texel, arrayed, depth)` | GLCompute | `OpImageFetch` jedes Texels einer Mip-Stufe (float / uint / sint, 2D-Array, Tiefe) |
| `cs_sample(minor, SampleDim)` | GLCompute | `OpImageSampleExplicitLod` an Koordinaten aus einem Puffer (2D, 2D-Array, Würfel, Würfel-Array) |
| `cs_storage_write` / `cs_storage_read` / `cs_storage_rw(minor, Fmt)` | GLCompute | Speichertexturen; `storage_texel`, `storage_read`, `storage_rw_ref` sind die Referenzen |
| `cs_query(minor)` | GLCompute | `OpImageQuerySizeLod` (Mip 0 und 1), `OpImageQueryLevels`, `OpImageQuerySize` |
| `cs_bindless_sample()` | GLCompute (1.6) | Texturen und Sampler einer Bindless-Tabelle, nicht einheitlich indiziert (`NonUniform`) |

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
`_struct_N`, Konstanten `uint_4`, `int_n1`, `float_0_5`, `true`, Built-ins ohne OpName wie
`gl_GlobalInvocationID`, `gl_WorkGroupSize`; doppelte Namen bekommen `_0`, `_1` …;
Spezialisierungskonstanten behalten ihre Nummer), Enums und Masken mit Namen,
`OpSpecConstantOp` mit seiner Operation ohne „Op“ (`OpSpecConstantOp %uint IMul %WG %uint_2`),
Gleitkommazahlen wie `%.9g` (`dis::fmt_float`; inf / NaN hexadezimal). Für alle zwölf
Benchmark-Module und alle 35 Beispiel-Module ist die Ausgabe Zeile für Zeile gleich der von
`spirv-dis` aus SPIRV-Tools v2026.4 (Vulkan SDK 1.4.363.0). Unbekannte Opcodes erscheinen als
`OpUnknown(N)` mit ihren Wörtern.

## Validator (`val`)

`val::validate(words) -> Result<(), Vec<Error>>`, `Error { rule, at, msg }` (`at`: Wort-Offset
der Instruktion). Er prüft, was ein Modul-Builder falsch machen kann – kein vollständiger Ersatz
für `spirv-val`. `val::validate_with(words, &val::Options { uniform_standard_layout })` nimmt
Uniform-Blöcke im Storage-Layout an (std430 statt std140, wie mit Vulkans
uniformBufferStandardLayout; `lib/wgsl` braucht das für WGSLs `mat2x2f` in Uniform-Puffern).

| Regel | Was geprüft wird |
|---|---|
| `Header` | Magic, Version 1.0 bis 1.6, Schema 0, jede Id unter dem Bound |
| `Grammar` | bekannte Opcodes, Operanden passen zur Grammatik |
| `Section` | logische Reihenfolge der Abschnitte, genau ein `OpMemoryModel`, nichts Modulweites in Funktionen, keine Funktion ohne `OpFunctionEnd` |
| `IdUnique` | jede Ergebnis-Id einmal definiert |
| `IdDefined` | jede benutzte Id definiert, vor ihrer Benutzung (außer Namen, Dekorationen, Entry Points, Execution Modes, Labels, Phis, aufgerufene Funktionen); funktionslokale Ids nur in ihrer Funktion |
| `IdKind` | Ergebnistypen sind Typen, Sprungziele Labels, Operanden Werte, Callees Funktionen, `OpExtInst`-Sets Importe |
| `Type` | Typen wohlgeformt (Vektor 2–4, Matrix aus Float-Vektoren, Array-Längen: Konstante ≥ 1 oder ganzzahlige Spezialisierungskonstante, Laufzeit-Arrays zuletzt) und Typ-Übereinstimmung für Arithmetik, Bit-Operationen, Shifts, Produkte, Vergleiche, Logik, Select, Konvertierungen, Access Chains, Load / Store, Composites, Shuffles, GLSL.std.450, Atomics, Barrieren, Bilder (Koordinatenlänge, Dref als f32-Skalar, Ergebnis der Dref-Instruktionen, kein Fetch aus Würfeln; die Abfragen: `QuerySizeLod` / `QueryLevels` nur an gesampelten 1D/2D/3D/Würfel-Bildern ohne MS, `QuerySize` an Speicher-, MS-, Rect- und Buffer-Bildern, Komponentenzahl des Ergebnisses), Ableitungen, Aufrufe, Phis, Returns, Konstanten, Spezialisierungskonstanten (`OpSpecConstantOp`: nur die unter `Shader` erlaubten Operationen, nur konstante Operanden, die Typregeln der Operation), Variablen-Initialisierer, Funktionsparameter |
| `Decoration` | Ziel passt zur Dekoration (`Block` an Structs, `ArrayStride` an Arrays, `Offset` / `MatrixStride` nur an Membern, `Location` / `Flat` an Input / Output, `Binding` / `DescriptorSet` an Ressourcen, `NoContraction` an Ergebnissen, `SpecId` an skalaren Spezialisierungskonstanten und jeder Wert einmal, `BuiltIn` an einer Konstanten nur als uvec3-`WorkgroupSize`, `NonUniform` an Werten), Member-Indizes |
| `Storage` | Speicherklasse der Variable = die ihres Pointers, Function-Variablen zuerst im Eintrittsblock, Vulkans Regeln: Block-Structs in Uniform / StorageBuffer / PushConstant, `DescriptorSet` + `Binding` an Ressourcen, keine an Push-Konstanten, `Location` oder `BuiltIn` an Input / Output, nur Bilder / Sampler in UniformConstant, kein Store in Input / Uniform / PushConstant / UniformConstant, Atomics nur auf Buffer-, Workgroup- und Image-Speicher, höchstens eine Push-Konstanten-Variable je Entry Point |
| `BlockLayout` | `Offset` an jedem Block-Member, ausgerichtet nach std140 (Uniform) bzw. std430 (StorageBuffer, PushConstant), keine Überlappung; `ArrayStride` / `MatrixStride` vorhanden, Vielfache der Ausrichtung und nicht kleiner als das Element; `ColMajor` / `RowMajor` an Matrizen; Laufzeit-Arrays nur zuletzt im äußeren Block; Push-Konstanten höchstens 128 Byte |
| `Cfg` | Blöcke beginnen mit `OpLabel` und enden mit genau einem Terminator; Merge-Instruktionen direkt vor ihrem Branch (`OpSelectionMerge` vor `OpBranchConditional` / `OpSwitch`, `OpLoopMerge` vor `OpBranch` / `OpBranchConditional`); Merge- und Continue-Ziele sind Blöcke der Funktion, ein Merge-Block gehört zu höchstens einem Header und steht nach ihm; bedingte Sprünge ohne Merge nur als break / continue; Rückwärtskanten nur zu Loop-Headern, jede Schleife hat eine; der Eintrittsblock ist kein Sprungziel; Phis am Blockanfang mit Vorgängern als Eltern |
| `Interface` | Entry Point ist eine void-Funktion ohne Parameter; Interface nur aus globalen Variablen, vor 1.4 nur Input / Output, ab 1.4 jede höchstens einmal; es enthält jede globale Variable des Aufrufbaums (ab 1.4) bzw. jede Input- / Output-Variable (vor 1.4); keine zwei Entry Points gleichen Namens und Modells |
| `ExecMode` | `LocalSize` (oder `LocalSizeId` mit drei 32-Bit-Ganzzahlkonstanten, gewöhnliche ≥ 1, oder das WorkgroupSize-Built-in) für GLCompute, Id-Modes nur in `OpExecutionModeId` und umgekehrt, `OriginUpperLeft` und kein `OriginLowerLeft` für Fragment, Modes nur für passende Modelle, `LocalSize` ≥ 1, `DepthReplacing` wenn `FragDepth` geschrieben wird, fragmentexklusive Instruktionen (discard, demote, Ableitungen, Sampling mit impliziter LOD) nur unter Fragment-Entry-Points, Workgroup-Variablen nur unter Compute |
| `Capability` | Capabilities, die Typen (Int8/16/64, Float16/64), Instruktionen (DerivativeControl, ImageQuery, DemoteToHelperInvocation, Int64Atomics, *WithoutFormat, GroupNonUniform und seine Vote, Arithmetic, Ballot, Shuffle, ShuffleRelative, Quad), Built-ins (ClipDistance, CullDistance, DrawParameters, SampleRateShading, MultiView, die Subgroup-Built-ins), Bildtypen, Modelle, Laufzeit-Arrays von Ressourcen (RuntimeDescriptorArray) und `NonUniform` (ShaderNonUniform) brauchen (`Shader` impliziert `Matrix`) |
| `Version` | `OpTerminateInvocation` / `OpDemoteToHelperInvocation` vor 1.6 nur mit ihrer Extension, `OpCopyLogical`, `OpSelect` auf Composites und `UConvert` in `OpSpecConstantOp` erst ab 1.4, `OpExecutionModeId` ab 1.2, `OpGroupNonUniform*` ab 1.3, Laufzeit-Arrays von Ressourcen und `NonUniform` vor 1.5 nur mit `SPV_EXT_descriptor_indexing`, StorageBuffer vor 1.3 nur mit Extension |

Nicht geprüft (dafür ist `spirv-val` zuständig): Dominanz im eigentlichen Sinn (nur „vorher
definiert“ in Instruktionsreihenfolge), die vollständigen Regeln strukturierter Konstrukte
(Verschachtelung, Ausgänge außer break / continue, Switch-Fallthrough, Header-Dominanz), Anzahl
der Phi-Eltern gleich Anzahl der Vorgänger, Typregeln der Opcodes, die der Builder nicht schreibt
(Bitfelder, Kernel; bei `OpGroupNonUniform*` nur Scope als Konstante und Ergebnistyp gleich
Wert), Typ und Speicherklasse der Built-ins
(`Position` als vec4-Output …), überlappende Locations / Components und das Zusammenpassen der
Stufen, Vulkan-Memory-Model, relaxed / scalar Block Layout (der Validator verlangt die strenge
Ausrichtung), Gerätegrenzen außer den 128 Byte Push-Konstanten, die Werte von
Spezialisierungskonstanten (ob eine Workgroup-Größe nach der Spezialisierung ≥ 1 ist, prüft
`lib/gpu`), ob `LocalSizeId` auf dem Gerät erlaubt ist (`maintenance4`), Regeln einzelner
Extensions.

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

## MSL für Metal (`msl`, AP6)

`msl::translate(words, &Options)` übersetzt ein SPIR-V-Modul in die Metal Shading Language, für das
Metal-Backend von `lib/gpu` auf macOS und iOS ([APPLE.md](APPLE.md), AP6). So läuft jedes Modul, das
`lib/wgsl` oder dieser Builder schreibt, ohne Änderung des Aufrufers auch auf Metal. Gelesen wird das
SPIR-V dieser beiden Erzeuger: logische Adressierung, strukturierter Kontrollfluss, die Opcodes von
`spv::OPS` und `GLSL.std.450`.

- **Ergebnis:** der MSL-Text und je Entry Point der Name der MSL-Funktion, die Workgroup-Größe (mit
  eingesetzten Konstanten), die Plätze der Ressourcen, ob Push-Konstanten und Pufferlängen gelesen
  werden, die Orte der Vertex-Eingaben.
- **Plätze:** Puffer, Texturen und Sampler des Moduls je ab 0 in der Reihenfolge (Set, Binding), Puffer
  unter `FIRST_VERTEX_SLOT` (21); Push-Konstanten als `constant`-Block an `PUSH_SLOT` (29), die Längen der
  gebundenen Pufferbereiche in Byte als `constant uint*` an `SIZES_SLOT` (30, Index = Platz des Puffers;
  `arrayLength` und die Robustheit lesen sie); Vertex-Puffer i an `vertex_slot(i)` (28 − i).
- **Spezialisierungskonstanten** werden eingesetzt (`Options::spec`, `reflect::constant_values`): eine
  Pipeline mit Konstanten ist eine eigene Übersetzung.
- **Typen:** ein MSL-Struct je SPIR-V-Struct; mit `Offset`-Dekorationen bis zu den Offsets aufgefüllt,
  Dreiervektoren dort als `packed_`-Typen; Arrays als `jf_arr<T, N>` (Wertsemantik), Laufzeit-Arrays als
  `T m[1]`. Strides von Arrays und Matrizen müssen die von MSL sein (sonst ein Fehler).
- **Werte und Kontrollfluss:** jeder SSA-Wert ist eine Variable der Funktion, zugewiesen, wo SPIR-V ihn
  definiert; Zeiger, Texturen und Sampler bleiben Ausdrücke. Strukturierter Kontrollfluss wird `if`,
  `switch` und `while (true)`; ein Sprung zum Continue-Ziel einer Schleife wiederholt das
  Continue-Konstrukt an Ort und Stelle. Ressourcen, Built-ins, private und Workgroup-Variablen sind
  Parameter jeder Funktion, die sie benutzt. Vorzeichenbehaftete Ganzzahl-Arithmetik läuft über
  vorzeichenlose (sie läuft über wie in SPIR-V und WGSL; in MSL wäre ein Überlauf undefiniert).
- **Texturzugriffe:** `textureLoad` klemmt Stufe, Schicht, Probe und Koordinate in die Textur, ein Speichern
  außerhalb fällt weg (Metals `read()`/`write()` außerhalb sind undefiniert, WGSL erlaubt beides); Proben mit
  Gleitkomma-Koordinate runden und klemmen die Schicht.
- **Grenzen:** Farbausgaben an den Orten 0 bis 7 (Metal), keine Laufzeit-Arrays von Ressourcen
  (Bindless), keine 64-Bit-Gleitkommazahlen, keine nicht konstanten Texel-Offsets, kein Sprung aus
  einer Schleife heraus innerhalb eines `switch`.

**Geprüft** von `wgsltest --only msl` (Gruppe `msl`): jedes Modul der Gruppe `spirv` (Benchmark,
Abdeckung, die gültigen Fälle der Prüfregeln, das Korpus), dazu die WGSL-Shader der Bibliotheken
(`gpu_out.wgsl`, render3d) mit und ohne Robustheit und alle Module, die dieser Builder schreibt
(`samples`, `bench`), je auf beiden Stufen: 805 Module, 802 übersetzt, 3 erwartete Ausnahmen
(Farbort 8, Bindless). `--msl-out DIR` schreibt die MSL-Dateien; auf dem Mac nahm Metals Übersetzer zur
Laufzeit alle 802 an (2026-11-10). Ob sie auch richtig rechnen, prüft `gputest` auf Metal (AP6b).

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
- `validate`: alle Benchmark-Shader beider Stufen gültig; 62 kaputte Module, mindestens eines je
  Regel (darunter Spezialisierungskonstanten, `LocalSizeId`, Id-Modes, Bild-Abfragen, Dref,
  Koordinaten, Descriptor-Indexing, Gruppen-Operationen, Subgroup-Built-ins), jedes von genau der
  erwarteten Regel abgelehnt.
- `refs`: Hash-Werte, `floor`, `sincos_turns` gegen f64, Eigenschaften von `move_ref`,
  Datensätze, Würfel-Windung, Kamera (NDC y oben, rechts / oben, Windung der Würfelseiten in NDC),
  `visible` gegen einen Clip-Space-Test, `cull_ref`.
- `spec`: was der Builder je Stufe schreibt (Built-in auf 1.3, `LocalSizeId` auf 1.6),
  `reflect` (Spezialisierungskonstanten von `spec_fill`, Workgroup-Größen mit und ohne
  Pipeline-Konstanten, jede der 39 erlaubten `OpSpecConstantOp`-Operationen gegen Rust-Rechnung,
  die undefinierten Fälle als `Err`), die Halbgenauigkeits-Umrechnung (Grenzfälle, alle 63 490
  Halbwerte außer NaN hin und zurück), der Validator auf allen 35
  Beispiel-Modulen, die Disassembly von `spec_fill` gegen `golden/spec_fill*.spvasm`,
  Determinismus der Beispiele.
- `interp`: ein kleiner SPIR-V-Interpreter (nur in der Test-App) führt die Benchmark-Shader auf
  der CPU aus: `move` und `cull` gleich `move_ref` / `cull_ref` Bit für Bit (3000 Objekte),
  `cull_reset`, Vertex-Shader (Position, Normale, Farbe) und `fs_lambert` gegen f64-Rechnung;
  `spec_fill` beider Stufen mit Standardwerten und drei Sätzen Pipeline-Konstanten
  (Workgroup-Größen 1, 32, 256) gleich `spec_fill_ref` -- eine Workgroup läuft dabei
  abwechselnd bis zur nächsten Barriere, Workgroup-Speicher geteilt.
- `determinism`: jeder Shader zweimal und in anderer Reihenfolge gebaut, gleiche Wörter.
- `sdk`: wenn das Vulkan SDK da ist (`--sdk DIR`, `%VULKAN_SDK%\Bin`, das neueste
  `C:\VulkanSDK\*\Bin`, `PATH`): `spirv-val --target-env vulkan1.1` (1.3-Module) bzw.
  `vulkan1.3` (1.6-Module) akzeptiert jedes gebaute Modul samt den Beispielen und lehnt 59 der 62
  kaputten ab (nicht geprüft von `spirv-val`: die 128-Byte-Grenze für Push-Konstanten, eine
  `SpecId` zweimal, ein Float in `LocalSizeId`); `spirv-dis` und `dis::disassemble` werden für
  die Benchmark- und die Beispiel-Module verglichen (Unterschiede werden gemeldet, nicht als
  Fehler gezählt).
  Ohne SDK meldet die Gruppe, dass sie übersprungen wurde. Das SDK ist nur Testwerkzeug; die
  Crate braucht es nicht.

## WGSL-Referenz: Tint

Der WGSL-Übersetzer muss annehmen, was Chromes Tint annimmt (der Browser bekommt das WGSL
direkt). [`tools/wgsl_ref.mjs`](../tools/wgsl_ref.mjs) lässt das installierte Chrome (headless,
ohne Flags, über Playwright) jede gegebene `.wgsl`-Datei übersetzen und je Entry Point eine
Pipeline anlegen und schreibt Urteil, Meldungen (Zeile, Spalte, Offset, Länge, Text) und
Pipeline-Fehler als JSON. [`tools/wgsl_corpus`](../tools/wgsl_corpus/README.md) sind 932 selbst
geschriebene Fälle zu den Regeln der Spezifikation, `expected.json` ist Tints Urteil darüber
(Chrome 156): Tests vergleichen damit ohne Chrome. Das README des Korpus nennt die Fälle, in
denen Tint anders urteilte als erwartet (etwa: konkretes `i32` läuft auch in
Konstantenausdrücken modulo 2³² über, `01f` ist für Tint ein gültiges Literal).

## Was fehlt

Mesh- und Task-Shader (`MeshEXT` / `TaskEXT` sind nur als Modelle bekannt), geclusterte
Subgroup-Operationen (die anderen schreibt `lib/wgsl` über `Module::emit`),
`OpTypeForwardPointer` / Physical Storage Buffer, Row-Major-Layouts
im Layout-Helfer, Switch-Literale breiter als 32 Bit, MSL-Ausgabe; der Validator deckt die oben
genannten Lücken nicht ab. Der WGSL-Übersetzer, der diese Crate als Back-End benutzt, ist
[`lib/wgsl`](WGSL.md) (G2c).

# lib/wgsl – der WGSL-Übersetzer

`lib/wgsl` übersetzt WGSL-Text in ein geprüftes, vollständig typisiertes Modul: Lexer, Parser,
semantische Prüfung, Konstantenauswertung und ein IR (Schritt G2a), und dieses IR mit dem
[Back-End](#back-end-spir-v-g2c) nach SPIR-V (mit [`lib/shader`](SHADER.md), Schritt G2c), samt
Pipeline-Konstanten und den Prüfungen, die WebGPU beim Anlegen einer Pipeline macht. Im Browser
(G4) bekommt WebGPU denselben Text direkt; deshalb soll das Front-End genau das annehmen, was
Browser annehmen.
Maßstab ist die W3C-Spezifikation (Candidate Recommendation vom 26.10.2026); wo Chrome 156 (Tint)
abweicht, folgt die Crate der Spezifikation, und die Abweichungen stehen
[unten](#abweichungen-zwischen-spezifikation-und-chrome).

Die Crate ist `no_std` + `alloc`, ohne Abhängigkeiten und ohne `unsafe`. Sie gerät bei keiner
Eingabe in Panik: Verschachtelung über den Grenzen ist ein Fehler, nie ein Stapelüberlauf,
Konstanten sind in der Größe begrenzt.

| Datei | Inhalt |
|---|---|
| [`lib.rs`](../lib/wgsl/lib.rs) | API, `Diagnostic`, `Options`, `feature`, `LineMap` |
| [`lex.rs`](../lib/wgsl/lex.rs) | Tokens, Template-Listen, Literalwerte, Rundung |
| [`xid.rs`](../lib/wgsl/xid.rs) | XID_Start / XID_Continue (erzeugt von `tools/gen_xid.py`) |
| [`ast.rs`](../lib/wgsl/ast.rs), [`parse.rs`](../lib/wgsl/parse.rs), [`print.rs`](../lib/wgsl/print.rs) | Syntaxbaum, Parser, Ausgabe als Text |
| [`check.rs`](../lib/wgsl/check.rs) | Modulebene: Direktiven, Reihenfolge, Namen, Typen, Attribute, Deklarationen |
| [`check_expr.rs`](../lib/wgsl/check_expr.rs) | Ausdrücke, Überladungen, Konstruktoren, Aufrufe, Alias-Analyse |
| [`check_stmt.rs`](../lib/wgsl/check_stmt.rs) | Anweisungen, Verhalten (Behaviors), Schleifen-Regeln |
| [`entry.rs`](../lib/wgsl/entry.rs) | Einstiegspunkte: Schnittstelle, Stufen-Regeln, Ressourcen |
| [`types.rs`](../lib/wgsl/types.rs) | Typtabelle, Typeigenschaften, Layout, ConversionRank |
| [`consteval.rs`](../lib/wgsl/consteval.rs), [`builtins.rs`](../lib/wgsl/builtins.rs), [`math.rs`](../lib/wgsl/math.rs) | Konstanten, eingebaute Funktionen, f64-Mathematik |
| [`ir.rs`](../lib/wgsl/ir.rs) | das IR (siehe [Das IR](#das-ir)) |
| [`bench.rs`](../lib/wgsl/bench.rs), [`bench/*.wgsl`](../lib/wgsl/bench) | die sechs Benchmark-Shader als WGSL |
| [`spirv.rs`](../lib/wgsl/spirv.rs), [`spirv_func.rs`](../lib/wgsl/spirv_func.rs), [`spirv_builtin.rs`](../lib/wgsl/spirv_builtin.rs) | das Back-End nach SPIR-V (siehe [Back-End](#back-end-spir-v-g2c)) |
| [`pipeline.rs`](../lib/wgsl/pipeline.rs) | Pipeline-Konstanten, Overrides auswerten, Pipeline-Prüfungen |

## API

```rust
let (ast, diags) = wgsl::parse(src);          // Syntaxbaum + Syntaxfehler (mehrere je Lauf)
let text = wgsl::print(&ast);                 // wieder WGSL; parse(print(ast)) == ast
match wgsl::check(src) {                      // parse + Prüfung mit Options::default()
    Ok(m) => { /* m: ir::Module, Warnungen in m.warnings */ }
    Err(diags) => for d in &diags { println!("{}", d) },   // "3:5: Fehler: …" + Hinweise
}
let m = wgsl::check_with(src, &wgsl::Options { features: wgsl::feature::ALL & !wgsl::feature::F16 });
let (toks, lex_diags) = wgsl::lex::tokens(src); // Tokens (Template-Listen schon erkannt)
let out = wgsl::compile(src, &wgsl::CompileOptions::new(shader::VOLL))?;   // SPIR-V, siehe Back-End
```

`Diagnostic { line, col, end_col, error, msg, start, end, notes }`: Zeile und Spalte 1-basiert,
Spalten zählen Zeichen (wie `lib/script`), `end_col` exklusiv auf derselben Zeile, `start`/`end`
der Bytebereich, `notes` zeigen auf zugehörige Stellen („hier deklariert“, „das erste“). Zeilen
trennt WGSL mit LF, VT, FF, CR, CRLF, NEL, LS und PS; `LineMap` zählt genauso. Die Texte sind
deutsch: „erwartet `;` nach der Anweisung, gefunden `}`“, „`x` ist hier nicht bekannt“, „Typ
`vec3<f32>` passt nicht zu `f32`“, „`&` und `|` dürfen nicht ohne Klammern gemischt werden“.

`Options::features` (`feature::*`): welche Spracherweiterungen die Implementierung unterstützt
(sie gelten wie im Browser auch ohne `requires`) und welche Enable-Erweiterungen das Gerät anbietet
(sie gelten erst nach `enable`). Voreinstellung `feature::ALL` = alles, was die Crate kann:

| Erweiterung | Art | Stand |
|---|---|---|
| `readonly_and_readwrite_storage_textures`, `packed_4x8_integer_dot_product`, `unrestricted_pointer_parameters`, `pointer_composite_access` | Sprache | unterstützt |
| `uniform_buffer_standard_layout`, `texture_and_sampler_let`, `texture_formats_tier1`, `linear_indexing`, `immediate_address_space`, `swizzle_assignment` | Sprache | unterstützt |
| `fragment_depth` | Sprache | unterstützt, aber nicht in `feature::ALL` (Chrome 156 bietet es nur mit `--enable-unsafe-webgpu`) |
| `buffer_view`, `subgroup_id`, `subgroup_uniformity` | Sprache | Fehler „noch nicht unterstützt“ |
| `f16`, `clip_distances`, `dual_source_blending`, `primitive_index`, `atomic_vec2u_min_max` | enable | unterstützt |
| `subgroups`, `subgroup_size_control` | enable | Fehler „noch nicht unterstützt“ |

Chrome 156 meldet in `wgslLanguageFeatures` dieselben Spracherweiterungen (ohne `fragment_depth`)
plus die drei nicht unterstützten.

## Lexer

- Leerraum ist Pattern_White_Space (auch NEL, LRM, RLM, LS, PS), Blockkommentare schachteln,
  U+0000 ist überall ein Fehler, ein BOM ebenso.
- Bezeichner: XID_Start oder `_`, dann XID_Continue (Unicode 15.1 aus Pythons Datenbank, eine
  Tabelle von 1822 Läufen, 7 KB); `_` allein ist das Token Underscore, `__x` ein Fehler, die 148
  reservierten Wörter sind Fehler (der Parser nimmt sie danach als Namen, damit es keine
  Folgefehler gibt).
- Zahlen nach den regulären Ausdrücken von 3.5.2, längster Treffer: `012` ist ein Fehler, `012.5`
  eine Gleitkommazahl, `0x1.5f` hexadezimal ohne Suffix (das `f` ist eine Ziffer), `1e` ist `1`
  gefolgt vom Namen `e`. Werte: ganze Zahlen nach Suffix geprüft (i32, u32, AbstractInt = i64),
  dezimale Gleitkommazahlen über den korrekt rundenden Parser von `core` (f32 direkt, f16 über
  f64), hexadezimale exakt mit eigener Rundung (`round_bits`); `f`/`h`-Hexzahlen müssen exakt
  darstellbar sein, Überlauf ist ein Fehler.
- Template-Listen: der Algorithmus aus 3.9 Zeichen für Zeichen über dem Quelltext
  (`discover_templates`); die gefundenen `<` und `>` werden zu TemplateStart / TemplateEnd, sonst
  gilt der längste Operator (`>>=`, `<<=`, `&&` …). `vec2<vec2<f32>>` schließt zwei Listen.
- Tokens, die die Grammatik kürzer braucht, teilt der Parser: `a--b` ist `a - (-b)`, `--x` ist
  `-(-x)`, `&&p` ist `&(&p)` (Spezifikation 3.1: das längste in diesem Zustand gültige Token).

## Parser

Rekursiver Abstieg nach Kapitel 18 der Spezifikation. Ausdrücke folgen der Grammatik genau, auch
in dem, was sie verbietet: `&`, `|`, `^` nehmen nur unäre Operanden und mischen weder
untereinander noch mit anderen Operatoren, eine Verschiebung nimmt unäre Operanden, Vergleiche
verketten nicht, `&&` und `||` mischen nicht – jeweils mit eigener Meldung. Attribute stehen
überall, wo die Grammatik sie erlaubt (vor Anweisungen nur vor Blöcken, if, switch, loop, for,
while und vor Schleifenrümpfen); welche dort gültig sind, prüft der Checker.

Fehlerbehandlung: ein Fehler beendet die Anweisung; der Parser springt zum nächsten `;` oder
zum `}` derselben Klammerebene (in Funktionen) bzw. zur nächsten Deklaration auf Ebene 0 und
macht weiter. So meldet ein Lauf mehrere Fehler (höchstens `MAX_ERRORS` = 25).

Grenzen (`parse.rs`): Verschachtelung (Klammern, Template-Listen, unäre Operatoren, Blöcke)
höchstens `MAX_DEPTH` = 128 Ebenen, Blöcke in einer Funktion höchstens `MAX_BRACES` = 127
(die Mindestgrenze der Spezifikation; wie Tint), die Höhe eines Ausdrucksbaums höchstens
`MAX_EXPR_DEPTH` = 256 (Operatorketten, Member-Ketten, Aufrufe zählen mit; jeder spätere Durchgang
rekursiert so tief). Gemessen (Linux, Entwicklungs-Build, opt-level 1): der tiefste gültige
Quelltext braucht unter 400 KB Stapel (`wgsltest --stack worst 96 KB`); das Front-End braucht
also einen Thread mit mindestens 512 KB, der Hauptthread (1 MB unter Windows) genügt.

`print(&ast)` schreibt den Baum mit vier Leerzeichen Einzug zurück; Klammern genau da, wo der Baum
`Paren`-Knoten hat, Literale wie geschrieben, Kommentare entfallen. `parse(print(ast))` ergibt
denselben Baum (die Tests vergleichen das zweimalige Ausgeben und das Prüfergebnis).

## Prüfung

- **Reihenfolge**: Zuerst sammelt ein Durchgang je Moduldeklaration die Modulnamen, die sie
  erwähnt (mit den lokalen Gültigkeitsbereichen, damit lokale Namen keine falschen Kanten
  ergeben). Eine iterative Tiefensuche ordnet sie (Aufgerufene vor Aufrufern) und meldet Zyklen
  („zyklische Abhängigkeit: `a` → `b` → `a`“, Rekursion, sich selbst enthaltende Strukturen). Danach
  wird jede Deklaration genau einmal geprüft, ohne von einer Deklaration in eine andere zu
  rekursieren – 20 000 verkettete Konstanten sind kein Problem.
- **Gültigkeit**: Modulnamen gelten im ganzen Modul, lokale ab der nächsten Anweisung bis zum
  Blockende; zwei Deklarationen mit demselben Namen und demselben Ende sind ein Fehler (Parameter
  und Funktionsrumpf, `for`-Initialisierer und Schleifenrumpf teilen sich das Ende), innere Blöcke
  dürfen verdecken, Deklarationen verdecken vordeklarierte Namen (`fn min()`, `const rgba8unorm`).
  Eine fehlgeschlagene lokale Deklaration bleibt als „vergiftet“ deklariert: keine Folgefehler.
- **Typen**: bool, i32, u32, f32, f16 (mit `enable f16`), AbstractInt / AbstractFloat (nur aus
  Literalen und `const`), vecN/matCxR und die Aliasse `vec3f`, `mat4x4h` …, atomic (auch
  `atomic<vec2<u32>>`), Arrays fester, override- und Laufzeit-Größe, Strukturen, `ptr<AS, T, AM>`,
  Referenzen, Swizzle-Views, alle Texturtypen (auch Speichertexturen mit Format und Zugriff,
  Tiefen-, Multisample- und externe Texturen), sampler / sampler_comparison. Eigenschaften nach
  Kapitel 6: konstruierbar, speicherbar, host-teilbar, feste Größe beim Erzeugen bzw. bei der
  Pipeline.
- **Konvertierungen und Überladungen**: ConversionRank aus 6.1.2, Überladungsauflösung aus 6.1.3
  über voll ausgeschriebene Kandidaten (auch für Operatoren); Kandidaten mit abstrakten
  Parametern fallen weg, sobald ein Argument kein const-Ausdruck ist. `let`/`var`/`override`
  konkretisieren (AbstractInt → i32, AbstractFloat → f32), `const` behält den abstrakten Typ.
- **Konstanten**: jeder const-Ausdruck wird ausgewertet. AbstractInt als i64, jeder Überlauf ist
  ein Fehler; AbstractFloat als f64; f32/f16 werden in f64 gerechnet und einmal gerundet (für
  + − × ÷ und sqrt bitgenau, da 53 ≥ 2p + 2); i32/u32 laufen modulo 2³² über, außer Division durch
  null, MIN / −1 und den Schieberegeln. NaN, ±∞ und Werte außerhalb des Zieltyps sind Fehler,
  ebenso Indizes außerhalb der Grenzen. Der rechte Operand von `false && x` / `true || x` wird
  geprüft, aber nicht ausgewertet (Array-Größen darin schon). Alle `@const`-Funktionen der
  Spezifikation sind ausgewertet; die transzendenten Funktionen (`math.rs`, eigene f64-Routinen)
  sind auf wenige ulp genau, für f32/f16 nach dem Runden in der Regel exakt.
- **Override-Ausdrücke** bleiben symbolisch (Override-Knoten im IR); Fehler, die erst ihr Wert
  zeigt (Division durch null, Array-Größe ≤ 0, workgroup_size ≤ 0), sind Pipeline-Fehler und
  hier nicht geprüft.
- **Attribute**: unbekannte und doppelte Attribute, Attribute am falschen Ort; Werte von
  `@group @binding @location @id @align @size @blend_src @workgroup_size` als const-Ausdrücke vom
  Typ i32/u32 (Bereiche, Zweierpotenzen, Vielfache der Ausrichtung); `@builtin` mit Namen und
  Tiefenmodus, `@interpolate` mit erlaubten Kombinationen, `@invariant` nur an `position`,
  `@must_use` nur mit Rückgabetyp, `@const` nie an eigenen Funktionen, `@diagnostic` mit
  Schweregrad und Regel (unbekannte einteilige Regeln: Warnung), widersprüchliche Filter.
- **Variablen und Adressräume**: function/private (konstruierbar, Initialisierer const- oder
  override-Ausdruck bei private), workgroup (feste Größe, override-Größe nur hier), uniform
  (konstruierbar und host-teilbar), storage (host-teilbar, Atomics nur read_write), immediate
  (ohne Arrays), handle (Texturen und Sampler ohne Adressraum); Zugriffsmodus nur bei storage;
  Ressourcen brauchen `@group` und `@binding`, andere dürfen sie nicht haben.
- **Layout** (14.4): Ausrichtung und Größe aller Typen, Offsets mit `@align`/`@size`,
  Array-Stride; die Regeln des Adressraums (storage: Offsets und Strides Vielfache der
  Ausrichtung; uniform ohne `uniform_buffer_standard_layout` zusätzlich 16-Byte-Strides und
  Struct-Abstände). Arrays über 4 GiB sind ein Fehler.
- **Anweisungen**: Verhalten nach 9.7 (fehlendes `return`, Schleifen ohne Ausgang, `continuing`
  ohne `return`/`continue`), `break`/`continue`/`break if` an den richtigen Stellen, ein
  `continue`, das eine in `continuing` benutzte Deklaration überspringt, `switch` (gemeinsamer
  Ganzzahltyp, const-Selektoren, Duplikate, genau ein `default`), Zuweisungen (Referenz mit
  Schreibzugriff, konstruierbarer Typ), Swizzle-Zuweisung ohne doppelte Komponenten, `++`/`--`
  nur auf i32/u32, `_ =` nur für konstruierbare Werte, Zeiger, Texturen, Sampler, `@must_use` und
  Konstruktoren nicht als Aufrufanweisung. Code nach `return`/`break`/`continue` ist eine
  Warnung („unerreichbarer Code“).
- **Aufrufe**: Argumentzahl und -typen, Zeiger exakt (Adressraum, Typ, Zugriff), Einstiegspunkte
  sind nicht aufrufbar; die Alias-Analyse aus 11.4.1 (zwei Zeiger auf denselben Speicher, von
  denen einer beschrieben wird; ein Zeiger auf eine globale Variable, die die Funktion selbst
  liest oder schreibt) über die Lese-/Schreibmengen aller Aufgerufenen.
- **Einstiegspunkte**: eine Stufe je Funktion; Ein- und Ausgaben je Parameter oder Struct-Member
  mit genau einem von `@builtin` / `@location`; Built-ins je Stufe und Richtung mit ihrem Typ;
  Locations numerisch, eindeutig in den Eingaben, nicht bei Compute; ganzzahlige Ein-/Ausgaben
  zwischen den Stufen brauchen `@interpolate(flat)`; Vertex-Shader geben `position` aus;
  `@workgroup_size` (const oder override, alle i32 oder alle u32, const > 0) nur und immer bei
  Compute; was erreichbar ist: `discard`, Ableitungen und `textureSample`/`Bias`/`Compare` nur
  in Fragment-Shadern, Barrieren und `workgroupUniformLoad` nur in Compute, workgroup-Variablen
  nur in Compute, beschreibbare Speicherpuffer und -texturen nicht im Vertex-Shader, höchstens
  eine immediate-Variable, `(group, binding)` eindeutig je Einstiegspunkt.

## Das IR

`ir::Module` ist das Ergebnis von `check`. Es ist so gebaut, dass G2c es direkt absenken kann:

- **Typen**: `types: Vec<Type>`, internalisiert – gleiche Typen haben gleiche `TypeId`. Feste Ids:
  `BOOL` 0, `AINT` 1, `AFLOAT` 2, `I32` 3, `U32` 4, `F32` 5, `F16` 6, `VOID` 7 (Typ eines Aufrufs
  ohne Ergebnis). `Type::Vector(n, Scalar)`, `Matrix(cols, rows, Scalar)`, `Atomic(TypeId)`,
  `Array { elem, count, stride }` mit `ArrayCount::Fixed(n) | Override(o) | Expr(e) | Runtime`
  (`stride` = roundUp(align, size) des Elements), `Struct(i)` (Index in `structs`), `Pointer` /
  `Reference { space, store, access }`, `SwizzleView { space, elem, n, k }`, `Sampler`,
  `Texture(TexDim, TexKind)`. `Module::type_name` schreibt die WGSL-Schreibweise. Abstrakte Typen
  kommen in Funktionen und im Modul-Arena nicht vor.
- **Strukturen**: `Struct { name, members, align, size, runtime_sized, builtin }`, je Member
  `offset`, `size` (`@size` oder die des Typs), `align` (`@align` oder die des Typs) und die
  IO-Attribute (`Io`). Das Layout gilt für jeden Adressraum (WGSL kennt nur eins); `size` einer
  Struktur mit Laufzeit-Array endet vor dessen Elementen. `builtin` markiert die Ergebnisstrukturen
  von `frexp`, `modf` (Member `fract`, `exp` / `whole`) und `atomicCompareExchangeWeak`
  (`old_value`, `exchanged`).
- **Globale Variablen**: `Global { name, space, access, ty, binding, init, std140 }`; `ty` ist der
  Speichertyp, `init` (nur private) ein Knoten in `Module::exprs`, `std140` sagt bei uniform, ob
  der Typ auch die strengen Uniform-Regeln erfüllt (sonst braucht Vulkan das
  uniformBufferStandardLayout-Feature).
- **Overrides**: `Override { name, id: Option<u16>, ty, init }` (konkreter Skalar, `init` in
  `Module::exprs`, darf andere Overrides enthalten). Override-Ausdrücke auf Modulebene
  (Initialisierer, private-Initialisierer, Workgroup-Größen, override-Array-Größen) liegen in
  `Module::exprs`.
- **Funktionen** (`functions`, Aufgerufene vor Aufrufern): `params` (mit IO-Attributen), `ret`,
  `ret_io`, `stage`, `locals` (`Local { ty, kind: Var | Let }`), `exprs` (die Ausdrucks-Arena der
  Funktion), `body`, `calls`, `globals` und `overrides` (benutzt, Aufgerufene eingeschlossen,
  aufsteigend), `uses_discard`.
- **Ausdrücke** `Expr { kind, ty, span }`, jeder mit seinem konkreten Typ:
  - `Const(Value)` – jeder const-Ausdruck ist gefaltet. `Value`: Bool, I32, U32, F32, F16 (als f32
    gehalten, `f16_bits` liefert die Bits), `Comp(Vec<Value>)` (Vektorkomponenten,
    Matrixspalten, Array-Elemente, Struct-Member), `Zero` (Nullwert des Knotentyps, auch für sehr
    große Arrays – als `OpConstantNull` absenkbar).
  - `Override(o)`, `Global(g)`, `Local(l)`, `Param(k)`. `Global` und `Local` einer `var` sind
    Referenzen; `Local` eines `let` ist der Wert (auch ein Zeiger, eine Textur oder ein Sampler).
  - `Load(e)` – die Load-Regel ist explizit: jeder Lesezugriff auf eine Referenz (auch auf
    Textur-/Sampler-Variablen, Typ `ref<handle, …>`) oder einen Swizzle-View ist ein Load.
  - `AddrOf` (Referenz → Zeiger), `Deref` (Zeiger → Referenz). `p.x` auf einem Zeiger
    (pointer_composite_access) erscheint als `Member(Deref(p), 0)`.
  - `Index(base, i)` (i: i32 oder u32) und `Member(base, k)` (Struct-Member oder
    Vektorkomponente): auf einer Referenz ergibt das eine Referenz, auf einem Wert einen Wert.
  - `Swizzle(base, [c0..c3], n)`: auf einem Wert ein Vektor; auf einer read_write-Referenz
    (swizzle_assignment) ein `SwizzleView` – `Load` liest den Vektor, `Assign` / `Compound`
    schreiben die genannten Komponenten (Lesen–Ändern–Schreiben des ganzen Vektors). Ein
    dynamischer Index in einen Swizzle-View ist `Index(vref, Index(Const(array<u32,k>), i))`.
  - `Unary(Neg | Not | BitNot, e)`, `Binary(op, a, b)`: Operanden gleichen Typs, außer Matrix ×
    Skalar, Skalar × Matrix, Matrix × Vektor, Vektor × Matrix, Matrix × Matrix, Gleitkomma-Vektor
    × Skalar in beiden Reihenfolgen (für OpVectorTimesScalar), den Verschiebungen (rechts u32 bzw.
    vecN<u32>) und Vergleichen (Ergebnis bool / vecN<bool>). Sonst gemischte Skalar-/Vektor-
    Arithmetik bekommt ein explizites `Splat`. `LogAnd` / `LogOr` werten kurzschließend aus.
  - `Construct(args)`: Vektoren aus Skalaren und Vektoren wie geschrieben, Matrizen immer aus
    Spaltenvektoren (Skalar-Argumente sind schon zu Spalten-`Construct`s gruppiert), Arrays und
    Structs aus Elementen. `Splat(e)`: Vektor aus einem Skalar. `Convert(e)`: numerische
    Umwandlung (Skalar, Vektor, Matrix; Identitäten sind entfernt; f32 → i32/u32 schneidet ab und
    klemmt, i32 ↔ u32 deutet die Bits um, bool ↔ Zahl). `Bitcast(e)`.
  - `Call(f, args)` (Index in `functions`), `Builtin(b, args)`: die Argumente haben schon die
    Typen der gewählten Überladung, die damit aus `b` und den Argumenttypen folgt (Texturen:
    Koordinaten, Array-Index, Level, Offset; Offsets und die Gather-Komponente sind `Const`).
- **Anweisungen** `Stmt`: `Block`, `Var { local, init }`, `Let { local, init }`,
  `Assign { lhs, rhs }`, `Compound { lhs, op, rhs }` (die linke Seite wird einmal ausgewertet; `++`
  ist `Compound` mit `Add` und der Konstante 1 des Speichertyps; `rhs` hat den Typ des rechten
  Operanden der gewählten Operator-Überladung, also z. B. f32 bei `v *= 2.0` mit `v: vec3f`),
  `Phony(e)`, `Call(e)`, `If { cond, then, else_ }` (else-if als geschachteltes `If`),
  `Switch { selector, cases }` (Selektorwerte als i64, `default` markiert), `Loop { body,
  continuing, break_if }`, `Break`, `Continue`, `Return`, `Discard`. `for (init; c; u) b` wird zu
  `Block[init, Loop { [If { c, [], [Break] }, b…], continuing: [u] }]`, `while c b` zu
  `Loop { [If { c, [], [Break] }, b…] }`. `const`-Deklarationen und `const_assert` verschwinden.
- **Einstiegspunkte** `EntryPoint { name, func, stage, workgroup_size, inputs, outputs, globals,
  resources, overrides, immediate }`: `workgroup_size` als `[WgSize; 3]` (`Const(n)` oder
  `Override(e)`), `inputs`/`outputs` als `IoVar { name ("param", "param.member", "return.member"),
  ty, param, member, kind, invariant, depth_mode }` mit `IoKind::Builtin(b)` oder
  `IoKind::Location { location, blend_src, interp }` (Interpolation mit den Voreinstellungen:
  perspective/center bzw. flat/first; `None` für Vertex-Eingaben und Fragment-Ausgaben),
  `globals` alle statisch benutzten Modulvariablen, `resources` davon die mit `(group, binding)`,
  `overrides` auch über Initialisierer, Array-Größen und die Workgroup-Größe, `immediate` die eine
  immediate-Variable.
- **Sonst**: `enables` / `requires` (feature-Bits), `diagnostic_filters` (Schweregrad, Regel,
  betroffener Bereich als Bytespanne; global = ganzes Modul) für die Uniformitätsanalyse (G2d),
  `consts` (die Modul-Konstanten, nur zur Information), `warnings`.

## Die Benchmark-Shader in WGSL

[`lib/wgsl/bench`](../lib/wgsl/bench) enthält `move`, `cull`, `cull_reset`, `vs_instanced`,
`vs_single` und `fs_lambert` mit denselben Bindungen und demselben Datenvertrag wie die
SPIR-V-Shader ([SHADER.md](SHADER.md), „Benchmark-Vertrag“); `wgsl::bench::ALL` liefert die Texte.
Jede Datei ist ein vollständiges Modul mit dem Einstiegspunkt `main`.

- **Push-Konstanten**: WGSL hat dafür `var<immediate>` (Spracherweiterung
  immediate_address_space, Chrome 156 unterstützt sie): `var<immediate> pc: Push;` mit denselben
  20 Byte ab Offset 0. Damit bleibt der Vertrag unverändert; `vs_single` liest weiter
  `pc.object_index`. (Die Alternative wäre `@builtin(instance_index)` mit `first_instance` =
  Objektindex beim direkten Draw – braucht kein Feature, ändert aber den Vertrag.)
- `DrawCmd` hat in `cull` ein `atomic<u32>` als `instance_count`, in `cull_reset` (das nur
  schreibt) ein gewöhnliches `u32`; das Layout ist gleich (20 Byte).
- Die Farbe geht mit `@interpolate(flat)` vom Vertex- zum Fragment-Shader (WebGPU verlangt
  gleiche Interpolation auf beiden Seiten; der SPIR-V-Vertex-Shader hatte kein Flat).
- `move` und `cull` rechnen in der Reihenfolge der CPU-Referenzen; die Konstanten (π als
  `0x1.921fb6p+1f`, 2⁻²⁴, die Taylor-Koeffizienten als f32-Divisionen) ergeben dieselben
  f32-Werte wie in `lib/shader/bench.rs`. Vom Back-End übersetzt (`move` und `cull` mit
  `exact`) ist `move` auf der GPU bitgleich zu `move_all`, `cull` gleich `cull_ref`, und die
  Bilder der Szene sind Pixel für Pixel die der handgeschriebenen SPIR-V-Shader (gputest
  `scene`, docs/GPU.md „Messszene“).

## Tests: `apps/wgsltest`

`python -u build_rs.py test wgsltest` (Windows und Linux) bzw. `build/win/wgsltest.exe [--only
GRUPPE] [--verbose]`, `--file F.wgsl` prüft eine Datei, `--spv F.wgsl [--out DIR]` übersetzt sie
auf beiden Stufen und schreibt die `.spv`-Dateien, `--stack ART N KB` misst den Stapel einer
Verschachtelung in einem Thread (genau nur unter Linux), `--only robust --mutations N --seed S`
ist ein Dauertest (bei der Entwicklung 560 000 Mutationen ohne Panik). 3453 Prüfungen unter
Windows mit Vulkan SDK (ohne SDK fehlen die `spirv-val`-Prüfungen der Gruppe `spirv`):

| Gruppe | Prüfungen | Inhalt |
|---|---|---|
| `lex` | 64 | Token-Klassen, Literalwerte und Bereiche, Template-Listen (alle Beispiele aus 3.9), Kommentare, Leerraum, Bezeichner, Positionen in Zeichen |
| `parse` | 58 | jede Anweisungsform, Vorrang, verbotene Mischungen, Token-Teilung, Fehlerbehandlung (mehrere Fehler je Lauf), Tiefengrenzen |
| `print` | 40 | parse → print → parse für die Benchmark-Shader und ein Beispiel mit allen Formen |
| `check` | 370 | 82 gültige und 262 ungültige Module (jeder Fehler mit Position und Textteil), Warnungen, Hinweise, 14 Fälle mit abgeschalteten Features |
| `consts` | 164 | je 300 Zufallsfälle AbstractInt gegen i64, AbstractFloat gegen f64, f32 gegen Rusts f32; Umwandlungen; alle const-Funktionen |
| `ir` | 65 | keine abstrakten Typen, Faltung, Load-Regel, Splats, Matrixspalten, Schleifen, Layout-Beispiele der Spezifikation, Overrides, Schnittstellen |
| `bench` | 73 | die sechs Shader: Stufe, Workgroup, Bindungen, immediate, Ein-/Ausgaben, Layouts (Object 48, Camera 176, DrawCmd 20, CubeVertex 32, Push 20) |
| `robust` | 32 | 2400 zufällige Mutationen der gültigen Testshader, Verschachtelung an und über allen Grenzen, der tiefste gültige Fall in 512 KB (auch durch das Back-End auf beiden Stufen), lange Ketten, Zyklen, riesige Konstanten |
| `perf` | 5 | ein erzeugter Shader mit 27 010 Zeilen (883 KB, 1501 Funktionen), auch nach SPIR-V 1.6 abgesenkt (Entwicklungs-Build 57 ms, 705 431 Wörter) |
| `spirv` | 1988 | 322 Quellen auf beiden Stufen: die Benchmark-Shader, 13 Abdeckungs-Module ([`coverage.rs`](../apps/wgsltest/src/coverage.rs): jede Texturüberladung, Stufen-Built-ins und Interpolationen, f16 in jedem Adressraum und an der Schnittstelle, Zeiger in jeden Adressraum als Parameter, Kontrollfluss, Overrides überall, Atomics, `discard`, Werte und Umwandlungen), die gültigen Fälle der Gruppe `check` (ohne Einstiegspunkt als Bibliothek) und jede Korpus-Datei, die Tint annimmt: 672 Module, alle von `shader::val` und `spirv-val --target-env vulkan1.1` / `vulkan1.3` angenommen, jedes zweimal übersetzt wortgleich |
| `corpus` | 512 | das Urteil über die 494 Korpus-Dateien gegen Tints (siehe [Korpus-Vergleich](#korpus-vergleich)) |
| `backend` | 79 | die API: was Einstiegspunkte melden (Ressourcen mit Art und Mindestgröße, immediate, Workgroup, Stufen-Variablen, `Needs`), `robust` (kein `OpArrayLength` ohne), `exact` (`NoContraction` an jeder Gleitkomma-Operation), die Interface-Listen je Stufe, Overrides (SpecIds, `constants` mit abgeleiteten und versteckten Werten, eingesetzte Werte ohne Spezialisierungskonstante, sieben Fehlerfälle), Pipeline-Grenzen, Schnittstellen, Bibliothek, Fehler |
| `perf_backend` | 2 | die Übersetzungszeit der sechs Benchmark-Shader je Stufe |

Übersetzungszeit (Windows, Release-Build): Parsen 58 ms, Parsen + Prüfen 135 ms für die
27 010 Zeilen (≈ 200 000 Zeilen/s, 144 014 IR-Ausdrücke); die sechs Benchmark-Shader zusammen
0,8 ms (mit dem Absenken nach SPIR-V 1,2–1,3 ms, siehe Back-End). Entwicklungs-Build: 74 / 169 ms
(Windows), 104 / 239 ms (Linux in WSL).

Auf der GPU prüft `gputest --test wgsl` (docs/GPU.md „Tests“) das Back-End: jeder Operator und
jede numerische eingebaute Funktion auf Laufzeitwerten gegen den Konstantenauswerter des
Front-Ends (150 Fälle, 13 469 Ergebnisse, auf SPIR-V 1.6 und 1.3 je 0 Abweichungen), Overrides,
Robustheit, Workgroup-Speicher, `discard` und Texturen. Dieselben Shader in Chrome 156 (Tint →
D3D12, über [`tools/wgsl_run.mjs`](../tools/wgsl_run.mjs): `gputest --test wgsl --dump F`,
`node tools/wgsl_run.mjs F R`, `gputest --test wgsl --chrome R`) geben auf einem Gerät mit allen
Features dieselben Ergebnisse innerhalb der Toleranzen (0 Abweichungen). Auf einem Gerät ohne
`shader-f16` gab Chrome für `quantizeToF16` den Wert unverändert zurück (`quantizeToF16(0.1)` =
0.1; 4 von 8 Werten falsch): ein Fehler in Chromes Weg ohne f16.

Die 344 Tabellenfälle der Gruppe `check` und rund 410 weitere Proben (alle Texturfunktionen, Überladungen,
Grenzen, Konstanten, Einstiegspunkte) liefen während der Entwicklung auch durch Chrome 156: bis
auf die Fälle im nächsten Abschnitt stimmen Annahme und Ablehnung überein.

## Back-End: SPIR-V (G2c)

`wgsl::compile` übersetzt WGSL nach SPIR-V (mit [`lib/shader`](SHADER.md)), ein SPIR-V-Modul je
Einstiegspunkt, für die Stufen Basis (SPIR-V 1.3, Vulkan 1.1) und Voll (SPIR-V 1.6, Vulkan 1.3;
1.4 und 1.5 gehen auch). Dateien: [`spirv.rs`](../lib/wgsl/spirv.rs) (API, Typen, Konstanten,
Overrides, Modulvariablen), [`spirv_func.rs`](../lib/wgsl/spirv_func.rs) (Einstiegspunkte,
Anweisungen, Ausdrücke, Speicher, Aufrufe), [`spirv_builtin.rs`](../lib/wgsl/spirv_builtin.rs)
(eingebaute Funktionen, Texturen), [`pipeline.rs`](../lib/wgsl/pipeline.rs) (Overrides auswerten,
Pipeline-Prüfungen).

```rust
let o = wgsl::compile(src, &wgsl::CompileOptions::new(shader::VOLL))?;   // oder shader::BASIS
let e = o.entry("main").unwrap();
let module = gpu::ShaderModule::from_spirv(&dev, &e.words)?;
// e.resources: (group, binding, name, kind, min_size) für die Bind-Group-Layouts,
// e.immediate_size: die Push-Konstanten-Bytes, e.workgroup_size, e.inputs / e.outputs, e.needs
let given = [("n", 64.0), ("5", 2.0)];                  // wie WebGPU: Name, bei @id die Zahl
o.check_pipeline("main", &given, &wgsl::Limits::WEBGPU)?;   // was WebGPU beim Anlegen prüft
let consts: Vec<gpu::Constant> = o.constants("main", &given)?.into_iter()
    .map(|(id, value)| gpu::Constant { id, value }).collect();
let ws = o.workgroup_size("main", &given)?;              // für dispatch()
let pipe = dev.create_compute_pipeline(&gpu::ComputePipelineDesc { layout: &pl,
    shader: gpu::ShaderStage { module: &module, entry: "main", constants: &consts } })?;
```

`CompileOptions { minor, entry, constants, features, robust, exact }`: `entry` = nur dieser
Einstiegspunkt; `constants: Some(&[(Schlüssel, Wert)])` setzt die Overrides als Konstanten ein
(siehe unten); `features` wie `Options::features`; `robust` (Voreinstellung an) die Robustheit
von WebGPU; `exact` (aus) `NoContraction` an jeder Gleitkomma-Addition, -Subtraktion,
-Multiplikation, -Division und jedem Produkt. `Output { entries, overrides, module, warnings,
minor }`, je `EntryOut { name, stage, words, workgroup_size, resources, immediate_size, inputs,
outputs, needs, spec, overrides }`. `compile_module(&Module, &opt)` für ein schon geprüftes
Modul; `spirv::compile_library` übersetzt die Funktionen eines Moduls ohne Einstiegspunkt
(neben einem leeren Compute-Einstiegspunkt; für Tests, Validatoren prüfen jede Funktion).
`Needs` sagt, welche Geräte-Features das Modul braucht: `f16` (shaderFloat16), `storage16` /
`uniform16` / `push16` (die 16-Bit-Speicher-Features), `uniform_standard_layout`, `demote`,
`int64_atomics`, `storage_without_format`, `primitive_index`, `clip_distance`,
`sample_rate_shading`; `lib/gpu` schaltet die ersten fünf ein, wo das Gerät sie hat
(`Features::shader_f16`, `storage_16bit`, `uniform_16bit`, `push_constant_16bit`,
`uniform_standard_layout`, `demote`).

### Was abgesenkt wird

- **Ein Modul je Einstiegspunkt** mit nur dem, was dessen Aufrufbaum braucht (wie Tint). Das
  löst auch die Einschränkung von Basis, dass das `WorkgroupSize`-Built-in für jeden
  Compute-Einstiegspunkt eines Moduls gilt. Der SPIR-V-Einstiegspunkt ist eine Hülle ohne
  Parameter: Workgroup-Speicher nullen, private Variablen mit Override-Initialisierer setzen, die
  Eingaben laden (Structs aus ihren Membern bauen), die WGSL-Funktion aufrufen, das Ergebnis in
  die Ausgaben schreiben. `OpEntryPoint` trägt den WGSL-Namen.
- **Typen.** Jeder WGSL-Typ hat einen Werttyp und, wenn er host-teilbar ist, einen
  Speichertyp mit `Offset` (die Offsets des IR), `ArrayStride`, `ColMajor` + `MatrixStride`
  (Spalten im Abstand roundUp(align, size) des Spaltenvektors; nie `RowMajor`). Vulkan verbietet
  explizite Layouts außerhalb von Uniform-, Storage- und Push-Konstanten-Speicher
  (VUID-StandaloneSpirv-None-10684; `spirv-val` prüft das für SPIR-V 1.4+ überall, für 1.3 beim
  Workgroup-Speicher): auf Voll sind Wert- und Speichertyp verschiedene Typen, Laden und
  Speichern ganzer Structs und Arrays wandelt mit `OpCopyLogical`; auf Basis ist der Werttyp
  jedes host-teilbaren Teils der Speichertyp, nur Workgroup-Speicher und Stufen-Variablen haben
  den schlichten Typ (Umbau Member für Member). atomic → die Ganzzahl, `atomic<vec2<u32>>` → u64.
- **Adressräume:** function → Function (Variablen im Eintrittsblock, an der Deklaration mit dem
  Initialisierer oder dem Nullwert beschrieben, also bei jedem Durchlauf neu), private → Private
  (Initialisierer `OpConstant*`, sonst der Nullwert), workgroup → Workgroup (genullt: ein Aufruf
  mit `local_invocation_index` 0 schreibt den Nullwert, dann `workgroupBarrier`; Override-Arrays
  in einer Schleife), uniform → Uniform, storage → StorageBuffer (`read` mit `NonWritable`),
  immediate → PushConstant, handle → UniformConstant (Speichertexturen `read` mit
  `NonWritable`, `write` mit `NonReadable`). Puffer: der Speichertyp steckt in einem
  `Block`-Struct `{ inner }`, außer er ist ein Struct mit Laufzeit-Array (Vulkan erlaubt
  Laufzeit-Arrays nur als letztes Member des Blocks selbst: dann ist das Struct der Block).
- **Stufen-Ein- und -Ausgaben** einzeln: `Location` (+ `Index` für `@blend_src`),
  Interpolation (linear → `NoPerspective`, flat → `Flat`, centroid → `Centroid`, sample →
  `Sample` + SampleRateShading), Built-ins (`position` als Eingabe = `FragCoord`, `frag_depth` →
  `DepthReplacing` und `DepthLess` / `DepthGreater`, `sample_mask` als `array<u32, 1>`,
  `sample_index` = `SampleId`, `primitive_index` = `PrimitiveId` mit Capability Geometry,
  `clip_distances` = `ClipDistance`, `@invariant` → `Invariant`; `workgroup_index` und
  `global_invocation_index` aus `workgroup_id` / `global_invocation_id`, `num_workgroups` und der
  Workgroup-Größe gerechnet). f16-Ein- und -Ausgaben gehen als f32 über die Schnittstelle
  (`FConvert`), so braucht es kein storageInputOutput16. Vertex-Shader schreiben `PointSize` 1.
  Fragment-Shader haben `OriginUpperLeft`; kein Kippen von y (das macht `lib/gpu` am Viewport).
- **Funktionen mit Zeiger-, Textur- oder Sampler-Parametern.** SPIR-V nimmt unter Logical
  Addressing als Zeiger-Argument nur eine Variable oder einen Zeiger-Parameter, keine Access
  Chain (`spirv-val`: „must be a memory object declaration“). Ein Zeiger ist im Back-End
  deshalb Wurzel + Access Chain (erst bei Gebrauch ausgegeben), und eine Funktion wird je
  Aufrufform spezialisiert (wie Tints DirectVariableAccess): ein Zeiger in eine Modulvariable
  wird fest eingesetzt, seine dynamischen Indizes werden u32-Parameter; ein Zeiger in eine
  Funktionsvariable übergibt die Variable plus Indizes; Texturen und Sampler werden fest
  eingesetzt. WGSL kennt keine Rekursion, die Spezialisierungen sind endlich.
- **Kontrollfluss** strukturiert: `if` / `else` (`OpSelectionMerge`), `switch` mit
  Fallauswahl-Listen und `default` auch in einer Liste, `loop` mit `continuing` und
  `break if`, `&&` / `||` mit Kurzschluss (über eine Funktionsvariable). Unerreichbare
  Merge-Blöcke enden mit `OpUnreachable`, ein nie erreichter Continue-Block springt nur zurück.
- **Alle eingebauten Funktionen** (GLSL.std.450 wo möglich): Texturen mit allen Überladungen
  (Arrays: die Schicht als Gleitkomma-Koordinate beim Abtasten, Offsets als `ConstOffset`,
  Tiefentexturen → `.x`), Atomics (relaxed, Bereich Device, im Workgroup-Speicher Workgroup;
  `atomicCompareExchangeWeak` → das Ergebnis-Struct), Barrieren (`workgroupBarrier`,
  `storageBarrier`, `textureBarrier` als `OpControlBarrier` mit WorkgroupMemory / UniformMemory /
  ImageMemory), `workgroupUniformLoad` = Barriere, Laden, Barriere, Ableitungen (Fine / Coarse),
  pack / unpack, `modf` / `frexp` (`ModfStruct` / `FrexpStruct`), `quantizeToF16`, f16 überall.
- **`discard`** mit WGSLs Bedeutung (der Aufruf wird zum Helfer und läuft weiter, nur was vorher
  geschah, wirkt): auf Voll `OpDemoteToHelperInvocation` (SPIR-V 1.6, Feature
  shaderDemoteToHelperInvocation, das `lib/gpu` einschaltet); auf Basis ohne Erweiterung wie
  Tints DemoteToHelper: eine private Variable `discarded`, Schreibzugriffe auf Speicherpuffer,
  Atomics und `textureStore` danach übersprungen, die Hülle endet mit `OpKill`.

### WGSLs Laufzeitregeln, wo SPIR-V anderes tut

| Fall | SPIR-V | Back-End |
| --- | --- | --- |
| `a / 0`, `a % 0`, `i32::MIN / -1`, `i32::MIN % -1` | undefiniert | der Divisor wird 1 (`select`): `a / 0 = a`, `a % 0 = 0` |
| `a % b` mit i32 | `OpSRem` | `a - (a / b) * b`: der Treiber der RTX 2070 SUPER gab `1 % -1 = 1` mit `OpSRem` (Tint vermeidet es ebenso) |
| `a << b`, `a >> b` | undefiniert ab der Bitbreite | `b & 31` |
| `i32(f)`, `u32(f)` | undefiniert außerhalb | klemmen (`NMax` / `NMin` auf den in f32 darstellbaren Bereich, f16 passt immer), dann umwandeln; NaN → die Untergrenze |
| `min`, `max`, `clamp`, `saturate` (float) | `FMin` / `FClamp` undefiniert bei NaN, `FClamp` bei low > high | `NMin` / `NMax` (NaN-Operand: der andere), `clamp` = `min(max(e, low), high)` auch für Ganzzahlen |
| `extractBits`, `insertBits` | undefiniert, wenn offset + count > 32 | `o = min(offset, 32)`, `c = min(count, 32 - o)` |
| `countLeadingZeros`, `countTrailingZeros` | — | `31 - FindUMsb(x)` (32 für 0), `min(FindILsb(x), 32)` |
| `pack4x8snorm`, `pack4x8unorm`, `pack2x16snorm`, `pack2x16unorm` | GLSL rundet Halbe vom Nullpunkt weg | WGSLs `floor(0.5 + k · clamp(e))` je Komponente in f32 |
| `round` | — | `RoundEven` |
| `select(f, t, c)` mit skalarem `c` und Vektoren | Bedingung als Vektor vor SPIR-V 1.4 | `c` verbreitert |
| `dot` auf Ganzzahlvektoren, `dot4I8Packed`, `pack4xI8` … | — | ausgeschrieben |

**Kontraktion.** WGSL lässt einer Implementierung das Zusammenfassen von `x * y + z` zu einem
FMA (es gibt kein `precise`; Tint schreibt kein `NoContraction`, der Browser darf also
fusionieren). Das Back-End schreibt `NoContraction` nur mit `CompileOptions::exact`. Die
Messszene zeigt, dass es nötig ist: `move` aus WGSL ohne `exact` weicht in 1,33 Mio. von
3,6 Mio. Wörtern (drei Zeitpunkte, 100 003 Objekte) von `move_all` ab (der Treiber fusioniert),
mit `exact` ist es bitgleich. gputest übersetzt `move` und `cull` deshalb mit `exact` (wie die
handgeschriebenen SPIR-V-Shader, die `NoContraction` nur dort tragen), die Vertex- und
Fragment-Shader ohne.

### Robustheit

`lib/gpu` schaltet robustBufferAccess aus (Tempo), also macht das Back-End Zugriffe außerhalb der
Grenzen unmöglich, wie Tints Robustness-Transform (`CompileOptions::robust`, voreingestellt an):

- Indizes in Arrays fester Länge, Vektoren und Matrizen: `UMin(i, n - 1)` (i32-Indizes als u32
  gelesen: negative werden groß und auf das letzte Element geklemmt); konstante Indizes prüft
  schon das Front-End. Laufzeit-Arrays: `UMin(i, arrayLength - 1)` mit `OpArrayLength`.
  Arrays mit Override-Länge: `UMin(i, n - 1)` mit der Spezialisierungskonstante. Dynamische
  Indizes in Matrix- und Array-Werte gehen über eine Funktionsvariable.
- `textureLoad`: Mip-Stufe auf `textureNumLevels - 1`, Koordinaten auf die Größe dieser Stufe
  minus 1, Schicht auf `textureNumLayers - 1`, Sample-Index auf `textureNumSamples - 1`
  geklemmt (WGSL erlaubt jedes Texel innerhalb); `textureStore` außerhalb wird übersprungen
  (WGSL: „does not execute“); `textureDimensions` klemmt die Stufe.
- Voraussetzung wie in WebGPU: jede Puffer-Bindung ist mindestens `Resource::min_size` groß
  (der feste Teil plus ein Element eines Laufzeit-Arrays). WebGPU prüft das beim Zeichnen,
  `lib/gpu` nicht; mit einer kleineren Bindung ist `arrayLength - 1` ein Unterlauf.

Kosten in der Messszene (RTX 2070 SUPER, Median aus drei abwechselnden Läufen mit
Prozesspriorität „hoch“, docs/GPU.md „Messszene“): `move` mit 1M Objekten 0,251 ms robust gegen
0,244 ms ohne (+2,8 %), `cull` 0,133 ms in beiden, das Zeichnen (drei Laufzeit-Arrays im
Vertex-Shader) 1,234 gegen 1,236 ms; die GPU-Zeit je Bild 1,617 gegen 1,613 ms (1080p) und
2,313 gegen 2,309 ms (4K), also +0,2 %. Ohne Robustheit schrieb der Treiber Indizes über ein
Laufzeit-Array hinaus nicht in den Puffer (er prüft offenbar selbst die Bindungsgrenze), über ein
festes Array in einem Struct hinaus aber schon (gputest `wgsl`).

### Overrides

Ohne `CompileOptions::constants` wird jedes Override, das der Einstiegspunkt in Funktionen
benutzt, eine Spezialisierungskonstante: `SpecId` = `@id`, ohne `@id` `spirv::OVERRIDE_BASE`
(0x10000) + sein Index (außerhalb von 0 … 65535, die WGSL für `@id` erlaubt). Ihr
Standardwert im Modul ist der ausgewertete Initialisierer (0 ohne). Workgroup-Größen und
Array-Längen aus Override-Ausdrücken werden versteckte u32-Spezialisierungskonstanten
(`spirv::HIDDEN_BASE` 0x20000 + der Ausdruck; Workgroup-Größe auf Voll `LocalSizeId`, auf Basis
das `WorkgroupSize`-Built-in). `OpSpecConstantOp` kommt nicht vor:

- `Output::constants(entry, &[(Schlüssel, Wert)])` nimmt die Konstanten wie WebGPUs
  `GPUProgrammableStage.constants` (Schlüssel: der Name, bei einem Override mit `@id` die Zahl;
  Werte als f64, umgewandelt wie WebGPU: bool = nicht 0 und nicht NaN, Ganzzahlen abgeschnitten
  und im Bereich, Gleitkomma endlich und im Bereich), wertet jedes Override des Einstiegspunkts
  mit dem Konstantenauswerter des Front-Ends aus (auch abgeleitete mit Gleitkomma-Arithmetik und
  eingebauten Funktionen) und liefert für **jede** Spezialisierungskonstante des Moduls den
  fertigen Wert, die versteckten eingeschlossen. Fehler wie in WebGPU: unbekannter Schlüssel, ein
  `@id`-Override beim Namen, ein Wert außerhalb des Typs, ein Override ohne Initialisierer und ohne
  Wert, ein Auswertungsfehler (Division durch 0, Überlauf), Workgroup-Größe oder Array-Länge 0.
- `Output::workgroup_size(entry, &given)` die Workgroup-Größe für `dispatch`.
- `CompileOptions::constants = Some(&given)` setzt die Werte ein (Tints Weg: neu übersetzen):
  das Modul hat dann keine Spezialisierungskonstante, `LocalSize` fest; Fehler sind Fehler von
  `compile`.

Override-Ausdrücke in Funktionsrümpfen (`x * a` mit einem Override `a`) rechnet der Shader zur
Laufzeit mit der Spezialisierungskonstante; Pipeline-Fehler, die erst ihr Wert zeigt, findet
`constants` nur für Overrides, Workgroup-Größen und Array-Längen.

### Pipeline-Prüfungen

`Output::check_pipeline(entry, &given, &Limits)` prüft, was WebGPU beim Anlegen der Pipeline
prüft: die Overrides (wie `constants`), Workgroup-Größe je Achse und Aufrufe gesamt,
Workgroup-Speicher (je Variable auf 16 Byte aufgerundet, Override-Arrays mit ihrem Wert),
Fragment-Ausgaben `@location` < maxColorAttachments, Vertex-Ausgaben und Fragment-Eingaben <
maxInterStageShaderVariables, die immediate-Bytes, und die Speichertextur-Formate des Kerns
(bgra8unorm nur mit bgra8unorm-storage, `read_write` nur r32float / r32sint / r32uint, mit
`texture_formats_tier2` mehr). `Limits::WEBGPU` sind WebGPUs Standardgrenzen (256 Aufrufe,
256 × 256 × 64, 16 KiB, 8, 16, 64 Byte). `check_interface(&module, &vertex, &fragment)` prüft
WebGPUs Stufen-Regeln: zu jeder Fragment-Eingabe eine Vertex-Ausgabe an derselben Location mit
demselben Komponententyp, mindestens so vielen Komponenten und derselben Interpolation.
`lib/gpu` prüft dagegen nur die Gerätegrenzen.

### Korpus-Vergleich

Die Gruppe `corpus` vergleicht das Urteil von `lib/wgsl` (prüfen, übersetzen, dann je
Einstiegspunkt `check_pipeline` mit `Limits::WEBGPU` und der `// constants:`-Zeile, und
`check_interface` zwischen dem ersten Vertex- und jedem Fragment-Einstiegspunkt) mit Tints
Urteil in `expected.json`, ohne Chrome: **478 der 494 Dateien stimmen überein**, alle 16
Pipeline-Fehler eingeschlossen (Workgroup-Grenzen 2, Fragment-Location 8, Location 16,
Workgroup-Speicher, Override-Fehler 6, immediate 80 Byte, Speichertextur-Formate 2,
Schnittstellen 2). Die Gruppe schlägt fehl bei jeder neuen Abweichung außerhalb der beiden
Listen in [`backend.rs`](../apps/wgsltest/src/backend.rs):

- **Wir nehmen an, Tint lehnt ab** (`ALLOW_ACCEPT`, 11, alle wegen der fehlenden
  Uniformitätsanalyse, G2d): `diag/err_default_error`, `diag/err_error_directive`,
  `unif/err_sample_after_nonuniform_loop_break`, `unif/err_sample_after_nonuniform_return`,
  `unif/err_short_circuit_nonuniform` (Ableitungen / `textureSample` in nicht uniformem
  Kontrollfluss), `unif/err_barrier_nonuniform_if`, `unif/err_barrier_read_write_storage`,
  `unif/err_parameter_nonuniform_call`, `unif/err_pointer_param_nonuniform`,
  `unif/err_storage_barrier_nonuniform`, `unif/err_workgroup_plain_load` (Barrieren und
  `workgroupUniformLoad` in nicht uniformem Kontrollfluss).
- **Wir lehnen ab, Tint nimmt an** (`KNOWN_REJECT`, 5): `lex/err_float_leading_zero_suffix`
  (`01f`: die Grammatik der Spezifikation erlaubt keine führende Null vor einem Suffix; Tint
  nimmt es an), `enable/ok_subgroups`, `enable/ok_subgroups_f16`,
  `diag/ok_subgroup_uniformity_rule`, `requires/ok_subgroup_id` (Subgroups fehlen).

`requires/err_unsafe_only_feature` stimmt jetzt: `fragment_depth` ist umgesetzt, aber nicht mehr
in `feature::ALL`, weil Chrome 156 es nur mit `--enable-unsafe-webgpu` anbietet (einschalten
mit `Options { features: feature::ALL | feature::FRAGMENT_DEPTH }`). Beide Listen werden selbst
geprüft: ein Eintrag, der nicht mehr abweicht, lässt die Gruppe ebenfalls fehlschlagen.

### Übersetzungszeit

Die sechs Benchmark-Shader zusammen (prüfen + absenken, `wgsltest --only perf_backend`, Median aus
drei Läufen unter Last, Windows): Release-Build 1,29 ms für SPIR-V 1.3 und 1,22 ms für 1.6, davon
prüfen 0,81 ms und absenken 0,37 / 0,46 ms (5768 / 5875 Wörter); Entwicklungs-Build 1,61 / 1,60 ms
(absenken 0,44 / 0,43 ms).

## Abweichungen zwischen Spezifikation und Chrome

Wo beide sich widersprechen, folgt `lib/wgsl` der Spezifikation:

| Fall | Spezifikation (lib/wgsl) | Chrome 156 |
|---|---|---|
| `f32(5000000000)` | f32 nimmt nur konkrete Skalare: die Überladung über i32 gewinnt (Rang 3), 5000000000 passt nicht → Fehler | angenommen |
| `for (var i = 0; …) { var i = 1; }` | Initialisierer und Rumpf haben dasselbe Ende des Gültigkeitsbereichs → Fehler (Beispiel in Kapitel 5) | angenommen |
| `array<i32, select(1, 2, 1 <= 3)>` | der Algorithmus aus 3.9 nimmt das `=` nach einem `<`, vor dem kein Name steht, als Zuweisung und verwirft die offenen Listen → Syntaxfehler | angenommen (Tint erkennt Templates über Tokens) |
| `-(-9223372036854775807 - 1)` | AbstractInt darf nicht überlaufen → Fehler | angenommen |
| Ketten mit 256 bis 511 Gliedern | `MAX_EXPR_DEPTH` = 256 (Stapelbudget) → Fehler | bis 512 angenommen |

Weitere Unterschiede, die keine Widersprüche sind: Chrome lehnt nicht uniforme Aufrufe von
Ableitungen und `textureSample` ab (Uniformitätsanalyse, kommt mit G2d); Chrome nimmt
`ptr<uniform, array<T>>` und `ptr<workgroup, array<T>>` an (erlaubt durch buffer_view, das hier
fehlt); `&&p` lehnen beide ab, Chrome als Syntaxfehler, `lib/wgsl` weil `&` keinen Zeiger nimmt.

## Was fehlt

- **Uniformitätsanalyse** (G2d): `derivative_uniformity` und `subgroup_uniformity` werden nicht
  geprüft; die Diagnose-Filter stehen im IR bereit. Bis dahin nimmt das Front-End Shader an, die
  Chrome wegen nicht uniformer Ableitungen ablehnt.
- **Subgroups**: `enable subgroups` / `subgroup_size_control`, `requires subgroup_id` /
  `subgroup_uniformity`, die Subgroup- und Quad-Funktionen und -Built-ins, `@subgroup_size` – alle
  mit der Meldung „noch nicht unterstützt“.
- **buffer_view**: `buffer<N>` / `buffer`, `bufferView`, `bufferArrayView`, `bufferLength` und die
  Zeigerregeln, die davon abhängen.
- **Pipeline-Fehler**: `check_pipeline` und `check_interface` finden Override-Fehler,
  Grenzen, Speichertextur-Formate und Schnittstellen (alle 16 des Korpus). Nicht geprüft:
  Fehler von Override-Ausdrücken in Funktionsrümpfen (`1 / a` mit `a` = 0 rechnet der Shader mit
  WGSLs Laufzeitregel), Bind-Group-Kompatibilität mit `layout: 'auto'`, die Anzahl der Bindungen
  je Art und Stufe, Stufen-Variablen über die Location-Grenze hinaus, wenn Built-ins mitzählen.
- **Back-End**: Uniform-Puffer mit Matrizen `matCx2` (Spaltenabstand 8) und andere nicht
  std140-taugliche Layouts brauchen uniformBufferStandardLayout (Tint zerlegt solche Matrizen in
  Spalten); Bindungen kleiner als `Resource::min_size` prüft weder das Back-End noch `lib/gpu`;
  Konstanten-Composites über 65 000 Elemente sind ein Fehler (SPIR-V-Instruktionsgrenze);
  `primitive_index`, `clip_distances`, Sample-Interpolation, `@blend_src` brauchen
  Geräte-Features (`lib/gpu` schaltet die Shader-Seite ein, Dual-Source-Blending in der Pipeline
  fehlt); `atomicStoreMin/Max` auf `atomic<vec2<u32>>` braucht 64-Bit-Atomics (ungetestet auf der
  GPU); `texture_external` ist eine gewöhnliche 2D-Textur (eine Ebene, keine Farbumwandlung).
  Mit Chrome verglichen sind nur die numerischen Fälle von `gputest --test wgsl`, nicht die
  Overrides-, Robustheits-, `discard`- und Texturteile.
- **Grenzen der Spezifikation** (Mindestwerte, die eine Implementierung prüfen sollte): mehr als
  1023 Struct-Member, Typ-Schachtelung über 15, mehr als 255 Parameter, mehr als 1023
  case-Werte, Byte-Grenzen von private (8192), function (8192), workgroup (16384) und immediate
  (64) werden nicht gemeldet.
- **Genauigkeit**: sin/cos/tan reduzieren das Argument mit einem dreiteiligen π/2 (genau bis etwa
  2²⁰ · π/2; darüber verlieren sie Stellen); die anderen transzendenten Funktionen sind auf wenige
  ulp genau. Warnungen vor Homoglyphen in Bezeichnern (eine „should“-Empfehlung) fehlen.

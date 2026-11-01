# WGSL-Korpus: Tints Urteil als Referenz

932 kleine WGSL-Module, selbst geschrieben, die die Regeln der WGSL-Spezifikation von beiden
Seiten prüfen: was gültig sein muss und was abgelehnt werden muss. `expected.json` ist das
Urteil von Chromes WGSL-Übersetzer (Tint) über jede Datei, erzeugt von
[`tools/wgsl_ref.mjs`](../wgsl_ref.mjs). Damit können die Tests von JFLibs eigenem
WGSL-Übersetzer (G2) ohne Chrome vergleichen: was der Browser annimmt, muss auch JFLib annehmen.

## Dateien

Ein Ordner je Bereich, eine Datei je Fall. Der Name beginnt mit unserer Erwartung: `ok_`
(gültig), `err_` (Fehler beim Übersetzen des Moduls), `pipe_` (das Modul ist gültig, aber eine
Pipeline lässt sich nicht anlegen: Overrides, Grenzwerte, Schnittstellen zwischen den Stufen).

| Ordner | Fälle | Inhalt |
| --- | ---: | --- |
| `lex` | 42 | Kommentare (verschachtelt), Leerraum (auch Unicode), Bezeichner (Unicode, `_`, `__`, reservierte Wörter, Schatten vordeklarierter Namen), Literale (Ganzzahlen, Hex, Suffixe, Bereiche, Gleitkomma dezimal und hexadezimal, Überlauf) |
| `templ` | 11 | Erkennung von Template-Listen: `<` als Vergleich, `&&` / `\|\|` beenden Kandidaten, `>>`, Aufrufe und Klammern in Template-Argumenten |
| `decl` | 34 | Deklarationen und Sichtbarkeit: Reihenfolge auf Modulebene, Schatten, Neudeklaration, Rekursion, Zyklen, `const` / `let` / `var` / `override`, Phony-Zuweisung, Structs, Aliase, Schleifen-Gültigkeitsbereiche |
| `types` | 45 | Vektoren, Matrizen, Arrays (Anzahl, Laufzeit-Arrays, Override-Größen), Atomics, konstruierbare Typen, Konstruktoren, Zeiger, Operatoren (Mischen von Bit-Operatoren, Ketten, Vergleiche) |
| `abstract` | 22 | abstrakte Zahlen und ihre Umwandlung: Darstellbarkeit, Konkretisierung durch `let`, f64-Genauigkeit, Shifts |
| `consteval` | 39 | `const_assert`, Überlauf und Division durch 0 in Konstantenausdrücken, Sättigung bei Float→Int, eingebaute Funktionen in Konstantenausdrücken (Definitionsbereiche), Kurzschluss, Indizes |
| `override` | 58 | Overrides: Typen, `@id`, Ableitungen, Pipeline-Konstanten nach Name und Zahl, Fehler erst bei der Pipeline, Workgroup-Größen, Override-Ausdrücke in Funktionsrümpfen, die mit dem Wert zum Fehler werden |
| `attr` | 32 | Attribute: `@align`, `@size`, Doppelte, unbekannte, `@workgroup_size` (Typen, Grenzen), `@interpolate`, `@invariant`, `@must_use` |
| `addr` | 23 | Adressräume und Zugriffsarten, Initialisierer, Speichern in Werte, Adressen, Zeigerparameter, `var<immediate>` |
| `layout` | 13 | Speicherlayout: Uniform-Regeln (mit `uniform_buffer_standard_layout`), Ausrichtung, Größen, Laufzeit-Arrays, f16, Grenzwerte für Uniform- und Workgroup-Speicher |
| `entry` | 26 | Entry Points und ihre Schnittstellen, Built-ins (Typen, Stufen), Locations, Zusammenspiel Vertex/Fragment, Grenzwerte |
| `behav` | 29 | Verhaltensanalyse (fehlende Returns, Schleifen ohne Ausgang) und Kontrollfluss (`break if`, `continue`, `discard`, `switch`) |
| `builtin` | 41 | Überladungen eingebauter Funktionen, Stufenbeschränkungen (Ableitungen, Barrieren), `@must_use` |
| `tex` | 39 | Texturen und Sampler: Sample-, Load-, Store-, Compare-, Gather- und Abfragefunktionen, Speichertexturen und ihre Formate, Offsets, Bindungen |
| `atomic` | 12 | Atomics: Operationen, Adressräume, Zugriffsarten, Benutzung als Wert |
| `requires` | 18 | die WGSL-Sprachfeatures, die Chrome 156 anbietet, und Fehlerfälle |
| `enable` | 89 | `enable f16`, `subgroups`, `clip_distances`, `dual_source_blending`, `primitive_index`, unbekannte und experimentelle Erweiterungen; die Subgroup- und Quad-Funktionen, ihre Built-ins und `@subgroup_size` |
| `diag` | 68 | `diagnostic`-Direktiven und -Attribute: Schweregrade, Regelnamen, Konflikte, jede Stelle eines Filters, Filter durch Funktionsketten, `chromium.unreachable_code` |
| `unif` | 229 | Einheitlichkeitsanalyse: Barrieren, `textureSample`, Speicher- und Workgroup-Werte, Parameter, Kurzschluss, `discard`, Schleifen, `continuing`, `break if`, `switch`, Funktionsketten, Zeiger, `let` / `var`, Quellen, der Subgroup-Bereich |
| `limits` | 62 | die Grenzen aus Kapitel 2.4 und die von Chrome: Struct-Member, Schachtelung, Parameter, case-Werte, Klammern, Tiefe der Anweisungen, Bytes je Adressraum, Array-Konstruktoren, Arrays ab 65536 Elementen |

Jede Datei beginnt mit Kommentarzeilen:

```wgsl
// WGSL "Literals": a decimal integer literal has no leading zeros
// expect: error
// uniformity                     (nur bei Einheitlichkeitsfällen)
// constants: {"n": 7}            (Pipeline-Konstanten für die Pipelines, optional)
// features: all                  (Gerät mit allen Features auch ohne enable, optional)
```

Die erste Zeile nennt den Abschnitt der Spezifikation (seine Überschrift) und die Regel,
`expect` unsere Erwartung. Die Dateien in `unif` und die meisten in `diag` sind als
Einheitlichkeitsfälle markiert.
Alle Fälle sind selbst geschrieben; nichts ist aus der WebGPU-CTS, aus Tint oder aus der
Spezifikation kopiert. Neue Fälle bitte genauso.

## `expected.json`

Oben die Umgebung: Chrome-Version, ob headless, Flags, Adapter (`vendor`, `architecture`,
`isFallbackAdapter`), seine Features und die WGSL-Sprachfeatures. Dann je Datei (Schlüssel: der
Pfad im Korpus): `rule` und `expect` aus der Datei, `verdict` (`valid`, `error`,
`pipeline-error`), alle Meldungen von `getCompilationInfo()` mit `type`, `lineNum`, `linePos`,
`offset`, `length`, `text` (auch Warnungen und Hinweise; `offset` und `length` zählen wie in
WebGPU UTF-16-Code-Einheiten, Zeile und Spalte beginnen bei 1), die Pipelines je Entry Point mit ihren
Fehlern (`kind`: `validation`, `internal` oder `exception`). Am Ende die Zählung und die
Abweichungen von `expect`. Die Datei enthält nichts Zeitabhängiges: zwei Läufe mit demselben
Chrome geben dieselben Bytes. Der Ordner steht in `.gitattributes` auf `-text`: Git ändert
keine Zeilenenden, die Offsets passen auf jedem System, und `lex/ok_blankspace_tab_cr.wgsl`
behält seine CR.

Wie das Werkzeug die Pipelines baut (Layout `auto`, Platzhalter-Ziele, ein erzeugter
Vertex-Shader für allein stehende Fragment-Shader) und was Chrome dafür braucht, steht im Kopf
von `tools/wgsl_ref.mjs`.

## Neu erzeugen

```text
set PLAYWRIGHT_MODULE=<Pfad zum Playwright-Paket>      (oder Playwright über npm auffindbar)
node tools/wgsl_ref.mjs --corpus
```

Das schreibt `tools/wgsl_corpus/expected.json` neu (rund 15 Sekunden). Chrome muss installiert
sein (oder `CHROME_PATH`), headless genügt, keine Flags. Ein neues Chrome kann Urteile ändern:
dann die Unterschiede in `expected.json` ansehen, bevor sie eingecheckt werden. Einzelne
Dateien: `node tools/wgsl_ref.mjs DATEI.wgsl ...` (JSON auf stdout).

Stand der eingecheckten Datei: Chrome 156.0.8078.76, Windows 11, GeForce RTX 2070 SUPER
(Adapter `nvidia` / `turing`), 389 gültig, 490 Fehler, 53 Pipeline-Fehler. Fälle, deren Urteil
vom Gerät abhängt (`@subgroup_size` außerhalb des Adapter-Bereichs), stehen nicht im Korpus.

## Wo Tint anders urteilte als erwartet

Beim ersten Lauf wichen 13 Fälle von unserer Erwartung ab. In 12 lag die Erwartung falsch oder
die Regel gehört zur WebGPU-API statt zu WGSL; diese Fälle tragen jetzt Tints Urteil als
Erwartung (die Regelzeile erklärt es). Einer bleibt als Abweichung stehen, dazu seit G2d 13 Dateien
aus `limits`: JFLib nimmt die Mindestgrenzen der Spezifikation als Regel, die Chrome überschreitet
(`docs/WGSL.md`, „Grenzen“); sie heißen `err_` und stehen in `summary.mismatches`.

| Fall | erwartet | Tint | Bemerkung |
| --- | --- | --- | --- |
| `lex/err_float_leading_zero_suffix` | Fehler | gültig | `01f` passt auf keine Form der Grammatik für dezimale Gleitkommaliterale (`0f` und `[1-9][0-9]*f` ohne Punkt oder Exponent); Tint nimmt es an. **Bleibt als Abweichung**: JFLib folgt der Spezifikation und lehnt es ab. |
| `consteval/ok_i32_add_wraps`, `ok_i32_mul_wraps`, `ok_i32_neg_wraps`, `ok_u32_sub_wraps` | Fehler | gültig | Tint rechnet konkretes `i32` / `u32` auch in Konstantenausdrücken modulo 2³² (`const_assert 2147483647i + 1i < 0i` gilt). Fehler gibt es nur für abstrakte Ganzzahlen, beim Umwandeln, bei Division durch 0, `i32 min / -1` und bei Shifts, die das Vorzeichen ändern. |
| `override/ok_overflow_at_pipeline` | Pipeline-Fehler | gültig | dasselbe für Override-Ausdrücke bei der Pipeline-Erzeugung |
| `addr/err_address_of_component` | gültig | Fehler | `&v.x` ist verboten: von einer Vektorkomponente gibt es keinen Zeiger |
| `behav/err_infinite_loop_void` | gültig | Fehler | eine Schleife ohne Ausgang (`loop {}`) ist ein Fehler („loop does not exit“), auch in einer Funktion ohne Ergebnis |
| `layout/err_align_too_small_unused` | gültig | Fehler | `@align` muss ein Vielfaches der Ausrichtung des Typs sein, auch wenn das Struct nie in einem Puffer liegt |
| `layout/ok_uniform_too_big` | Pipeline-Fehler | gültig | die Größe eines Uniform-Puffers prüft WebGPU erst beim Anlegen einer Bind Group, nicht bei der Pipeline mit Layout `auto` |
| `tex/pipe_storage_read_write_rgba8` | Fehler | Pipeline-Fehler | WGSL nimmt `read_write` mit jedem Format an; das Kerngerät lehnt rgba8unorm erst im Bind-Group-Layout ab |
| `tex/pipe_storage_bgra8_core` | Fehler | Pipeline-Fehler | ebenso `bgra8unorm` ohne das Feature `bgra8unorm-storage` |
| `requires/ok_immediate_address_space` | gültig | Pipeline-Fehler | unser erster Fall nutzte 80 Byte `var<immediate>`; die Grenze ist 64 Byte. Der Fall nutzt jetzt 32 Byte, `requires/pipe_immediate_too_big` prüft die Grenze |

Weitere Beobachtungen, die für den Übersetzer wichtig sind: Tint meldet unerreichbaren Code als
Warnung (`behav/ok_unreachable_after_return`), unbekannte Diagnose-Regeln als Warnung
(`diag/ok_unknown_rule`); Overrides mit `@id` werden nur über die Zahl gesetzt, nicht über den
Namen (`override/pipe_given_by_name_with_id`); Division durch 0 in Override-Ausdrücken und eine
Workgroup-Größe 0 meldet Dawn erst beim Erzeugen des HLSL („An error occurred while generating
HLSL“, Art `validation`); `textureBarrier`, `var<immediate>`, Swizzle-Zuweisungen, Texturen in
`let` und Zeigerparameter auf Speicherpuffer funktionieren ohne `requires`.

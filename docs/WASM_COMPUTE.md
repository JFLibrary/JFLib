# WASM SIMD und parallele Jobs

Stand: 12. Oktober 2026. Alle Pfade und Befehle relativ zu `RustLib/`.

## Bauen und starten

```powershell
rustup target add wasm32-unknown-unknown
rustup component add rust-src
python build_rs.py app PaintCraft --target web --out build/wasm-compute/after -j 4
python tools/serve_web.py build/wasm-compute/after/web/PaintCraft --port 8766
```

Der Standard `--web-mode auto` baut `APP.wasm`, `APP.simd.wasm` und
`APP.threads.wasm` mit einem gemeinsamen Host und `variants.json`. Einzelne Varianten
können mit `--web-mode scalar|simd|threads` gebaut werden. Die drei Bibliothekscaches und
Linkverzeichnisse sind getrennt; gleichzeitige Links überschreiben keine temporären
Objektdateien. Ein separater `--out`-Ordner hält Vergleichsbuilds reproduzierbar.
Der Build-Manager verwendet dieselbe Auswahl und übernimmt pro Stil ein vollständiges
Bundle mit allen drei Varianten. Mehrere Stile erhalten eigene Bundle- und Linkordner.

Für den Thread-Build baut `tools/web_sysroot.py` die mit Rust gelieferten Bibliotheken
`core`, `compiler_builtins` und `alloc` erneut mit Atomics, Bulk Memory und SIMD128.
Das verwendet direkt `rustc`, kein Cargo und keine zusätzlichen Crates. Die internen
Rust-Bibliotheken erfordern `RUSTC_BOOTSTRAP=1`, ausschließlich für diese drei Aufrufe.
Der Cache ist an Compiler-Version und Buildrezept gebunden. Rust bezeichnet das
`atomics`-Target-Feature weiterhin als instabil; ein Toolchainwechsel verlangt eine
erneute Prüfung. Der Linker prüft die Features aller Objekte; es gibt kein Umgehen
dieser Prüfung.

## SIMD128

`lib/simd/wasm.rs` implementiert alle 112 Methoden des vorhandenen `Simd`-Traits mit
128-Bit-WASM-Registern. `dispatch` und der historische Baseline-Name `Sse2` wählen im
SIMD-Bundle diesen Backend. Skalar- und native SSE2-/AVX2-Backends bleiben verfügbar.
Das umfasst gepackte Pixel, saturierende Arithmetik, Shuffles, Integer-Produkte und
f32-/f64-Vektoren. Überschiebungen, Min/Max mit ungeordneten Werten und ungültige
Float-Konvertierungen werden an den bestehenden SSE-Vertrag angepasst. Relaxed SIMD
und FMA werden nicht aktiviert.

Der Browser prüft SIMD mit einem kleinen WASM-Modul. Dadurch hängt die Auswahl von
den tatsächlichen Features statt vom Browsernamen ab. Für kontrollierte Tests:

- `?compute=scalar`: bisheriger Skalarpfad, kooperative Jobs.
- `?compute=simd`: SIMD128, kooperative Jobs.
- `?compute=threads&workers=4`: SIMD128, vier zusätzliche Rechen-Worker.
- Ohne Parameter: Threads, sonst SIMD, sonst Skalar, soweit im Bundle vorhanden.

`window.__jfguiCompute` zeigt die gewählte Variante und die Zahl bereitgestellter
Rechen-Worker. Die App kann über ihre normale Thread-Einstellung weniger verwenden.

## Gemeinsamer Speicher und Lebensdauer

`web/worker.js` kompiliert das Modul einmal, erzeugt einen gemeinsamen
`WebAssembly.Memory` und instanziiert es in den Rechen-Workern. Alle Worker melden
Bereitschaft, bevor der App-Einstieg aufgerufen wird. Während synchroner Rust-Aufrufe
werden keine neuen Worker gestartet: der Host vergibt bereits bereitstehende Worker.
Damit hängen ihre Initialisierung und Modulnachrichten nicht am blockierten App-Worker.

Der Thread-Build importiert und exportiert den gemeinsamen Speicher. LLD initialisiert
statische Daten atomar einmalig. Vor jedem Thread-Einstieg setzt der Host dessen
instanzeigenen `__stack_pointer` auf einen exklusiven, ausgerichteten Stack.
`Thread::join` wartet auf ein Atomics-Signal, das JavaScript erst **nach** der Rückkehr
aus WASM schreibt. Erst dann werden Closure-Record und Stack freigegeben. Worker können
für einen späteren Pool-Start wiederverwendet werden.

`web_threads.rs` liefert Semaphoren über `Atomics.wait/notify`; gewartet wird nur in
Workern. `pool.rs` behält seine Regeln für geliehene Closures, aktive Teilnehmer,
Abbruch, verschachtelte Schleifen und Abschluss bei. Die vorhandenen Allocator-Locks
schützen gemeinsame Freilisten und `memory.grow`. Zeitmessungen aller Worker verwenden
dieselbe monotone Basis. Die UI läuft nur im App-Worker. Ein vom Rechen-Worker abgefangener
WASM-/JavaScript-Fehler wird über ein gemeinsames Fehlersignal auch während blockierender
Warteoperationen erkannt; die Sitzung meldet den Fehler statt mit unvollständigen Ergebnissen fortzufahren.
Kachel-Weckmeldungen werden in einem atomaren Flag zusammengefasst und erst im
App-Worker in ein Ereignis umgesetzt. Tausende fertige Kacheln können dadurch keine
Maus-/Tastaturereignisse aus dem begrenzten Eingabering verdrängen.

Der Browserhost stellt standardmäßig `min(hardwareConcurrency - 1, 8)`, mindestens
einen, zusätzliche Worker bereit. PaintCrafts vorhandene Kachel-Jobs und parallele Zeilen-
und Bandverarbeitung profitieren ebenso wie Aufrufer derselben API in anderen Apps.
Codeabschnitte ohne SIMD-Kerne oder parallele Jobs werden dadurch nicht automatisch
parallel. Der gemeinsame wasm32-Speicher beginnt bei 32 MiB und ist auf 2 GiB begrenzt;
mehrere Bildebenen, Effektkopien und Undo benötigen entsprechend Platz.

## Hosting und Fallback

Threads benötigen einen sicheren Kontext (auch localhost) sowie
`Cross-Origin-Opener-Policy: same-origin` und
`Cross-Origin-Embedder-Policy: require-corp`. Der mitgelieferte lokale Server setzt beide
Header. Auf einem eigenen Server müssen sie für die App und ihre Worker korrekt
ausgeliefert werden. Ohne Cross-Origin-Isolation lädt der automatische Host die SIMD-
Variante ohne gemeinsamen Speicher. Ein explizit erzwungener, nicht verfügbarer Modus
meldet einen Fehler. Ein altes Bundle ohne Manifest bleibt über `APP.wasm` verwendbar.

Primärquellen: [Rust WASM-Target](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html),
[Rust WASM-Intrinsics](https://doc.rust-lang.org/core/arch/wasm32/index.html),
[MDN: SharedArrayBuffer und Isolation](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer).

## Reproduzierbare Prüfungen

```powershell
python build_rs.py all --target web --out build/wasm-compute/after -j 4
node tools/tests/web_compute.mjs build/wasm-compute/after
node tools/tests/web_apps.mjs build/wasm-compute/after --engine=chromium,firefox --only=template,gallery,paint,PaintCraft,jfWord,officeicontest --isolated=1
node tools/tests/web_frames.mjs build/wasm-compute/after
node apps/PaintCraft/tests/security_web.mjs build/wasm-compute/after --deeper --isolated
node apps/PaintCraft/tests/wasm_compute.mjs build/wasm-compute/after
```

`portabletest` vergleicht SIMD gegen den Skalar-Corpus, der auch gegen native SSE2/AVX2
geprüft wird. Der Thread-Test erzwingt mindestens vier zeitlich überlappende Aufgaben,
prüft getrennte Scratch-Slots, verschachtelte Aufrufe, konkurrierende Allokationen,
Fortschritt, Abbruch, `set_count` und Shutdown/Neustart. Browserprüfungen umfassen
Chromium und Firefox sowie automatischen Fallback ohne Isolation.

Der PaintCraft-Vergleich erzeugt deterministische 2048×2048- und 4096×3072-Bilder und
verwendet die normale Oberfläche: Gauß-Radius 12 und Bewegungsunschärfe mit Entfernung
32. Gemessen wird vom Effektbefehl bis zur als fertig gemeldeten Vorschau, einschließlich
Jobplanung, Komposition und UI-Abfrage (30-ms-Abfrageintervall). Der Zeitpunkt des
tatsächlichen Bildschirm-Refreshs wird nicht gemessen. Nach einem Aufwärmdurchlauf folgen drei Messungen; berichtet
wird der Median. Commit, PNG-Export und Undo liegen außerhalb dieses Zeitfensters.
SHA-256 der exportierten PNG-Dateien muss über alle Varianten und Browser übereinstimmen.
Die Rohdaten samt Browser-Version, CPU, Einzelzeiten und Prüfsummen stehen nach dem Lauf
unter `build/wasm-compute/after/PaintCraft-compute/results.json`.
Ein unterbrochener Lauf kann mit `RESUME=1` bei unveränderten Bundles fortgesetzt werden;
abgeschlossene Fälle werden übernommen, CPU, Browser-Version und Bildprüfsummen geprüft.
Die Ergebnisdatei wird nach jedem abgeschlossenen Fall atomar ersetzt.

Der Vergleich wartet vor Messungen auf laufende `rustc`-Prozesse und verwirft Proben,
bei deren Ende ein Compiler aktiv ist. Nur der lokale Benchmark-Server verlängert das
Agenten-Anfragelimit von 30 auf 240 Sekunden für langsame Skalar-Aufwärmdurchläufe;
der ausgelieferte Browserhost bleibt unverändert.

## Gemessene PaintCraft-Leistung am 12. Oktober 2026

AMD Ryzen 5 3600, 6 Kerne / 12 logische CPUs; Windows; Chromium 155.0.8059.39
und Firefox 153.0, jeweils headless. Entwicklungskonfiguration: Rechenkerne mit
`opt-level=2`, UI mit `opt-level=1`, kein LTO. Vier zusätzliche Rechen-Worker.
Alle Varianten stammen aus demselben Quellstand und verwenden dieselben Bilder
und Effekte. Die Zahlen sind Mediane von je drei Messungen in Sekunden.

| Browser | Bild | Effekt | Skalar | SIMD128 | SIMD + 4 Worker | Faktor ggü. Skalar |
|---|---|---|---:|---:|---:|---:|
| Chromium | 2048 × 2048 | Gauß, Radius 12 | 2,53 | 0,74 | 0,32 | 8,0× |
| Chromium | 2048 × 2048 | Bewegungsunschärfe, Entfernung 32 | 2,46 | 0,79 | 0,29 | 8,4× |
| Chromium | 4096 × 3072 | Gauß, Radius 12 | 7,17 | 1,42 | 0,56 | 12,8× |
| Chromium | 4096 × 3072 | Bewegungsunschärfe, Entfernung 32 | 7,99 | 1,04 | 0,47 | 17,2× |
| Firefox | 2048 × 2048 | Gauß, Radius 12 | 21,25 | 3,86 | 1,09 | 19,5× |
| Firefox | 2048 × 2048 | Bewegungsunschärfe, Entfernung 32 | 19,89 | 3,24 | 0,92 | 21,6× |
| Firefox | 4096 × 3072 | Gauß, Radius 12 | 74,18 | 14,38 | 3,91 | 19,0× |
| Firefox | 4096 × 3072 | Bewegungsunschärfe, Entfernung 32 | 93,95 | 21,53 | 2,62 | 35,9× |

Die großen Firefox-Skalarwerte streuten deutlich: Gaussian blur 66,44–102,20 s; Motion blur 74,10–123,14 s.
Die Messung fand im normalen Desktopbetrieb statt. Der Compilerfilter prüft vor
und nach einer Probe; er überwacht nicht lückenlos den gesamten Zeitraum. Die
Geschwindigkeitsfaktoren sind daher Orientierungswerte für diesen Lauf.

Alle 24 Vergleichsfälle bestanden. Die gespeicherten PNG-Dateien sind für jede
Kombination aus Bild und Effekt über beide Browser und alle drei Backends bytegleich.
Der Gewinn ist abhängig von Browser, Bild und Effekt; andere Rechner und Release-Builds
können andere Zeiten liefern. Der Vergleich misst die gesamte Vorschau, keinen isolierten
Rechenkern. Insbesondere kleine Zeitwerte enthalten UI- und Abfragekosten.

[Rohdaten mit Einzelzeiten, Browser-Versionen, PNG-Prüfsummen und WASM-Bundle-Prüfsummen](WASM_COMPUTE_RESULTS.json).

Zusätzlich bestanden 10 Backend-/Fallback-Prüfungen, 20 App-/DPI-Prüfungen
(template, gallery, paint, PaintCraft, jfWord, officeicontest), 18 PaintCraft-Prüfungen
zu Bearbeitung, Undo/Redo und dauerhaftem Speichern sowie 9 Frame-/Eingabe-Prüfungen.
Vier weitere Build-Manager-Integrationsprüfungen bestanden; ein echter Build für
WinUI und KDE lieferte sechs validierte WASM-Dateien mit getrennten Stil-Bundles.
Der Parallelitätstest beobachtete fünf Ausführungskontexte und erzwang vier
gleichzeitig laufende Aufgaben. Der portable SIMD-Corpus stimmte mit dem nativen
SSE2-/AVX2-Corpus überein (`5a22a1f803872742`).

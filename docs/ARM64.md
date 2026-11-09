# ARM64-Plan: Windows und Linux auf ARM-Prozessoren

Stand 2026-11-03, auf Basis von `main` 7074d800. Bisher sind nur ARM0, ARM1 und ARM6 umgesetzt, und von ARM1
lief noch nichts auf ARM. Für den Rest gilt: Die Zahlen sind aus dem Code gezählt, die Aufwände geschätzt. Syscalls,
Flags und `struct stat` sind mit ARM1 gegen die Kernel-Header geprüft (`tools/kernel_table_check.py`), die
Relocations noch nicht. Der Eintrag in der Roadmap steht in
[ROADMAP.md](ROADMAP.md) §9.

## Ziel

Alle Programme, die heute für Windows und Linux auf x86-64 laufen, laufen auch auf
`aarch64-pc-windows-msvc` und `aarch64-unknown-linux-gnu`. Gebaut wird weiter vom x86-PC aus, ein
ARM-Rechner ist nur zum Testen nötig. Die x86-64-Programme ändern sich dabei nicht.

Nicht Teil dieses Plans: 32-Bit-ARM, ARM64EC (x64-Plugins in einem ARM-Prozess), macOS und iOS.
macOS und iOS bleiben eigene Punkte der Roadmap, profitieren aber von NEON, vom ARM-Assembler und
von der Plugin-ABI.

## Ausgangslage

Schon vorhanden:

- **Android ARM64** baut, dafür gibt es `lib/rt/portable/` (`cpu`, `fx`, `math`, `mem`). Android
  nutzt aber Bionic statt eigener Syscalls, der Linux-Pfad ohne libc ist also neu.
- **`lib/simd`** hat mit `neon.rs` ein Backend für ARM64 (ARM6).
- **`lib/gpu`** enthält keinen Architektur-Code. Nur die Syscall-Nummern in `vk_dmabuf.rs` hängen an
  x86-64.
- **`lib/update/manifest.rs`** kennt `windows-aarch64` und `linux-aarch64`.
- **Werkzeuge:** Cross-Build für Linux läuft heute schon von Windows aus (`ld.lld`, eigene
  glibc-Stubs), Windows importiert über `raw-dylib`, und die ARM64-Bibliotheken des SDK liegen unter
  `Windows Kits/10/Lib/10.0.26100.0/um/arm64`. In WSL sind `qemu-aarch64`, `qemu-system-aarch64`
  (8.2.2) und die Firmware AAVMF installiert.

Nur auf x86-64 ausgelegt:

| Bereich | Umfang | Wo |
|---|---|---|
| Syscall-Nummern als Konstanten | 178 Stellen in 29 Dateien | u. a. `plat/fs.rs`, `plat/net.rs`, `plat/linux_x11.rs`, `rt/os.rs`, `gpu/vk_dmabuf.rs` |
| Syscall-Nummern als Zahl im Aufruf | 87 Stellen in 11 Dateien | vor allem `plat/process_linux.rs` (41), `plat/control_ipc_linux.rs` (22) |
| Syscalls, die es auf ARM64 nicht gibt | `open`, `poll`, `fork`, `mkdir`, `unlink`, `rename`, `readlink`, `access`, `dup2`, `chmod`, `arch_prctl` | wie oben |
| Kernel-Layouts und Flags | `struct stat`, `O_DIRECTORY`, `O_NOFOLLOW`, `clone` | `plat/fs.rs`, `plat/linux_sys.rs`, `plat/linux_sandbox.rs`, `plat/linux_thread.rs` |
| Assembler Linux | `_start` (2), Thread-Start, `sigreturn` (2) | `plat/linux.rs`, `linux_glibc.rs`, `linux_thread.rs`, `linux_crash.rs`, `linux_guard.rs` |
| Assembler `rt` | `memcpy` und Verwandte, `cpuid`, x87-Mathematik, `__chkstk`, Syscall | `rt/mem.rs`, `rt/cpu.rs`, `rt/x87.rs`, `rt/fx.rs`, `rt/os.rs` |
| Guard um Plugin-Aufrufe | Trampolin und Test-Code | `plat/plib.rs` (2 Blöcke) |
| Assembler Plugins und Module | `memcpy` und Verwandte, `ud2`, `prepare_tramp` | `plugin/mem.rs`, `plugin/lib.rs`, `plugin/fx.rs`, `modules/modrt/lib.rs` |
| Plugin- und Modul-ABI | 90-mal `extern "win64"` in 13 Dateien, auch auf Linux | `modules/`, `plat/modabi.rs`, `plat/modhost.rs`, `plat/plib.rs`, `apps/plattest/` (PaintCraft's plugins: since 2026-11 in JFLibrary/JFPainter) |
| Eigene Loader | nur `EM_X86_64` und `R_X86_64_*`, nur PE-Machine `0x8664` | `plat/linux_elfload.rs`, `plat/win_lib.rs`, `pe_verify` / `elf_verify` in `build_rs.py` |
| Crash-Berichte | `CONTEXT`, `ucontext`, DWARF-Register, Erkennen von `call` | `plat/win_crash.rs`, `plat/linux_crash.rs` |
| Sandbox | `AUDIT_ARCH_X86_64`, Liste der Syscall-Nummern | `plat/linux_sandbox.rs` |
| Pakete | `amd64`, `x86_64`, `x64` fest eingetragen | `tools/package.py`, `pkg_linux.py`, `pkg_msix.py`, `linux_task.py` |

## Entscheidungen

Das sind Vorschläge, die der Plan voraussetzt. Abweichungen ändern vor allem ARM0 und ARM4.

1. **Architektur als eigene Option:** `build_rs.py ... --arch x86_64|aarch64`, Standard `x86_64`, wie
   schon `--android-abi`. `TARGETS` der Apps bleibt unverändert. Ausgabe nach `build/win-arm64/` und
   `build/linux-arm64/`.
2. **Eine Kernel-Tabelle:** Nummern, Flags und Layouts stehen pro Architektur an einer Stelle in `rt`.
   `plat` und `gpu` definieren keine eigenen mehr.
3. **Alte Syscalls überall ersetzen:** `openat`, `ppoll`, `clone`, `mkdirat`, `unlinkat`, `renameat2`,
   `readlinkat`, `faccessat`, `dup3`, `fchmodat` auch auf x86-64. So gibt es nur einen Codepfad, und
   die bestehenden x86-Tests prüfen ihn mit. Die Sperrliste der Sandbox behält auf x86-64 auch die
   alten Aufrufe.
4. **Plugin-ABI:** x86-64 bleibt bei `win64`, vorhandene Plugins laufen weiter. ARM64 nutzt `extern "C"`,
   ein Makro wählt das aus. Plugins sind pro Architektur eigene Dateien, ein x86-Plugin wird auf ARM
   mit klarer Meldung abgelehnt.
5. **Eine Datei pro Architektur:** getrennte Setup-, MSIX-, deb- und AppImage-Dateien, kein gemeinsames
   Setup.
6. **Mathematik:** ARM nutzt `rt/portable/math.rs` wie Android. Das ist nicht bitgleich zu x87, die
   Abweichungen werden gemessen und dokumentiert (ARM2).
7. **Seitengrößen:** 4 KB und 16 KB werden unterstützt, die Größe kommt zur Laufzeit aus `AT_PAGESZ`.
   64 KB wird abgelehnt oder später nachgezogen.

## Meilensteine

Größe: S klein, M mittel, L groß, jeweils nur relativ zueinander.

### ARM0: Build-Grundlage (S)

- `rustup target add aarch64-pc-windows-msvc aarch64-unknown-linux-gnu`.
- `--arch` in `build_rs.py`: `TRIPLE`, Ausgabeordner, `GLIBC_INTERP` (`/lib/ld-linux-aarch64.so.1`),
  `GLIBC_VERSION` (`GLIBC_2.17`), die `rustc`-Aufrufe der Stubs und Plugins, `text_shape` über Cargo.
- Nebenprobe: die heutige x64-Version unter Emulation auf `windows-11-arm` starten. Läuft sie, gibt es
  für Windows auf ARM sofort eine Zwischenlösung.

**Fertig, wenn:** `build_rs.py lib --target linux --arch aarch64` startet und die Fehlerliste den
Stellen dieses Plans entspricht, und die x86-Programme bytegleich zu vorher sind.

**Umgesetzt:** `--arch` gilt für `lib`, `app`, `all` und `plugin`, die Ausgabe liegt in `win-arm64` und
`linux-arm64` unter dem jeweiligen Ausgabeordner (`build/`, `build/release/`, `--out`). `run`, `test` und `watch`
lehnen `--arch aarch64` ab, bis ARM2 den Test-Runner bringt, `--target web` und `android` ebenso.

- **x86-64:** `coretest`, `gallery`, `PaintCraft` mit `jxlenc`, alle elf Plugins und die glibc-Stubs sind im
  Release-Bau für beide Ziele Byte für Byte gleich, beide Stände aus demselben Verzeichnis gebaut. Unter Windows
  weichen die drei Programme nur im Zeitstempel des Linkers ab. Auch die Stamps sind gleich, also Quellen und Flags.
- **Linken:** Mit kleinen Programmen ohne die Bibliothek geprüft. Das statische läuft unter `qemu-aarch64` und
  liefert den erwarteten Exit-Code. Das gegen die Stubs gelinkte nennt `/lib/ld-linux-aarch64.so.1` und
  `exit@GLIBC_2.17`, das für Windows hat die PE-Machine `0xAA64`; diese beiden wurden nicht gestartet.
  `text_shape` baut über Cargo für beide Ziele.
- **Nebenprobe:** `.github/workflows/arm64-emulation.yml` startet nur per Hand. Der Workflow baut `coretest`, `plattest`
  und `gallery` auf x64 und lässt ihre Skripte auf x64 und auf `windows-11-arm` (Cobalt 100) laufen, dort unter
  Emulation. Auf x64 sind 85 von 85 grün, unter Emulation 84 von 85, darunter alle 42 der `gallery`, die auch
  richtig zeichnet. Als Zwischenlösung taugt die x64-Version also. Nicht geprüft: der Release-Bau, andere Programme,
  ein echtes Gerät.
  - Die x87-Register der Emulation halten nur Doubles, also 53 Bit Mantisse und den Exponenten eines Double, auch
    wenn das Kontrollwort 64 Bit verlangt. Daran scheiterte zuerst `plattest/ini.uis`: `5e-324` wurde zu `0e-293`,
    `1e-300` zu `2.3372036854775808e-292`. Eine Nachrechnung mit reinen Doubles traf alle zehn Texte aus dem Lauf.
    Jetzt prüft `lib/base/num.rs` die Einheit beim ersten Gebrauch und nimmt ohne echtes x87 `rt::portable::math`,
    damit ist `ini.uis` auch unter Emulation grün. Auf der Einheit bleiben dort `log10_round`, dessen Abweichung
    `f64_format` selbst korrigiert, und `pow`, `exp2`, `log2`, `sin`, `cos` und `atan2`. Diese dürften ungenauer
    rechnen, gemessen ist das nicht; `coretest` bleibt grün.
  - `plattest/screen.uis` scheitert am Runner. Über dem ganzen Bildschirm liegt dort die Windows-Einrichtung
    („Choose privacy settings“, Bild `desktop.png` des Laufs) und verdeckt das Testfenster. Aufnahme und Farbwahl
    lassen sich so auf diesem Runner nicht prüfen.

**Fehlerliste, Stand ARM0.** `build_rs.py lib --target linux --arch aarch64` startet und endet in `rt` mit 150
Fehlern, alle aus ARM2:

| Datei | Fehler | Was |
|---|---|---|
| `rt/x87.rs` | 128 | Register `st(0)` bis `st(7)`, Modifizierer `e`; ARM nimmt `portable/math.rs` |
| `rt/mem.rs` | 9 | `rep movsb`, `rep stosb` |
| `rt/os.rs` | 9 | `syscall` |
| `rt/cpu.rs` | 4 | `core::arch::x86_64`, `xgetbv`; ARM nimmt `portable/cpu.rs` |

Mit `--target win` sind es dieselben ohne `rt/os.rs`, also 141. `build_rs.py plugin --arch aarch64` endet für beide
Ziele mit 25-mal `win64` in `lib/plugin/` (ARM4).

Hinter `rt` kommt der echte Bau nicht. Um den Rest zu sehen, lief jede Crate, jedes Programm und jedes Plugin einzeln
durch `rustc`, ohne zu linken, auf einer Wegwerf-Kopie: `rt` mit den portablen Dateien, `svc 0` und Schleifen statt
`rep movsb`, `extern "C"` statt `win64`. Diese Notlösungen sind nicht im Repository und wurden nie gestartet. Dabei
kam noch dies:

| Stelle | Meldung | Im Plan |
|---|---|---|
| `plat/modabi.rs`, `modhost.rs`, `plib.rs` | 26-mal `win64` | ARM4 |
| `plat/linux_glibc.rs:167` | `SYS_ARCH_PRCTL` fehlt, auf ARM64 gibt es den Aufruf nicht | ARM3 |
| `apps/plattest`, `apps/linuxtest/t_files.rs`, PaintCraft `src/diagnostics/crash.rs` | `plat::crash_test`, `crash_test_caller`, `avx2_probe`, `stack_aligned` fehlen | ARM3, `linuxtest` und PaintCraft nachgetragen |
| `plat`, `global_asm!` | x86-Befehle, erst beim Erzeugen des Codes | ARM3 |
| `rt/os.rs:77` | `__chkstk` (nur Windows), erst beim Erzeugen des Codes | ARM5 |
| `plugin/fx.rs`, `plugin/mem.rs`, `plugin/lib.rs:168` | `prepare_tramp`, `memcpy` und Verwandte, `ud2` | ARM4, `mem.rs` und `ud2` nachgetragen |
| `modules/modrt` | 139 Fehler in den per `#[path]` eingebundenen `rt/cpu.rs`, `rt/fx.rs`, `rt/x87.rs` und in `lib.rs:232`; `lib.rs:75` beim Code | ARM3 nennt nur `modrt`, Einzelheiten unter ARM4 nachgetragen |
| `plugins/crash/lib.rs:22`, `plugins/crashpf/lib.rs:38` | x86-Befehle | ARM3 |

Alles andere baut bis zum Objektcode: für Linux 64 Crates und 15 statische Programme, gegen glibc 64 Crates und 48
Programme, für Windows 63 Crates und 63 Programme, darunter `simd`, `gpu`, `ui` und alle Styles. Neue Stellen außerhalb von
`rt`, `plat`, `lib/plugin`, `modules` und den genannten Programmen gibt es nicht. Viel mehr zeigt der Compiler aber
nicht: Bei `global_asm!` hört LLVM nach dem ersten Fehler einer Codeeinheit auf, deshalb sind die
Assembler-Stellen zusätzlich aus den Quellen gezählt, und die Fallen weiter unten findet er gar nicht.

### ARM1: Kernel-ABI an einer Stelle, noch ohne ARM (L)

- Nummern, `O_*`, `stat`-Felder und `clone`-Reihenfolge in eine Tabelle in `rt`, alle 265 Stellen darauf
  umstellen.
- Alte Syscalls nach Entscheidung 3 ersetzen, `poll` als Helfer mit Millisekunden.
- Lint-Regel: keine Zahl als erstes Argument von `syscall*`, kein `const SYS_*` außerhalb der Tabelle.

**Fertig, wenn:** die Linux-Tests auf x86-64 grün sind (`test --quick`, dazu `plattest`, `linuxtest`,
`sandboxtest`, `lifetest`). Dieser Schritt lässt sich ohne ARM mergen und nützt auch so. Er fasst rund
40 Dateien an, also vorher prüfen, welche Sessions gerade in `lib/plat` arbeiten, und schnell mergen.

**Umgesetzt:** Die Tabelle ist [`lib/rt/kernel.rs`](../lib/rt/kernel.rs), auch erreichbar als `rt::os::*`. Dort
stehen außerdem die Helfer `poll`, `fork`, `clone`, `dup2` und `rename`. `tools/kernel_table_check.py` vergleicht
beide Tabellen mit den Kernel-Headern des Android-NDK, `plattest --test kernel` prüft Flags, `struct stat`, die
`clone`-Reihenfolge und die Helfer auf dem laufenden Kernel. Die Lint-Regel prüft auch, dass beide Tabellen
dieselben Namen enthalten. Die Sperrliste der Sandbox enthielt keinen der ersetzten Aufrufe, sie hat auf x86-64
dieselben Nummern in derselben Reihenfolge wie vorher und kommt jetzt aus der Tabelle.

### ARM2: `rt` auf Linux ARM64 (M)

- `svc 0` statt `syscall`, `_start`, Thread-Start, `rt/lib.rs` wählt für ARM `portable/cpu.rs`,
  `portable/math.rs` (`aarch64/fx.rs` wählt es schon, seit ARM6).
- `rt/mem.rs` trennen: Der Heap mit den 64 Shards gilt für alle, `memcpy` und Verwandte sowie `load8`
  kommen pro Architektur.
- Test-Runner: `build_rs.py test --arch aarch64` startet Programme über `qemu-aarch64` in WSL. Das geht für
  statische Programme; für gegen glibc gelinkte fehlt dort eine glibc für ARM64.
- Outline-Atomics prüfen: `_start` führt ohne libc keine Initialisierung aus, also klären, ob die
  LSE-Erkennung läuft.

**Fertig, wenn:** `coretest` und `portabletest` unter `qemu-aarch64` grün sind und die Abweichungen der
Mathematik zu x86-64 gemessen sind. Zusätzlich ein statisches `coretest` per `adb` auf dem A56 starten:
Das wäre echte ARM-Hardware ohne Zukauf (ob `/data/local/tmp` das zulässt, zeigt der Versuch).

### ARM3: `plat` auf Linux ARM64 (L)

- Signale: `sigreturn` (`mov x8, 139`, `svc 0`), `ucontext`, Guard-Trampolin mit `x19`-`x28`,
  `d8`-`d15`, `FPCR`.
- Crash-Berichte: DWARF-Register, `calls_into` erkennt `bl` und `blr`.
- ELF-Loader: `EM_AARCH64`, `R_AARCH64_RELATIVE`, `ABS64`, `GLOB_DAT`, `JUMP_SLOT`, Befehlscache leeren,
  Seitengröße zur Laufzeit.
- Sandbox: `AUDIT_ARCH_AARCH64` (`0xC00000B7`), die Sperrliste kommt schon aus der Tabelle.
- glibc-Variante: `_start`, `TPIDR_EL0` statt `arch_prctl`.
- `plat/cpu.rs` (Stack-Zeiger, AVX-Probe) und die Crash-Tests in `plat/lib.rs`, für die es den Zweig für
  Android schon gibt. `plattest` und `linuxtest` rufen sie auf (und JFPainter, früher PaintCraft, in seinem eigenen Repository).
- Testprogramme: `jfgui_gt_code` in `plib.rs`, `plugins/crash`, `plugins/crashpf`, `modrt`,
  `tools/memory_safety/native_probe.rs`.

**Fertig, wenn:** `plattest`, `linuxtest`, `lifetest` und `sandboxtest` in einer ARM-VM grün sind. Die
Sandbox-Tests müssen zeigen, dass jeder gesperrte Aufruf wirklich scheitert.

### ARM4: Plugin- und Modul-ABI (M)

- Makro für `extern "win64"` nach Entscheidung 4.
- `prepare_tramp` in `lib/plugin/fx.rs` gibt zwei Register zurück: auf ARM `x0` und `x1`, sauber über
  eine `repr(C)`-Struktur.
- `lib/plugin/mem.rs`: `memcpy` und Verwandte in Assembler, je ein Block für Windows und System V. Auf ARM64
  rufen beide Systeme gleich auf, einer reicht. `plugin::trap` in `lib/plugin/lib.rs` nutzt `ud2`.
- `modules/modrt` hat Kopien von `rt/cpu.rs`, `rt/fx.rs`, `rt/x87.rs` und `plugin/mem.rs` (`shared/`, `lint()` hält sie gleich) und bindet sie per `#[path]` ein, die
  Auswahl in `rt/lib.rs` gilt dort nicht. Dazu kommen `ud2` und der Testfehler `JXT_FAULT` in `modrt/lib.rs`.
- `build_rs.py plugin`, `pe_verify`, `elf_verify`, Meldung für Plugins der falschen Architektur.

**Fertig, wenn:** das Modul `jxlenc` und `plattest` (`t_guard`, `t_lib`) auf ARM64 und unverändert auf x86-64
laufen (JFPainters Plugins: in dessen Repository).

### ARM5: Windows ARM64 (M)

- `__chkstk` (Größe in `x15` mal 16, nur `x16` und `x17` frei), TEB über `x18` statt `gs:`
  (`win_crash.rs`, `win_lib.rs`).
- `CONTEXT` und `RtlVirtualUnwind` in `win_crash.rs`, Guard-Trampolin mit Unwind-Daten.
- PE-Loader: Machine `0xAA64`, `FlushInstructionCache`.
- `native_link_args`: Das Ziel braucht keine eigene Angabe, `lld-link` nimmt die Maschine aus den Objekten
  (ARM0, an einem kleinen Programm geprüft). Ob ein ganzes Programm so startet, zeigt erst dieser Schritt.
- `win_clr.rs` lädt die CoreCLR einer anderen Anwendung in den Prozess. Das geht nur, wenn sie auch
  ARM64 ist, bei einer x64-Runtime braucht es eine klare Meldung statt eines Fehlers beim Laden.

**Fertig, wenn:** `coretest`, `plattest` und ein Start der `gallery` auf `windows-11-arm` grün sind.

### ARM6: NEON für `lib/simd` (erledigt 2026-11-03)

- `lib/simd/neon.rs` nach dem Muster von `wasm.rs`: `Neon128 as Sse2`, `Scalar256 as Avx2`, ohne
  Erkennung zur Laufzeit. Die Ergebnisse sind Bit für Bit die von SSE2, auch bei NaN, Unendlich und
  Überlauf. Am Anfang der Datei steht, wo das mehr braucht als den naheliegenden Befehl.
- Der Preis der NaN-Regel: Jede Float-Rechnung prüft ihr Ergebnis (`fmaxv`, `fcmp`, `b.vs`), ein NaN nimmt
  einen kalten Pfad. Ohne das wäre `sqrtf(-1)` auf ARM `7fc00000` statt `ffc00000`.
- `portabletest` läuft von `adb shell` (`--target android`, `tests/android.py`) und prüft jede Operation
  gegen den skalaren Code, jetzt auch mit Grenzwerten: Sättigung, alle Schiebeweiten, Float-Operationen auf
  rohen Bits mit jedem Paar aus signierten Nullen, denormalen Zahlen, Unendlich und NaN. Derselbe Digest
  auf dem SM-A566B, unter Windows und Linux (SSE2, AVX2) und im Browser (Chromium, Firefox, Node).
- `copy_stream` und `stream_fence` bekommen nichts Eigenes: `stnp` ist nur ein Hinweis und bräuchte wegen
  seiner lockeren Ordnung eine Barriere, ein gewöhnliches Kopieren ganzer Zeilen erkennen ARM-Kerne selbst
  und legen sie dann nicht mehr im Cache ab (Write Streaming). Nur `render_par` ruft sie, und das läuft auf Android nicht
  (kein Pool). Übergeben wird über `SpinLock` und `parallel_for`, also mit Release und Acquire. Ob sich das
  mit Threads auf ARM auszahlt, lässt sich erst mit ARM3 messen.

Gemessen auf dem SM-A566B (Android 16, 120 Hz, Bitmap 768 × 1496), Entwicklungsbau, je drei Durchgänge
abwechselnd mit `apps/androidGallery/tests/android_perf.py`, Median der Durchgänge. Android Studio spiegelte
dabei den Bildschirm; `nativeRender` stört das kaum (vorher 5,7 ms wie ohne Spiegelung am 01.11.), die Werte von
`gfxinfo` taugen damit nur zum Vergleich untereinander:

| | Startseite vorher | nachher | „Werte & Farben“ vorher | nachher |
|---|---|---|---|---|
| `nativeRender` Median | 5,72 ms | 5,22 ms | 6,88 ms | 6,31 ms |
| `nativeRender` Mittel | 5,84 ms | 5,30 ms | 7,17 ms | 6,53 ms |
| `nativeRender` 95 % | 6,84 ms | 6,33 ms | 9,47 ms | 7,95 ms |
| davon Darstellen (Mittel) | 2,13 ms | 2,25 ms | 2,00 ms | 2,04 ms |
| ruckelnd (`gfxinfo`) | 2,6 % | 2,0 % | 17 % | 6 % |
| Bildzeit Median / 95 % | 11 / 13 ms | 11 / 13 ms | 13 / 17 ms | 11 / 15 ms |
| verpasste Vsyncs | 0 | 0 | 2 | 0 |

Alle zehn Seiten aus `GalleryPresentInstrumentation -e page NAME` haben vorher und nachher denselben
Bitmap-Hash. `coretest` und `gallery` sind für x86-64 im Release-Bau Byte für Byte gleich, unter Windows
bis auf den Zeitstempel des Linkers. Das gilt nur für Bauten aus demselben Verzeichnis: Die Metadaten der
von Cargo gebauten Abhängigkeiten von `text_shape` hängen vom Pfad ab, und das verschiebt `gallery`.

Das Rechnen ohne Darstellen wird also 18 % schneller (3,71 auf 3,05 ms), viel weniger, als die Roadmap
erwartet hatte: `simpleperf` (alle Threads der App, Startseite) zeigt, dass `gfx::blend::fill_over` von
12,2 auf 3,2 % fällt und `mask_over` von 3,4 auf 1,9 %, der Rest liegt nicht in `lib/simd`: `memmove` 32 %,
`plat::web::windows::to_rgba` 17 % (im Entwicklungsbau nicht vektorisiert, `plat` hat Stufe 1),
`rt::fx::sqrt_bits` und `rt::fx::nearest` zusammen 6 % (portabler Code, wo ARM `fsqrt` und `fcvtns` hat).

Diese letzten 6 % sind seitdem weg: `rt::fx` hat auf ARM64 eine eigene Datei mit den Befehlen der CPU
(`lib/rt/aarch64/fx.rs`, die Ergebnisse von SSE), `portabletest` vergleicht sie Wert für Wert mit
`portable/fx.rs`, auch um die Grenzen von `i32` und `i64` und bei exakten Halben. Gegen den Stand nur mit
NEON, gleiche Messung wie oben (die Zahlen schwanken von Tag zu Tag, deshalb immer im Wechsel):

| | Startseite vorher | nachher | „Werte & Farben“ vorher | nachher |
|---|---|---|---|---|
| `nativeRender` Median | 5,02 ms | 4,53 ms | 6,20 ms | 5,75 ms |
| `nativeRender` Mittel | 5,15 ms | 4,65 ms | 6,46 ms | 5,84 ms |
| `nativeRender` 95 % | 6,30 ms | 5,85 ms | 7,55 ms | 6,75 ms |
| ruckelnd (`gfxinfo`) | 2,2 % | 1,9 % | 4,8 % | 2,6 % |
| Bildzeit Median / 95 % | 11 / 13 ms | 10 / 13 ms | 11 / 14 ms | 11 / 13 ms |
| verpasste Vsyncs | 0 | 0 | 2 | 0 |

### ARM7: Atomics-Prüfung auf echter Hardware (M)

- Rund 280-mal `Ordering::Relaxed` in `lib/` durchsehen, zuerst `rt/sync.rs`, `plat/pool.rs`,
  `plat/sync.rs`, den Heap in `rt/mem.rs`, dann die Codecs und `gpu`.
- Stresstests mit `asynctest` und dem Pool auf einem ARM-Runner und dem A56.

**Fertig, wenn:** die Prüfung abgeschlossen ist und die Stresstests auf echter Hardware stabil laufen.
QEMU reicht dafür nicht, siehe Fallen.

### ARM8: Fenster und GPU (M)

- Wayland und X11 in der VM, `.uis`-Skripte der wichtigen Apps (`gallery`, `jfWord`; JFPainter in seinem Repository).
- `gputest` und `uigputest` mit lavapipe in der VM, Treiber auf echtem Gerät, sobald eines da ist.
- Windows: Verhalten, wenn `vulkan-1.dll` fehlt oder kein Treiber da ist, der Software-Renderer bleibt
  der Rückfall.

**Fertig, wenn:** die Skripte in der VM und auf `windows-11-arm` grün sind, soweit dort Fenster möglich
sind, und ein echtes Gerät von Hand geprüft ist.

### ARM9: Pakete, Updates, CI, Dokumentation (M)

- `.deb` `arm64`, RPM und AppImage `aarch64`, MSIX `arm64`, Flatpak `--arch=aarch64`, Setup pro
  Architektur, `apps/appimagert` für ARM, Agent-Ordner von BackupCraft, Update-Kanäle.
- `.github/workflows/arm64.yml`: Bauen auf einem x86-Runner wie lokal, Tests auf `ubuntu-24.04-arm` und
  `windows-11-arm`. Start nur per Hand (`workflow_dispatch`), bis der Lauf stabil ist.
- Erst jetzt README, [PACKAGING.md](PACKAGING.md), GUIDE und Website anpassen.

**Fertig, wenn:** `pkg_check.py` grün ist, ein Update auf ARM durchläuft und die Website nur beschreibt,
was getestet ist.

## Reihenfolge

```text
ARM0 -> ARM1 -> ARM2 -> ARM3 -> ARM4 -> ARM5 -> ARM7 -> ARM8 -> ARM9
ARM6 (NEON): erledigt
```

ARM1 lässt sich sofort und ohne ARM-Hardware erledigen. ARM2 und ARM3 sind der Kern. ARM5 kann nach ARM2
parallel zu ARM3 laufen, wenn ARM4 vorher festlegt, wie das Makro aussieht.

## Fallen, die der Compiler nicht findet

- **Falsche Syscall-Nummern kompilieren:** `SYS_FORK = 57` ist auf ARM64 `close`, die x86-Nummer 80
  (`chdir`) ist dort `fstat`. Deshalb die Lint-Regel in ARM1.
- **`O_NOFOLLOW` verschwindet still:** Die x86-Werte `O_DIRECTORY = 0x10000` und `O_NOFOLLOW = 0x20000`
  bedeuten auf ARM64 `O_DIRECT` und `O_LARGEFILE`. Auf ARM64 sind es `0x4000` und `0x8000`. Das wäre
  eine Sicherheitslücke ohne jede Fehlermeldung.
- **`struct stat`:** 144 Bytes auf x86-64, 128 auf ARM64, `st_mode` liegt bei 24 und 16. `st_size` und
  `st_mtim` bleiben gleich, `android_files.rs` beschreibt das schon. `linux_sys.rs` und
  `linux_sandbox.rs` rechnen mit festen Stellen.
- **`clone`:** x86-64 hat `child_tid, tls`, ARM64 `tls, child_tid`.
- **`dup3`:** Anders als `dup2` lehnt es `old == new` mit `EINVAL` ab. `process::launch` kann dorthin kommen,
  wenn Standardeingabe geschlossen war. `rt::os::dup2` fängt das ab.
- **Sandbox offen statt zu:** Eine falsche Nummer in der Sperrliste sperrt still den falschen Aufruf.
- **Befehlscache:** ARM hält ihn nicht von selbst aktuell. Die eigenen Loader leeren ihn heute nicht, auf
  x86 ist das auch nicht nötig. Geladene Plugins können sonst zufällig abstürzen.
- **Speichermodell:** x86 verzeiht falsche `Relaxed`-Zugriffe. QEMU auf einem x86-Rechner verzeiht sie
  meist genauso, solche Fehler zeigen sich erst auf echter Hardware.
- **Stack:** Auf ARM64 muss `sp` bei Speicherzugriffen 16-Byte-ausgerichtet sein, das betrifft jeden
  handgeschriebenen Einstieg.
- **Seitengröße:** `4096` steht an 20 Stellen in 11 Linux-Dateien von `plat`, nicht alles davon meint
  Seiten. Asahi Linux läuft mit 16 KB.
- **`c_char`:** auf Linux ARM64 `u8`, auf Windows ARM64 `i8`, das meldet der Compiler aber. Beim Stand von ARM0
  meldet er dazu nichts.

## Testen

| Was | Wo | Grenze |
|---|---|---|
| `coretest`, `portabletest`, Logik | `qemu-aarch64` in WSL | langsam, kein echtes Speichermodell, nur statische Programme |
| Signale, Sandbox, Crash, `process_vm_readv` | `qemu-system-aarch64` in WSL (Gast noch einzurichten) | sehr langsam, keine KVM-Beschleunigung; seccomp und Landlock nicht über `qemu-aarch64` prüfen |
| Echte ARM-CPU unter Linux | `ubuntu-24.04-arm`, statische Programme per `adb` auf dem A56 | Fenster auf dem Runner offen, das Telefon steckt nur auf Zuruf |
| Windows ARM64 | `windows-11-arm` | Fenster-Skripte laufen, die Windows-Einrichtung verdeckt aber den Bildschirm (ARM0); lokal keine Möglichkeit |
| Android und NEON | A56, Emulator | Emulator ist x86-64 |
| Echte GPU auf ARM | noch kein Gerät | Vulkan sonst nur mit lavapipe |

Die Regel gegen Fenster-Fluten gilt auch hier: dieselben Fenster-Tests nicht wiederholt laufen lassen,
höchstens drei Runden pro Variante. Die Runner von GitHub sind für öffentliche Repositories kostenlos.

## Offene Fragen

1. **Surface:** Welches Modell ist es? Ein Snapdragon-Gerät wäre ein echtes Testgerät für ARM5 und ARM8.
2. **Echtes ARM-Gerät für Linux:** Ein Raspberry Pi 5 oder Ähnliches wird spätestens für ARM8 nötig. Das
   kann bis dahin warten.
3. **CI:** Sollen die ARM-Läufe später auch automatisch bei jedem Push laufen? Vorschlag: nur bei Releases
   und per Hand.
4. **64-KB-Seiten:** Nur für manche Server-Kernel relevant, nach Entscheidung 7 zunächst nicht unterstützt.

# ARM64-Plan: Windows und Linux auf ARM-Prozessoren

Stand 2026-11-09, auf Basis von `main` 39e30a8d. Umgesetzt sind ARM0, ARM1, ARM2, ARM3 und ARM6: `rt` und `plat` laufen
auf Linux ARM64, statisch und gegen glibc; `plattest`, `linuxtest`, `lifetest` und `sandboxtest` sind in einer ARM64-VM
grün, Plugins und Module fehlen noch (ARM4). Für den Rest gilt: Die Zahlen sind aus dem Code gezählt, die Aufwände
geschätzt. Syscalls, Flags und `struct stat` sind mit ARM1 gegen die Kernel-Header geprüft (`tools/kernel_table_check.py`),
die Relocations mit ARM3 an einem Test-Plugin. Der Eintrag in der Roadmap steht in
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
   Abweichungen werden gemessen und dokumentiert (ARM2); `pow`, `sin` und `cos` rechnen dort seit dem Nachtrag zu
   ARM2 auf ein halbes ulp genau.
7. **Seitengrößen:** 4 KB und 16 KB werden unterstützt, die Größe kommt zur Laufzeit aus `AT_PAGESZ`. (Seit ARM3 laufen die
   Tests auch mit 64-KB-Seiten, siehe dort.)
   64 KB wird abgelehnt oder später nachgezogen.

## Meilensteine

Größe: S klein, M mittel, L groß, jeweils nur relativ zueinander.

### ARM0: Build-Grundlage (S)

- `rustup target add aarch64-pc-windows-msvc aarch64-unknown-linux-gnu`.
- `--arch` in `build_rs.py`: Ziel-Triple, Ausgabeordner, Interpreter `/lib/ld-linux-aarch64.so.1` und Version
  `GLIBC_2.17` der glibc-Stubs (seit dem Umzug auf Cargo `glibc_abi` in `lib/jflib_build/src/target.rs`), die
  Plugins und `text_shape` über Cargo.
- Nebenprobe: die heutige x64-Version unter Emulation auf `windows-11-arm` starten. Läuft sie, gibt es
  für Windows auf ARM sofort eine Zwischenlösung.

**Fertig, wenn:** `build_rs.py lib --target linux --arch aarch64` startet und die Fehlerliste den
Stellen dieses Plans entspricht, und die x86-Programme bytegleich zu vorher sind.

**Umgesetzt:** `--arch` gilt für `lib`, `app`, `all` und `plugin`, die Ausgabe liegt in `win-arm64` und
`linux-arm64` unter dem jeweiligen Ausgabeordner (`build/`, `build/release/`, `--out`). `watch` und `test` ohne App
lehnen `--arch aarch64` ab, `--target web` und `android` ebenso; `run` und `test NAME` starten seit ARM2 statische
Linux-Programme über `qemu-aarch64`.

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

**Umgesetzt** (2026-11-09):

- **`rt`:** `os.rs` ruft den Kernel mit `svc 0` (Nummer in `x8`, Ergebnis in `x0`). `lib.rs` wählt für ARM64
  `portable/cpu.rs` und `portable/math.rs` über `target_arch = "aarch64"`. In `mem.rs` bleibt der Heap gemeinsam,
  ebenso `memcmp`, `bcmp` und `strlen`; `memcpy`, `memmove`, `memset` und `load8` gibt es pro Architektur, für ARM64
  in `aarch64/mem.rs` (32 Bytes pro Schritt mit `ldp`/`stp`, der Rest nach seinen Bits).
- **`plat`:** `_start` (`linux.rs`), der Thread-Start `jfgui_rs_clone` (`linux_thread.rs`, `clone` in der
  Reihenfolge von ARM64) und der Rückweg der Signale (`linux_crash.rs`). Der Crash-Handler meldet auf ARM64 bis ARM3
  nur Signal, `pc` und Adresse: Register und Aufrufkette kennen nur x86-64. Plugin-Loader und Guard
  (`linux_elfload.rs`, `linux_guard.rs`, `plib.rs`, `modhost.rs` mit `win64`) sind auf Linux ARM64 aus, `lib.rs` nimmt
  dort wie Android `web_lib.rs` und `web_modhost.rs`; `current_exe` und `plat_tid` stehen bis ARM4 als Kopie in
  `linux.rs`. Für Windows ARM64 gilt das nicht (ARM4 und ARM5).
  Die x86-Teile tragen ihr `cfg` in derselben Zeile, so verschiebt sich keine Zeilennummer (Panic-Stellen).
- **Outline-Atomics:** Die `compiler_builtins` von Rust für `aarch64-unknown-linux-gnu` enthält den C-Teil von
  compiler-rt. Dessen `__aarch64_cas*`, `__aarch64_ldadd*` usw. nehmen LSE (die Atomics von ARMv8.1) nur, wenn
  `__aarch64_have_lse_atomics` gesetzt ist, und das setzt ein Konstruktor in `.init_array` mit `getauxval(AT_HWCAP)`.
  Ohne libc fehlte `getauxval` beim Linken, und der Konstruktor liefe nie, es bliebe bei LL/SC. (Der Konstruktor von
  `std` hilft nicht: Er entsteht nur ohne diesen C-Teil.) Jetzt liest `rt::os::process_start`, von `_start` gerufen,
  den Auxiliary Vector und führt `.init_array` aus, `rt::os::getauxval` antwortet daraus. Geprüft mit `qemu-aarch64
  -d in_asm` auf `portabletest`: Mit `-cpu max` liefen nur LSE-Befehle (14, kein LL/SC), mit `-cpu cortex-a53`
  (ARMv8.0) nur LL/SC (28). Programme gegen glibc oder mit `std` brauchen das nicht, dort macht es die libc.
- **Test-Runner:** `tools/host_exec.py` liest die Architektur aus dem ELF- oder PE-Kopf und startet ein Linux-Programm
  eines anderen Prozessors als `qemu-aarch64 PROGRAMM` in WSL. Damit gilt das für jeden Aufrufer, `build_rs.py run`
  und `test NAME` mit `--arch aarch64 --target linux` eingeschlossen. Abgelehnt mit Meldung: Windows-ARM64 auf
  x86-64, Apps gegen glibc (in WSL liegt unter `/usr/aarch64-linux-gnu` nur Binutils, keine glibc; `--static` geht),
  `test` ohne App (die ganze Suite braucht ARM3 und ARM4).
- **Ergebnis unter `qemu-aarch64`** (8.2.2, WSL): `coretest` 4728 von 4728 Prüfungen in 72 s, auch mit `-cpu
  cortex-a53`. `portabletest` ist grün mit dem Digest `edea7e9d9579c77a`, demselben wie unter Windows und Linux auf
  x86-64 (auch mit `--simd sse2`).

**Mathematik:** ARM64 rechnet `rt::x87` mit `portable/math.rs`. `portabletest --math-dump` gibt beide Fassungen für
dieselben 360 000 Eingaben aus, `apps/portabletest/tests/math_diff.py` vergleicht den Lauf auf x86-64 mit dem auf ARM64
und misst beide gegen die C-Bibliothek (Pythons `math`, etwa 1 ulp genau). `portable/math.rs` gibt auf beiden
Prozessoren in allen Zeilen dieselben Bits. Je 20 000 Eingaben, Stand nach den beiden Nachträgen unten (in Klammern die
Werte von ARM2 davor):

| Gruppe | Eingaben | gleiche Bits | ARM64 zu x86-64, max / 99 % | x86-64 zur libm, max | ARM64 zur libm, max |
|---|---|---|---|---|---|
| `scale` (`f64_parse`) | m < 2^63, 10^-340 bis 10^308 | 100 % | – | – | – |
| `log10_round`, `scale_round` (`f64_format`) | jeder positive Double, 1 bis 17 Stellen | 100 % | – | – | – |
| `exp2` | -1100 bis 1030, -1 bis 1 | 99,97–99,99 % (40–42 %) | 1 / 1 ulp (4 / 3) | 1 ulp | 1 ulp (4) |
| `log2` | jeder positive Double; 0,5 bis 2 | 99,96 %; 99,98 % (99,8 %; 57 %) | 1 ulp (2; 5) | 1 ulp | 1 ulp (5) |
| `pow` | x bis 100, y ±20 | 99,56 % (5,8 %) | 1 / 1 ulp (118 / 59) | 1 ulp | 1 ulp (118) |
| `pow` (Gamma) | x 0 bis 1, y 0,1 bis 10 | 99,95 % (22 %) | 1 / 1 ulp (58 / 17) | 1 ulp | 1 ulp (58) |
| `sin`, `cos`, `sincos` | -20 bis 20 | 99,91–99,97 % (23–24 %) | 1 ulp, `cos` 38 (9036 / 141) | 1 ulp, `cos` 38 | 1 ulp (9037) |
| `sin`, `cos` weit | -10^6 bis 10^6 | 11 % (0 %) | 47 500 / 309 ulp, absolut ≤ 1,3e-15 (3,9e-11) | 47 500 ulp | 1 ulp (1,4e9) |
| `atan2` | ±100, ±100 | 99,97 % (67 %) | 1 / 1 ulp (5 / 3) | 1 ulp | 1 ulp (5) |
| `rz_exp2`, `rz_log2` | ±16; 0 bis 256 | über 99,99 %; 99,97 % (41 %; 96 %) | 1 ulp (4; 2) | 1 ulp | 1 ulp (4; 2) |

Der Text einer Zahl und die Zahl eines Textes sind also auf ARM64 dieselben wie auf x86-64 (`f64_format` korrigiert
`log10_round` ohnehin selbst). `exp2`, `log2`, `pow`, `sin`, `cos` und `atan2` (auch `rz_exp2`, `rz_log2`) geben in
99,6 bis über 99,99 % der Fälle dieselben Bits wie x87; wo nicht, liegt eine Seite ein ulp daneben, bei `cos` nahe seinen
Nullstellen x87 selbst (bis 38 ulp). Außerhalb von ±20 ist jetzt x87 der ungenauere Teil: `fsin` und `fcos` reduzieren
mit einem 66-Bit-π (bis 47 500 ulp), `portable/math.rs` bleibt bei 1 ulp zur libm. Die Effekte, Audio und die
Oberfläche nutzen diese Funktionen; bitgleiche Bilder zu x86-64 sind damit nicht garantiert, wo Ergebnisse in Pixel oder
Samples gerundet werden, aber selten verschieden.

**Nachtrag: `pow`, `sin` und `cos` genauer** (2026-11-09). `portable/math.rs` rechnet die drei mit Double-Doubles
(hi + lo, Dekkers Produkte mit Veltkamp-Teilung, also ohne FMA) und rundet erst am Ende:

- `pow` = 2^(y log2 x): `log2` aus einer Tabelle `log2(1 + j/64)` (47 Einträge) und `atanh` mit |z| < 0,0056, das
  Produkt mit y als Double-Double, `exp2` aus `2^(j/32)` (32 Einträge) und einer Reihe in |g| ≤ ln 2 / 64.
- `sin`, `cos`: Unter 2^19 reduziert Cody-Waite mit π/2 in vier Teilen (172 Bit, die Produkte exakt), darüber
  Payne-Hanek mit 1216 Bit von 2/π in Ganzzahlarithmetik, also für jeden endlichen Double. Die Taylor-Reihen in r
  (|r| ≤ π/4) haben ihre führenden Glieder als Double-Doubles.

Gegen eine exakte Referenz (Pythons `decimal`, π auf 1400 Bit) liegen 18 Gruppen mit je 20 000 Eingaben höchstens
0,502 ulp daneben: die Gruppen oben, dazu `pow` über den ganzen Exponentenbereich und mit x nahe 1, Winkel bis 10^308, nahe
Vielfachen von π/2 (auch bei 2^19) und der bekannte schlechteste Fall der Reduktion, 6381956970095103 · 2^797. Das eine ulp zur libm in der Tabelle
kommt von der libm. Weiterhin nur IEEE-Operationen auf Doubles und Ganzzahlen, kein `f64::sin` aus std, `no_std`.
`scale`, `scale_round`, `log10_round`, `exp2`, `log2` und `atan2` sind unverändert. Neuer Digest von `portabletest`:
`44721bded2985ff2` unter Windows und Linux auf x86-64 (auch mit `--simd sse2`), unter `qemu-aarch64` und in Nodes
WebAssembly (`compare.mjs`, `portabletest.wasm` und `.simd.wasm`). Zeit je Aufruf auf dem Entwicklungsrechner (x86-64,
unter Last gemessen): `pow` 70 ns (vorher 80), `sin` 54 ns (46), `sincos` 71 ns (47).

**Nachtrag: `exp2`, `log2` und `atan2` genauer** (2026-11-09). Sie bauen auf denselben Double-Doubles auf:

- `exp2` ist das `exp2` aus `pow` (Tabelle `2^(j/32)`) mit exaktem Argument. Subnormale Ergebnisse rundet es jetzt
  einmal statt zweimal: Zu r in [0,98, 2) kommt C = 2^(-1022 - n), dessen ulp genau dem Raster von 2^-1074 nach dem
  Skalieren entspricht; danach ist das Abziehen von C und das Skalieren exakt (gilt auch für `pow`).
- `log2` ist das gerundete `log2` aus `pow` (Tabelle `log2(1 + j/64)`). `log10_round` behält die alte Reihe
  (`log2_series`): Seine ganzen Zahlen müssen die von `fldlg2`/`fyl2x`/`fistp` bleiben, und nahe halben Zahlen
  entscheidet die letzte Stelle.
- `atan2`: t = n/d mit 0 < n ≤ d als Double-Double (der Rest über ein exaktes Produkt), c = j/32 der nächste Wert,
  atan t = atan c + atan((t - c) / (1 + t c)) mit |s| ≤ 1/64, `atan(j/32)` aus einer Tabelle (33 Einträge), die Reihe
  in s bis s^11. Dazu 0, π/2 oder π als Double-Double, eine Rundung am Ende. Sehr kleine Quotienten (t < 2^-60) gehen
  direkt durch, sehr große oder kleine Operanden werden vorher mit 2^±600 skaliert.

Gegen die exakte Referenz (`decimal`, 70 Stellen) liegen 20 Gruppen mit je 20 000 Eingaben höchstens 0,50002 ulp
daneben: die Bereiche der Tabelle, dazu `exp2` mit subnormalen Ergebnissen, knapp unter 1024 und nahe ganzen Zahlen,
`log2` nahe 1 und für subnormale x, `atan2` mit beliebigen Doubles, Quotienten von 2^-70 bis 2^70, nahe den
Stützstellen j/32, an den Grenzen 2^±400 und 2^-60 und mit subnormalen Operanden; Vorzeichen von Nullen und Unendlich
wie vorher. `pow`, `sin` und `cos` bleiben bei 0,502 ulp. `math_diff.py`: `log10_round` weiter 100 % gleich, die
Tabelle oben. Neuer Digest von `portabletest`: `af6940ef15c56d1f` unter Windows und Linux auf x86-64 (auch mit `--simd
sse2`), unter `qemu-aarch64` und in Node (`compare.mjs`, `portabletest.wasm` und `.simd.wasm`). Das Tempo ist nicht
schlechter (Rechner unter Last, verrauscht): `exp2` 26 ns (vorher 38–45), `log2` 33–60 ns (43–48), `atan2` 54–70 ns
(55–80). Die Zeilen 1 bis 260 von `math.rs` behalten ihre Nummern; auf x86-64 nutzt `base/num.rs` daraus nur `scale`
und `scale_round`, beide unverändert.

**A56** (SM-A566B, Android 16, Kernel 6.6 mit 4-KB-Seiten, Cortex-A720 und A520 mit LSE): Das statische `coretest`
aus `/data/local/tmp`, gestartet über `adb shell` mit eigenem `TMPDIR`, läuft auf echter ARM-Hardware: 4720 von 4724
Prüfungen grün. Die vier Fehler kommen von Android, nicht von ARM: `hard link` verbietet die Shell dort (ein `ln` in
`/data/local/tmp` scheitert ebenso), vermutlich aus demselben Grund `unix bind` mit EACCES, und für die beiden Prüfungen
der lokalen Zeitzone fehlt `/etc/localtime` (Android hat die Zone in `persist.sys.timezone`). Vorher geprüft: Das Telefon war seit zehn Minuten nicht bedient, der Bildschirm
bleibt dort wegen einer sehr langen Zeitsperre an; das Programm berührt die Oberfläche nicht und wurde danach gelöscht.

**x86-64 unverändert:** Release-Bau von `coretest`, `plattest`, `gallery` und `PaintCraft` mit `jxlenc` für Windows und
Linux, `16ec0ab4` und dieser Stand nacheinander aus demselben Verzeichnis: Alle Programme sind Byte für Byte gleich,
unter Windows bis auf den Zeitstempel des Linkers (die `.map` nur in dessen Zeile), unter Linux auch `gallery` und
`PaintCraft` gegen glibc. Anders ist nur der C#-Host `pdn-host`, den `dotnet` baut, mit unveränderten Quellen.
`cargo check -p jflib` mit und ohne `std` ist grün für Windows, Linux, wasm32 und Android, für Linux ARM64 ohne `std`;
mit `std` fehlt dort `SYS_ARCH_PRCTL` (`plat/linux_glibc.rs`, ARM3). `build_rs.py lib --target linux --arch aarch64`
baut die ganze Bibliothek. Windows ARM64 kommt jetzt durch `rt` und endet an 26-mal `win64` in `plat` (ARM4), dahinter
wartet `__chkstk` (ARM5).

**Beim Merge mit `apple/ap2`** (AP2 der Apple-Portierung, nicht gemergt) widersprechen sich zwei Stellen, beide mit
derselben Lösung, nämlich beide Bedingungen in das `any(...)`: in `lib/rt/lib.rs` die Zeilen vor `pub mod cpu;` und
`pub mod x87;` (`target_arch = "aarch64"` gegen `target_vendor = "apple"`), in `lib/plat/lib.rs` die vor `mod plib;`
und `mod modhost;` (dort `all(target_os = "linux", target_arch = "aarch64")`), dazu `CHANGELOG.md` (beide Einträge
behalten). `rt/mem.rs` schließt AP2 auf Apple ganz aus, das passt zur Trennung hier. `tools/host_exec.py` und
`build_rs.py` mischen sich ohne Konflikt (geprüft mit `git merge-tree`).

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
- Testprogramme: `jfgui_gt_code` in `plib.rs`, `plugins/crash`, `plugins/crashpf` (seit dem Umzug in JFPainters
  Repository), `modrt`, `tools/memory_safety/native_probe.rs`.
- Von ARM2 übrig: Der Crash-Handler auf ARM64 (`linux_crash.rs`, am Ende) füllt nur `pc` und Adresse, `access` ist
  „unbekannt“ (das ESR im `__reserved` des `sigcontext` sagt Lesen, Schreiben oder Ausführen); die Seitengröße steckt
  noch fest in `GUARD_SIZE` (`linux_thread.rs`) und `PAGE` (`rt/mem.rs`: dort nur verschwendet, nicht falsch).

**Fertig, wenn:** `plattest`, `linuxtest`, `lifetest` und `sandboxtest` in einer ARM-VM grün sind. Die
Sandbox-Tests müssen zeigen, dass jeder gesperrte Aufruf wirklich scheitert.

**Umgesetzt** (2026-11-09). Die x86-Teile tragen ihr `cfg` in derselben Zeile, die ARM-Teile stehen am Ende der Dateien:

- **Crash-Berichte** (`linux_crash.rs`): `CrashInfo::regs` hat auf ARM64 `x0` bis `x15` und `pc` (wie auf Apple), `access`
  kommt aus dem ESR-Eintrag im `__reserved` des `sigcontext` (EC 0x20/0x21 Ausführen, 0x24/0x25 Lesen oder Schreiben nach
  WnR). Die Aufrufkette nutzt das CFI-Lesen von x86-64 mit den DWARF-Registern von ARM64 (`x0`–`x30`, `sp` = 31,
  Rücksprungspalte `x30`, ein eigener Platz für `pc`). Code ohne CFI wird über das Link-Register an seinen Aufrufer
  gehängt, wenn er selbst fehlte (ein Blatt wie `crash_test`), sonst über ein Stack-Wort nach `bl`/`blr`, das in ihn ruft
  (`calls_into`); der Stack-Scan beginnt bei `sp`, das Link-Register zuerst.
- **glibc-Variante** (`linux_glibc.rs`): `_start` für ARM64 (`ld.so` lässt den Finalisierer in `x0`), `glibc_thread_ids` liest
  `TPIDR_EL0`. Fallen, die erst beim Linken oder Starten auffallen: `pthread_attr_t` hat dort 64 Bytes statt 56,
  `PTHREAD_STACK_MIN` ist 128 KB (glibc rechnet mit 64-KB-Seiten), `thread_spawn` fragt dort nie weniger an; TLS ist
  Variante I, `pthread_self()` liegt knapp unter dem Thread-Zeiger (`gputest --test glibc` prüft das jetzt so).
- **Seitengröße zur Laufzeit:** `rt::os::page_size()` aus `AT_PAGESZ` (statisch aus dem Auxiliary Vector von ARM2, gegen glibc
  über `getauxval`, das die glibc-Stubs für ARM64 jetzt mitbringen). Die Page-Runs des Heaps (`rt/mem.rs`) richten sich
  danach. Die Guard-Seiten der Threads sind auf ARM64 fest 64 KB, die größte Seitengröße: So liegen beide Guards und der
  Signal-Stack dazwischen bei 4, 16 und 64 KB auf Seitengrenzen, belegt wird nur Adressraum.
- **Sandbox:** `AUDIT_ARCH_AARCH64`; der Schritt für das x32-Bit von x86-64 bleibt, keine ARM64-Nummer erreicht es.
  `plat::sandbox::denied_syscalls()` gibt die Sperrliste heraus, `sandboxtest` ruft jeden gesperrten Aufruf einmal auf
  (dazu `execve` und `execveat`), mit harmlosen Argumenten, die ohne Filter wo möglich eine andere Antwort als EPERM
  bekommen; auf beiden Architekturen.
- **`cpu.rs`, `lib.rs`:** `stack_aligned` liest `sp`, `avx2_probe` ist `false`, `cpu_brand` kommt aus `MIDR_EL1`
  (Hersteller und Kern, etwa „ARM Cortex-A720“, wenn `AT_HWCAP` `HWCAP_CPUID` hat, sonst „ARM64“); `crash_test` und
  `crash_test_caller` wie auf Apple. `tools/memory_safety/native_probe.rs` hat einen `_start` für ARM64.
- **Für ARM4 geschrieben, aber noch aus** (`plib` bleibt auf Linux ARM64 bis dahin aus): `guard_sig` setzt `pc`, `sp` und
  `x29` im `sigcontext` (`linux_guard.rs`); das Trampolin in AAPCS64 sichert `x19`–`x28`, `x29`/`x30` im Frame und
  `FPCR`, `d8`–`d15` im Record, dessen Felder ihre Offsets behalten (`plib.rs`); der Test-Code `jfgui_gt_code` (ARM64
  teilt ganzzahlig durch null ohne Trap: Fall 4 schickt sich `SIGFPE` mit `tgkill`, der Fehler liegt bei +40); der
  ELF-Loader nimmt `EM_AARCH64` mit `RELATIVE`, `ABS64`, `GLOB_DAT` und `JUMP_SLOT` (die beiden letzten mit Addend, anders als
  auf x86-64), rechnet in der Seitengröße des Kernels (Segmente, die sich eine solche Seite teilen, lehnt er ab) und
  leert den Befehlscache der ausführbaren Segmente (`dc cvau`, `ic ivau`, Zeilen aus `CTR_EL0`). Geprüft mit einem
  temporären Schalter, der `plib` einschaltet und `win64` durch `extern "C"` ersetzt (nicht im Repository, Inhalt unter ARM4).
- **`plattest`:** `t_guard` und `t_lib` sind auf Linux ARM64 bis ARM4 Platzhalter wie auf Apple; die Erwartungen in
  `t_guard.rs` (Offsets 4 und 40) kennen ARM64 schon. Der Platzhalter-Loader fragt dort wie auf Apple zuerst das Manifest.

Getestet in einer VM: `qemu-system-aarch64` 8.2 mit AAVMF in WSL (TCG, 8 Kerne, 6 GB), Ubuntu 24.04 arm64 (Cloud-Image),
Kernel 6.8.0-147 mit 4-KB-Seiten, Weston (headless) und Xvfb als Anzeige:

| Programm | Ergebnis |
|---|---|
| `plattest` | 41 von 42 Skripten; `screen.uis` scheitert nur an Xvfb (RandR ohne Bildwiederholrate). Mit XWayland in der VM besteht diese Prüfung, dort fehlen dem randlosen XWayland Root-Bild und XTEST; unter `qemu-aarch64` mit WSLg ist `screen.uis` grün |
| `plattest --test crash` | 7 von 7: `access` = Schreiben aus dem ESR, `x0` = 16, `pc`, Code-Bereich, 2 Frames mit `crash_test_caller` + 4 |
| `plattest --test kernel` | 50 von 50 (auch `CLONE_PARENT_SETTID`) |
| `linuxtest` (ohne Skripte, jeder Test einmal) | `files` 15, `synth` 23, `x11synth` 118, `cursors`, `window` 7, `wlwin` 8, `pcheck` 9; `crash` Exit 3, `protoerr` Exit 1 wie erwartet |
| `lifetest` | 3 von 3 Skripten |
| `sandboxtest` | 0 Fehler: Landlock ABI 4 (Dateien und Netz), seccomp; alle 23 gesperrten Aufrufe geben EPERM |
| `coretest`, `portabletest` | 3605 von 3605 Prüfungen (56 s); Digest `af6940ef15c56d1f` wie überall |
| ARM4-Teile (temporärer Schalter) | `guard` 24 von 24 (`FPCR` und `d8`–`d15` nach 64 Fehlern zurück, 8 Worker, Stack-Überlauf), `guardlimit` 3 von 3, `guardcrash` 8 von 8; ein Test-Plugin mit allen fünf Relocation-Arten, für 4 und 64 KB gelinkt, lädt und rechnet unter dem Guard 55 |

Gegenprobe zu `sandboxtest`: Dieselben Aufrufe ohne Filter (Python mit `ctypes`, gleicher Kernel) antworten in 16 von 23
Fällen anders (ESRCH, 0, EFAULT, EINVAL, EOPNOTSUPP, EBADF); dort beweist das EPERM den Filter. Sieben geben auch ohne
Filter EPERM, weil das Recht fehlt (`kexec_load`, `kexec_file_load`, die drei Modul-Aufrufe, `pivot_root`,
`open_by_handle_at` vor Linux 6.9): Sie scheitern also, nur nicht nachweislich am Filter. In WSL (x86-64, Kernel 6.18)
antworten 18 anders.

**Mit 64-KB-Seiten** (Ubuntus Kernel `6.8.0-149-generic-64k` in derselben VM, `getconf PAGESIZE` = 65536): dasselbe Bild.
`plattest` 41 von 42 (wieder nur `screen.uis` an Xvfb), `linuxtest` wie oben, `lifetest` 3 von 3, `sandboxtest` ohne
Fehler, `coretest` 3605 von 3605, `portabletest` mit demselben Digest; mit dem temporären Schalter `guard` 24 von 24 und
`guardlimit` (140 Threads mit 64-KB-Guards), und der Loader lehnt das für 4 KB gelinkte Test-Plugin mit `LIBE_FORMAT` ab,
das für 64 KB gelinkte lädt und rechnet. Ein Kernel mit 16-KB-Seiten war nicht zur Hand.

Unter `qemu-aarch64` (User-Mode) waren vorher `plattest` 40 von 42 Skripten und alle `linuxtest`-Tests grün. Grenzen
dieses Wegs: QEMU schreibt keinen ESR-Eintrag in den Signal-Frame (`access` bleibt „unbekannt“) und setzt
`CLONE_PARENT_SETTID` nur im Speicher des Kindes (`kernel.uis`); seccomp und Landlock prüft er nicht.

**x86-64 unverändert:** Release-Bau von `coretest`, `plattest`, `gallery` und `gputest` für Windows und Linux, `39e30a8d` und
dieser Stand nacheinander aus demselben Verzeichnis: unter Linux Byte für Byte gleich (`gallery` und `gputest` gegen glibc),
unter Windows bis auf den Zeitstempel des Linkers (die `.map` nur in dessen Zeile). Absichtlich anders ist `sandboxtest`
(die neuen Proben). `cargo check -q -p jflib` mit und ohne `std` ist grün für Windows, Linux, wasm32, Android und Linux ARM64.

Offen: echte ARM-Hardware (der Befehlscache und das Speichermodell zeigen sich unter TCG nicht, ARM4 und ARM7), ein Kernel
mit 16-KB-Seiten (Asahi), `screen.uis` mit einer vollständigen Anzeige in der VM. `plugins/crash` und `crashpf` gehören zu
JFPainter.

### ARM4: Plugin- und Modul-ABI (M)

- Makro für `extern "win64"` nach Entscheidung 4.
- `prepare_tramp` in `lib/plugin/fx.rs` gibt zwei Register zurück: auf ARM `x0` und `x1`, sauber über
  eine `repr(C)`-Struktur.
- `lib/plugin/mem.rs`: `memcpy` und Verwandte in Assembler, je ein Block für Windows und System V. Auf ARM64
  rufen beide Systeme gleich auf, einer reicht. `plugin::trap` in `lib/plugin/lib.rs` nutzt `ud2`.
- `modules/modrt` hat Kopien von `rt/cpu.rs`, `rt/fx.rs`, `rt/x87.rs` und `plugin/mem.rs` (`shared/`, `lint()` hält sie gleich) und bindet sie per `#[path]` ein, die
  Auswahl in `rt/lib.rs` gilt dort nicht. Dazu kommen `ud2` und der Testfehler `JXT_FAULT` in `modrt/lib.rs`.
- `build_rs.py plugin`, `pe_verify`, `elf_verify`, Meldung für Plugins der falschen Architektur.
- Zurücknehmen, was ARM2 ausgeschaltet hat: `all(target_os = "linux", target_arch = "aarch64")` vor `mod plib;` und `mod modhost;` in
  `plat/lib.rs`, die `cfg` vor `linux_elfload.rs` und `linux_guard.rs` in `linux.rs` und dort die Kopien von
  `current_exe` und `plat_tid`, das `allow(dead_code)` an `open_read_nofollow` (`linux_sys.rs`).
- Von ARM3 vorbereitet und mit einem temporären Schalter in der VM geprüft (siehe ARM3 „Umgesetzt“): Guard-Handler,
  Trampolin, `jfgui_gt_code` und der ELF-Loader für ARM64. Der Schalter tat genau dies, ARM4 macht es dauerhaft: in
  `plat/lib.rs` nur bei `mod plib;` das `all(target_os = "linux", target_arch = "aarch64")` streichen, in `linux.rs` die vier
  `#[cfg(target_arch = "x86_64")]` vor `elfload` und `guard` und die Kopie von `plat_tid`, in `plib.rs` `extern "win64" fn
  thunk` und in `apps/plattest/t_guard.rs` die vier Host-Funktionen auf das Makro, in `apps/plattest/main.rs` die Platzhalter
  für `t_guard` und `t_lib` wieder auf Apple allein. Dazu die Apple-und-ARM64-Bedingung vor `plugin_allowed` in
  `web_lib.rs`, die dann nur noch Apple betrifft.
- Plugins für ARM64 mit `-z max-page-size=65536` linken: Der Loader lehnt Segmente ab, die sich eine Seite des laufenden
  Kernels teilen, ein für 4 KB gelinktes Plugin lädt also auf Kerneln mit 16 oder 64 KB nicht (in der VM mit 64-KB-Seiten
  geprüft). Rust ruft innerhalb eines Plugins über die GOT (`GLOB_DAT`); `JUMP_SLOT` entsteht erst mit `-Z plt=yes`.
- Der Befehlscache: `sync_icache` ist unter TCG nicht prüfbar (QEMU hält Code und Daten von selbst gleich). Ein echtes
  Gerät (A56 mit einem statischen `plattest`, ein Runner `ubuntu-24.04-arm`) sollte `t_lib` laufen lassen.

**Fertig, wenn:** das Modul `jxlenc` und `plattest` (`t_guard`, `t_lib`) auf ARM64 und unverändert auf x86-64
laufen (JFPainters Plugins: in dessen Repository).

### ARM5: Windows ARM64 (M)

- `__chkstk` (Größe in `x15` mal 16, nur `x16` und `x17` frei), TEB über `x18` statt `gs:`
  (`win_crash.rs`, `win_lib.rs`).
- `CONTEXT` und `RtlVirtualUnwind` in `win_crash.rs`, Guard-Trampolin mit Unwind-Daten. Das Trampolin für ARM64 steht seit
  ARM3 in `plib.rs` (`target_arch = "aarch64"`, ohne Betriebssystem-Teile); es braucht unter Windows `.pdata`/`.xdata`.
  `jfgui_gt_code` gibt es nur für Linux: Windows bekommt für die Division durch null `brk #0xF003`
  (`STATUS_INTEGER_DIVIDE_BY_ZERO`), die Offsets in `t_guard.rs` (`+40`) gelten dann nur für Linux.
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
- Beide Wege der Outline-Atomics: LSE, wo die CPU es hat (seit ARM2 auch ohne libc), LL/SC auf ARMv8.0.

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
- **Befehlscache:** ARM hält ihn nicht von selbst aktuell. Der ELF-Loader leert ihn seit ARM3 (`sync_icache`), der
  PE-Loader noch nicht (ARM5). Geladene Plugins können sonst zufällig abstürzen; QEMU zeigt das nicht.
- **Speichermodell:** x86 verzeiht falsche `Relaxed`-Zugriffe. QEMU auf einem x86-Rechner verzeiht sie
  meist genauso, solche Fehler zeigen sich erst auf echter Hardware.
- **Stack:** Auf ARM64 muss `sp` bei Speicherzugriffen 16-Byte-ausgerichtet sein, das betrifft jeden
  handgeschriebenen Einstieg.
- **Seitengröße:** `4096` stand an 20 Stellen in 11 Linux-Dateien von `plat`, nicht alles davon meint Seiten. Seiten
  meinten nur die Guards der Threads, der ELF-Loader und der Heap in `rt` (ARM3). Asahi Linux läuft mit 16 KB.
- **glibc auf ARM64:** `pthread_attr_t` hat 64 Bytes, `PTHREAD_STACK_MIN` ist 128 KB, die TLS ist Variante I
  (`TPIDR_EL0` zeigt auf den TCB über `struct pthread`, nicht auf `pthread_self()`). Ein zu kleines `pthread_attr_t`
  überschreibt still den Stack.
- **Relocations:** `R_AARCH64_GLOB_DAT` und `JUMP_SLOT` addieren den Addend, `R_X86_64_GLOB_DAT` nicht.
- **QEMU im User-Mode:** kein ESR-Eintrag im Signal-Frame, `CLONE_PARENT_SETTID` nur beim Kind, seccomp und Landlock
  gar nicht. Ein grüner Lauf dort sagt über Signale und Sandbox wenig.
- **`c_char`:** auf Linux ARM64 `u8`, auf Windows ARM64 `i8`, das meldet der Compiler aber. Beim Stand von ARM0
  meldet er dazu nichts.

## Testen

| Was | Wo | Grenze |
|---|---|---|
| `coretest`, `portabletest`, Logik | `qemu-aarch64` in WSL: `build_rs.py test NAME --target linux --arch aarch64` | kein echtes Speichermodell, nur statische Programme; `-cpu cortex-a53` prüft ARMv8.0 ohne LSE |
| Signale, Sandbox, Crash, glibc, Fenster | `qemu-system-aarch64` in WSL: Ubuntu 24.04 arm64, Kernel mit 4- und 64-KB-Seiten (ARM3; Skripte in den Übergabenotizen von ARM3) | TCG, etwa so schnell wie `qemu-aarch64`, Start 5 Minuten; kein echtes Speichermodell, kein Befehlscache; Weston und Xvfb ohne volles RandR |
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
4. **64-KB-Seiten:** Nur für manche Server-Kernel relevant, nach Entscheidung 7 zunächst nicht unterstützt. Seit ARM3
   laufen `rt` und `plat` dort (VM mit Ubuntus Kernel `generic-64k`); offen ist, ob Pakete und Plugins das zusagen sollen.

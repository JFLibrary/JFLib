# Android-Plan: Android so weit wie Windows, Linux und Apple

Stand 2026-11-28, auf Basis von `main` 7cdabce4. AN0 bis AN4 sind umgesetzt (Emulator und A56), AN5 auf dem Emulator
(der CI-Job ist lokal geprüft, auf GitHub noch nicht gelaufen); die Aufwände sind geschätzt. Der Plan wartete auf ARM1 bis ARM3 ([ARM64.md](ARM64.md)): die Kernel-Tabellen
und `plat` auf Linux ARM64 ohne libc.

## Ziel

Eine App mit JFLib kann auf Android dasselbe wie auf Windows, Linux und den Apple-Systemen, soweit Android es
überhaupt kennt: echte Threads und den Pool, Netzwerk mit `netio` (HTTP, TLS, DNS, SMTP, SSH), das Dateisystem mit
`datastore` und `cas`, Ordnerüberwachung, Drucken und die Schriften des Systems. Was es auf Android nicht gibt,
steht unter [Bewusst nicht](#bewusst-nicht) und antwortet dort mit `Unsupported`, statt beim Bauen zu fehlen.

Die Oberfläche, die Dateien des Host-Adapters, Geheimnisse, Benachrichtigungen, Klang, Sensoren, Standort und die
GPU gibt es auf Android schon. Sie ändern sich nur, wo dieser Plan sie berührt. Windows, Linux, Apple und Web
ändern sich nicht.

Nicht Teil dieses Plans: die 3D-Ansicht in der UI auf Android (G5e, [GPU.md](GPU.md)), weitere Android-ABIs als
`arm64-v8a` und `x86_64`, der Play Store als Ziel.

## Ausgangslage

Android läuft über den Host-Weg, den es mit dem Browser und iOS teilt: Ein Java-Host (`android/`) treibt die
Oberfläche mit dem `Choreographer`, `plat` nimmt den Host-Adapter [`web.rs`](../lib/plat/web.rs) mit den
Android-Teilen, `rt` die Auswahl aus [`android.rs`](../lib/rt/android.rs) und `portable/`. Gegen Bionic gelinkt
wird schon heute.

Was auf Android fehlt (Code auf `main` 7cdabce4):

- **Keine Threads in `plat`.** `thread_spawn` liefert `None`, `cpu_count()` ist 1, `mem_total()` 0
  ([`web.rs`](../lib/plat/web.rs)). Android nimmt den Pool des Browsers
  ([`web_pool.rs`](../lib/plat/web_pool.rs)): Jobs, `parallel_for` und `spawn_blocking` laufen zwischen den Bildern
  auf dem UI-Thread. Das A56 rechnet also auf einem seiner acht Kerne, und `render_par` läuft auf Android nicht
  ([ARM64.md](ARM64.md), ARM6). (Seit AN1 behoben.)
- **Keine Module ohne Oberfläche.** `process`, `sync`, `net`, `fs`, `service`, `watch` und `certs` sind in
  [`lib/plat/lib.rs`](../lib/plat/lib.rs) nur für Windows, Linux und Apple da. `rand` hat Android seit b5134991
  (`arc4random_buf`). (`sync` seit AN1, `fs` und `watch` seit AN3, `process` und `service` seit AN4 als Module, die
  „nicht unterstützt“ antworten.)
- **Die Fassade lässt Crates weg.** [`lib/jflib/Cargo.toml`](../lib/jflib/Cargo.toml) nimmt `cas`, `datastore`,
  `netio` und `update` auf Android und im Browser nicht auf („plat has no fs and net there“). (`cas` und
  `datastore` seit AN3.)
- **Das Manifest kennt kein Netz.** Das Standard-Manifest von `cargo jflib android`
  ([`android.rs`](../lib/cargo_jflib/src/android.rs), `default_manifest`) hat keine `INTERNET`-Berechtigung, und
  `tools/android.py` prüft nur `location` gegen das Manifest (`manifest_gaps`). (Seit AN2 behoben; `netio` ist seit
  AN2 auch in der Fassade.)
- **Kein Druck, keine Systemschriften.** `capabilities()` meldet auf Android weder `CAP_PRINT` noch
  `CAP_SYSTEM_FONTS` noch `CAP_THREADS`; `policy::font_dirs()` kennt `/system/fonts` nicht. (`CAP_THREADS` seit AN1,
  der Rest seit AN4.)
- **Kaum Tests ohne Oberfläche.** Nur `androidGallery`, `audiotest`, `gputest`, `portabletest`, `render2dtest` und
  `screentest` haben `android` in `TARGETS`; `coretest`, `plattest` und `asynctest` nicht. Es gibt keinen CI-Lauf
  auf Android, nur `cargo check` in `crates.yml`.

Was hilft:

- **iOS ist die Vorlage.** iOS nimmt denselben Host-Weg und hat darunter seit AP3 und AP5 echte Threads
  ([`apple_thread.rs`](../lib/plat/apple_thread.rs), pthreads), den richtigen Pool ([`pool.rs`](../lib/plat/pool.rs))
  und `sync`, `net`, `fs`, `certs` ([APPLE.md](APPLE.md)). Genau diese Aufteilung braucht Android.
- **Bionic-Threads gibt es schon.** Die GPU-Ausgabe startet ihre Jobs mit `pthread_create`
  ([`android_gpu.rs`](../lib/plat/android_gpu.rs)), und die Host-Aufrufe antworten seit 692a5aac pro Thread in einem
  eigenen Puffer.
- **Bionic-Dateien gibt es schon.** [`android_files.rs`](../lib/plat/android_files.rs) legt die virtuellen Namen
  (`/config`, `/data`, `/cache`, `/tmp`, `/documents`, `/imports`) über Bionic auf echte Dateien im internen Speicher.
- **Der Linux-Code läuft auf Androids Kernel.** Das statische `coretest` mit den eigenen Syscalls lief auf dem A56
  (Android 16, Kernel 6.6) mit 4720 von 4724 Prüfungen grün ([ARM64.md](ARM64.md), ARM2). Die vier Fehler kamen von
  Android, nicht von ARM: Hardlinks, `unix bind` mit EACCES, zweimal `/etc/localtime`. Die Kernel-Tabellen in
  [`rt/kernel.rs`](../lib/rt/kernel.rs) sind für x86-64 und ARM64 gegen die Header des NDK geprüft (ARM1).
- **Die Zonen sind eingebaut.** `datetime` bringt die Zonen aus der tz-Datenbank selbst mit
  ([`zones.rs`](../lib/datetime/zones.rs)); Android muss nur den Namen der eigenen Zone liefern.
- **`std`-Apps** (CR6) können auf Android heute schon `std::thread` und `std::net` nutzen. Das Netz braucht dafür
  ein eigenes Manifest mit `INTERNET`. Dieser Plan macht die Bibliothek selbst gleichwertig.

## Entscheidungen (Vorschläge)

1. **Android wie iOS.** Die Oberfläche bleibt auf dem Host-Weg. Darunter bekommt Android die Dienste ohne
   Oberfläche so wie iOS: Threads, `sync`, `net`, `fs`, `watch`, `certs`. Kein eigenes Desktop-Backend.
2. **Kernel-Aufrufe aus dem Linux-Code, Bionic nur, wo Android es verlangt.** Sockets, Dateien, Futex und
   inotify nehmen die Linux-Module von `plat` mit den Tabellen aus `rt/kernel.rs` (beide ABIs, `arm64-v8a` und
   `x86_64`). Bionic nehmen:
   - Threads: `pthread_create`, weil Bionic jedem Thread seinen TLS-Block einrichtet; ein eigener `clone` würde in
     der ersten Bionic-Funktion abstürzen,
   - Namen: `getaddrinfo`, weil Android DNS über `netd` auflöst (privates DNS, VPN, Netz pro App),
   - Zufall (`arc4random_buf`, schon da) und die Zeitzone (Systemeigenschaft `persist.sys.timezone`).
   Die Alternative wäre ein dritter Weg wie bei Apple: alles über libc-Funktionen. Das wäre mehr neuer Code
   ohne Gewinn, denn Androids Kernel-ABI ist die von Linux.
3. **Jede Prüfung auch im App-Prozess.** `adb shell` läuft in der SELinux-Domäne `shell` und ohne den
   seccomp-Filter, den Zygote jeder App setzt. Ein Lauf per `adb shell` ist nur eine schnelle Vorprüfung, fertig
   ist ein Meilenstein erst im APK (Domäne `untrusted_app`).
4. **Wecken über eine eventfd am Looper des UI-Threads.** `wake()` aus einem anderen Thread schreibt in eine
   eventfd, die der Host mit `ALooper_addFd` beobachtet; der Rückruf läuft auf dem UI-Thread und holt ein Bild.
   Kein JNI pro Wecken, kein Warten auf die 250 ms des Leerlaufs.
5. **`INTERNET` aus `PERMISSIONS`.** Das Standard-Manifest bekommt `INTERNET`, wenn die App kein `PERMISSIONS`
   hat (alles erlaubt) oder `net.connect` oder `net.listen` nennt. `tools/android.py` und `cargo jflib android`
   verweigern ein eigenes Manifest ohne `INTERNET` bei solchen Regeln, wie heute bei `location`.
6. **Nur die Zertifikate des Systems.** `certs` liest die CA-Ordner des Systems. Vom Nutzer installierte CAs
   bleiben draußen, wie bei Android-Apps ab API 24 üblich (Network Security Config).
7. **`MIN_API` bleibt 26.** Was Bionic erst später hat, nimmt der Linux-Code ohnehin als Syscall.
8. **Fehlen heißt `Unsupported`.** Module, die es auf Android nicht gibt (`process`, `service`), sind trotzdem
   da und antworten mit `Unsupported` oder `Unavailable`, wie heute schon `hotkey`. Apps bauen damit für alle Ziele
   ohne eigene `cfg`.

## Meilensteine

Größe: S klein, M mittel, L groß, nur relativ zueinander.

### AN0: Prüfwerkzeug (S)

- `coretest`, `plattest` und `asynctest` bekommen `android` in `TARGETS`, als Programme ohne Oberfläche
  (`build_exe` in `tools/android.py`), mit einem Läufer nach dem Muster von
  [`apps/portabletest/tests/android.py`](../apps/portabletest/tests/android.py): hochladen nach
  `/data/local/tmp`, starten, Ausgabe lesen, löschen.
- Ein Test-APK, das ein Konsolenprogramm im App-Prozess startet (der Einstieg aus `exe_main.c` hinter dem
  Java-Host), Ausgabe und Exit-Code ins Log und in eine Datei schreibt und sich dann beendet. Damit laufen
  dieselben Tests in `untrusted_app` mit seccomp.
- Eine Liste aller Syscalls, die `rt` und die Linux-Module von `plat` benutzen, gegen das, was eine App auf
  Android darf. Zuerst durch Probieren im Test-APK; die Listen aus AOSP (`SYSCALLS.TXT`, die seccomp-Listen) nur
  nach Rückfrage herunterladen.
- Was heute schon geht, bleibt grün: `androidGallery` mit seinen Skripten, `android_files.py`,
  `GalleryInstrumentation`.

**Fertig, wenn:** der heute auf Android lauffähige Teil von `coretest` per `adb shell` und im Test-APK läuft, auf
dem Emulator (x86-64, API 35) und auf dem A56, und die Syscall-Liste die Kandidaten für Ärger nennt.

**Stand 2026-11-17:** umgesetzt und auf dem Emulator grün; das A56 war nicht dran (nur auf Zuruf).

- `coretest`, `plattest` und `asynctest` haben `android` in `TARGETS`. `build_rs.py app NAME --target android` baut
  ein Programm ohne Oberfläche (`build_exe`; das gilt jetzt auch für Apps mit `ui`, aber ohne
  `android/AndroidManifest.xml`) und daneben sein Test-APK `NAME-<abi>-console.apk` (`build_console_apk` in
  `tools/android.py`): `android/exe_main.c` mit `-DJFGUI_CONSOLE_APK` als `libjfgui_console.so`, gestartet von
  [`ConsoleInstrumentation.java`](../tools/android_console/ConsoleInstrumentation.java) (Paket
  `org.jflib.console.NAME`, keine Activity, debuggable, Debug-Schlüssel) auf einem eigenen Thread mit 8 MiB Stack.
  Die Ausgabe geht nach `files/console.txt` (sie übersteht ein SIGSYS), danach nach logcat (Tag `jfgui-console`), der
  Exit-Code als letzte Zeile `exit=N`. Die erste Zeile nennt SELinux-Kontext und seccomp-Modus.
- Der Läufer [`tools/android_console.py`](../tools/android_console.py) `APP [--mode shell|app|both] [-- ARGS]`:
  `adb shell` in `/data/local/tmp/jfgui_APP/`, dann das APK über `am instrument`, die Ausgabe über `run-as`,
  danach deinstalliert. Ein App-Lauf außerhalb von `untrusted_app` oder ohne seccomp-Filter zählt als Fehler. Auf
  einem Gerät, das kein Emulator ist, installiert er das APK nur mit `--allow-install` (das geht über
  `/data/local/tmp` hinaus). `--file` legt Dateien an die Wurzel der App-Dateien (`files/fs/`), `--dir` leere Ordner.
  `build_rs.py test` nimmt mit `jfgui_TEST_ANDROID=SERIAL` `coretest` und `asynctest` (`tasks.uis`) mit.
- Was Android fehlt, meldet sich per `cfg` als „not on Android yet“ (fs, net, sync, Threads: AN1 bis AN3) oder als
  „not on Android“ mit Verweis auf [Bewusst nicht](#bewusst-nicht) (Kindprozesse, Autostart, Dateizuordnungen).
  `coretest` lässt dafür `datastore`, `netio` und `cas` auf Android weg
  (`[target.'cfg(not(target_os = "android"))'.dependencies]`; `derived_workspace` und `program_manifest_lint` in
  `build_rs.py` kennen solche Abschnitte jetzt).
- `ui::launch` nimmt auf Android wie auf iOS den Headless-Weg des Desktops für `--shot`, `--script` und die
  Agent-Protokolle; so laufen die Skripte von `asynctest` im Programm ohne Oberfläche (AN1 braucht sie im Test-APK).
  Das kostet jedes APK etwa 290 KB (die `androidGallery` als Debug-Build 5,97 statt 6,25 MB); ein Feature oder
  eine `cfg` nur für Test-Builds wäre die Alternative (Offene Fragen, 6). `rt::kernel` gibt es auch auf
  Android (nur die Tabelle, ohne die Helfer von `rt::os`). `exe_main.c` antwortet für den Klang „kein Gerät“.
- `android_files.rs` braucht `jfgui_ANDROID_FILES` und `jfgui_ANDROID_CACHE`; Läufer und Test-APK setzen sie wie
  `GalleryActivity`.

| Lauf (Emulator x86-64, API 35) | `adb shell` | App (`untrusted_app`, seccomp 2) |
|---|---|---|
| `coretest` | 3267 von 3267 Prüfungen (nach dem Merge von `origin/main`) | 3267 von 3267 |
| `plattest --test all` | 183 bestanden, 1 Fehler (Fund 2) | dasselbe |
| `asynctest --script tasks.uis`, `copy.uis` | grün | grün |
| `plattest --test syscalls` | 126 von 126 erreichen den Kernel | 96 erreichen den Kernel, 30 enden mit SIGSYS |

Funde von `plattest` (Fehler der Bibliothek auf Android):

1. **Behoben: `plat::date()` und `plat::unix_time_ns()` lieferten immer 1970-01-01.** Der Kalender kam aus
   `jfgui_date` ([`web.rs`](../lib/plat/web.rs)), und das ruft nur der Browser-Host (`web/worker.js`), weder
   `bridge.c` noch der Java-Host. Das traf jede Android-App (Uhrzeit, Dateizeiten, Logs, in AN2 die Gültigkeit von
   Zertifikaten). Jetzt nimmt Android die Uhr des Systems (Bionics `CLOCK_REALTIME`), UTC wie Linux; die
   Umrechnung teilen sich Linux, Apple und Android (`utc_date` in `plat`). Die Zone bleibt Sache von AN3.
2. **Behoben in AN3: `plat::mkdir` über einer vorhandenen Datei meldete Erfolg.** `mkdirs` in
   [`android_files.rs`](../lib/plat/android_files.rs) nahm EEXIST auch hin, wenn der Name eine Datei war.
3. `plat::cpu_brand()` ist leer (rt nimmt auf Android den portablen CPU-Code); die Quelle wäre `/proc/cpuinfo`.
   `plattest` meldet das als „not on Android yet“, ebenso `mem_total()` (AN1).

**Syscalls.** Alle Aufrufe von `rt` ([`os.rs`](../lib/rt/os.rs)) und den Linux-Modulen von `plat`
([`net.rs`](../lib/plat/net.rs), [`fs.rs`](../lib/plat/fs.rs), [`sync.rs`](../lib/plat/sync.rs),
[`watch.rs`](../lib/plat/watch.rs)), geprüft mit `plattest --test syscalls`: jeder Aufruf der Tabelle aus
[`rt/kernel.rs`](../lib/rt/kernel.rs) in einem eigenen Kindprozess (`fork`), mit Argumenten, die nichts tun können
(ein falscher Deskriptor, ein Null-Zeiger, abgelehnte Flags). Die Spalten nennen die Antwort des Kernels oder
**SIGSYS** (vom Filter beendet). Geprüft nur auf x86-64 (Emulator, API 35); arm64 hat andere Nummern und eine eigene
Liste des Filters, das zeigt erst der Lauf auf dem A56. AOSP-Listen wurden nicht heruntergeladen. (Der Lauf auf dem
A56 kam mit AN1: dort endet kein Aufruf mit SIGSYS, der Filter antwortet EPERM, und `openat2` erreicht den Kernel;
siehe AN1.)

| Syscall | x86-64 | arm64 | benutzt von | `adb shell` | App |
|---|---|---|---|---|---|
| `read` | 0 | 63 | watch | EBADF | EBADF |
| `write` | 1 | 64 | fs, rt | EBADF | EBADF |
| `close` | 3 | 57 | fs, net, watch | EBADF | EBADF |
| `fstat` | 5 | 80 | fs | EBADF | EBADF |
| `mmap` | 9 | 222 | rt | EINVAL | EINVAL |
| `munmap` | 11 | 215 | rt | EINVAL | EINVAL |
| `ioctl` | 16 | 29 | net | EBADF | EBADF |
| `sched_yield` | 24 | 124 | rt | ok | ok |
| `madvise` | 28 | 233 | rt | EINVAL | EINVAL |
| `socket` | 41 | 198 | net | EINVAL | EINVAL |
| `connect` | 42 | 203 | net | EBADF | EBADF |
| `sendto` | 44 | 206 | net | EFAULT | EFAULT |
| `recvfrom` | 45 | 207 | net | EFAULT | EFAULT |
| `shutdown` | 48 | 210 | net | EBADF | EBADF |
| `bind` | 49 | 200 | net | EBADF | EBADF |
| `listen` | 50 | 201 | net | EBADF | EBADF |
| `getsockname` | 51 | 204 | net | EBADF | EBADF |
| `getpeername` | 52 | 205 | net | EBADF | EBADF |
| `setsockopt` | 54 | 208 | net | EINVAL | EINVAL |
| `getsockopt` | 55 | 209 | net | EBADF | EBADF |
| `clone` | 56 | 220 | rt | EINVAL | EINVAL |
| `fcntl` | 72 | 25 | net, rt | EBADF | EBADF |
| `fsync` | 74 | 82 | fs | EBADF | EBADF |
| `fchmod` | 91 | 52 | fs | EBADF | EBADF |
| `geteuid` | 107 | 175 | fs | ok | ok |
| `statfs` | 137 | 43 | fs | EFAULT | EFAULT |
| `futex` | 202 | 98 | sync | EFAULT | EFAULT |
| `getdents64` | 217 | 61 | fs | EBADF | EBADF |
| `exit_group` | 231 | 94 | rt | ok | ok |
| `inotify_add_watch` | 254 | 27 | watch | EINVAL | EINVAL |
| `inotify_rm_watch` | 255 | 28 | watch | EBADF | EBADF |
| `openat` | 257 | 56 | fs | EFAULT | EFAULT |
| `mkdirat` | 258 | 34 | fs | EFAULT | EFAULT |
| `fchownat` | 260 | 54 | fs | EINVAL | EINVAL |
| `newfstatat` | 262 | 79 | fs | EINVAL | EINVAL |
| `unlinkat` | 263 | 35 | fs | EINVAL | EINVAL |
| `renameat` | 264 | 38 | fs, rt | EFAULT | EFAULT |
| `linkat` | 265 | 37 | fs | EINVAL | EINVAL |
| `symlinkat` | 266 | 36 | fs | EFAULT | EFAULT |
| `readlinkat` | 267 | 78 | fs | EINVAL | EINVAL |
| `fchmodat` | 268 | 53 | fs | EFAULT | EFAULT |
| `ppoll` | 271 | 73 | rt | EFAULT | EFAULT |
| `utimensat` | 280 | 88 | fs | EFAULT | EFAULT |
| `accept4` | 288 | 242 | net | EBADF | EBADF |
| `dup3` | 292 | 24 | rt | EINVAL | EINVAL |
| `inotify_init1` | 294 | 26 | watch | EINVAL | EINVAL |
| `renameat2` | 316 | 276 | fs, rt | EINVAL | EINVAL |
| `openat2` | 437 | 437 | fs | EFAULT | **SIGSYS** |

- **Kandidat für Ärger: `openat2`.** `fs.rs` öffnet damit unter einem offenen Ordner ohne Links auf dem Weg
  (`plat::fs::Dir`); im App-Prozess endet der Aufruf mit SIGSYS. AN3 braucht auf Android einen Weg ohne `openat2`
  (etwa `openat` Schritt für Schritt mit `O_NOFOLLOW`). `renameat2` erreicht den Kernel.
- Die übrigen 29 Treffer des Filters stammen aus Linux-Modulen, die auf Android nicht laufen: SysV-Shared-Memory der
  X11-Ausgabe (`shmget`, `shmat`, `shmctl`, `shmdt`) und die Sperrliste der Sandbox in `linux_sandbox.rs` (`chroot`,
  `mount`, `umount2`, `init_module`, `delete_module`, `kexec_load`, `add_key`, `request_key`, `keyctl`,
  `open_by_handle_at`, `finit_module`, `kexec_file_load`, `io_uring_setup`, `io_uring_enter`, `io_uring_register`,
  `open_tree`, `move_mount`, `fsopen`, `fsconfig`, `fsmount`, `fspick`, `mount_setattr`, `landlock_create_ruleset`,
  `landlock_add_rule`, `landlock_restrict_self`). Die anderen 96 Aufrufe der Tabelle erreichen den Kernel, darunter
  `clone`, `execve`, `ptrace`, `bpf`, `seccomp`, `unshare` und `memfd_create`, aus der Sperrliste auch `clone3` und
  `pidfd_getfd`. `rt_sigreturn` prüft die Probe nicht (Bionic braucht ihn für jeden Signal-Handler).
- EACCES oder EPERM kämen von SELinux oder fehlenden Rechten, nicht vom Filter: Ohne `INTERNET` scheitert `socket()`
  mit EACCES. Die Probe gibt ungültige Argumente und sieht nur den Filter.

### AN1: Threads, Pool, Wecken (M)

- `thread_spawn`, `Thread::join` und `Sem` über Bionic (`pthread_create`, Futex), nach `android_gpu.rs` und
  `apple_thread.rs`; Stackgrößen wie auf Linux, gerundet auf 16-KiB-Seiten.
- `cpu_count()` aus `sysconf(_SC_NPROCESSORS_ONLN)`, `mem_total()` aus `/proc/meminfo`, `thread_yield()`.
- `sync` (Mutex, Condvar, RwLock, Kanäle) mit dem Futex-Code von Linux.
- Android verlässt `web_pool.rs` und nimmt [`pool.rs`](../lib/plat/pool.rs) wie iOS. Damit laufen `parallel_for`,
  der Blocking-Pool von `task.rs` und `aio` auf echten Threads, und `render_par` kann auf Android rechnen.
- Wecken nach Entscheidung 4: eventfd, `ALooper_addFd` in `bridge.c`, der Rückruf holt ein Bild.
- Panics in Threads enden wie überall mit Exit-Code 3 (`rt::abort_on_unwind`). Ob `plat` auf Android eigene
  Signal-Handler für Absturzberichte setzt, entscheidet AN1 erst nach einem Blick auf `debuggerd` (Fallen).
- `capabilities()` meldet `CAP_THREADS`.

**Fertig, wenn:** die Thread-, Pool- und `sync`-Tests von `plattest` und `asynctest` im Test-APK auf dem Emulator
und dem A56 grün sind, die Oberfläche der `androidGallery` während eines langen Jobs flüssig bleibt (Skript) und
`android_perf.py` den Unterschied auf dem A56 misst.

**Erst nach dem Merge von ARM7:** ARM7 ändert `web_pool.rs` und prüft die Speicherordnungen in `pool.rs` und
`sync.rs` (Branch `claude/arm7-atomics-b63ee5`, Stand 2026-11-17 noch nicht in `main`).

**Stand 2026-11-18:** umgesetzt, auf dem Emulator und dem A56 grün (beide Modi). Damit ist auch der A56-Teil von AN0
erledigt (`coretest` per `adb shell` und im Test-APK).

- [`android_thread.rs`](../lib/plat/android_thread.rs) (neu, aus `web.rs` eingebunden wie `android_gpu.rs`):
  `thread_spawn` und `Thread::join` über Bionics `pthread_create` / `pthread_join`, Stack 1 MB ohne Angabe, höchstens
  256 MB, aufgerundet auf 16 KiB; Bionic richtet TLS, Signal-Stack und Schutzseite selbst ein. `Sem` ist der
  Futex-Zähler von `linux_thread.rs`, `plat::sync` nimmt den Futex-Teil von Linux (`sync.rs`, nur die `cfg`).
  `cpu_count()` aus `sysconf(_SC_NPROCESSORS_ONLN)`, `mem_total()` aus `MemTotal` in `/proc/meminfo`,
  `thread_yield()` über `sched_yield`, `sleep_ms()` schläft jetzt wirklich (`ppoll` ohne Deskriptoren, wie Linux;
  vorher tat es auf Android nichts, was mit Threads Warteschleifen zu Spinnern machte), `plat_tid()` per `gettid`
  (für die Executoren in `task.rs`).
- `rt::os` hat auf Android jetzt `syscall6`, `syscall3`, `poll` und die Tabelle aus `rt::kernel` (wie unter Linux), für
  Futex, eventfd, `/proc` und später die Linux-Module von AN2 und AN3 (Entscheidung 2).
- Android nimmt [`pool.rs`](../lib/plat/pool.rs) statt `web_pool.rs` (wie iOS): `parallel_for`, `Job`, der
  Blocking-Pool von `task.rs` und `aio` laufen auf Threads, `thr_count()` ist `cpu_count()`, `render_par` rechnet in
  Bändern. `jfgui_tick` ruft `pump_jobs` nur noch im Browser. `capabilities()` meldet `CAP_THREADS`.
- Die Arbeiter des Pools und des Blocking-Pools laufen auf Android unter dem UI-Thread: nice 10, was
  `Process.setThreadPriority(THREAD_PRIORITY_BACKGROUND)` setzt (`thread_background` in `android_thread.rs`). Ohne
  das verdrängten vier rechnende Arbeiter auf vier Kernen den UI-Thread im Bitmap-Weg (er rasterisiert selbst)
  bis zu 385 ms lang; mit nice 10 ist die längste Lücke während eines Jobs so lang wie ohne Job, der Job braucht
  etwa 8 % länger (Emulator, `android_job.py --gpu 0`). Wo die Oberfläche ruht, bekommen die Arbeiter jeden Kern.
- Wecken nach Entscheidung 4: `wake()` setzt die Marke und schreibt in eine eventfd (beim ersten Gebrauch angelegt,
  `jfgui_wake_fd`). [`bridge.c`](../android/bridge.c) meldet sie nach dem Start mit `ALooper_addFd` am Looper des
  UI-Threads an; der Rückruf leert sie und ruft `GalleryActivity.frameNow()` (ein JNI-Aufruf auf dem UI-Thread,
  keiner im weckenden Thread), dessen Bild in `jfgui_tick` aus der Marke ein `E_WAKE` macht. Fand der Rückruf nichts
  mehr, hatte ein Bild das Wecken schon geholt. Ein Programm ohne Oberfläche (`exe_main.c`) wartet in
  `wait_events` mit `ppoll` auf dieselbe eventfd, wie unter Linux. `android_gpu.rs` startet seine Jobs jetzt mit
  `thread_spawn`.
- `cpu_brand()` (Fund 3 von AN0) liest `/proc/cpuinfo`: auf x86-64 `model name` (im Emulator der Prozessor des
  Rechners), auf ARM64 `Hardware`, wo der Kernel es noch schreibt, sonst jede Kernart einmal aus `CPU implementer`
  und `CPU part` mit der MIDR-Liste von Linux ARM64 (`midr_name` in `cpu.rs`, jetzt geteilt).
- Panics in Threads enden mit Exit-Code 3 (`rt::abort_on_unwind`, wie überall). Eigene Signal-Handler setzt `plat`
  auf Android nicht: Signale gehören Bionic und `debuggerd`, das für einen Absturz einen Tombstone und den
  Stapel in logcat schreibt. Ein eigener Handler müsste sich vor den von `debuggerd` hängen und weiterreichen; das
  bringt nur einen `CrashInfo` für Hardwarefehler, den sonst keiner auf Android liest. Entscheidung: so lassen,
  bis eine App Absturzberichte selbst sammeln will.
- Tests wieder eingeschaltet: `plattest --test threads` (dazu `CAP_THREADS`), `--test atomics` (ARM7), `--test async`
  (auch `drops` mit den Kanälen), `mem_total` und `cpu_brand` in `--test all`; `coretest`: die Gruppe `plat` mit
  `sync` und `rand` (Netz, Dateien, `watch`, Zone und `service` melden AN2 bis AN4) und der 64-KB-Thread von `pattern`.
- Die `androidGallery` hat einen Selbsttest ohne eigene Texte: mit `jfgui_SELFTEST=job`
  ([`jobtest.rs`](../apps/androidGallery/jobtest.rs)) läuft ein langer Job (240 Stücke zu 20 ms Rechnung) auf dem
  Pool, während ein Timer alle 16 ms ein Bild anfordert (der Fortschrittsbalken der Karte 01 folgt ihm); danach enden fünf
  Jobs mit einem schlafenden Stück, während die Oberfläche ruht. Das Skript
  [`android_job.py`](../apps/androidGallery/tests/android_job.py) startet ihn (nach einer Installation erst ein
  Start zum Aufwärmen) und prüft: Arbeiter im Pool, höchstens `--max-over` (2) Lücken über `--max-gap-ms` (150 ms)
  während des Jobs, eine Bildrate von mindestens der Hälfte der ohne Job, kein Wecken über `--max-wake-ms` (100).
  Die Bilder fordert ein Timer von 16 ms an, wie eine Animation: ein `invalidate` in jedem Bild hält die UI nach acht
  Bildern an (`UIWARN f13`), daran scheiterte der erste Entwurf.

- Auf dem A56 (SM-A566B, Android 16, API 36; 4 × Cortex-A520 mit 1,95 GHz, 3 × Cortex-A720 mit 2,6 GHz, 1 × mit
  2,91 GHz) laufen dieselben Prüfungen grün, in `adb shell` und im Test-APK. `cpu_brand()` sagt dort
  „ARM Cortex-A520 + ARM Cortex-A720“, `mem_total()` 7417 MB. Die Gallery lief als Test-Build unter eigenem Paket
  (`jfgui_ANDROID_PACKAGE=org.jflib.gallery.an1`, danach deinstalliert): die installierte Gallery hat einen anderen
  Debug-Schlüssel und blieb unberührt. `android_job.py` und `android_perf.py` haben dafür `--package`.
- **Syscalls auf arm64** (`plattest --test syscalls` auf dem A56, Fund für AN0 bis AN3): im App-Prozess erreichen alle
  125 Aufrufe den Kernel, keiner endet mit SIGSYS. Der Filter von Android 16 auf arm64 antwortet mit EPERM, wo
  `adb shell` etwas anderes bekommt: `chroot`, `mount`, `kexec_load`, `add_key`, `keyctl`, `io_uring_setup`,
  `open_tree`, `mount_setattr`, `shmget` (die Probe zählt EPERM als „erreicht den Kernel“; erst der Vergleich mit
  der Shell zeigt den Filter). **`openat2` erreicht dort den Kernel** (E2BIG), im x86-64-Emulator mit API 35 endet es
  mit SIGSYS. AN3 braucht den Weg ohne `openat2` also trotzdem, kann ihn aber auf Geräten wie dem A56 nur als
  Ausweichweg nehmen. Futex, eventfd2, ppoll, gettid und `sched_getaffinity` gehen auf beiden.
- **big.LITTLE** (`plattest --test pool-bench`, neu: `parallel_for` mit 1 bis `cpu_count + 1` Threads, je 512 Stücke,
  das beste von drei): Rechnen skaliert auf dem A56 bis zum Schluss, 1,9× mit 2 Threads, 3,3× mit 4, 4,3× mit 8 und
  4,5× mit 9 (8 Arbeiter und der Aufrufer, so wie der Pool heute startet); im Test-APK 2,0×, 2,8×, 4,3×, 4,5×.
  Speicherlastige Stücke (256 KB je Stück) sind dafür zu kurz und streuen (3× bis 4× ab 7 Threads). Acht Threads
  bringen also etwa 30 % mehr als vier, die sparsamen Kerne lohnen sich; weil sich die Threads die Stücke einzeln holen,
  bremsen sie nicht, solange eine Schleife viele kleine Stücke hat. **Entscheidung: der Pool bleibt bei `cpu_count()`
  Arbeitern**; eine Schleife mit wenigen großen Stücken endet aber mit dem langsamsten Kern, so wenige Stücke
  sollte man auf Telefonen nicht schneiden.
- **`render_par` auf dem Telefon** (`android_perf.py --page colors`, Bitmap-Weg, 768 × 1496): im Automatikmodus rechnet
  es auf dem A56 gar nicht, denn es beginnt erst bei 3 Mio. Pixeln Schaden (`PAR_MIN_PX`), das Bild hat 1,15 Mio. Mit
  `jfgui_RENDER_PAR=1` (immer in Bändern) wird ein Bild langsamer: im Mittel 7,3 bis 7,7 ms statt 6,6 ms, 28 bis 33 %
  ruckelnde Bilder statt 15 %. Die Schwelle bleibt also; mit und ohne den Pool kostet ein Bild auf dem A56 gleich viel
  (6,5 ms). Während der Messung spiegelte Android Studio den Bildschirm (das Skript warnt, die Zeiten sind etwas zu hoch).

| Lauf | Emulator x86-64, API 35, 4 Kerne | A56 (arm64, API 36, 8 Kerne) |
|---|---|---|
| `plattest --test threads` | 32 von 32 (beide Modi) | 32 von 32 (beide Modi) |
| `plattest --test atomics` | 15 von 15 | 15 von 15 |
| `plattest --test async` | 85 von 85 | 85 von 85 |
| `plattest --test all` | 217 bestanden, 1 Fehler (Fund 2, offen) | dasselbe |
| `plattest --test syscalls` (App) | (AN0) 96 erreichen den Kernel, 30 SIGSYS | 125 erreichen den Kernel, 0 SIGSYS |
| `coretest` | 3281 von 3281 | 3281 von 3281 |
| `asynctest --script tasks.uis`, `copy.uis` | grün | grün |

`android_job.py` (je drei Runden, die Werte von–bis; ohne Job: dieselben Bilder eine Sekunde vor dem Job):

| Weg | Bilder/s ohne Job | längste Lücke ohne Job | Bilder/s mit Job | längste Lücke mit Job | Job (240 × 20 ms) | Wecken nach Job-Ende |
|---|---|---|---|---|---|---|
| Emulator, GPU (SurfaceView, SwiftShader) | 43–53 | 61–63 ms | 53–63 | 63–82 ms | 1,42–1,46 s | 0–32 ms |
| Emulator, Bitmap (`--gpu 0`) | 49–51 | 61–62 ms | 44–65 | 63–140 ms | 1,41–1,83 s | 0–32 ms |
| A56, GPU (Vulkan) | 61–65 | 34–37 ms | 80–81 | 24–25 ms | 0,62–0,64 s | 5–19 ms |
| A56, Bitmap (`--gpu 0`) | 61 | 33–48 ms | 79–82 | 24–27 ms | 0,61–0,63 s | 3–22 ms |

Auf dem A56 rechnen 8 Arbeiter den Job (die Stücke sind 20 ms Zeit, nicht Rechnung: 240 × 20 ms / 8 = 0,6 s), und
keine Lücke kam über 50 ms. Die Grundlinie ohne Job ist die erste Sekunde nach dem Aufwärmen und hat ihre eigene
längste Lücke (33 bis 48 ms).
Vor AN1 liefen die Stücke zwischen den Bildern auf dem UI-Thread (`pump_jobs(16)`: 16 × 20 ms je Tick, also über
320 ms zwischen zwei Bildern), und ein Job, der endete, während die Oberfläche ruhte, kam erst mit dem nächsten
Tick des Hosts (bis 250 ms). Ohne nice 10 für die Arbeiter lagen die längsten Lücken im Bitmap-Weg bei 86 bis
385 ms. Die Gallery-Instrumentierungen (`GalleryInstrumentation`, `GalleryA11yInstrumentation`) bleiben grün.

### AN2: Netz (M)

- `plat::net` auf Android: der Linux-Teil von [`net.rs`](../lib/plat/net.rs) für Sockets (TCP, UDP, IPv4, IPv6,
  Timeouts). Namen über `getaddrinfo` wie bei Apple (`resolve_system`), nicht über `netio::dns`, denn
  `/etc/resolv.conf` gibt es nicht. `netio::dns` bleibt für eigene Anfragen an genannte Server.
- Die Schnittstellen des Geräts: was ohne Netlink geht (`SIOCGIFCONF`, `/sys/class/net`), ohne MAC-Adressen.
- `certs`: die CA-Ordner des Systems, zuerst `/apex/com.android.conscrypt/cacerts` (ab Android 14), dann
  `/system/etc/security/cacerts`; dort liegt ein PEM pro Datei. `root_bundle` liefert sie,
  `verify_tls_server` und `verify_tls_server_roots` antworten `None` wie auf Linux (netio prüft gegen
  die Ordner; Schnittstelle seit Phase 2c des Sicherheitsplans, `lib/plat/certs.rs`). Die Gruppe
  `systrust` von `coretest` und `tools/tls_web_check.py --internet-only` bekommen dann einen Android-Weg.
- `netio` in der Fassade auch für Android, `docs.rs` baut es für `aarch64-linux-android`.
- Manifest nach Entscheidung 5, in `cargo jflib android` und `tools/android.py`.
- Ohne `INTERNET` scheitert `socket()` mit EACCES. Das wird ein klarer `NetError` mit Hinweis auf das Manifest.
- Die Durchsetzung von `net.connect` und `net.listen` (`policy`) gilt dann auch auf Android.

**Fertig, wenn:** die Netz-, DNS-, TLS- und HTTP-Tests von `coretest` im Test-APK auf dem Emulator und dem A56 grün
sind, die Prüfungen von `tools/tls_web_check.py --internet` auf dem A56 öffentliche Server erreichen (das Skript
bekommt dafür einen Android-Weg; nur mit Zustimmung, im WLAN) und ein Lauf ohne `INTERNET` die richtige Meldung zeigt.

**Stand 2026-11-27:** fertig. Auf dem Emulator und dem A56 grün (beide Modi, auch ohne `INTERNET`), und auf dem A56
erreicht `tools/tls_web_check.py` die öffentlichen Server aus der Shell und aus dem Test-APK (unten).

- [`net.rs`](../lib/plat/net.rs): Android nimmt das Linux-Modul `sys` (Sockets über `rt::os` mit den Nummern aus
  `rt::kernel`: TCP, UDP, IPv4, IPv6, Timeouts, `SO_PEERCRED`). Namen über Bionics `getaddrinfo` (`resolve_system`,
  dieselbe Liste wie bei Apple, aber mit Bionics `struct addrinfo`), das `netd` fragt: privates DNS, VPN, das Netz der
  App, `/system/etc/hosts`. `dns_servers()` ist leer (`netd` behält sie), `hostname()` aus Bionics `gethostname`.
  Die Schnittstellen ohne Netlink und ohne MAC-Adressen: IPv4 über `SIOCGIFCONF`, IPv6 aus `/proc/net/if_inet6`, wo
  die App es lesen darf, `if_index` über `/sys/class/net/NAME/ifindex`, sonst `SIOCGIFINDEX`; Dateien von `/proc` und
  `/sys` über den echten Pfad (`proc_file` aus AN1), nicht über die Namen von `android_files.rs`.
- Unix-Sockets bleiben auf Android vorerst weg: Pfad, Aufräumen und `socket_id` brauchen `plat::fs` (AN3).
- **Ohne `INTERNET`** bekommt die App keinen IPv4- oder IPv6-Socket. Der Code ist nicht überall derselbe: EACCES,
  wo der Kernel die Gruppe `inet` prüft, **EPERM** auf dem Emulator mit Android 15 und dem A56 mit Android 16 (dort
  prüft das eBPF-Programm von `netd`). Beides wird `NetErrorKind::Denied` mit diesem Code; `NetError::no_internet()` erkennt es, die Meldung
  nennt das Manifest. `getaddrinfo` sagt ohne `INTERNET` nur „keine Adresse“; schlägt eine Auflösung fehl, prüft
  `resolve` deshalb mit einem Socket nach und meldet ebenfalls `no_internet`.
- [`certs.rs`](../lib/plat/certs.rs): `root_bundle()` liest den ersten Ordner aus `ANDROID_CA_DIRS`, der Dateien hat
  (`/apex/com.android.conscrypt/cacerts`, dann `/system/etc/security/cacerts`), eine Datei je CA (PEM, danach ein
  Textauszug, den `pem::decode_all` überliest), sortiert nach Namen, über `getdents64` (ohne `plat::fs`).
  `android_ca_dir()` nennt den Ordner. Nur die CAs des Systems (Entscheidung 6): `SSL_CERT_FILE` und
  `SSL_CERT_DIR` zählen auf Android nicht, vom Nutzer installierte CAs auch nicht. `verify_tls_server` und
  `verify_tls_server_roots` antworten `None` wie auf Linux; netio prüft selbst (`roots::verify_system`).
- `policy`: `net.connect` und `net.listen` gelten auf Android (vorher blieben die Regeln dort ungenutzt). Dazu
  zählen jetzt auch die Thread-Kennung (`plat_tid`) und das Warten anderer Threads auf eine Antwort (`wait_answer`);
  bis AN2 hatten alle Threads unter Android die Kennung 0, die Bereiche von Plugins (`scoped_here`) hätten also
  Threads verwechselt.
- `netio::dns::resolve` nimmt auf Android den Weg des Systems (`plat::net::resolve`); `Config::system()` hat dort
  keine Namensserver und keine Hosts-Datei (es gibt kein `/etc/resolv.conf`), `resolve_with` bleibt für Anfragen an
  genannte Server.
- Fassade: `netio` ist auf Android dabei (`lib/jflib/Cargo.toml`, `src/lib.rs`), `docs.rs` baut netio auch für
  `aarch64-linux-android`. `cas`, `datastore` (AN3) und `update` (Bewusst nicht) fehlen dort weiter.
- Manifest nach Entscheidung 5: `cargo jflib android` (`default_manifest`) schreibt `INTERNET`, wenn die App kein
  `PERMISSIONS` hat oder dort `net.connect` oder `net.listen` nennt; `manifest_gaps` (dort und in
  `tools/android.py`) verweigert ein eigenes Manifest ohne `INTERNET` bei diesen Regeln. Ein eigenes Manifest einer
  App ohne `PERMISSIONS` darf `INTERNET` weglassen (die Sockets sagen es dann). Das Test-APK
  (`build_console_apk`) bekommt `INTERNET` nach derselben Regel; `jfgui_ANDROID_NO_INTERNET=1` baut
  `NAME-<abi>-console-nonet.apk` ohne, `tools/android_console.py --no-internet` startet es (nur der App-Modus, die
  Shell hat das Netz immer).
- `coretest`: die Gruppen `http`, `tls`, `tlsweb`, `systrust`, `tlsres`, `ocsp`, `acme`, `dns` und `tlscore` laufen
  auf Android, dazu die Server-Modi außer SSH und in `plat` die Sockets (`addrs`, TCP über IPv4 und IPv6, UDP) und
  `android_net`: `getaddrinfo` über `/system/etc/hosts` (`ip6-localhost`), ein unbekannter Name, die Adressen und
  Schnittstellen, ohne `INTERNET` die Meldung. `systrust` prüft auf Android die CAs des Systems (ISRG Root X1 dabei,
  `SSL_CERT_FILE` ohne Wirkung, eine eigene CA nicht darunter). `ssh` braucht Kindprozesse und bleibt aus.

| Lauf (Emulator x86-64, API 35) | `adb shell` | App (`untrusted_app`, seccomp 2) |
|---|---|---|
| `coretest` (alle Gruppen) | 5143 von 5143 (vor AN2: 3281) | 5140 von 5140 (Fund 1) |
| davon `http`, `tls`, `tlsweb`, `tlsres`, `ocsp`, `acme`, `tlscore` | 38, 52, 94, 62, 115, 76, 148 | dasselbe |
| `dns` | 53 | 50 (Fund 1) |
| `systrust` | 22, die CAs aus `/apex/com.android.conscrypt/cacerts` | dasselbe |
| `plat` (mit Sockets und `android_net`) | 48 | 48 |
| `coretest --only plat` ohne `INTERNET` (`--no-internet`) | – | 19 von 19, Meldung „lacks android.permission.INTERNET … (os error 1)“ |

Auf dem A56 (SM-A566B, arm64, Android 16, API 36, im WLAN) dieselben Zahlen in jeder Zeile: `coretest` 5143 von 5143
in der Shell (104 s) und 5140 von 5140 im APK (100 s), `dns` 53 und 50, `systrust` aus
`/apex/com.android.conscrypt/cacerts`, ohne `INTERNET` 19 von 19 mit derselben Meldung (os error 1, also auch dort
EPERM). Die Shell sieht dort die IPv4- und alle IPv6-Adressen, das APK nur `127.0.0.1` und die IPv4-Adresse (Fund 1).

**Öffentliche Server:** `python tools/tls_web_check.py --internet-only --target android [--serial S] [--mode
shell|app|both] [--allow-install]` ist der Android-Weg des Skripts. Es nimmt `build/android/coretest-<ABI>` und das
Test-APK (`android_console.py`), installiert es einmal für alle Hosts und deinstalliert es am Ende; jeder Host läuft
als eigenes `am instrument` mit `--tls-web URL`, ein Lauf außerhalb von `untrusted_app` mit seccomp zählt als
falsch. Auf dem A56 im WLAN (2026-11-27) waren alle 12 gewerteten Hosts in beiden Modi richtig, keiner
unerreichbar: www.google.com mit TLS 1.3 und X25519MLKEM768 (0x11EC), github.com mit TLS 1.3 und X25519, die
badssl-Hosts mit TLS 1.2; expired und sha384 „certificate expired“, wrong.host „certificate for another name“,
self-signed und untrusted-root „certificate not trusted“. Nur gezeigt: revoked.badssl.com wird genommen (wie auf
Linux: ohne Staple kein Widerruf), incomplete-chain „not trusted“ (wie macOS und Linux, nichts wird nachgeladen).

Funde:

1. **Im App-Prozess keine IPv6-Adressen.** `/proc/net/if_inet6` darf eine App nicht lesen (seit Android 10), also
   liefert `local_addrs()` dort nur IPv4 (`SIOCGIFCONF`); in der Shell kommen die IPv6-Adressen mit. `dns` übergeht
   deshalb im APK die drei Prüfungen mit einem Namensserver auf einer link-lokalen Adresse. Ohne Netlink gibt es
   keinen anderen Weg; Bionics `getifaddrs` fragt per `RTM_GETADDR`, was Apps noch dürfen (nur `bind` und
   `RTM_GETLINK` sind gesperrt). Ob `local_addrs` das als Ausweg nehmen soll, ist offen (Entscheidung „ohne Netlink“).
2. **EPERM statt EACCES.** Ohne `INTERNET` lehnen Android 15 (Emulator) und Android 16 (A56) `socket()` mit EPERM
   ab, nicht mit EACCES wie im Plan (Fallen); `plat::net` nimmt beide.
3. **`policy::tid()` war auf Android immer 0**, obwohl es seit AN1 Threads gibt; behoben (siehe oben).
4. `ssh` und der TCP-Teil von `sshcore` brauchen den Test-Server aus `sshd.rs`, der `plat::process` nimmt; sie
   bleiben auf Android aus. Unix-Sockets warten auf AN3.
5. Clippy (`build_rs.py check --target android`) bricht auf x86-64 schon in `rt` ab: `rt/lib.rs` lädt
   `portable/fx.rs` zweimal (`duplicate_mod`). Das ist älter als AN2; Clippy über `plat`, `netio` und `coretest` für
   `aarch64-linux-android` ist sauber.

Geprüft außerdem: `cargo check -p jflib` mit und ohne `std` ohne Warnung für Windows (x86-64, ARM64), Linux
(x86-64, ARM64), wasm32, Android (x86-64, ARM64) und macOS (ARM64); `build_rs.py check` (Lint, Clippy für Windows
und Linux); `build_rs.py test coretest` auf Windows und Linux; die Tests von `cargo-jflib`
(`internet_follows_the_network_rules`).

### AN3: Dateisystem, Ordnerüberwachung, Crates (M)

- `plat::fs` auf Android: `stat`, `lstat`, `read_dir`, Links, `set_times`, `free_space`, `write_file_private` mit
  dem Linux-Code, auf den virtuellen Namen von `android_files.rs` und auf echten Pfaden (für `std`-Apps und
  `/system/fonts`). `struct stat` unterscheidet sich zwischen `x86_64` und ARM64; beide stehen in `rt/kernel.rs`.
- `local_zone_id` aus `persist.sys.timezone`; die Zone selbst kommt aus den eingebauten Daten von `datetime`.
- `watch` mit inotify (der Linux-Teil von [`watch.rs`](../lib/plat/watch.rs)) für die eigenen Ordner der App.
- Hardlinks: im App-Prozess prüfen, sonst ein sauberes `Unsupported`.
- `datastore` und `cas` in der Fassade auch für Android.

**Fertig, wenn:** die Dateisystem- und `watch`-Tests von `coretest` und `plattest` sowie die Tests von `datastore`
und `cas` im Test-APK grün sind und `android_files.py` unverändert besteht.

**Stand 2026-11-28:** umgesetzt, auf dem Emulator und dem A56 grün (beide Modi; das A56 am 2026-11-28 auf Zuruf
des Nutzers, siehe unten).

- `plat::fs` und `plat::watch` sind auf Android die Linux-Teile von [`fs.rs`](../lib/plat/fs.rs) und
  [`watch.rs`](../lib/plat/watch.rs) (nur `cfg`): die Syscalls über `rt::os`, `struct stat` aus `rt::kernel` (x86-64 und
  ARM64). Sie nehmen die Namen von [`android_files.rs`](../lib/plat/android_files.rs): `os_z` macht daraus den echten
  Pfad für den Kernel, die Regeln (`policy`) sehen den Namen. `\` trennt wie in diesen Namen und in den Regeln (`join`,
  `parent`, die Namen eines `Dir`). `rt::os` hat auf Android dafür auch `rename` (`renameat2`, sonst `renameat`).
- **Namen und echte Pfade (Entscheidung):** `/`, `/config`, `/data`, `/cache`, `/tmp`, `/documents`, `/imports` und
  alles Übrige unter `/` bleiben Namen im Speicher der App. Die Ordner des Systems selbst (`/system`, `/system_ext`,
  `/product`, `/vendor`, `/odm`, `/apex`, `/proc`, `/sys`, `/dev`, `/storage`, `/sdcard`, `/mnt`) sind ihre echten
  Pfade, für `plat::fs` und genauso für `read_file`, `dir_list` und die übrigen Dateifunktionen: eine Regel in
  `android_files.rs` (`SYSTEM`); so liest AN4 `/system/fonts` ohne eigenen Weg. Einen echten Pfad in den Speicher
  der App (`/data/user/0/PAKET/files/...`) gibt es nicht: `/data` ist schon ein Name, und ein zweiter Name derselben
  Datei liefe an den Regeln vorbei (`fs.write /data/**` träfe sonst `/config`). `std`-Apps nehmen für `std::fs` den
  echten Ordner, für `plat` die Namen.
- **Kein `openat2`:** `Dir::open_dir` geht auf Android immer Schritt für Schritt (`openat` mit `O_NOFOLLOW` unter dem
  offenen Ordner); der Versuch mit `openat2` ist dort nicht einmal gebaut. `plattest --test kernel` läuft dafür jetzt
  auch auf Android (ohne `fork`, `clone` und `dup2`, die Helfer von `rt::os` nur unter Linux): 65 von 65 im
  App-Prozess des x86-64-Emulators, wo `openat2` SIGSYS gäbe, und des A56 (arm64, dort mit der `struct stat` von
  ARM64; `openat2` wäre dort erlaubt und bleibt ungenutzt).
- **Links:** ein absolutes Ziel (`symlink`, `Dir::symlink`) liegt als echter Pfad im Link (der Kernel löst Links auf
  echten Pfaden auf), `read_link` liest es wieder als Namen; relative Ziele bleiben, wie sie sind.
- **Hardlinks:** SELinux verweigert sie einer App (`avc: denied { link }` für `untrusted_app` auf `app_data_file`),
  auch im eigenen Speicher, und `adb shell` ebenso. `fs::hard_link` antwortet auf Android `false`, ohne den Kernel zu
  fragen (jeder Versuch hinterließe eine Zeile im Log); es ist `bool`, ein eigenes `Unsupported` hat es nicht.
- **Zone:** `local_zone_id` nimmt `TZ`, sonst die Systemeigenschaft `persist.sys.timezone` (`__system_property_get`,
  wie Bionics `tzset`); die Zone kennt `datetime` (Emulator: `GMT`).
- **`watch`:** inotify wie unter Linux; Wurzeln und gemeldete Pfade sind Namen, die Uhren liegen auf den echten
  Ordnern. Auch die Lücken (ein Watcher gleich nach `new`, eine gelöschte und neue Wurzel, eine verschobene Wurzel,
  eine Liste, die beim ersten Mal scheitert) prüft `coretest` auf Android.
- **Fund 2 von AN0 behoben:** `mkdirs` nimmt EEXIST am Ende nur noch für einen Ordner hin; `plat::mkdir` über einer
  Datei ist `false`.
- `File::create_new` gibt es auch auf Android (`O_EXCL`, 0600), damit auch die exklusiven Geschwister von
  `AtomicFile` auf Ordner-Mounts; `plattest --test content` prüft es wieder.
- Fassade: `cas` und `datastore` auch auf Android (`lib/jflib/Cargo.toml`, `src/lib.rs`), docs.rs baut sie für
  `aarch64-linux-android`. `coretest` nimmt `datastore` und `cas` mit, seine Gruppe `plat` prüft Dateien, `watch`
  und die Zone (`service` meldet AN4; das Netz seit dem Mischen mit AN2, ohne Unix-Sockets). `plattest`: `vfs`, `kernel`, `trash` (Android hat keinen
  Papierkorb: `false`, die Datei bleibt) und `content` laufen auf Android.

| Lauf (Emulator x86-64, API 35) | `adb shell` | App (`untrusted_app`, seccomp 2) |
|---|---|---|
| `coretest` | 4697 von 4697 | 4697 von 4697 |
| darin `plat`, `datastore`, `cas` | 58, 95, 109 | 58, 95, 109 |
| `plattest --test all` | 219 bestanden, 0 Fehler (Fund 2 behoben) | dasselbe |
| `plattest --test kernel` (`fs::Dir`) | 65 von 65 | 65 von 65 |
| `plattest --test vfs` | 132 von 132 | 132 von 132 |
| `plattest --test content` | 97 von 97 | 97 von 97 |
| `plattest --test trash` | 4 von 4 | 4 von 4 |
| `asynctest --script tasks.uis`, `copy.uis` | grün | grün |

**Auf dem A56** (SM-A566B, arm64, Android 16, API 36; auf Zuruf des Nutzers am 2026-11-28, je eine Runde, danach
das Test-APK deinstalliert und `/data/local/tmp/jfgui_plattest` gelöscht; die Apps des Nutzers blieben unberührt).
`coretest` (darin `plat`, `datastore`, `cas`) und `plattest --test all` liefen dort schon mit AN4 grün (siehe dort).

| Lauf (A56, arm64) | `adb shell` | App (`untrusted_app`, seccomp 2) |
|---|---|---|
| `plattest --test kernel` (`fs::Dir`, arm64-`struct stat`) | 65 von 65 | 65 von 65 |
| `plattest --test vfs` | 132 von 132 | 132 von 132 |
| `plattest --test content` | 97 von 97 | 97 von 97 |
| `plattest --test trash` | 4 von 4 | 4 von 4 |

Offen nach AN3:

- `plattest --test dirs` prüft auf Android die XDG-Ordner von Linux (25 Fehler, schon vor AN3; nicht in `all`).
  Android kennt nur `/documents` und `/data` als Ordner der Logs; der Test braucht einen eigenen Zweig.
- Unix-Sockets von `net` (mit `fs::socket_id`): nach dem Mischen von AN2 und AN3 sind beide Teile da, die Sockets
  selbst sind auf Android noch nicht eingeschaltet.

### AN4: Drucken, Schriften, Rest (M)

- Drucken: der Druckauftrag als PDF (`print.rs` schreibt es schon) über den `PrintManager` des Hosts, mit einem
  `PrintDocumentAdapter`, der das PDF liefert. `CAP_PRINT` auf Android.
- Systemschriften: `/system/fonts` und `/product/fonts` in `policy::font_dirs()`, die Reihenfolge der Ersatzschriften
  aus `/system/etc/fonts.xml`. `CAP_SYSTEM_FONTS` auf Android.
- `process` und `service` als Module mit `Unsupported` (Entscheidung 8), damit die Fassade keine Android-Ausnahme
  mehr braucht außer für `update`.

**Fertig, wenn:** eine Seite aus der `androidGallery` (oder `jfWord`, falls es Android bekommt) auf dem A56 den
Druckdialog von Android öffnet und „Als PDF speichern“ das richtige Dokument schreibt (von Hand) und `fontdb` die
Schriften des Systems findet (Roboto, Noto CJK).

**Stand 2026-11-28:** fertig. Auf dem Emulator und dem A56 grün (beide Modi), und auf beiden öffnet die
`androidGallery` den Druckdialog von Android und „Als PDF speichern“ schreibt das richtige Dokument (unten).

- **Drucken:** [`android_print.rs`](../lib/plat/android_print.rs) (neu, aus `web.rs` eingebunden) legt den Auftrag wie
  überall als PDF an (`print_pdf`, Papier und Lage des Auftrags) und gibt ihn mit der neuen Anfrage `REQUEST_PRINT`
  (35) an den Host. [`GalleryPrint.java`](../android/GalleryPrint.java) legt das PDF in den Cache (`jfgui-print/`) und
  ruft `PrintManager.print` mit einem `PrintDocumentAdapter`, der es unverändert weiterreicht; Papier und Lage des
  Auftrags sind die erste Wahl des Dialogs. Der Dialog des Systems (Drucker oder „Als PDF speichern“, Papier, Kopien,
  Seiten, Vorschau) kommt erst, wenn `print` zurück ist: der UI-Thread darf nicht warten (ANR), 0 heißt also „der Dialog
  hat den Auftrag“, nicht „gedruckt“. `PrintDoc` hat keinen Dialog vor den Seiten: `begin` fragt den Host, ob es einen
  Druckdienst gibt (`CALL_PRINT`, 70), die Seiten gehen in ein PDF (`pdf_native`, auch `add_pdf`), `end` übergibt es.
  Ein Programm ohne Java-Host (`adb shell`, das Test-APK) antwortet `ERR_UNSUPPORTED`, die Testvariante
  `jfgui_PRINT_TO` schreibt das PDF wie überall in eine Datei. `capabilities()` meldet `CAP_PRINT` und
  `CAP_SYSTEM_FONTS` (fest, wie `CAP_SHARE`).
- **Die Antwort des Dialogs:** `REQUEST_PRINT` ist eine „detached“ Anfrage. Der Host lehnt sofort ab (-3, `print`
  sieht `ERR_UNSUPPORTED`) oder meldet in `onFinish` 2 (das System hat den Auftrag), -4 (abgebrochen) oder einen
  Fehler. Wer den Dialog mit Zurück schließt, hinterlässt auf AOSP einen Auftrag im Zustand CREATED (der Spooler
  vergisst ihn, die App behält den Zustand von `print()`), „Als PDF speichern“ endet dort gleich mit COMPLETED. Auf
  dem A56 (One UI) steht der Auftrag in `onFinish` in beiden Fällen noch auf CREATED und erst einen Moment später auf
  CANCELED oder COMPLETED: ein CREATED fragt `GalleryPrint` deshalb nach 1,5 s noch einmal, was dann noch CREATED ist,
  zählt wie CANCELED als abgebrochen. Eine App liest die Antwort heute nicht (es gibt kein `print_poll` wie
  `share_poll`).
- **Systemschriften:** `policy::font_dirs()` nennt auf Android `/system/fonts` und `/product/fonts` (echte Pfade, Regel
  `SYSTEM` aus AN3); mit Manifest darf die App auch `/system/etc/fonts.xml` lesen. [`fontconf.rs`](../lib/text/fontconf.rs)
  liest diese Datei wie eine fontconfig-Datei (erkannt an `<familyset>`, auch über `jfgui_FONTCONF`): eine benannte
  Familie wird ein Alias, der die Familien ihrer Schriften vorzieht (sans-serif → Roboto, monospace → Droid Sans
  Mono), ein `<alias>` nimmt sein Ziel an (arial → sans-serif; sein Gewicht geht verloren), eine Ersatzfamilie einer
  Sprache (ja, ko, zh-Hans, zh-Hant; nicht ein Schriftsystem allein wie `und-Arab`) kommt in sans-serif nach vorn und
  ihre `fallbackFor="serif"`-Schriften in serif, `und-Zsye` sind die Emoji. Die Familiennamen liest
  `fontdb::file_family` aus den Dateien (Kopf und `name`-Tabelle, etwa 25 Dateien).
- **Ersatzreihenfolge:** [`fallback.rs`](../lib/text/fallback.rs) probiert auf Android nach den Schriften der Sprache
  alle Familien der Datei in ihrer Reihenfolge (`FontConf::sys_families`: Roboto, die Schriftsysteme, CJK, Emoji,
  Symbole; `ignore="true"` nicht), danach die eingebaute Liste. Fehlt die Datei, springen eingebaute Android-Regeln
  und eine Ersatzliste nach AOSP ein.
- **Gleichnamige Dateien:** FontDb behält von Gesichtern mit gleichem Namen, Gewicht und Stil nur eines. Android
  verlinkt `DroidSans.ttf` und `DroidSans-Bold.ttf` auf `Roboto-Regular.ttf`, im Scan steht Roboto deshalb unter
  `DroidSans-Bold.ttf`; `FontConf::sys_family` liest den Namen dann aus der Datei selbst (`FontDb::face_by_file` findet
  sonst ein Gesicht nach seinem Dateinamen).
- **`process` und `service`** sind auf Android Module (Entscheidung 8; [`process_android.rs`](../lib/plat/process_android.rs),
  ein Android-Teil in `service.rs`): jeder Start antwortet `process::ENOTSUP` (95), `run_elevated` und die
  Dienstverwaltung einen Text, `elevated()` ist false, `find()` None, ein Dienst `NotInstalled`, `run_service` ruft
  die Funktion direkt. Die Fassade brauchte dafür nichts mehr (beide sind Teile von `plat`, ihre einzige
  Android-Ausnahme bleibt `update`); der Inspektor (`ui/inspect_src.rs`) nimmt jetzt auch auf Android
  `process::launch`.
- Tests: `fonttest` hat `android` in `TARGETS` und `--fallback LANG CP..` (die Familien von `fonts.xml` in ihrer
  Reihenfolge, dann die Ersatzschrift je Zeichen); sein Selbsttest liest eine `fonts.xml` über die Testschriften (auf
  jedem Ziel). `coretest`: die Gruppe `plat` prüft `process` und `service` auf Android. Die `androidGallery` druckt aus
  ihrer Karte 06 ein Testbild (vier Farbbänder, A4, 15 mm Rand), `android_system.py` prüft den Dialog, Zurück und
  „Als PDF speichern“ (auf einem Telefon nur mit `--save-pdf`: es speichert dort und löscht danach nur die neue Datei;
  der Knopf des Dialogs nur bei „Als PDF speichern“, nie bei einem echten Drucker). Für das Telefon hat das Skript
  `--package` (ein Test-Build mit `jfgui_ANDROID_PACKAGE` neben der installierten Galerie), `--only print` (nur der
  Druck, keine Einstellung des Systems ändert sich) und `--pdf-out DATEI` (eine Kopie des gespeicherten PDFs).

| Lauf (Emulator x86-64, API 35) | `adb shell` | App (`untrusted_app`, seccomp 2) |
|---|---|---|
| `fonttest --scan` | 213 Gesichter, 185 Familien in 7 ms, darunter Roboto, Noto Sans CJK und Noto Serif CJK (JP, KR, SC, TC, HK), Noto Color Emoji | dasselbe |
| `fonttest --fallback -` | 150 Familien aus `fonts.xml`; U+4E2D Noto Sans CJK SC, U+0627 Noto Naskh Arabic, U+05D0 Noto Sans Hebrew, U+0E01 Noto Sans Thai, U+0915 Noto Sans Devanagari, U+1F600 Noto Color Emoji, U+2603 Noto Sans Symbols, A Roboto | dasselbe |
| `fonttest --fallback ja 4E2D` | Noto Sans CJK JP | dasselbe |
| `fonttest --conf` | sans-serif, Arial, system-ui → Roboto; serif → Noto Serif; monospace → Droid Sans Mono; emoji → Noto Color Emoji; für ja zuerst Noto Sans CJK JP, für zh-TW Noto Sans CJK TC | dasselbe |
| `plattest --test print`, `printdoc` | 12 von 12, 16 von 16 (ohne Host `ERR_UNSUPPORTED`, die Testvariante schreibt das PDF) | dasselbe |
| `coretest --only plat` | 106 von 106 | 106 von 106 |
| `plattest --test all` | 219 bestanden, 0 Fehler | dasselbe |
| `coretest` | 5554 von 5554 | 5551 von 5551 (AN2, Fund 1) |

`android_system.py` (Galerie-APK, Emulator): 59 von 59. Der Druck darin: „Drucken“ gibt 0, der Dialog des Spoolers
(`com.android.printspooler/.ui.PrintActivity`) ist vorn und zeigt ISO A4 und 1/1; Zurück schließt ihn, der Auftrag
endet mit -4. Danach „Als PDF speichern“ über den Speicherdialog (DocumentsUI): der Auftrag endet mit 2, und
`Download/JFLib Galerie.pdf` (4374 Bytes) enthält unser Bild (die Pixel gleich, unser Inhaltsstrom: 15 mm Rand, mittig).
Der Spooler schreibt die Datei mit PDFium neu (CRLF, `/ID`, MediaBox 595 × 841), unser `/Producer` und `/Title`
bleiben.
`GalleryInstrumentation`, `GalleryA11yInstrumentation` und `android_files.py` bleiben grün (`android_files.py` im
zweiten Lauf; der erste scheiterte an der Zeit, wie in den Fallen von AN3).

**Auf dem A56** (SM-A566B, arm64, Android 16, One UI; auf Zuruf des Nutzers, danach alles deinstalliert und gelöscht):

| Lauf | `adb shell` | App |
|---|---|---|
| `fonttest --scan` | 289 Gesichter, 236 Familien in 14 ms (im APK 10 ms), darunter Roboto, Noto Sans CJK, Samsungs SEC CJK (JP, KR, SC, TC, HK), Noto Color Emoji, SamsungColorEmoji | dasselbe |
| `fonttest --fallback -` | 159 Familien aus Samsungs `fonts.xml`; U+4E2D SEC CJK SC, U+0627 SEC Naskh Arabic, U+05D0 Noto Sans Hebrew, U+0E01 Noto Sans Thai, U+0915 SEC Devanagari, U+1F600 SamsungColorEmoji, U+2603 Noto Sans Symbols, U+AC00 One UI Sans KR, A Roboto | dasselbe |
| `fonttest --conf` | sans-serif → Roboto (für ja zuerst SEC CJK JP, dann Noto Sans CJK JP), serif → Noto Serif, monospace → Droid Sans Mono, emoji → SamsungColorEmoji | dasselbe |
| `plattest --test print`, `printdoc` | 12 von 12, 16 von 16 | dasselbe |
| `plattest --test all` | 219 bestanden, 0 Fehler | dasselbe |
| `coretest` | 5554 von 5554 (`plat` 106) | 5551 von 5551 |

`android_system.py --package org.jflib.gallery.an4 --only print --save-pdf`: 19 von 19. Der Dialog des Spoolers zeigt
ISO A4 und 1/1, Zurück → -4 (CREATED, dann CANCELED); „Als PDF speichern“ geht dort über Samsungs „Eigene Dateien“
(`com.sec.android.app.myfiles`, Knopf „OK“), nicht über DocumentsUI, und legt `JFLib Galerie.PDF` in die Wurzel des
Speichers (Endung groß). Der Auftrag endet mit 2 (CREATED, dann COMPLETED), die Datei (4374 Bytes, MediaBox
595 × 841, `/Producer(jfguiLib)`) enthält unser Bild Pixel für Pixel, wie auf dem Emulator. Die ersten Läufe fanden den
Knopf von „Eigene Dateien“ und die Datei nicht (das Skript suchte DocumentsUI, `.pdf` klein, und adb reicht die Wörter
ungequotet an die Shell weiter); das Skript kennt beides jetzt.

Geprüft außerdem: `cargo check -p jflib` mit und ohne `std` ohne Warnung für Windows (x86-64, ARM64), Linux (x86-64,
ARM64), wasm32, Android (x86-64, ARM64) und macOS (ARM64); `build_rs.py check` (Lint, Clippy für Windows und Linux);
Clippy mit denselben Regeln über `plat` und `text` für `aarch64-linux-android` (für x86-64 bricht es wie in AN2, Fund
5, vorher in `rt` ab); `gendoc --check`; `build_rs.py test coretest` auf Windows und Linux; `fonttest --selftest` auf
Windows und Linux mit den neuen Prüfungen grün (der Vergleich mit `expected.txt` scheitert wie bekannt an CRLF im
Worktree).

Offen nach AN4:

- `NotoSansSymbols-Regular-Subsetted2.ttf` (`und-Zsym`) heißt wie `…-Subsetted.ttf` und fällt im Scan als Doppel weg;
  seine 125 Symbole fehlen der Ersatzkette, bis FontDb gleichnamige Dateien verschiedenen Inhalts behält.
- Die Familien des Herstellers (`/product/etc/fonts_customization.xml`) liest `fontconf` nicht, `/product/fonts` wird
  nur durchsucht; Schriften, die das System nachlädt (`/data/fonts`), sieht die App nicht.
- Die Antwort des Druckdialogs (gedruckt, abgebrochen) erreicht die App nicht (ein `print_poll` wäre der Weg).
- `plattest --test policy` bleibt auf Android aus (Netz, Prozesse und echte Programme in einem Lauf).

### AN5: Apps, CI, Dokumentation (M)

- `TARGETS`: welche Apps zusätzlich Android bekommen, entscheidet der Nutzer (Offene Fragen; entschieden: `jfWord`).
- CI: ein Job auf einem Ubuntu-Runner mit KVM, der den Emulator (x86-64, API 35) startet und `coretest`,
  `plattest` und `asynctest` im Test-APK laufen lässt. Ohne fremde Actions, mit `sdkmanager` und `emulator`.
- Dokumentation: [API.md](API.md) (welche Module auf Android da sind), [GUIDE.md](GUIDE.md), die Tabelle der Plattformen im
  README, CHANGELOG, `lib/jflib/docs`, die Website (Plattformen), [ROADMAP.md](ROADMAP.md) §9.

**Fertig, wenn:** der CI-Job grün ist und die Dokumentation Android so beschreibt, wie es ist.

**Stand 2026-11-28:** umgesetzt und auf dem Emulator (x86-64, API 35) geprüft. Der CI-Job ist lokal unter Windows
gelaufen (dasselbe Skript wie auf GitHub), auf GitHub noch nicht: grün ist er erst nach dem ersten Lauf dort. Das A56
war für `jfWord` noch nicht dran (nur auf Zuruf).

- **`jfWord` hat Android** (Entscheidung des Nutzers vom 2026-11-28, Offene Frage 1): `android` in `TARGETS`, ein
  eigenes Manifest (`apps/jfWord/android/`, Paket `org.jfreund.jfWord`, keine Berechtigung, kein `INTERNET`: jfWord
  öffnet keine Sockets; ein Platzhalter als Symbol). Auf dem Emulator von Hand: Start, neues Dokument, Tippen über die
  Tastatur des Systems, eine Schrift des Systems aus der Liste (Dancing Script aus `/system/fonts`; die Liste zeigt die
  Familien von `fonts.xml`), Speichern über den Dateiwähler (die Datei enthält Text und Schrift), Öffnen einer DOCX- und
  einer RTF-Datei über den Wähler, Bearbeiten und Speichern an Ort und Stelle (`content://`), Drucken: der Druckdialog
  von Android zeigt die Seite (ISO A4, 1/1), „Als PDF speichern“ schreibt `Dokument.pdf` (MediaBox 595 × 841,
  `/Producer(jfguiLib)`).
- **Funde in `jfWord` (behoben, auf jedem Ziel):** Der Dateityp kam aus dem Pfad, ein `content://`-Name hat aber keine
  Endung: Speichern endete mit „Bitte als .docx speichern.“, die vom Wähler angelegte Datei blieb leer. Jetzt kommen
  Dateityp und Titel aus dem Anzeigenamen (`plat::display_name`, einmal pro Pfad). Nach dem Speicherdialog des Systems
  fragte jfWord noch einmal „Datei ersetzen“ (der Wähler legt das Dokument schon an); bei Namen des Wählers entfällt das.
  Die Liste „Zuletzt verwendet“ setzte `content://`-Namen hinter den Arbeitsordner und zeigte die ID als Namen; sie
  behält jetzt den Namen (`name=` in `recent.ini`) und sucht auch darin.
- **Drucken in `jfWord`** gab es bisher auf keinem Ziel („Drucken“ war grau). Datei / Drucken legt jetzt die Seiten so
  an, wie die Vorschau sie umbricht, ein Bild je Seite mit 200 dpi (`PageDraw::thumbnail`), und gibt sie an
  `plat::PrintDoc` (Windows' Dialog, das Portal unter Linux, unter Android der Druckdialog nach den Seiten), in die
  druckbare Fläche eingepasst; der Knopf ist da, wo `CAP_PRINT` gilt und das Dokument nicht im Erhaltungsmodus ist.
  Ein PDF mit Text (FEATURES.md OUT-005) bleibt offen.
- **Telefon:** In der Dateiansicht ist die Leiste 150 statt 200 DIP breit, die Liste „Zuletzt verwendet“ zeigt Name und
  Datum ohne die Spalten Datum und Anheften (sie überlagerten sich auf 411 DIP). Der Editor mit Menüband, Linealen und
  Statusleiste war ohne Änderung bedienbar (Menüband und Statusleiste rollen). Ein größerer Umbau für Telefone (etwa
  ein einfaches Menüband) ist eigene Arbeit.
- **`process`, `service`, `update`, Plugins, Dateizuordnungen:** jfWord nutzt keines davon; es gibt dafür keine Menüs.
  „Exportieren“ und „Optionen“ sind wie auf dem Desktop grau (noch nicht gebaut, mit Hinweis).
- **Tests:** `jfgui_ANDROID_CONSOLE=1 python build_rs.py app jfWord --target android` baut jfWord wie `asynctest` als
  Programm ohne Oberfläche mit Test-APK. [`apps/jfWord/tests/android.py`](../apps/jfWord/tests/android.py) lässt den
  Selbsttest und `files.uis`, `editor.uis` und das neue `print.uis` (Testvariante `jfgui_PRINT_TO`) per `adb shell` und
  im App-Prozess laufen; Skripte, die eine Datei eines früheren Laufs ersetzen, bekommen eine leere
  (`android_console.py --touch`). Die Suite nimmt das mit `jfgui_TEST_ANDROID` mit.
- **CI:** [`tools/android_ci.py`](../tools/android_ci.py) baut `coretest`, `plattest`, `asynctest` und `jfWord` für
  x86-64, legt ein AVD in einem eigenen Ordner an (Bild `system-images;android-35;default;x86_64`, nichts wird
  geladen), startet den Emulator ohne Fenster, wartet auf den Start und lässt die Tests laufen (`coretest`, `plattest`
  `all`, `threads`, `kernel`, `vfs`, `asynctest` `tasks.uis` und `copy.uis`, `jfWord`), danach Emulator weg, AVD
  gelöscht; `--dry-run`, `--keep`, `--only`. [`.github/workflows/android.yml`](../.github/workflows/android.yml)
  (von Hand gestartet, `ubuntu-24.04`): eine udev-Regel öffnet `/dev/kvm`, `sdkmanager` holt platform-tools,
  `platforms;android-36`, `build-tools;36.0.0`, NDK r29, den Emulator und das Bild, dann das Skript; das Log des
  Emulators als Artefakt, wenn etwas scheitert. Keine fremden Actions.
- **Lokal geprüft:** `python tools/android_ci.py` unter Windows (das SDK des Rechners, nichts geladen): Start des
  Emulators in 104 s, dann 8 von 8 grün, je per `adb shell` und im App-Prozess (`untrusted_app`, seccomp 2): `coretest`
  5557 und 5554 Prüfungen (345 s), `plattest` `all` 219, `threads` 32, `kernel` 65, `vfs` 132, `asynctest` `tasks.uis`
  und `copy.uis`, `jfWord` (Selbsttest 10669 von 10669, `files.uis`, `editor.uis`, `print.uis`; 87 s); zusammen etwa
  11 Minuten nach dem Bau. Unter WSL Ubuntu-24.04 (Linux, ohne Android-SDK): die YAML lädt, die Skripte übersetzen,
  `android_ci.py --dry-run` nennt die Schritte, ohne SDK und ohne Zugriff auf `/dev/kvm` bricht es mit klarer Meldung ab;
  der Schritt mit `sdkmanager` lief unter `bash -eo pipefail` (wie auf GitHub) mit einem nachgebauten `sdkmanager`:
  ein gescheiterter Download bricht ab und zeigt das Log. Nicht lokal prüfbar: die udev-Regel (kein sudo), der Bau
  mit dem Linux-NDK und der Emulator unter Linux.
- **Doku:** README (Plattformen, Apps, Android-Absatz, `android_ci.py`), GUIDE §12, `plat` (welche Module wo; daraus
  API.md und `lib/jflib/docs`), die README der Fassade, CHANGELOG, ROADMAP §9, `apps/jfWord/README.md` und
  `FEATURES.md`. Die Website (`jflibWebsite`) ist nicht geändert; der Text für die Plattformen wird zuerst mit dem
  Nutzer abgestimmt.

Offen nach AN5:

- Der erste Lauf von `android.yml` auf GitHub (dort erst wird der Job grün).
- `jfWord` auf dem A56 von Hand (nur auf Zuruf).
- Androids Downloads-Anbieter gibt derselben Datei je nach Weg verschiedene `content://`-Namen (angelegt:
  `document/2`, geöffnet: `document/raw:…`); „Zuletzt verwendet“ kann sie zweimal zeigen.
- jfWord sichert nichts, wenn Android den Prozess im Hintergrund beendet (wie auf dem Desktop beim Absturz); eine
  Wiederherstellung wäre eigene Arbeit.
- Drucken in jfWord als Bilder; der Druck läuft auf dem UI-Thread (ein sehr langes Dokument kann unter Android einen
  ANR auslösen).

## Bewusst nicht

| Was | Warum |
|---|---|
| Kindprozesse (`process`), Programme aus `BINS` | Android führt ab `targetSdk` 29 nichts aus beschreibbaren Ordnern aus (W^X), nur aus dem Ordner der nativen Bibliotheken des APK; das Modul ist da, jeder Start antwortet `process::ENOTSUP` (AN4) |
| Systemdienste (`service`) | Das Gegenstück wäre ein Vordergrunddienst oder WorkManager, ein anderes Modell (Offene Fragen); das Modul ist da, `install` und die übrigen antworten einen Fehlertext (AN4) |
| `update` | Android-Apps aktualisieren über ihren Store ([UPDATES.md](UPDATES.md)) |
| Native Plugins | Code nur aus dem APK, wie heute |
| Tray, Taskleiste, globale Hotkeys | gibt es auf Android nicht; `hotkey` antwortet schon `Unavailable` |
| Autostart | Activities dürfen seit Android 10 nicht aus dem Hintergrund starten |
| Dateizuordnungen und Shell-Integration im Desktop-Sinn | laufen auf Android über Intents im Manifest |
| Lokale IPC zwischen Programmen (`control_ipc`) | eine App ist auf Android ein Prozess; Einzelinstanz ist dort die Regel |
| Hardlinks (`fs::hard_link`) | SELinux verweigert sie jeder App, auch im eigenen Speicher (AN3); `false` |
| Papierkorb (`trash`) | eine App hat keinen, in den sie Dateien legen könnte; `false`, die Datei bleibt |

## Reihenfolge

```text
AN0 -> AN1 -+-> AN2 -----------+-> AN5
            +-> AN3 -> AN4 ----+
```

AN0 kann sofort beginnen. AN1 wartet auf den Merge von ARM7. AN2 und AN3 können nach AN1 nebeneinander laufen
(`net.rs` und `certs.rs` gegen `fs.rs` und `watch.rs`; beide ändern ein paar `cfg`-Zeilen in `lib.rs` und die
Fassade). AN4 braucht AN3 für die Schriften des Systems.

## Fallen

- **`adb shell` ist nicht die App.** Andere SELinux-Domäne, kein seccomp-Filter. Was dort grün ist, kann im APK
  scheitern (Entscheidung 3).
- **seccomp beendet statt abzulehnen.** Ein Syscall außerhalb der erlaubten Liste beendet die App mit SIGSYS,
  ohne `ENOSYS` zum Ausweichen. Aus den Modulen, die Android bekommen soll, trifft es `openat2` (in `fs.rs`);
  `renameat2` geht (AN0, Tabelle dort; arm64 noch offen).
- **Threads nur über `pthread_create`.** Ein eigener `clone` hat keinen TLS-Block von Bionic.
- **fdsan** (ab Android 10) beendet die App, wenn sie einen Dateideskriptor schließt, der einem anderen gehört.
  Nur eigene Deskriptoren schließen.
- **Signale gehören Bionic und `debuggerd`.** Keine eigenen `sigaction`-Syscalls an Bionic vorbei; Bionic reserviert
  Echtzeitsignale für sich.
- **Kein `/etc/resolv.conf`, keine `net.dns*`-Eigenschaften** (seit Android 8 verborgen). Namen nur über
  `getaddrinfo`.
- **Netlink ist eingeschränkt.** Apps ab `targetSdk` 30 dürfen `NETLINK_ROUTE` nicht binden und kein `RTM_GETLINK`
  senden. MAC-Adressen sind verborgen.
- **Ohne `INTERNET` kein Socket.** Die App gehört dann nicht zur Gruppe `inet`, `socket()` gibt EACCES; auf dem
  Emulator mit Android 15 und dem A56 mit Android 16 gibt es EPERM (das eBPF-Programm von `netd` prüft). `plat::net` nimmt beide (AN2).
- **Network Security Config gilt nur für Java.** Klartext-Verbote und Pins des Manifests sehen die eigenen
  Sockets nicht. `netio` muss TLS selbst richtig machen, wie überall.
- **Der UI-Thread darf nicht warten.** Fünf Sekunden ohne Antwort ergeben einen ANR. Netz nur über `task` und den
  Blocking-Pool, nie direkt im Ereignis.
- **Im Hintergrund friert Android Prozesse ein** (Cached-Apps-Freezer, Doze). Threads stehen dann, Verbindungen
  laufen in Timeouts. Der Plan dokumentiert das, löst es aber nicht.
- **Hardlinks** sind in der Shell verboten (ARM64.md) und im App-Prozess auch (AN3: SELinux, `link` auf
  `app_data_file`); `fs::hard_link` ist dort `false`.
- **Kein `/etc/localtime`.** Die Zone kommt aus `persist.sys.timezone`.
- **`fonts.xml` ist abgekündigt.** `/system/etc/fonts.xml` sagt selbst, Apps sollten sie nicht lesen; Android 15 hat
  daneben `font_fallback.xml`, die schon `adb shell` nicht lesen darf. Die eingebauten Android-Regeln von `fontconf.rs`
  und `fallback.rs` springen ein, wenn die Datei fehlt; der Weg des Systems wäre `ASystemFontIterator` (NDK, API 29).
- **Der Druckdialog verdeckt die Activity.** Der Spooler und der Speicherdialog sind eigene Activities: die App
  bekommt `onStop` / `onStart` (`UIC_SUSPEND`) wie beim Wechsel in eine andere App.
- **Emulator und Gerät unterscheiden sich:** `x86_64` gegen ARM64 (`struct stat`, Syscall-Nummern), API 35 gegen
  Android 16. Jeder Meilenstein läuft auf beiden.
- **big.LITTLE:** Das A56 hat schnelle und sparsame Kerne. Wie viele Pool-Threads sinnvoll sind, misst AN1, statt
  einfach `cpu_count()` zu nehmen. (Gemessen in AN1: alle acht Kerne lohnen sich, der Pool bleibt bei `cpu_count()`;
  Schleifen brauchen viele kleine Stücke, sonst wartet die Schleife auf einen sparsamen Kern.)

## Testen

| Was | Wo | Grenze |
|---|---|---|
| Logik ohne Oberfläche, Vorprüfung | Emulator (x86-64, API 35, Port 5560) und A56 per `adb shell` | Domäne `shell`, kein seccomp |
| Logik im App-Prozess | Test-APK auf Emulator und A56 | A56 nur auf Zuruf und unbedient |
| Netz ins Internet | A56 im WLAN, Emulator | nur mit Zustimmung, nur öffentliche Prüfserver wie in `tls_web_check.py` |
| Oberfläche | Skripte der `androidGallery`, `GalleryInstrumentation` | keine echte Eingabe, solange der Nutzer das Telefon bedient |
| Drucken | Emulator und A56 per Skript (`android_system.py`: Dialog, Zurück, „Als PDF speichern“; das A56 mit `--package --only print --save-pdf`) | „Als PDF speichern“, kein echter Drucker nötig; der Speicherdialog ist auf Samsung „Eigene Dateien“ |
| CI | Ubuntu-Runner mit KVM und Emulator: `.github/workflows/android.yml` startet `tools/android_ci.py` (lokal genauso, AN5) | kein echtes Gerät, keine Oberfläche, nur x86-64 |
| `jfWord` | `apps/jfWord/tests/android.py` (Selbsttest, Skripte; Bau mit `jfgui_ANDROID_CONSOLE=1`), das APK von Hand | der Dateiwähler und der Druckdialog nur von Hand |

Die Regel gegen Fenster-Fluten gilt auch hier: höchstens drei Runden pro Variante, keine echte Eingabe unter dem
Finger oder Zeiger des Nutzers.

## Offene Fragen

1. **Welche Apps bekommen Android?** Heute nur Test-Apps und die `androidGallery`. Vorschlag: nach AN2 `jfWord`
   oder `gallery`; JFPainter und JFNetworker entscheiden in ihren eigenen Repositories.
   **Entschieden (Nutzer, 2026-11-28):** `jfWord` bekommt Android, sonst keine weitere App dieses Repositories.
   JFPainter und JFNetworker entscheiden in ihren eigenen Repositories. (Umgesetzt in AN5.)
2. **Hintergrundarbeit:** Braucht eine App (etwa der Agent von JFBackup) Arbeit, die ohne sichtbare Activity
   weiterläuft? Das wäre ein eigener Plan mit Vordergrunddienst oder WorkManager. Vorschlag: nicht in diesem Plan.
3. **Prozesse aus dem Ordner der nativen Bibliotheken:** Braucht eine App eigene Helferprogramme auf Android?
   Vorschlag: nein, `process` bleibt `Unsupported`.
4. **Zertifikate vom Nutzer** (etwa eine Firmen-CA) über die KeyChain von Android? Vorschlag: nein (Entscheidung 6).
5. **`MIN_API` 26** (Android 8.0) beibehalten? Vorschlag: ja (Entscheidung 7).
6. **Der Headless-Weg in jedem APK?** Seit AN0 linkt `ui::launch` auf Android den Script-Läufer des Desktops mit
   (etwa 290 KB), wie auf iOS. Vorschlag: so lassen, solange niemand die Größe vermisst; sonst ein Feature von
   `ui`, das nur die Test-Builds einschalten.

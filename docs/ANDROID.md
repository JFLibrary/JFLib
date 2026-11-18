# Android-Plan: Android so weit wie Windows, Linux und Apple

Stand 2026-11-18, auf Basis von `main` 7cdabce4. AN0 und AN1 sind umgesetzt (auf dem Emulator; das A56 steht aus),
der Rest nicht; die Aufwände sind geschätzt. Der Plan wartete auf ARM1 bis ARM3 ([ARM64.md](ARM64.md)): die Kernel-Tabellen
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
  (`arc4random_buf`).
- **Die Fassade lässt Crates weg.** [`lib/jflib/Cargo.toml`](../lib/jflib/Cargo.toml) nimmt `cas`, `datastore`,
  `netio` und `update` auf Android und im Browser nicht auf („plat has no fs and net there“).
- **Das Manifest kennt kein Netz.** Das Standard-Manifest von `cargo jflib android`
  ([`android.rs`](../lib/cargo_jflib/src/android.rs), `default_manifest`) hat keine `INTERNET`-Berechtigung, und
  `tools/android.py` prüft nur `location` gegen das Manifest (`manifest_gaps`).
- **Kein Druck, keine Systemschriften.** `capabilities()` meldet auf Android weder `CAP_PRINT` noch
  `CAP_SYSTEM_FONTS` noch `CAP_THREADS`; `policy::font_dirs()` kennt `/system/fonts` nicht.
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
2. **Offen: `plat::mkdir` über einer vorhandenen Datei meldet Erfolg.** `mkdirs` in
   [`android_files.rs`](../lib/plat/android_files.rs) nimmt EEXIST auch hin, wenn der Name eine Datei ist.
3. `plat::cpu_brand()` ist leer (rt nimmt auf Android den portablen CPU-Code); die Quelle wäre `/proc/cpuinfo`.
   `plattest` meldet das als „not on Android yet“, ebenso `mem_total()` (AN1).

**Syscalls.** Alle Aufrufe von `rt` ([`os.rs`](../lib/rt/os.rs)) und den Linux-Modulen von `plat`
([`net.rs`](../lib/plat/net.rs), [`fs.rs`](../lib/plat/fs.rs), [`sync.rs`](../lib/plat/sync.rs),
[`watch.rs`](../lib/plat/watch.rs)), geprüft mit `plattest --test syscalls`: jeder Aufruf der Tabelle aus
[`rt/kernel.rs`](../lib/rt/kernel.rs) in einem eigenen Kindprozess (`fork`), mit Argumenten, die nichts tun können
(ein falscher Deskriptor, ein Null-Zeiger, abgelehnte Flags). Die Spalten nennen die Antwort des Kernels oder
**SIGSYS** (vom Filter beendet). Geprüft nur auf x86-64 (Emulator, API 35); arm64 hat andere Nummern und eine eigene
Liste des Filters, das zeigt erst der Lauf auf dem A56. AOSP-Listen wurden nicht heruntergeladen.

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

**Stand 2026-11-18:** umgesetzt und auf dem Emulator grün (beide Modi); das A56 war nicht dran (nur auf Zuruf),
also auch nicht die Messung mit `android_perf.py` und die Frage nach big.LITTLE.

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
  Pool, während jedes Bild das nächste anfordert (der Fortschrittsbalken der Karte 01 folgt ihm); danach enden fünf
  Jobs mit einem schlafenden Stück, während die Oberfläche ruht. Das Skript
  [`android_job.py`](../apps/androidGallery/tests/android_job.py) startet ihn (nach einer Installation erst ein
  Start zum Aufwärmen) und prüft: Arbeiter im Pool, höchstens `--max-over` (2) Lücken über `--max-gap-ms` (150 ms)
  während des Jobs, eine Bildrate von mindestens der Hälfte der ohne Job, kein Wecken über `--max-wake-ms` (100).
  Die Bilder fordert ein Timer von 16 ms an, wie eine Animation: ein `invalidate` in jedem Bild hält die UI nach acht
  Bildern an (`UIWARN f13`), daran scheiterte der erste Entwurf.

| Lauf (Emulator x86-64, API 35, 4 Kerne) | `adb shell` | App (`untrusted_app`, seccomp 2) |
|---|---|---|
| `plattest --test threads` | 32 von 32 | 32 von 32 |
| `plattest --test atomics` | 15 von 15 | 15 von 15 |
| `plattest --test async` | 85 von 85 | 85 von 85 |
| `plattest --test all` | 217 bestanden, 1 Fehler (Fund 2, offen) | dasselbe |
| `coretest` | 3281 von 3281 | 3281 von 3281 |
| `asynctest --script tasks.uis`, `copy.uis` | grün | grün |

`android_job.py` auf demselben Emulator (je drei Runden, die Werte von–bis; ohne Job: dieselben Bilder eine Sekunde
vor dem Job):

| Weg | Bilder/s ohne Job | längste Lücke ohne Job | Bilder/s mit Job | längste Lücke mit Job | Job (240 × 20 ms) | Wecken nach Job-Ende |
|---|---|---|---|---|---|---|
| GPU (SurfaceView, SwiftShader) | 43–53 | 61–63 ms | 53–63 | 63–82 ms | 1,42–1,46 s | 0–32 ms |
| Bitmap (`--gpu 0`) | 49–51 | 61–62 ms | 44–65 | 63–140 ms | 1,41–1,83 s | 0–32 ms |

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
  `/system/etc/security/cacerts`; dort liegt ein PEM pro Datei.
- `netio` in der Fassade auch für Android, `docs.rs` baut es für `aarch64-linux-android`.
- Manifest nach Entscheidung 5, in `cargo jflib android` und `tools/android.py`.
- Ohne `INTERNET` scheitert `socket()` mit EACCES. Das wird ein klarer `NetError` mit Hinweis auf das Manifest.
- Die Durchsetzung von `net.connect` und `net.listen` (`policy`) gilt dann auch auf Android.

**Fertig, wenn:** die Netz-, DNS-, TLS- und HTTP-Tests von `coretest` im Test-APK auf dem Emulator und dem A56 grün
sind, die Prüfungen von `tools/tls_web_check.py --internet` auf dem A56 öffentliche Server erreichen (das Skript
bekommt dafür einen Android-Weg; nur mit Zustimmung, im WLAN) und ein Lauf ohne `INTERNET` die richtige Meldung zeigt.

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

### AN5: Apps, CI, Dokumentation (M)

- `TARGETS`: welche Apps zusätzlich Android bekommen, entscheidet der Nutzer (Offene Fragen).
- CI: ein Job auf einem Ubuntu-Runner mit KVM, der den Emulator (x86-64, API 35) startet und `coretest`,
  `plattest` und `asynctest` im Test-APK laufen lässt. Ohne fremde Actions, mit `sdkmanager` und `emulator`.
- Dokumentation: [API.md](API.md) (welche Module auf Android da sind), [GUIDE.md](GUIDE.md), die Tabelle der Plattformen im
  README, CHANGELOG, `lib/jflib/docs`, die Website (Plattformen), [ROADMAP.md](ROADMAP.md) §9.

**Fertig, wenn:** der CI-Job grün ist und die Dokumentation Android so beschreibt, wie es ist.

## Bewusst nicht

| Was | Warum |
|---|---|
| Kindprozesse (`process`), Programme aus `BINS` | Android führt ab `targetSdk` 29 nichts aus beschreibbaren Ordnern aus (W^X), nur aus dem Ordner der nativen Bibliotheken des APK |
| Systemdienste (`service`) | Das Gegenstück wäre ein Vordergrunddienst oder WorkManager, ein anderes Modell (Offene Fragen) |
| `update` | Android-Apps aktualisieren über ihren Store ([UPDATES.md](UPDATES.md)) |
| Native Plugins | Code nur aus dem APK, wie heute |
| Tray, Taskleiste, globale Hotkeys | gibt es auf Android nicht; `hotkey` antwortet schon `Unavailable` |
| Autostart | Activities dürfen seit Android 10 nicht aus dem Hintergrund starten |
| Dateizuordnungen und Shell-Integration im Desktop-Sinn | laufen auf Android über Intents im Manifest |
| Lokale IPC zwischen Programmen (`control_ipc`) | eine App ist auf Android ein Prozess; Einzelinstanz ist dort die Regel |

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
- **Ohne `INTERNET` kein Socket.** Die App gehört dann nicht zur Gruppe `inet`, `socket()` gibt EACCES.
- **Network Security Config gilt nur für Java.** Klartext-Verbote und Pins des Manifests sehen die eigenen
  Sockets nicht. `netio` muss TLS selbst richtig machen, wie überall.
- **Der UI-Thread darf nicht warten.** Fünf Sekunden ohne Antwort ergeben einen ANR. Netz nur über `task` und den
  Blocking-Pool, nie direkt im Ereignis.
- **Im Hintergrund friert Android Prozesse ein** (Cached-Apps-Freezer, Doze). Threads stehen dann, Verbindungen
  laufen in Timeouts. Der Plan dokumentiert das, löst es aber nicht.
- **Hardlinks** sind in der Shell verboten (ARM64.md); im App-Prozess prüfen.
- **Kein `/etc/localtime`.** Die Zone kommt aus `persist.sys.timezone`.
- **Emulator und Gerät unterscheiden sich:** `x86_64` gegen ARM64 (`struct stat`, Syscall-Nummern), API 35 gegen
  Android 16. Jeder Meilenstein läuft auf beiden.
- **big.LITTLE:** Das A56 hat schnelle und sparsame Kerne. Wie viele Pool-Threads sinnvoll sind, misst AN1, statt
  einfach `cpu_count()` zu nehmen. (Stand AN1: noch nicht gemessen, das A56 war nicht dran; der Pool nimmt bis dahin
  `cpu_count()`, `--threads N` oder `thr_init(n)` setzen weniger.)

## Testen

| Was | Wo | Grenze |
|---|---|---|
| Logik ohne Oberfläche, Vorprüfung | Emulator (x86-64, API 35, Port 5560) und A56 per `adb shell` | Domäne `shell`, kein seccomp |
| Logik im App-Prozess | Test-APK auf Emulator und A56 | A56 nur auf Zuruf und unbedient |
| Netz ins Internet | A56 im WLAN, Emulator | nur mit Zustimmung, nur öffentliche Prüfserver wie in `tls_web_check.py` |
| Oberfläche | Skripte der `androidGallery`, `GalleryInstrumentation` | keine echte Eingabe, solange der Nutzer das Telefon bedient |
| Drucken | A56 von Hand | „Als PDF speichern“, kein echter Drucker nötig |
| CI | Ubuntu-Runner mit KVM und Emulator (AN5) | kein echtes Gerät |

Die Regel gegen Fenster-Fluten gilt auch hier: höchstens drei Runden pro Variante, keine echte Eingabe unter dem
Finger oder Zeiger des Nutzers.

## Offene Fragen

1. **Welche Apps bekommen Android?** Heute nur Test-Apps und die `androidGallery`. Vorschlag: nach AN2 `jfWord`
   oder `gallery`; JFPainter und JFNetworker entscheiden in ihren eigenen Repositories.
2. **Hintergrundarbeit:** Braucht eine App (etwa der Agent von JFBackup) Arbeit, die ohne sichtbare Activity
   weiterläuft? Das wäre ein eigener Plan mit Vordergrunddienst oder WorkManager. Vorschlag: nicht in diesem Plan.
3. **Prozesse aus dem Ordner der nativen Bibliotheken:** Braucht eine App eigene Helferprogramme auf Android?
   Vorschlag: nein, `process` bleibt `Unsupported`.
4. **Zertifikate vom Nutzer** (etwa eine Firmen-CA) über die KeyChain von Android? Vorschlag: nein (Entscheidung 6).
5. **`MIN_API` 26** (Android 8.0) beibehalten? Vorschlag: ja (Entscheidung 7).
6. **Der Headless-Weg in jedem APK?** Seit AN0 linkt `ui::launch` auf Android den Script-Läufer des Desktops mit
   (etwa 290 KB), wie auf iOS. Vorschlag: so lassen, solange niemand die Größe vermisst; sonst ein Feature von
   `ui`, das nur die Test-Builds einschalten.

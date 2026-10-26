# Roadmap: Was JFLib zur vollwertigen GUI-Library fehlt

Bestandsaufnahme vom 2026-10-16, erstellt anhand von `lib/ui`, `lib/plat`, `lib/text`,
[`Todo.md`](../Todo.md) und der internen Aufgabenliste der Arbeitspakete. Wo schon etwas
existiert, steht es unter „Vorhanden“, damit klar ist, worauf aufgebaut werden kann.

**Priorität:** **P1** = ohne das ist eine GUI-Library nicht vollwertig · **P2** = wird
erwartet · **P3** = optional / später
**Status:** *fehlt* · *teilweise* (Basis vorhanden) · *in Arbeit* · *erledigt*

Bekannte Grundgrenzen (Entscheidung „kein libc“): kein GPU-Rendering, kein fontconfig. Sie
stehen unten mit in der Liste, damit die Liste vollständig ist. (Die Barrierefreiheit läuft
inzwischen ohne libc über eigene D-Bus- und COM-Anbindungen, siehe Abschnitt 11.)

---

## 1. Berechtigungen und Sicherheit (wie bei Tauri)

Vorhanden: [`plat/capabilities.rs`](../lib/plat/capabilities.rs) meldet, welche Dienste der
Host bietet (13 `CAP_*`-Bits). Seit Paket 1 und 2 (2026-10-16) gibt es das Manifest
`apps/NAME/PERMISSIONS` ([PERMISSIONS.md](PERMISSIONS.md),
[`plat/policy.rs`](../lib/plat/policy.rs), [`ui/perm.rs`](../lib/ui/perm.rs)).

- **P1** · *erledigt* · Berechtigungs-Manifest pro App (`apps/NAME/PERMISSIONS` neben
  `TARGETS` / `STYLE`); `build_rs.py` prüft es, `ui::main!` und `rt::main!` installieren es,
  `plat` setzt es durch. Ohne Datei bleibt alles erlaubt.
- **P1** · *erledigt* · Scopes: Dateien unter Mustern mit `$CONFIG` / `$TEMP` / `$HOME` /
  `$EXE` / `$FONTS`, vom Nutzer gewählte Dateien (`fs.user`: Dialog, Drop, Kommandozeile),
  Prozesse, `open_url`-Schemes, Plugins, Drucken, Klangausgabe (`audio.output`, seit
  2026-10-22) und Netz (`net.connect` / `net.listen`:
  Namen, `*.domain`, IPs, Adressblöcke, `localhost`, Port-Bereiche; Namen werden beim
  Auflösen geprüft, ihre Adressen dann vorgemerkt).
- **P1** · *erledigt* · Zentrale Durchsetzung in `plat` (Dateien, `fs.rs`, Prozesse, Shell,
  Plugins, Drucken, Sockets und Namensauflösung inkl. `netio::dns`) statt in jeder App;
  Verweigerungen als `PERMDENY` auf stderr und in `plat::policy::denials()`,
  `capabilities()` blendet verweigerte Dienste aus. Seit 2026-10-18 in jedem Programm der App
  ([PERMISSIONS.md](PERMISSIONS.md) „Every program of the app“): `rt::main!` übergibt Name,
  Manifest und Assets über `rt::boot` an `plat::boot` (der Prozess-Einstieg von `plat`
  registriert sich, `rt` bleibt ohne `plat`), Konsolenprogramme behalten den Schlüssel nicht;
  Programme aus `BINS` (`cli` und `gui`) bekommen Manifest, App-Namen und `app://`, auf Wunsch
  ein eigenes `apps/NAME/PERMISSIONS.<bin>`; die Android-APK baut Manifest und Assets ein
  (`tools/android.py`). Getestet mit `apps/permcli` (Konsolen-Hauptprogramm, BINS mit eigenem
  und geteiltem Manifest, `gui`-Programm; je ein verbotener Systempfad) unter Windows und
  Linux. Unter Android gelten die Regeln für die virtuellen Dateien des Host-Adapters
  (`/config`, `/data`, `/tmp`, `/documents` …; `$EXE` ohne Wert, keine Prozesse/Sockets in
  `plat`); seit 2026-10-20 liegen sie als echte Dateien im internen Speicher, und die vom
  Nutzer gewählten Dokumente (`content://`, §9) brauchen bei einem Manifest `fs.user`. Auf
  einem Gerät läuft der Test nicht in der Suite.
- **P2** · *erledigt* (2026-10-19) · Laufzeit-Rückfragen („App möchte auf … zugreifen“) mit
  gespeicherter Entscheidung und Widerruf in den Einstellungen ([PERMISSIONS.md](PERMISSIONS.md)
  „Asking the user“): `ask KEY ARGS "Grund"` im Manifest (jeder Schlüssel außer `fs.user`), eine
  Entscheidung pro Zeile, eine schlichte Regel gewinnt. Die Prüfung bleibt in `plat` (jeder
  Thread): der UI-Thread zeigt den Bibliotheksdialog (was, Pfad/Host/Programm, Plugin, Grund,
  Regel; „Einmal erlauben“ / „Immer erlauben“ / „Ablehnen“ / „Nie erlauben“), sein eigener
  Zugriff wird solange abgelehnt und die App bekommt danach `UIC_PERMISSION`; andere Threads
  warten (60 s, `jfgui_PERM_TIMEOUT`) und werden dann abgelehnt; headless und
  Konsolenprogramme fragen nie (Skripte: `perm answer …`, Env `jfgui_PERM_ANSWER`). „Immer“ /
  „Nie“ liegen in `config://permission-decisions.txt` (die App selbst darf die Datei nicht
  schreiben), `ui.permissions_page(id)` listet alle Antworten mit Zurücksetzen. Getestet mit
  `apps/permprompt` (fünf `.uis`: Dialog und jede Antwort, Hintergrund-Thread mit Timeout,
  headless, gespeicherte Antworten, Agent) und `plattest --test permask` unter Windows und
  Linux. Es fehlen: Fragen pro Datei/Host statt pro Zeile, ein blockierender Dialog für den
  UI-Thread (die App muss auf `UIC_PERMISSION` erneut zugreifen), Schutz der Antwortdatei
  gegen nativen Code (nur `plat` verweigert sie), Browser/Android nur gebaut, nicht im Gerät
  getestet.
- **P2** · *erledigt* (2026-10-21) · Sicherer Speicher für Geheimnisse (seit 2026-10-17, [SECRETS.md](SECRETS.md)):
  `plat::secret` (`secret_set` / `secret_get` / `secret_delete` / `secret_available`), Namen mit
  dem App-Präfix (`vfs::app_id()`), Manifest-Recht `secrets`. Windows: Credential Manager
  (generische Einträge, `CRED_PERSIST_LOCAL_MACHINE`), Werte über 2560 Bytes per DPAPI als
  Datei unter `data://secrets/`, im Eintrag per SHA-256 gebunden. Linux: Secret Service über
  den eigenen D-Bus-Client, Transport verschlüsselt (`dh-ietf1024-sha256-aes128-cbc-pkcs7`,
  dafür AES-Entschlüsselung und `pow_ct` in `lib/crypto`), Entsperr-Prompts, fehlende
  Standardsammlung; ohne Dienst `Unavailable`, kein Klartext-Fallback. Getestet mit dem echten
  Credential Manager (`plattest --test secret`) und einem Fake-Secret-Service
  (`tools/platshared/secret_bus.py`). Android (2026-10-21): ein AES-256-GCM-Schlüssel pro App
  im Android Keystore (Alias `jfgui-secrets`, beim ersten `secret_set` erzeugt, nicht
  exportierbar, ohne Nutzerauthentifizierung, auch bei gesperrtem Gerät nutzbar), die Werte
  damit verschlüsselt (Name als zusätzliche Daten) in `noBackupFilesDir/jfgui-secrets/<SHA-256
  des Namens>.bin`, geschrieben über `AtomicFile` (`android/GallerySecrets.java`, synchron über
  `GalleryHost.call`). Browser (2026-10-21): Speicher des Web-Hosts (`web/secrets.js`), ein
  nicht extrahierbarer WebCrypto-AES-GCM-Schlüssel pro App und die verschlüsselten Werte in
  IndexedDB (`jfgui-secrets-v1`); der Host entschlüsselt die Einträge vor dem Start, `set` und
  `delete` ändern die Kopie sofort und werden der Reihe nach asynchron festgeschrieben
  (scheitert das, meldet der Eintrag `Failed`); ohne sicheren Kontext `Unavailable`. Getestet
  auf einem x86_64-Emulator mit Android 15 / API 35 (`apps/androidGallery/tests/android_secrets.py`:
  Selbsttest 33 + 9 Prüfungen, Schlüssel und Dateien von außen, eine beschädigte Datei,
  Neustart, Shell-Nutzer ohne Zugriff) und in Chromium und Firefox
  (`apps/androidGallery/tests/secrets_web.mjs`, auch die Threads-Variante). Es fehlt (bewusst):
  Trennung der Programme eines Nutzers durch das OS auf dem Desktop, Schutz vor Skripten
  desselben Origins im Browser, an Nutzerauthentifizierung gebundene Schlüssel. Nicht geprüft:
  ein echtes Android-Gerät mit TEE oder StrongBox.
- **P2** · *erledigt* (2026-10-19) · Rechte für native Plugins und Module und Rechte pro
  Aktion für die Agent-/MCP-Steuerung ([PERMISSIONS.md](PERMISSIONS.md) „Plugins“, „Agent
  control“). Plugins: `<name>.permissions` neben der Bibliothek (Manifest-Syntax ohne `ask` /
  Agent-Zeilen, `$PLUGIN`), `plat::policy::plugin_rights` / `on_behalf`: Aufrufe im Auftrag des
  Plugins brauchen seine Rechte **und** die der App (Dateien, Netz, Namen, Prozesse, URLs,
  Plugins, Drucken, Geheimnisse, Autostart), Pool-Jobs und `parallel_for` behalten den
  Scope, verschachtelte Scopes schneiden sich; App mit Manifest + Plugin ohne = nichts.
  PaintCraft lädt die Rechte vor der Bibliothek und ruft Plugin-Code darin auf. Module
  (`modules/`) haben in ihrer Host-Tabelle gar keine Datei-/Netz-/Prozessfunktionen. Agent:
  `agent_core::Access` (`ActionSpec::confirm()` / `local_only()`) und `agent.confirm` /
  `agent.deny` im Manifest; Bestätigung pro Aktion über denselben Dialog, Fehler `forbidden`
  / `confirmation_required` an den Agenten; `ui.command` einer Domänenaktion hat deren Recht,
  `ui.*`-Eingaben sind gesperrt, solange eine Rückfrage offen ist. Getestet mit
  `plattest --test plugperm` und `apps/permprompt/tests/agent.uis`. Es fehlt (bewusst): ein
  echter Sandkasten für In-Process-Plugins (Plugin-Code kann `plat` umgehen, siehe
  PERMISSIONS.md; dafür ein eigener Prozess oder die OS-Sandbox, P3); `ui.click` auf einen
  Knopf einer gesperrten Aktion läuft als Nutzereingabe (Apps sperren dafür `ui.*`).
- **P3** · *erledigt* (2026-10-22) · OS-Sandbox nutzen ([PERMISSIONS.md](PERMISSIONS.md)
  „OS sandbox", [`plat/sandbox.rs`](../lib/plat/sandbox.rs)): opt-in pro App über eine Zeile
  `sandbox` (bestmöglich) oder `sandbox strict` (startet nicht ohne OS-Durchsetzung) im
  Manifest; `plat::boot` wendet sie nach dem Installieren des Manifests an, bevor App-Code oder
  Threads laufen, Apps ohne die Zeile bleiben unverändert. **Linux** (`linux_sandbox.rs`, rohe
  Syscalls, ABI-Erkennung): Landlock beschränkt das Dateisystem auf die Ordner des Manifests
  plus die Systempfade, die die Bibliothek liest (`/usr`, `/etc`, Schriften, `/proc` … nur
  lesen; `/tmp`, `$XDG_RUNTIME_DIR`, `/dev` schreibbar), und TCP-bind/connect auf die Ports von
  `net.listen`/`net.connect` (Landlock-ABI 4+); seccomp-bpf verbietet eine Liste gefährlicher
  Syscalls (ptrace, process_vm_\*, kexec, Modul-Laden, mount, bpf, perf_event_open,
  userfaultfd, keyctl …) mit EPERM und `execve`, wenn das Manifest kein Programm erlaubt (auch
  `spawn_self`). Die Anzeige-Sockets (X11/Wayland, auch D-Bus, PulseAudio) sind Unix-Sockets:
  Verbinden wird von Landlock nicht gefiltert. Vom Nutzer zur Laufzeit gewählte Dateien
  (`fs.user`) kann Landlock nicht nachträglich freigeben – in einer Flatpak laufen sie über das
  Dokumente-Portal, nativ bleibt der Dialogpfad außerhalb der Freigaben (dokumentiert).
  **Windows** (`win_sandbox.rs`): Prozess-Mitigation-Policies (`SetProcessMitigationPolicy`):
  Image-Load (keine Remote-/Low-Label-Images), Extension-Points aus,   dynamischer Code verboten
    (außer das Manifest erlaubt Plugins; `VirtualAlloc(PAGE_EXECUTE_READWRITE)` scheitert dann); `strict`
  sperrt zusätzlich die Erzeugung von Kindprozessen, wenn weder `process` noch `url`/`print`
  im Manifest stehen (geprüft: `CreateProcess` scheitert). AppContainer per Re-exec ist bewusst nicht
  umgesetzt (bräuchte ACL-Änderungen an den echten Nutzerordnern und Tests pro App; die
  MSIX-Capabilities in `tools/pkg_msix.py` folgen schon dem Manifest, docs). **Flatpak**: die
  finish-args aus `tools/pkg_linux.py` sind bereits so eng wie das Manifest (Portale statt
  Löcher), der In-Process-Sandkasten greift darin zusätzlich. Getestet mit `apps/sandboxtest`
  (`sandbox strict`, versucht per rohen Syscalls Verbotenes und Erlaubtes) unter Windows und
  Linux (WSL, Landlock-ABI 7) und `plattest --test permask`.
- **P3** · *fehlt* · OS-Berechtigungen für Kamera, Mikrofon, Standort (Android-Laufzeitrechte,
  Portale).

## 2. Virtuelles Dateisystem und Ressourcen

Vorhanden: [`plat/vfs.rs`](../lib/plat/vfs.rs) ([VFS.md](VFS.md)) seit 2026-10-16,
[`plat/web_files.rs`](../lib/plat/web_files.rs) (virtuelle Dateien im Browser),
[`plat/fs.rs`](../lib/plat/fs.rs), [`plat/watch.rs`](../lib/plat/watch.rs),
[`plat/dirs.rs`](../lib/plat/dirs.rs) (Standardordner), [`plat/trash.rs`](../lib/plat/trash.rs)
(Papierkorb), [`lib/archive`](../lib/archive/lib.rs).

- **P1** · *erledigt* (2026-10-22) · Einheitliche VFS-Schicht für nativ, Web und Android: Alle
  Datei-Funktionen von `plat` verstehen `schema://pfad`. Fertig sind:
  - `app://` mit den eingebetteten Assets
  - `config://`, `data://`, `cache://` als App-Ordner, durch die Berechtigungen immer erlaubt
  - `mem://` im Speicher
  - eigene Mounts: `DirFs`, `Overlay` für Themes und Mods, `Sub`, `Bundle`, ZIP über
    `archive::ZipFs`
  - seit 2026-10-18 ([VFS.md](VFS.md)): `File`-Streams (`open`, `create_new`, `open_temp`) und
    `FileLock` auf jedem Mount – `MemFs` mit geteilten Bytes (Schreiben sofort sichtbar),
    `Bundle` (gespeicherte Dateien ohne Kopie, LZ4 einmal entpackt), `ZipFs` (nur lesen),
    `Overlay`/`Sub` über ihre Schichten, eigene Provider über `Provider::open` (`Opened`) oder
    ganz gelesen und beim `sync`/Drop zurückgeschrieben; `FileLock` ohne OS-Datei gilt im
    Prozess
  - `plat::fs` auf Mounts: Ordner-Mounts als OS-Pfad (geprüft), sonst `stat` (Art, Größe,
    Zeiten), `read_dir`, `set_times` (`MemFs`; ZIP: DOS-Zeit als UTC; Bundles ohne Zeit),
    keine Links
  - Whiteouts im Overlay: `.wh.NAME` in der oberen Schicht verbirgt Dateien und Ordner der
    unteren, ein neu angelegter Ordner wird mit `.wh..wh..opq` undurchsichtig; `rename` kopiert
    nach oben; ein `DirFs` oben behält die Marker auf der Platte
  - seit 2026-10-20 ([VFS.md](VFS.md) „Android“): Android SAF als `content://`
    (`plat::content::ContentFs` über den `ContentResolver`, Host-Aufruf über JNI): einzelne
    Dokumente und gewählte Ordner, darunter Namen nach Anzeigenamen; `read_file`,
    `write_file`, `File`-Streams auf dem Dateideskriptor, `AtomicFile` (an Ort und Stelle),
    `mkdir`, `delete`, `rename`, `dir_list`, `vfs::stat`, `plat::display_name`; dauerhafte
    Freigaben. Getestet auf dem Desktop gegen einen SAF-Anbieter im Speicher (`plattest --test
    content`) und auf dem Emulator gegen Androids Speicher-Anbieter (§9)
  - seit 2026-10-22 ([VFS.md](VFS.md) „Android“, „The browser“, „Flatpak“): Datei-Handles aus
    Portalen und Dokumenten
    - Browser (Chromium, File System Access API, `web/handles.js`): `ui.filedlg` /
      `plat::file_dialog` mit `FD_OPEN` / `FD_SAVE` und `pick_file` zeigen die Picker des
      Systems; die gewählte Datei ist `/handles/ID/NAME`, an ihr Handle gebunden, und jede
      Änderung (`write_file`, `AtomicFile`, `rename`, `File`-`sync`/Drop) wird per
      `createWritable` zurückgeschrieben (Swap-Datei des Browsers: ein Fehler lässt die Datei
      unverändert; `persist` liefert die Anfrage, `Durable`). Handles bleiben in IndexedDB
      (`jfgui-handles-v1`); nach dem Neuladen liest der Host Dateien mit weiter gewährter
      Berechtigung vor dem Start ein, sonst `plat::web::reopen(path)` aus der Geste des Nutzers
      (fragt nach der Berechtigung). Firefox/Safari: wie bisher eigener Dialog, `/imports`,
      Download
    - Android `content://`: Schreiben über ein Journal (`data://.content-journal`, erst die
      Kopie, dann „rwt“ an Ort und Stelle, Zurücklesen und Vergleichen, Kopie weg;
      unterbrochene Schreibvorgänge vollendet `ContentFs::recover` beim nächsten Start), die
      Dokument-ID bleibt; Namens-Cache in gewählten Bäumen (Dokument-URI je Pfad, jedes
      Dokument vor Gebrauch einmal gefragt, vergessen bei create/delete/rename, geleert bei
      gemeldeten Änderungen über einen `ContentObserver`, höchstens 60 s)
    - Flatpak: Dokumente des Dokumenten-Portals (`$XDG_RUNTIME_DIR/doc/ID/NAME`) sind im
      Sandkasten mit `fs.user` ohne neue Freigabe erlaubt; der Dateidialog des Portals liefert
      dauerhafte Dokumente (geprüft mit xdg-desktop-portal 1.18), sie öffnen nach einem Neustart
      aus `ui.recent_add` ohne Dialog. `plat::persistent_name` fragt `Documents.Add` (persistent,
      eigener D-Bus-Client mit Deskriptor-Übergabe) nach der dauerhaften ID
    - getestet: `files_web.mjs` (Chromium mit OPFS-Handles statt der echten Picker, Firefox),
      `android_files.py` auf dem Emulator (überschreiben, Cache, gemeldete Änderung, nach sechs
      Bytes unterbrochener Schreibvorgang), `plattest --test content`, `docs_bus.py` (falsches
      Portal; `--real` xdg-document-portal und der Dateidialog von xdg-desktop-portal für eine
      App in einer `bwrap`-Sandbox)

  Grenzen: Dokumente aus „Öffnen mit“ und Datei-Transfer gelten im Flatpak nur für die Sitzung
  (das Portal liefert für Deskriptoren auf seinem Mount die eigene ID, eine App kann sie nicht
  dauerhaft machen); im Browser nur Chromium, keine Ordner-Handles, keine Handles für
  gezogene Dateien; `content://` zeigt anderen Apps während des Schreibens einen Teil
- **P1** · *erledigt* · Assets beim Build bündeln: `apps/NAME/assets/` wird von
  `tools/assetpack.py` als JFA1 gepackt (LZ4 wie `lib/compress`, „stored“, wenn LZ4 nicht
  mindestens ein Zehntel spart) und per `include_bytes!` eingebettet. Der Inhalts-Hash steuert
  den Rebuild, `ui::main!` und `rt::main!` mounten das Bundle als `app://`, seit 2026-10-18
  auch in Programmen aus `BINS` und in Android-Builds.
- **P1** · *erledigt* · Pfade normalisieren und `..`-Ausbrüche verhindern
  (`plat::policy::normalize`: Windows-Präfixe, UNC, Laufwerke, Geräte- und Stream-Namen;
  unter Linux wird `..` abgelehnt, weil der Kernel es erst nach Symlinks anwendet). Die
  Prüfung ist lexikalisch: Symlinks in erlaubten Ordnern können noch hinausführen.
- **P2** · *erledigt* · Alle Standardordner ([`plat/dirs.rs`](../lib/plat/dirs.rs), seit
  2026-10-17): neben `config_dir`, `data_dir`, `cache_dir`, `temp_dir`, `home` jetzt
  `log_dir` (Linux `$XDG_STATE_HOME`, Windows `%LOCALAPPDATA%`) und
  `user_dir(DIR_DESKTOP | DIR_DOCUMENTS | DIR_DOWNLOADS | DIR_PICTURES | DIR_MUSIC |
  DIR_VIDEOS)`: Windows über `SHGetKnownFolderPath` (auch verschobene Ordner, OneDrive; der
  Installer nutzt dieselbe Funktion), Linux aus `~/.config/user-dirs.dirs` (`$HOME`-Ersetzung,
  Escapes, sonst `$HOME/Documents` …), Browser/Android nur `/documents`. Als VFS-Schemes
  `logs://` (immer erlaubt, `vfs::app_dir(APP_LOG)`) und `desktop://`, `documents://`,
  `downloads://`, `pictures://`, `music://`, `videos://` (nur mit Regeln); im Manifest
  `$APPLOG`, `$DESKTOP`, `$DOCUMENTS`, `$DOWNLOADS`, `$PICTURES`, `$MUSIC`, `$VIDEOS`. Das
  Logging schreibt jetzt nach `logs://APP.log` statt `data://logs/APP.log`.
- **P2** · *erledigt* · In den Papierkorb verschieben statt löschen: `plat::trash(path)`
  ([`plat/trash.rs`](../lib/plat/trash.rs), seit 2026-10-17). Windows `IFileOperation` mit
  `FOFX_RECYCLEONDELETE` auf eigenem STA-Thread; ein Progress-Sink lehnt ab, wo die Shell
  endgültig löschen würde (Laufwerk ohne Papierkorb, Netzfreigabe) – dann bleibt die Datei
  und `trash` liefert `false`. Linux nach der freedesktop-Spezifikation
  (`$XDG_DATA_HOME/Trash`, auf anderen Dateisystemen `.Trash/$uid` bzw. `.Trash-$uid` im
  Mountpunkt, `.trashinfo` exklusiv zuerst, Namenskollisionen als `a.2.txt`). Zählt als
  Schreiben auf den Pfad (`fs.write`). Es fehlen: Ortszeit im `DeletionDate` (heute UTC),
  `directorysizes`, Papierkorb im Browser und unter Android.
- **P2** · *erledigt* · Asynchrone bzw. streamende Datei-API mit Fortschritt und Abbruch
  ([`plat/aio.rs`](../lib/plat/aio.rs), seit 2026-10-18): `aio::read_file`, `write_file`,
  `copy_file` laufen stückweise (Vorgabe 256 KiB) auf dem Blocking-Pool von `plat::task`, mit
  `Ctl` (Fortschritt in Bytes, `CancelToken`, Stückgröße); `Reader` / `Writer` lesen bzw.
  schreiben Stück für Stück, jedes Stück ein Pool-Job. Geschrieben wird atomar wie
  `write_file` über das neue `plat::AtomicFile` (Geschwisterdatei, Commit per Rename, Linux
  behält die Zugriffsrechte; auf Mounts eine `NAME.tmp-..`-Datei des Providers): Abbruch,
  Fehler oder Drop entfernen die Geschwisterdatei, das Ziel bleibt unverändert. VFS-Routing und
  Berechtigungen gelten unverändert (dieselben Aufrufe auf dem Pool-Thread, ein
  `Key::trusted`-Aufrufer bleibt dort vertrauenswürdig). `netio::Client::download` schreibt
  HTTP-Antworten genauso. Getestet mit `plattest --test async` (u. a. Abbruch zwischen zwei
  Stücken über einen „gated“ Mount, Rechte, Policy im Kindprozess) und `apps/asynctest`. Es
  fehlen: echte asynchrone OS-E/A (IOCP / io_uring; heute belegt jede laufende Operation einen
  Pool-Thread), ein asynchrones `dir_list` / `stat`; unter Android und im Browser ohne Shared
  Memory laufen die Jobs ohne Threads zwischen den Frames.
- **P3** · *teilweise* · „Zuletzt verwendet“ auf OS-Ebene ([`plat/recent.rs`](../lib/plat/recent.rs),
  seit 2026-10-19, GUIDE §21): `plat::recent_add(path)`, `recent_list(max)`, `recent_remove`,
  `recent_clear`; in der UI `ui.recent_add(path)` mit der eigenen Liste der App
  (`ui.recent_files()`, `config://recent.list`, höchstens `RECENT_KEEP`). Windows:
  `SHAddToRecentDocs` (Explorer: Start/Schnellzugriff, Recent-Ordner), Rücklesen über
  `IApplicationDocumentLists`, Entfernen über `IApplicationDestinations`; `ui.taskbar_tasks`
  behält die Kategorie „Zuletzt verwendet“ (`AppendKnownCategory`). Linux:
  `~/.local/share/recently-used.xbel` im freedesktop-Format (GTK/KDE: MIME-Typ, Eintrag der App
  mit `exec` und `count`), unter Sperre geändert und atomar ersetzt, fremde Lesezeichen
  bleiben Byte für Byte. Nur Dateien, die die App lesen darf (`fs.read` / `fs.user`);
  Headless-Läufe schreiben nichts ins System; `jfgui_RECENT=DATEI` lenkt auf eine xbel-Datei
  um (alle OS, Tests). Getestet mit `plattest --test recent` (xbel, GLib-Datei, Manifest) und
  von Hand `--test recent-win`. Es fehlen: Windows ordnete die Datei auf dem Testrechner
  (Windows 11 26200) nicht der Jump List von plattest zu (auch mit Fenster, expliziter
  AppUserModelID und `AppUserModelID` im ProgID) – `recent_list` bleibt dort leer, die eigene
  Liste der App ist die verlässliche; Explorer behält entfernte Einträge (der Nutzer entfernt
  sie dort); keine explizite AppUserModelID für JFLib-Apps; Flatpak sieht die Liste des Hosts
  nur mit `--filesystem=xdg-data/recently-used.xbel`; Web und Android.

## 3. Desktop-Integration

- **P1** · *erledigt* · Nur eine Instanz (`AF_SINGLE_INSTANCE`, `plat::instance`): Spätere
  Starts geben Ordner und Kommandozeile an die laufende Instanz (Windows: Named Pipe pro
  Nutzer-SID; Linux: abstrakter Socket pro UID; beide Seiten prüfen den Nutzer). Das Fenster
  kommt nach vorn, die App bekommt `UIC_OPEN`. PaintCraft öffnet so Dateien als Tabs.
- **P1** · *erledigt* · Dateizuordnungen und URL-Schemes: `PACKAGE` `filetypes=` / `schemes=`.
  Das Windows-Setup registriert ProgIDs, „Öffnen mit“ und Schemes unter HKCU (das Deinstallieren
  entfernt sie wieder), das .deb trägt `MimeType=` und `%U` / `%F` ein, eigene Typen bekommen
  ein shared-mime-info-Paket; MSIX (`windows.fileTypeAssociation`, `windows.protocol`, jede
  Endung nur einmal) und Flatpak (exportierte .desktop-Datei und MIME-Paket) deklarieren sie
  selbst. Seit 2026-10-19 ohne Setup ([`plat/assoc.rs`](../lib/plat/assoc.rs), GUIDE §21):
  jede App mit `ui::main!` versteht `--register`, `--register-replace` und `--unregister`
  (API `plat::assoc::assoc_register` / `assoc_conflicts` / `assoc_unregister`; `build_rs.py`
  baut PACKAGE als `JFGUI_PACKAGE` ein). Windows schreibt die Schlüssel des Setups unter
  `HKCU\Software\Classes` (gemeinsame Definition `reg_entries`, auch für das Setup) und meldet
  `SHChangeNotify`; Linux `~/.local/share/applications/<app_id>.desktop`, eigene Typen als
  `~/.local/share/mime/packages/<app_id>.xml` und Standards direkt in `mimeapps.list` (ohne
  `xdg-mime`; `update-mime-database` / `update-desktop-database` nur, wenn vorhanden).
  Standards anderer Programme (Standard-ProgID einer Endung, gleichnamiger Schlüssel mit
  anderem Befehl, `mimeapps.list`-Eintrag, andere .desktop-Datei mit dem Typ) werden nur nach
  Rückfrage übernommen; `UserChoice` bleibt immer. Ein Protokoll (`config://assoc.list`)
  erlaubt exaktes Entfernen samt Wiederherstellen des Ersetzten. Linux-Dateimanager und
  Flatpaks Dokument-Portal übergeben `file://`-URIs: `Request::paths()`, der `fs.user`-Grant
  und `plat::assoc::file_uri_path` machen Pfade daraus. Getestet mit `plattest --test assoc`
  (Windows: Testnamen im echten HKCU, wieder entfernt; Linux: XDG-Ordner im Scratch,
  `desktop-file-validate`, `update-mime-database`) und `tools/pkg_check.py` (MakeAppx,
  desktop-file-validate, appstreamcli). Es fehlen: macOS; eine Abfrage-UI in der App (die API
  liefert die Konflikte, die App fragt selbst); in MSIX und Flatpak ist `--register`
  `Unavailable`; ein Symbol für die selbst registrierte .desktop-Datei (Entwickler-Builds haben
  kein installiertes Icon).
- **P1** · *erledigt* · OS-Benachrichtigungen (`plat::tray::notify`, `ui.notify`; Klick →
  `UIC_NOTE`): Windows als Balloon des Infobereich-Symbols, den Windows 10/11 als Toast zeigt;
  Linux über `org.freedesktop.Notifications` mit Bild und Standard-Aktion. Es fehlen noch:
  echte WinRT-Toasts mit Buttons, Web und Android.
- **P1** · *erledigt* · Symbol im Infobereich (`ui.tray_set` mit Menü aus der Befehlstabelle,
  `UIC_TRAY`): Windows `Shell_NotifyIconW` mit nativem Menü; Linux StatusNotifierItem mit
  dbusmenu auf einer eigenen D-Bus-Verbindung (getestet gegen einen Fake-Bus). GNOME zeigt
  StatusNotifierItems nur mit AppIndicator-Erweiterung.
- **P2** · *erledigt* · Systemeinstellungen live übernehmen
  ([`plat/settings.rs`](../lib/plat/settings.rs)): Hell / Dunkel, Akzentfarbe, hoher Kontrast
  mit Farben, „Animationen reduzieren“, Textskalierung, Doppelklickzeit, Blinkrate des Cursors –
  beim Start und live über `UIC_SETTINGS`. Windows aus Registry und Systemaufrufen
  (`SPI_GETHIGHCONTRAST`, `GetSysColor`, `SPI_GETCLIENTAREAANIMATION`, `TextScaleFactor`,
  `GetDoubleClickTime`, `GetCaretBlinkTime`), Linux aus dem Settings-Portal
  (`org.freedesktop.appearance`, GNOME-Schlüssel; getestet gegen ein Fake-Portal); die
  Systemsprache seit §5. `jfgui_*`-Variablen überschreiben, headless gelten nur sie. Es fehlen
  noch: KDE ohne Portal (kdeglobals), Web (`prefers-*`) und Android.
- **P2** · *erledigt* · Drag & Drop ([`ui/dnd.rs`](../lib/ui/dnd.rs),
  [`plat/dnd.rs`](../lib/plat/dnd.rs)): Widgets sind Quellen (`drag_source` / `drag_set` mit
  `DragData`: Dateien, Text, URLs, HTML, PNG, eigene Art + Bytes) und Ziele (`drop_target` mit
  `DT_*`-Arten): Hervorhebung, Etikett am Zeiger, Esc bricht ab, RTL-verträglich. Verlässt der
  Zeiger das Fenster, übernimmt die Plattform (Windows `DoDragDrop` mit eigenem `IDataObject` /
  `IDropSource`, X11 Xdnd als Quelle, Wayland `wl_data_device.start_drag`); zurück in der App
  kommt die eigene Art wieder an. Herein kommen Dateien, Text, URLs, HTML und Bilder (Windows
  `IDropTarget` mit `DROPEFFECT`, X11 Xdnd 5 mit `XdndTypeList` und INCR, Wayland
  `wl_data_device`), mit Hover (`DS_HOVER`, `drag_over`) und synchroner Antwort an die Quelle;
  außerhalb der Ziele wie bisher `UIC_DROP`. Dateien werden nie verschoben. Getestet mit
  `apps/dndtest`, `plattest --test dnd` (OLE-Objekte ohne Benutzer) und
  `tools/platshared/dnd_x11.py` (Xdnd in beide Richtungen unter WSLg). Es fehlen noch: Web und
  Android, Drag-Bilder (`IDragSourceHelper`, Xdnd-/Wayland-Icon), virtuelle Dateien
  (`FileGroupDescriptor`, XDS), verzögerte Daten, automatische Tests unter Wayland und mit
  echter Maus (von Hand).
- **P2** · *erledigt* · Strom und Sitzung ([`plat/power.rs`](../lib/plat/power.rs), seit
  2026-10-18): `plat::inhibit(grund, INHIBIT_SUSPEND | INHIBIT_IDLE)` gibt einen Guard, der
  beim Drop wieder freigibt; `INHIBIT_IDLE` hält zusätzlich den Bildschirm an (kein
  Bildschirmschoner, keine Sperre), `plat::inhibited()` sagt, was gerade gehalten wird.
  Windows: ein Power-Request mit dem Grund als Text (`PowerCreateRequest`,
  `PowerRequestSystemRequired` / `DisplayRequired`; `powercfg /requests` zeigt ihn). Linux:
  das Inhibit-Portal (auch in Flatpak; es antwortet vor seinem Backend, eine spätere Ablehnung
  wird 300 ms abgewartet, `INHIBIT_IDLE` fällt bei Backends, die nur „idle“ können, auf
  Flag 8 zurück), ohne Portal `org.freedesktop.ScreenSaver` bzw.
  `org.freedesktop.PowerManagement.Inhibit`; alles auf einer eigenen Session-Bus-Verbindung,
  die mit dem letzten Guard schließt. logind wird nicht benutzt: sein `Inhibit` liefert einen
  Dateideskriptor, den der eigene D-Bus-Client nicht empfängt. Die Ereignisse gab es schon
  (§7): Ruhezustand / Aufwachen als `UIC_SUSPEND` (`WM_POWERBROADCAST`, logind
  `PrepareForSleep`), Abmelden / Herunterfahren als `UIC_SESSION_END` (`WM_QUERYENDSESSION`,
  SIGTERM). Getestet mit echten Power-Requests (`plattest --test power`), einem Fake-Bus
  (`tools/platshared/desktop_bus.py`) und dem echten xdg-desktop-portal (GTK-Backend) in WSL.
  Es fehlen: Web (Screen Wake Lock), Android (WakeLock / `FLAG_KEEP_SCREEN_ON`), Abmelden
  aufhalten (`ShutdownBlockReasonCreate`, Portal-Flag 1).
- **P2** · *erledigt* · Autostart ([`plat/autostart.rs`](../lib/plat/autostart.rs), seit
  2026-10-18): `plat::autostart_set(an, args)`, `autostart_get()`, `autostart_entry()`;
  Ändern braucht das Manifest-Recht `autostart`. Windows: der Wert `<app id>` unter
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` mit zitierten Argumenten; der Schalter
  des Task-Managers (`Explorer\StartupApproved\Run`) zählt für `autostart_get` und wird beim
  Einschalten zurückgesetzt. Linux: `$XDG_CONFIG_HOME/autostart/<desktop-id>.desktop` mit
  Exec-Quoting nach der Desktop-Entry-Spezifikation (`$APPIMAGE` statt der Programmdatei, wenn
  gesetzt); in Flatpak das Background-Portal (`RequestBackground` mit `autostart` und
  `commandline`, wartet auf die Antwort des Nutzers, gemerkt in `config://autostart`).
  Getestet mit dem echten Run-Schlüssel (wieder entfernt), XDG in einem Scratch-Ordner und
  einem Fake-Portal. Es fehlen: MSIX (`StartupTask`), Web und Android.
- **P2** · *erledigt* · Taskleiste ([`plat/taskbar.rs`](../lib/plat/taskbar.rs),
  [`ui/taskbar.rs`](../lib/ui/taskbar.rs), seit 2026-10-18): `ui.taskbar_progress(..)` mit
  `TaskbarProgress::Normal(f)` / `Paused(f)` / `Error(f)` / `Indeterminate` / `None`,
  `ui.taskbar_badge(n)` (ein roter Punkt mit der Zahl, über 99 „99+“), `ui.taskbar_tasks(..)`
  (Aufgaben der Sprungliste). Windows: `ITaskbarList3` (`SetProgressState`,
  `SetProgressValue`, `SetOverlayIcon`), nach „TaskbarButtonCreated“ erneut angewendet (auch
  vor dem ersten Zeigen gesetzt, nach einem Explorer-Neustart); Sprunglisten über
  `ICustomDestinationList` mit `IShellLinkW`-Aufgaben (dieses Programm mit Argumenten; mit
  `AF_SINGLE_INSTANCE` kommen sie als `UIC_OPEN` an). Linux: das Signal
  `com.canonical.Unity.LauncherEntry.Update` (Fortschritt, Zähler) für KDE Plasma, Ubuntu-Dock
  / Dash to Dock und Plank; dort kein eigenes Aussehen für pausiert / Fehler / unbestimmt und
  keine Sprungliste zur Laufzeit (die kommt aus den `Actions` der .desktop-Datei). Getestet:
  Probelauf (`plattest --test taskbar`, `apps/traydemo`), ein echtes Fenster mit allen
  Zuständen, Badge und Sprungliste (`plattest --test taskbar-win`, von Hand), die Signale
  gegen einen Fake-Bus und einen echten dbus-daemon. Seit 2026-10-19 behält die Sprungliste die
  Kategorie „Zuletzt verwendet“ (§2). Es fehlen: eigene Kategorien in der Sprungliste
  (`AppendCategory`), Vorschau-Schaltflächen
  (`ThumbBarAddButtons`), Quicklists über dbusmenu unter Linux.
- **P3** · *erledigt* · Monitore, globale Tastenkürzel, Farbe vom Bildschirm, Bildschirmfotos
  ([`plat/screen.rs`](../lib/plat/screen.rs), [`plat/hotkey.rs`](../lib/plat/hotkey.rs),
  [`ui/screen.rs`](../lib/ui/screen.rs), seit 2026-10-22; GUIDE.md, Abschnitt 21):
  `ui.monitors()` / `plat::monitors()` mit Name, Rechteck, Arbeitsbereich, DPI,
  Bildwiederholrate, primär und einer stabilen ID; Änderungen kommen als `UIC_SETTINGS` mit
  `SET_MONITORS`. Windows: `EnumDisplayMonitors`, `GetMonitorInfoW`, `GetDpiForMonitor`,
  `EnumDisplaySettingsW`, `WM_DISPLAYCHANGE` und geänderter Arbeitsbereich; X11: RandR 1.5
  (`GetMonitors`, die Wiederholrate aus dem Modus der CRTC), `_NET_WORKAREA`, RandR-Ereignisse;
  Wayland: `wl_output` mit `xdg-output` (logische Lage und Größe, DPI auch bei gebrochener
  Skalierung); Browser: `window.screen`, alle Bildschirme über `getScreenDetails`, wenn die
  Seite es schon darf (fragt nie selbst); Android: die Anzeigen des `DisplayManager`. Globale
  Tastenkürzel `ui.hotkey_register(id, "Ctrl+Alt+K", Beschreibung)` → `UIC_HOTKEY` (Recht
  `input.global`): Windows `RegisterHotKey` (`MOD_NOREPEAT`, belegte Kürzel als `Conflict`),
  X11 `XGrabKey` am Root-Fenster für jede Taste und jede Kombination aus Feststell- und
  Num-Taste (`BadAccess` als `Conflict`), Wayland und Flatpak das GlobalShortcuts-Portal
  (Sitzung, `BindShortcuts` mit Beschreibung und bevorzugtem Kürzel, `Activated`; was der
  Desktop nicht bindet, kommt als `UIC_HOTKEY_LOST`); Browser und Android `Unavailable`. Eine
  Farbe von irgendwo auf dem Bildschirm: `ui.pick_color()` → `UIC_PICK`, im Farbwähler
  `CP_PIPETTE` (der Pipetten-Knopf erscheint nur, wo es geht; kein Recht nötig, der Nutzer
  wählt das Pixel): Windows ein eingefrorenes Bild aller Monitore mit Lupe und Hex-Wert
  (Pfeiltasten pixelweise, Esc bricht ab), X11 ein Zeigergriff mit Fadenkreuz (bei rootlosem
  Xwayland das Pixel aus dem Fenster unter dem Zeiger), Wayland / Flatpak
  `Screenshot.PickColor`, Chromium der `EyeDropper`. Bildschirmfotos `ui.screenshot(rect)` →
  `UIC_SCREENSHOT`, `ui.screenshot_take()` als `Surface` (Recht `screen.capture`): Windows
  `BitBlt` mit `CAPTUREBLT`, X11 `GetImage` des Root-Fensters in Streifen, Wayland / Flatpak
  `Screenshot.Screenshot` (die PNG-Datei des Portals). Getestet: `plattest --test screen`
  (Windows: Monitore gegen `GetSystemMetrics`, ein Musterfenster aufgenommen und gepickt, das
  Overlay mit Klick und Esc, `RegisterHotKey` über `SendInput` einer freien F13–F24-Taste,
  belegtes Kürzel eines zweiten Prozesses; Linux mit dem X-Server von WSLg: RandR, `GetImage`,
  der Pick nach dem Klick-Ereignis, Esc über XTEST, `XGrabKey` mit XTEST-Tasten und den
  Lock-Kombinationen, `BadAccess` eines zweiten Prozesses), `--test screen-wl` (die
  Wayland-Ausgaben von WSLg), `--test screen-policy`, `tools/platshared/screen_bus.py`
  (Fake-Portale: PickColor, Screenshot, GlobalShortcuts mit nicht gebundenem Kürzel,
  `Activated`, neue Sitzung nach dem Abmelden, Ablehnung, kein Portal), `apps/screentest`
  (Skripte für Monitore, Kürzel und Pipette; `tests/screen_web.mjs` in Chromium und Firefox;
  `tests/android_screen.py` im Emulator: Größe, DPI, Bildwiederholrate und Name gegen `wm size`,
  `wm density` und `dumpsys display`, Änderungen über `wm size`).
  Es fehlen: ein echtes Bildschirmfoto über X11 auf einem rootlosen Server (WSLg verweigert
  `GetImage` am Root-Fenster; dort ist nur dieser Fehler geprüft), die echten Dialoge der
  Portale (nur gegen Fake-Busse getestet), unter X11 eine Lupe beim Pick und eine DPI je
  Monitor, der Arbeitsbereich unter Wayland (kein Protokoll), Bildschirmfotos im Browser
  (`getDisplayMedia` mit Auswahldialog), der Arbeitsbereich unter Android (ohne
  Systemleisten) und DXGI Desktop Duplication statt `BitBlt` (schneller bei großen
  Bildschirmen).
- **P3** · *erledigt* · Audio-Ausgabe ([`plat/audio.rs`](../lib/plat/audio.rs),
  [`ui/sound.rs`](../lib/ui/sound.rs), seit 2026-10-22; GUIDE.md, Abschnitt 33): Streams mit
  Rate, Kanälen und Latenzziel (Standard 40 ms), gefüllt über einen Ring, den die Plattform
  leert (`Stream::open` + `write` / `write_i16`), oder per Callback auf dem Audio-Thread
  (`open_fill`); Start / Pause / Stopp / Ende, Lautstärke, Latenz, gespielte Frames, Aussetzer,
  Gerätename und Gerätewechsel; `play` / `play_file` mischt kurze Klänge (WAV PCM 8–32 Bit und
  Float, Ogg Vorbis mit eigenem Decoder) in einen gemeinsamen 48-kHz-Stream; `sound(SOUND_*)`
  spielt die Systemklänge, `ui.beep(kind)` und `ui.dialog_sound(kind)` (nur wo Meldungsdialoge
  klingen) mit dem App-Schalter `ui.set_sounds`. Windows: WASAPI Shared Mode ereignisgesteuert
  auf eigenem Thread, `IMMNotificationClient` folgt dem Standardgerät, Systemklänge über
  `PlaySound`-Aliase. Linux: eigener Client des PulseAudio-Protokolls über den Unix-Socket
  (pipewire-pulse, PulseAudio, WSLg) mit Cookie, der Server verschiebt Streams zum neuen
  Standardgerät; Systemklänge aus dem freedesktop-Soundtheme (Theme und `event-sounds` über das
  Settings-Portal). Browser: AudioWorklet aus einem SharedArrayBuffer-Ring (bei
  Cross-Origin-Isolation) oder geposteten Puffern, Pumpe im Worker, `Blocked` bis zur ersten
  Nutzergeste ([`web/audio.js`](../web/audio.js)); Systemklänge als erzeugte Töne. Android:
  AAudio über den Host ([`android/audio.c`](../android/audio.c)), getrenntes Gerät öffnet neu;
  Benachrichtigungston und `ToneGenerator`. Getestet: WASAPI-Loopback hört die Töne,
  WSLg-PulseServer listet den Stream samt Latenz, Playwright (Chromium, Firefox) misst den Ton
  mit einem AnalyserNode, im Emulator spielt audioserver die AAudio-Spur
  (`apps/audiotest`, `plattest --test audio`). Seit 2026-10-22 braucht Klang bei einem
  Manifest das Recht `audio.output` (Entscheidung: nur Apps, die Klang brauchen, bekommen es;
  [PERMISSIONS.md](PERMISSIONS.md)): ohne es verweigert `plat` Streams, `play`, `play_file`
  und Systemklänge (`AudioError::Denied`, `ui.beep` liefert `false`, einmal
  `PERMDENY audio.output`), Dialogklänge bleiben ohne Rückfrage still; `ask audio.output`
  fragt beim ersten eigenen Klang. Das Flatpak-Paket bekommt `--socket=pulseaudio` (der auch
  Aufnahmen erlaubt) nur mit dem Recht (`tools/package.py`, geprüft in `tools/pkg_check.py`),
  MSIX und die übrigen Pakete brauchen nichts. Getestet: `permtest` (verweigert),
  `permprompt/tests/sound.uis` (Rückfrage, stiller Dialog), `audiotest` (mit Recht),
  `plattest --test policy` / `permask`. Es fehlen: Ausgabe ohne Soundserver unter
  Linux (ALSA direkt), Wahl des Ausgabegeräts, der Stream-Name im Windows-Lautstärkemixer,
  Floor 0 von Vorbis; später Video, Kamera, Mikrofon.
- **P3** · *fehlt* · WebView einbetten (WebView2, WebKitGTK); ohne libc schwierig, bewusst
  zu entscheiden.

## 4. Text und Schrift

Vorhanden: [`text/fontdb.rs`](../lib/text/fontdb.rs) (durchsucht die Font-Ordner selbst),
HarfRust-Shaping, Bidi beim Shaping, IME unter Wayland (text-input-v3), Windows und Android.
Seit 2026-10-17 auch Glyph-Fallback und IME unter X11.

- **P1** · *erledigt* · Glyph-Fallback ([`text/fallback.rs`](../lib/text/fallback.rs)): Fehlt
  einer Schrift ein Zeichen, nimmt `Text` die erste Schrift der Kette, die es hat – zuerst die
  der App (`set_fallback_fonts`), dann Systemschriften aus einer Liste je OS (Segoe UI Symbol /
  Emoji, YaHei, … bzw. DejaVu, Noto, WenQuanYi, …), gefunden über `FontDb` (Cache
  `cache://fontdb.fdb`). Gilt für den einfachen Textlauf und für das Shaping (HarfRust bekommt
  die Schriften als Fallback-Engines). Im Fenster an, headless aus (gleiche Shots überall);
  `jfgui_FONT_FALLBACK`, `jfgui_FALLBACK_FONTS`. Seit 2026-10-18 kommen zuerst die Schriften
  der UI-Sprache (Han-Zeichen in japanischer, vereinfachter oder traditioneller Form). Es fehlen
  noch: farbige Emoji (COLR / CBDT).
- **P1** · *erledigt* · IME unter X11 ([`plat/linux_ibus.rs`](../lib/plat/linux_ibus.rs)):
  IBus über das Portal `org.freedesktop.portal.IBus` auf einer eigenen D-Bus-Verbindung
  (fcitx5 über sein IBus-Frontend). Fokus und Cursorposition folgen dem Textfeld, Tasten
  gehen zuerst an IBus, Preedit / Commit / weitergeleitete Tasten kommen als
  `E_PREEDIT` / `E_IME_COMMIT` / `E_TEXT`. Getestet gegen einen Fake-Bus, auch mit
  synthetischen X-Ereignissen. Es fehlen noch: XIM (ältere Eingabemethoden ohne IBus-Portal),
  Surrounding Text.
- **P2** · *erledigt* · fontconfig-Ersatz light ([`text/fontconf.rs`](../lib/text/fontconf.rs)):
  generische Familien (`sans-serif`, `serif`, `monospace`, `cursive`, `fantasy`, `system-ui`,
  `emoji`, `math`), Ersatzschriften (Arial → Liberation Sans, Helvetica → Arial …) und
  Schriften je Sprache (ja, zh-Hans, zh-Hant, ko, th, Indisch, Arabisch, Hebräisch …). Unter
  Linux liest ein kleiner Leser die `<alias>`- und Sprachregeln aus `/etc/fonts` (mit
  `<include>` und `conf.d`), danach gelten eingebaute Tabellen je OS; `FontDb::resolve` nimmt
  die erste installierte Schrift (ergibt dieselben Schriften wie `fc-match` in WSL).
  `jfgui_FONTCONF` setzt eigene Regeln (Tests). Darstellung des Systems in `SysSettings`
  (`font_aa`, `font_hinting`, `font_rgba`): Windows `SPI_GETFONTSMOOTHING*`, Linux Portal
  (`font-antialiasing`, `font-hinting`, `font-rgba-order`) bzw. die conf.d-Links; „Glättung
  aus“ zeichnet schwarz-weiße Glyphen (`Text::set_antialias`), die Subpixel-Reihenfolge
  schaltet die Subpixel-Glättung ein (`Text::set_subpixel`, seit 2026-10-20, siehe unten),
  Hinting steht der App zur Verfügung (`ui.font_prefs()`).
- **P2** · *erledigt* · Schriftauswahl ([`ui/fontpick.rs`](../lib/ui/fontpick.rs)):
  `ui.font_picker` (Aufklappliste mit Suche und virtualisierter Familienliste, Größe in Punkt)
  und `ui.font_dialog` (modaler Dialog mit Suche, Familien, den Schnitten der Familie, Größen,
  Vorschau in der Schrift); `FontSel`, `ui.font_load` / `font_resolved` (auch generische
  Namen), `ui.text_in_font`. Getestet mit `fontpicktest` (Auswahl, Dialog, Glättung aus) und
  `fonttest --selftest` / `--conf`.
- **P2** · *erledigt* · Rechtschreibprüfung ([`spell`](../lib/spell/lib.rs),
  [`ui/edit_spell.rs`](../lib/ui/edit_spell.rs)): eigener Hunspell-Leser ohne libc (`SET`
  UTF-8 / ISO-8859-x / KOI8 / cp125x, `FLAG` long / num / UTF-8, `AF`, `PFX` / `SFX` mit
  Bedingungen, Kreuzprodukt und zwei Suffixen, `NEEDAFFIX`, `CIRCUMFIX`, `FORBIDDENWORD`,
  `KEEPCASE`, `ONLYINCOMPOUND`, Komposita mit `COMPOUNDFLAG` / `BEGIN` / `MIDDLE` / `END`,
  `COMPOUNDPERMITFLAG`, `FORCEUCASE`, `CHECKCOMPOUND*`, `CHECKSHARPS`, `BREAK`, `ICONV`,
  `IGNORE`); Vorschläge über `REP`, `MAP`, `KEY`, `TRY`, Vertauschungen und Editierdistanz.
  Windows nimmt `ISpellChecker` ([`plat/spell.rs`](../lib/plat/spell.rs)) und fällt auf
  Hunspell-Dateien zurück; Linux findet die Wörterbücher in `/usr/share/hunspell`, `myspell`,
  `~/.local/share/hunspell`, `DICPATH`; Web / Android nur `app://hunspell`. Textfelder mit
  `TB_SPELL`: rote Wellenlinie, Kontextmenü mit Vorschlägen, „Zum Wörterbuch hinzufügen“
  (`config://spell`), „Ignorieren“; geprüft je Textrevision und Wort-Cache, Wörterbuch im
  Hintergrund geladen; seit 2026-10-19 entstehen auch die Vorschläge auf einem Worker
  (`plat::Job`, `Speller::suggester`): das Menü öffnet sofort mit „Vorschläge werden gesucht …“
  und füllt sich, sobald sie da sind (ein langes Kompositum braucht mit de_DE_frami bis
  ~0,2 s), per Tastatur geöffnet bekommt dann der erste Vorschlag den Fokus; Windows'
  `ISpellChecker` antwortet weiter im UI-Thread, Skripte bleiben synchron
  (`spell_suggest_async(true)` / `spelltest --spell-async` + `sync` testen das Warten). Tests: `apps/spelltest` (eigenes Testwörterbuch, Engine-Selbsttest,
  Skripte), `plattest --test spell`.
  Seit 2026-10-19 gegen echte Wörterbücher geprüft und an Hunspell 1.7 angeglichen (de_DE_frami,
  en_US, en_GB und zwölf weitere aus dem Freedesktop-SDK 26.08; Referenz Hunspell 1.7.3 über
  `libhunspell`; `spelltest --compare AFF DIC WÖRTER`): `COMPOUNDRULE` (`*` / `?`, Flags in
  Klammern, Affixe am letzten Teil; en_US-Ordinalzahlen „21st“, „111th“), versteckte
  Großschreib-Einträge gemischt geschriebener Wörter („KdF“ → „KDF“, „StRG“ → „STRG“), verbotene
  Wurzeln auch mit Endung und als Kompositum („Fotographen“), Kompositum-Flags wie Hunspell
  (Präfix allein: seine Fortsetzungsklasse, mit Suffix: die des Suffixes – frami-
  Bindestrich-Komposita „Initialisierungs-Zeichenkette“), `CIRCUMFIX` bei einem Präfix allein,
  Präfixe in Mittelteilen, `FORCEUCASE` / `KEEPCASE` von der Wurzel, Komposita aus drei oder
  mehr Teilen nur, wenn keine einfache Korrektur ein Wort ist (Hunspell 1.7: „Fahrradweg“ =
  „Fahrrad weg“), Wortpaare mit Leerzeichen, `CHECKCOMPOUNDPATTERN` mit Flags (nl
  „Resolutie-eenheid“), `CHECKCOMPOUNDREP` wie Hunspell, `ICONV` mit „0“ als Ziffer (uk_UA),
  Vorschläge mit zweiteiligen Komposita. Übereinstimmung beim Prüfen (Wortlisten aus den
  Übersetzungen der Flatpak-Runtimes, eigene Listen mit Komposita, Beugungen, ß, Kontraktionen,
  Tippfehlern): de_DE 100 % auf 719 Handwörtern, 4000 Komposita und 4339 Tippfehlern, 99,93 %
  auf 51 046 Textwörtern; en_US 100 % / 99,97 % (32 464); en_GB 100 % / 99,96 %; es, it, ru, pl,
  pt_BR, cs, nl, uk 100 %, sv und fr 99,99 %, da 99,87 %, hu 98,6 % (je 20 000 Wörter). Die
  Abweichungen bei de / en sind GROSS geschriebene Wörter mit Bindestrich („CD-ROM“,
  „INFO-DATEI“), die Hunspell durch einen Fehler ablehnt (es zerlegt die schon in „Cd-rom“
  umgewandelte Form) – JFLib nimmt sie bewusst an. Vorschläge für Tippfehler richtiger Wörter
  (je ~390): das Original steht vorn bei de 83,8 % (Hunspell 75,4 %), en_US 78,4 % (72,0 %),
  en_GB 75,5 % (59,2 %, mit `PHONE`), unter den ersten fünf bei 96,4 / 97,9 / 94,3 % (Hunspell
  86,3 / 91,3 / 77,5 %). Alle Wort-Erwartungen von `--test engine` (de_TEST und 31 kleine
  Wörterbücher, 354 Prüfungen) stimmen mit echtem Hunspell überein. Zeiten in WSL, nacheinander
  gemessen: Laden de_DE 0,13 s (Hunspell 0,21 s), en_US 17 ms (35 ms), en_GB 47 ms (78 ms);
  Speicher höchstens 21 MB für de_DE (Hunspell +27 MB), en_US 4,7 MB, en_GB 10,6 MB (wie
  Hunspell); Prüfen von 51 046 deutschen Textwörtern 9,3 s (Hunspell 12,8–15,3 s), 32 464
  englischen 50 ms (135 ms); Vorschläge de Ø 32 ms, höchstens 0,14–0,19 s (Hunspell 67–79 ms /
  0,23–0,37 s), en_US Ø 3 ms, höchstens 7 ms (15 / 35 ms), en_GB Ø 6 ms (61 ms). Ein langes
  Kompositum aus drei oder mehr Teilen kostet beim ersten Prüfen bis ~15 ms (die
  Vorschlagsprobe; danach im Wort-Cache) – Textfelder prüfen daher höchstens 6 ms je Frame
  (`BUDGET_US`), der Rest folgt per Timer.
  Es fehlen noch: `COMPLEXPREFIXES` (zwei Präfixe; von 109 Wörterbüchern nur ckb), Präfix plus
  zwei Suffixe, `PHONE`-Vorschläge (en_GB, en_ZA, oc_FR), `OCONV`, `COMPOUNDSYLLABLE` /
  `COMPOUNDROOT` (hu_HU), `COMPOUNDMORESUFFIXES` (ko), die Ersetzungen von
  `CHECKCOMPOUNDPATTERN`, Morphologie; Prüfen in `textbox_surface` (jfWord) und im Barrierefreiheitsbaum (Fehler werden
  Screenreadern nicht gemeldet); Grammatik; ein Sprachwechsel je Textfeld (heute eine Sprache
  für die ganze UI).
- **P2** · *erledigt* · Unicode-Segmentierung ([`text/segment.rs`](../lib/text/segment.rs),
  Unicode 15.0): Grapheme und Wörter nach UAX #29 (GB3–GB13, WB3–WB16), Zeilenumbruch nach
  UAX #14 (LB2–LB31, LB25 als Paarregeln, LB30 mit ostasiatischer Breite, LB30a/b). Die Tabelle
  (`segment_data.rs`, ~4100 Bereiche) erzeugt `tools/gen_segment.py` aus Perls Kopie der
  Unicode-Datenbank (in Git für Windows enthalten) und Pythons `unicodedata`. Vorher liefen
  Cursor und Entf pro Codepunkt, Wörter nur nach ASCII-Klassen, Umbruch nur an Leerzeichen.
  Jetzt: Pfeile / Entf über ganze Grapheme (Akzente aus zwei Codepunkten, Emoji-Familien,
  Flaggen, Hangul), Rücktaste löscht Emoji-Folgen ganz und sonst den letzten Codepunkt,
  Strg+Pfeile / Doppelklick über Wörter (`can't`, `3.14` als ein Wort), Klicks rasten auf
  Graphemgrenzen ein, `TB_WRAP` und `TX_WRAP` umbrechen nach Bindestrichen, zwischen
  Ideogrammen, nie in Zahlen oder vor schließenden Klammern. Tests: `uitest --only segment`
  (51 Fälle aus den Regeln; die offiziellen `*BreakTest.txt` liegen hier nicht vor),
  `gallery/tests/segment.uis`. Es fehlen noch: GB9c (indische Konjunkte, kam mit Unicode 15.1),
  Wörterbuch-Segmentierung für Thai / CJK-Wörter (jedes Ideogramm ist ein Wort), das Layout von
  jfWord (`doc_layout` bricht selbst um).
- **P3** · *erledigt* (2026-10-20) · Subpixel-Kantenglättung (ClearType-artig,
  [`text/lcd.rs`](../lib/text/lcd.rs)): Glyphen in dreifacher Auflösung quer zu den Subpixeln
  (RGB / BGR waagrecht, VRGB / VBGR senkrecht), FreeTypes 5-Tap-LCD-Filter, Mischung je
  Farbkanal im linearen Licht (Gamma 1,6, Deckung nach der Helligkeit der Textfarbe angepasst:
  schwarzer und weißer Text so kräftig wie Graustufen-Text, `fonttest --selftest` misst es).
  Nur wo es sicher ist: auf dem deckenden Framebuffer (`FMT_XRGB`); A8- und PARGB-Ziele
  (Ebenen, Bilder), Farb-Glyphen (COLR / CBDT), das Icon-Font, Glyphen zu groß für den
  Glyphen-Cache und schwarz-weißer Text bleiben in Graustufen; der Cache-Schlüssel enthält die
  Art. Die UI übernimmt die Reihenfolge des Systems im Fenster (Wechsel zur Laufzeit zeichnet
  alles neu); headless bleibt alles grau, außer `jfgui_FONT_RGBA` erzwingt es
  (`fontpicktest` `subpixel_*.uis`). RTL-Fenster spiegeln Befehle, nicht Pixel: die Reihenfolge
  bleibt die physische. Es fehlen: Hinting, Abschalten bei skalierten Fenstern (Wayland-
  Compositor ohne passende Skalierung), Subpixel-Positionierung in Dritteln statt Vierteln.
- **P3** · *erledigt* (2026-10-20) · Silbentrennung ([`spell/hyphen.rs`](../lib/spell/hyphen.rs),
  [`ui/hyph.rs`](../lib/ui/hyph.rs)): Liangs Algorithmus auf TeX-Mustern (`\patterns`,
  `\hyphenation`) und auf libhyphens `hyph_xx_YY.dic` (alle Zeichensätze der ersten Zeile,
  `LEFTHYPHENMIN` / `RIGHTHYPHENMIN` / `COMPOUNDLEFTHYPHENMIN` / `COMPOUNDRIGHTHYPHENMIN`,
  `NOHYPHEN`, zwei Ebenen mit `NEXTLEVEL`). .dic-Dateien werden wie von libhyphen gelesen (sein
  Automat wendet nur das längste Muster-Präfix an, Teile laufen erneut durch die erste Ebene):
  auf 21 Wörterbüchern des freedesktop-SDK (de, fr, nl, ru, uk, cs, hu, …) gleiche Ergebnisse
  wie libhyphen 2.8 für 214 000 Wörter (`spelltest --hyphenate`). Zeilenumbruch:
  `text::segment::wrap_end_hyph` (Haken für Trennstellen, weiche Trennstriche U+00AD, sonst
  unsichtbar), `TX_HYPHENATE` / `TB_HYPHENATE`, Sprache aus der UI-Sprache
  (`ui.hyph_language`, `ui.next_hyph_language`), Wörterbücher wie Hunspell-Dateien
  (LibreOffice-Erweiterungen, `/usr/share/hyphen`, `config://hyphen`, `app://hyphen`).
  Tests: `spelltest --test engine --only hyphen`, `uitest --only segment`,
  `spelltest/tests/hyphen*.uis`. Es fehlen: libhyphens nicht-standardmäßige Trennung mit
  Ersetzung (Zucker → Zuk-ker, v. a. hu_HU: solche Stellen trennen nicht), Trennung im Layout von
  jfWord (`doc_layout`), Blocksatz-Abwägung (heute: die letzte passende Trennstelle).
- **P3** · *fehlt* · Achsen variabler Fonts in der UI.

## 5. Internationalisierung

Vorhanden: [`ui/i18n.rs`](../lib/ui/i18n.rs) mit TSV-Katalogen, Deutsch als Quellsprache;
einstellbares Dezimaltrennzeichen. Seit 2026-10-17 auch Systemsprache, Regionalformate,
Pluralregeln und ICU-artige Nachrichten, seit 2026-10-18 Bibliothekstexte in neun Sprachen,
Sortierung nach Sprache und Groß- / Kleinschreibung.

- **P1** · *erledigt* · RTL-Layout ([`ui/rtl.rs`](../lib/ui/rtl.rs)): Ist die UI-Sprache
  rechts-nach-links (Arabisch, Hebräisch, Persisch, Urdu …) oder `--rtl` / `set_rtl` gesetzt,
  wird jedes Fenster als Ganzes gespiegelt (wie `WS_EX_LAYOUTRTL`): Layout und Hit-Tests
  bleiben logisch, die aufgezeichneten Befehle werden vor dem Rendern gespiegelt. Boxen,
  Scrollbars, Menüs / Untermenüs, Checkboxen, Regler und eigene Titelleisten kippen; Text,
  Bilder und Zeichenflächen behalten ihre Richtung; Richtungs-Icons werden getauscht. Zeiger
  und Pfeiltasten folgen dem Bildschirm. Textfelder, Zeichenfläche und Farbwähler sind
  LTR-Inseln (`ltr_begin` / `ltr_end`). Skripte, Agent, Popup-Fenster, WM_NCHITTEST,
  Fenstergriffe und IME sehen physische Koordinaten. Getestet mit `gallery/tests/rtl.uis`.
  Bibliothekstexte gibt es auch auf Arabisch und Hebräisch (`datagallery/tests/lang_ar.uis`).
  Es fehlen noch: rechtsbündige Eingabe mit Bidi-Cursor für RTL-Text in Textfeldern, der
  native Fensterrahmen (Titelleiste des Systems).
- **P1** · *erledigt* · Pluralregeln (CLDR) und Platzhalter mit Namen bzw. Index
  ([`ui/i18n_msg.rs`](../lib/ui/i18n_msg.rs)): `i18n::msg` versteht `{name}`, `{0}`, `{}`,
  `plural` (Kategorien `zero` … `other`, `=N`, `#`, `offset`), `select`, `number` (gruppiert,
  `percent`, `integer`, `currency/EUR`, `size`) und `date` / `time` / `datetime`.
  Eigene Pluralregeln für über 30 Sprachen (u. a. fr, ru, pl, cs, ar, he, ja); alle übrigen
  wie Deutsch / Englisch (`one` nur für 1).
- **P1** · *erledigt* · Locale-Formate und Systemsprache
  ([`ui/i18n_locale.rs`](../lib/ui/i18n_locale.rs), [`plat/locale.rs`](../lib/plat/locale.rs)):
  Zahlen (Gruppierung inkl. Indien, Mindestgruppierung), Prozent, Währungen (Symbol und
  Stellung je Region, Nachkommastellen je Währung), Dateigrößen, kurzes / langes Datum mit
  Genitiv-Monatsnamen, Uhrzeit 12 / 24 h, erster Wochentag, Schreibrichtung – rund 45
  Regionen. Systemsprache und Regionalformat: Windows (`GetUserPreferredUILanguages`,
  `GetUserDefaultLocaleName`), Linux (`LANGUAGE`, `LC_*`, `LANG`), Browser
  (`navigator.languages`), Android (`LocaleList`); `jfgui_LANG` überschreibt, headless bleibt
  es Deutsch. Kataloge wählen die passende Spalte selbst. Zahlenfelder und Datumsauswahl
  nutzen das Format. Es fehlen noch: Ziffern anderer Schriften (arabisch-indisch).
- **P2** · *erledigt* · Bibliothekstexte in weiteren Sprachen ([`ui/i18n.rs`](../lib/ui/i18n.rs)):
  Deutsch, Englisch, Französisch, Spanisch, Italienisch, Niederländisch, Polnisch, Arabisch,
  Hebräisch (`i18n::library_languages`), je Sprache auch Monats- und Wochentagsnamen
  (`month_name`, `month_short`, `weekday_short`, `weekday_abbr`; eigenständige Formen, Polnisch
  „październik“). Die Bibliothekssprache folgt `language_code()`, sonst Englisch. Jetzt über
  `tr` verdrahtet: Dateidialog (Titel, Knöpfe, Meldungen, Werkzeugleiste), Tastennamen der
  Kürzel („Strg+Umschalt+S“ → „Ctrl+Maj+S“), Register-Pfeile, Baum-Chevrons, Ribbon,
  Zurücksetzen-Knopf, Datumsauswahl, Toasts. `--lang CODE` wirkt wie `jfgui_LANG` (Skripte:
  `#!args --lang ar`). Getestet: `i18ntest --only library` (jede Sprache hat jeden Text,
  Arabisch / Hebräisch in ihrer Schrift), `datagallery/tests/lang_fr.uis`, `lang_ar.uis`.
  Es fehlen noch: die englischen Beschriftungen der eigenen Titelleiste (`chrome.rs`:
  Minimize / Maximize / Close), weitere Sprachen.
  *Bewertung Englisch als Quellsprache:* Für die Bibliothek lohnt es sich – englische Schlüssel
  sind für Mitwirkende ohne Deutsch lesbar, passen zu gettext / Weblate / Crowdin, und Englisch
  ist ohnehin die Rückfallsprache. Der Umbau ist mechanisch und auf `lib/ui` begrenzt (rund 45
  Aufrufe, Tabelle mit Englisch in Spalte 0); headless bleibt Deutsch, Skripte ändern sich
  nicht. Vorschlag: (1) Bibliothek: `tr("Open")` mit Englisch als Schlüssel, Deutsch als
  Spalte; (2) App-Kataloge: die erste Spalte des TSV-Kopfs ist schon frei wählbar, nur
  `LANG_DE` = Spalte 0 und `t()` (Quelltext bei Spalte 0) setzen Deutsch voraus – ein
  `LANG_SOURCE`-Alias und die Quellsprache aus dem Kopf genügen, BackupCraft und PaintCraft
  bleiben deutsch.
- **P2** · *erledigt* · Sortierung nach Locale und Unicode-korrekte Groß- / Kleinschreibung
  ([`ui/i18n_collate.rs`](../lib/ui/i18n_collate.rs)): `i18n::collate(a, b)`,
  `Locale::collate`, `Collator::new(tag)` mit Stärken (Grundbuchstabe, Akzente, Groß / Klein,
  identisch), `numeric` („Datei 2“ vor „Datei 10“, auch arabisch-indische Ziffern),
  `phonebook` (ä = ae; auch `-u-co-phonebk`, `de-DE_phoneb`), Sortierschlüssel (`key`, `sort`).
  Vereinfachter UCA ohne CLDR-Daten: Leerzeichen < Satzzeichen < Ziffern < Latein < andere
  Schriften; Akzente aus einer generierten Zerlegungstabelle
  ([`ui/i18n_unicode.rs`](../lib/ui/i18n_unicode.rs) aus UnicodeData per
  [`tools/gen_unicode.py`](../tools/gen_unicode.py), 1033 Zerlegungen, rund 8 KB).
  Sprachregeln: de, sv, fi, da, nb, es, cs, sk, pl, tr; überall ß = ss, æ = ae, ø / ł / đ als
  Varianten, kyrillische Zusatzbuchstaben an ihrem Platz, hebräische Endformen, Katakana wie
  Hiragana. Groß / Klein: `to_upper`, `to_lower`, `fold`, `eq_ignore_case` (Sprache aus dem
  Regionalformat, `*_in(lang, s)` für eine feste): core / alloc bringen die vollständigen
  Unicode-Abbildungen (ß → SS, finales Sigma) – ergänzt sind Türkisch / Aserbaidschanisch und
  Case Folding. Der Dateidialog sortiert damit (Ordner zuerst, Zahlen nach Wert); Raster
  sortiert die App (`datagallery` zeigt es mit `collator().numeric(true)`). Getestet:
  `i18ntest --only collate`, `--only case`. Es fehlen noch: weitere Sprachregeln (hu, ro, lt,
  hr, Litauisch-Punkt, Griechisch-Großschreibung ohne Akzente), Persisch-Reihenfolge,
  `contains` ohne Groß / Klein für Filter.
- **P3** · *erledigt* · Werkzeug, das Übersetzungsschlüssel extrahiert und fehlende Einträge
  meldet (`python build_rs.py i18n [APP|lib] [--update]`, [`tools/i18n_check.py`](../tools/i18n_check.py),
  seit 2026-10-22; GUIDE.md „Checking the catalogues“): liest die Texte aus den Quellen
  (`t` / `tc` / `tr` / `tf`, `msg!`-Vorlagen, `list(&TABELLE)`, mit `// i18n` markierte
  Tabellen, Befehls- und Menübeschriftungen, wenn die App sie übersetzt; bei der Bibliothek
  `tr("…")` gegen `LIB_TEXTS`) und meldet je App fehlende Zeilen, unübersetzte Zellen je
  Sprache, nicht mehr benutzte Zeilen, abweichende Platzhalter (`{}`, `{0}`, `{name}`, Arten
  von `i18n::msg`; `tf`-Texte nur `{}`) und Pluralformen (`other`, die CLDR-Kategorien der
  Sprache). `--update` hängt fehlende Texte mit leeren Zellen an den passenden Katalog an
  (Quellreihenfolge, bestehende Zeilen bleiben unberührt). Die Suite prüft mit `--check`:
  Fehler und Lücken in den heute vollständigen Katalogen (BackupCraft, JFNetworker,
  Bibliothek) machen sie rot, die übrigen Lücken werden nur gezählt. Stand 2026-10-22:
  BackupCraft und JFNetworker vollständig (JFNetworker: eine unbenutzte Zeile „Disk“),
  Bibliothek vollständig („Neue Ausgabe“ des Terminals ergänzt; „Dateityp:“ und „Neuer
  Ordner“ unbenutzt), PaintCraft: 147 Texte ohne Zeile (u. a. Effekt-Untermenüs,
  Einstellungen, Druck-, Speicher- und Absturzdialog) und 7 unbenutzte Zeilen; „B&ild“ (das
  Menü hieß im Katalog noch „&Bild“) korrigiert. Getestet in `tools/tests/test_devkit.py`.

## 6. Widgets und Layout

Gut abgedeckt: Grid, Tree, Ribbon, Splitter, Docking, Rich-Text- und Code-Editor, Breadcrumb,
Wizard, Charts, Datums- und Farbwahl, Toasts, mehrere echte Fenster, Popups über den
Fensterrand hinaus.

- **P2** · *erledigt* · Grid-Layout ([`ui/grid.rs`](../lib/ui/grid.rs), `gridbox_begin`):
  Spalten fest / natürlich / Füllanteil, Zeilen nach dem höchsten Kind oder vorgegeben, Spannen
  über Spalten und Zeilen, freie Zellwahl, Ausrichtung je Grid und je Kind; gemessen über
  Pässe wie die Boxen, gespiegelt mit RTL. Getestet mit `gallery/tests/gridbox.uis`.
- **P2** · *erledigt* · Docking-Framework ([`ui/dock.rs`](../lib/ui/dock.rs),
  `dock_begin` / `dock_next` / `dock_end`): Panels links, rechts und unten um einen
  Dokumentbereich in Tab-Gruppen, Splitter mit gespeicherten Größen, Andocken per Ziehen mit
  Zielmarken und Vorschau (Seitenränder, als Register, davor / dahinter, in eine Tab-Leiste),
  Lösen als schwebendes Toolfenster und Zurückdocken (auch per Ziehen aus dem Toolfenster),
  Ein- / Ausblenden, Einklappen zur Icon-Leiste, Gruppenmenü; der Zustand `DockState` gehört
  der App und wird als Text gespeichert / geladen. RTL gespiegelt, Tastatur (Tab-Leiste als
  Fokusstopp, Strg+Bild auf / ab, `DockState::focus` für Kommandos). Getestet mit
  `apps/docktest` (8 Skripte, Windows und Linux). PaintCraft behält seine eigene Platzierung
  schwebender Paletten im Hauptfenster (anderes Modell,
  [`dock.rs`](../apps/PaintCraft/src/workspace/panels/dock.rs)).
- **P2** · *erledigt* · Allgemeine Undo/Redo-History für Apps
  ([`ui/history.rs`](../lib/ui/history.rs)): `History<S>` mit `Edit` (apply / revert / merge),
  `Snapshot` für kleine Zustände, Gruppen, Grenze, gespeicherter Stand (`dirty`), `undo_keys()`
  für Strg+Z / Strg+Y außerhalb von Textfeldern. Getestet mit `apps/uitest`.
- **P2** · *erledigt* · Fertiges Rich-Text-Widget und Code-Editor (2026-10-20). Beide halten ihr
  Dokument in einem eigenen Modell mit Auswahl und Undo (`ui::History`, Tippen und
  Backspace-Folgen als ein Schritt) und teilen einen Editier-Kern
  ([`ui/ed_core.rs`](../lib/ui/ed_core.rs): Graphem-Cluster, Wörter, Zeilen mit
  Vorzugs-x, Seiten, Zwischenablage- und Undo-Tasten wie die Textfelder); solange sie den Fokus
  haben, übernehmen sie den Bearbeitungszustand der Textfelder (Tastenfilter, `textbox_select`,
  Screenreader-Auswahl), IME und Lupen bekommen den Caret über `ime_pos`, die Komposition
  (IBus- / Wayland-Preedit) wird am Caret gezeichnet, der Baum bekommt einen
  `K_TEXTBOX`-Eintrag.
  * Rich-Text: [`ui.richtext`](../lib/ui/rich_edit.rs) über `RichDoc`
    ([`ui/rich_doc.rs`](../lib/ui/rich_doc.rs)): Absätze mit Läufen (fett, kursiv,
    unterstrichen, durchgestrichen, Farbe, Hervorhebung, Größe, Schriftfamilie über
    `ui.font_register` / `ui.font_load`, Links), Ausrichtung inkl. Blocksatz, Aufzählungen und
    Nummerierung nach Ebene, Überschriften, Zitate; Umbruch mit `text::segment`
    (Zeilenumbruch-Stellen, Cluster in zu langen Wörtern), nur sichtbare Absätze gezeichnet,
    Layout je Absatzversion gecacht. Tastenkürzel (Strg+B / I / U, Strg+K Link-Flyout, Strg+L /
    E / R / J, Strg+Umschalt+8 / 7, Strg+1..3 / 0, Tab-Ebenen …), `ui.richtext_toolbar`
    (Undo, Formatvorlage, Schrift, Größe, Zeichenformate, Farben, Link, Ausrichtung, Listen,
    Einzug, Format löschen), Links per Strg+Klick (`richtext_link`). Zwischenablage: Text und
    HTML mit eingebettetem exaktem Fragment; HTML von Browsern / Office wird gelesen
    ([`ui/rich_html.rs`](../lib/ui/rich_html.rs)). Speichern: JSON (`to_json` / `from_json`),
    HTML und Text mit Listenzeichen.
  * Code: [`ui.code_editor`](../lib/ui/code_edit.rs) über `CodeDoc`
    ([`ui/code_doc.rs`](../lib/ui/code_doc.rs)) mit `Highlighter`
    ([`ui/highlight.rs`](../lib/ui/highlight.rs)): Zeilennummern, Monospace-Schrift
    (`mono_font_set`), Tabs / Einrückung (Auto-Einrückung, schließende Klammer eine Ebene
    zurück, Block-Einzug mit Tab / Umschalt+Tab, Backspace bis zum Tabstopp), aktuelle Zeile,
    Klammerpaare (außerhalb von Strings / Kommentaren), Suchen / Ersetzen (inkrementell,
    Groß/Klein, ganzes Wort, F3, „Alle ersetzen“ als ein Schritt), Strg+/ Kommentare,
    Alt+Hoch / Runter Zeilen verschieben. Große Dateien: nur sichtbare Zeilen werden gesetzt
    und hervorgehoben, Zustände je Zeile folgen dem Änderungsjournal (`changes_since`) und
    stoppen, sobald die alten Zustände wieder passen – 100 000 Zeilen: Frame-Aufbau ≈ 0,2 ms,
    Tippen oben tokenisiert 1 Zeile. Regel-Engine (Zeilenkommentare, verschachtelte
    Blockkommentare, Strings mit Escapes / mehrzeilig, Rust-Raw-Strings, Wortlisten, Zahlen,
    eigene Funktionen) mit Grammatiken für Rust, JSON, INI, TOML und eine Markdown-Teilmenge;
    Farben `CodeTheme` hell / dunkel / hoher Kontrast (`ui.code_theme()`).
  * Diagnosen (L6, 2026-10-21): `CodeDoc::set_diagnostics(&[Diag])` (Zeile / Spalte ab 1 in
    Zeichen, Fehler / Warnung / Hinweis, Meldung) zeichnet Wellenlinien in der Farbe der
    Schwere und Marken in einer eigenen Spalte der Leiste (Kreis mit Kreuz, Dreieck, „i“ –
    Formen, auch im hohen Kontrast unterscheidbar; `CE_MARKS` hält die Spalte); Tooltip mit
    den Meldungen, F8 / Umschalt+F8 zur nächsten / vorigen, die am Caret in `code_status` /
    `code_diag` und als Eintrag für Screenreader. Sie folgen jeder Änderung (Tippen, Enter,
    Löschen, Block-Einzug, Kommentare, verschobene Zeilen, Undo / Redo) bis zur nächsten
    Prüfung: ersetzter Text lässt eine Diagnose schrumpfen, ganz gelöscht entfällt sie. Nur
    sichtbare Zeilen werden angesehen, eine Wellenlinie ist ein Zeichenbefehl (100 000 Zeilen
    mit 13 000 Diagnosen so schnell wie ohne). Grammatik `Highlighter::jfs()` für die
    Skriptsprache (`${…}` mit hervorgehobenem Code: neue Regel `Rule::Interp`), Token für
    Token gegen `script::tokens` geprüft. Getestet mit `tests/code_diag.uis` (Hover, F8,
    Bearbeiten, Prüfen beim Tippen, 100 000 Zeilen, Shots hell / dunkel bei 100 / 150 %,
    hoher Kontrast) und `--test model` (jetzt 266 Prüfungen). Offen: Quick-Fixes,
    Markierungen in der Bildlaufleiste, mehrzeilige Tooltips (lange Meldungen werden bei
    300 DIP abgeschnitten); das Folgen vergleicht alten und neuen Text der geänderten Zeilen –
    mehrdeutige Änderungen (eine Zeile löschen, die wie die nächste beginnt) können eine
    Diagnose bis zur nächsten Prüfung leicht verschieben.
  * Lange Zeilen (2026-10-22): eine Zeile von 7000 Zeichen kostete bis zu 1 s je Durchgang,
    100 000 Zeichen rund 2 Minuten – `text::segment` suchte für jeden Cluster und jedes Wort ab
    dem Zeilenanfang (jede Spaltenberechnung quadratisch), der Editor tokenisierte die Zeile je
    Frame und bei der Klammersuche je Kandidat neu und lief bis zur sichtbaren Spalte durch.
    Jetzt schauen Graphem- und Wortfunktionen nur in die Nähe der Stelle (ab der nächsten Stelle
    davor, an der keine Regel verbindet oder hinüberschaut; neu `segment::cluster_end`), und
    [`ui/code_long.rs`](../lib/ui/code_long.rs) hält für Zeilen ab 4 KB Spalten-Stützpunkte
    (alle ~1 KB) und die Token samt Wiederaufsetzpunkten; nach einer Änderung werden Spalten ab
    dem Stützpunkt davor bis zum Wiedertreffen der alten neu gezählt (danach verschoben, ein Tab
    gleicht die Verschiebung aus) und Token ab 4 KB vor der Änderung neu gescannt, bis der Scan
    wieder dort ist, wo der alte war. Gezeichnet wird nur der sichtbare Teil, Treffer und
    Diagnosen nur dort. 1 MB in einer Zeile (minifiziertes JSON, Rust mit Umlauten / CJK /
    Emoji / Tabs): Tippen 0,7–2 ms, Caret, Auswahl, seitliches Scrollen, F3 0,05–0,2 ms je
    Frame; das erste Laden ~30–80 ms. Getestet mit `tests/code_longline.uis` (Benchmarks mit
    Grenze 30 ms, Spalten und Treffer nach Änderungen), `--test model` (`ui::code_long_check`:
    gehaltene Spalten und Token durch 65 Änderungen gegen frisches Zählen und Scannen, Rust,
    JSON, JFS, Text; jetzt 274 Prüfungen) und `uitest` (die nahen Graphem- und Wortsuchen gegen
    die ganze Zeile); die 63 Shots von editortest und mdviewtest sind byteweise gleich geblieben.
    Offen: ein Konstrukt, das weiter als 4 KB vorausschaut (Markdown-Hervorhebung, die auf ihr
    Ende wartet), wird erst beim nächsten Scan der ganzen Zeile neu bewertet; Undo hält je
    Schritt die ganze alte und neue Zeile (bei 1 MB 2 MB je Schritt); der
    Barrierefreiheits-Text einer einzelnen Zeile über 64 KB bleibt leer.
  * Getestet mit `apps/editortest` (13 Skripte mit `--verify-dirty`, Windows und Linux, und
    `--test model`: 168 Prüfungen der Modelle und Grammatiken).
  * Offen: kein bidirektionaler Text innerhalb eines Absatzes (wie bei den Textfeldern bleibt
    die Reihenfolge LTR); Android-IME (Komposition über `android_ime_*`) nicht angebunden –
    auf Android kommt nur getippter Text an; keine Rechtschreibprüfung, kein Kontextmenü, kein
    Ziehen von Text, keine Bilder / Tabellen im Rich-Text; RTF-Export fehlt (nur HTML / Text /
    JSON); Code-Editor ohne Zeilenumbruch, Mehrfach-Cursor, Regex-Suche und Code-Faltung;
    der Barrierefreiheits-Text großer Dateien (> 64 KB) enthält nur die sichtbaren Zeilen.
- **P2** · *erledigt* · Popups und Tooltips über den Fensterrand hinaus (2026-10-18; die Teile für
  Toolfenster seit U3): Flyouts, Menüs und Untermenüs, Kombolisten, der Kalender von
  `date_picker`, Kontextmenüs und Tooltips **aller** Fenster, auch des Hauptfensters, dürfen
  den Arbeitsbereich des Monitors nutzen – unter / rechts vom Anker, wenn Platz ist, sonst
  darüber bzw. links davon ([`ui/popup.rs`](../lib/ui/popup.rs) `pp_bounds`,
  [`ui/win.rs`](../lib/ui/win.rs) `uiw_popup_area`). Was außerhalb des Fensters liegt, wird
  ein Popup-Fenster der Plattform (Win32 `WS_POPUP` owned / `NOACTIVATE`
  ([`plat/win_popup.rs`](../lib/plat/win_popup.rs)), X11 override-redirect, Wayland
  `xdg_popup`); Eingaben kommen als Ereignisse des Besitzers zurück, Fokus und Tasten bleiben
  beim Fenster, DPI des Besitzers. Wayland verrät die Fensterposition nicht: Popups, die ins
  Fenster passen, bleiben darin, größere bekommen einen `xdg_positioner` mit Anker, Flip und
  Slide ([`plat/linux_wl_popup.rs`](../lib/plat/linux_wl_popup.rs)), und die UI legt das
  Popup dort aus, wo der Compositor es laut `xdg_popup.configure` hingesetzt hat
  (`plat::popup_anchor` / `popup_placed`). Schließen: Druck anderswo in der App, Esc, und
  neu: Verlust der Tastatur durch die App. Headless: Skriptbefehl `screen X Y W H` für einen
  virtuellen Bildschirm, `shot … desktop` zeigt die Popups. Tests:
  `gallery/tests/popup_edges.uis` (Testhaken 9105 / `--popup-edges`: Steuerelemente an allen
  Rändern, Bildschirm mit Platz, rechts unten, links oben, 150 %),
  `gallery/tests/popups.uis` (Toolfenster), echte Fenster `tools/wincheck.py popups|edges`,
  `tools/x11check.py popups|edges --backend x11|wayland`, `tools/wlproxy/popflip.py`
  (Proxy spielt einen Compositor, der Popups verschiebt). Offen: Win32 / X11 begrenzen auf
  den Monitor des Fensters (nicht den des Ankers, bei Fenstern über zwei Monitore);
  Popup-Fenster rendern in der DPI des Besitzers; X11-Popups ohne runde Ecken (kein SHAPE);
  Wayland: keine `xdg_popup.grab` (die UI schließt selbst), bei Positionswechsel neues
  Popup (`reposition` erst ab Version 3), Flip / Slide nur mit einem Compositor, der es kann
  (GNOME, KDE; mit echtem Compositor nicht von Hand ausprobiert – Weston in WSLg setzt Popups
  ohne Anpassung).
- **P2** · *erledigt* · Animationen mit Easing-Kurven und Übergängen: `ease(curve, t)`,
  `anim_ex` mit `EASE_LINEAR` / `IN` / `OUT` / `IN_OUT` / `OUT_CUBIC` / `OUT_BACK`,
  `anim_color` für Farbübergänge; „Animationen reduzieren“ lässt alles springen.
- **P3** · *erledigt* · SVG-Renderer für Bilder und Icons und Bildbetrachter (2026-10-20,
  [`lib/img_svg`](../lib/img_svg/lib.rs), [`ui/imgview.rs`](../lib/ui/imgview.rs)): eine
  praktische Teilmenge von SVG 1.1 / SVG Tiny, einmal geparst und über den Pfad-Rasterizer von
  gfx in jeder Größe scharf gezeichnet (ohne libc): `svg` (auch verschachtelt mit viewBox /
  preserveAspectRatio), `g`, `defs`, `use` + `symbol`, `switch`, `a`; `path` (alle Befehle,
  Bögen), `rect` (rx / ry), `circle`, `ellipse`, `line`, `polyline`, `polygon`;
  Transformationen; Füllung / Kontur (Farben inkl. Namen und `currentColor`, Deckkraft,
  fill-rule, Breite, Enden, Ecken, Gehrungsgrenze, Strichmuster); lineare und radiale Verläufe
  (Stops, Einheiten, gradientTransform, spreadMethod, Brennpunkt, href-Vererbung); Gruppen-
  Deckkraft in Ebenen; `clipPath`; `<style>` mit Typ-, Klassen-, ID- und Attribut-Selektoren,
  Nachfahren- und Kind-Kombinatoren, `style`-Attribut; Einheiten px / pt / pc / mm / cm / in /
  em / ex / %; `text` / `tspan` mit den Schriften der UI (eine Zeile je Abschnitt,
  text-anchor); `image` mit `data:`-URIs. Robust gegen kaputte Dateien (nachsichtiger
  XML-Leser, Grenzen für Größe, Elemente, Verschachtelung, Entitäten, `use`-Zyklen und
  -Explosionen). Als Bildformat `IMGF_SVG` registriert (`.svg`, `.svgz`: `img_core::io::load`
  / `load_ex` liefern die Bitmap in Eigengröße); `ui.svg(id, &doc, w, h)` zeichnet ein SVG
  scharf in seiner Layoutgröße (currentColor = Textfarbe). `ui.image_view(id, &mut
  ImageViewState, ViewImage::Bitmap / Svg, flags)`: Zoom mit dem Rad am Zeiger, Strg +/-/0,
  Strg+1, Einpassen / 100 % / Zoom auf Rechteck (Strg + Ziehen), Verschieben mit linker oder
  mittlerer Maustaste, Bildlaufleisten, Pfeiltasten / Bild auf / ab / Pos1 / Ende, Doppelklick
  100 % ↔ Einpassen; Bitmaps über Mip-Stufen und bilinear, ab 400 % pixelgenau mit Pixelraster
  ab 800 %, SVGs bei jedem Zoom neu gezeichnet (nur der sichtbare Teil); Schachbrett unter
  Transparenz; hell / dunkel / hoher Kontrast; `K_IMAGE`-Eintrag mit Zoom in Prozent.
  * Getestet mit `apps/svgtest` (`--test unit`: 321 Prüfungen, darunter Präfixe und Mutationen
    aller Fixtures; `gallery.uis` mit 13 selbstgeschriebenen SVG-Fixtures hell / dunkel / hoher
    Kontrast / 200 %, `viewer.uis` und `viewer_modes.uis`: Zoom und Verschieben per Skript;
    alle mit `--verify-dirty`, Windows und Linux).
  * Offen: `pattern` (es gilt die Ersatzfarbe), `mask`, `marker`, `filter`, Text auf Pfaden,
    Positionen je Glyphe, `letter-spacing`, externe Dateien in `use` / `image`, CSS-Pseudoklassen
    und `@media`, Animation. Ungleichmäßig skalierte Konturen werden mit der mittleren Breite
    gezeichnet. Eingebettete Bilder brauchen die Decoder der Bildformate (die Laufzeit
    registriert PNG, JPEG, BMP, GIF).
- **P3** · *erledigt* · Markdown- / HTML-Viewer, Listen mit variabler Zeilenhöhe (2026-10-21,
  [`ui/md_doc.rs`](../lib/ui/md_doc.rs), [`ui/md_view.rs`](../lib/ui/md_view.rs),
  [`ui/vlist.rs`](../lib/ui/vlist.rs)): `ui.markdown_view(id, &mut MdViewState, text, MV_*)` /
  `ui.html_view(...)` zeigen ein Dokument nur lesend (Hilfeseiten, README, Versionshinweise,
  Chat-Ausgabe). Markdown nach CommonMark mit den GFM-Teilen: Überschriften (ATX, Setext),
  Absätze mit faulen Fortsetzungszeilen, Zitate, Aufzählungen und nummerierte Listen
  (Startnummern, enge / lockere Abstände, Aufgaben `[ ]` / `[x]`), Code-Blöcke mit Zaun und
  eingerückt, Trennlinien, Tabellen mit Ausrichtung, HTML-Blöcke, Link-Referenzen; im Text
  Hervorhebung nach den Begrenzer-Regeln, `~~durch~~`, Code, Links (inline, Referenz, `<…>`,
  nackte `https://` / `www.`), Bilder, harte und weiche Umbrüche, Entities, Escapes, etwas
  Inline-HTML. HTML über den Leser des Rich-Text-Editors (`rich_html.rs`, erweitert um Regeln,
  Bilder, `pre` / `code class="language-x"`, Tabellen, Listennummern; kein zweiter
  HTML-Parser). Layout: Einzüge, Zitatbalken, Listenmarken nach Tiefe, Aufgaben-Kästchen,
  Überschriften 1–6, Code in Monospace mit der Hervorhebung des Code-Editors, Tabellen mit
  Spaltenbreiten aus den Zellen, Bilder über das VFS (`app://`, Pfade nach `base`; SVG über
  `lib/img_svg`, nie `http(s)`), Umbruch mit `text::segment`; Hebräisch / Arabisch von rechts
  nach links (Richtung je Absatz und Dokument aus dem ersten starken Zeichen, geformt mit
  `text_shape`). Nur sichtbare Blöcke werden gesetzt und gezeichnet, gescrollt wird am ersten
  sichtbaren Block verankert (50 000 Blöcke: rund 1 ms pro Bild). Auswahl mit Maus
  (Ziehen, Doppelklick, Umschalt), Strg+A / Strg+C als Text (Listenmarken, Zellen mit Tab) und
  HTML; Tastatur (Pfeile, Bild auf / ab, Leertaste, Pos1 / Ende, Tab durch die Links, Enter
  aktiviert, Esc); Links werden gemeldet (`MdEv::link`), geöffnet nur mit `MV_OPEN_LINKS` nach
  Klick oder Enter über `open_url`. Barrierefreiheit: neue Art `K_HEADING` (Wert: Ebene),
  `K_LINK`, `K_LISTITEM`, `K_CHECKBOX`, `K_IMAGE` (Alt-Text), `K_TEXT`. Robust gegen kaputte und
  böswillige Eingaben (Verschachtelung begrenzt, Budget für Link- und Tag-Suche, Index für
  Code-Spans: linear). `ui.vlist_begin(id, &mut VListState, count)` / `vlist_item_begin` /
  `vlist_item_end` / `vlist_end`: virtualisierte Liste mit gemessenen oder vorgegebenen
  Zeilenhöhen (Fenwick-Bäume: O(log n)), Verankerung an der ersten sichtbaren Zeile (Zeilen
  darüber dürfen wachsen, eingefügt oder entfernt werden, ohne dass die Ansicht springt),
  Tastatur, Auswahl, `reveal`, Bildlaufleiste und weiches Mausrad (100 000 Zeilen: rund 1 ms
  pro Bild); der Viewer scrollt seine Blöcke mit demselben Höhen-Cache.
  * Getestet mit `apps/mdviewtest` (`--test unit`: 152 Prüfungen, darunter böswillige Eingaben
    mit Zeitgrenzen, die Höhen von 100 000 Zeilen und die Überschriften im
    Barrierefreiheits-Baum; `doc_shots.uis` hell / dunkel / hoher
    Kontrast / HTML / Hebräisch, `doc_rtl.uis`, `doc_scale.uis` (150 %), `doc_links.uis`,
    `doc_select.uis`, `doc_keys.uis` (auch 50 000 Blöcke), `vlist.uis` (100 000 Zeilen); alle mit
    `--verify-dirty`, Windows und Linux).
  * Offen: Suche im Dokument, Fußnoten und Definitionslisten, HTML-Blöcke nur bis zur nächsten
    Leerzeile, Bidi-Neuordnung über Formatwechsel hinweg (je Lauf), synthetischer Fettdruck für
    Ersatzschriften in geformten Läufen, Bilder aus dem Netz und `data:`-URIs; Screenreader
    finden nur die Überschriften der sichtbaren Blöcke (nur sie haben Records).
  * Seit 2026-10-22 kommen die Überschriften als echte Überschriften mit Ebene bei den
    Screenreadern an (UIA, AT-SPI, Web, Android; Abschnitt 11), vorher als Beschriftung.
- **P3** · *teilweise* · Mehrfarbige Vektor-Symbole aus SVG (2026-10-19,
  [`ui/symbol.rs`](../lib/ui/symbol.rs), [`tools/vsym.py`](../tools/vsym.py)): handgezeichnete
  SVG-Dateien (Pfade, Grundformen, Gruppen, Transformationen) werden zu Pfad-Ebenen mit
  Paletten-Rollen; `Symbol` parst sie einmal, `SymPalette::from_theme` färbt hell / dunkel /
  hoher Kontrast, `SymbolCache` rastert einmal pro Größe und Palette. Erster Nutzer: die 26
  Gerätesymbole von JFNetworker (`apps/symboltest`). Beliebige SVG-Dateien (Verläufe, Text,
  Clipping, CSS, Bilder) zeichnet seit 2026-10-20 `lib/img_svg` (`ui.svg`).
- **P3** · *erledigt* · Feste Obergrenzen lockern (2026-10-22, [`ui/ctx.rs`](../lib/ui/ctx.rs)):
  Tasten eines Durchgangs, Befehle und Timer wachsen nach Bedarf (vorher 16 / 32 / 16), ebenso
  Zeigerpunkte, Hit-Boxen, Widget-Records, Clip-Ebenen, Register (Tabs), Leinwände und
  umbrochene Zeilen; die Obergrenzen (1024 Tasten, 65536 Befehle, 4096 Timer ...) fangen nur
  durchgehende Apps ab, im Normalbetrieb wird nichts nachalloziert. Fällige Timer feuern nach
  ihrer Frist, gleiche Fristen in der Reihenfolge des Setzens. Was eine Obergrenze oder eine
  feste Schachtelungstiefe (Fenster, Popups, Menüs, Listen) zurückhält, meldet ein `UIWARN`
  einmal (`limit_hit`). Getestet mit `apps/lifetest/tests/queues.uis` (neuer Skriptbefehl
  `keys N`: 100 Tasten in einem Durchgang, 200 und 5000 Befehle, 100 Timer). Der 256er-
  Ereignisring von plat hat eine Überlaufschlange (bis 65536 Ereignisse, in Reihenfolge): X11
  und Wayland verlieren keine Tasten mehr, wenn ein Lesevorgang mehr als etwa 85 Tastendrücke
  bringt, auch nicht beim Warten auf eine Antwort (`tools/keyburst.py`, `plattest --test evq`).
- **P2** · *erledigt* · Knoten-/Kanten-Leinwand ([`ui/diagram.rs`](../lib/ui/diagram.rs),
  `ui.diagram` / `diagram_minimap` / `diagram_sems`, 2026-10-19): Zoom und Verschieben (Rad,
  Strg+Rad am Zeiger, Mitteltaste, Leertaste, Hand-Werkzeug), Raster mit Einrasten,
  räumlicher Index (256er-Felder), Hit-Test Wegpunkt > Port > Knoten > Kante, Rahmenauswahl,
  Verschieben, Verbinden (aus Ports ziehen, Verbinden-Werkzeug, zwei Klicks), Kanten gerade /
  Kurve / rechtwinklig mit Wegpunkten, Tastatur, Minimap, ein `listitem` pro sichtbarem
  Knoten; die App malt über `DgPaint`. Getestet mit `apps/diagramtest` (8 Skripte mit
  `--verify-dirty`, auch gespiegelt). Seit 2026-10-20 auch die Kanten für Screenreader
  (`diagram_edge_sems`), Tab über die Kanten (`tab_edges`) und ein Name der Leinwand.
- **P2** · *erledigt* · Konsolen-Widget ([`ui/term.rs`](../lib/ui/term.rs), `ui.terminal` /
  `TermState`, 2026-10-19): Scrollback-Ring mit ANSI-Teilmenge (SGR, `\r`, `ESC[K`, `ESC[2J`,
  BEL), 16 Farben aus den Theme-Tokens, Eingabezeile auf `textbox_surface` (IME, Zwischenablage),
  Verlauf, Tab → `Complete`, IOS-`?` (`TF_QHELP`), Strg+C / Strg+Umschalt+6 → `Interrupt`,
  `--More--`-Pager, Busy-Modus mit Type-Ahead, Roh- und Passwortmodus, Mausauswahl, Suche,
  „Neue Ausgabe“; `ui.mono_font_set` / `mono_font` (Apps betten DejaVu Sans Mono ein).
  Getestet mit `apps/termtest` (9 Skripte mit `--verify-dirty`, Windows und Linux). Seit
  2026-10-20 eine feste Seitenlänge `page_len` (wie `terminal length`), und eine beschäftigte
  oder blätternde Konsole hält die Tastatur wie ein Textfeld (keine Einzeltasten-Kürzel,
  Strg+Z / Strg+Y bleiben bei ihr); im Einsatz in den Konsolen von JFNetworker (U3).
- **P2** · *erledigt* · Zeitleiste und Hex-Ansicht für den Simulationsmodus
  ([`ui/timeline.rs`](../lib/ui/timeline.rs) `timeline`, [`ui/hexview.rs`](../lib/ui/hexview.rs)
  `hex_view`): Zeitachse mit 1-2-5-Schritten, Ereignis-Ticks je Spur als Balken pro
  Pixelspalte (viele Tausend Ticks bleiben billig, ein gecachtes Zeichenobjekt), Abspielkopf
  mit Zeit, Suchen per Klick / Ziehen, Strg+Rad-Zoom, Verschieben, Bereichsauswahl, Tasten von
  Tick zu Tick, „Live folgen“; virtualisierte Hex-/ASCII-Zeilen in Monospace mit
  durchscheinenden Markierungen (Protokollfelder), Auswahl per Klick / Ziehen / Tasten, der
  Schlüssel der Markierung unter dem Klick, Strg+C als Hex, Tooltip mit Offset; beide bleiben
  in RTL-Fenstern links → rechts. Getestet mit `datagallery/tests/timeline.uis`,
  `hexview.uis`, `simulation.uis` (hell, dunkel, 150 %).
- **P2** · *erledigt* · Liniendiagramm für Zeitreihen ([`ui/chart.rs`](../lib/ui/chart.rs)
  `line_chart` / `SeriesBuf`, L5 für das Verteilte-Systeme-Labor von JFNetworker): Ringpuffer
  mit Kapazität (Lücken als NaN, Binärsuche, Spalten mit Min/Max je Pixel, sodass 100 000
  Punkte so schnell gezeichnet werden wie 500), mehrere Serien als Linie oder Fläche, x als
  Zeit (Sekunden oder Nanosekunden, Beschriftung 0:05 / 1:00 / 1:30) oder Zahl, y mit
  „schönen“ Schritten und Einheiten (ms / µs, req/s, %, Bytes mit k / M / G), Bereich fest
  oder automatisch, Raster, Legende, Fadenkreuz mit Karte aller Werte, Markierungen (Linien
  und Bereiche: Fehlerinjektionen) und Schwellwerte mit Etiketten, die sich nie überdecken,
  „Live folgen“, Leerzustand, Zusammenfassung für Screenreader; hoher Kontrast mit Strichmustern,
  in RTL-Fenstern links → rechts. Getestet mit `datagallery/tests/linechart.uis`,
  `linechart_hc.uis` und `uitest --only chart`.

## 7. App-Framework und Architektur

- **P1** · *erledigt* · Logging-Framework ([`plat/log.rs`](../lib/plat/log.rs)): Stufen
  ERROR … TRACE, `plat::log!` formatiert nur bei aktiver Stufe; Ringpuffer (64 KiB) für
  Absturzberichte – ein Absturz oder Panic gibt die letzten Zeilen auf stderr aus und markiert
  die Datei, bevor der Crash-Hook der App läuft (`rt::crash` Vor-Hook); stderr ab WARN; Datei
  `logs://APP.log` (der Log-Ordner der App, §2) mit Rotation (1 MiB, 3 alte Dateien) per `AF_LOG_FILE` oder
  `jfgui_LOG_FILE`; `jfgui_LOG` setzt die Stufen. `Ui::warn` landet ebenfalls im Log.
- **P1** · *erledigt* · Lebenszyklus-Hooks als `UIC_*`-Befehle: `UIC_ACTIVE` (App-Fokus),
  `UIC_SUSPEND` (Ruhezustand / Aufwachen: Windows `WM_POWERBROADCAST`, Linux logind
  `PrepareForSleep`), `UIC_SESSION_END` (Abmelden / Herunterfahren: `WM_QUERYENDSESSION`, SIGTERM),
  `UIC_SETTINGS` (Theme, Format, Kontrast, Schriften: `WM_SETTINGCHANGE`, Settings-Portal;
  Theme und Locale folgen dem System selbst); Start und Exit gibt es mit `init` und `UIC_EXIT`
  (auch bei `--auto-close`). Getestet mit `lifetest` (Skriptbefehl `system …`) und Fake-Bussen.
  Es fehlen noch: Web (`visibilitychange`) und Android (`onPause` / `onResume`).
- **P2** · *erledigt* · Typisierte Einstellungen ([`base/settings.rs`](../lib/base/settings.rs)):
  `Key<bool | i64 | f64 | String | usize (Auswahl) | u32 (Farbe)>` mit Vorgabe, Grenzen,
  Beschriftung, Hilfetext, „versteckt“ und „Neustart nötig“; ein `Schema` mit Version,
  Abschnittstiteln und Migration (`migrate(from, &mut Ini)`, `rename`). In der Datei stehen nur
  Werte, die von der Vorgabe abweichen; ungültige Werte gelten als Vorgabe (WARN im Log),
  Zahlen werden begrenzt. `ui.settings_page(id, &mut settings)`
  ([`ui/settings_page.rs`](../lib/ui/settings_page.rs)) baut die Einstellungsseite aus dem
  Schema: Schalter, Zahlenfelder, Textfeld, Auswahlliste, Farbwahl, je Zeile ein
  Zurücksetzen-Knopf. Fensterzustand ([`ui/winstate.rs`](../lib/ui/winstate.rs)): mit
  `AF_SAVE_STATE` merkt sich die Laufzeit Position, normale Größe und „maximiert“ des
  Hauptfensters sowie offen/Platzierung der Werkzeugfenster (Paletten) in
  `config://windows.ini`; eine Position außerhalb aller Monitore wird zentriert. Getestet mit
  `uitest` und `prefstest` (Seite, Zustand, IDs) sowie echten Fensterläufen unter Windows,
  Wayland und X11. Unter WSLg legt der Compositor X11-Fenster selbst ab, dort kommt die
  gespeicherte Position nicht an.
- **P2** · *erledigt* · Asynchrone Ausführung (Executor in der Ereignisschleife), nicht
  blockierende Netz- und Dateiaufrufe ([`plat/task.rs`](../lib/plat/task.rs),
  [`ui/tasks.rs`](../lib/ui/tasks.rs), seit 2026-10-18): ein eigener Executor ohne libc über
  `core::future` (Waker per `alloc::task::Wake`, kein `unsafe`). `ui.spawn(fut) -> Task<T>`
  lässt Futures (auch ohne `Send`) auf dem UI-Thread zwischen den Pässen laufen; `App::frame`
  fragt den Handle ab (`take`, `is_done`, `cancel`, Drop bricht ab, `detach`), ein gelaufener
  Task bekommt einen weiteren Pass – ohne `UIC_WAKE`-Verdrahtung. Weckrufe anderer Threads
  kommen als `plat::wake` / `E_WAKE`; die Fensterschleife wartet höchstens bis zum nächsten
  Task-Timer, Web und Android bekommen ihn als Frame-Verzögerung. Dazu `spawn_blocking` /
  `spawn_cancellable` (eigener Pool mit höchstens 8 Threads, getrennt vom Rechen-Pool),
  `sleep` (headless auf der virtuellen Skript-Uhr), `CancelToken`, `Progress`, `join` /
  `join_all` / `select`, `block_on` für Konsolenprogramme. Netz:
  `netio::Client::request_async` und `download` (Fortschritt aus Content-Length, Abbruch
  zwischen zwei Stücken). Skripte bleiben deterministisch: `sync` wartet auf Pool-Jobs und
  deren Weckrufe, `wait` bewegt die Task-Uhr. Getestet mit `plattest --test async`
  (Reihenfolge, Weckrufe aus anderen Threads, Abbruch, Timer, Panics in Tasks und Jobs),
  `coretest` (HTTP) und `apps/asynctest` (Skripte mit Fortschrittsbalken und Abbrechen-Knopf,
  `--selftest` im echten Fenster unter Windows, X11 und Wayland). Es fehlen: asynchrone
  Sockets ohne Pool-Thread (Reaktor über IOCP / epoll), asynchrone DNS-Auflösung, ein
  Abbruch laufender HTTP-Anfragen innerhalb eines Lese-Timeouts (heute zwischen zwei Stücken),
  Threads für den Blocking-Pool unter Android und im Browser ohne Shared Memory (dort laufen
  die Jobs zwischen den Frames auf dem UI-Thread; der `web_threads`-Build ist ungetestet).
- **P2** · *erledigt* · Hilfen für Widget-IDs ([`ui/ids.rs`](../lib/ui/ids.rs)): `ui.id("name")`
  und `ui.id_n(i)` hashen Namen bzw. Nummern im aktuellen Bereich (`id_push` / `id_push_str` /
  `id_pop`, `id_scope(i, |ui| …)`); `wid("name")` und `wid_path(&["a", "b"])` liefern dieselben
  Werte als Konstanten. Gehashte IDs liegen ab `ID_HASH_BASE` (0x0800_0000) und treffen nie
  handvergebene IDs; Skripte und `jfgui_DEBUG=1` warnen bei zwei Namen mit gleichem Hash, bei
  doppelt benutzten IDs (mit Namen) und bei offenen Bereichen am Ende des Frames.
- **P3** · *erledigt* (2026-10-22) · Stil zur Laufzeit wechseln bzw. eigene Themes für Apps ohne
  neues Style-Crate: App-Themes ([`ui/theme_app.rs`](../lib/ui/theme_app.rs),
  [STYLES.md](STYLES.md#app-themes), [GUIDE §34](GUIDE.md)) überschreiben die Tokens des aktiven
  Stils – Farben je Rolle für Hell, Dunkel und hohen Kontrast, den Akzent (über die Akzent-Rampe
  des Stils), Eckenradien (DIP oder Prozent; Pillen bleiben Pillen), Abstände (Polsterung und
  Lücken der Boxen und Raster), die Höhe der Standard-Steuerelemente, Größe, Zeilenhöhe und
  Stärke je Textstil und eine Schriftfamilie (Datei neben dem Theme, VFS-Pfad oder installierte
  Familie). Format: INI (`base::ini`, neu `Ini::items` mit Zeilennummern); `Theme::from_ini` /
  `load` / `to_ini` / `save`, `ui.set_theme` / `clear_theme` / `load_theme`, `--theme-file`.
  Fehler nennen die Zeile in der UI-Sprache, ein kaputtes Theme lässt das alte stehen; wirksam
  ab dem nächsten Frame. Kein Stil musste dafür umgebaut werden: die Theme-Schicht sitzt in
  `theme_build`, `cmd_rrect` / `cmd_shadow`, `box_begin` / `gridbox_begin`, den Größenaufrufen
  der Standard-Controls und `ts_font` / `ts_line`; ohne Theme rendert alles pixelgleich (Shots
  von gallery in winui, classic und kde und von datagallery samt den Shots ihrer Skripte unter
  Windows und Linux). Stilwechsel zur Laufzeit: `build_rs.py` linkt jedes `style_NAME`, das die
  Quellen einer App nennen, dazu; `ui.styles_add` / `style_names` / `set_style` (`Style::name`;
  `Text::set_embedded` tauscht die eingebetteten Schriften, abgeleitete Schriften folgen).
  Getestet mit `apps/themedemo` (Seite „Darstellung“ mit Live-Editor, Speichern nach
  `config://`, vier gelinkte Stile; Skripte für Token, Pixel und Rechtecke in Hell, Dunkel,
  hohem Kontrast und bei 150 % Text, kaputte Dateien, gleiche Pixel nach dem Rückweg),
  `apps/gallery/tests/themes*.uis` (mit jedem der sieben Stile gebaut) und `uitest --only
  themes` (Format, Fehler mit Zeile, Textform). Grenzen: feste Werte eines Stils bleiben (die
  graue 3D-Fläche von classic kommt aus seiner eigenen Palette, Titelleisten-Schriften,
  Innenabstände der Widgets).
- **P3** · *erledigt* · Bausteine für Simulationen (seit 2026-10-19, für JFNetworker):
  deterministische Zufallszahlen [`base/rng.rs`](../lib/base/rng.rs) (PCG32 nach der
  Referenzimplementierung, SplitMix64, Sprung nach vorn, abgeleitete Generatoren pro Objekt)
  und das Paket-Crate [`netpkt`](../lib/netpkt/lib.rs) ohne I/O: Adressen und Präfixe,
  Internet-Prüfsumme (auch inkrementell) und CRC-32, Codecs für Ethernet, 802.1Q, LLC, ARP,
  IPv4 (Fragmentierung, Reassembly), IPv6-Kopf, ICMP, UDP, TCP mit Optionen, ein Dissektor
  mit Feldern und Bytebereichen für Paket-Inspektoren (auch für IP-Pakete ohne Ethernet, etwa
  auf seriellen Leitungen) sowie pcapng schreiben und lesen.
  Getestet mit `coretest` (Gruppen `netpkt`, `rng`; RFC-Vektoren, unabhängig berechnete
  Prüfsummen). Seit E6 auch DHCP (RFC 2131/2132 mit typisierten Optionen, Relay-Information
  RFC 3046, lange Optionen RFC 3396), DNS-Nachrichten (RFC 1035 mit Namenskompression in beide
  Richtungen, A/AAAA/NS/CNAME/PTR/MX/SOA/TXT, Kürzen auf 512 Byte), NTP und Syslog samt
  Dissektor, getestet mit einem echten mitgeschnittenen DHCP DISCOVER und den Beispielen aus
  RFC 1035 (Gruppen `dhcp`, `dnsmsg`, `ntp`), sowie Metrik-Grundtypen
  [`base/metrics.rs`](../lib/base/metrics.rs) (Counter, Gauge, log-lineares Histogramm,
  Fenster mit 300 × 1 s für p50/p95/p99 ohne Einzelwerte; Gruppe `metrics`). Seit T2 auch
  HTTP/1.1 als inkrementeller Codec ([`netpkt/http.rs`](../lib/netpkt/http.rs): Parser für
  Stücke beliebiger Größe, Content-Length, chunked, Keep-Alive, Pipelining, Dissektor; Gruppe
  `httpmsg`). Seit D1 auch Consistent Hashing mit vnodes und Präferenzlisten
  ([`base/hashring.rs`](../lib/base/hashring.rs)) und eine LRU-Tabelle nach Anzahl und Gewicht
  ([`base/lru.rs`](../lib/base/lru.rs); Gruppen `hashring`, `lru`), im Einsatz im
  Loadbalancer, KV-Speicher und Cache des Labors von JFNetworker. Dazu typisiertes JSON
  ([`agent_core/src/jmap.rs`](../lib/agent_core/src/jmap.rs): `record!`, `wire_enum!`, aus
  BackupCraft in die Library gezogen; `bc_proto` nutzt es mit) und eingerücktes JSON für
  Dateien (`json::stringify_pretty`), getestet mit der coretest-Gruppe `jmap`.
  Seit D2 fasst `Window::hist_between` die Slots eines Zeitraums zu einem Histogramm zusammen
  (die Zeitreihen des Labors von JFNetworker: ein Punkt pro Intervall, Perzentile beliebiger
  Zeiträume; Gruppe `metrics`). Seit R1 (2026-10-22) übersetzt
  [`netpkt/nat.rs`](../lib/netpkt/nat.rs) Adressen und Ports in Paketen an Ort und Stelle und
  führt jede Prüfsumme inkrementell nach (RFC 1624; ICMP-Fehler samt Zitat nach RFC 5508,
  Fragmente), `checksum::update` folgt einem geänderten Bytebereich (Gruppe `netpkt`: das
  Ergebnis gleicht dem frisch gebauten Paket Byte für Byte); im Einsatz in NAT und den
  Zugriffslisten von JFNetworker.
  Seit R4 auch die BPDUs von STP und RSTP ([`netpkt/stp.rs`](../lib/netpkt/stp.rs):
  Configuration-, TCN- und RST-BPDUs mit ihren Flags, Bridge- und Port-IDs, Pfadkosten nach
  802.1D-2004, Prüfung nach 9.3.4, Frames mit LLC, Dissektor; Gruppe `stp`), im Einsatz im
  Spanning Tree der Switches von JFNetworker.
  Seit R2 auch RIP ([`netpkt/rip.rs`](../lib/netpkt/rip.rs): Version 2 nach RFC 2453 und 1 nach
  RFC 1058, Requests und Antworten mit ihren Einträgen, der Authentifizierungs-Eintrag erkannt,
  Dissektor mit den Problemen, die ein Empfänger ablehnt; Gruppe `rip`), im Einsatz in den
  Routern und L3-Switches von JFNetworker.
  Seit R3 auch OSPF ([`netpkt/ospf.rs`](../lib/netpkt/ospf.rs): Version 2 nach RFC 2328, der
  Kopf mit Prüfsumme und Authentifizierungsfeld, Hello, Database Description, Request, Update und
  Acknowledgement, Router-, Network-, Summary- und AS-externe LSAs mit der Fletcher-Prüfsumme,
  welche von zwei Instanzen neuer ist, Dissektor mit jedem LSA; Gruppe `ospf`), im Einsatz in
  den Routern und L3-Switches von JFNetworker.
  Seit R5 auch ICMPv6 und Neighbor Discovery ([`netpkt/icmpv6.rs`](../lib/netpkt/icmpv6.rs):
  Echo und Fehler nach RFC 4443 mit Zitat und Prüfsumme über den Pseudo-Header, Router- und
  Neighbor-Solicitations und -Advertisements und Redirects nach RFC 4861 mit ihren Optionen und
  der Prüfung des Empfängers; [`netpkt/ipv6.rs`](../lib/netpkt/ipv6.rs): Erweiterungsköpfe,
  Fragmente und Reassembly nach RFC 8200 und 5722; Solicited-Node-Gruppen und EUI-64,
  Dissektor mit jedem Feld; Gruppe `icmpv6`), im Einsatz im IPv6 von JFNetworker.
  Seit R5b auch die Auswahl der Zieladresse nach RFC 6724
  ([`netpkt/addrsel.rs`](../lib/netpkt/addrsel.rs): Scope, Präzedenz und Label der
  Standard-Policy-Tabelle, die Regeln 1, 2, 3, 5, 6, 8, 9 und 10; Gruppe `addrsel` mit den
  Beispielen aus RFC 6724 10.2), im Einsatz im Resolver von JFNetworker.
  Seit W1 auch EAPOL ([`netpkt/eapol.rs`](../lib/netpkt/eapol.rs): der EAPOL-Key-Rumpf des
  4-Wege-Handshakes mit seinen Key-Information-Bits, welche der vier Nachrichten ein Frame ist,
  das MIC-Feld und was es abdeckt, RSN-Element, GTK- und IGTK-KDEs, Dissektor; Gruppe `netpkt`)
  und in [`crypto`](../lib/crypto/lib.rs) SHA-1, PBKDF2 über jeden Hash, der AES Key Wrap nach
  RFC 3394 und AES-CMAC nach RFC 4493 (Gruppe `crypto`), im Einsatz im WLAN von JFNetworker.
- **P2** · *erledigt* · Style `netlab` ([`styles/netlab`](../styles/netlab)) für JFNetworker:
  nüchterne graue Flächen, 1-DIP-Rahmen, kleine Radien, Auswahl als Akzent-Tönung mit Rahmen,
  klassische Dock-Register, kompakt (28-DIP-Controls, 22-DIP-Grid-Zeilen), Inter und eigene
  Vektor-Icons ([STYLES.md](STYLES.md)).
- **P3** · *erledigt* · Skriptsprache für JFNetworker ([`script`](../lib/script/lib.rs), seit
  2026-10-19, Meilenstein SK1): `.jfs`-Dateien, klein und dynamisch typisiert (JS/Lua-artig,
  Closures, `try`/`catch`, `??`, Interpolation `"${x}"`). Lexer mit `tokens()` für die
  Syntaxhervorhebung (deckt jedes Byte ab, scheitert nie), Pratt-Parser mit Fehlerkorrektur
  (weiter bei der nächsten Anweisung, ein Fehler pro Zeile, fehlende Klammern am Zeilenende
  werden ergänzt), Syntaxbaum mit Positionen (Zeile/Spalte), `check()` für den Editor
  (unbekannte Namen mit Vorschlag, doppelte Deklarationen, `break`/`continue`/`return` am
  falschen Ort; deutsche Meldungen), `outline()`; Verschachtelung begrenzt (`MAX_DEPTH`).
  Getestet mit `coretest --only script`. Seit 2026-10-20 (SK2) auch Compiler und VM
  ([GUIDE §29](GUIDE.md)): Stack-Bytecode mit Slots, Closures über Zellen, Ausnahmetabellen,
  Zeile und Spalte je Instruktion; VM mit 16-Byte-Werten, Arena-Heap mit Mark-and-Sweep, Fuel
  als virtuelle CPU-Zeit (`run(host, fuel)` → `Done | Yielded | Error`; ein Lauf in beliebig
  kleinen Stücken ergibt dasselbe wie ein ununterbrochener), harte Grenzen für Heap, Stapel,
  Aufruftiefe und Fuel pro Aufruf mit deutschen Meldungen samt Stacktrace, Float-Arithmetik in
  Software (`script::num`, bit-gleich auf allen Zielen), Host-Schnittstelle mit
  Native-Funktionen nach Namen, Methoden der eingebauten Typen und Rückrufen (`Cx::call`),
  `Vm: Clone + Send`. Getestet mit `coretest --only scriptvm` (auch Yield an jeder Instruktion
  gegen einen ununterbrochenen Lauf, Soft-Float gegen die Hardware, Windows und Linux). Seit
  2026-10-20 (SK3) mit Standardbibliothek ([GUIDE §29](GUIDE.md)), die jeder Host bekommt und
  die vor dem Host nach Namen gefunden wird (`s.split(",")` = `str.split(s, ",")`): `str`
  (Längen und Indizes in Zeichen, `slice`, `split`/`join`, `trim`, `find`, `replace`,
  `upper`/`lower` mit vollem Unicode, `format` mit Breite, Ausrichtung, Genauigkeit und
  Hex), `list` (`insert`, `remove`, `slice`, stabiles `sort` mit Vergleichsfunktion,
  `map`/`filter`/`reduce`, `contains`, `index`), `map` (`keys`/`values`/`entries` in
  Einfügereihenfolge, `has`, `get`, `delete`, `merge`), `bytes` (UTF-8, Hex, `slice`), `math`
  (`exp`, `log`, `log2`, `log10`, `pow`, `sin`, `cos` als fdlibm-Portierung auf `script::num`,
  unter 1 ulp, `exp`/`pow` bitgleich mit Javas `StrictMath`, exakte Argumentreduktion;
  Konstanten), `int.parse`/`float.parse`, `json.encode`/`decode` über `agent_core::json` mit
  Maps in Einfügereihenfolge und Fehlerposition. Fehler sind fangbare deutsche Meldungen;
  Fuel nach Arbeit (Text, Listen, Map-Einträge, Vergleiche), lange Operationen bezahlen
  unterwegs und brechen an der Fuel-Grenze mittendrin ab, Ergebnisse unterliegen der
  Heap-Grenze (große Texte werden vorab geprüft); Ausnahmen aus Rückrufen behalten ihre
  Position. `agent_core::json` prüft doppelte Schlüssel großer Objekte jetzt in O(n log n).
  Getestet mit `coretest --only scriptlib` (Windows und Linux mit gleichem Hash über die
  Mathe-Ergebnisse) und `tools/script_math_check.py --target both` (Genauigkeit gegen
  Pythons `decimal`, Windows gegen Linux Bit für Bit). Seit 2026-10-21 (SK4) laufen Skripte als
  Dienste in den Hosts von JFNetworker (`jfn_sim::scriptsvc`, App-README „Skripte in Hosts“):
  ereignisgesteuert über Rückrufe (UDP, TCP, HTTP-Server und -Client, DNS, Timer, Disk,
  Metriken), Fuel als Rechenzeit einer CPU mit einem Thread (`Cx::fuel()` nennt den Augenblick
  eines Aufrufs), Grenzen und Fehler mit Zeile und Spalte, der Befehl `script` in der
  Linux-Shell; getestet mit der Selbsttest-Gruppe `script` und dem Golden-Szenario
  `scen_script_kv.uis`. Seit 2026-10-22 (U7) bearbeitet der Reiter Skripte im Gerätefenster
  eines Hosts sie mit `ui.code_editor` (Prüfung mit `check()` beim Tippen, die Fehler des
  laufenden Programms mit Zeile und Spalte als Marken, F8), Übernehmen ist ein
  Rückgängig-Schritt, nach dem das laufende Skript Disk und Zahlen behält
  (`Action::ScriptSource`; App-README „Skript-Editor“).
- **P3** · *fehlt* · Eigenständiger jfs-Runner mit Konsolenausgabe (gewünscht am 2026-10-25):
  Skripte der Sprache aus `lib/script` auch außerhalb von JFNetworker ausführen, wie
  `python datei.py`. Ein CLI-Programm `jfs` (eigene App `apps/jfs` mit `rt::main!` wie
  `apps/permcli`, oder eine Zeile in einer `BINS`-Datei): `jfs DATEI.jfs [ARGS…]` übersetzt
  und startet ein Skript, `jfs --check DATEI.jfs` meldet nur die Diagnosen von `check()` mit
  Zeile und Spalte, `jfs -e "CODE"` und `-` (stdin) für Einzeiler; optional eine kleine REPL.
  Ein Konsolen-Host (`impl Host`): `print` nach stdout (`plat::write_stdout`), `log` nach stderr,
  `now` / `rand` über `plat`, `config` mit Kommandozeile und Umgebung, `timer` über eine
  Ereignisschleife mit echter Uhr oder virtueller Uhr (`--virtual-time`, für reproduzierbare
  Tests), `disk` in einer Datei (`--disk DATEI`, sonst im Speicher). Kein freier
  Dateisystem- oder Netzzugriff aus Skripten (die Sprache bleibt eine Sandbox); `net`, `dns` und
  `http` gehören JFNetworker und melden im Runner einen fangbaren Fehler. Grenzen per Flag
  (`--fuel`, `--heap`, `--depth`, sonst `Limits::default()`). Fehler beim Übersetzen oder
  Laufen erscheinen mit Meldung, Zeile:Spalte und Aufrufkette auf stderr, Exit-Code 1.
  Sprache und VM bleiben unverändert; in `lib/script` höchstens `compile_with(name, src,
  globals)`, damit unbekannte Namen gegen die Globals des jeweiligen Hosts geprüft werden (heute
  nur Warnungen). Dasselbe Skript läuft in JFNetworker und im Runner, solange es nur die
  Standardbibliothek, `print`, `log` und `timer` nutzt; Export und Import als `.jfs`-Datei im
  Skript-Editor gehören zu JFNetworker (SK5 im App-README). Tests: Beispielskripte mit
  erwarteter Ausgabe (stdout, stderr, Exit-Code) über `build_rs.py test`, Windows und Linux.
- **P3** · *fehlt* · Bausteine für den Webbrowser von JFNetworker (gewünscht am 2026-10-25;
  der Browser selbst mit den Meilensteinen B1 bis B4 steht im App-README unter „Fahrplan“).
  Der Browser zeigt Seiten mit `ui.html_view` (`md_doc`, `md_view`, `rich_html`) statt mit einer
  eigenen Engine. Dafür braucht die Library: (1) Bilder aus anderen Quellen: eine Rückfrage,
  über die die App die Bytes einer Adresse liefert (für B1; heute nur VFS und lokale Pfade,
  sonst der Alt-Text). (2) Formulare im Dokument (für B2): `<form>`, `<input>` (Text, Passwort,
  Häkchen, Radio, versteckt), `<textarea>`, `<select>`, `<button>` als bedienbare Elemente im
  Textfluss, ihre Werte im `MdViewState`, `MdEv` meldet Eingabe und Absenden mit den Feldern;
  ohne Formular bleibt die Ansicht lesend wie heute. (3) Ein veränderbares Dokument (für B3,
  Seitenskripte): Elemente in `md_doc` über id, Klasse oder Tag finden, Text, Attribute und
  Klassen ändern, Elemente einfügen und entfernen, danach neu setzen, ohne Bildlauf und
  Auswahl zu verlieren. (4) Später eine CSS-Teilmenge in `rich_html` (für B4): `<style>` mit
  Selektoren nach Tag, Klasse und id, die Eigenschaften, die heute aus `style=` gelesen werden,
  dazu Abstände und Rahmen. In `lib/script` nur `compile_with` (siehe den Runner), damit der
  Browser-Host seine eigenen Globals (`document`, `fetch` …) prüfen lässt. Bewusst nicht: eine
  vollständige HTML/CSS-Engine (Box-Modell, Floats, Flexbox, Grid) und JavaScript. Tests:
  coretest für Dokument-Änderungen, `.uis`-Skripte mit einem Formular (Windows und Linux).
- **P3** · *erledigt* · Befehlsbäume für Konsolen (seit 2026-10-19, für JFNetworker):
  [`base/cmdtree.rs`](../lib/base/cmdtree.rs) mit IOS-Abkürzungen (`sh ip int br`), `?`-Hilfe
  im IOS-Layout, Tab, Modi mit Prompt, `no`-Form, typisierten Parametern (Zahl mit Bereich,
  IPv4, Maske, Präfix, MAC, Wort, Rest der Zeile, Namen wie Interfaces von der App) und der
  Fehlerposition für `% Invalid input detected at '^' marker.`; getestet mit der
  coretest-Gruppe `cmdtree`, im Einsatz in der IOS-Konsole von JFNetworker. Seit R1 findet
  `find_mode` einen Modus über seinen Prompt (die Modi der Zugriffslisten von JFNetworker).

## 8. Rendering

- **P2** · *fehlt* · GPU-Rendering (D3D11 / Vulkan / OpenGL / WebGPU); heute rechnet alles
  die CPU.

## 9. Plattformen

- **P1** · *fehlt* · Native macOS-Version (Cocoa über `objc_msgSend` ohne libc ist eigener
  Aufwand); heute macOS nur über das Web.
- **P2** · *fehlt* · Native iOS-Version.
- **P2** · *erledigt* (2026-10-20) · Android vervollständigen: Dateiauswahl, dauerhafter
  Speicher, System-Zwischenablage ([VFS.md](VFS.md) „Android“, GUIDE §5, §7, §8,
  [apps/androidGallery/README.md](../apps/androidGallery/README.md)).
  - Dateiauswahl: `ui.filedlg` / `plat::file_dialog` öffnen das Storage Access Framework
    (`FD_OPEN` `ACTION_OPEN_DOCUMENT`, `FD_SAVE` `ACTION_CREATE_DOCUMENT`, neu `FD_FOLDER`
    `ACTION_OPEN_DOCUMENT_TREE`; Startort über `EXTRA_INITIAL_URI`). Der Picker antwortet
    später: `file_dialog` gibt `FD_PENDING`, `plat::file_dialog_poll` die Antwort, der Dialog
    wartet ohne zu zeichnen. Das Ergebnis ist ein `content://`-Name für alle Dateifunktionen
    (§2), die Freigabe wird dauerhaft genommen (`plat::web::content_grants` /
    `content_release`). `FD_FOLDER` auf dem Desktop: Linux-Portal mit `directory`, sonst der
    eigene Dialog nur mit Ordnern (auch Windows). `pick_file` liefert den Namen als Drop,
    `download` speichert eine Kopie über den Picker.
  - Dauerhafter Speicher: die virtuellen Ordner des Host-Adapters (`config://`, `data://`,
    `cache://`, `logs://`, `documents://`) sind echte Dateien in `filesDir` / `cacheDir`
    (`lib/plat/android_files.rs`, Bionic) statt Arbeitsspeicher: atomares Schreiben
    (Geschwisterdatei, fsync, rename), `flock`, `/tmp` beim Start geleert; `persist` ist
    sofort `Durable`. `documents://` bleibt der App-eigene Ordner (Androids geteilter
    Dokumente-Ordner ist nur über den Picker erreichbar).
  - System-Zwischenablage: Text und HTML über den `ClipboardManager`, Bilder als PNG über den
    App-Provider `GalleryFiles`; RTF / DOCX bleiben in der App, solange der Systemclip der
    eigene ist (Zeitstempel, ohne Fokus der Inhalt). Strg+V in Textfeldern fügt den Systemtext
    ein.
  - Getestet: `plattest --test content` (55 Prüfungen, Windows und Linux),
    `apps/androidGallery/tests/android_files.py` auf dem x86_64-Emulator (Android 15 / API 35):
    Phase 1 Selbsttest der App mit 76 Prüfungen und 8 der Instrumentation (der echte
    DocumentsUI-Picker per UiAutomation bedient: Ordner, Öffnen, Speichern), Neustart, Phase 2
    mit 15 + 6 (Dateien und Freigaben nach dem Neustart, Einfügen und Kopieren im Textfeld);
    `GalleryInstrumentation` (78) und `GalleryA11yInstrumentation` (30) weiter grün.
  - Es fehlen: atomares Schreiben auf `content://`, ein Namens-Cache in Bäumen, Mehrfachauswahl,
    Antworten des Pickers nach einem Prozessende, Papierkorb und `plat::fs` auf Android, Tests
    auf einem echten Gerät (dort nur die arm64-APK gebaut) und mit Cloud-Anbietern, die über
    Pipes liefern; die Zwischenablage liest nur mit Fensterfokus (Androids Regel).
- **P2** · *fehlt* · Safari / iOS im Web-Testplan.

## 10. Auslieferung und Updates

Vorhanden: `.deb`, Windows-Setup ([`plat/win_inst.rs`](../lib/plat/win_inst.rs),
[`tools/package.py`](../tools/package.py)), Absturzhandler. Seit 2026-10-17 auch der Updater
im Framework und die Code-Signierung, seit 2026-10-18 AppImage, Flatpak, MSIX und RPM-.spec.

- **P1** · *erledigt* · Auto-Updater im Framework mit Signaturprüfung und Rollback
  ([`lib/update`](../lib/update/lib.rs), [UPDATES.md](UPDATES.md)): signiertes `update.json`
  (Ed25519 über die Bytes, öffentlicher Schlüssel in der App) mit Version, Ziel-OS/Arch,
  Adresse, Größe und SHA-256; Prüfung von App-ID, Ablaufdatum, Downgrade und Replay (nie
  älter als ein schon gesehenes Manifest). Download über `netio` im Hintergrund mit Fortschritt,
  Abbruch und Fortsetzen per `Range`; die `net.connect`-Regeln der App gelten auch für
  Umleitungen. Bereitgestellt unter `data://update`, beim nächsten Start eingesetzt (Dateien
  als `.jfupd-new` geschrieben, alte gesichert und weg-umbenannt, Neustart über
  `plat::restart_self`). Die neue Version läuft auf Probe bis `update::healthy()`; sonst stellt
  der nächste Start die alte wieder her und sperrt die Version. Arten `exe`, `bundle` (Programm
  mit Modulen) und `package` (Installer, nur geprüft bereitgestellt).
  `tools/update_manifest.py` (Ed25519 in reinem Python) bzw. `package.py --update` schreiben
  und signieren den Kanal; der Schlüssel kommt aus Datei oder Umgebung. Getestet mit
  `apps/updatetest` gegen `tools/update_server.py`, auch mit echten Prozessen (Neustart,
  Rollback) unter Windows und Linux. Es fehlen noch: Delta-Updates, ein Wächter-Prozess für
  Versionen, die vor `startup()` abstürzen oder hängen; automatischer Health-Marker über die
  Lebenszyklus-Hooks (§7).
- **P1** · *erledigt* · Code-Signierung ([`tools/codesign.py`](../tools/codesign.py),
  [PACKAGING.md](PACKAGING.md) „Signing“): Authenticode mit signtool für Programme, Module und
  Setup (der Installer findet seine Nutzlast vor der Signatur, `uninstall.exe` ohne
  Signaturverweis); signiertes apt-Repository (`InRelease`, `Release.gpg`, `repo.gpg`) mit gpg;
  Android `--release` mit Release-Schlüssel aus der Umgebung statt Debug-Key. Ohne
  Werkzeug/Schlüssel meldet `package.py`, was fehlt (`--sign`: Fehler). Ohne echte Zertifikate
  geprüft mit `tools/sign_check.py`: selbstsigniertes Test-Zertifikat (signtool, Installation
  des signierten Setups), Wegwerf-GPG-Schlüssel (gpgv), Test-Keystore (apksigner). Es fehlen
  noch: Zeitstempel (nicht getestet), Signatur im `.deb` selbst (apt prüft sie nicht), AAB /
  Play App Signing.
- **P2** · *teilweise* · Weitere Paketformate (`package.py --formats`, [PACKAGING.md](PACKAGING.md)
  „Other formats“): **AppImage** je Stil mit eigener Runtime
  ([`apps/appimagert`](../apps/appimagert/src/main.rs): statisch, ohne libc und ohne FUSE, liest
  das SquashFS selbst (zstd/lz4), entpackt beim ersten Start nach `$XDG_CACHE_HOME/appimage-run`
  und startet AppRun über das neue `plat::process::exec`; Typ-2-Magie und `.upd_info` setzt
  `package.py`), Abbild über mksquashfs bzw. appimagetool; **Flatpak**-Manifest mit finish-args
  aus `PERMISSIONS` (Portale statt Löcher, wo plat sie nutzt), Bundle über flatpak-builder;
  **MSIX** mit AppxManifest (Identität, Logos, Dateitypen, Protokolle, StartupTask, Fähigkeiten),
  MakeAppx und signtool; **RPM**-.spec mit allen Stilen als `update-alternatives`. In MSIX meldet
  `autostart_set` jetzt `Unavailable` (HKCU ist dort virtualisiert). Geprüft mit
  [`tools/pkg_check.py`](../tools/pkg_check.py) (die Suite ruft `--quick`): Validatoren
  (desktop-file-validate, appstreamcli, MakeAppx), Runtime gegen mksquashfs-Abbilder, Template als
  AppImage und im Flatpak-Sandkasten gestartet, MSIX mit Test-Zertifikat signiert. Es fehlen
  noch: ein gebautes RPM (rpmbuild fehlt im Testsystem: nur .spec und Tarball), AAB für Google
  Play (bundletool fehlt; die Schritte stehen in PACKAGING.md), StartupTask aus der App (WinRT),
  Updates über `lib/update` für AppImage, Flatpak und MSIX, Signaturen für AppImage, RPM und
  Flatpak, eine Installation der Pakete im Test; später Notarisierung für macOS.
- **P2** · *erledigt* · Versionsverwaltung (2026-10-20, [VERSIONING.md](VERSIONING.md)): die
  Library hat eine Version (`lib/VERSION`, `ui::VERSION`), jede App ihre eigene (`version=` in
  `PACKAGE`, im Code `rt::app_version!()`, auch für `update::Config`); SemVer-Regeln,
  `build_rs.py api-diff` und `release`, `CHANGELOG.md` (Details in Abschnitt 12).

## 11. Barrierefreiheit

Basis vorhanden: `sem()`-Records (semantischer Widget-Baum), Fokusrahmen, Mnemonics. Seit
2026-10-17 der Barrierefreiheits-Baum für Screenreader: [`ui/a11y.rs`](../lib/ui/a11y.rs) baut
aus den Records jedes Frames Fenster → Knoten mit Rolle, Name, Zustand, Wert und Rechteck,
[`plat/a11y.rs`](../lib/plat/a11y.rs) veröffentlicht ihn, leitet Ereignisse aus den
Unterschieden ab und nimmt Aktionen entgegen, die der nächste Pass über das normale
Widget-Verhalten ausführt ([GUIDE.md](GUIDE.md), Abschnitt 19).

- **P1** · *erledigt* · Die Records abbilden auf UIA (Windows), AT-SPI (Linux, D-Bus),
  `AccessibilityNodeInfo` (Android) und einen ARIA-Schattenbaum im DOM (Web).
  Linux über AT-SPI2 ([`plat/linux_atspi.rs`](../lib/plat/linux_atspi.rs):
  Accessibility-Bus, Registry-Embed, Accessible / Component / Action / Value / Text /
  EditableText, Ereignisse für Fokus, Zustände, Namen, Werte, Text und Kinder) und Windows
  über UI Automation ([`plat/win_uia.rs`](../lib/plat/win_uia.rs): Provider für
  `WM_GETOBJECT`, Fragment-Baum, Invoke / Toggle / Value / RangeValue / SelectionItem /
  ExpandCollapse / Text / Text2, Fokus-, Eigenschafts-, Text- und Strukturereignisse).
  Seit 2026-10-19 Browser und Android: beide Hosts halten einen Spiegel des Baums, den die
  Library nach jedem Frame mit den Unterschieden füttert
  ([`plat/a11y_wire.rs`](../lib/plat/a11y_wire.rs): neue / geänderte Knoten, Reihenfolge bei
  Strukturänderungen, Ereignisse; [`plat/web_a11y.rs`](../lib/plat/web_a11y.rs): Bridge,
  `jfgui_a11y` / `jfgui_a11y_act`). Web ([`web/a11y.js`](../web/a11y.js)): unsichtbare
  Elemente über dem Canvas an den Knotenrechtecken mit ARIA-Rollen, Namen, Zuständen und
  Werten, echte `<input>` / `<textarea>` mit Text und Auswahl, `<input type=range>` für
  Slider; der DOM-Fokus folgt dem App-Fokus, sobald Tastatur oder Screenreader steuern (ein
  Zeiger gibt die Tastatur an das versteckte `#input` zurück); Klick, Fokus, Wert, Text und
  Auswahl aus dem DOM gehen als `a11y::Act` zurück; Updates laufen vom Worker als
  Service-Anfrage 11 zur Seite; `?a11y=0` schaltet ab. Android
  ([`android/GalleryAccessibility.java`](../android/GalleryAccessibility.java)):
  `AccessibilityNodeProvider` mit virtuellen Views (Klassen, Text, Hinweis, Zustände,
  `RangeInfo`, Auswahl, Bildschirm-Rechtecke), Aktionen Klick / Fokus / Accessibility-Fokus /
  Fortschritt setzen / Scrollen / Text setzen / Auswahl, Explore-by-Touch (Hover), Ereignisse
  für Fokus, Text, Auswahl, Zustand, Wert, Struktur und Dialoge; an- und ausgeschaltet mit
  dem Accessibility-Schalter des Systems. Tests: `plattest --test a11y` (beide Suites, auch
  das Update-Format), `tools/platshared/atspi_bus.py` (Fake-Bus und echtes at-spi2-core über
  libatspi), `plattest --test uia` (IUIAutomation, von Hand),
  `apps/androidGallery/tests/a11y_web.mjs` (Chrome headless über das lokale Playwright:
  Chromiums eigener Accessibility-Baum per DevTools-Protokoll, Klicks, Slider, Tab-Fokus,
  Tippen, Auswahl, Ersetzung, Zeiger, `?a11y=0`; 33 Prüfungen, seit 2026-10-22 mit
  Überschriften 36) und `GalleryA11yInstrumentation` (x86_64-Emulator, Android 15 / API 35:
  über UiAutomation, die Client-Seite von TalkBack – Baum, Aktionen, Ereignisse,
  Accessibility-Fokus, Hover mit Touch-Exploration; 30 Prüfungen, seit 2026-10-22 31; dazu
  `uiautomator dump`). Von Hand geprüft
  (2026-10-19): der Schattenbaum im eingebauten Browser der Desktop-App (Baum, Klicks,
  Slider, Fokus, Text). Offen: mit echten Screenreadern (Orca, Narrator, NVDA, JAWS,
  VoiceOver, TalkBack) noch nicht ausprobiert – auf dem vorhandenen Emulator-Image ist
  TalkBack nicht installiert (bräuchte ein Google-APIs-Image oder die TalkBack-APK);
  Touch-Exploration mobiler Browser über den Schattenbaum (der keine Zeiger-Ereignisse
  annimmt) ungetestet; IME-Kandidatenfenster liegt bei Tastaturfokus im Schatten-Textfeld nur
  ungefähr am Caret (Zeilenhöhe übernommen, Spalte nicht); Live-Regionen für Meldungen gibt es
  nicht (der Baum kennt keine).
- **P2** · *erledigt* · Überschriften (2026-10-22): `K_HEADING`-Records (die Überschriften des
  Markdown-/HTML-Betrachters, neu `ui.heading(level, style, text)` für Abschnittstitel) werden
  `Role::Heading` mit Ebene (`Node::value`, `Node::level()`; vorher eine Beschriftung), damit
  die Überschriften-Navigation der Screenreader (H / 1–6 in NVDA, JAWS, Orca, Narrator,
  TalkBack) sie findet. UIA: Steuerelementtyp Text mit `HeadingLevel1`–`9`, dazu AriaRole
  `heading` und AriaProperties `level=N`, Eigenschaftsereignis bei neuer Ebene; AT-SPI2: Rolle
  `heading` mit Objektattribut `level:N` (`GetAttributes`, `object:attributes-changed`); Web:
  `role="heading"` mit `aria-level` im Schattenbaum; Android: `setHeading(true)` ab API 28,
  darunter (API 26 / 27) das Heading-Flag einer `CollectionItemInfo` (Android kennt keine
  Ebene). Ein Textfeld, Slider usw. direkt unter einer Überschrift nimmt wie bei einem Text
  deren Namen. Die Abschnittstitel der androidGallery sind Überschriften. Tests:
  `plattest --test a11y` (Rolle, Ebene, Diff, Rollennummer im Update-Format), `--test uia`
  (HeadingLevel über IUIAutomation), `atspi_bus.py` (Fake-Bus und libatspi: Rolle, `level:2`),
  `a11y_web.mjs` (Chromiums Baum: heading, level 2), `GalleryA11yInstrumentation`
  (`isHeading()`), `mdviewtest --test unit` (die Überschriften des Betrachters im Baum); am
  2026-10-22 alle grün (`--test a11y` 56, `--test uia` 50, `atspi_bus.py --real` 78,
  `a11y_web.mjs` 36, `GalleryA11yInstrumentation` 31 auf dem API-35-Emulator, `--test unit`
  152). Offen:
  Überschriften im Dokument des Rich-Text-Editors (ein Textfeld: bräuchte Text-Attribute, UIA
  `StyleId_Heading1`… / AT-SPI-Textattribute), `nav_section` (Gruppentitel der Navigation, als
  Text gelassen: Skripte wählen ihn als `text:`), die Ebene unter Android (keine API) und der
  API-26-Weg (nur auf einem API-35-Emulator getestet); mit echten Screenreadern nicht
  ausprobiert.
- **P2** · *erledigt* · Rollennamen in der UI-Sprache (2026-10-22): UIA bekam für Dialoge,
  Schalter und Überschriften fest „Dialog“, „Umschalter“, „Überschrift“ als
  `LocalizedControlType`, AT-SPI2 für jede Rolle englisch („push button“) als
  `GetLocalizedRoleName`. Jetzt hält `plat::a11y::ROLE_TEXTS` die Namen aller Rollen (dazu
  Fenster, Anwendung, unbekannt) deutsch als Schlüssel; ui reicht der Plattform beim ersten
  Frame `i18n::tr` (`plat::a11y::set_translator` – plat darf ui nicht kennen), `role_name`
  liefert sie in der UI-Sprache und folgt einem Sprachwechsel sofort; die Texte stehen in
  `LIB_TEXTS` in allen neun Sprachen (Englisch wie AT-SPI sie nennt). UIA nennt die übrigen
  Steuerelementtypen weiter selbst (in der Sprache des Systems); `GetRoleName` bleibt englisch.
  Web (ARIA-Rollen, die der Browser benennt) und Android (Klassennamen, die TalkBack benennt)
  hatten keine festen Texte. Tests: `plattest --test a11y` (deutsch, englisch nach Wechsel),
  `--test uia` (`LocalizedControlType` von Schalter und Überschrift deutsch, nach Wechsel
  englisch und französisch, eine Schaltfläche behält den von UIA), `atspi_bus.py` (ein zweiter
  Lauf mit `--lang en`), `i18ntest` (jeder Rollenname ein Bibliothekstext, in de / en / fr / he);
  am 2026-10-22 grün: `--test a11y` 59, `--test uia` 54, `atspi_bus.py --real` 136, `i18ntest`
  2514 (Windows und Linux). Offen: `web/index.html` hat fest `lang="de"` (die Seite folgt nicht der UI-Sprache); weitere
  feste deutsche Texte in plat (Titel der Linux-Dateidialoge über das Portal, Fehlermeldungen von
  Diensten, Zertifikaten, Prozessen) laufen noch nicht über eine Übersetzung.
- **P1** · *erledigt* · Theme mit hohem Kontrast ([`ui/theme.rs`](../lib/ui/theme.rs)
  `contrast_tokens`): alle Tokens aus dem Kontrastschema – unter Windows die Systemfarben,
  sonst Weiß auf Schwarz bzw. Schwarz auf Weiß –, deckende Flächen, Ränder in Textfarbe,
  Akzente in der Hervorhebungsfarbe, Links in der Linkfarbe; folgt dem System
  (`SPI_GETHIGHCONTRAST`, Portal `contrast`, GNOME `high-contrast`) oder `--contrast high` /
  `ui.set_contrast`.
- **P2** · *erledigt* · „Animationen reduzieren“ folgt dem System, Textskalierung getrennt von
  der UI-Skalierung (`ui.set_text_scale`, `--text-scale`, System-Wert). Cursorposition für
  Bildschirmlupen (2026-10-18): Textfelder melden ihren Caret (`ime_pos` →
  `Node::caret_rect`, Auswahl `Node::anchor`, `char_rect` / `offset_at`: am Caret exakt,
  daneben geschätzt). Windows: versteckter System-Caret im Fenster mit der Tastatur (vorher
  immer im Hauptfenster; Lupe, ZoomText über `GetGUIThreadInfo` und die Caret-Ereignisse),
  UIA Text- / Text2-Pattern ([`plat/win_uia_text.rs`](../lib/plat/win_uia_text.rs): Bereiche,
  `GetCaretRange`, Auswahl, `Select`), Ereignisse `TextSelectionChanged` / `TextChanged`.
  Linux: `CaretOffset`, `GetCharacterExtents` in Bildschirmkoordinaten,
  `object:text-caret-moved` auch beim Fokuswechsel ins Textfeld, `SetCaretOffset` /
  `SetSelection` / `GetSelection`. Aktion `a11y::Act::Select`. Tests: `plattest --test a11y`
  (headless), `--test uia` und `--test caret` (Windows, echtes Fenster: System-Caret und
  TextPattern2 stimmen überein und folgen), `tools/platshared/atspi_bus.py` (Fake-Bus und
  `--real` mit libatspi). Offen: Zeichenrechtecke abseits des Carets nur geschätzt
  (umbrochene Zeilen unbekannt); mit Windows-Lupe, NVDA, Orca und der GNOME-Lupe noch nicht
  von Hand ausprobiert. Browser / Android: das Schatten-Textfeld bzw. die virtuelle View trägt
  Text und Auswahl, das Caret-Rechteck nur indirekt (Browser: über die Lage des Feldes).
- **P2** · *erledigt* · Vollständige Tastaturbedienung (2026-10-20,
  [`ui/kbnav.rs`](../lib/ui/kbnav.rs), [GUIDE.md](GUIDE.md) Abschnitt 27). Vorher: Tab /
  Umschalt+Tab über alle Fokus-Boxen, Fokusrahmen, Menüleiste mit Alt / F10 / Mnemonics,
  Beschleuniger, Listen-Tasten. Neu: **Fokusgruppen** (ein Tab-Stopp, Pfeile innen, Pos1 / Ende;
  Radios einer Variablen mit Auswahl, die dem Fokus folgt, Werkzeugknöpfe einer Variablen,
  Farbfelder einer Box, `focus_group_begin` für Werkzeugleisten und Knopfraster; eine Zeile oder
  Spalte läuft um, ein Raster geht zum nächsten Nachbarn), **Bereiche** und **Fenster** (F6 /
  Umschalt+F6 über die Bereiche eines Fensters – Dock-Gruppen, Dokumentbereich, beide Seiten
  einer Split-Ansicht, `focus_pane_begin` – und weiter zum nächsten Fenster; Strg+F6 direkt zum
  nächsten Fenster; Toolfenster leihen sich dann die Tasten des Hauptfensters bis Esc, F6 oder
  Klick; Tab, F6 und Pfeile in Gruppen bleiben im Toolfenster statt an den Besitzer zu gehen),
  **Splitter** per F8 (Split-Ansicht, Dock-Seiten und -Gruppen; Pfeile 8 DIP, Umschalt 1 DIP,
  Pos1 / Ende, Enter / Esc zurück; mit Datensatz „Teiler“), **Menütaste / Umschalt+F10** überall
  (ohne `context_menu` als Rechtsklick des fokussierten Widgets, `menu_popup` öffnet dann am
  Widget; in Listen, Rastern und Bäumen für die ausgewählte Zeile), **Listen**: Enter =
  Doppelklick, Leertaste = Kontrollkästchen der Zeile, `LF_REORDER` mit Strg+Pfeil / Strg+Pos1 /
  Strg+Ende, Widgets in Zeilen nur in der ausgewählten Zeile als Tab-Stopp (Raster und Bäume:
  Leertaste / Enter gingen vorher im Fokus-Verhalten verloren); **Fokus**: Dialogfenster
  starten auf dem ersten Steuerelement (Esc schließt trotzdem sofort), Tab nach einem
  verschwundenen Fokus-Widget geht an seiner Stelle weiter, Kontrollkästchen / Radios / Schalter
  nehmen nur die Leertaste (Enter gehört der Standardschaltfläche), die Tab-Stopps eines
  Bereichs bleiben zusammen (Dock: Tab-Leiste, Knöpfe, Inhalt), Strg+Tab bleibt der App;
  Farbfelder und die Teile des Farbwählers (Spektrum, Farbton, Deckkraft) sind fokussierbar.
  PaintCraft: Ebenenfenster (Pfeile wählen, Strg+Auf / Strg+Ab / Strg+Pos1 / Strg+Ende
  verschieben, Leertaste Sichtbarkeit, Enter Eigenschaften mit Deckkraft, Menütaste Ebenenmenü,
  Esc zurück), Werkzeug-Raster, Farbpalette (16 × 6 mit Pfeilen, Menütaste = Sekundärfarbe),
  Primär- / Sekundärfarbe, Bild-Tabs, Titel- und Werkzeugleiste als Gruppen (Haupt-Tab-Ring
  18 statt 30 Stopps), Strg+F6 durch Werkzeuge / Verlauf / Ebenen / Farben (F5 – F8 bleiben
  paint.net-Fensterschalter). Werkzeug `tools/kbwalk.py` (Tab-Ring per Skript abgehen), Dump mit
  `"keys"` / Fenster-`"focus"`, Skript `expect window ID keys`. Tests: `gallery/tests/keyboard.uis`,
  `PaintCraft/tests/keyboard.uis`, `docktest/tests/kbnav.uis`. Offen: Zugriffstasten
  (Alt+Buchstabe) für Schaltflächen und Beschriftungen in Dialogen (heute nur Menüs);
  Toolfenster und Dialogfenster mit Bibliotheksrahmen per Tastatur verschieben / in der Größe
  ändern (Systemrahmen: Alt+Leertaste des Systems); der Schließen-Knopf der Bild-Tabs (Strg+W
  reicht); mit echten Fenstern unter Windows / X11 / Wayland nur headless geprüft (das Ausleihen
  der Tasten durch Toolfenster ist dort derselbe Weg wie beim Klick, U3).

## 12. Entwickler-Erlebnis

- **P2** · *fehlt* · Cargo-Integration bzw. Veröffentlichung als Crates; heute baut nur
  `build_rs.py` mit `rustc`, fremde Crates sind schwer einzubinden.
- **P2** · *erledigt* · Projekt-Generator (`build_rs.py new NAME`), stabile API mit SemVer,
  Changelog (2026-10-20). `python build_rs.py new NAME --kind window|console|tool|service
  [--style S] [--title T]` ([`tools/scaffold.py`](../tools/scaffold.py)) legt `apps/NAME` an:
  `src/main.rs` (Fenster mit Befehlstabelle, Menü, Info-Dialog und Statusleiste; Konsole mit
  `--check`; Werkzeug als Fenster + Kommandozeilenprogramm in `BINS` + app-lokale Crate; Dienst
  mit Arbeitsschleife, `plat::service` und `--once`), `TARGETS`, `STYLE`, `PERMISSIONS` (nötige
  Regeln aktiv, weitere als Kommentar), `PACKAGE` (`version=0.1.0`, `app_id` und `homepage` aus
  dem Git-Remote, z. B. `io.github.<owner>.<App>`), `tests/smoke.uis`, `README.md`; verweigert
  vorhandene Namen (auch in anderer Schreibweise), ungültige Bezeichner, Crate-Namen und
  Schlüsselwörter. `new --lib NAME --deps rt,plat` legt `lib/NAME/src/lib.rs` an, trägt die
  Crate in `LIBS` und `tools/gendoc.py` ein und erzeugt API.md neu. `build_rs.py test NAME` baut
  eine App und führt nur ihre Skripte aus. Alle vier Arten unter Windows und Linux gebaut und
  getestet; `apps/scaffoldtest` ist die Ausgabe der Art `tool` und läuft in der Suite mit
  (`tools/tests/test_devkit.py` prüft, dass der Generator genau sie erzeugt). Versionen
  ([VERSIONING.md](VERSIONING.md)): eine Bibliotheksversion in `lib/VERSION` (`rt::VERSION`,
  `plat::VERSION`, `ui::VERSION`, zur Übersetzungszeit als SemVer geprüft), die App-Version
  bleibt `version=` in `PACKAGE` und kommt als `rt::app_version!()` in jedes Programm der App;
  SemVer-Regeln für die öffentliche API (was öffentlich ist, was welche Stelle erhöht,
  0.x wie bei Cargo, Abkündigung mit `#[deprecated]`). `build_rs.py api-diff`
  ([`tools/apidiff.py`](../tools/apidiff.py)) vergleicht die öffentlichen Items (wie in API.md:
  Pfade über `pub use`, Signaturen ohne Parameternamen, Felder, Varianten, Trait-Mitglieder,
  Derives) mit dem Schnappschuss der letzten Version (`docs/api/0.1.0.txt`) und meldet
  Entferntes, geänderte Signaturen, neue Varianten erschöpfender Enums, neue Pflichtmethoden
  öffentlicher Traits als inkompatibel; die Suite zeigt eine Zusammenfassung. `CHANGELOG.md`
  (Keep a Changelog, 0.1.0 aus der Git-Historie nach Roadmap-Bereichen), `build_rs.py changelog`
  fügt Einträge an, `build_rs.py release X.Y.Z` schneidet die Version (Changelog,
  `lib/VERSION`, Schnappschuss; verweigert zu kleine Sprünge). Es fehlen noch: Prüfung der
  Nicht-Rust-Verträge (`PERMISSIONS`-Syntax, `PACKAGE`-Schlüssel, Skriptsprache) bei einer
  Version, Git-Tags und Veröffentlichung (bewusst Handarbeit), eine Versionsprüfung beim
  Bauen fremder Apps gegen eine Mindestversion der Library.
- **P2** · *teilweise* · Website und Dokumentation für die Library. Generator erledigt
  (2026-10-21, [`tools/site.py`](../tools/site.py), nur Python-Standardbibliothek):
  `python build_rs.py site [--out DIR]` schreibt eine statische Website nach `build/site/`, die
  auch ohne Server aus dem Dateisystem läuft. Startseite aus den Abschnitten der README
  (Einführung, Funktionen als Karten, Feature-Tour mit den Screenshots, Plattformen,
  Schnellstart, Dokumentation, Lizenz); als Seiten README, die Dokumente aus `docs/`, die
  App-READMEs, Changelog, Roadmap (als deutsch markiert, wie alle deutschen Seiten) und
  Drittanbieter-Hinweise; GUIDE.md mit einer Seite pro Abschnitt (Vor/Zurück), API.md mit einer
  Seite pro Crate (große Crates pro Abschnitt bzw. Modul) und einem Anker für jedes Item
  (`#fn.run`, `#Ui.text`). Die GitHub-Anker der Überschriften gelten weiter, Links auf `.md`
  werden zu Seite und Anker, andere Dateien und Ordner des Repositorys zu GitHub-Links.
  Navigation an der Seite (am Telefon als Schublade), Kopfleiste mit Suche über Titel,
  Überschriften und API-Items, helles und dunkles Design (System oder Schalter, gemerkt in
  localStorage), Rust-Code farbig, keine fremden Schriften oder CDNs. Der Markdown-Konverter
  kann genau das, was die Dokumente nutzen (Tabellen, Code-Blöcke auch in Listen, verschachtelte
  Listen, `<img>`, `<br>`, `<a id>`); anderes HTML erscheint als Text.
  `python build_rs.py site --check` (läuft in jeder Suite, einige Sekunden) baut in einen
  temporären Ordner und prüft interne Links und Anker, Bilder, Links auf Repository-Dateien
  (auch deren Anker), die HTML-Verschachtelung, Titel, Beschreibung und Sprache jeder Seite,
  dass kein lokaler Pfad in der Ausgabe steht und dass zwei Läufe byteweise gleich sind.
  Es fehlt die Veröffentlichung, bewusst offen (Entscheidung des Maintainers): etwa GitHub
  Pages über einen Workflow (bei jedem Push auf `main`: `python build_rs.py site`, dann
  `actions/upload-pages-artifact` mit `build/site` und `actions/deploy-pages`; in den
  Repository-Einstellungen Pages mit der Quelle „GitHub Actions“), ein `gh-pages`-Zweig oder ein
  beliebiger statischer Host; dazu wahlweise eine eigene Domain und eine Fassung pro Release.
- **P3** · *teilweise* · Hot-Reload, Live-Inspector bzw. Debug-Overlay (`--dump` als Basis).
  Live-Inspector erledigt (2026-10-20, [GUIDE.md §30](GUIDE.md), `lib/ui/inspect.rs`): aus
  ohne Kosten und ohne Einfluss auf die Bilder (byteweise gleich), an mit `--inspect`,
  `jfgui_INSPECT=1` (nur Tastenkürzel) oder `ui.inspect_enable` / `ui.inspect_show`, umgeschaltet
  mit Strg+Umschalt+I und F12 (`ui.inspect_keys`). Overlay in allen Fenstern der App: Widget unter
  dem Zeiger mit Rechteck und Etikett (Art, Kennung hex mit dem Namen aus `ui.id`, Beschriftung,
  Größe, Zustand), Auswahl, wahlweise alle Layout-Boxen, Tab-Reihenfolge und die neu gezeichneten
  Rechtecke der letzten Bilder (verblassend); es liegt außerhalb der Befehlslisten, die
  Dirty-Regionen der App bleiben ihre eigenen, `--verify-dirty` prüft es mit. Werkzeugfenster
  „Inspektor“: Baum des aktuellen Durchgangs (semantische Einträge in ihren Layout-Boxen,
  einklappbar, Auswahl hebt in der App hervor), Eigenschaften (Rechteck in DIP und px, Kennung,
  Art, Beschriftung, Wert/Min/Max, Zustand, Ebene, Fenster), Skript-Selektor (`kind:"Label"` /
  `#id`) mit Kopieren und Strg+C, Bildstatistik (Durchgänge, Aufbau- und Zeichenzeit,
  neu gezeichnete Fläche, Zeichenbefehle, Glyphen-Cache), Theme-Farben, Auswählen per Klick
  (der Klick erreicht die App nicht, Esc beendet). Das eigene Fenster fehlt in Baum, Overlay und
  Auswahl; es stiehlt keinen Fokus. Skriptbefehle `inspect …` (u. a. `expect same` / `idle`
  für „aus ändert nichts“), Tests `apps/gallery/tests/inspect.uis` und `inspect_rtl.uis`
  (Windows und Linux, auch mit `--verify-dirty`). Es fehlt: Bearbeiten im Inspektor (nächster
  Eintrag); Hot-Reload: Stufe 1 des nächsten Eintrags erledigt (2026-10-26).
- **P2** · *teilweise* · Hot-Reload und beschreibbarer Inspektor („Live-Edit“, wie das F12-Menü
  eines Browsers), als Debug-Variante. Festgelegt am 2026-10-25: (a) Eine Art Hot-Reload: Code
  ändern, die laufende App zeigt die Änderung; die Elemente bleiben gebauter Rust-Code, nichts
  wird zur Laufzeit beschrieben oder interpretiert. (b) Nur als Debug-Variante und nur wenn
  möglich: im Entwicklungs-Build (Standard von `build_rs.py`), nicht in `--release`. (c) Es wird
  nichts umgebaut, damit der Inspektor veränderlicher wird: keine neue Pflicht-API in den Apps,
  keine vorgeschriebene Zustandsstruktur, kein Plugin-Umbau. Grundgrenze: immediate-mode, die
  Werte von Slider, Häkchen und Textfeld gehören der App, der Inspektor schreibt sie nicht
  direkt. Stufen, jede einzeln nützlich: (1) Hot-Reload (`build_rs.py watch NAME`): beobachtet
  die Quellen (Dateibeobachter `lib/plat/watch.rs` prüfen), baut beim Speichern inkrementell und
  startet die App neu; wiederhergestellt werden Fensterposition und -größe, Seite, Skalierung,
  Stil und der Inspektor mit seiner Auswahl; den App-Zustand holt das Abspielen des mit
  `--log-events` protokollierten Eingabestroms zurück (virtuelle Uhr, wie ein `.uis`-Skript),
  ohne Änderung am App-Code, bei Abweichung startet die App frisch; schlägt der Bau fehl, läuft
  die alte Fassung weiter und der Fehler erscheint im Inspektor. (2) Werte im Inspektor ändern,
  nur was es schon gibt: Tab „Farben“ editierbar (`T_*`-Tokens mit Farbwähler), Eckenradius,
  Abstände, Höhe der Steuerelemente, Textgrößen und Schrift, Skalierung, hell/dunkel/Kontrast,
  Akzent, Stil und Sprache, über das vorhandene App-Theme (`ui.set_theme`,
  [GUIDE.md §34](GUIDE.md); `apps/themedemo` hat einen Live-Editor); das ausgewählte Widget
  bedienen (ziehen, setzen, tippen, `cmd ID`) über dieselben synthetischen Eingaben wie die
  `.uis`-Skripte (`lib/ui/auto.rs`); geänderte Werte markiert, einzeln zurücksetzbar,
  Rückgängig/Wiederholen (`Snapshot`). (3) Übernehmen: als `.theme`-Datei speichern oder die
  Änderungen als Liste kopieren; Layout und Texte ändert man im Code, das zeigt Stufe 1.
  (4) Optional Quelltext öffnen: Datei und Zeile des Widget-Aufrufs per `#[track_caller]`, nur
  im Entwicklungs-Build bei eingeschaltetem Inspektor, „Im Editor öffnen“. Bewusst nicht:
  Debug-Variablen per `ui.inspect_var`, App-Logik als nachladbares Plugin, Zustand als `Ini`
  sichern (alle verlangen Änderungen an den Apps). Bleibt wie bisher: aus kostet nichts und
  ändert kein Bild (`inspect expect same` / `idle`); Änderungen im Inspektor gelten nur für die
  laufende Sitzung, bis man sie übernimmt. Tests: Skriptbefehle `inspect set …` /
  `inspect reset`, `apps/gallery/tests/inspect*.uis`.
  Stufe 1 erledigt (2026-10-26, [GUIDE.md §30](GUIDE.md) „Hot reload“, `tools/hotreload.py`,
  `lib/ui/hotreload.rs`): `build_rs.py watch NAME [--target win|linux] [-- ARGS]` beobachtet die
  Quellen der App und aller Bibliotheks- und Stil-Crates, die sie linkt (Änderungszeiten per
  `os.scandir` je Ordner abgefragt; `lib/plat/watch.rs` ist Rust und passt nicht in das
  Python-Werkzeug), baut beim Speichern den Entwicklungs-Build und startet die App als Kopie
  `NAME.hot[.exe]` in ihrer Sitzung `build/<t>/hot/NAME` neu. Nur im Entwicklungs-Build für Windows
  und Linux (`--cfg jfgui_dev` für `ui`) und nur mit `jfgui_HOTRELOAD`: die App zeichnet ihre
  Eingaben auf (die Felder von `--log-events` mit Zeit, dazu Fenstergröße und dpi, `UIC_ACTIVE`
  und die Antworten der Zwischenablage); der nächste Start spielt sie vor dem ersten Warten ab
  (virtuelle Uhr: Timer zu ihren Fristen, Animationen springen), Zwischenablage, Dateidialoge,
  Töne und andere Programme des Systems bleiben dabei unberührt. Prüfung: jedes Drücken, Rad,
  Loslassen, jede Taste und jeder Text muss dasselbe Widget erreichen wie bei der Aufnahme, sonst
  „diverged“, Exit-Code 3 und frischer Start; Eingaben von außen (Ablegen aus anderen Programmen,
  der Dateidialog des Systems, Ziehen hinaus) machen das Protokoll unabspielbar. Daneben bleiben
  Fensterposition und -größe (`window.ini` über `jfgui_STATE`), Stil, Textskalierung und der
  Inspektor mit Reiter, Optionen und Auswahl (Skript-Selektor) erhalten (`carry.ini`), auch bei
  frischem Start. Schlägt der Bau fehl, läuft die alte Fassung weiter und der Fehler steht oben im
  Inspektor („Bau fehlgeschlagen“), der sich dafür öffnet. Tests: `tools/tests/test_hotreload.py`
  (der Beobachter mit einem Ersatzprogramm, läuft in der Suite; eine vorübergehende App gebaut,
  aufgezeichnet, abgespielt, abgewichen, Bau gescheitert, headless auf Windows und Linux; die
  Fensterschleife unter Windows auf einem eigenen Desktop, nie auf dem Bildschirm). Grenze: was das
  Abspielen nicht kennt (Dateien, Netz, Zeitpunkte von Hintergrundaufgaben, Agenten-Aktionen, die
  echte Uhr), kann nach dem Neustart abweichen.

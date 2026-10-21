# PaintCraft – Diagnose, Absturzbericht, Info (CORE-008 / -009 / -017 / -018)

Dateien: `src/crash.s` (Absturzprotokoll, Berichtsfenster), `src/dlg_misc.s` (Hilfe ▸ Info),
Diagnosetext aus `src/dlg_settings.s` (`set_diag_text`, auch die Seite „Diagnose“ der
Einstellungen, SETTINGS.md §3).  Bibliothek: `plat_crash_hook`, `plat_spawn_self`,
`plat_date` (legacy/ASMLIB/docs/modules/CRASH.md).

## 1. Absturz

Die Bibliothek fängt Abstürze ab (Windows: VEH für Ausnahmen im eigenen Image + Unhandled-
Filter; Linux: SIGSEGV / SIGBUS / SIGILL / SIGFPE auf dem Signal-Stack), füllt CRASHINFO
(Code, Art, Zugriff, Datenadresse, Register, Aufrufkette) und ruft den Hook von PaintCraft:

1. Protokoll `Absturz_JJJJ-MM-TT_hh-mm-ss.txt` im Absturzordner: `--crash-dir DIR`, sonst
   `CrashLogs` neben `settings.ini` (`--config DIR`, `$PaintCraft_CONFIG_DIR`,
   `%APPDATA%/PaintCraft` bzw. `~/.config/PaintCraft`).  Testläufe ohne Konfiguration und ohne
   `--crash-dir` installieren keinen Hook (nur die stderr-Zeile der Bibliothek, Exit-Code 3).
2. Inhalt: Einleitung, Programmversion, Zeitpunkt (Linux: UTC), Ausnahme
   („0xC0000005 Zugriffsverletzung (beim Schreiben an Adresse 0x10)“), Adresse mit Abstand im
   Programm (`PaintCraft.exe + 0x…`; die Funktionen stehen in der `.map`-Datei des Builds), die 17
   Register, die Aufrufkette (Absturzort, Aufrufer der Blattfunktion, dann die RBP-Kette der
   PROLOG-Rahmen bis zum Einstieg), danach „---- Diagnose ----“ mit dem Diagnosetext.  Die Datei
   wird zweimal geschrieben: erst ohne Diagnose (die ruft App-Code auf und könnte erneut
   abstürzen), dann vollständig.
3. Mit echtem Fenster startet der Hook PaintCraft neu: `--crash-report DATEI` (+ dasselbe
   `--config`).  Dann endet der abgestürzte Prozess mit Exit-Code 3.

## 2. Berichtsfenster (`--crash-report DATEI`)

Kein anderes Modul läuft (kein Bild, keine Einstellungen werden gelesen oder geschrieben); das
Hauptfenster wird 680 × 520 DIP groß, zentriert, Titel „PaintCraft Absturz“: Überschrift
„PaintCraft wurde unerwartet beendet“, die Meldung (eigener Text), der
Pfad des Protokolls, „Details anzeigen“ / „Details ausblenden“ (Protokoll als schreibgeschützter
mehrzeiliger Text), „In die Zwischenablage kopieren“, „Ordner der Absturzberichte öffnen“
(ui_open_folder), „PaintCraft neu starten“ (Akzent; neuer Prozess mit demselben `--config`,
dann Ende) und „Beenden“.  Testlauf: die Knöpfe werden nur gezählt (Probe #6720: Wert CRF_* 1 neu
starten, 2 Beenden, 4 kopiert, 8 Ordner; Text = Protokollpfad).

## 3. Hilfe ▸ Info

Modales Fenster „Info“ (WINID_DLG0 + 0x58): Name, „Version x (Windows, x64)“, Herkunft (Nachbau
von paint.net 5.1 in x86-64-Assembler auf jfguiLib, nur kernel32 / user32 / gdi32 bzw.
Systemaufrufe), Markenhinweis, Diagnose (schreibgeschützt) + „In die Zwischenablage kopieren“,
„Schließen“.  Die Web-Einträge des Hilfe-Menüs (Dokumentation, Suche, Forum, Lernprogramme,
Plug-Ins, Webseite, Spenden, Feedback) führen bei paint.net auf dessen eigene Seiten und bleiben
in diesem Nachbau deaktiviert (die Bibliothek hätte `ui_open_url`).

## 4. Tests

* `tests/crash_report.uis` (Berichtsfenster mit `tests/fixtures/crash_sample.txt`),
  `tests/info.uis`.
* `python tools/pn_crash_check.py [--target win|linux|both] [--out build/x]`: `cmd 911`
  (crash_test, Schreibzugriff auf 0x10) mit `--crash-dir`: Exit-Code 3, genau ein Protokoll mit
  Datum im Namen, Ausnahme / RAX / Register / Diagnose, Aufrufkette per `.map` aufgelöst
  `crash_test <- a1_diag_cmd <- pn_cmd <- … <- plat_entry / _start`, dann `--crash-report` mit
  diesem Protokoll.
* Bibliothek: `plattest --test crash` (CRASHINFO-Felder, Rücksprungadresse im Stapel),
  `plattest --test spawn` (plat_spawn_self), `plattest --test cpu` (plat_date).

Test-Hooks: `cmd 911` (absichtlicher Absturz), Probe #9920 (Canvas-Cursor, cursors.s).

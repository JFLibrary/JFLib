# Linux-Testplan: was auf echten Desktops noch zu prüfen ist

Stand 2026-10-30, Branch `linux-wayland-fixes` (Commits `1069f11`, `e0f2cd4`); ergänzt am
2026-10-31 um das, was seitdem auf `main` kam (Abschnitt 7), und um die Übergabe an eine
Sitzung auf dem Linux-Rechner (Abschnitt 8); am 2026-11-01 um den Lauf auf Fedora KDE (1a). Die
Linux-Überarbeitung lief zuerst nur in einer Cloud-Umgebung (headless GNOME 46, sway, Weston,
Xvfb mit openbox), dann auf dem KDE-Rechner (Plasma 6.6.6, Wayland, ein Monitor eDP-1 mit
125 %, Layout de, keine Eingabemethode). Dieser Plan hält fest, was dort geprüft ist und was
noch fehlt, getrennt nach dem Rechner, auf dem es sich prüfen lässt.

---

## 1. Erledigt auf KDE Plasma

- **Secret Service von KWallet** (`ksecretd`): Etwa jeder zweite Zugriff schlug fehl, weil
  `ksecretd` die Zahlen des Schlüsselaustauschs anders kodiert (Vorzeichenbyte, ungepolsterter
  gemeinsamer Wert). Behoben in `lib/plat/linux_secret.rs`. Fünf Läufe gegen das echte
  `ksecretd` mit 36/36; der Fake-Bus hat den Modus `ksecretd`.
- **xdg-activation** (`lib/plat/linux_wl_act.rs`): `win_activate` tat unter Wayland nichts.
  Jetzt mit Token vom Start, vom Tray, von Benachrichtigungen und von der Übergabe an die
  laufende Instanz. Geprüft ist nur der Start-Pfad (im Wayland-Trace); der Rest steht in 2.
- **Dekorationen**: KWin handelt sie richtig aus (gallery: Server, PaintCraft: eigene). Nur
  auf Protokollebene geprüft, nicht im Bild.
- **Testwerkzeuge für X11** (`ibus_bus`, `dnd_x11`, `icc_x11`): schicken jetzt das
  Xauthority-Cookie und laufen gegen KWins Xwayland.
- **Gesamtsuite** 417/420. Offen: zwei PaintCraft-Plugin-Skripte und `portal_bus` (brauchen
  die Assembly-Referenz `build/rwref`), `plattest/screen.uis` mit X11-Pipette und -Hotkey
  (rootloses Xwayland, XTEST ohne Portal-Freigabe, Bildschirm war gesperrt).
- **Schriften**: Ubuntus Noto CJK ist statisches CFF; das CFF2-Problem betrifft diesen Rechner
  nicht.

---

## 1a. Erledigt auf Fedora 44 KDE Plasma (2026-11-01)

Rechner: Fedora 44, Plasma und KWin 6.6.4, Wayland, ein Monitor 1920×1080 bei 100 %, Pentium
G4560T, 7,6 GB RAM, Intel HD 610 mit Mesa 26.0.3. Zweig `linux-tests-minipc-2026-11-01`, Stand
`539cbf0`. Die Skripte sind kopflos (`--script`); echte Fenster liefen über `--agent`, KWin-Skripte
(nur auf den eigenen Prozessen) und `spectacle` (nur das aktive Fenster).

- **Gesamtsuite** 543/544 mit `build/rwref` und den jfWord-Testdateien. Offen ist nur
  `plattest/screen.uis` (X11-Pipette und -Hotkey unter rootlosem Xwayland, auch entsperrt; siehe 4).
- **Behobener Fehler, Schriften**: `sans-serif`, `serif` und `monospace` lieferten Noto Sans Arabic
  bzw. Noto Naskh Arabic, weil Fedoras Regeln sie voranstellen. fontconfig reiht die Sprache vor
  solche Regeln, `FontDb::resolve` nahm die erste installierte Schrift. Folgen: Kästchen in der
  Vorschau von fontpicktest, Hex-Ansicht, Terminal und Code ohne feste Breite und mit doppelt hohen
  Zeilen (`datagallery/hexview.uis`). Jetzt zählen Schriften ohne A oder a in der cmap
  (`FDFF_NOLATIN`, Cache-Version 4) nur als letzte Wahl, und `<default>` steht wie in fontconfig am
  Ende der ganzen Liste. Die Reihenfolge entspricht nun `fc-match -s monospace`. Grenze: Mit `ar`
  als Sprache wählt die Library ebenfalls die lateinische Schrift; Arabisch kommt aus `fallback.rs`.
- **Behobener Fehler, PaintCraft**: Geöffnete Dateien kamen nicht in `recently-used.xbel`, die App
  führte nur ihre eigene Liste. Jetzt ja, und „Liste leeren“ entfernt nur ihre Einträge
  (`apps/PaintCraft/tests/recent_xbel_check.py`). KDEs `recentlyused:/` zeigt die Datei.
- **CFF2 mit der echten Schrift** (`NotoSansCJK-VF.ttc`): fünf Schnitte mit Flag `0xa8`, Achse `wght`
  100–900 und sieben benannte Instanzen; Standard und `wght=900` richtig gezeichnet; Vergleich mit
  HarfBuzz: 1171 Glyphen, 0 Abweichungen, größte Distanz 0,00049 Einheiten. Laden 8,8 ms,
  65 535 Konturen × 3 in 760 ms, 2000 Glyphen bei 16 px in 15,7 ms, Speicher beim Rendern 34 MiB.
  Schriftdialog mit Regler für die Strichstärke geöffnet, Text in einer Instanz nicht verglichen.
- **GPU**: `gputest` und `uigputest` werden nur für Windows gebaut (G4 fehlt), `gallery` enthält
  keinen Vulkan-Code. `jfgui_GPU=1`, `=0` und ohne Schalter liefern dasselbe Bild.
- **Dekorationen**: KWin zeichnet den Rahmen, mit `jfgui_CSD=1` die App; je eine Titelleiste im Bild.
- **Fenster**: Maximieren und Minimieren vom Desktop, gleiche Größe danach.
- **Vollbild** (`desktest`): F11 und Knopf, aus maximiert und zurück, vom Desktop aus gesteuert (die
  App folgt). Werkzeugfenster sind Subsurfaces und tun hier wie in GUIDE §35 beschrieben nichts.
- **Popups**: bei 100 % und `jfgui_SCALE=1`; passt die Liste nicht ins Fenster, wird sie ein
  `xdg_popup`, klappt am unteren Rand nach oben um und bleibt über KDEs Leiste.
- **KDE-Dateidialog**: `.png`, `.PNG`, `.jpg`, `.JPG` sichtbar, `text.txt` nicht, Filter lesbar.
- **Tray**: Symbol beim Watcher angemeldet, `Activate` kommt an, Fenster danach aktiv.
- **Zweiter Start** (PaintCraft mit Datei): Der zweite Prozess endet, das minimierte Fenster kommt
  aktiv zurück, die Datei ist offen.
- **Zwischenablage hinein**: Text mit Umlauten, € und CJK über `wl-copy` per Strg+V in der App.
- **Hot-Reload**: neu gebaut und gestartet in 16 s, gleiche Größe, Inspektor wieder offen. Die
  Position geht verloren, weil Wayland sie einer App nicht verrät.
- **Klang**: Stream `audiotest` / „Ton-Generator“ in PipeWire am Analog-Ausgang, 27,8 ms Latenz,
  0 Aussetzer (nicht angehört).
- **Wach halten**: Portal-Anfrage und `systemd-inhibit --list` zeigen es, danach frei. KDEs
  `PolicyAgent.ListInhibitions` bleibt leer.
- **plattest**: `--test all` 212/212; `recent` 43, `trash` 34, `autostart` 17, `power` 5, `secret` 36
  (KWallet), `settings` 32, `a11y` 59, `audio` 82 ohne Fehler. `linuxtest --test activate` und `clip`
  ohne Fehler.
- **Leerlauf-CPU** (Abschnitt 6): 19 von 21 Apps 0 ms je Sekunde, `buildManager` 6 ms, `editortest`
  3 ms.
- **RPM**: `package.py Kritzel --formats rpm` baut (2,1 MB), nicht installiert.

**Offen auf diesem Rechner**, braucht jemanden am Gerät oder sudo:

1. Text aus der App kopieren (Strg+C, Einfügen in Kate): Die Agent-Tasten haben keine Eingabe-Serial
   für `set_selection`, der Test blieb deshalb ohne Ergebnis.
2. Theme live (Abschnitt 2, Punkt 5); Skalierung 125 % und 150 % (`jfgui_SCALE` kennt nur ganze
   Zahlen).
3. Paletten in PaintCraft und Titelleiste mit echter Maus und Stift ziehen; Malen flüssig.
4. Aktion einer Benachrichtigung, Tray-Menü, Drag & Drop mit Dolphin, Klipper mit Bildern.
5. Tottasten, AltGr, Tastenwiederholung, GlobalShortcuts-Portal; Pipette und Screenshot-Portal.
6. Klang anhören, Ausgabegerät wechseln; Orca und KWin-Lupe; Autostart mit Ab- und Anmelden.
7. „Im Editor öffnen“ des Inspektors (nicht geprüft); Bild und Text im Schriftdialog vergleichen.
8. RPM installieren und starten; AppImage (`squashfs-tools` fehlt); Flatpak: `flatpak-builder` zieht
   hier ein KDE-Update mit, das an einem Dateikonflikt zwischen `kf6-kmime` und `kmime` scheitert.

**Gefunden und behoben**: `--agent` an echten Fenstern. Die Revision war der Bildzähler
(`lib/ui/agent.rs`), jede Anfrage zeichnet selbst ein Bild, ändernde Aufrufe scheiterten daher mit
„The application state changed after it was inspected“. Jetzt bewegt sie sich nur, wenn sich die
Bedienelemente ändern oder eine Agentenaktion etwas geändert hat
(`apps/gallery/tests/agent_revision.uis`, `apps/agenttest/tests/gui_ipc_check.py`, auch auf diesem
Rechner). Ein Testlauf mit `--agent` braucht also keine geratene Revision mehr.

---

## 2. Offen auf KDE Plasma (dieser Rechner)

Voraussetzung: Bildschirm entsperrt. Solange er gesperrt ist, meldet KWin jedes Fenster als
„activated, suspended“ und die Tastatur bleibt beim Sperrbildschirm; alle Fokus-Tests scheitern
dann, ohne dass die Library schuld ist.

1. `build/linux/linuxtest --test activate` (Minimieren, dann über `win_activate` zurück) und
   `--test clip`.
2. **Werkzeugfenster** in PaintCraft: Paletten am Rand, Ziehen, Andocken, Größe ändern mit Maus
   und Stift; Hauptfenster minimieren und maximieren; Zeigerposition während des Ziehens.
3. **Dekorationen** im Bild: keine doppelte Titelleiste; Gegenprobe mit `jfgui_CSD=1`.
4. **Popups, Menüs, Kombolisten** bei 125 % und 150 %: Lage, Schärfe, Umklappen am
   Bildschirmrand, lange Listen begrenzt (`POPUP_PANEL_ROOM` = 96 DIP gegen KDEs Leiste).
5. **Theme** live über das Settings-Portal: hell, dunkel, Akzentfarbe (in den
   Systemeinstellungen umschalten, die App muss sofort folgen).
6. **KDE-Dateidialog**: Filter `*.[pP][nN][gG]` lesbar dargestellt und wirksam; Dateien mit
   `.JPG` sichtbar.
7. **Tray und Benachrichtigungen** (traydemo, BackupCraft): „Fenster anzeigen“ holt ein
   minimiertes Fenster zurück (Token über `ProvideXdgActivationToken`); die Aktion einer
   Benachrichtigung holt das Fenster nach vorn; ein zweiter Start (Datei aus Dolphin öffnen)
   holt die laufende Instanz nach vorn.
8. **Zwischenablage** mit Klipper: Text mit Umlauten und €, Bilder. **Drag & Drop** mit Dolphin
   in beide Richtungen.
9. **Tastatur**: Tottasten, AltGr, Tastenwiederholung, GlobalShortcuts-Portal.
10. **Bildschirm**: Pipette und Screenshot-Portal (screentest). Mehrere Monitore nur mit
    externem Monitor.
11. **Leistung**: Malen in Kritzel und PaintCraft flüssig; Leerlauf-CPU aller GUI-Apps ≈ 0
    (Skript in 6).
12. `plattest/screen.uis` noch einmal mit entsperrtem Bildschirm. Scheitert X11-Pipette oder
    -Hotkey weiter, auf echtem X11 prüfen (Abschnitt 4).

---

## 3. Fedora Workstation (GNOME) auf einem alten Laptop

**Einrichtung**

- Automatische Anmeldung an, Bildschirmsperre und automatische Bereitschaft aus.
- `sudo dnf install git python3 gcc lld openssh-server`, dann `sudo systemctl enable --now sshd`
  (`gcc`: cargo linkt die Proc-Macros von `lib/text_shape` mit `cc`).
- rustup, dann `rustup toolchain install 1.98`.
- In der Claude-App als SSH-Verbindung eintragen (oder Hostname und Benutzer nennen).
- Das Repo ist öffentlich: `git clone https://github.com/JFLibrary/JFLib.git` (in einem
  vorhandenen Klon `git pull --ff-only`). Ein Git-Bundle vom KDE-Rechner geht weiterhin.
- Für Abschnitt 7: `sudo dnf install rpm-build rpm-sign` (RPM bauen und signieren); Flatpak
  und Orca bringt Fedora Workstation mit.

**Prüfen**

- **CFF2** (seit 2026-11-01 gelesen, `lib/text/cff.rs`; bisher nur mit selbst erzeugten
  Testschriften und gegen HarfBuzz/FreeType geprüft, mit keiner echten Schrift):
  1. `rpm -q google-noto-sans-cjk-vf-fonts google-noto-serif-cjk-vf-fonts`, `rpm -ql
     google-noto-sans-cjk-vf-fonts` (erwartet etwa
     `/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc`); Schnitte und
     Familien zum Abgleich mit `fc-query DATEI`.
  2. `build/linux/fonttest --scan`: die Noto-CJK-Schnitte mit den Flags `0xa8`
     (CFF | CFF2 | VAR).
  3. `build/linux/fonttest --dump DATEI SCHNITT`: Achse `wght` und die benannten Instanzen.
  4. `build/linux/fonttest --render DATEI SCHNITT 40 "永中文日本語한국어" out.png`, einmal
     ohne und einmal mit `--axes wght=900`; die PNGs ansehen.
  5. `python3 tools/cff2_crosscheck.py font DATEI --face N --axes wght=700 --fonttest
     build/linux/fonttest`: „0 differ“, kleine Distanz zu HarfBuzz.
  6. `build/linux/fonttest --bench DATEI SCHNITT` (Geschwindigkeit).
  7. Im Fenster: `fontpicktest`, im Dialog „Noto Sans CJK SC“ wählen und den Regler der
     Strichstärke bewegen; CJK-Text in einem Textfeld der Galerie zeigt Zeichen statt
     Kästchen (ein alter Schriften-Cache wird wegen der neuen Version neu eingelesen).
     Speicher und erste Ladezeit beobachten: Jeder geladene Schnitt einer Collection liest
     heute die ganze Datei.
- **Eigene Titelleiste** (mutter kennt xdg-decoration nicht): GNOMEs `button-layout` und die
  Doppelklick-Aktion werden ignoriert; die oberen ~5 DIP der Knöpfe und eine Ecke von
  „Schließen“ starten ein Resize.
- **xdg-activation unter mutter** (strenger als KWin): `linuxtest --test activate`, Tray,
  Benachrichtigung, zweiter Start.
- **Eingabemethode**: IBus ist aktiv. Japanisch oder Chinesisch einrichten, Vorschlagsfenster
  an der Schreibmarke, Übernahme, Abbruch.
- **gnome-keyring** als Secret Service: `plattest` mit dem echten Dienst (die Gegenprobe zur
  KWallet-Kodierung).
- **GNOME-Portale**: Dateidialog (GTK), Screenshot, Akzentfarbe, Dunkelmodus.
- **Benachrichtigungen** mit Aktionen; **Tray** einmal ohne, einmal mit der
  AppIndicator-Erweiterung.
- **Alte Hardware**: Malen und Leerlauf-CPU; niedriger Bildschirm (etwa 768 px) für lange Listen
  und Popups; 100 % Skalierung.
- Die Punkte aus 2, soweit sie unter GNOME Sinn ergeben, dazu `build_rs.py test` und
  `plattest --test all`.

---

## 4. Fedora-Xfce-Spin (echtes X11), falls ein zweiter Laptop frei ist

GNOME und KDE auf Fedora haben keine X11-Sitzung mehr; Xfce läuft auf X11.

- `plattest --test all` samt `x11err` und `x11synth`.
- `plattest/screen.uis`: X11-Pipette und -Hotkey gegen einen echten X-Server.
- `dnd_x11.py` und `icc_x11.py` gegen echtes X11.
- Werkzeugfenster, Popups, Zwischenablage, Drag & Drop mit Thunar, Tray im xfce4-panel.

---

## 5. Offen ohne passenden Rechner (mit Jakob abstimmen)

- CFF2-Schriften: seit 2026-11-01 mit der echten Schrift geprüft (1a); unter GNOME noch nicht.
- Die eigene Titelleiste zeigt immer alle Knöpfe (siehe 3).
- Resize-Zone über den Titelleisten-Knöpfen (siehe 3).
- DNS-Server auf IPv6-Link-Local-Adressen scheitern.
- Dateinamen, die kein UTF-8 sind, werden übersprungen.
- Dunkles Theme ohne Settings-Portal.
- Windows: F-03 und F-04 ungetestet, ebenso die Windows-Seite der Token-Übergabe in
  `lib/plat/instance.rs`. Braucht einen Windows-Rechner mit Build.
- `portal_bus` und die PaintCraft-Plugin-Skripte brauchen `build/rwref`.

---

## 6. Hinweise zur Umgebung

- Baut `ld.lld` nicht, weil es nicht im System liegt, hilft der lld von Rust:
  `export PATH=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld:$PATH`.
- Der Code zielt auf Rust 1.98; 1.99 baut ihn ebenfalls (nur die Versionswarnung).
- `pkill -f MUSTER` und `pgrep -f MUSTER` treffen auch die eigene Shell, wenn das Muster in
  ihrer Befehlszeile steht. Eine Warteschleife `until ! pgrep -f MUSTER` endet dann nie.
  Prozesse per PID oder `pgrep -x` verfolgen.
- Leerlauf-CPU messen (Start, 5 s warten, CPU-Zeit über 3 s, beenden):

```bash
T=$(mktemp -d)
for a in BackupCraft JFNetworker Kritzel PaintCraft jfWord buildManager gallery datagallery \
         editortest docktest dndtest diagramtest themedemo traydemo screentest spelltest svgtest \
         termtest mdviewtest fontpicktest; do
  [ -x build/linux/$a ] || { echo "$a: not built"; continue; }
  mkdir -p $T/cfg-$a
  build/linux/$a --config $T/cfg-$a > $T/$a.log 2>&1 & p=$!
  sleep 5
  kill -0 $p 2>/dev/null || { echo "$a: ended early"; continue; }
  c0=$(awk '{print $14+$15}' /proc/$p/stat); sleep 3; c1=$(awk '{print $14+$15}' /proc/$p/stat)
  kill -9 $p; wait $p 2>/dev/null
  echo "$a: $(( (c1 - c0) * 10 / 3 )) ms CPU per s idle"
done
```

---

## 7. Neu seit dem 2026-10-30: auf jedem echten Desktop prüfen

Was seit Abschnitt 1 bis 6 auf `main` kam und bisher nur in WSL (WSLg: Weston, rootloses
Xwayland, PulseAudio-Server von WSLg) oder gegen Fake-Busse lief.

1. **Vollbild** (`apps/desktest`, GUIDE §35): Haupt- und Werkzeugfenster ins Vollbild und
   zurück, auch aus maximiert; danach dieselbe Lage und Größe. Verlässt man das Vollbild über
   den Desktop (KDE: Fenstermenü, GNOME: Super+↑/↓ bzw. Doppelklick), muss die App folgen
   (`UIC_FULLSCREEN`). Mehrere Monitore nur mit externem Monitor.
2. **Fenster in einem Bereich und Taskleiste** (`apps/desktest`, `lib/ui/desk.rs`): Ziehen,
   Größe, Einrasten, Minimieren in die Taskleiste, bei 100 %, 125 % und 150 %.
3. **Zuletzt verwendet** (`plat::recent_add`, `~/.local/share/recently-used.xbel`): eine Datei
   in PaintCraft öffnen und speichern; sie erscheint in Dolphin bzw. Nautilus unter „Zuletzt
   verwendet“ und in den „Zuletzt“-Listen der Dateidialoge, mit PaintCraft als Programm. Aus der
   Liste der App entfernt oder geleert, verschwindet nur der Eintrag von PaintCraft; Einträge
   anderer Programme bleiben. `build/linux/plattest --test recent` läuft auf einer Testdatei.
4. **Hot-Reload und Inspektor** nativ (nicht WSL):
   `python3 build_rs.py watch gallery --target linux -- --inspect`. Eine Quelldatei der Galerie
   speichern: Die App baut neu und startet mit demselben Fenster, derselben Seite und dem
   Inspektor samt Auswahl. Im Inspektor zeigt „Quelle“ Datei und Zeile des Widgets; „Im Editor
   öffnen“ öffnet den installierten Editor an dieser Zeile (`jfgui_EDITOR`, sonst
   `$VISUAL`/`$EDITOR`, sonst CLion oder VS Code). Der Editor bleibt offen, wenn die App endet.
5. **Variable Schriften** (`build/linux/fontpicktest`, Schriftdialog): die variablen
   Systemschriften (GNOME: Cantarell bzw. Adwaita Sans, Noto als VF) zeigen ihre Achsen und
   benannten Instanzen, Text in einer Instanz sieht aus wie im Dialog. **CFF2**
   (Noto CJK VF): die Schritte in Abschnitt 3.
6. **Pakete** (PACKAGING.md „Other formats“; Fedora: `rpm-build`, `rpm-sign`):
   `python3 tools/package.py Kritzel --formats rpm,flatpak,appimage` mit Signatur
   (`--sign`, Testschlüssel). Das RPM mit `sudo dnf install ./build/dist/…rpm` installieren,
   aus dem Startmenü starten, Dateizuordnung prüfen, mit `dnf remove` entfernen. Das Flatpak
   für den Nutzer installieren, starten, Dateidialog über das Portal, Update auf die nächste
   Version (`updatetest` hat keine `PACKAGE`-Datei, `--no-deb` lässt alle Linux-Formate weg, das
   AppImage braucht `squashfs-tools`, das Flatpak `flatpak-builder`). Das AppImage per Doppelklick und im Terminal starten (ohne FUSE), danach das
   Update der App selbst (`updatetest`). Danach alles wieder entfernen.
7. **Klang über echtes PipeWire** (`audiotest`, spielt hörbar ab): Wiedergabe, der Name des
   Streams in der Lautstärkeregelung des Desktops, Ausgabegerät während der Wiedergabe wechseln.
8. **Screenreader und Lupe**: Orca mit der Galerie (Tab-Fokus, Knöpfe, Häkchen, Slider-Werte,
   Textfeld mit Schreibmarke und Auswahl, Überschriften im Markdown-Betrachter); die GNOME-Lupe
   bzw. KWin-Lupe folgt der Schreibmarke. Bisher nur gegen at-spi2-core ohne echten
   Screenreader geprüft.
9. **Papierkorb** (`plat::trash`, `plattest --test trash`): eine Datei in den Papierkorb
   legen; sie erscheint in Dolphin bzw. Nautilus mit Herkunftsordner und lässt sich
   wiederherstellen. Bekannt: Das Löschdatum steht in UTC (ROADMAP §2) und kann um die
   Zeitzone verschoben angezeigt werden.
10. **Autostart** (`plat::autostart_set`, `updatetest` bzw. `plattest --test autostart`):
    einschalten, ab- und wieder anmelden, die App startet; ausschalten entfernt die Datei.
11. **Standby verhindern** (`plattest --test power`): Während die App sperrt, zeigt der Desktop
    das an (KDE: Energieverwaltung; GNOME: kein Bereitschaftsmodus), danach wieder frei.

---

## 8. Übergabe an eine Sitzung auf dem Linux-Rechner

Für eine Claude-Sitzung, die auf dem Linux-Rechner selbst läuft (oder per SSH dorthin).

- **Stand holen**: `git clone https://github.com/JFLibrary/JFLib.git` bzw. im vorhandenen Klon
  `git pull --ff-only`. Nie `git init`; nie die alte Geschichte (`archive/main`, Wurzel
  `2e0b359`) mergen oder pushen.
- **Arbeitszweig**: `git switch -c linux-tests-<rechner>-<datum>`. Ergebnisse und Fixes dort
  committen, nie auf `main`. Den Zweig nur mit Jakobs Zustimmung pushen, sonst als Bundle
  (`git bundle create ../linux-tests.bundle origin/main..HEAD`).
- **Reihenfolge**: Einrichtung (Abschnitt 3 bzw. 6) prüfen und nur nachfragen, was fehlt
  (Installationen mit `sudo dnf`/`apt` nur nach Jakobs Ja); `python3 build_rs.py test --quick
  --target linux`; dann der Abschnitt des Desktops (2 KDE, 3 GNOME, 4 Xfce/X11) und
  Abschnitt 7. Fenster dürfen hier sichtbar aufgehen; danach schließen.
- **Einstellungen**: Was für einen Test geändert wird (Skalierung, Theme, Akzentfarbe,
  Eingabemethode, Autostart), danach zurückstellen.
- **Ergebnisse**: in dieser Datei. Erledigtes nach Abschnitt 1 („Erledigt auf …“, mit Desktop
  und Version), Offenes in die Liste des Desktops, Fehler mit Befehl, Erwartung und Befund.
  Ein klarer, kleiner Fehler darf gleich behoben werden, mit Test (`.uis`-Skript oder
  plattest), sonst nur beschreiben.
- **Bericht** an Jakob auf Deutsch, kurz: was geprüft ist, was scheiterte, was behoben ist,
  welcher Zweig oder welches Bundle es enthält.

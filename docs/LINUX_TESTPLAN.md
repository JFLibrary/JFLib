# Linux-Testplan: was auf echten Desktops noch zu prüfen ist

Stand 2026-10-30, Branch `linux-wayland-fixes` (Commits `1069f11`, `e0f2cd4`). Die
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
- `sudo dnf install git python3 lld openssh-server`, dann `sudo systemctl enable --now sshd`.
- rustup, dann `rustup toolchain install 1.98`.
- In der Claude-App als SSH-Verbindung eintragen (oder Hostname und Benutzer nennen).
- Das Repo kommt als Git-Bundle vom KDE-Rechner (`git bundle create`), ohne Push.

**Prüfen**

- **CFF2**: Ist Noto CJK als variable Schrift installiert (`rpm -q
  google-noto-sans-cjk-vf-fonts`), zeigt die Library vermutlich Kästchen. Dann CFF2 nachrüsten
  (`lib/text/cff.rs`, `fontdb.rs`: Standardinstanz, Blend-Operanden ignorieren) und dort prüfen.
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

- CFF2-Schriften werden nicht gelesen (siehe 3).
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

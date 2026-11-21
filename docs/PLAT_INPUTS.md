# PLAT_INPUTS -- Daten anderer Programme in plat

`plat` ist die Schicht mit dem meisten `unsafe` der Library (rund 3000 Stellen), weil es die Schnittstellen der
Betriebssysteme aufruft. Gefährlich wird `unsafe` dort, wo Daten ankommen, die ein anderes Programm bestimmt: die
Zwischenablage, Drag and Drop, der Compositor und der X-Server, der Session-Bus, die Agenten-Schnittstelle. Diese
Seite listet jeden solchen Weg, wie er begrenzt ist, wo das `unsafe` sitzt und was ihn prüft. Stand: Paket U6 aus
Abschnitt 13 der [ROADMAP](ROADMAP.md), 21.11.2026.

Das Muster ist überall dasselbe, und die Durchsicht von U6 hat es an jeder Stelle unten nachgeprüft: Ein kleiner
`unsafe`-Block kopiert die fremden Bytes in einen eigenen Puffer, begrenzt durch die Größe, die das System nennt
(`GlobalSize`, die Länge von `recvmsg`, `read`), und durch eine Obergrenze der Bibliothek. Alles Weitere liest
sicheres Rust aus diesem Puffer: Längen und Versätze aus den Daten werden gegen ihn geprüft, ein Fehler ist ein
`None` oder ein Abbruch der Verbindung, nie ein Zugriff daneben. Ein Panic in einem solchen Leser würde die App
beenden (`panic = "abort"`): deshalb gehen alle Leser durch den Fuzz-Korpus von `plattest --test wirefuzz`.

## Die Wege

| Weg | Woher | Datei | Grenze | Leser (sicheres Rust) |
| --- | --- | --- | --- | --- |
| Zwischenablage, Windows | jedes Programm des Desktops | [`win_clip.rs`](../lib/plat/win_clip.rs) | `GlobalSize`, höchstens `CLIP_MAX` (1 GiB), Rich Text `CLIP_RICH_MAX` (64 MiB) | `clip_wide_text`, `clip_html_decode`, `clip_dib_to_bmp`, der DOCX-Block |
| Drag and Drop, Windows | das Quellprogramm (`IDataObject`) | [`win_dnd.rs`](../lib/plat/win_dnd.rs) | nur `TYMED_HGLOBAL`, `GlobalSize`, höchstens `BLOCK_MAX` (256 MiB) | `hdrop_list` (höchstens 4096 Pfade), `clip_wide_text`, `drop_uri_list` |
| Dateien fallen lassen, Windows (`WM_DROPFILES`) | das Quellprogramm | [`win_clip.rs`](../lib/plat/win_clip.rs) | shell32 `DragQueryFileW`, höchstens 4096 Dateien zu 32767 Zeichen | – |
| Wayland | der Compositor (darin Daten anderer Clients) | [`linux_wl_wire.rs`](../lib/plat/linux_wl_wire.rs) | 64 KiB Empfangspuffer, 64 Deskriptoren im Ring (mehr: Ende), Größe und Ausrichtung jeder Nachricht | `decode_args` (Strings mit NUL, Arrays, höchstens 24 Argumente) |
| Zwischenablage und Drop, Wayland | andere Clients über Pipes | [`linux_wl_clip.rs`](../lib/plat/linux_wl_clip.rs) | Text 512 MiB, Rich Text 64 MiB, `text/uri-list` 1 MiB, Bilder 256 MiB, eine Zeitgrenze je Übertragung | `drop_uri_list`, `clip_bmp_fix` |
| X11 | der X-Server (darin Eigenschaften anderer Clients) | [`linux_x11_wire.rs`](../lib/plat/linux_x11_wire.rs) | 4 MiB Empfangspuffer; die Antwort beim Verbindungsaufbau und jede Antwort gegen ihn geprüft | die Leser von `linux_x11_clip.rs` und `linux_x11_dnd.rs` (ohne `unsafe`) |
| D-Bus (Session- und Systembus) | jeder Client des Busses kann der App schreiben | [`linux_dbus.rs`](../lib/plat/linux_dbus.rs), [`linux_portal.rs`](../lib/plat/linux_portal.rs) | 1 MiB je Nachricht (größer oder big-endian: übersprungen, die Verbindung bleibt), Verschachtelung 32 | `decode`: Tray, IBus, AT-SPI, Portale, logind |
| Agenten-Schnittstelle | lokale Programme desselben Benutzers | [`control_ipc_linux.rs`](../lib/plat/control_ipc_linux.rs), [`control_ipc_win.rs`](../lib/plat/control_ipc_win.rs), [`control_ipc_apple.rs`](../lib/plat/control_ipc_apple.rs) | nur derselbe Benutzer (`SO_PEERCRED`, `getpeereid`, unter Windows die SID des Prozesses), unter Linux ein Socket in einem Ordner mit 0700 | die Rahmen liest `agent_io` (sicheres Rust, `agenttest`) |

## Was es prüft

- **`plattest --test wirefuzz`** ([`apps/plattest/t_wire.rs`](../apps/plattest/t_wire.rs), in der Suite): erst gültige
  Daten mit den Antworten, die die Formate festlegen (CF_UNICODETEXT, ein DROPFILES-Block breit und ANSI, eine
  `text/uri-list`, ein DIB, CF_HTML; unter Linux ein D-Bus-Signal und Wayland-Ereignisse), dann 100 000 zufällige und
  veränderte Eingaben mit festem Seed durch jeden Leser: kein Panic, Längen innerhalb der Eingabe, Text als UTF-8,
  Bilder als BMP-Datei. Einmal mit 2 Millionen Fällen gelaufen: nichts gefunden. Die Leser erreicht der Test über
  `plat::fuzz` (versteckt, keine API), ohne Compositor, Bus oder fremdes Programm. Die beiden Leser, die nur Windows
  brauchte (`clip_wide_text`, `hdrop_list`), liegen dafür seit U6 in [`clipdrop.rs`](../lib/plat/clipdrop.rs) und
  laufen auf jedem System durch den Test.
- **`plattest --test clip`, `cliprich`, `dnd`, `dnd-x11`** prüfen die Wege mit echten Programmen auf dem Desktop.
- **`agenttest`** prüft die Rahmen der Agenten-Schnittstelle (geteiltes UTF-8, CRLF, zu große Datensätze).
- **[`tools/memory_safety_check.py`](../tools/memory_safety_check.py)** prüft mit ASan Proben aus `rt` und `simd`.
  Für die Leser oben bringt ASan nichts dazu: Sie sind sicheres Rust, ein Fehler dort ist ein Panic, kein Zugriff
  daneben; das prüft der Fuzz-Test.

## Was U6 behoben hat

- Wayland: Ein Ereignis für ein Objekt, das schon gelöscht ist, gibt seine Deskriptoren zurück, damit sie geschlossen
  werden. Bisher wurden nur die ersten 8 geschlossen, die übrigen blieben offen. Die Dekodierung der Argumente ist eine
  reine Funktion (`decode_args`); eine Nachricht, die nicht hält, was ihre Signatur sagt, beendet die Verbindung wie
  bisher.

## Was offen ist

- **UI Automation unter Windows** ([`win_uia.rs`](../lib/plat/win_uia.rs), [`win_uia_text.rs`](../lib/plat/win_uia_text.rs),
  zusammen rund 370 Stellen mit `unsafe`): Bildschirmleser rufen die COM-Schnittstellen der App mit Zeigern und
  `VARIANT`s auf. U6 hat sie nicht durchgesehen; dafür braucht es einen Windows-Rechner mit Narrator und NVDA.
- **macOS und iOS** (Zwischenablage, Drag and Drop, Bedienungshilfen): auf diesem Rechner nicht zu prüfen.
- **Ein Ereignis für ein unbekanntes Objekt oder mit unbekanntem Opcode unter Wayland:** Seine Deskriptoren bleiben
  im Ring, weil ohne Signatur niemand weiß, wie viele es sind (libwayland hat dieselbe Grenze). Ein Compositor, der so
  etwas schickt, verletzt das Protokoll; die nächste Nachricht mit Deskriptor bekommt dann einen falschen.
- **Ein Fuzzer mit Abdeckung** über `plat::fuzz` in `tools/net_fuzz` (die Sicherheits-Sitzung).

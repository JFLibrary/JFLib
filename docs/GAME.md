# Spiel-Engine: Plan für G7

Stand 2026-11-17, auf Basis von `main` ab7e77e1. Umgesetzt sind die ersten Teile von G7a, G7c und G7f (Welle 1), von G7b, G7d und G7g (Welle 2), von G7e und die
zweiten Teile von G7c und G7f (Welle 3), der zweite Teil von G7a und der erste von G7h (Welle 4), die zweiten Teile
von G7b, G7e und G7g (Welle 5), siehe dort; Stand 2026-11-20, die Entscheidungen nach Welle 5 vom 24.11.2026
stehen bei G7a, G7b, G7d, G7e und G7h. Die Zahl der Läufe je Stufe ist
geschätzt (ein Lauf ist eine Arbeitsrunde eines Agenten mit Durchsicht, wie bei G5 und G6).

## Ziel

Eine eigene Spiel-Engine auf [`render3d`](RENDER3D.md), in der Richtung von Unreal Engine und Open 3D Engine. Das
Fernziel sind sehr realistische Welten bis zu ganzen Städten. Der Weg dahin hat zwei Säulen:

1. **Spiellogik an einem kleinen Spiel lernen.** Spiel 1 ist ein eigener schneller Mehrspieler-Shooter in
   Cartoon-Grafik, im Browser und auf dem Desktop. Krunker ist nur ein Beispiel für das Genre: Name, Figuren, Karten
   und Aussehen sind eigen, nichts wird nachgebaut.
2. **Eine Umgebung, in der KI gut modellieren kann.** KI setzt keine einzelnen Ecken, sie beschreibt Modelle in
   JFScript aus Regeln, Bausteinen und Parametern; die Engine baut daraus Meshes, zeigt Bilder aus festen Kameras und
   prüft Maße und Regeln. Eine Engine-IDE kommt erst, wenn dieses Modellieren gut funktioniert.

Realismus ist eine Sache der Stufe Voll (RTX-Klasse). Spiel 1 läuft auch auf Stufe Basis: im Browser, auf der Iris
Xe, auf älteren GPUs.

## Entscheidungen

Vom Nutzer am 17.11.2026:

| Nr. | Entscheidung |
| --- | --- |
| E1 | Alles entsteht zuerst in JFLib: Crates in `lib/`, Spiele und Werkzeuge in `apps/`. Kein Release auf crates.io, solange sich `render3d` stark ändert. |
| E2 | Engine, Spiele und IDE ziehen später in eigene Repositories. `render3d` bleibt dauerhaft in JFLib, wie `render2d`. |
| E3 | Die Spiel-Bausteine bekommen eigene Crates; nichts davon kommt in `render3d`. |
| E4 | Modelle und Welten werden in JFScript beschrieben; JFScript wird dafür ausgebaut. |
| E5 | Die Physik ist eine eigene, keine Portierung. |
| E6 | Spiel 1 hat Mehrspieler und läuft im Browser und auf dem Desktop. |
| E7 | JFScript bleibt im eigenen Repository. JFLib nimmt es per git wie `editortest`; die Anbindung liegt in `apps/modeltool` und den Spielen, nicht in veröffentlichten Crates. Was die Sprache braucht, wird dort gebaut und eingespielt. |
| E8 | Im Browser trägt WebSocket das Netz von Spiel 1. WebTransport (Datagramme über QUIC) erst, wenn `netio` QUIC kann; das wäre eine eigene Stufe. |
| E9 | Das Crate des Spielkerns heißt `lib/game` (`jflib-game`). |

E7 bis E9 waren Vorschläge dieses Plans, angenommen am selben Tag. Was später zu entscheiden ist, steht unter
[Offen](#offen).

## Ausgangslage

Was es gibt (Code auf `main` ab7e77e1):

- **GPU:** [`lib/gpu`](GPU.md) mit Vulkan (Windows, Linux, Android), WebGPU (Browser) und Metal (macOS, iOS), Stufen
  Basis und Voll; der eigene WGSL-Compiler ([`lib/wgsl`](WGSL.md)). Mesh-Shader und Raytracing meldet `lib/gpu` in
  `Features`, schaltet sie aber nicht ein. Kein MSAA.
- **3D:** [`render3d`](RENDER3D.md) hängt nur an `gpu` und `gltf`: Instanzen ohne Befehl je Objekt, Sichtbarkeit auf
  der GPU, Hi-Z-Verdeckung in zwei Phasen, Detailstufen, PBR nach glTF, kaskadierte Schatten der Sonne, Punkt- und
  Spotlichter, Umgebungslicht, Positionen in `f64` mit einem Ursprung, den das Programm setzt. Eine Änderung an einer
  Instanz kostet rund 0,5 µs auf der CPU. In der UI über `ui.gpu_view` und `AF_GPU_3D`.
- **Was `render3d` fehlt** ([Was nicht gebaut ist](RENDER3D.md#was-nicht-gebaut-ist)): Durchsichtiges (glTFs BLEND
  wird deckend), Skinning, Kantenglättung, Nachbearbeitung, viele Lichter (je Licht rund 0,1 ms bei 1 Mio. Würfeln),
  Schatten für Punkt- und Spotlichter, komprimierte Texturen, Meshes und Texturen entfernen (die Pools wachsen nur),
  doppelseitige Materialien, Partikel. Durchsichtiges, Entfernen und doppelseitige Materialien bringt G7f Teil 1.
- **glTF:** [`lib/gltf`](GLTF.md) liest keine Animationen, Skins und Morph-Ziele; seit G7g Teil 1 schreibt `write_document` ein
  `.glb` aus einem Dokument (Meshes, Materialien nach ihren Faktoren, Knoten, Szenen).
- **Eingabe:** Tastatur, Maus und Touch auf allen Zielen. Kein Gamepad, keine relative Maus, kein Pointer-Lock
  (die beiden letzten bringt G7a Teil 1).
- **Klang:** [`plat::audio`](../lib/plat/audio.rs) mit einem gemeinsamen Mischer (`play(&Sound) -> Voice`), ohne
  Raumklang.
- **Netz:** `plat::net::UdpSocket` und TCP auf Windows, Linux und Apple; [`netio`](../lib/netio/README.md) mit
  HTTP/1.1-Server und -Client und TLS 1.3. WebSocket seit G7d Teil 1 (`netio::ws`, im Browser
  `plat::net::WebSocket`). Im Browser und auf Android hat `plat` kein Netz
  ([ANDROID.md](ANDROID.md) bringt es für Android).
- **JFScript:** lebt im eigenen Repository [JFLibrary/JFScript](https://github.com/JFLibrary/JFScript); `editortest`
  nimmt es per git, die Wurzel-`Cargo.toml` legt `jflib-agent-core` per `[patch.crates-io]` auf diesen Workspace.
  Deterministisch (Gleitkomma in Software, keine Uhr, kein Zufall), mit Brennstoff, Grenzen und Sprachdiensten. Für
  schwere Geometrie zu langsam; die Rechnung gehört deshalb in Rust, das Skript beschreibt.
- **Nicht da:** Physik (seit G7c Teil 1 die Kollision in `lib/physics`), Animation, Netcode. Werkzeuge zum
  Modellieren seit G7g Teil 1 (`lib/geom`, `apps/modeltool`).

## Crates

| Crate | Inhalt | hängt an | Lage nach dem Auszug (G7p) |
| --- | --- | --- | --- |
| `render3d` (wächst) | nur Zeichnen, auch Durchsichtiges, Partikel, Skinning auf der GPU, Nachbearbeitung | `gpu`, `gltf`, `rt` (seit G7f) | JFLib (E2) |
| `plat` (wächst) | Gamepad, relative Maus, Pointer-Lock, Tasten nach Lage; WebSocket im Browser | – | JFLib |
| `lib/physics` (neu) | Formen, Kollision, Abfragen, Character-Controller, Starrkörper und Gelenke (seit U18) | nur Mathematik | Vorschlag: JFLib |
| `lib/anim` (neu, G7i) | Skelette, Clips, Überblenden, IK (die CPU-Seite) | `gltf` | Vorschlag: JFLib |
| `lib/geom` (neu, G7g) | der Modellierkern: Netze, Grundkörper, Extrudieren, CSG, UV, Vereinfachen | `rt` (nur Mathematik) | Vorschlag: JFLib |
| `lib/game` (neu, G7b) | Weltmodell, Takt, Eingabe-Aktionen, Kameras, Assets, Raumklang, Netcode | `physics`, `plat`, `base` (`netchan` mit G7e); `render3d` nicht | eigenes Repository |
| `lib/netchan` (neu, G7d) | Kanal mit Folgenummern über UDP und WebSocket; ein WebSocket-Typ für alle Ziele | `plat`, `crypto`; `netio` nur auf dem Desktop | Vorschlag: JFLib |
| `lib/game_render3d` (neu, G7b) | Brücke: Entitäten mit Mesh → Instanzen von `render3d` | `game`, `render3d`, `gpu` | eigenes Repository (mit `game`) |
| `apps/gametest`, `apps/gamedemo` | Tests des Kerns ohne Fenster; Demo mit Ego-Kamera | `game`, `game_render3d` | eigenes Repository |
| `apps/modeltool` (G7g) | JFScript → Mesh → Bilder und Prüfungen, für KI und Mensch | `geom`, `render3d`, `gltf`, JFScript | eigenes Repository |
| `apps/<spiel1>`, `apps/<spiel1>-server` | Spiel 1 | `game` | eigenes Repository |

Regeln:

- **Spiellogik, Physik und Netz ohne GPU.** Der Server läuft ohne Fenster; unter Linux bleibt er wie alle
  Serverprogramme statisch ohne libc. Deshalb kennt `lib/game` `render3d` nicht; die Brücke ist das eigene Crate
  `lib/game_render3d`. Ein Feature von `lib/game` ginge nicht: Cargo vereinigt die Features eines Crates über alles,
  was ein Build baut, und hätte `render3d` mit `gpu` in den Server gezogen, der mit seinem Client gebaut wird.
- **Die neuen Bibliotheken wie die übrigen:** `#![no_std]`, `#![forbid(unsafe_code)]`, keine fremden Crates,
  gebaut für Windows, Linux, Browser und Android, dazu die Apple-Ziele.
- **Die JFScript-Anbindung liegt nicht in einem veröffentlichten Crate** (ein Crate auf crates.io darf nicht von
  einem git-Stand abhängen), sondern in `apps/modeltool` und den Spielen. `lib/geom` hat eine reine Rust-API (E7).

## Stufen

Phase 1 baut die Grundlagen und Spiel 1. G7a, G7c, G7d und G7f hängen nicht voneinander ab und können in einer
Welle nebeneinander laufen; G7b braucht G7a, G7e braucht G7b und G7d, G7h braucht alles davor.

| Stufe | Inhalt | Läufe |
| --- | --- | ---: |
| G7a | Eingabe in `plat` | 2 |
| G7b | Spielkern und Spielfenster (`lib/game`) | 2–3 |
| G7c | Physik 1 (`lib/physics`) | 2–3 |
| G7d | Transport: UDP, WebSocket, Browser-Netz | 2 |
| G7e | Netcode (`lib/game`) | 2 |
| G7f | `render3d` für Spiel 1 | 2–3 |
| G7g | Modellieren mit JFScript 1 (`lib/geom`, `apps/modeltool`) | 2–3 |
| G7h | Spiel 1 | 3–4 |
| G7i | Bewegung: Animation, Skinning, Physik 2 | 3–4 |
| G7j | Große Welten | 3–4 |
| G7k | Bild: TAA, Nachbearbeitung, viele Lichter | 3–4 |
| G7l | Licht und Luft: Atmosphäre, Nebel, globale Beleuchtung | 3–5 |
| G7m | Virtuelle Geometrie | 4–6 |
| G7n | Prozedurale Stadt mit KI-Schleife | 4–6 |
| G7o | Editor / IDE | 4–6 |
| G7p | Auszug in eigene Repositories | 1–2 |

Phase 1 sind rund 17 bis 22 Läufe, die Phasen danach sind grob geschätzt und werden vor ihrem Start neu geschnitten.

Wellen in Phase 1, je höchstens drei Agenten, so geschnitten, dass in einer Welle keine zwei dieselben Dateien
anfassen:

| Welle | Stufen | Warum so |
| --- | --- | --- |
| 1 | G7a, G7c, G7f (je Teil 1) | `plat` (Eingabe), `lib/physics`, `render3d`: keine gemeinsamen Dateien. G7a liegt auf dem längsten Weg (G7a → G7b → G7e → G7h). Fertig und gepusht am 18.11.2026. |
| 2 | G7b, G7d, G7g (je Teil 1) | G7b braucht G7a; G7d fasst den Browser-Host in `plat` an, den in Welle 1 G7a ändert. Fertig am 18.11.2026. |
| 3 | G7e (Teil 1), G7c Teil 2, G7f Teil 2 | G7e braucht G7b und G7d; der Controller (`lib/physics`) und `render3d` Teil 2 berühren `lib/game` nicht. Fertig am 19.11.2026. |
| 4 | G7a Teil 2, G7h (Teil 1) | die Gamepads fassen den Browser-Host an wie G7d, deshalb nach Welle 2; Spiel 1 braucht alles davor, Gamepads nur als Zugabe. Fertig am 20.11.2026. |
| 5 | G7b Teil 2, G7e Teil 2, G7g Teil 2 | G7b nimmt neue Module in `lib/game` und die GPU-Ausgabe von `lib/ui`, G7e den Netzcode und das Netz von Spiel 1, G7g `lib/geom` und `modeltool`: keine gemeinsamen Dateien außer Registrierungen. Fertig am 20.11.2026. |

### Phase 1: Grundlagen und Spiel 1

**G7a Eingabe** (`plat`)

- Relative Maus und Pointer-Lock: Windows Raw Input (`WM_INPUT`) mit `ClipCursor`; Wayland
  `zwp_relative_pointer_manager_v1` und `zwp_pointer_constraints_v1`; X11 XInput 2 (`XI_RawMotion`) mit Grab; macOS
  `CGAssociateMouseAndMouseCursorPosition` und die Deltas der Ereignisse; Browser `requestPointerLock` (wo möglich mit
  `unadjustedMovement`) und `movementX/Y`. Lösen bei Fokusverlust und mit Esc, immer.
- Gamepad: Windows XInput (Xbox-kompatible Pads); Linux evdev; Browser Gamepad API (Standard-Belegung); macOS
  `GameController`. Andere Pads über HID später. Android und iOS gehören zu einer späteren Stufe mit Touch-Steuerung.
- Tasten nach ihrer Lage (Scancode) neben dem Zeichen, damit WASD auf jeder Tastaturbelegung an derselben Stelle
  liegt; prüfen, was `plat` heute meldet.
- Fertig, wenn: ein Test mit eingespeisten Ereignissen je Ziel läuft und der Nutzer Maus, Pointer-Lock und ein Pad
  einmal von Hand auf Windows, Linux, macOS und im Browser geprüft hat. Keine echten Eingaben durch Agenten.
- **Stand 2026-11-17: Teil 1 fertig.** Pointer-Lock und rohe Bewegung auf Windows, Wayland, X11, macOS und im
  Browser: `plat::win_pointer_lock`, `win_pointer_lock_state`, `win_raw_motion_take`, `PointerLock`, die Ereignisse
  `E_RAW_MOTION` und `E_POINTER_LOCK`; in der UI `ui.pointer_lock`, `ui.pointer_motion`. Tasten nach ihrer Lage als
  USB-HID-Usage (`KC_*`, `key_code`, `key_label`, `KeyEvent::code`, `ui.key_code_down`), außer auf Android (der
  Java-Host schickt keinen Scancode). Die Bewegung hat je Plattform eigene Einheiten: Zählwerte der Maus (Windows,
  X11), unbeschleunigte libinput-Werte (Wayland), Punkte nach der Beschleunigung (macOS), CSS-Pixel oder rohe Werte
  (Browser). Test-App `apps/inputtest` mit einer Checkliste für die Prüfung von Hand. Wayland kann nur von Hand
  geprüft werden: ein Compositor ohne Bildschirm hat keinen Zeiger und gewährt den Lock nicht. Offen: die Prüfung
  von Hand und Teil 2 (Gamepads).
- Vom Nutzer entschieden am 18.11.2026 (Vorschläge nach Teil 1): Esc gehört dem Lock, die App sieht nur, dass er
  endet (ein Spiel zeigt dann sein Pausenmenü); der Zeiger kommt unter Windows und X11 an die alte Stelle zurück wie
  unter macOS und Wayland; der Klick, der den Lock auslöst, kommt bei der App als Klick an und `lib/game` übergeht
  ihn (G7b); die Einheiten der Bewegung gleicht `lib/game` mit einem Faktor je Plattform aus (G7b).
- **Stand 2026-11-20: Teil 2 fertig** ([GAMECORE.md](GAMECORE.md), `lib/plat/README.md`). Gamepads in der
  Standard-Belegung des W3C auf allen vier Zielen: Windows über XInput (aus System32, die Guide-Taste über
  `XInputGetStateEx`), Linux über evdev (Pads nach ihren Fähigkeiten erkannt, Xbox-, Sony-, Nintendo- und einfache
  HID-Pads belegt, Hotplug über inotify), macOS über GameController, der Browser über die Gamepad API (die Seite
  fragt nur ab, solange die App zuschaut und ein Pad verbunden ist); Android und iOS noch nicht. `plat::pads()`,
  `E_GAMEPAD`, in der UI `ui.gamepads()`; abgefragt aus dem Warten der Fensterschleife, ohne Pads kostet das eine
  Abfrage. In `lib/game` Tasten, Achsen und Richtungen der Sticks als Quellen, eine radiale Totzone (0,15), Schauen
  mit dem Stick (180°/s bei vollem Ausschlag, Kurve 2), eigene Einstellungen neben denen der Maus; das zuletzt
  benutzte Pad steuert. Ein Pad spielt ohne Pointer-Lock: der erste Druck startet das Spiel und wird verschluckt wie
  der Klick, der den Lock holt. Geprüft nur mit eingespeisten Eingaben: `plattest --test pad` auf Windows und Linux
  je 80, auf macOS 76 Prüfungen, die Eingabe-Aktionen auf allen vier Zielen, der Browser in Chromium und Firefox;
  die Prüfsumme von `gametest` unverändert. Offen: die Prüfung von Hand mit echten Pads, Vibration, Pads ohne
  XInput unter Windows (PlayStation nur über Steam Input), im Flatpak `--device=input`.
- Vom Nutzer entschieden am 24.11.2026 (Fragen nach Teil 2): ein Pad spielt ohne Pointer-Lock, wie gebaut. Mit einer
  PERMISSIONS-Datei liest eine App Pads nur mit dem eigenen Schlüssel `gamepad`, und nur damit setzt das Flatpak
  `--device=input` (das zeigt der App alle Eingabegeräte, auch die Tastatur). Umgesetzt am 24.11.2026 (PERMISSIONS.md,
  PACKAGING.md; `ask gamepad` fragt beim ersten Zugriff). Vibration kommt später als eigener
  Schritt für alle Systeme (XInput, evdev, Gamepad API, Core Haptics).

**G7b Spielkern und Spielfenster** (`lib/game`)

- Weltmodell: Entitäten mit Generation, Komponenten in dichten Spalten je Typ (Sparse-Sets), Systeme als Funktionen
  in fester Reihenfolge. Archetypen oder ein Scheduler erst, wenn eine Messung sie verlangt.
- Takt: feste Simulationsrate (Vorgabe 60 Hz), das Bild interpoliert zwischen zwei Zuständen; die Zeit kommt von
  außen, damit Tests mit einer virtuellen Uhr laufen.
- Spielfenster: Vollbild und Fenster, Bildrate-Begrenzer, Present-Modus. Zwei Wege werden gemessen (Eingabe bis Bild,
  Bildabstand): eine `gpu_view` über das ganze Fenster mit der UI für HUD und Menüs darüber, oder eine eigene
  Swapchain mit der UI als Textur über dem Bild. Die Stufe schlägt einen Weg vor; im Browser ein Canvas mit
  `requestAnimationFrame`.
- Kamera-Steuerungen (Ego, Verfolger, frei), Eingabe-Aktionen mit Belegung, Empfindlichkeit und Invertieren, in den
  Einstellungen gespeichert. Die Belegung geht nach der Lage der Tasten (`KeyEvent::code`), die Anzeige nach
  `key_label`. Die rohe Bewegung (`ui.pointer_motion`) bekommt einen Faktor je Plattform, damit dieselbe
  Empfindlichkeit überall ähnlich dreht; der Klick, der den Pointer-Lock auslöst, löst keine Aktion aus (beides
  entschieden am 18.11.2026).
- Assets: glTF und Texturen über das VFS laden, im Hintergrund, mit Handles; Raumklang über den Mischer von
  `plat::audio` (Lautstärke und Panorama nach Entfernung und Richtung).
- Brücke zu `render3d`: Entitäten mit Mesh ↔ Instanzen der Szene, nur Geändertes; der Ursprung folgt der Kamera.
- Fertig, wenn: `apps/gametest` Takt, Weltmodell und Brücke ohne Fenster prüft (die Brücke gegen `render3d`
  offscreen) und eine Demo-Szene mit Ego-Kamera auf Windows, Linux, macOS und im Browser läuft.
- **Stand 2026-11-18: Teil 1 fertig** ([GAMECORE.md](GAMECORE.md)). `lib/game` (`jflib-game`): Entitäten mit
  Generation, Sparse-Sets mit einer Version je Änderung, Systeme in fester Reihenfolge; fester Takt mit genauem
  Akkumulator und Interpolation (`Pose`); Eingabe-Aktionen nach der Lage der Tasten mit den Regeln des Pointer-Locks
  (der auslösende Klick feuert nichts, das Ende des Locks ist die Pause), Empfindlichkeit in 0,022° je Zählwert und
  ein Faktor je Plattform (macOS 2,5 nur geschätzt); Ego-, Verfolger- und freie Kamera. Die Brücke
  `lib/game_render3d` schreibt nur Geändertes und lässt den Ursprung der Kamera folgen. `apps/gametest`: 901
  Prüfungen auf Windows, Linux und macOS, Prüfsumme überall gleich; `apps/gamedemo` baut für alle Ziele, lief
  aber nur ohne Fenster. Offen: Teil 2 (Spielfenster messen, Assets, Raumklang), die Prüfung von Hand.
- Vom Nutzer bestätigt am 18.11.2026: die Brücke bleibt das eigene Crate `lib/game_render3d` (statt eines Moduls
  in `lib/game`), damit ein Server ohne GPU sie nie mitbaut.
- **Stand 2026-11-20: Teil 2 fertig** ([GAMECORE.md](GAMECORE.md), [GPU.md](GPU.md)). Das Zurücklesen unter einer
  HUD war vermeidbar: Zellen, in denen über einer Ansicht nur UI liegt, werden jetzt als „Schleier“ auf der GPU
  gemischt (die UI einmal über Schwarz und einmal über Weiß gezeichnet, der Shader rechnet K + T × Bild); ein neues
  Bild der Ansicht kostet sie nichts mehr. Im Schleier ist Text in Graustufen, in Ruhe kommen die echten Pixel;
  `jfgui_GPU_VEIL=0` schaltet zurück. Gemessen auf der RTX 2070 SUPER bei 165 Hz (Fenster 1280 × 800): Weg a
  (eine `gpu_view` mit der UI darüber, MAILBOX) von der Taste bis zum Bildschirm rund 10 ms, Bildabstand p95 6,4 ms
  (vorher mit Zurücklesen 15–24 ms); Weg b (eigene Swapchain) mit MAILBOX 20 ms, mit IMMEDIATE 6 ms bei Zerreißen,
  FIFO ohne Begrenzer auf beiden Wegen rund 37 ms. Der Vorsprung von b kommt nur daher, dass Weg a eine Ansicht
  höchstens alle 4 bis 5 ms zeichnet (eine Einstellung von `lib/ui`). Vorschlag: Spiele nehmen Weg a (dieselbe
  Latenz, ganz `lib/ui` über dem Spiel, ein Weg auf allen Zielen); entscheidet der Nutzer. Im Browser zeichnet die
  Ansicht nur rund 30 Bilder/s (der Worker meldet sich bei einem Zeitgeber von 1 ms nicht als beschäftigt; eine Zeile
  in `web/worker.js` brächte die Bildrate des Bildschirms, nicht eingebaut). Assets mit Handles (`game::assets`:
  im Hintergrund oder über Bilder verteilt geladen, Zustände, Zählung, Entladen) und `Models` / `Textures` in
  `lib/game_render3d`; Raumklang (`game::audio`: Hörer, Quellen an Entitäten, Abfall mit Nah- und Fernbereich,
  Panorama mit gleicher Leistung) über den Mischer von `plat::audio`, der dafür je Stimme zwei Kanäle bekam
  (`Voice::set_gains`). Prüfungen: `assets` 75, `audio` 44 (Probe für Probe), `uigputest` 62; die Prüfsummen von
  `gametest` unverändert. Nicht gemessen: Iris Xe, HD 610, der Mac (dort ist die GPU-Ausgabe der UI aus, Ansichten
  lesen jedes Bild zurück), echte Fenster unter Linux.
- Vom Nutzer entschieden am 24.11.2026 (Fragen nach Teil 2): Spiele nehmen Weg a, die `gpu_view` mit der UI darüber;
  auf dem Mac fehlt dafür noch die GPU-Ausgabe der UI. Im Browser zeichnet eine laufende Ansicht mit der Bildrate
  des Bildschirms (die Zeile in `web/worker.js`), ruhende Apps bleiben dabei ruhig. Umgesetzt am 24.11.2026: der Worker meldet die
  nächste Frist der App, die Seite gibt die Runde zu ihr (in Chrome bei 165 Hz 147–161 Bilder/s statt 30); eine App
  in Ruhe bekommt weiter die 30 Runden je Sekunde des Leerlauf-Takts, ein langsamer Zeitgeber ersetzt einen davon
  ([GAMECORE.md](GAMECORE.md)).

**G7c Physik 1** (`lib/physics`)

- Formen: Kugel, Kapsel, Quader, konvexe Hülle, statisches Dreiecksnetz.
- Breitphase: dynamischer AABB-Baum für Bewegtes, ein fester BVH für die Karte.
- Abfragen: Strahl, Kugel- und Kapsel-Sweep, Überlappung. Daraus Treffer (Hitscan) und der Controller.
- Character-Controller (kinematisch): gehen, laufen, springen, ducken, Stufen, Rampen bis zu einem Winkel, an Wänden
  entlanggleiten, Steuerung in der Luft. Das Bewegungsgefühl hängt an Parametern.
- Deterministisch: feste Schritte, nur `+ − × ÷ √` und die Mathematik aus `rt`, also bitgleich auf allen Zielen.
  Das macht die Vorhersage im Netz genau. Geprüft mit Prüfsummen derselben Läufe auf allen Zielen.
- Fertig, wenn: Abfragen gegen Brute-Force-Referenzen stimmen, zufällige Läufe nichts durchtunneln (bis zu einer
  festen Geschwindigkeit) und die Prüfsummen auf Windows, Linux, macOS, Browser und Android gleich sind.
- **Stand 2026-11-17: Teil 1 fertig** ([PHYSICS.md](PHYSICS.md)). `lib/physics` (`jflib-physics`, in der Fassade
  `jflib::physics`) mit Kugel, Kapsel, Quader, konvexer Hülle (Quickhull) und Dreiecksnetz (binned SAH, aktive
  Kanten gegen Hänger an Nähten); drei dynamische AABB-Bäume (feste Netze, Ruhendes, Bewegtes); Strahl, Sweep,
  Überlappung, Kontakte und nächster Punkt, einzeln (`query`) und in der Welt (`World`). Alles in `f64`, Netze in
  `f32` im eigenen Bezug wie in `render3d`; nur `+ − × ÷` und die korrekt gerundete Wurzel, Sinus, Kosinus und
  Arkustangens aus `rt`. `apps/physicstest` prüft gegen eigene Referenzen (rund 1,5 Mio. Prüfungen); die Prüfsumme
  ist auf Windows, Linux (x86-64 und ARM64), macOS (M2 und Rosetta), im Browser und im Android-Emulator gleich. Auf
  dem Ryzen 5 3600 rund 1,3 Mio. Strahlen und 285 000 Kapsel-Sweeps je Sekunde gegen eine Karte mit 50 000
  Dreiecken. Noch nicht gebaut: drehende Sweeps, Netz gegen Netz, Höhenfelder, zusammengesetzte Formen. Offen:
  Teil 2, der Character-Controller.
- **Stand 2026-11-19: Teil 2 fertig** ([PHYSICS.md](PHYSICS.md)). `CharacterController` in `lib/physics`: aufrechte
  Kapsel; gehen, laufen, springen, ducken (Aufstehen nur mit Platz); Stufen bis `step_height` (eine Kante trägt, wenn
  die Fläche dahinter begehbar ist); Rampen bis 45°; Gleiten mit den Clip-Ebenen von Quake 3 (kein Zittern und kein
  Hängenbleiben in Ecken); Einrasten auf dem Weg nach unten; Luftsteuerung wie in Quake. Am Boden nähert sich die
  Geschwindigkeit der gewünschten an, statt wie in Quake zu beschleunigen (sonst war man schräg an einer Wand
  schneller als frei). `step` liest nur Lage, Geschwindigkeit und Ducken; das ist alles, was der Netcode abgleichen
  muss. Vom Nutzer am 18.11.2026 freigegeben: die eine portable Wurzel `rt::fx::sqrt_portable`, die `physics` und
  `geom` nehmen; `rt::fx::sqrtd` ist damit im Browser und im x86-64-Android-Emulator 5- bis 20-mal schneller, Bit für
  Bit gleich (auch NaN). `apps/physicstest`: 448 Szenen-Prüfungen mit genauen Erwartungen, 86 400 zufällige Schritte
  ohne Überlappung und ohne Durchtunneln bis 1000 m/s; Prüfsumme des Controllers `13dd288c669c6eaa` gleich auf
  Windows, Linux, macOS, im Browser und im Android-Emulator, die alten Prüfsummen unverändert. 178 Schritte je ms
  auf dem M2 (32 Figuren, 50 000 Dreiecke). Offen: bewegte Plattformen, `World::set_shape` für eine duckende Figur,
  die Vorgaben von Hand (mit G7h).

**G7d Transport** (`plat`, `netio`)

- Desktop: UDP über `plat::net::UdpSocket`.
- Browser: ein WebSocket-Client über den JS-Host in `plat` (heute hat `plat` im Browser kein Netz), auf dem Server
  der WebSocket-Handschlag (RFC 6455) im HTTP-Server von `netio`, auch über TLS (`wss:` für Seiten mit https).
- Für `lib/game` ein Kanal mit Nachrichten und Folgenummern: unzuverlässig und ungeordnet über UDP, zuverlässig über
  WebSocket. Der Netcode muss beides vertragen.
- Grenzen wie überall in `netio`: Größe je Nachricht, Rate je Verbindung, Zeitlimits.
- Fertig, wenn: Desktop und Browser denselben Server erreichen, gemessen mit Umlaufzeit und Durchsatz, und Fuzzing
  der Rahmen nichts zum Absturz bringt.
- **Stand 2026-11-18: Teil 1 fertig** ([TRANSPORT.md](TRANSPORT.md)). WebSocket nach RFC 6455 in `netio`
  (`ws::Acceptor` im HTTP-Server über `Body::Upgrade`, `ws::connect`, ein Decoder für fremde Bytes mit den
  Schließcodes 1002/1007/1008/1009); `plat::net` auf allen Zielen, im Browser `plat::net::WebSocket` über den Worker
  (Anfrage 32, die Sockets leben im Worker), geprüft gegen `net.connect` (`policy::net_host_allowed`). Das neue
  Crate `lib/netchan` (`jflib-netchan`): Kanal mit Folgenummern und Bestätigungen, über UDP mit zustandslosem
  Cookie-Handschlag ohne Verstärkung, über WebSocket zuverlässig mit demselben Kopf; das Protokoll auch ohne E/A für
  Simulationen. Ein eigenes Crate, weil `netio` nicht für den Browser gebaut wird. `apps/nettest`: 475 Prüfungen
  auf Windows, Linux und macOS, 2,12 Millionen Fuzz-Fälle ohne Absturz; Chrome und Firefox erreichen denselben
  Server wie der Desktop. Umlauf auf Loopback: UDP etwa 0,05 ms, WebSocket 0,1 ms, Browser 0,3–0,4 ms.
  Offen: Teil 2 (`wss:` prüfen, echtes Netz zwischen Rechnern, Grenzen abstimmen), iOS nicht gebaut, Android
  `Unsupported`.
- Vom Nutzer entschieden am 18.11.2026 (Fragen nach Teil 1): der Name `lib/netchan` bleibt; der eigene Server der
  Seite braucht im Browser keine `net.connect`-Regel (er hat die App ausgeliefert; umgesetzt); UDP bekommt später
  eine Verschlüsselung mit Authentisierung (AEAD, ChaCha20-Poly1305 aus `crypto`, in Teil 2 oder danach);
  netchan stückelt nicht: Karten und Assets kommen über HTTP, der volle Stand beim Beitreten über WebSocket (am
  19.11.2026 geändert: über UDP nach Priorität aufgefüllt, der WebSocket als Option, siehe G7e), die
  laufenden Stände bleiben mit Delta-Kompression unter 1200 Bytes (neu zu entscheiden, wenn G7e zeigt, dass sie
  nicht passen); die Grenzen je Client (300 Pakete/s, 256 KiB/s, 64 Verbindungen, 16 je Adresse, 5 s) gelten als
  Anfang und werden in Teil 2 mit Messungen abgestimmt.
- Vom Nutzer entschieden am 24.11.2026 (Frage nach G7e Teil 2): ein UDP-Socket, den `net.listen` erlaubt hat,
  darf Gegenstellen antworten, die ihm zuerst gesendet haben, ohne `net.connect`-Regel; `game1-server` braucht
  `net.connect` dann nur noch für den eigenen Client auf Loopback (`--check`). Umgesetzt am 24.11.2026: Antworten an Absender der
  letzten 60 s, höchstens 1024 je Socket ([TRANSPORT.md](TRANSPORT.md)); im LAN geprüft, ohne `PERMDENY`.

**G7e Netcode** (`lib/game`)

- Der Server entscheidet: Er simuliert mit fester Rate, Clients schicken ihre Eingaben mit Tick-Nummer, der Server
  schickt Schnappschüsse als Delta gegen den zuletzt bestätigten Stand, quantisiert.
- Client: Vorhersage der eigenen Figur, Abgleich bei Abweichung (zurück zum Stand des Servers, Eingaben neu
  anwenden), die anderen Figuren interpoliert mit rund 100 ms Puffer.
- Treffer mit Lag-Ausgleich: Der Server stellt die Trefferzonen auf die Zeit des Schützen zurück, höchstens 300 ms
  (die 100 ms der Interpolation und 200 ms darüber hinaus; vom Nutzer entschieden am 19.11.2026, vorher rund 200 ms).
- Räume per Code oder Link, ein Name, kein Konto. Bots für Tests.
- Fertig, wenn: eine simulierte Leitung im Prozess (Verzögerung, Schwankung, Verlust, Umordnung) Vorhersagefehler,
  Bandbreite je Spieler und die CPU des Servers bei 16 Bots misst und die Werte im Plan stehen.
- **Stand 2026-11-19: Teil 1 fertig** ([NETCODE.md](NETCODE.md)). `game::net` in `lib/game` über `lib/netchan`:
  Der Server rechnet mit 60 Hz und gibt jeder Figur die Eingabe ihres Clients für den Tick (fehlt sie, die letzte
  noch einmal). Clients laufen auf einer gesteuerten Uhr 2 Ticks vor, schicken mit jeder Eingabe die, die dem
  Server fehlen können, sagen ihre Figur voraus und gleichen Bit für Bit ab (der Bewegungszustand liegt als ganze
  Zahlen auf einem Raster von 1/1024 m); die anderen Figuren 100 ms hinter dem neuesten Schnappschuss.
  Schnappschüsse mit 30 Hz als Delta gegen den Stand, den der Client als entschlüsselt bestätigt (über WebSocket
  gegen den zuletzt gesendeten, am Anfang gegen die Grundlinie der Karte), nach aufaddierter Priorität auf 1200
  Bytes geschnitten. Den vollen Stand eines Beitritts, der nicht in ein Paket passt, füllen die Schnappschüsse nach
  Priorität auf (vom Nutzer entschieden am 19.11.2026: so schnell wie über einen eigenen WebSocket, rund 200 ms, ohne
  zweite Verbindung); der WebSocket mit Ticket und FULL bleibt als Option `ServerOptions::fetch_full`. Lag-Ausgleich mit zurückgestellten Kapseln und `hitscan`; Räume per Code oder
  Link, Namen; die Bewegung hinter `Movement` (Bezug `Walker`, der Controller aus G7c kommt mit G7h); das
  Bezugsspiel `Arena`, Bots, eine simulierte Leitung. Gemessen mit 16 Bots, 300 Requisiten, typisches Netz: je
  Spieler aufwärts 3,3 KB/s bei 60 Paketen/s, abwärts 13,2 KB/s; Schnappschüsse im Mittel 428, höchstens 502
  Bytes; ohne Schießen 10,6 Korrekturen je Minute mit 7 mm im Mittel, nach dem Ende schlechten Wetters jede
  Vorhersage exakt; der Server braucht 110 µs je Tick auf dem M2. Ein Beitritt mit 300 Requisiten, von denen die
  Hälfte bewegt wurde, ist 2035 Bytes und passt nicht in ein Paket. Prüfsumme einer aufgezeichneten Sitzung gleich
  auf Windows, Linux, macOS und im Browser (Node); echte Sockets über UDP und WebSocket auf Loopback; `netchan` baut
  für iOS. Die Grenze des Lag-Ausgleichs ist 300 ms (200 ms griffen schon ab etwa 50 ms Umlauf, weil Puffer und Marge
  mitzählen; vom Nutzer entschieden am 19.11.2026). Offen: Teil 2
  (echtes Netz, Browser als Client, Korrekturen glätten, Ereignisse zuverlässig, Räume auf Threads).
- **Stand 2026-11-20: Teil 2 fertig** ([NETCODE.md](NETCODE.md)). Zuverlässige Ereignisse (`game::net::event`):
  jedes genau einmal und in Reihenfolge, nach der Regel der zuverlässigen Befehle von Quake 3 in den Paketen, die
  ohnehin gehen (INPUT, SNAPSHOT), ohne Zeitgeber und ohne eigene Pakete; in `game::net` statt in `netchan`, weil
  der Netzcode schon jeden Tick sendet und bestätigt. Abmelden (`LEAVE`, Netzcode-Version 2): die Figur geht sofort.
  Geglättete Korrekturen (die Vorhersage bleibt bitgleich, nur das gezeichnete Bild klingt in 60 ms ab, über 1 m
  springt es). Mehrere Räume auf dem Worker-Pool, bitgleich zu einem Thread (8 Räume mit 16 Bots: 3,4-mal schneller
  auf dem Ryzen, 3,1-mal auf dem M2). Raumcodes und Tickets aus dem kryptographischen Zufall des Systems. Echtes
  Netz: Fedora und das ThinkPad waren nicht erreichbar, der Server lief auf dem Mac mini im WLAN, die Clients auf dem
  PC: Umlauf rund 17 ms, kein Verlust, über UDP keine Korrektur ohne Schießen; ein WebSocket-Client hält einen Tick
  mehr Marge (`stream_margin`, Korrekturen über WLAN von 12 auf 2,7 je Minute). Der Browser als Client (Chrome und
  Firefox ohne Fenster): jede Vorhersage stimmte. Spiel 1 nimmt Ereignisse (Abschussmeldungen, Treffermarke),
  Abmelden und Glätten; dabei behoben: der Server antwortete UDP-Clients im LAN nicht (PERMISSIONS), Firefox wartete
  5,7 s auf freie HTTP-Worker. Die Grenze des Lag-Ausgleichs bleibt 300 ms. Offen: ein Lauf mit Fedora als Server,
  WLAN an beiden Enden, Telefone, Deltas gegen eine Vorhersage. Die Verschlüsselung über UDP kam am 21.11.2026 mit
  U3 (TRANSPORT.md „Verschlüsselung“).
- Vom Nutzer entschieden am 24.11.2026 (Fragen nach Teil 2): der eine Tick mehr Marge für Clients über WebSocket
  bleibt (16,7 ms mehr Verzögerung nur im Browser, über WLAN 2,7 statt 12 Korrekturen je Minute); die Grenze des
  Lag-Ausgleichs bleibt 300 ms, bis Messungen im echten Netz (Fedora als Server, WLAN an beiden Enden) etwas
  anderes zeigen.

**G7f `render3d` für Spiel 1**

- Durchsichtiges: eine Klasse BLEND, je Instanz von hinten nach vorn sortiert, eigener Durchgang nach dem deckenden,
  ohne Tiefe zu schreiben.
- Ein Material für den Cartoon-Look (flache Farben, Licht in Stufen, wahlweise Umrisse). Wie es aussieht, entscheidet
  der Nutzer an Bildern.
- Partikel als Billboards aus einem Puffer, bewegt im Rechen-Shader (Mündungsfeuer, Treffer, Rauch).
- Die Waffe vor der Kamera in einem eigenen Durchgang mit eigenem Sichtfeld und Tiefenbereich.
- Billige Kantenglättung als Nachbearbeitung (FXAA oder SMAA); TAA kommt mit G7k.
- Meshes und Texturen entfernen (Karte wechseln ohne neuen Renderer), doppelseitige Materialien.
- Die Zähler der Listen in `cull.wgsl` je Arbeitsgruppe vorsammeln (ROADMAP, Entscheidung vom 09.11.2026: mit der
  ersten richtigen 3D-App), vorher und nachher gemessen.
- Fertig, wenn: jedes Stück einen Test in `render3dtest` hat (gegen den Strahlenwerfer, wo er es kann), auf Basis und
  Voll, im Browser und auf den Intel-GPUs, und die alten Bilder Byte für Byte gleich bleiben.
- **Stand 2026-11-18: Teil 1 fertig** ([RENDER3D.md](RENDER3D.md)). Meshes, Materialien und Texturen entfernen
  (`remove_mesh`, `remove_material`, `remove_texture`; Ids mit Generation, die Pools nehmen freie Bereiche wieder);
  `MaterialDesc` mit `blend` und `double_sided`, glTFs BLEND und `doubleSided` kommen jetzt durch. Durchsichtiges:
  die Sichtbarkeit sammelt alle durchsichtigen Instanzen in eine Liste, ein bitonisches Sortieren auf der GPU legt sie
  von hinten nach vorn, ein Zeichenbefehl zeichnet sie alle (vormultipliziertes Alpha, bereit für additive Partikel);
  sie werfen keine Schatten und verdecken nichts. Die Zähler der Listen addieren in vier Runden je Liste statt
  vorgesammelt in Arbeitsgruppen-Speicher (der war in drei Varianten langsamer): ohne Verdeckung 0,067 statt 0,145
  ms bei 10 % MASK, 0,067 statt 0,317 ms bei zwei Listen im Wechsel. `render3dtest --test all` bestanden auf der RTX,
  Quadro T500, Iris Xe und dem M2, im Browser; die alten Bilder Byte für Byte gleich. Nicht geprüft: die HD 610.
  Offen: Teil 2 (Cartoon-Material, Partikel, Waffe vor der Kamera, FXAA/SMAA); dabei beachten, dass jede
  durchsichtige Instanz mit den Ecken des größten durchsichtigen Meshes läuft.
- **Stand 2026-11-19: Teil 2 fertig** ([RENDER3D.md](RENDER3D.md)). Vom Nutzer am 18.11.2026 dazugenommen: eine
  Kamera ohne Perspektive (`Camera::ortho`; Sichtbarkeit, Verdeckung, Detailstufen, Sortieren und Schatten wie
  perspektivisch) und Umrisse aus dem Tiefenpuffer (Silhouetten und Falten, ein Durchgang über das Bild);
  `modeltool`s Ansichten von vorn, Seite und oben sind seitdem ohne Perspektive mit Umrissen. Dazu das
  Cartoon-Material (`MaterialDesc::toon`: Stufen, Weichheit, Randlicht; billiger als PBR), FXAA 3.11, die Waffe vor
  der Kamera (eigene Linse, Tiefe 0,5–1, nie in der Wand, im Licht und Schatten der Szene) und Partikel (auf der CPU
  aus einem Seed geboren, im Rechen-Shader in geschlossener Form bewegt, auf der GPU sortiert, ein Zeichenbefehl,
  nicht über den Weg der durchsichtigen Instanzen). Kosten in 1080p auf der Iris Xe: Umrisse 0,27 ms, FXAA 0,43 ms,
  10 000 Partikel 0,58 ms, alles zusammen 1,1 ms. `render3dtest --test all` bestanden auf RTX 2070 SUPER, M2, Iris
  Xe, T500 und llvmpipe, im Browser; die alten Bilder Byte für Byte gleich; auf der Iris Xe kein Shader mit Spills.
  Dabei behoben: `FrameStats::visible` las ohne durchsichtige Instanzen einen nie geschriebenen Platz. Nicht
  geprüft: die HD 610.
- Vom Nutzer gewählt am 19.11.2026 (an den vier Bildern in `Documents\JFLib-wip-archiv\g7f2-toon`): Variante C,
  vier weiche Stufen ohne Umrisse (`Toon { bands: 4, softness: 0.5, rim: 0.2, rim_width: 0.35 }`, kein
  `set_outline`). Das Randlicht bleibt je Material: auf Figuren und Requisiten, auf Boden und Wänden 0.

**G7g Modellieren mit JFScript 1** (`lib/geom`, `apps/modeltool`)

- `lib/geom`: indizierte Netze, Grundkörper (Quader, Zylinder, Kugel, Kegel, Torus, Treppe, Bogen), Extrudieren
  entlang eines Pfads, Drehkörper, Abschrägen, Spiegeln, Felder, Normalen, UV per Box-Projektion im Weltmaß. Boolesche
  Operationen zuerst auf konvexen Körpern (wie Brushes in Karten-Editoren: mit exakten Prädikaten robust), allgemeine
  Netz-Booleans später. glTF schreiben, damit Modelle auch außerhalb ankommen.
- JFScript-Anbindung in `apps/modeltool`: Host-Funktionen rechnen in Rust, das Skript beschreibt nur. Weil JFScript
  deterministisch ist, ergibt ein Skript auf jedem Ziel dasselbe Modell, und ein Hash aus Skript und Eingaben reicht als
  Cache-Schlüssel.
- Die Rückmeldeschleife für KI: `modeltool` baut aus einem Skript das Mesh, zeichnet Bilder aus festen Kameras
  (offscreen über `render3d`) und schreibt maschinenlesbar Maße, Dreiecke, Flächen und die Ergebnisse der Prüfungen
  (geschlossen, Normalen nach außen, keine entarteten Dreiecke, Maßstab in Metern, Budget). Für Menschen ein
  Beobachtungsmodus mit einer `gpu_view`, die beim Speichern neu baut.
- Was JFScript dafür braucht (zum Beispiel Vektoren, Zahlenfelder, Host-Objekte), stellt die Stufe fest; gebaut wird
  es im Repository von JFScript.
- Die ersten Nutzer: Karten und Requisiten von Spiel 1.
- Fertig, wenn: ein Agent aus einer Beschreibung in Text eine Reihe von Bezugsobjekten baut (Kiste, Treppe, Tür,
  Haus aus Modulen, eine kleine Karte), mit den Bildern als einziger Sicht, und der Nutzer die Ergebnisse angesehen
  hat.
- **Stand 2026-11-18: Teil 1 fertig** ([GEOM.md](GEOM.md)). `lib/geom` (`jflib-geom`): Netze in `f64` mit einer
  Topologie aus Punkten (Teile, die sich nur berühren, bleiben getrennt, `weld` verbindet ausdrücklich),
  Grundkörper (Quader, Zylinder, Kegel, Kugel, Torus, Treppe, Bogen), Extrudieren, Prisma, Sweep entlang eines
  Pfads, Drehkörper, Verschieben, Drehen in Grad (bei Vielfachen von 90° exakt), Spiegeln, Felder, Normalen, UV per
  Box-Projektion; Prüfungen (geschlossen, mannigfaltig, Normalen nach außen, entartete Dreiecke) mit JSON.
  `gltf::write_document` schreibt `.glb`. `apps/modeltool`: JFScript per git, Host-Funktionen `geom.*`,
  `profile.*`, `vec.*`, `model.*`; je Lauf `report.json`, `model.glb`, Ansichten von vorn, Seite, oben, schräg und
  ein Blatt mit allen vier; Cache über SHA-256. 31 338 Prüfungen, Prüfsumme `8d93441afd1b79d7` gleich auf Windows,
  Linux, macOS und im Browser. Fünf Beispiele (Kiste, Treppe, Tür, Haus aus Modulen, Säule) als Maßstab.
  Offen: Teil 2 (konvexe Booleans mit exakten Prädikaten, Abschrägen, Beobachtungsmodus, der Lauf, in dem ein
  Agent nur mit den Bildern modelliert); die Liste der Wünsche an JFScript in GEOM.md (Vektoren mit Operatoren,
  Host-Objekte, Position des Aufrufs, benannte Argumente, mehr Mathematik), gebaut erst nach dem Ja des Nutzers.
- **Stand 2026-11-20: Teil 2 fertig** ([GEOM.md](GEOM.md)). Boolesche Operationen auf konvexen Körpern
  (`geom::union`, `difference`, `intersection`; ein Körper ist eine Menge konvexer Zellen wie die Brushes eines
  Karten-Editors, Ergebnisse lassen sich weiter verknüpfen): Ebenen in `f64`, auf 1e-9 der Ausdehnung verschweißt,
  Ecken als Schnitt dreier Ebenen, die Seite eines Punkts exakt (gefilterte Determinante, nahe null mit exakter
  Arithmetik), Flächen ohne Koordinaten trianguliert; Abschrägen und Runden (`bevel`); Befunde nennen ihren Ort.
  `modeltool --watch` baut beim Speichern neu (Fenster mit den festen und einer freien Ansicht, Fehler mit Zeile).
  Ein Skript liest nur noch Module in seinem Ordner und darunter, keinen Verknüpfungen nach. Der Bewertungslauf:
  Kiste, Treppe, Tür, Haus aus Modulen und eine kleine Karte nur nach den Bildern gebaut, 10 Runden, alle Prüfungen
  bestanden (`Documents\JFLib-wip-archiv\g7g2-eval`); geholfen haben die Booleschen Schnitte und `expect_size`,
  schwer waren Positionen von Hand, fehlende Achsen in den Bildern, kein bedingter Ausdruck in JFScript. 38 773
  Prüfungen, Prüfsumme `0fae175a31a0f15e` gleich auf Windows, Linux, macOS und in Node (ohne das neue Beispiel und
  die neuen Operationen weiter `8d93441afd1b79d7`). JFScript blieb unverändert. Offen: die Prüfung des
  Beobachtungsmodus von Hand, Tempo bei runden Körpern (ein Turm 311 ms), ebene Teilflächen zusammenführen, Abschrägen
  nichtkonvexer Körper; welche Wünsche an JFScript gebaut werden (Rangfolge des Laufs: Vektoren mit Operatoren,
  Mathematik in Grad, benannte Argumente, bedingter Ausdruck, Position des Aufrufs).
- **Stand 2026-11-24:** `modeltool` nimmt Stufe S1 von JFScript (dort `docs/RICHTUNG.md`, die Richtung der Sprache
  für alle JF-Apps): die Namen der Skript-API sind englisch, auch die Parameter (`m.bevel(width, segments, axis)`);
  Warnungen des Hosts kommen mit ihrer Zeile in Konsole, `report.json` und Beobachtungsmodus; eine fehlgeschlagene
  Regel (`model.expect_size`, `model.budget`) nennt ihre Zeile; Mathematik in Grad, `list.range` und
  `list.linspace` kommen aus JFScript ([GEOM.md](GEOM.md)). Die Prüfsumme blieb `0fae175a31a0f15e`. Die übrigen
  Wünsche folgen dem Richtungspapier (Vektoren und Host-Objekte mit S2, benannte Argumente und der bedingte
  Ausdruck mit S3).
- **Stand 2026-11-25:** `modeltool` nimmt Stufe S2 von JFScript (a21a0c3): Meshes sind Objekte des Hosts (`type(m)`
  ist `"mesh"`, `m.tris` eine Eigenschaft), die der Host freigibt, sobald das Skript sie nicht mehr hält; Vektoren
  sind der Werttyp `vec` mit `+ − × ÷` (`treppe.jfs`: `b + richtung * 0.1`); eine Beschreibung (`host::api()`,
  `script::Api`) liefert `--api` und den Sprachdiensten ihren Blick auf den Host, deren Warnungen (eine Methode, die
  ein Mesh nicht hat, `int / int`) `modeltool` jetzt mit ausgibt ([GEOM.md](GEOM.md)). Die Prüfsumme ist
  `ff9e060c8233c600`: nur der Brennstoff der Treppe ist anders, die Bits aller Meshes sind gleich. Offen für S3:
  benannte Argumente, der bedingte Ausdruck, Zerlegen.

**G7h Spiel 1**

- Ein eigenes Spiel im Cartoon-Stil; den Namen wählt der Nutzer (ohne fremde Marken).
- Inhalt: zwei bis drei Karten aus JFScript, Figuren aus starren Teilen (ohne Skelett), drei bis vier Waffen (Hitscan
  und Geschosse), Jeder gegen jeden und Teams, Runden, Punktetafel, HUD (Leben, Munition, Fadenkreuz, Treffermarke),
  Menüs (Raum beitreten, Einstellungen für Empfindlichkeit, Sichtfeld, Tasten, Grafik, Lautstärke), Klang, Bots.
- Ziele: Chrome und Firefox, Windows, Linux, macOS. Mindestens 60 Bilder/s in 1080p auf der Iris Xe (Basis), über
  144 auf der RTX 2070 SUPER; bis 16 Spieler je Raum, ein Raum auf einem Kern des Servers.
- Fremde Klänge oder Texturen (etwa CC0) nur mit einem Ja des Nutzers je Download. Der Server auf einem eigenen
  Rechner (etwa bei jfreund.org) nur mit einem Ja des Nutzers.
- Fertig, wenn: der Nutzer eine Runde mit Mitspielern gespielt hat, Browser und Desktop gemischt.
- **Stand 2026-11-20: Teil 1 fertig** ([GAME1.md](GAME1.md)). Arbeitsname `game1` („Spiel 1“, App-ID vorläufig
  `org.jfreund.game1`), bis der Nutzer den Namen wählt. `apps/game1` (Fenster unter Windows, Linux, macOS und im
  Browser), `apps/game1-server` (ohne Fenster, unter Linux statisch, ohne `--bind` nur auf 127.0.0.1) und die
  Spiellogik ohne GPU in `apps/game1/crates/game1_core` (Spielcode, keine Engine: bleibt aus `lib/`). Eine Karte
  („Werft“, 56 × 40 m, 12 Startpunkte) und die Figur aus starren Teilen in JFScript, mit `modeltool` und
  `game1 --bake` zu eingecheckten `.g1m`-Dateien gebacken (`tools/content.py --check` baut sie Byte für Byte nach);
  Client und Server kollidieren gegen dieselben Bytes. Die Bewegung ist der Character-Controller hinter `Movement`
  auf dem Raster von 1/1024 m; eine Hitscan-Waffe mit Lag-Ausgleich, Leben, Munition, Nachladen, Tod und Rückkehr,
  jeder gegen jeden mit Punktetafel, Bots auf dem Server; Look C, die Waffe vor der Kamera, Partikel, HUD, Start-,
  Pausen- und Einstellungsmenü; Üben gegen sieben Bots ohne Server, auch im Browser. `lib/` blieb unverändert.
  20 240 Prüfungen auf Windows und macOS, dieselben Skripte unter Linux, in Node 20 233; fünf Prüfsummen gleich auf
  Windows, Linux, macOS und in Node. Auf sauberer Leitung keine Korrektur; Treffer auf ein seitlich laufendes Ziel
  mit Lag-Ausgleich bis 200 ms Umlauf zu 100 %, bei 250 ms zu 23 % (die Sicht des Schützen liegt dann rund 390 ms
  zurück, über der Grenze von 300 ms). Auf der RTX 2070 SUPER: 1080p mit 16 Figuren 0,7 ms GPU-Zeit je Bild; der
  Server 140 µs je Tick mit 16 Bots; je Spieler 2,75 KB/s hinauf, 5 KB/s herunter. Offen: Teil 2 (weitere Karten
  und Waffen, Teams und Runden, Tastenbelegung, Klang), Messungen auf Iris Xe, HD 610 und dem Mac, ein echter
  Browser gegen den Server, die Runde des Nutzers mit Mitspielern.
- Vom Nutzer entschieden am 24.11.2026 (Fragen nach Teil 1): der Arbeitsname `game1` und `org.jfreund.game1`
  bleiben bis zur ersten Veröffentlichung (danach kostet ein Wechsel den Umzug gespeicherter Einstellungen); die
  Waffe vor der Kamera wird kleiner und rückt nach rechts unten, die Mitte bleibt frei (umgesetzt am 24.11.2026:
  Variante B, 0,7 der alten Größe, in jedem Fensterformat rechts unten; [GAME1.md](GAME1.md)); die Werft wird in Teil 2
  mit den Booleschen Operationen aus G7g gebaut (echte Öffnungen statt Kisten, die Prüfsumme der Karte ändert
  sich dabei gewollt).

### Phase 2: Bewegung und große Welten

**G7i Bewegung.** Skins, Animationen und Morph-Ziele in `lib/gltf` lesen; Skinning in `render3d` (auf der GPU, mit
Verdeckung und Schatten); `lib/anim`: Clips abtasten, überblenden, ein Zustandsautomat, Bewegung aus der Wurzel,
einfache IK für Füße und Blick. Physik 2: Starrkörper (Kisten, Fässer), Gelenke, Ragdolls; die Starrkörper und Gelenke sind
seit U18 da ([PHYSICS.md](PHYSICS.md), „Starrkörper“), ein Ragdoll ist ein Satz von Gelenken, sein Bau aus einem Skelett kommt
mit `lib/anim`.

**G7j Große Welten.** Die Welt in Zellen, im Hintergrund geladen und entladen; der Ursprung folgt der Kamera von
selbst. Komprimierte Texturen (BC7 auf dem Desktop, ASTC auf Apple und Telefonen, im Browser nach
`texture-compression-*`), ein Kodierer im Werkzeug, KTX2 lesen; Textur-Streaming nach Bedarf. Gelände (Höhenfeld,
Clipmaps, gemischte Materialien), Vegetation als Instanzen mit Detailstufen und Impostoren, Netze vereinfachen für
Detailstufen (`lib/geom`), Verdeckung für die Schatten-Kaskaden.

### Phase 3: Realismus (Stufe Voll)

**G7k Bild.** TAA mit Bewegungsvektoren, ein HDR-Ziel mit Nachbearbeitung (Bloom, automatische Belichtung,
Farbkorrektur), GTAO, Reflexionen im Bildschirmraum, Cluster-Shading für viele Lichter, Schatten für Punkt- und
Spotlichter in einem Atlas, Überblenden zwischen Kaskaden.

**G7l Licht und Luft.** Eine physikalische Atmosphäre (Himmel und Luftperspektive), Volumennebel, globale
Beleuchtung über ein Gitter von Sonden (mit `ray_query`, wo die GPU es kann, sonst gerastert), Reflexionssonden.

**G7m Virtuelle Geometrie.** Netze in Cluster zerlegt, eine Hierarchie der Cluster mit Fehlermaß, Sichtbarkeit je
Cluster auf der GPU gegen die Hi-Z-Pyramide, Mesh-Shader auf Voll eingeschaltet, sonst ein Weg über Compute,
Streaming der Cluster. Das Prinzip, das Nanite in Unreal bekannt gemacht hat; mehrere Teilstufen.

### Phase 4: Stadt

**G7n Prozedurale Stadt mit KI-Schleife.** In JFScript auf `lib/geom`: Straßennetz, Blöcke, Parzellen, eine
Grammatik für Gebäude (Baukörper, Geschosse, Fassadenfelder, Module), Dächer, Straßenmöbel, Bäume, prozedurale
Materialien (Ziegel, Putz, Asphalt). Die Schleife: Beschreibung, Skript, Bilder aus festen Kameras und Messwerte,
Kritik, Änderung. Budgets je Zelle, die Zellen gehen in das Streaming aus G7j. Am Ende eine begehbare Stadt-Demo.

### Phase 5: IDE und Auszug

**G7o Editor / IDE.** Mit `lib/ui`: der JFScript-Editor mit den Sprachdiensten von JFScript, Vorschau in einer
`gpu_view`, Szenenbaum, Inspektor, Griffe zum Verschieben und Drehen, Asset-Browser, Spielen im Editor.

**G7p Auszug.** Engine (`lib/game`), Werkzeuge, Spiele und IDE in eigene Repositories mit `jflib` von crates.io, wie
PaintCraft zu JFPainter. `render3d` bleibt (E2); für `physics`, `anim` und `geom` gilt der Vorschlag in der Tabelle
[Crates](#crates), entschieden wird beim Auszug.

## Offen

Später zu entscheiden: der Faktor der Mausbewegung unter macOS (G7b Teil 2, von Hand), Name (vor der ersten
Veröffentlichung) und Server von Spiel 1 (G7h), Kartendaten aus OpenStreetMap für G7n (Lizenz ODbL), welche Crates
beim Auszug in JFLib bleiben (G7p).

## Arbeitsweise und Tests

- Jede Stufe baut ein Agent in einem eigenen Worktree (Opus 5.5 auf xhigh, höchstens drei auf einmal, Modell und
  Wellengröße werden vorher genannt). Durchsicht, Merge auf einem Prüf-Zweig, die Suiten, Push nach einem Ja des
  Nutzers.
- Jede Bibliothek hat eine Test-App ohne Fenster mit unabhängigen Referenzen, wie `render3dtest` mit seinem
  Strahlenwerfer: Brute-Force für die Physik, eine simulierte Leitung für das Netz, Bezugsbilder für das Modellieren.
- Gebaut wird für alle Ziele; echte Fenster sparsam und ohne Wiederholungsschleifen, keine echten Eingaben durch
  Agenten. Das Gefühl von Maus, Pad und Bewegung prüft der Nutzer von Hand.
- Gemessen wird auf der RTX 2070 SUPER, der Iris Xe, der T500, der HD 610 und im Browser; die Werte kommen in diesen
  Plan und in die Dokumentation der Crates.

## Bewusst nicht

Konsolen (ihre SDKs stehen unter NDA), VR, Pfadverfolgung als Hauptweg, Anti-Cheat über die Entscheidung des Servers
hinaus, Konten, Bezahlen und Shop, Matchmaking-Dienste, Telefone in Spiel 1 (Touch-Steuerung kommt später), ein Import
aus Unreal oder anderen Engines, Nachbauten bestehender Spiele.

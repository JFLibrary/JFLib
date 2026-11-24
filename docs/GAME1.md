# Spiel 1 -- der spielbare Kern (G7h Teil 1)

Spiel 1 ist das erste Spiel der Spiel-Engine ([GAME.md](GAME.md), Stufe G7h): ein schneller Mehrspieler-Shooter in
Cartoon-Grafik, jeder gegen jeden, im Browser und auf dem Desktop. „Spiel 1“ und `game1` sind Arbeitsnamen; den Namen
wählt der Nutzer (ohne fremde Marken). Name, Figuren, Karte und Waffe sind eigen, nichts ist nachgebaut, nichts
heruntergeladen: Karte und Figur sind JFScript für [`apps/modeltool`](../apps/modeltool/README.md), alles andere ist
Code.

Stand: Teil 1 (dieser Lauf). Spielbar sind eine Karte, eine Waffe, Räume per Code, Bots auf dem Server, das Üben gegen
Bots ohne Server, HUD, Start-, Pausen- und Einstellungsmenü, im Fenster unter Windows, Linux und macOS und im Browser.
Was Teil 2 bringt, steht unter [Für Teil 2](#für-teil-2). Seit G7e Teil 2 (Netcode): Abschussliste und Treffermarke als
zuverlässige Ereignisse, Raum verlassen mit LEAVE, die eigene Figur mit geglätteten Korrekturen, die Räume des Servers auf
Threads, Clients ohne Fenster für Messungen (`--client`), der Browser als Client gegen den Server geprüft.

| Teil | Wo |
| --- | --- |
| Regeln: alle Zahlen | [`rules.rs`](../apps/game1/crates/game1_core/rules.rs) |
| Karte und Modelle (`.g1m`), die Physik-Welt der Karte | [`map.rs`](../apps/game1/crates/game1_core/map.rs) |
| Bewegung und Waffe hinter `Movement` | [`movement.rs`](../apps/game1/crates/game1_core/movement.rs) |
| Der Raum (`RoomSim`): Treffer, Tod, Rückkehr | [`sim.rs`](../apps/game1/crates/game1_core/sim.rs) |
| Bots auf dem Server | [`bot.rs`](../apps/game1/crates/game1_core/bot.rs) |
| Der Server auf echten Sockets | [`host.rs`](../apps/game1/crates/game1_core/host.rs), [`apps/game1-server`](../apps/game1-server/README.md) |
| Der Client: Verbindung, Vorhersage, Effekte, HUD-Zustand | [`play.rs`](../apps/game1/src/play.rs), [`local.rs`](../apps/game1/src/local.rs) |
| Das Bild: Look C, Figuren, Waffe, Partikel | [`scene.rs`](../apps/game1/src/scene.rs), [`look.rs`](../apps/game1/src/look.rs) |
| Fenster, Menüs, HUD | [`app.rs`](../apps/game1/src/app.rs) |
| Karte und Figur in JFScript, gebacken | [`content/`](../apps/game1/content/werft.jfs), [`bake.rs`](../apps/game1/src/bake.rs), [`tools/content.py`](../apps/game1/tools/content.py) |
| Tests und Messungen | [`t_map.rs`](../apps/game1/src/t_map.rs), [`t_move.rs`](../apps/game1/src/t_move.rs), [`t_match.rs`](../apps/game1/src/t_match.rs), [`t_net.rs`](../apps/game1/src/t_net.rs), [`t_render.rs`](../apps/game1/src/t_render.rs), [`bench.rs`](../apps/game1/src/bench.rs), [`loopback.rs`](../apps/game1/src/loopback.rs) |

## Spielen

```sh
python build_rs.py run game1                        # Fenster: „Üben gegen Bots“ oder einem Server beitreten
build/win/game1-server.exe --bots 4                 # Server auf 127.0.0.1:27960, Räume mit vier Bots
build/win/game1-server.exe --bind 0.0.0.0 --web build/web/game1   # im LAN, dazu die Browser-Ausgabe auf http://…:27960/
build/linux/game1-server --bind 192.0.2.10 --port 47320 --log     # mit Zählern je Spieler alle 10 s (Messungen)
build/win/game1.exe --client 192.0.2.10:47320 --clients 3 --secs 60   # Clients ohne Fenster: Umlauf, Schwankung, Verlust, Korrekturen
```

Im Startmenü: Name, Server (Adresse:Port; im Browser spielt man auf dem Server, von dem die Seite kam), Raumcode.
„Raum erstellen“ macht einen Raum, sein Code steht oben links und im Pausenmenü; „Raum beitreten“ nimmt den Code (auch
als Link: `#CODE`, `?room=CODE`). Steuerung: ein Klick in die Ansicht fängt den Zeiger (der Klick schießt nicht), WASD
nach der Lage der Tasten, die Maus schaut, linke Taste schießt, R lädt nach, Leertaste springt (gehalten bei jeder
Landung), Strg oder C duckt, Umschalt geht leise, Tab hält die Punktetafel, Esc pausiert (der Lock endet: Pausenmenü).

## Crates

- **`apps/game1`** (Client, `org.jfreund.game1` als Arbeits-Id): Fenster, Bild, Menüs, die Vorhersage, das Üben,
  die Test-Gruppen und Messungen (`--check`, `--sums`, `--bench`, `--loopback`), das Backen (`--bake`). Windows,
  Linux (glibc, wegen der GPU), macOS, Browser.
- **`apps/game1-server`** (`org.jfreund.game1_server`): der Server, ein Konsolenprogramm; unter Linux statisch ohne
  libc wie die anderen Server, auch Windows. Er nennt nie `render3d` oder `gpu`.
- **`apps/game1/crates/game1_core`** (lokales Crate, beide nennen es): Regeln, Karte, Bewegung, Raum, Bots, die
  Server-Schleife. Ohne GPU, `#![no_std]`, `#![forbid(unsafe_code)]`.

Warum ein lokales Crate unter `apps/game1` und nicht in `lib/`: Es ist Spiel, keine Engine (E3: Spiel-Bausteine in
eigene Crates, das Spiel selbst in `apps/`); beim Auszug (G7p) geht es mit dem Spiel ins eigene Repository. Warum
nicht ein zweites Programm in `apps/game1/BINS`: Der Plan nennt `apps/<spiel1>-server`; ein eigenes App-Verzeichnis
gibt dem Server eigene PACKAGE, PERMISSIONS und Tests. Cargo vereinigt keine Features, die es nicht gibt: Der Server
nennt `game1_core`, `game`, `netchan`, `netio`, nie den Client, also kommt keine GPU in seinen Abhängigkeitsgraphen.

## Karte

**Gewählt: JFScript → modeltool → `.glb` → `game1 --bake` → `.g1m`, eingecheckt.** Weder Client noch Server führen
JFScript beim Start aus; beide lesen dieselbe kleine Binärdatei (75 KB für die Karte, 40 KB für die Figur) und bauen
daraus dasselbe Dreiecksnetz der Kollision aus denselben `f32` (Prüfsumme des Netzes in den Tests). Gründe:

- **modeltool bleibt die eine JFScript-Anbindung** (E7): Die Karte nutzt `geom.*` und `model.*`, wie sie sind, und
  bekommt die Bilder (Ansichten vorn, Seite, oben, schräg) und Prüfungen (geschlossen, Normalen außen, Budget, Maße)
  umsonst, die Rückmeldeschleife für KI wie für Menschen. Eine eigene Anbindung in Spiel 1 hätte modeltools 900
  Zeilen Host-Funktionen verdoppelt.
- **Was eine Karte mehr braucht als ein Modell, steht in den Namen der Teile**: `@spawn 90` (ein Startplatz, Blick
  90° links von −Z), `@weg` (Wegpunkt der Bots), `@muendung` (die Mündung der Waffe); `#deko` (gezeichnet, ohne
  Kollision), `#unsichtbar` (Kollision, nicht gezeichnet), `#rand` (Randlicht wie Figuren und Requisiten). So
  braucht modeltool keine Änderung.
- **Das Format `.g1m`** (Materialien, Teile mit Positionen, Normalen, Indizes, Marker, Verbindungen der Wegpunkte)
  liest der Server ohne glTF (kein JSON, keine Bilddecoder im statischen Server). Die Verbindungen der Wegpunkte
  rechnet das Backen einmal mit `lib/physics` (eine Kugel von 0,3 m kommt in 1 m Höhe durch, der Boden entlang der
  Linie ist da und springt nirgends um mehr als 0,45 m).
- **Neu bauen gibt dieselben Bytes**: `python apps/game1/tools/content.py --check` baut beide Skripte neu und
  vergleicht Byte für Byte (geprüft: gleich). Das ist kein Test von `build_rs.py test game1`, weil der modeltool und
  JFScript braucht; die Tests prüfen die eingecheckte Datei (Rücklesen Byte für Byte, Prüfsumme der Kollision,
  Startplätze, Wegpunkte, 20 000 Strahlen).

**Die Werft** ([`werft.jfs`](../apps/game1/content/werft.jfs)): ein Hof von 56 × 40 m zwischen Ziegelmauern (4,5 m),
in der Mitte eine Plattform (1,6 m) mit zwei Rampen und einer Deckung oben, im Norden ein Steg auf 3 m mit
Geländern, Pfeilern und Treppen an beiden Enden, fünf Container (einer auf einem anderen), Kisten und Fässer mit
Randlicht, sieben niedrige Deckungsmauern, gelbe Linien. 1520 Dreiecke gezeichnet, 1472 in der Kollision, 12
Startplätze am Rand (jeder blickt zur Mitte), 33 Wegpunkte mit 81 Verbindungen (ein zusammenhängender Graph).

**Die Figur** ([`figur.jfs`](../apps/game1/content/figur.jfs)): 1,8 m wie die Kapsel, aus starren Teilen ohne
Skelett: Beine, Rumpf, Kopf mit Mütze und Augen, Arme, Hände, die Waffe (ein „Funkenwerfer“ mit leuchtendem Tank). Das
Spiel bewegt die Teile: die Beine um die Hüfte (Gehen, in der Luft angezogen), Arme, Hände und Waffe um die Schulter
mit dem Nicken, der Kopf mit 40 % davon; geduckt werden die Beine gestaucht und der Rest sinkt um 0,6 m. Die Jacke
bekommt je Spieler eine von acht Farben.

## Bewegung

`movement::Mover` setzt den `CharacterController` aus `lib/physics` (G7c Teil 2) hinter `game::net::Movement`
(NETCODE.md: „der Controller aus G7c kommt mit G7h“). Der Zustand `MoveState` sind 8 ganze Zahlen: Füße und
Geschwindigkeit auf dem Raster von 1/1024 m und 1/1024 m/s, Flags (geduckt, am Boden, tot, geschossen, gesprungen,
gelandet; die Knöpfe des Ticks für die Kante von R) und die Waffe (Patronen, Abklingen, Nachladen, ein Zähler der
Schüsse). Nach jedem Tick wird gerundet; der Controller liest nur Lage, Geschwindigkeit und Ducken, und eine Rundung
unter einem Millimeter steckt die Kapsel nie in etwas (ihre Haut ist 5 mm). **Die Waffe gehört zum vorhergesagten
Zustand**: Abklingen, Magazin und Nachladen rechnet der Client Bit für Bit wie der Server, sein Mündungsfeuer kommt also
sofort und stimmt mit dem Schuss des Servers überein; ob er trifft, entscheidet der Server. Eine tote Figur steht in
`Mover::step` still (Flag im Zustand), also sagt der Client auch den Tod ohne Abweichungen voraus, bis der Server sie
zurückbringt.

Startwerte (die Vorgaben von `CharacterParams`, „für einen schnellen Cartoon-Arena-Shooter gestimmt“; der Nutzer
stimmt sie von Hand):

| Wert | Start | |
| --- | --- | --- |
| Kapsel | Radius 0,4 m, 1,8 m hoch, geduckt 1,2 m | Auge 0,15 m unter dem Scheitel |
| laufen (Vorgabe), gehen (Umschalt), geduckt | 9, 5, 3 m/s | |
| Beschleunigung, Reibung, Stopp | 12 /s, 6 /s, 2,5 m/s | |
| Luftsteuerung | 0,3 | Strafe-Jumping wie in Quake 3 |
| Schwerkraft, Sprunghöhe | 20 m/s², 1,2 m | Absprung 6,93 m/s, 0,69 s in der Luft |
| Stufen, Rampen | 0,4 m, 45° | |
| Sprung gehalten | springt bei jeder Landung | Bunny-Hopping |

## Waffe und Raum

Startwerte ([`rules.rs`](../apps/game1/crates/game1_core/rules.rs)):

| Wert | Start |
| --- | --- |
| Leben | 100 |
| Schaden Körper / Kopf (obere Halbkugel der Kapsel) | 18 / 36 (6 bzw. 3 Treffer) |
| Schüsse | alle 7 Ticks (8,6 je Sekunde), Hitscan, ohne Streuung, 150 m |
| Magazin, Nachladen | 20, 1,4 s (automatisch bei 0, R jederzeit); Vorrat ohne Ende |
| Rückkehr nach dem Tod | 2 s, am Startplatz mit dem größten Abstand zu den Lebenden |
| Schutz nach der Rückkehr | 1,5 s oder bis zum ersten Schuss |
| Lag-Ausgleich | höchstens 300 ms (18 Ticks), wie entschieden |
| Figuren je Raum | 16 (Spieler und Bots) |

Ein Tick des Raums (`sim::Match`, ein `RoomSim`): Wer an der Reihe ist, kommt zurück; jeder Bot denkt (aus dem Stand
am Anfang des Ticks); jede Figur bewegt sich mit ihrer Eingabe (nach Id); die Trefferzonen nach dem Tick werden
gemerkt, stehende und geduckte Kapseln getrennt (`HitHistory` kennt nur eine Form); dann jeder Schuss des Ticks (Flag
im Zustand) nach Id: die anderen auf die Sichtzeit des Schützen zurückgestellt, ein Strahl gegen Karte und Kapseln,
der nächste Treffer zählt. Wer unter `KILL_Y` fällt, ist auch raus.

**Was der Client davon sieht**, steht in den Feldern der Figur (14): die 8 der Bewegung, Gier, Nicken, Leben,
Abschüsse, Tode und ein Feld, ob sie geschützt ist und in welche Richtung der Startplatz blickt (der Client dreht seine
Kamera einmal je Leben dorthin). **Was geschieht**, kommt seit G7e Teil 2 als zuverlässiges Ereignis des Netcodes (je
einmal, in Reihenfolge, über UDP und WebSocket; `rules::GameEvent`): jeder Abschuss an alle (Täter, Opfer, Kopf: die
Abschussliste, „X hat dich erwischt“), jeder Treffer an seinen Schützen (Opfer, Kopf, Abschuss: die Treffermarke), Spieler,
die kommen und gehen (mit Namen). Vorher standen Treffer, Kopftreffer und der Täter als Zähler im Feld der Figur (zwei
Ereignisse zwischen zwei Schnappschüssen fielen zusammen). **Namen** sind Entitäten einer zweiten Art (16 Felder, 60
Bytes UTF-8, Id 512 + Figur): Sie ändern sich nie und kosten nach dem ersten Schnappschuss nichts. Die Bots haben
ausgedachte Namen („Bot Kiesel“, „Bot Pfeffer“, …).

**Bots** (`bot::Brain`, auf dem Server): wandern zwischen den Wegpunkten (nie zurück, wenn es einen anderen Nachbarn
gibt), bemerken eine Figur in Sicht (Strahl durch die Karte) und im Blickfeld (±69°) bis 60 m, reagieren nach 14 bis
24 Ticks (233–400 ms), drehen mit höchstens 7 rad/s auf sie zu, mit einem Zielfehler von anfangs bis zu ±5,7° (Nicken
die Hälfte), der je Tick auf 94 % schrumpft, dazu ±0,23° Zittern; sie schießen, wenn sie innerhalb von 1,4° zielen,
weichen seitlich aus, halten 8–18 m Abstand, springen manchmal, laden in ruhigen Momenten nach, und wer festhängt
(0,75 s kaum bewegt), springt und nimmt einen anderen Wegpunkt. Ihr Zufall kommt aus ihrem Seed: Ein Spiel mit Bots ist
auf jedem Ziel dasselbe. Der Server füllt jeden Raum mit `--bots N`, ein Spieler nimmt den Platz eines Bots, der Bot
kommt zurück, wenn der Spieler geht.

## Client

- **Bild**: `render3d` in einer `ui.gpu_view` über das ganze Fenster (der Weg von `apps/gamedemo`), HUD und Menüs mit
  `ui` darüber. Look C, wie entschieden: `Toon { bands: 4, softness: 0.5, rim: 0.2, rim_width: 0.35 }` auf Figuren,
  Waffe und den Teilen mit `#rand`, Randlicht 0 auf Boden und Wänden, keine Umrisse, FXAA, Schatten der Sonne nach der
  Stufe des Geräts. Die Waffe und die Hände der Figur vor der Kamera (render3ds Viewmodel, eigene Linse 57°), klein
  unten rechts auf jedem Fensterformat ([Die Waffe vor der Kamera](#die-waffe-vor-der-kamera)), mit Wippen beim Laufen,
  Rückstoß und Senken beim Nachladen. Partikel: Mündungsfeuer (vor der Kamera und an den Waffen der anderen),
  Leuchtspuren, Funken und Staub, wo ein Schuss die Karte trifft, eine Wolke in der Farbe der Figur, wenn sie raus ist.
- **HUD**: Leben (Zahl und Balken), Munition („17 / 20“, „lädt …“), Fadenkreuz, Treffermarke (weiß, gelb für Kopf,
  rot für einen Abschuss), roter Rand bei Schaden, Abschussliste oben rechts (6 s; auch „X ist dabei“, „X hat den Raum
  verlassen“), Raumcode und Umlauf oben links, „X hat dich erwischt“, Punktetafel mit Tab. Treffermarke und Abschussliste
  kommen aus den Ereignissen des Raums.
- **Die eigene Figur** zeigt das Bild zwischen den letzten zwei vorhergesagten Ticks und mit dem abklingenden Rest einer
  Korrektur (`NetClient::smoothing`, 60 ms, über 1 m springt es: die Rückkehr nach dem Tod); die Vorhersage selbst bleibt
  Bit für Bit die des Servers.
- **Raum verlassen** (Pausenmenü) schickt LEAVE: Der Server nimmt die Figur sofort heraus, die anderen lesen es in der
  Abschussliste.
- **Menüs**: Start (Name, Server, Code; erstellen, beitreten, üben, Einstellungen, beenden), Pause (weiter,
  Einstellungen, Raum verlassen), Einstellungen (Empfindlichkeit 0,05–10, Sichtfeld 70–120° waagrecht bei 16:9,
  Maus umkehren). Gespeichert in `config://settings.ini`: die Belegung und die Maus mit `Input::store` (lib/game), Name,
  Server und Sichtfeld im Abschnitt `game1`. Ein Lauf mit `--script` nimmt `mem://` und liest und schreibt die
  Einstellungen der Person nie.
- **Üben**: ein Server mit sieben Bots im selben Prozess über die simulierte Leitung (1 ms je Richtung) – derselbe
  Weg wie online (Vorhersage, Schnappschüsse), überall, auch im Browser.
- **Browser**: der Client baut für den Browser und spricht dort über den WebSocket `/game1` des Servers, von dem die
  Seite kam (`game1-server --web build/web/game1` liefert sie aus), auf dem Desktop über UDP (E8). Über einen WebSocket
  läuft seine Uhr einen Tick weiter vor (3 statt 2: TCP hält zurück, was einem verlorenen Segment folgt; NETCODE.md).
- **Clients ohne Fenster** (`game1 --client SERVER`, [`client.rs`](../apps/game1/src/client.rs)): Clients, die der
  Verstand der Bots spielt, für eine Zeit gegen einen Server, dann je Client Umlauf, Schwankung, Verlust, verglichene
  Vorhersagen, Korrekturen, Marge, Ereignisse; auf dem Desktop und im Browser (die Seite ohne Fenster,
  `tests/browser.mjs`). Damit lief das echte Netz zwischen zwei Rechnern.
- **Der Server** rechnet die Räume eines Ticks nebeneinander auf dem Pool von `plat` (ein Raum je Kern, dieselben
  Nachrichten wie auf einem Thread) und zieht Raumcodes und Tickets aus dem Zufall des Systems; `--log` schreibt
  Verbindungen und die Zähler je Spieler.

### Die Waffe vor der Kamera

Entschieden am 2026-11-24: kleiner und weiter rechts unten, die Mitte (Fadenkreuz, Ziele) bleibt frei; Look C und die
Figur, wie die anderen sie sehen, bleiben. Gezeichnet werden die Teile `waffe`, `tank` und `haende` der Figur mit
render3ds Viewmodel und seiner eigenen Linse (57° senkrecht, `look::VM_FOV`): Das Sichtfeld der Einstellungen
ändert die Waffe nicht. Ihren Platz beschreibt `VmLayout` in [`look.rs`](../apps/game1/src/look.rs): ein Maßstab und
wo die Mitte ihres Kastens auf dem Bildschirm liegt, `right` vom rechten und `bottom` über dem unteren Rand, gemessen
in Bildschirmhöhen, 0,62 m vor dem Auge.

- **Jedes Format**: Auf 16:10, 4:3 und 21:9 liegt sie so weit von der Ecke wie auf 16:9. Ein Fenster schmaler als
  4:3 (hoch, ein schmales Browserfenster) misst in Breite / (4/3): Die Waffe wird mit der Breite kleiner und bleibt in
  der Ecke. Breiter als 16:9 liegt sie weiter außen und dreht sich um so viel nach außen, dass man sie von derselben
  Seite sieht wie auf 16:9 (sonst wüchse ihr hinteres Ende auf 21:9 in die Munitionsanzeige).
- **Bewegung**: Wippen, Rückstoß und Senken wachsen und schrumpfen mit dem Maßstab (gestimmt waren sie für 0,55), das
  Mündungsfeuer auch; es kommt aus der Mündung (`@muendung`) in Richtung des Laufs.
- **Leuchtspur**: Die eigene beginnt an dem Punkt der Welt, den das Bild an der Mündung zeigt (`look::vm_to_world`,
  auf jedem Format und bei jedem Sichtfeld). Getroffen wird weiter vom Auge aus (Hitscan): Am Spiel, am vorhergesagten
  Zustand und an den fünf Prüfsummen ändert sich nichts.

| Variante | Maßstab | `right` | `bottom` | Kasten auf 16:9 (1280 × 720, in Ruhe, etwa) |
| --- | --- | --- | --- | --- |
| heute (G7h Teil 1) | 0,55 | (Kamera x 0,15, y −0,16) | | x 736–927, y 385–697 |
| A (0,6 ×) | 0,33 | 0,36 | 0,12 | x 953–1129, y 550 bis unter den Rand |
| **B (0,7 ×), eingebaut** | 0,385 | 0,39 | 0,14 | x 925–1125, y 521 bis unter den Rand |
| C (0,8 ×) | 0,44 | 0,42 | 0,17 | x 900–1120, y 485 bis unter den Rand |

Eine andere Variante ist eine Zeile: `VM_LAYOUT = VM_VARIANTS[i].1` in `look.rs`; die Gruppe `render` hält alle drei.
Die Bilder zur Wahl (derselbe Moment der Übung: heute, A, B, C; B auf 16:10, 4:3, 21:9 und 540 × 960; Nachladen)
liegen außerhalb des Repositorys. Offen: In einem schmalen hohen Fenster liegt die Munitionsanzeige (die HUD misst
in Höhen) über der kleinen Waffe.

## Tests

`python build_rs.py test game1` führt `tests/check.uis` (`--check`: sechs Gruppen), `tests/loopback.uis` (echte Sockets
auf 127.0.0.1) und `tests/shot.uis` (das Fenster ohne Bildschirm: Startmenü, Üben, gehen, schießen, Pause, zurück) aus;
`python build_rs.py test game1-server` dessen `--check`.

| Gruppe | Prüft |
| --- | --- |
| `map` | die eingebackene Karte und Figur Byte für Byte zurück, ihre Teile und Marker; jeder Startplatz frei (stehende Kapsel) und auf dem Boden, je 4 m auseinander; jeder Wegpunkt auf dem Boden mit einem Nachbarn, der Graph zusammenhängend; Prüfsumme des Kollisionsnetzes gegen den festen Wert; 20 000 Strahlen als Prüfsumme; 2000 kaputte Dateien (abgeschnitten, gekippte Bits, Zufall) ohne Panik |
| `move` | 20 000 Zustände durch die Felder und zurück; 16 zufällige Figuren über 1200 Ticks zweimal gleich, und ab den Feldern der Mitte weitergerechnet wieder gleich (was ein Client nach einem Abgleich tut); im Hof und über dem Boden; laufen 9 und gehen 5 m/s (10⁻²); ein Sprung landet nach 38–45 Ticks; ducken senkt das Auge um 0,6 m; die Schüsse genau alle 7 Ticks, nach dem 20. das Nachladen und der 21. Schuss genau 85 Ticks danach; R lädt nach; eine tote Figur steht und schießt nicht |
| `match` | 16 Bots eine Minute: sie schießen, treffen, beenden einander und kommen zurück, Tode = Abschüsse + Stürze, keiner steckt in der Karte, Leben in 0–100, mindestens 8 weit weg von ihrem Start, zweimal gleich; ein Spieler in einem vollen Raum nimmt den Platz eines Bots, der beim Gehen zurückkommt; ein Schuss von Hand: 18 Schaden am Körper, 36 am Kopf, die Zähler, raus nach genug Treffern, Rückkehr nach genau 120 Ticks mit Leben, Magazin und Schutz; Lag-Ausgleich im Raum: ein Schuss dorthin, wo das Ziel vor 10 Ticks war, trifft mit dieser Sicht und ohne nicht |
| `netsim` | Clients mit dem Verstand der Bots über die simulierte Leitung, Referenz ist der Server selbst: klar 4 Clients (einer über WebSocket) – jede erste Vorhersage gleich dem Zustand des Servers, keine Korrektur; typisch 10 s – beschränkt, und bei jeder Korrektur bewegt sich die gezeichnete Figur nicht (Glätten); danach klar – wieder jede Vorhersage exakt; ein Client verlässt den Raum – seine Figur ist 21 ms nach dem LEAVE beim Server weg, die anderen lesen es genau einmal; mit Schießen (4 Bots auf dem Server, 4 Clients) – alle spielen, keine fehlende Basis, kein kaputter Schnappschuss, keiner über 1200 Bytes, Abschüsse; jeder Abschuss seit dem Beitritt kommt bei jedem Client genau einmal und in Reihenfolge an, jeder Treffer bei seinem Schützen (gegen das Buch des Raums) |
| `lagcomp` | ein Client schießt auf einen anderen, der mit 9 m/s hin und her läuft, und zielt dorthin, wo er ihn sieht: Umlauf 50, 100, 150, 200, 250 ms, mit und ohne Lag-Ausgleich |
| `render` | ohne GPU: jede Ecke der Waffe durch ihre Linse auf neun Formaten von 9:21 bis 32:9, in Ruhe, mit Rückstoß, gesenkt und wippend – ganz rechts unter dem Fadenkreuz, ein Quadrat von 0,15 Bildschirmhöhen um es frei, in Ruhe links von der Munitionsanzeige; die eigene Leuchtspur beginnt auf dem Bild an der Mündung (Sichtfeld 70–120°). Mit GPU das Bild offscreen aus fester Kamera: dasselbe Bild zweimal gleich, Himmel oben, der Hof unten bunt, sechs Figuren ändern die Mitte, die Waffe nur im Viertel unten rechts und nicht um das Fadenkreuz (16:9, 4:3, 21:9, schmal), Partikel zeigen sich (kein Bezugsbild) |

Ergebnisse nach G7e Teil 2 (auf Windows, Linux, macOS und in Node dieselben Zahlen): Klar: 1423 erste Vorhersagen
verglichen, 0 verschieden, 0 Korrekturen. Typisch 10 s: 9 Korrekturen (13,5 je Minute und Client), größter Fehler 4 mm, alle
geglättet, die gezeichnete Figur bewegte sich dabei um 0,0 m; danach wieder jede Vorhersage exakt. Verlassen: 21 ms. Mit
Schießen, typisch 15 s: 159 Schüsse, 108 Treffer, 20 Abschüsse, Schnappschüsse bis 329 Bytes; 84 Abschüsse und 59 Treffer
als Ereignisse an 4 Clients, je einmal und in Reihenfolge. (Teil 1: 1424 verglichen, typisch 10 Korrekturen und 9 cm, mit
Schießen 165 / 101 / 17: die Marge von 3 Ticks des WebSocket-Clients ändert die Sitzung.) Lag-Ausgleich (unverändert):

| Umlauf | Sicht hinter dem Tick | Treffer mit Ausgleich | ohne |
| --- | --- | ---: | ---: |
| 50 ms | 191 ms | 100 % | 10 % |
| 100 ms | 232 ms | 100 % | 8 % |
| 150 ms | 291 ms | 100 % | 10 % |
| 200 ms | 332 ms (auf 300 gezogen) | 100 % | 8 % |
| 250 ms | 391 ms (auf 300 gezogen) | 23 % | 10 % |

Die Sicht liegt rund Umlauf + 140 ms hinter dem Tick (100 ms Interpolation, 2 Ticks Marge, das Alter des
Schnappschusses). Mit der Grenze von 300 ms trifft ein Schuss auf ein Ziel, das mit voller Geschwindigkeit seitlich
läuft, bis rund 200 ms Umlauf (die 30 ms darüber sind 27 cm, weniger als der Radius der Kapsel), bei 250 ms meist
nicht mehr (90 ms, 81 cm). Eine Frage an den Nutzer (unten).

## Prüfsummen

`game1 --sums` faltet: `map` (das Kollisionsnetz), `rays` (20 000 Strahlen), `move` (16 zufällige Figuren über 1200
Ticks), `match` (16 Bots eine Minute: jeder Zustand jedes Ticks, wie er im Schnappschuss steht), `net` (eine Sitzung
über die simulierte Leitung: 4 Bots auf dem Server, 3 Clients, einer über WebSocket, raues Wetter, dann klar; jede
erste Vorhersage jedes Clients, jeder Zustand des Servers, die Zähler der Clients am Ende).

| Ziel | Build | map | rays | move | match | net |
| --- | --- | --- | --- | --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | Entwicklung | `296751e538edff1f` | `864f3a7a354afce4` | `3df4a4fc907f338d` | `af3bbdb5e2f09a90` | `06883afe1ce5a42d` |
| Linux x86-64 (glibc) in WSL Ubuntu 24.04 | Entwicklung | gleich | gleich | gleich | gleich | gleich |
| macOS ARM64 (Mac mini M2, macOS 27.2), Rust 1.98.1 | Entwicklung | gleich | gleich | gleich | gleich | gleich |
| WebAssembly in Node 22.17: einfach, SIMD, Threads (`tests/web.mjs`) | Entwicklung | gleich (alle drei) | gleich | gleich | gleich | gleich |

Die Summen der Partie: 1492 Schüsse, 1144 Treffer, 198 Abschüsse; die Sitzung faltet 6942 Werte (seit G7e Teil 2 auch jedes
Ereignis, das ein Client bekam).

**G7e Teil 2 änderte zwei Summen**, aus echten Gründen: `match` von `d64fd476dd466992` zu `af3bbdb5e2f09a90`, weil das Feld
`F_INFO` keine Zähler mehr trägt (die Partie selbst ist dieselbe: 1492 / 1144 / 198); `net` von `834566b66cd092bc` zu
`06883afe1ce5a42d`: dasselbe Feld, die Ereignisse in den Schnappschüssen und gefaltet, die Marge von 3 Ticks des
WebSocket-Clients. `map`, `rays`, `move` blieben. Die neuen Werte gleich auf Windows, Linux (WSL), macOS (M2, macOS 27.2.1)
und in Node (drei Bündel).

| Ziel | `--check` | Skripte |
| --- | --- | --- |
| Windows x86-64 | `check.uis` bestanden (Teil 1: 20 240 von 20 240) | `check`, `loopback`, `shot` bestanden; `game1-server --check` bestanden |
| Linux x86-64 (WSL) | `check.uis` bestanden (das Bild über llvmpipe) | `check`, `loopback`, `shot` bestanden; der Server statisch ohne libc, `--check` bestanden |
| macOS ARM64 (M2) | `--test map,move,match,netsim,lagcomp`: 20 248 von 20 248 | – ; `game1-server --check` bestanden |
| Node (drei Bündel) | `--test map,move,match,netsim,lagcomp`: 20 248 von 20 248 | – |
| Chrome 157, Firefox 153 (headless) | `tests/browser.mjs`: zwei Clients im Browser gegen `game1-server --web` bestanden | – |

Ein echtes Fenster lief einmal (Windows, `gpu on` ohne Aktivierung, das Skript startet das Üben, geht, schießt,
pausiert): die Ansicht über die Swapchain, 13 dargestellte Bilder zurückgelesen, keins anders als der Framebuffer.

## Messwerte

`game1 --bench` im Release-Build, Windows, Ryzen 5 3600 mit RTX 2070 SUPER; der Rechner wurde von anderen Sitzungen
mitbenutzt, drei Läufe, die Spannen stehen da.

- **Das Bild offscreen in 1920 × 1080** mit 16 Figuren (176 Instanzen der Teile, 198 mit Karte und Waffe), die gehen,
  sich drehen und schießen (Mündungsfeuer, Leuchtspuren, Funken), der Waffe vor der Kamera, Schatten, FXAA: die GPU
  braucht im p50 **0,68–0,72 ms** je Bild (ein Lauf unter Last 3,4 ms); ein Bild für sich, CPU und GPU nacheinander,
  im p50 1,1 ms (unter Last 28 ms); mit zwei Bildern in Arbeit **396 Bilder/s** (unter Last 115–176). Das Ziel „über 144
  auf der RTX 2070 SUPER“ hält offscreen weit. Im Fenster liest seit G7b Teil 2 keine Zelle mit HUD das Bild mehr
  zurück (Schleier, GAMECORE.md).
- **Der Server, ein Raum mit 16 Bots** (niemand verbunden: Bots denken, bewegen, schießen; jeder zweite Tick ein
  Schnappschuss): p50 **139–143 µs** je Tick, Mittel 293–357 µs, p99 556–598 µs (Ausreißer bis 83 ms sind Lücken der
  Planung des geteilten Rechners), also 1,8–2,1 % eines Kerns bei 60 Hz.
- **Der Server mit 16 Clients** über die simulierte Leitung (typisches Wetter, sie schießen): `NetServer::step` p50
  **174–175 µs**, Mittel 266–414 µs.
- **Bandbreite je Spieler** (16 Clients, typisch): aufwärts **2,75 KB/s** in 60 Paketen/s (mit IPv4 und UDP 4,43 KB/s),
  abwärts **4,97 KB/s** in 30 Paketen/s (5,81 KB/s); Schnappschüsse bis 612 Bytes, keiner geschnitten.

Die Messungen auf der Iris Xe, der HD 610 und dem Mac kommen mit Teil 2.

## Entscheidungen

- **Die Waffe im vorhergesagten Zustand** (siehe [Bewegung](#bewegung)): Mündungsfeuer ohne Verzögerung, ohne eigenen
  Abgleich für Munition und Abklingen. Gegenweg: Munition nur aus den Schnappschüssen, das Feuer erst nach einem Umlauf.
- **Der Tod im Zustand der Bewegung**: Die tote Figur steht still, auch in der Vorhersage; ohne das gäbe es während
  der 2 s jeden Schnappschuss eine Korrektur.
- **Ereignisse als Ereignisse** (G7e Teil 2): Abschüsse, Treffer und wer kommt und geht über den zuverlässigen Kanal des
  Netcodes; nur die Schüsse bleiben ein Zähler im Zustand der Waffe (das Mündungsfeuer der anderen gehört zum Bild, das
  100 ms zurückliegt, und ein Zähler kommt genau dann an). Bis dahin galt:
- **Ereignisse als Zähler in den Feldern** (Treffer, Kopftreffer, Schüsse im Zustand der Waffe, Tode mit dem Täter):
  Der Netcode hat keinen zuverlässigen Ereigniskanal über UDP (NETCODE.md, „Für Teil 2“); ein Zähler geht nicht
  verloren, wenn ein Schnappschuss verloren geht, nur zwei Ereignisse zwischen zwei Schnappschüssen fallen zusammen.
- **Zwei Formen im Lag-Ausgleich**: `HitHistory` kennt eine Form; der Raum führt eine für stehende und eine für
  geduckte Kapseln und nimmt den näheren Treffer. Keine Änderung an `lib/game`.
- **Namen als Entitäten**: kein neues Protokoll, sie kosten nach dem ersten Schnappschuss nichts.
- **Keine Änderung in `lib/`**: Alles, was Spiel 1 brauchte, gaben `lib/game` (Netcode, Eingabe, Kameras),
  `lib/physics` (Controller), `render3d` (Look, Waffe, Partikel) und `lib/game_render3d` (Kamera, Transformation) schon
  her.

## Was nicht getestet ist

- **Von Hand gespielt**: niemand. Gefühl der Bewegung, Empfindlichkeit, Treffermarke, Bots: das prüft der Nutzer.
  Ein echtes Fenster lief einmal mit einem Skript (oben); die Punktetafel (Tab) und die Einstellungen sah niemand im
  Bild.
- **Der Browser als Client gegen den Server** (G7e Teil 2): Chrome und Firefox headless spielten ohne Fenster gegen
  `game1-server --web` (`tests/browser.mjs`, oben); das Fenster mit dem Bild im Browser gegen den Server noch nicht. Bis
  dahin: gebaut, die Prüfsummen in Node; im Browser selbst gespielt hat ihn
  niemand.
- **Ein echtes Netz zwischen Rechnern** (G7e Teil 2): der Server auf dem Mac mini (WLAN), Clients auf dem Windows-Rechner,
  UDP und WebSocket, mit und ohne Bots: über UDP keine Korrektur ohne Schießen, jede Vorhersage exakt (NETCODE.md, „Echtes
  Netz“). Fedora als Server war nicht erreichbar.
- **Die HUD über der Ansicht** kostete bis G7b Teil 2 ein Zurücklesen je Bild; seitdem mischt die GPU-Ausgabe von
  `lib/ui` solche Zellen als „Schleier“ (GAMECORE.md, GPU.md), ohne Änderung an Spiel 1.

## Für Teil 2

Aus GAME.md (G7h): zwei weitere Karten, drei weitere Waffen (auch Geschosse mit Flugzeit), Teams und Runden, ein Menü
für die Tasten, Grafik und Lautstärke, Klang; dazu aus diesem Lauf:

- die Messungen auf der Iris Xe, der HD 610 und dem Mac;
- Figuren, die einander blockieren (heute laufen sie durcheinander); das Glätten der Korrekturen kam mit G7e Teil 2;
- Bots, die Rampen und Treppen gezielt nehmen (heute folgen sie den Wegpunkten, wer festhängt, springt), und eine
  Schwierigkeit;
- der Browser im echten Fenster gegen `game1-server --web`, gemischt mit Desktop-Spielern (Bedingung für „fertig“; ohne
  Fenster lief er mit G7e Teil 2);
- der Server auf einem eigenen Rechner (nur mit einem Ja des Nutzers).

## Fragen an den Nutzer

- **Lag-Ausgleich bei 250 ms Umlauf**: Die Sicht liegt dort rund 390 ms hinter dem Tick, die Grenze von 300 ms zieht
  sie, und ein seitlich laufendes Ziel wird meist verfehlt (23 %). Bleibt es bei 300 ms (wer so weit weg spielt, zielt
  vor), oder soll die Grenze auf rund 400 ms?
- **Name** des Spiels (und damit die App-Id statt `org.jfreund.game1`).
- **Die Figur und die Karte** an den Bildern (`build/game1-content/*/sheet.png` nach `tools/content.py`, im Spiel
  `build/game1_play.png`): Proportionen, Farben.
- **Die Waffe vor der Kamera**: B ist eingebaut; bleibt es dabei, oder A oder C ([oben](#die-waffe-vor-der-kamera))?
- **Die Startwerte** von Bewegung und Waffe (Tabellen oben) nach dem ersten Spielen von Hand.

## Name

Der Arbeitsname steht an wenigen Stellen: `rules::GAME_NAME` (Fenstertitel, Menü), `display=` und `app_id=` in den zwei
PACKAGE-Dateien, die Ordner `apps/game1`, `apps/game1-server`, `game1_core` und die Wurzel-`Cargo.toml`. Ein neuer Name
ist ein Umbenennen dieser Stellen.

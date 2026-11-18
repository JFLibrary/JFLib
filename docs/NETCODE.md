# NETCODE -- Netz für Spiele mit Vorhersage

Der Netcode der Spiel-Engine ([GAME.md](GAME.md), Stufe G7e) liegt in `lib/game` als Modul `game::net`: Der Server entscheidet
und simuliert mit fester Rate, Clients schicken ihre Eingaben mit Tick-Nummer (mehrfach, damit ein verlorenes Datagramm nichts
kostet), der Server schickt Schnappschüsse als Delta gegen den Stand, den der Client hat, quantisiert und unter 1200 Bytes. Der
Client sagt seine eigene Figur voraus und gleicht sie mit dem Server ab; die anderen Figuren zeigt er rund 100 ms hinter dem
neuesten Schnappschuss. Treffer prüft der Server mit Lag-Ausgleich: Er stellt die Trefferzonen der anderen auf die Zeit zurück,
die der Schütze sah, höchstens rund 200 ms. Räume gibt es per Code oder Link, Spieler haben einen Namen, kein Konto; Bots spielen
in den Tests. Der Transport ist [`lib/netchan`](TRANSPORT.md) (UDP und WebSocket); `lib/game` bleibt ohne GPU und ohne
Betriebssystem (`#![no_std]`, `#![forbid(unsafe_code)]`, keine fremden Crates), der Server läuft unter Linux statisch.

Stand: G7e Teil 1 (dieser Lauf). Was fehlt, steht unter [Was nicht gebaut ist](#was-nicht-gebaut-ist) und
[Für Teil 2 und G7h](#für-teil-2-und-g7h).

| Teil | Wo |
| --- | --- |
| Server: Räume, Takt, Eingaben, Schnappschüsse je Client | [`server.rs`](../lib/game/src/net/server.rs) |
| Client: Beitritt, Uhr, Vorhersage, Schnappschüsse, Interpolation | [`client.rs`](../lib/game/src/net/client.rs), [`clock.rs`](../lib/game/src/net/clock.rs), [`predict.rs`](../lib/game/src/net/predict.rs) |
| Schnappschüsse und ihr Delta | [`snapshot.rs`](../lib/game/src/net/snapshot.rs), Bitstrom [`bits.rs`](../lib/game/src/net/bits.rs), Raster [`quant.rs`](../lib/game/src/net/quant.rs) |
| Eingaben, ihre Wiederholung, der Puffer des Servers | [`input.rs`](../lib/game/src/net/input.rs) |
| Nachrichten | [`proto.rs`](../lib/game/src/net/proto.rs) |
| Lag-Ausgleich, Hitscan | [`lagcomp.rs`](../lib/game/src/net/lagcomp.rs) |
| Räume, Codes, Links, Namen | [`room.rs`](../lib/game/src/net/room.rs) |
| Simulierte Leitung | [`sim.rs`](../lib/game/src/net/sim.rs) |
| Echte Sockets | [`link.rs`](../lib/game/src/net/link.rs) |
| Bezugsspiel: Bewegung, Arena, Bots | [`walker.rs`](../lib/game/src/net/walker.rs), [`arena.rs`](../lib/game/src/net/arena.rs), [`bot.rs`](../lib/game/src/net/bot.rs) |
| Tests, Messungen, Prüfsumme | [`apps/gametest`](../apps/gametest/README.md): [`net_codec.rs`](../apps/gametest/src/net_codec.rs), [`net_lag.rs`](../apps/gametest/src/net_lag.rs), [`net_room.rs`](../apps/gametest/src/net_room.rs), [`net_sim.rs`](../apps/gametest/src/net_sim.rs), [`net_session.rs`](../apps/gametest/src/net_session.rs), [`net_loop.rs`](../apps/gametest/src/net_loop.rs) |

## Ein Beispiel

```rust
use jflib::game::net::{arena, ClientOptions, NetClient, NetServer, ServerOptions};
use jflib::game::net::arena::{Arena, ArenaConfig};
use jflib::game::net::link::{serve, NetLink};
use jflib::game::net::walker::Walker;
use jflib::netchan::{ClientConfig, Endpoint, Server, ServerConfig};

// der Server: UDP und WebSocket an einem netchan::Server, das Spiel ist ein RoomSim (hier die Arena)
let mut chan = Server::new(ServerConfig { protocol: SPIEL1, ..Default::default() });
let udp = chan.bind_udp(&adresse)?;
let ws = chan.listen_ws(&adresse, "/spiel")?;
let opts = ServerOptions { ws_port: ws.port, ..Default::default() };
let mut spiel = NetServer::new(opts, plat::random_u64(), Box::new(|_code| Arena::new(ArenaConfig::default())));
loop {
    serve(&mut chan, &mut spiel, plat::time_us() * 1000);    // Ereignisse, fällige Ticks, Nachrichten
    chan.wait(2);                                            // nie plat::sleep_ms(1) unter Windows
}

// der Client: dieselbe Karte, dasselbe Schema, dieselbe Grundlinie wie der Server
let karte = arena::map(&cfg);
let nc = NetClient::new(ClientOptions { name: "Anna".into(), room: Some(code), ..Default::default() },
                        Walker::default(), arena::schema(), arena::baseline(&cfg));
let mut link = NetLink::connect(Endpoint::Udp(udp), ClientConfig { protocol: SPIEL1, ..Default::default() }, nc, "/spiel")?;
// je Bild:
let jetzt = plat::time_us() * 1000;
link.pump(&karte, jetzt);
for _ in 0..link.game.advance(jetzt) {
    let eingabe = eingabe_aus(&takt);                       // NetInput: Knöpfe, Achsen, Blick
    link.game.step(&karte, eingabe);                        // vorhersagen, senden
}
link.flush();
let ich = link.game.predicted();                            // die eigene Figur (mit alpha() zwischen zwei Ticks)
link.game.remotes(&mut andere);                             // die anderen, interpoliert
```

## API-Überblick

- **Server** (`net::server`): `RoomSim` (`schema`, `baseline`, `join(name) -> Option<u16>`, `leave`, `step(tick, dt, &[(id,
  NetInput, eigen)])`, `snapshot(tick, &mut Snapshot)`, `priority(betrachter, &Ent) -> u32`), `NetServer::new(ServerOptions,
  seed, RoomFactory)`, `on_event(&ServerEvent)`, `advance(now_ns)`, `step`, `take_out`, `take_close`, `create_room`, `rooms`,
  `room`, `room_mut`, `players`, `player_of`, `stats` (`ServerStats`), `link_stats` (`LinkStats`), `max_rewind_ticks`, `tick`.
  `ServerOptions { rate, snapshot_every, max_players, max_rooms, budget, stream_budget, margin, max_rewind_ms, ws_port,
  hello_ms, ticket_ms, empty_room_ms, start_tick, history }`.
- **Client** (`net::client`): `NetClient::new(ClientOptions, Movement, Schema, Grundlinie)`, `on_event`, `fetch_request`,
  `on_fetch_event`, `take_out`, `take_fetch_out`, `advance(now_ns)`, `step(map, NetInput) -> tick`, `predicted`,
  `predicted_prev`, `alpha`, `remotes(&mut Vec<Remote>)`, `render_time`, `newest`, `phase` (`Phase`), `welcome`, `own_id`,
  `stats` (`ClientStats`), `predictor`, `clock`, `interp`, `last_outcome`; `lerp_field`, `lerp_angle_field`.
  `ClientOptions { name, room, interp_ms, margin, ring, min_redundancy }`.
- **Vorhersage** (`net::predict`): `Movement` (`State`, `FIELDS`, `step(map, &State, &NetInput, dt)`, `to_fields`,
  `from_fields`, `position`), `Predictor` (`step`, `reconcile -> Outcome::{Stale, Match, Corrected { error, shift }, Reset}`,
  `current`, `previous`, `at`, `confirmed`, `pending`, `stats` (`PredictStats`), `clear`).
- **Uhren** (`net::clock`): `SteeredClock` (`set_tick`, `advance`, `next_tick`, `tick`, `alpha`, `on_margin`, `speed_ppm`,
  `margin`, `jumps`), `InterpClock` (`on_snapshot`, `advance`, `time`, `behind`).
- **Schnappschüsse** (`net::snapshot`): `NetState { kind, fields }`, `Ent { id, generation, state }`, `Snapshot { tick, ents }`
  (`get`, `put`, `remove`, `sort`), `Schema` (`with`, `fields`), `Change`, `Op`, `diff`, `select`, `encode`, `decode`,
  `apply`, `encoded_bits`, `full_bits`, `tick_diff`; `MAX_FIELDS` (16), `MAX_ENTITIES` (8192).
- **Eingaben** (`net::input`): `NetInput { buttons, move_x, move_z, yaw, pitch, view }`, `write_inputs`, `read_inputs`,
  `InputBuffer` (`put_packet`, `put`, `take`, `have`, `take_margin`, `next`, `stats`), `MAX_REDUNDANT` (16), `BUFFER` (64).
- **Lag-Ausgleich** (`net::lagcomp`): `view_time`, `HitHistory` (`record`, `rewind`, `span`), `hitscan -> Hit::{Figure,
  World}`.
- **Räume** (`net::room`): `RoomCode` (`from_random`, `parse`, `as_str`, `valid`), `room_link`, `room_from_link`,
  `clean_name`, `unique_name`, `CODE_ALPHABET`, `MAX_NAME`.
- **Nachrichten** (`net::proto`), **Raster** (`net::quant`: `pos_q`, `pos_f`, `angle_q`, `angle_f`, `pitch_q`, `axis_q`,
  `rot_q`, `rot_f`, `round_i64`), **Bitstrom** (`net::bits`: `BitWriter`, `BitReader`, `uvar`, `svar`).
- **Simulierte Leitung** (`net::sim`): `Weather` (`CLEAR`, `TYPICAL`, `ROUGH`, `STORM`), `SimNet` (`add_udp`, `add_ws`,
  `open_fetch`, `step`, `server_events`, `link_events`, `server_send`, `link_send`, `link_close`, `set_weather`), `PeerStats`.
- **Echte Sockets** (`net::link`): `serve(&mut netchan::Server, &mut NetServer, now_ns)` (Windows, Linux, Apple), `NetLink`
  (`connect`, `pump`, `flush`, `channel`, `fetching`).
- **Bezugsspiel**: `walker::Walker` (`WalkState`, `BTN_JUMP`, `BTN_RUN`, `BTN_FIRE`), `arena::{Arena, ArenaConfig, schema,
  map, baseline, spawn_point}`, `bot::{Bot, BotConfig, remote_feet}`.

## Entscheidungen

### Wer rechnet wann: Ticks mit fester Rate, der Client vorn

Der Server rechnet Tick für Tick mit fester Rate (60 Hz) und gibt in Tick T jeder Figur die Eingabe ihres Clients für T. Ein
Client rechnet seine eigenen Ticks voraus: so weit vor dem Server, dass seine Eingabe für T knapp vor dem Tick T beim Server
ankommt. Der Gegenweg wäre der von Source: Der Server führt jede Eingabe aus, sobald sie kommt, ohne gemeinsame Tick-Nummer.
Das braucht keine Uhr-Abstimmung, aber dann rechnet jede Figur zu ihrer eigenen Zeit, Begegnungen zweier Figuren hängen an der
Reihenfolge des Eintreffens, und Lag-Ausgleich und Vorhersage haben keine gemeinsame Zeitachse. Mit gemeinsamen Ticks rechnet
der Server alle Figuren in derselben Reihenfolge, und ein Tick des Clients ist ein Tick des Servers.

**Die Uhr des Clients** (`SteeredClock`): Der Server misst bei jedem Eingabe-Paket, wie viele Ticks es zu früh kommt (neueste
Eingabe minus nächster Tick des Servers), und schickt mit jedem Schnappschuss das Kleinste seit dem letzten (`margin`). Der
Client glättet es und läuft bis zu 10 % schneller oder langsamer, damit es bei 2 Ticks bleibt (Vorgabe des Servers in WELCOME).
Fehlen mehr als 6 Ticks, springt er vor (die übersprungenen Ticks bekommen die laufende Eingabe); ist er mehr als 30 zu früh,
steht er still. Den Anfang schätzt er aus der Zeit von HELLO bis WELCOME: Tick des Servers + Umlauf + Marge + 1. Alles in
Ganzzahlen (Geschwindigkeit in ppm, der Akkumulator in ns × Rate × ppm), damit die Uhr nichts zum Unterschied der Ziele
beiträgt.

**Fehlt eine Eingabe** zum Tick (verloren, zu spät), nimmt der Server die letzte noch einmal (`InputBuffer::take`) und meldet
es nicht: Die Vorhersage des Clients weicht dann ab und wird abgeglichen. Eine Eingabe, die nach ihrem Tick kommt, verwirft der
Server; eine, die mehr als 64 Ticks voraus ist, auch.

### Eingaben mehrfach

Jedes INPUT-Paket trägt die neueste Eingabe ganz und die älteren, die der Server noch nicht hat, jede gegen die nächstjüngere
(ein Bit, wenn sie gleich ist; sonst eine Maske der Felder und ihre Differenzen): höchstens 16, mindestens die letzten drei. Der
Server sagt in jedem Schnappschuss, bis zu welchem Tick er die Eingaben lückenlos hat (`have`). Ein verlorenes Datagramm kostet
so nichts: Die nächste Eingabe bringt die verlorene mit. 16 Eingaben, die sich nur im Blick unterscheiden, sind 34 Bytes.
`NetInput` ist für Spiele wie Spiel 1 geschnitten: 16 Knöpfe (gehalten; Kanten aus dem Vergleich mit dem Tick davor), zwei
Achsen (−127 bis 127), Gier (65 536 Schritte je Umdrehung), Nicken (dieselbe Schrittweite, ±90°) und die Sichtzeit für den
Lag-Ausgleich.

### Bitgleich: Zustände auf einem Raster

Die Vorhersage ist genau, wenn der Client dieselben Bits rechnet wie der Server. Dafür zwei Regeln:

- **Dieselben Operationen.** Die Bewegung (`Movement::step`) rechnet in `f64` nur mit `+ − × ÷`, der korrekt gerundeten
  Wurzel und `sin`, `cos`, `atan2` von `lib/physics::math`; ihre Abfragen sind die von `lib/physics` (auf allen Zielen
  bitgleich, [PHYSICS.md](PHYSICS.md)). Keine Hash-Tabellen, keine Uhr, keine Threads.
- **Der Zustand ist, was gesendet wird.** Der Bewegungszustand liegt nach jedem Tick als ganze Zahlen auf einem Raster von
  1/1024 m (Positionen) und 1/1024 m/s (Geschwindigkeiten); `WalkState` hält ihn so. Der Server rechnet mit genau diesem
  Zustand weiter, also enthält ein Schnappschuss nicht eine Näherung seines Zustands, sondern ihn selbst, und der Abgleich
  vergleicht Bit für Bit. Was der Schritt aus der Vergangenheit braucht (die Knöpfe des Ticks davor, damit ein Sprung ein Druck
  ist), gehört in den Zustand; so rechnet ein Client nach einem Abgleich vom Zustand des Servers aus genau weiter, auch wenn der
  Server eine Eingabe wiederholt hatte.

Das Raster von 1/1024 m (eine Zweierpotenz, also in `f64` exakt) reicht in `i32` bis ±2 097 km.

### Vorhersage und Abgleich

`Predictor` merkt sich je Tick die Eingabe und den vorhergesagten Zustand seit dem letzten bestätigten. Kommt ein neuerer
Schnappschuss, vergleicht er den Zustand der eigenen Figur darin mit seinem für denselben Tick: gleich, dann wirft er nur
Geschichte weg; verschieden, dann setzt er den Zustand des Servers und wendet die Eingaben seither noch einmal an (`Corrected {
error, shift }`: die Abweichung beim Tick des Servers und wie weit der jetzige Zustand springt). Ein älterer Schnappschuss
(umgeordnet) ändert nichts. Vor dem ersten Schnappschuss kennt der Client seine Figur nicht; er schickt trotzdem Eingaben, und
der erste Abgleich rechnet sie vom Zustand des Servers aus nach (`Reset`). Ein Teleport, Schaden oder ein Wiedereinstieg nach
dem Tod sind für den Client Abweichungen wie jede andere. Sichtbare Sprünge glättet Teil 1 nicht; das Bild kann zwischen
`predicted_prev` und `predicted` mit `alpha` blenden (wie `Pose` in [GAMECORE.md](GAMECORE.md)).

### Schnappschüsse: Delta, Raster, Budget

- **Was gesendet wird**, entscheidet das Spiel: Jede Entität ist eine Netz-Id (u16), eine Generation (u8: eine neue Entität
  unter einer alten Id wird nie mit der alten verblendet), eine Art und bis zu 16 ganze Zahlen (`NetState`), schon
  quantisiert. `Schema` sagt, wie viele Felder eine Art hat. Die Arena hat Figuren (13 Felder: die 7 der Bewegung, Gier,
  Nicken, Leben, Abschüsse, Tode, der Tick des letzten Schusses) und Requisiten (7: Position, Drehung als kleinste drei
  Komponenten mit je 11 Bits, 0,06°).
- **Delta**: Gegen eine Basis (unten) gehen nur Änderungen: die Zahl der Änderungen, je Änderung die Lücke zur vorigen Id
  (meist 5 Bits), die Art der Änderung (2 Bits: Änderung, neu, weg), bei einer Änderung eine Maske der Felder und je
  geändertem Feld die Differenz in einem Code mit 7, 12, 19 oder 35 Bits (eine Figur, die 4,5 m/s läuft, verschiebt sich
  zwischen zwei Schnappschüssen um rund 150 Schritte: 12 Bits). Eine neue Entität geht gegen ihren Eintrag in der Grundlinie
  der Karte, wenn sie dort mit derselben Art steht, sonst gegen null.
- **Die Basis**: über WebSocket der zuletzt gesendete Schnappschuss (alles kommt in Reihenfolge an, wie
  [TRANSPORT.md](TRANSPORT.md#für-g7e) vorschlägt); über UDP der neueste, den der Client als dekodiert meldet (jedes INPUT
  nennt ihn), oder, wenn das weniger Bytes ergibt, der neueste vollständige, den er meldet (der geholte volle Stand, s. u.);
  vor jeder Meldung die **Grundlinie der Karte** (`RoomSim::baseline`: die Requisiten, wie die Karte sie hinstellt; der
  Client baut dieselbe aus der Karte). Netchans `Delivered` reicht nicht als Bestätigung: Es sagt, dass ein Paket ankam,
  nicht, dass der Client es dekodieren konnte, und es gilt nicht für den Stand, der über den zweiten WebSocket kommt. Die
  Meldung im INPUT kommt mit demselben Paket wie netchans Acks, also nicht später.
- **Was der Client hat**, merkt sich der Server je Client genau (die letzten 32 gesendeten Stände, je mit „vollständig“ und
  „bestätigt“); der Client hält die letzten 64 dekodierten. Ein Delta gegen etwas, das der Client nicht mehr hat, zählt er
  (`no_base`); in allen Tests kam es nicht vor.
- **Das Budget**: netchan stückelt nicht (Entscheidung vom 18.11.2026), also muss ein Schnappschuss in 1200 Bytes passen
  (`ServerOptions::budget`, `client.max_payload()`). Die Änderungen werden geordnet: die eigene Figur, Entfernungen, dann nach
  einer Priorität, die je Schnappschuss aufaddiert wird, bis die Entität gesendet ist (`RoomSim::priority`: in der Arena
  Figuren 64, Requisiten näher als 20 m 16, bis 50 m 4, sonst 1). `select` nimmt gierig, was passt, misst danach genau und
  lässt die unwichtigsten weg, solange es zu groß ist. Was weggelassen wurde, steht nicht im gesendeten Stand; das nächste
  Delta trägt es. So passt jeder Schnappschuss, und was sich gerade nicht ändert, kostet nichts.

### Beitreten: der volle Stand

HELLO (Version, Raumcode oder keiner, Name) → WELCOME (Code des Raums, Id der Figur, Rate, Abstand der Schnappschüsse, Tick des
Servers, Marge, Grenze des Lag-Ausgleichs). Der volle Stand kommt dann mit dem ersten Schnappschuss, gegen die Grundlinie der
Karte. Passt er nicht in ein Datagramm, gilt die Entscheidung des Nutzers vom 18.11.2026: **Er kommt über einen WebSocket.**

- Ein Browser-Client hat ohnehin nur den WebSocket: Sein erster Schnappschuss bekommt das große Budget (`stream_budget`, 60
  KiB), danach gilt das normale.
- Ein UDP-Client bekommt in WELCOME ein Ticket und den Port des WebSockets des Servers (`fetch`). Er öffnet eine zweite
  netchan-Verbindung (`Endpoint::WebSocket`) und schickt FETCH mit dem Ticket; der Server antwortet beim nächsten
  Schnappschuss-Tick mit FULL (ein Schnappschuss gegen die Grundlinie ohne Budget, beim Server als vollständiger Stand dieses
  Clients vermerkt) und schließt. Der Client meldet ihn in seinen INPUTs (`full`), und der Server nimmt ihn als Basis. Ein
  Ticket gilt 10 s und einmal.
- **Bis dahin** schneidet das Budget die UDP-Schnappschüsse nach Priorität: Der Client sieht seine Figur und das Wichtigste
  sofort. Ohne WebSocket (`ws_port` 0) füllt dieser Weg den Stand allein auf; gemessen unten.

Ob der volle Stand passt, entscheidet der Server beim Beitritt aus seiner Größe (`Welcome::full_bytes`). Gemessen sind die
Größen [unten](#messwerte).

### Lag-Ausgleich

Jede Eingabe trägt, wie weit das Bild der anderen beim Client hinter ihrem Tick lag (`NetInput::view`, in 1/256 Ticks: die
Render-Zeit der Interpolation). Der Server merkt sich die Mittelpunkte der Trefferzonen aller Figuren je Tick (`HitHistory`, die
letzten `max_rewind + 8` Ticks). Schießt eine Figur in Tick T, stellt er die anderen auf T − view zurück (zwischen den zwei Ticks
darum geblendet wie ein Client zwei Schnappschüsse blendet), höchstens `max_rewind_ms` (200 ms, 12 Ticks); was weiter zurück
läge, wird auf die Grenze gezogen und gezählt (`ArenaStats::clamped`). Dann ein Strahl: gegen die Karte mit `World::ray_cast`,
gegen jede zurückgestellte Kapsel mit `physics::query::ray`; der nächste Treffer zählt, der Schütze selbst nicht. Die Zeit hinter
dem Tick enthält Marge, Umlauf und die 100 ms der Interpolation: Bei 200 ms Grenze wird schon ab rund 60 ms Umlauf gezogen (siehe
[Messwerte](#messwerte) und die Frage im Bericht).

### Die anderen: Interpolation

`InterpClock` hält eine Render-Zeit in Ticks des Servers (Tick und Bruchteil, damit sie den Umlauf von u32 übersteht),
`interp_ms` (100 ms, 6 Ticks) hinter dem neuesten Schnappschuss. Bei jedem neueren Schnappschuss vergleicht sie, wo sie steht,
und läuft bis zu 10 % schneller oder langsamer; weit daneben springt sie, ohne Nachschub hält sie am neuesten. `remotes` blendet
jede Entität zwischen den zwei Schnappschüssen um die Render-Zeit (gleiche Id, Generation und Art), sonst hält sie den älteren.
Diese Zeit ist getrennt vom Alpha des Takts in `Pose`: Das blendet die letzten zwei vorhergesagten Ticks der eigenen Figur.

### Ticks und Nummern laufen um

Ticks sind `u32` (bei 60 Hz nach 2,3 Jahren um), jeder Vergleich geht über `tick_diff` (die Differenz als `i32`); auf der
Leitung stehen sie ganz, Basen als Abstand (1 bis 255 Ticks). Die Folgenummern sind netchans (u16). Der Test beginnt 240 Ticks
vor dem Umlauf und läuft 70 000 Schnappschüsse über den Umlauf der Folgenummern.

### Räume, Namen, kein Konto

Ein Raum hat einen Code aus sechs Zeichen ohne Verwechsler (kein 0, O, 1, I, L: 31 Zeichen, rund 887 Millionen Codes), den der
Server aus seinem Zufall zieht. Ein Link ist die Adresse der Seite mit `#CODE` (das Fragment erreicht nie ein Server-Log);
`room_from_link` nimmt auch `?room=CODE`, `/r/CODE` und den nackten Code, in jeder Schreibweise, mit Leerzeichen und
Bindestrichen. HELLO ohne Code macht einen Raum. Namen werden gesäubert (Ränder weg, Leerraum als ein Leerzeichen, keine
Steuer-, Richtungs- und unsichtbaren Zeichen, 1 bis 16 Zeichen) und im Raum eindeutig gemacht („Anna 2“). Grenzen: 16 Spieler je
Raum, 64 Räume je Server, ein leerer Raum schließt nach 30 s, eine Verbindung ohne HELLO nach 5 s.

### Die simulierte Leitung

`net::sim::SimNet` setzt netchans Kerne (`endpoint::ServerCore`, `ClientCore`) auf eine virtuelle Uhr: je Client ein Wetter
für Verzögerung, Schwankung, Verlust, Doppelte und Umordnung, aus einem Seed. UDP-Datagramme treffen das Wetter; ein WebSocket
ist zuverlässig und geordnet (ein Verlust ist eine Wiederholung: zwei Verzögerungen und 20 ms später, nichts überholt ihn), er
öffnet nach zwei Umläufen. Nichts liest eine Uhr oder einen Socket, also ergibt derselbe Seed auf jedem Ziel dieselben Pakete;
darauf beruht die Prüfsumme einer ganzen Sitzung. Vorlage war die Simulation in `apps/nettest` (G7d).

| Wetter | Verzögerung je Richtung | Verlust | Doppelt | später |
| --- | --- | ---: | ---: | --- |
| `CLEAR` | 20 ms | 0 | 0 | – |
| `TYPICAL` | 30 ms ± 5 | 1 % | 0 | 2 % bis 10 ms |
| `ROUGH` (wie nettest) | 20 ms ± 5 | 20 % | 10 % | 20 % bis 30 ms |
| `STORM` (wie nettest) | 40 ms ± 20 | 50 % | 20 % | 30 % bis 60 ms |

### Das Bezugsspiel

`walker::Walker` ist die Bewegung der Tests: eine Kapsel (Halbhöhe 0,6 m, Radius 0,3 m), gehen 4,5 m/s, rennen 8, springen mit
5,5 m/s, Schwerkraft 18 m/s², etwas Steuerung in der Luft; an Hindernissen entlanggleiten über bis zu vier Casts durch
`lib/physics`, „am Boden“ über einen kurzen Cast nach unten (steiler als rund 45° ist Wand). Keine Stufen, keine Rampen-Haftung:
das bringt der Character-Controller von G7c Teil 2, den G7h hinter `Movement` setzt. `arena::Arena` ist ein `RoomSim`: ein
Quadrat mit Mauern und Kisten (eine statische `physics::World`, auf Server und Clients gleich), Figuren an einem Ring von
Startplätzen, Requisiten auf einem Raster, von denen ein Teil auf Kreisen fährt (sie stehen nicht in der Physik-Welt und halten
niemanden auf, damit die Karte der Vorhersage die des Servers bleibt), Hitscan mit 25 Schaden, 10 Schüsse je Sekunde, Tod und
Neustart. `bot::Bot` läuft zu zufälligen Zielen, dreht höchstens 4 rad/s, rennt und springt manchmal und zielt mit `fire` auf den
nächsten anderen in 25 m.

## Tests

TESTS_PLACEHOLDER

## Prüfsumme

CHECKSUM_PLACEHOLDER

## Messwerte

MEASURE_PLACEHOLDER

## Was nicht getestet ist

UNTESTED_PLACEHOLDER

## Was nicht gebaut ist

- Verschlüsselung über UDP (später, Entscheidung vom 18.11.2026); das Ticket für FETCH ist nur so geheim wie der Verkehr.
- Glätten sichtbarer Korrekturen der eigenen Figur (ein abklingender Versatz im Bild), Extrapolation der anderen, wenn
  Schnappschüsse fehlen (sie halten).
- Geschosse mit Flugzeit, Treffer auf Teile der Figur (Kopf), Figuren, die einander stoßen oder blockieren (die Vorhersage
  kennt die anderen nicht), Requisiten, die die Figuren aufhalten.
- Deltas gegen eine Vorhersage (Position + Geschwindigkeit × Zeit) statt gegen den Stand: spart bei laufenden Figuren
  vielleicht ein Drittel.
- Ein Raum je Kern: Alle Räume eines Servers rechnen auf einem Thread nacheinander.
- Zuschauer, Wiedereinstieg nach einem Verbindungsabbruch mit derselben Figur, Teams, Runden: das ist Spiel 1 (G7h).
- Eingaben, die das Spiel selbst definiert (`NetInput` ist fest); Ereignisse mit Zuverlässigkeit über UDP (Treffermarken und
  Abschüsse stehen in den Feldern der Figuren).

## Für Teil 2 und G7h

FOR_G7H_PLACEHOLDER

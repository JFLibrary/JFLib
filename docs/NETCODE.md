# NETCODE -- Netz für Spiele mit Vorhersage

Der Netcode der Spiel-Engine ([GAME.md](GAME.md), Stufe G7e) liegt in `lib/game` als Modul `game::net`: Der Server entscheidet
und simuliert mit fester Rate, Clients schicken ihre Eingaben mit Tick-Nummer (mehrfach, damit ein verlorenes Datagramm nichts
kostet), der Server schickt Schnappschüsse als Delta gegen den Stand, den der Client hat, quantisiert und unter 1200 Bytes. Der
Client sagt seine eigene Figur voraus und gleicht sie mit dem Server ab; die anderen Figuren zeigt er rund 100 ms hinter dem
neuesten Schnappschuss. Treffer prüft der Server mit Lag-Ausgleich: Er stellt die Trefferzonen der anderen auf die Zeit zurück,
die der Schütze sah, höchstens 300 ms (die 100 ms der Interpolation und 200 ms darüber hinaus, vom Nutzer entschieden
am 19.11.2026). Räume gibt es per Code oder Link, Spieler haben einen Namen, kein Konto; Bots spielen
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
so nichts: Die nächste Eingabe bringt die verlorene mit. 16 Eingaben, die sich nur im Blick unterscheiden, sind 41 Bytes.
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
  Komponenten mit je 12 Bits, an 20 000 Drehungen geprüft höchstens 0,1° daneben).
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
darum geblendet wie ein Client zwei Schnappschüsse blendet), höchstens `max_rewind_ms` (300 ms, 18 Ticks; bis zum 19.11.2026 200 ms); was weiter zurück
läge, wird auf die Grenze gezogen und gezählt (`ArenaStats::clamped`). Dann ein Strahl: gegen die Karte mit `World::ray_cast`,
gegen jede zurückgestellte Kapsel mit `physics::query::ray`; der nächste Treffer zählt, der Schütze selbst nicht. Die Zeit hinter
dem Tick enthält Marge, Umlauf und die 100 ms der Interpolation: Bei der ersten Grenze von 200 ms wurde schon ab rund 50 ms
Umlauf gezogen (siehe [Messwerte](#messwerte)); deshalb zählt die Grenze seit dem 19.11.2026 die 100 ms der Interpolation
nicht mit: 300 ms.

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

`python build_rs.py test gametest` führt `apps/gametest/tests/check.uis` (`--check`: auch die fünf Gruppen des Netcodes) und
`tests/loopback.uis` (`--loopback --secs 5`) aus. Die Referenzen teilen keinen Code mit `game::net`: eine einfache volle
Kodierung in ganzen Bytes für die Schnappschüsse, Listen jedes aufgezeichneten Ticks für das Zurückstellen, ein eigener Schnitt
von Strahl und Kapsel und ein Slab-Test für den Hitscan, eine eigene Bewegung aus ganzen Zahlen für den Abgleich; für die
Vorhersage ist die Referenz der Server selbst (sein Zustand jeder Figur in jedem Tick).

| Gruppe | Prüft | Ergebnis |
| --- | --- | --- |
| `netcodec` | Bitstrom: 20 000 zufällige Folgen von Werten mit 0 bis 32 Bits, die Codes an ihren Grenzen und ihre Größen, Lesen hinter dem Ende (Nullen und die Fehlermarke). Raster: 100 000 Positionen bis ±10⁵ m (höchstens ein halber Schritt, das Raster exakt), Winkel, Nicken, Achsen, 20 000 Drehungen (q und −q gleich). 4000 Paare von Schnappschüssen (bis 120 Entitäten, Arten mit 1 bis 16 Feldern, Ids mit großen Lücken, entfernt, geändert, neue Generation, neu, ein Drittel mit Grundlinie): Delta gegen die Basis und volle Kodierung gegen nichts ergeben beide den Stand der Referenzkodierung, die gemessene Größe ist die geschriebene. 3000 Schnitte durch zufällige Budgets mit zufälligen Prioritäten: nie über dem Budget, in Id-Reihenfolge, dekodiert gleich Basis + Gewähltes, jede Entität ist die der Basis oder die neue, „vollständig“ genau dann, wenn alles drin ist. 60 000 kaputte Byte-Folgen (zufällig, gekippte Bits, abgeschnitten, verlängert) an den Decoder und alle Leser der Nachrichten. 20 000 Eingabe-Pakete mit 1 bis 16 Eingaben; alle Nachrichten hin und zurück (ein Name, der mitten in einem Zeichen endet, verliert das Zeichen). Der Eingabepuffer über den Umlauf von u32: Reihenfolge, fehlend, zu spät, zu früh, Lücken, `have`, die Marge, ein weiter Sprung. Der Prädiktor gegen eine Neuberechnung: erster Stand, exakte Vorhersage, Abgleich, älterer Stand. Die Uhren: 60 Ticks je Sekunde, schneller und langsamer, Sprung, Halten; die Interpolation folgt über den Umlauf und hält ohne Nachschub | 67 Prüfungen, 0 falsch; Delta im Mittel 296 Bytes gegen 1248 voll; 10 046 der kaputten Folgen dekodierten noch (gültig), keine Panik; 16 Eingaben, die sich im Blick unterscheiden, 41 Bytes |
| `lagcomp` | `view_time` an 3 Ticks (auch am Umlauf) × 732 Werten; 200 Runden mit 1 bis 16 Figuren über bis zu 65 Ticks (manche fehlen in manchen Ticks, gemischt aufgezeichnet), je 50 Zeiten auch vor und nach dem Gemerkten, Bit für Bit gegen die Liste; 16 000 Strahlen gegen 0 bis 5 Kisten und bis zu 16 Kapseln gegen die eigene Rechnung; die Arena: ein Schuss dorthin, wo der Schütze das Ziel vor 10 Ticks sah, trifft mit dem Zurückstellen (Schaden 25) und ohne nicht, eine Sicht über 200 ms wird gezogen und gezählt | 8 Prüfungen, 0 falsch |
| `room` | 20 000 Codes (gültig, zurückgelesen, jede Schreibweise), verweigerte Codes, Links (Fragment, Abfrage, Pfad, nackt), acht Namen und ihre Säuberung, eindeutige Namen; der Server mit netchan-Ereignissen von Hand: HELLO macht einen Raum, zwei treten bei (Namen „Anna“, „anna 2“), voller Raum, unbekannter Raum, andere Version, leerer Name, der erste Schnappschuss eines WebSockets gegen die Grundlinie, Verlassen, der freie Platz mit Generation 1, eine stumme Verbindung nach 500 ms zu, ein leerer Raum nach 1 s zu, `max_rooms`; 300 bewegte Requisiten: Ticket für UDP, keines für WebSocket, ein falsches Ticket, FULL beim nächsten Schnappschuss-Tick und kein UDP-Schnappschuss in diesem Tick | 47 Prüfungen, 0 falsch; FULL 3146 Bytes |
| `netsim` | Sitzungen mit Bots (ein Viertel über WebSocket, 40 Requisiten, 8 fahren): klar 6 s mit 8, typisch 10 s mit 8, rau 10 s mit 6, Sturm 10 s mit 4, dann klar; nach der Heilung (2 s) 3 s lang jede erste Vorhersage gegen den Zustand des Servers Bit für Bit, kein Abgleich mit Unterschied, jeder Client im Raum, keine fehlende Basis, kein kaputter Schnappschuss, keiner über 1200 Bytes. Der Umlauf der Ticks (Start 240 vor `u32::MAX`, rau). Der volle Stand mit 300 Requisiten (30 fahren, 150 stehen nicht am Platz der Karte): geholt über den WebSocket und ohne WebSocket nur nach Priorität aufgefüllt, danach ist der neueste Stand jedes Clients der Welt des Servers gleich. Verlassen (die anderen sehen die Figur gehen) und neu beitreten (dieselbe Id, Generation 1). 70 000 Schnappschüsse an einen Client über den Umlauf der Folgenummern (5 % Verlust), danach bitgleich | 51 Prüfungen, 0 falsch; siehe unten |
| `bots` | 16 Bots, typisches Wetter, 300 Requisiten, Schießen: alle spielen, keiner getrennt, kein Schnappschuss über 1200 Bytes, aufwärts unter 300 Paketen und 256 KiB je Sekunde | 4 Prüfungen, 0 falsch |
| Skript `loopback.uis` | echte Sockets: siehe [unten](#echte-sockets) | bestanden |

Die Läufe der Gruppe `netsim` (dieselben auf allen Zielen):

| Wetter | Spieler | schlechtes Wetter | Abweichungen der Vorhersage darin | nach der Heilung verglichen | verschieden |
| --- | ---: | ---: | ---: | ---: | ---: |
| klar | 8 | 6 s | 0 von 5119 Ticks | 1440 | 0 |
| typisch | 8 | 10 s | 11 von 6983 | 1440 | 0 |
| rau | 6 | 10 s | 29 von 5242 | 1080 | 0 |
| Sturm | 4 | 10 s | 88 von 3245 | 720 | 0 |
| Umlauf der Ticks (rau) | 3 | 4 s | – | über 100 | 0 |
| Umlauf der Folgenummern (5 % Verlust) | 1 | 1170 s, 70 494 Schnappschüsse | – | 180 | 0 |

Beitreten mit 300 Requisiten (150 nicht am Platz der Karte, 30 fahren), typisches Wetter: Der volle Stand gegen die Grundlinie
sind 1853 Bytes, passt also nicht in ein Datagramm. Vollständig (der neueste Stand des Clients gleich der Welt des Servers) nach
rund 200 ms über UDP mit FULL über den WebSocket, ebenso schnell ohne WebSocket nur nach Priorität aufgefüllt (9 geschnittene
Schnappschüsse), 50 ms für den Browser-Client über WebSocket (gemessen in Schritten von 100 ms).

| Ziel | `--test netcodec,lagcomp,room,netsim,bots` | `--loopback` |
| --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100, Entwicklung | 177 von 177 | bestanden (8 Clients, 8 s) |
| Linux x86-64 (glibc) in WSL Ubuntu 24.04, Entwicklung | 176 von 176 (`--quick`) | bestanden (8 Clients, 6 s) |
| macOS ARM64 (Mac mini M2, macOS 27.2), Rust 1.98.1, Entwicklung | 177 von 177 | bestanden (8 Clients, 6 s) |

### Echte Sockets

`gametest --loopback` (Skript `tests/loopback.uis`): ein `netchan::Server` auf 127.0.0.1 mit UDP und dem WebSocket-Pfad
`/game`, die Arena mit 300 Requisiten (150 nicht am Platz der Karte), vier Bots über UDP (jeder holt seinen vollen Stand über
einen eigenen WebSocket: FULL) und vier über WebSocket, alles in einem Thread (`link::serve`, `NetLink`; `Server::wait(1)`
schläft, bis ein Paket kommt). Auf Windows (8 s), in WSL und auf dem M2 (je 6 s): alle acht spielen, jeder UDP-Client bekam
genau ein FULL (rund 1800 Bytes), keine fehlende Basis, kein kaputter Schnappschuss, 160 bis 240 Schnappschüsse je Client, die
Marge bei 2 Ticks, 2 bis 8 Abweichungen je Client (Tode: die Bots schießen), Umlauf über netchan 15 bis 23 ms (ein Ack reitet auf
dem nächsten Paket der Gegenseite, Schnappschüsse kommen alle 33 ms).

## Prüfsumme

`gametest --netsum` faltet eine aufgezeichnete Sitzung mit festem Seed in eine Zahl (FNV-1a): ein Server mit der Arena (200
Requisiten, 100 nicht am Platz der Karte, 10 fahren; die UDP-Clients holen ihren vollen Stand über einen WebSocket), sechs Bots
(vier über UDP, zwei über WebSocket), die schießen, 10 s raues Wetter, dann 5 s klar, Start 600 Ticks vor dem Umlauf von u32.
Gefaltet werden jede Nachricht des Servers (Verbindung und Bytes), der Zustand jeder Figur des Servers in jedem Tick, jede erste
Vorhersage jedes Clients, die Zahl der Abweichungen und ihre Fehler, die interpolierten anderen jeder Sekunde, Schüsse, Treffer,
Tode. Die Sitzung läuft über netchans Kerne auf der simulierten Leitung: Die Prüfsumme umfasst also auch den Handschlag (HMAC),
die Folgenummern und die Acks.

| Ziel | Build | `--netsum` | `--checksum` (der Kern, G7b) |
| --- | --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | Entwicklung und Release | `4118995e45c0c067` (177 246 Werte) | `8edfe993e4ffcb65` |
| Linux x86-64 (glibc) in WSL Ubuntu 24.04 | Entwicklung | `4118995e45c0c067` | `8edfe993e4ffcb65` |
| macOS ARM64 (Mac mini M2, macOS 27.2), Rust 1.98.1 | Entwicklung und Release | `52c4613d8ffdd103` mit der alten Grenze von 200 ms (wie die anderen damals); mit 300 ms nicht neu gerechnet | `8edfe993e4ffcb65` |
| WebAssembly in Node 22.17 (V8): einfach, SIMD, Threads (`tests/web.mjs`) | Entwicklung | `4118995e45c0c067` (alle drei) | `8edfe993e4ffcb65` (alle drei; für den Kern neu) |

Die Grenze des Lag-Ausgleichs ging am 19.11.2026 von 200 auf 300 ms; damit treffen andere Schüsse, und die Prüfsumme der
Sitzung wechselte von `52c4613d8ffdd103` (177 304 Werte, auf allen vier Zielen gleich) zu `4118995e45c0c067`.

Die Messungen von `--netbench` ergeben auf Windows und dem M2 dieselben Zahlen bis auf die Zeiten (siehe unten).

## Messwerte

`gametest --netbench` im Release-Build: 16 Bots in der Arena (60 × 60 m, 12 Kisten), je 30 s virtueller Zeit nach den
Beitritten. Die Bots laufen, rennen, springen, drehen und schießen auf den nächsten in 25 m (zielen sehr gut: rund 93 % Treffer,
also viele Tode und Neustarts). Requisiten: die Hälfte steht nicht mehr am Platz der Karte, ein Zehntel fährt Kreise.
Schnappschüsse mit 30 Hz, wo nicht anders genannt. Alles außer den Zeiten der Ticks ist deterministisch: Windows und der M2
messen dieselben Zahlen.

**Vorhersage** (je Spieler; „Fehler“: wie weit die Vorhersage für den Tick des Servers daneben lag, über die Abweichungen;
„Sprung“: wie weit der jetzige Zustand dabei springt):

| Lauf | Abweichungen je Minute | Fehler Mittel / p99 / max | Sprung p99 |
| --- | ---: | --- | ---: |
| klar, ohne Schießen (Gruppe `netsim`) | 0 | – | – |
| typisch, ohne Schießen | 10,6 | 0,007 / 0,18 / 0,18 m | 0,59 m |
| klar, Schießen | 29–31 | 6,2 / 11,4 / 12,5 m | 11,6 m |
| typisch, Schießen | 45,8 | 4,0 / 12,1 / 15,2 m | 12,1 m |
| typisch, Schnappschüsse mit 60 Hz | 51,9 | 3,3 / 11,2 / 12,2 m | 11,4 m |
| rau, Schießen | 54,9 | 3,1 / 11,2 / 14,1 m | 11,4 m |
| Sturm, Schießen | 197,1 | 0,6 / 9,9 / 18,2 m | 9,8 m |

Mit Schießen sind die meisten Abweichungen Neustarts nach einem Tod (ein Sprung an einen Startplatz, Meter weit); was das Netz
selbst kostet, zeigt der Lauf ohne Schießen: Bei 1 % Verlust und ±5 ms Schwankung kommt rund alle 6 s eine Eingabe zu spät (die
Marge von 2 Ticks reicht dann nicht), die Korrektur ist im Mittel 7 mm. Nach dem Ende schlechten Wetters ist jede Vorhersage
wieder exakt (Gruppe `netsim`).

**Bandbreite** je Spieler (Nutzlast und Kopf von netchan; „Leitung“: dazu 28 Bytes IPv4 und UDP je Paket):

| Lauf | aufwärts | Pakete/s | Leitung | abwärts | Pakete/s | Leitung |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| klar, ohne Requisiten | 2,99 KB/s | 60 | 4,67 KB/s | 7,41 KB/s | 30 | 8,25 KB/s |
| klar, 300 Requisiten | 3,00 KB/s | 60 | 4,68 KB/s | 12,8 KB/s | 30 | 13,6 KB/s |
| typisch, 300 | 3,30 KB/s | 60 | 4,98 KB/s | 13,2 KB/s | 30 | 14,1 KB/s |
| typisch, 300, 60 Hz | 3,17 KB/s | 60 | 4,85 KB/s | 25,4 KB/s | 60 | 27,1 KB/s |
| rau, 300 | 3,34 KB/s | 60 | 5,02 KB/s | 13,1 KB/s | 30 | 14,0 KB/s |
| Sturm, 300 | 5,13 KB/s | 60 | 6,81 KB/s | 14,0 KB/s | 30 | 14,8 KB/s |
| typisch, 500 Requisiten (50 fahren) | 3,29 KB/s | 60 | 4,97 KB/s | 25,9 KB/s | 30 | 26,8 KB/s |
| typisch, alle 300 fahren | 3,31 KB/s | 60 | 4,99 KB/s | 33,7 KB/s | 30 | 34,5 KB/s |

Aufwärts sind es 60 Pakete je Sekunde (eines je Tick), weit unter netchans 300; im Sturm wachsen die Pakete, weil mehr Eingaben
wiederholt werden. 16 Spieler kosten den Server abwärts rund 210 KB/s (30 Hz, 300 Requisiten).

**Schnappschüsse** (Nutzlast in Bytes; „geschnitten“: das Budget von 1200 Bytes ließ Änderungen weg):

| Lauf | Mittel | p50 | p99 | max | geschnitten |
| --- | ---: | ---: | ---: | ---: | ---: |
| ohne Requisiten | 234 | 234 | 270 | 283 | 0 % |
| 300 Requisiten, 30 fahren | 413–428 | 415–435 | 460–476 | 477–502 | 0 % |
| dasselbe im Sturm | 453 | 464 | 553 | 600 | 0 % |
| 500 Requisiten, 50 fahren | 851 | 868 | 911 | 950 | 0 % |
| 300 Requisiten, alle fahren | 1109 | 1112 | 1120 | 1124 | 100 % |

Die laufenden Stände passen also mit Spiel 1 (16 Spieler, einige hundert Requisiten, von denen wenige zugleich fahren) gut
unter 1200 Bytes. Erst wenn rund hundert Dinge zugleich in Bewegung sind, schneidet das Budget jeden Schnappschuss: Ferne
Requisiten bekommen dann seltener neue Stände (ihre Priorität wächst, bis sie dran sind).

**Der volle Stand beim Beitreten** (16 Spieler, die Hälfte der Requisiten nicht am Platz der Karte, ein Zehntel fährt):

| Requisiten | gegen die Grundlinie der Karte | gegen nichts (Neues gegen die Grundlinie) | ohne Grundlinie |
| ---: | ---: | ---: | ---: |
| 0 | 265 B | 265 B | 265 B |
| 100 | 860 B | 1143 B | 1633 B |
| 200 | 1443 B | 2008 B | 3031 B |
| 300 | 2035 B | 2884 B | 4447 B |
| 500 | 3217 B | 4629 B | 7200 B |

Ein Beitritt passt in ein Datagramm, solange höchstens rund 150 Requisiten von ihrem Platz bewegt wurden; bei Spiel 1 mit
einigen hundert, die im Lauf einer Runde umgestoßen werden, nicht. Deshalb der Weg über den WebSocket (FULL) und bis dahin das
Auffüllen nach Priorität; beide brauchten in der Gruppe `netsim` rund 200 ms bis zum vollständigen Stand (300 Requisiten, 150
bewegt). Die Grundlinie der Karte spart ein Drittel bis die Hälfte.

**Lag-Ausgleich** (die Zeit, die der Schütze hinter dem Tick des Schusses sah, und wie weit das zurückgestellte Ziel von dem
Platz entfernt stand, an dem der Schütze es sah):

| Lauf | Sicht hinter dem Tick: Mittel / max | auf die Grenze gezogen | Abstand Mittel / p99 | ohne Grenze |
| --- | --- | ---: | --- | --- |
| klar, 200 ms | 185 / 195 ms | 0 % | 0,012 / 0,045 m | gleich |
| typisch, 200 ms | 211 / 229 ms | 100 % | 0,079 / 0,29 m | 0,012 / 0,14 m |
| rau, 200 ms | 204 / 277 ms | 38 % | 0,077 / 0,82 m | 0,020 / 0,32 m |
| Sturm, 200 ms | 345 / 665 ms | 100 % | 1,13 / 10,1 m | 0,031 / 0,33 m |
| typisch, 60 Hz, 200 ms | 210 / 226 ms | 100 % | 0,063 / 0,11 m | 0,000 / 0,001 m |
| klar, 300 ms | 185 / 195 ms | 0 % | 0,011 / 0,025 m | gleich |
| typisch, 300 ms | 210 / 226 ms | 0 % | 0,010 / 0,027 m | gleich |
| typisch, 400 ms | 210 / 232 ms | 0 % | 0,009 / 0,029 m | gleich |

Die Sicht liegt um Marge (33 ms), Umlauf und Alter des Schnappschusses und die 100 ms der Interpolation hinter dem Tick: schon bei
20 ms je Richtung 185 ms. Mit der Grenze von 200 ms wird ab rund 25 ms je Richtung (50 ms Umlauf) gezogen, und das Ziel steht dann
im Mittel 8 cm, im p99 fast eine Kapselbreite (30 cm) neben dem, was der Schütze sah. Mit 60-Hz-Schnappschüssen ist der Abstand
ohne Grenze null (der Server blendet Ticks, der Client Schnappschüsse; mit einem Schnappschuss je Tick ist das dasselbe); mit 30 Hz
bleiben im Mittel 1 cm (eine Figur, die innerhalb von zwei Ticks dreht, an einer Wand hält oder neu startet). Die Frage an den
Nutzer: Er entschied am 19.11.2026 für 300 ms (die Zeilen mit 300 ms oben).

**Der Server** (16 Bots, ein Raum, alles in einem Thread; die Zeit eines Ticks: die Eingaben, die Bewegung aller Figuren, die
Requisiten, die Treffer, und jeden zweiten Tick 16 Schnappschüsse):

| Lauf | M2, macOS: Mittel / p50 / p99 / max | Ryzen 5 3600, Windows: p50 / p99 |
| --- | --- | --- |
| ohne Requisiten | 40 / 57 / 68 / 72 µs | 121 / 581 µs |
| 300 Requisiten (30 fahren) | 110 / 186 / 214 / 268 µs | 449 / 5511 µs |
| 300, Schnappschüsse mit 60 Hz | 204 / 204 / 215 / 223 µs | 534 µs / – |
| 500 Requisiten (50 fahren) | 358 / 581 / 743 / 779 µs | 1313 µs / – |
| 300, alle fahren (jeder Schnappschuss geschnitten) | 1371 / 2614 / 2711 / 2733 µs | 3895 µs / – |

Der p50 liegt bei den Ticks mit Schnappschüssen (jeder zweite), das Mittel darunter. Ein Raum mit 16 Spielern und 300 Requisiten
braucht also auf dem M2 rund 0,7 % eines Kerns (110 µs je Tick bei 60 Hz), auf dem Ryzen rund 3 %; „ein Raum auf einem Kern“
(G7h) hat viel Luft. Der Windows-Rechner wurde von anderen Sitzungen mitbenutzt: seine p99 und Maxima (bis über 100 ms) sind
Planungslücken, nicht der Netcode. Wo die Zeit geht: das Delta je Client (zwei Basen verglichen, jede Entität Feld für Feld) und
die Kopie des gesendeten Stands je Client; geschnittene Schnappschüsse kosten das Sortieren nach Priorität.

## Was nicht getestet ist

- **Ein echtes Netz zwischen Rechnern**: alles lief auf der simulierten Leitung oder auf 127.0.0.1. Wie sich echte Schwankung
  (WLAN, Mobilfunk) auf die Marge auswirkt, misst Teil 2.
- **Der Browser als Client**: Der WebSocket-Weg des Clients ist derselbe Code wie auf dem Desktop (`netchan::Client` mit
  `Endpoint::WebSocket`), gelaufen ist er im Browser nicht; die Prüfsumme lief in Node (in drei Varianten, auch die Gruppen netcodec bis bots mit `--quick`: 176 von 176).
- **Android, iOS, statisches Linux**: `lib/game` mit `game::net` baut für `aarch64-linux-android`, `aarch64-apple-ios` (auf dem
  Mac geprüft, auch netchan) und `wasm32-unknown-unknown`, mit und ohne `std`; gelaufen ist dort nichts. Ein statischer
  Linux-Server mit dem Netcode ist noch nicht gebaut (gametest braucht glibc für die Brücke).
- **Mehr als 16 Spieler, mehrere Räume unter Last, viele Server-Threads**: gemessen ist ein Raum mit 16 Bots; Räume laufen
  nacheinander auf einem Thread.
- **Böswillige Clients über die Fuzz-Tests hinaus**: ein Client, der Eingaben weit in der Zukunft schickt, wird begrenzt (64
  Ticks), einer, der falsche Acks meldet, schadet nur sich (der Server nimmt nur Stände, die er gesendet hat); Geschwindigkeit
  und Ziele prüft der Server, weil er rechnet. Schummeln mit Zielhilfen ist nicht Thema (GAME.md, „Bewusst nicht“).
- **Das Gefühl**: Ob 100 ms Interpolation, eine Marge von 2 Ticks und Korrekturen ohne Glätten sich gut spielen, zeigt erst
  Spiel 1 mit Menschen.

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

- **Den Controller einsetzen**: `walker::Walker` durch den Character-Controller von G7c Teil 2 ersetzen, hinter
  `predict::Movement`: seinen Zustand als ganze Zahlen auf dem Raster halten (alles, was der nächste Schritt braucht, auch
  Knöpfe des Ticks davor, Bodenkontakt, Ducken), `FIELDS` und `to_fields`/`from_fields` verlustfrei; die Karte des Spiels als
  `physics::World` auf Server und Client gleich. Die Prüfung, dass nach der Heilung jede erste Vorhersage exakt ist
  (`net_sim.rs`, `weather`), mit dem Controller wiederholen; ebenso `--netsum` auf allen Zielen.
- **Ein RoomSim für Spiel 1**: die Arena zeigt, wie: die Figuren und ihre Zustände, Requisiten mit festen Netz-Ids aus der Karte
  (die Grundlinie), `priority` nach Entfernung und Sichtbarkeit, Treffer mit `HitHistory` und `hitscan`. Teams, Runden,
  Punktetafel, Waffen, Geschosse mit Flugzeit sind des Spiels. Ein Spiel mit `World` und `Pose` baut den Schnappschuss aus den
  Spalten (eine Hilfe dafür gibt es noch nicht).
- **Die Grenze des Lag-Ausgleichs**: 300 ms (entschieden am 19.11.2026); ein RoomSim für Spiel 1 nimmt dieselbe Zahl wie
  `ServerOptions::max_rewind_ms` (in Ticks, wie `ArenaConfig::max_rewind`).
- **Korrekturen glätten**: ein abklingender Versatz im Bild der eigenen Figur nach `Outcome::Corrected` (die Physik bleibt beim
  korrigierten Zustand).
- **Treffermarken und Abschüsse** zuverlässig melden: heute stehen sie als Felder in den Figuren (das letzte Schuss-Tick, Leben,
  Abschüsse); ein Ereigniskanal mit Wiederholung bis zur Bestätigung wäre der nächste Schritt.
- **Der Browser-Client**: `NetLink` mit `Endpoint::WebSocket("/pfad")` im Web-Build; der Server mit `ws_endpoint` neben den
  Dateien der Seite (wie `nettest --serve`).
- **Teil 2 des Netcodes**: echtes Netz zwischen Rechnern (Windows, Fedora, ThinkPad, Mac, WLAN), die Marge und die
  Interpolationszeit danach einstellen; die Verschlüsselung über UDP (netchan, entschieden); Deltas gegen eine Vorhersage;
  mehrere Räume auf mehreren Threads.

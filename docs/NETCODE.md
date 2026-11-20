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
Seit Teil 2: zuverlässige Ereignisse in beide Richtungen (einmal, in Reihenfolge, über UDP und WebSocket), ein sauberes
Verlassen (LEAVE), geglättete Korrekturen der eigenen Figur, Räume nebeneinander auf Threads, Raumcodes und Tickets aus
dem kryptographischen Zufall des Systems; gemessen auf einem echten Netz zwischen zwei Rechnern und im Browser.

Stand: G7e Teil 2 (dieser Lauf). Was fehlt, steht unter [Was nicht gebaut ist](#was-nicht-gebaut-ist) und
[Offen](#offen).

| Teil | Wo |
| --- | --- |
| Server: Räume, Takt, Eingaben, Schnappschüsse je Client, Räume auf Threads | [`server.rs`](../lib/game/src/net/server.rs) |
| Client: Beitritt, Uhr, Vorhersage, Glätten, Schnappschüsse, Interpolation, Verlassen | [`client.rs`](../lib/game/src/net/client.rs), [`clock.rs`](../lib/game/src/net/clock.rs), [`predict.rs`](../lib/game/src/net/predict.rs) |
| Zuverlässige Ereignisse | [`event.rs`](../lib/game/src/net/event.rs) |
| Schnappschüsse und ihr Delta | [`snapshot.rs`](../lib/game/src/net/snapshot.rs), Bitstrom [`bits.rs`](../lib/game/src/net/bits.rs), Raster [`quant.rs`](../lib/game/src/net/quant.rs) |
| Eingaben, ihre Wiederholung, der Puffer des Servers | [`input.rs`](../lib/game/src/net/input.rs) |
| Nachrichten | [`proto.rs`](../lib/game/src/net/proto.rs) |
| Lag-Ausgleich, Hitscan | [`lagcomp.rs`](../lib/game/src/net/lagcomp.rs) |
| Räume, Codes, Links, Namen | [`room.rs`](../lib/game/src/net/room.rs) |
| Simulierte Leitung | [`sim.rs`](../lib/game/src/net/sim.rs) |
| Echte Sockets | [`link.rs`](../lib/game/src/net/link.rs) |
| Bezugsspiel: Bewegung, Arena, Bots | [`walker.rs`](../lib/game/src/net/walker.rs), [`arena.rs`](../lib/game/src/net/arena.rs), [`bot.rs`](../lib/game/src/net/bot.rs) |
| Tests, Messungen, Prüfsumme | [`apps/gametest`](../apps/gametest/README.md): [`net_codec.rs`](../apps/gametest/src/net_codec.rs), [`net_lag.rs`](../apps/gametest/src/net_lag.rs), [`net_room.rs`](../apps/gametest/src/net_room.rs), [`net_sim.rs`](../apps/gametest/src/net_sim.rs), [`net_session.rs`](../apps/gametest/src/net_session.rs), [`net_loop.rs`](../apps/gametest/src/net_loop.rs), [`net_event.rs`](../apps/gametest/src/net_event.rs) |
| Echtes Netz, Browser als Client | [`apps/game1`](GAME1.md): `game1 --client` ([`client.rs`](../apps/game1/src/client.rs)), [`tests/browser.mjs`](../apps/game1/tests/browser.mjs), `game1-server --log` |

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
// Codes und Tickets aus dem Zufall des Systems; die Räume eines Ticks nebeneinander auf dem Pool
let mut spiel = NetServer::secure(opts, Box::new(|_code| Arena::new(ArenaConfig::default())));
spiel.set_parallel(true);
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
let ich = link.game.drawn();                                // die eigene Figur: zwischen zwei Ticks, Korrekturen geglättet
link.game.remotes(&mut andere);                             // die anderen, interpoliert
while let Some(e) = link.game.take_event() { /* Abschüsse, Treffer: je einmal, in Reihenfolge */ }
link.game.send_event(KANAL_CHAT, b"gg");                    // an den RoomSim des Raums
// beim Verlassen:
link.leave();                                               // LEAVE: der Server nimmt die Figur sofort heraus
```

## API-Überblick

- **Server** (`net::server`): `RoomSim` (`schema`, `baseline`, `join(name) -> Option<u16>`, `leave`, `step(tick, dt, &[(id,
  NetInput, eigen)])`, `snapshot(tick, &mut Snapshot)`, `priority(betrachter, &Ent) -> u32`, `on_event(spieler, &NetEvent)`,
  `take_events(&mut Vec<RoomEvent>)`), `RoomEvent { to: To::{All, Player, AllBut}, channel, data }`,
  `NetServer::new(ServerOptions, seed, RoomFactory)` (Codes aus einem Seed), `secure(ServerOptions, RoomFactory)` (aus
  `plat::random_u64`), `with_random(ServerOptions, fn() -> u64, RoomFactory)`, `set_parallel(bool)` (die Räume auf dem Pool;
  `RoomSim: Send`), `parallel`, `on_event(&ServerEvent)`, `advance(now_ns)`, `step`, `take_out`, `take_close`, `send_event(conn,
  kanal, bytes)`, `create_room`, `rooms`, `room`, `room_mut`, `players`, `player_of`, `stats` (`ServerStats`, neu `left`,
  `events_out`, `events_in`, `event_overflow`), `link_stats` (`LinkStats`, neu `events`, `events_in`), `max_rewind_ticks`, `tick`.
  `ServerOptions { rate, snapshot_every, max_players, max_rooms, budget, stream_budget, event_budget, margin, max_rewind_ms,
  ws_port, fetch_full, hello_ms, ticket_ms, empty_room_ms, start_tick, history }`.
- **Client** (`net::client`): `NetClient::new(ClientOptions, Movement, Schema, Grundlinie)`, `set_reliable`, `on_event`,
  `fetch_request`, `on_fetch_event`, `take_out`, `take_fetch_out`, `advance(now_ns)`, `step(map, NetInput) -> tick`, `drawn`,
  `smoothing`, `smoother`, `predicted`, `predicted_prev`, `alpha`, `remotes(&mut Vec<Remote>)`, `render_time`, `newest`,
  `send_event(kanal, bytes) -> bool`, `take_event`, `events_out`, `events_in`, `events_pending`, `leave`, `phase` (`Phase`,
  neu `Left`), `welcome`, `own_id`, `stats` (`ClientStats`), `predictor`, `clock`, `interp`, `last_outcome`; `lerp_field`,
  `lerp_angle_field`. `ClientOptions { name, room, interp_ms, margin, stream_margin, ring, min_redundancy, smooth_ms, snap,
  event_budget }`.
- **Ereignisse** (`net::event`): `NetEvent { tick, channel, data }`, `Outbox` (`push`, `take_run`, `ack`, `pending`,
  `stats`), `Inbox` (`receive`, `take`, `ack_for_packet`, `stats`), `write_section`, `read_section`, `seq_diff`;
  `MAX_EVENT` (255 Bytes), `CHANNELS` (16), `MAX_PENDING` (512), `ACK_PACKETS` (32).
- **Vorhersage** (`net::predict`): `Movement` (`State`, `FIELDS`, `step(map, &State, &NetInput, dt)`, `to_fields`,
  `from_fields`, `position`), `Predictor` (`step`, `reconcile -> Outcome::{Stale, Match, Corrected { error, shift }, Reset}`,
  `current`, `previous`, `at`, `confirmed`, `pending`, `stats` (`PredictStats`), `clear`), `Smoother` (`correct`, `advance`,
  `offset`, `reset`; `decay_ms`, `snap`, `corrections`, `snaps`).
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
- **Nachrichten** (`net::proto`: neu `MSG_LEAVE`, `EXT`, `has_ext`, `write_leave`, `is_leave`, `SnapHeader::new`,
  `InputMsg::new`; `VERSION` 2), **Raster** (`net::quant`: `pos_q`, `pos_f`, `angle_q`, `angle_f`, `pitch_q`, `axis_q`,
  `rot_q`, `rot_f`, `round_i64`), **Bitstrom** (`net::bits`: `BitWriter`, `BitReader`, `uvar`, `svar`).
- **Simulierte Leitung** (`net::sim`): `Weather` (`CLEAR`, `TYPICAL`, `ROUGH`, `STORM`), `SimNet` (`add_udp`, `add_ws`,
  `open_fetch`, `step`, `server_events`, `link_events`, `server_send`, `link_send`, `link_close`, `set_weather`), `PeerStats`.
- **Echte Sockets** (`net::link`): `serve(&mut netchan::Server, &mut NetServer, now_ns)` (Windows, Linux, Apple), `NetLink`
  (`connect`, `pump`, `flush`, `leave`, `channel`, `fetching`).
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
dem Tod sind für den Client Abweichungen wie jede andere. Das Bild blendet zwischen `predicted_prev` und `predicted` mit
`alpha` (wie `Pose` in [GAMECORE.md](GAMECORE.md)) und glättet Korrekturen (unten).

### Korrekturen glätten (Teil 2)

Die Vorhersage bleibt Bit für Bit beim Zustand des Servers; geglättet wird nur, was ein Bild zeigt (`NetClient::drawn`):
die vorhergesagte Lage zwischen den letzten zwei Ticks plus ein Versatz (`predict::Smoother`). Springt die gezeigte Lage bei
einem Abgleich um `d`, nimmt der Versatz `-d` auf: In diesem Augenblick bewegt sich das Bild nicht (geprüft: höchstens
1e-9 m). Danach klingt der Versatz ab wie exp(−t / 60 ms) (eine rationale Näherung in Teilschritten von höchstens einer
halben Zeitkonstante, für jede Bildrate auf 0,1 % gleich; nach rund 0,5 s unter 0,1 mm). Was über 1 m hinausgeht (ein
Neustart nach dem Tod, ein Teleport, auch wenn sich mehrere Korrekturen dahin aufaddieren), wird nicht geglättet: Das
Bild springt mit dem Zustand. `ClientOptions::smooth_ms` (60; 0: aus) und `snap` (1 m); Source glättet ähnlich
(`cl_smoothtime` 0,1 s). Die Kamera der eigenen Figur folgt dem gezeichneten Punkt, also auch dem Glätten.

### Zuverlässige Ereignisse (Teil 2)

Abschüsse, Trefferbestätigungen, Nachrichten wie in einem Chat: Sie müssen genau einmal ankommen, in Reihenfolge, über UDP
wie über WebSocket. In Teil 1 standen sie als Zähler in den Feldern der Figuren (zwei Ereignisse zwischen zwei
Schnappschüssen fielen zusammen, ein Name oder ein Text passte nicht hinein).

- **Wie** (`net::event`): Jede Richtung hat eine Folge von Nummern (u16, umlaufend). Der Sender hält jedes Ereignis, bis
  der Empfänger es bestätigt, und jedes Paket trägt die ältesten unbestätigten, bis zu einem Budget (400 Bytes, eines
  immer). Der Empfänger kennt die Nummer, die er als nächste erwartet: Was davor liegt, ist doppelt, der Rest wird in
  Reihenfolge geliefert; seine Bestätigung (die letzte Nummer ohne Lücke) reitet auf seinen nächsten 32 Paketen. Das ist
  die Regel der „reliable commands“ von Quake 3: Weil der Sender nur Bestätigtes vergisst, beginnt jeder Lauf, der ankommt,
  bei oder vor der erwarteten Nummer. Es gibt also nie eine Lücke, nichts wartet, kein Puffer zum Umordnen, kein Timer, kein
  eigenes Paket. Über einen WebSocket geht jedes Ereignis einmal (der Transport liefert). Ereignisse eines Raums reiten in
  den SNAPSHOTs, die eines Clients in den INPUTs; Bit 7 von Byte 0 (`EXT`) sagt, dass ein Teil mit Bestätigung und Lauf
  folgt: Ohne Ereignisse sind die Bytes die von Version 1 (die Prüfsumme `--netsum` blieb gleich).
- **Der Preis**: Ein Ereignis fährt in jedem Paket mit, bis seine Bestätigung zurück ist: bei 30 Schnappschüssen je Sekunde
  und 40 ms Umlauf (klares Wetter der Tests) zweimal, bei 100 ms vier- bis fünfmal. Eine Zeile der Abschussliste sind 3
  Bytes; ein Text von 200 Bytes kostet einige Pakete lang Platz im Budget von 1200 Bytes (das Budget der Entitäten wird
  danach gerechnet).
- **API**: `RoomSim::take_events` (nach jedem Tick: `RoomEvent` an alle, einen Spieler oder alle außer einem),
  `RoomSim::on_event` (die Ereignisse eines Spielers am Anfang des nächsten Ticks, nach Figur und Ankunft: deterministisch),
  `NetServer::send_event`, `NetClient::send_event` / `take_event`. Kanäle 0 bis 15 sind des Spiels; ein Ereignis hat bis zu
  255 Bytes und den Tick, in dem es geschah.
- **Grenzen**: Ein Client, der 512 Ereignisse nicht bestätigt, wird getrennt (`ServerStats::event_overflow`): Er ist weg oder
  kaputt, und genau einmal ginge sonst nicht. Ein Client verweigert `send_event`, wenn 512 warten.
- **Warum in `game::net` und nicht in `netchan`**: Netchan stückelt nicht und hat keine Zuverlässigkeit über UDP
  (Entscheidung vom 18.11.2026: ein schlanker Kanal); ein zuverlässiger Teilkanal dort bräuchte eigene Pakete, eigene Timer
  für Wiederholungen und Acks je Nachricht. Der Netcode schickt ohnehin in jedem Tick ein INPUT und alle zwei Ticks einen
  Schnappschuss, und er bestätigt schon, was er dekodierte: Die Ereignisse fahren umsonst mit, ihr Platz zählt im Budget
  des Schnappschusses, sie tragen den Tick der Simulation, und die Prüfsummen der Sitzungen umfassen sie. Ein anderes Spiel
  über netchan, das keinen Takt hat, müsste es selbst bauen; dann wäre es Zeit für netchan.

### Verlassen (Teil 2)

`NetClient::leave` schickt LEAVE (eine Nachricht von einem Byte) und steht danach auf `Phase::Left`; `NetLink::leave` schickt es
und beendet netchans Verbindung (DISCONNECT, über UDP dreimal). Der Server nimmt die Figur heraus, sobald eines von beiden
ankommt (`RoomSim::leave`), die anderen sehen sie im nächsten Schnappschuss gehen. Gemessen auf der simulierten Leitung mit 20
ms je Richtung: nach 21 ms beim Server, nach 55 ms bei den anderen, auch wenn LEAVE verloren geht. Ein Client, der stumm
wird (Absturz, Kabel), geht wie bisher nach netchans 5 s. Ein `NetLink`, das fallen gelassen wird, schickte schon in Teil 1
DISCONNECT; neu ist, dass der Client es selbst sagen kann (auch ohne Sockets, etwa im Üben von Spiel 1) und der Server ein
Verlassen von einem Abbruch unterscheidet (`ServerStats::left`).

### Räume auf Threads (Teil 2)

Ein Raum berührt in seinem Tick nichts eines anderen: seine Eingaben, sein `step`, seine Ereignisse, seine Schnappschüsse
(der Zustand je Spieler liegt seit Teil 2 im Raum). `NetServer::set_parallel(true)` lässt die Räume eines Ticks nebeneinander
auf dem Pool von `plat` laufen (`plat::parallel_for`, ein Raum je Element: bei genug Kernen ein Raum je Kern; die Zahl der
Threads mit `--threads N`). Ihre Nachrichten und Zähler werden in der Reihenfolge der Raumcodes zusammengeführt: Der Server
schickt Bit für Bit dasselbe wie auf einem Thread (Gruppe `threads`). Netchans Ereignisse, die Uhr, Beitritte und Codes
bleiben auf dem Thread der Schleife. Ziele ohne Threads (der Browser ohne gemeinsamen Speicher) rechnen die Räume
nacheinander; dort läuft ohnehin nur das Üben. Gegenweg: ein `NetServer` je Thread mit eigener Uhr; das spart das Warten
auf den langsamsten Raum, braucht aber ein Verteilen der Verbindungen nach Raumcode und gäbe keine Gleichheit mit einem
Thread. Gemessen [unten](#räume-nebeneinander).

### Raumcodes und Tickets aus dem Zufall des Systems (Teil 2)

`NetServer::new(opts, seed, …)` zieht Codes und Tickets aus einem xorshift64* mit Seed: gut für Tests, die simulierte Leitung
und das Üben im Browser, aber ein Ticket geht an seinen Client, und xorshift64* verrät seinen Zustand mit einer einzigen
Ausgabe (die Multiplikation ist umkehrbar). Wer ein Ticket sah, kennt alle folgenden Codes und Tickets. `NetServer::secure`
nimmt deshalb `plat::random_u64` (BCryptGenRandom, getrandom, getentropy, arc4random) für jeden Code und jedes Ticket;
`with_random` nimmt eine andere Quelle. `game1-server` nimmt `secure`. Im Browser hat `plat` keinen Zufall des Systems;
dort gibt es nur das Üben ohne Netz, das den Seed nimmt.

### Die Marge über WebSocket (Teil 2, nach der Messung)

Über UDP hielt die Marge von 2 Ticks auch über WLAN (0 bis 1,3 Korrekturen je Minute, im Mittel 3 mm, [unten](#echtes-netz)).
Über einen WebSocket waren es 12 je Minute: TCP liefert, was einem verlorenen Segment folgt, erst nach dessen Wiederholung,
und dann kommen Eingaben zu spät. Mit 3 Ticks 2,7 je Minute. Ein Client über einen WebSocket (`set_reliable`, das `NetLink`
für `Endpoint::WebSocket` setzt) zielt deshalb auf die Marge des Servers plus `ClientOptions::stream_margin` (1 Tick, 16,7
ms mehr Weg für seine Eingaben; die eigene Figur reagiert weiter sofort). Die Interpolation von 100 ms lief über WLAN nie
leer (mit 50 ms in 0,04 % der Runden); sie bleibt.

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
Karte. Passt er nicht in ein Datagramm, **füllen ihn die Schnappschüsse nach Priorität auf** (vom Nutzer entschieden am
19.11.2026; seine erste Entscheidung vom 18.11.2026, der WebSocket, ist eine Option: `ServerOptions::fetch_full`).

- Ein Browser-Client hat ohnehin nur den WebSocket: Sein erster Schnappschuss bekommt das große Budget (`stream_budget`, 60
  KiB), danach gilt das normale.
- Ein UDP-Client bekommt seinen vollen Stand nach und nach: Das Budget schneidet seine Schnappschüsse nach Priorität, er
  sieht seine Figur und das Wichtigste sofort, den Rest in den nächsten Schnappschüssen (bei 300 Requisiten, von denen
  150 bewegt wurden, ist er nach rund 200 ms vollständig, so schnell wie über den WebSocket; gemessen unten). Keine
  zweite Verbindung, kein Ticket, kein erreichbarer WebSocket-Port nötig.
- **Mit `fetch_full`** bekommt ein UDP-Client in WELCOME ein Ticket und den Port des WebSockets des Servers (`fetch`). Er öffnet eine zweite
  netchan-Verbindung (`Endpoint::WebSocket`) und schickt FETCH mit dem Ticket; der Server antwortet beim nächsten
  Schnappschuss-Tick mit FULL (ein Schnappschuss gegen die Grundlinie ohne Budget, beim Server als vollständiger Stand dieses
  Clients vermerkt) und schließt. Der Client meldet ihn in seinen INPUTs (`full`), und der Server nimmt ihn als Basis. Ein
  Ticket gilt 10 s und einmal.
  Bis dahin füllen die Schnappschüsse wie ohne `fetch_full` auf. Die Sitzung der Prüfsumme (`--netsum`), die Messungen
  (`--netbench`) und `--loopback` schalten `fetch_full` ein, damit dieser Weg weiter geprüft wird; die Gruppe `netsim`
  prüft beide Wege.

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

`python build_rs.py test gametest` führt `apps/gametest/tests/check.uis` (`--check`: auch die neun Gruppen des Netcodes) und
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
| `events` (Teil 2) | Outbox und Inbox über eine Modell-Leitung des Tests (300 Läufe, bis 50 % Verlust, doppelt, verspätet, Bestätigungen verloren, Budgets von 16 bis 415 Bytes; danach klar, bis alles durch ist): jedes Ereignis genau einmal und in Reihenfolge, nichts bleibt liegen; dasselbe zuverlässig (WebSocket); 70 000 Ereignisse über den Umlauf der Nummern. Sitzungen auf der simulierten Leitung mit einer Arena, deren Raum an alle, einen und alle außer einem sendet (auch 200 Bytes) und die Ereignisse der Clients zurückschickt, klar, rau und Sturm, 6 Clients (2 über WebSocket), die selbst senden: jeder bekommt genau die Ereignisse für ihn seit seinem Beitritt, einmal und in Reihenfolge (gegen das Buch des Raums), der Raum die jedes Clients einmal und in Reihenfolge, das Echo kommt zurück, keine Lücke, nichts offen, kein Schnappschuss über 1200 Bytes. Ein Client, der nie bestätigt, wird nach 512 offenen getrennt | 83 Prüfungen, 0 falsch; Modell: 395 114 Ereignisse, je 4,2 Pakete (UDP) bzw. 1,0; Sitzungen: 8428 / 8131 / 6464 Ereignisse an die Clients, 6406 / 6155 / 4801 von ihnen |
| `leave` (Teil 2) | LEAVE über UDP, LEAVE verloren (nur DISCONNECT), über WebSocket: die Figur beim Server weg, die anderen sehen sie gehen, `Phase::Left`, `ServerStats::left`; ein stummer Client erst nach netchans Zeitlimit | 10 Prüfungen, 0 falsch; 21 ms beim Server, 55 ms bei den anderen (20 ms je Richtung), stumm 5006 ms |
| `smooth` (Teil 2) | Der Versatz nimmt den Sprung auf, fällt stetig, nach 0,4 bis 0,8 s unter 0,1 mm; nach 100 ms in Schritten von 1, 7, 16, 33, 50 und 100 ms auf 1 % gleich und auf 0,5 % bei exp; über 1 m (auch aufaddiert) kein Glätten; 20 000 zufällige Korrekturen: nie über 1 m, nie gewachsen ohne Korrektur; `smooth_ms` 0 ist aus. Rau, 6 Clients, 10 s: bei jeder geglätteten Korrektur bewegt sich das gezeichnete Bild nicht (gemessen Ereignis für Ereignis), danach klar: kein Versatz, keine Korrektur mehr | 10 Prüfungen, 0 falsch; 18 Korrekturen geglättet, das Bild bewegte sich dabei um 0,0 m |
| `threads` (Teil 2) | 6 Räume mit je 3 Bots (Schießen, 40 Requisiten), rau, dann klar: einmal nacheinander, einmal nebeneinander auf dem Pool, alle Nachrichten des Servers Bit für Bit gleich. Codes: mit Seed zweimal gleich, aus einer gegebenen Quelle genau `from_random` ihrer Zahl, `secure` 60 verschiedene gültige Codes, zwei Tickets verschieden und nicht 0 | 6 Prüfungen, 0 falsch; 4518 Nachrichten (442 452 Bytes) gleich, auf 12 (Windows) und 8 (M2) Threads; ihre Prüfsumme `21edeb35d6a7aa94` ist auf beiden gleich |
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

| Ziel | die neun Gruppen des Netcodes (Teil 1: die ersten fünf) | `--loopback` |
| --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100, Entwicklung | 407 von 407 (Teil 1: 177 von 177) | bestanden (8 Clients, 5 s) |
| Linux x86-64 (glibc) in WSL Ubuntu 24.04, Entwicklung | `check.uis` bestanden (Teil 1: 176 von 176 mit `--quick`) | bestanden (8 Clients, 5 s) |
| macOS ARM64 (Mac mini M2, macOS 27.2.1), Rust 1.98.1, Entwicklung | 407 von 407 (Teil 1: 177 von 177) | Teil 1 bestanden (8 Clients, 6 s) |

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
| macOS ARM64 (Mac mini M2, macOS 27.2.1), Rust 1.98.1 | Entwicklung | `4118995e45c0c067` (Teil 2; in Teil 1 nur `52c4613d8ffdd103` mit der alten Grenze von 200 ms gerechnet) | `8edfe993e4ffcb65` |
| WebAssembly in Node 22.17 (V8): einfach, SIMD, Threads (`tests/web.mjs`) | Entwicklung | `4118995e45c0c067` (alle drei) | `8edfe993e4ffcb65` (alle drei; für den Kern neu) |

Die Grenze des Lag-Ausgleichs ging am 19.11.2026 von 200 auf 300 ms; damit treffen andere Schüsse, und die Prüfsumme der
Sitzung wechselte von `52c4613d8ffdd103` (177 304 Werte, auf allen vier Zielen gleich) zu `4118995e45c0c067`.

**Teil 2 änderte sie nicht**: Die Sitzung hat keine Ereignisse und kein Verlassen, und ohne sie sind die Nachrichten die von
Version 1 (`EXT` nicht gesetzt); der Umbau des Servers (der Zustand je Spieler im Raum, die Räume auf Threads) ergibt Bit für
Bit dieselben Nachrichten. `4118995e45c0c067` (177 246 Werte) nach Teil 2 auf Windows, Linux (WSL), macOS (M2) und in Node
(siehe GAME1.md für die Prüfsummen von Spiel 1, die sich änderten).

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

### Echtes Netz

Teil 2, am 20.11.2026: `game1-server` auf dem Mac mini (M2, macOS 27.2.1, über WLAN angebunden, `--bind` auf seine
Adresse, nur der Testport, danach beendet), Clients auf dem Windows-Rechner (Ryzen 5 3600, über Kabel) mit `game1 --client`
(Spiel 1, der Character-Controller, die Clients vom Verstand der Bots gespielt). Der Fedora-Rechner und das ThinkPad waren an
diesem Tag nicht erreichbar; Fedora als Server und der Mac als Client bleiben offen. Der Ping (ICMP) zum Mac: Median 3 ms,
p90 57 ms, höchstens 122 ms (das WLAN des Mac spart Strom: Spitzen, wenn eine Weile nichts kam). „Umlauf“ ist netchans
geglätteter Wert aus dem neuesten Ack; ein Ack reitet auf dem nächsten Paket der Gegenseite, also stecken darin bis zu 17 ms
Warten. „Schwankung“: die mittlere Änderung der Laufzeit eines Schnappschusses von einem zum nächsten (|D| nach RFC 3550)
und die Spanne ihres p1 bis p99. „Korrekturen“: Abgleiche mit Unterschied ab der zweiten Sekunde nach dem Beitritt.

| Lauf | Clients | Umlauf Mittel / p99 | Schwankung, Spanne | Verlust ab / auf | Korrekturen je Minute (Fehler Mittel) | Marge |
| --- | --- | --- | --- | --- | --- | --- |
| UDP, ohne Bots, 60 s | 3 | 17,3 / 34,4 ms | 3,0 ms, 28 ms | 0 / 0 % | 0 (jede der 5501 Vorhersagen exakt) | 2,00 Ticks |
| UDP, ohne Bots, 45 s | 2 | 16,8 / 39,4 ms | 2,7 ms, 28 ms | 0 / 0 % | 0,0 und 1,3 (3 mm) | 1,9 |
| WebSocket, Marge 2, 45 s | 2 | 18,1 / 58,6 ms | 2,7 ms, 33 ms | 0 / 0 % | 12,0 (4 und 16 mm) | 2,00 |
| WebSocket, Marge 3, 45 s | 2 | 19,9 / 69 ms | 3,3 ms, 42 ms | 0 / 0 % | 2,7 (5 mm) | 3,5 |
| WebSocket, Marge 3 als Vorgabe (`stream_margin`), 45 s | 2 | 16,3 / 38,6 ms | 2,3 ms, 27 ms | 0 / 0 % | 2,7 (1 und 28 mm) | 3,00 |
| UDP, 4 Bots auf dem Server, Schießen, 60 s | 3 | 18,6 / 51 ms | 3,6 ms, 34 ms | 0 / 0 % | 17–22 (Tode und Rückkehr: 8–9 gesprungen) | 2,3–2,5 |
| WebSocket, 4 Bots, Schießen, 40 s (Marge 2) | 2 | 28,3 / 84 ms | 7,1 ms, 45 ms | 0 / 0 % | 24 und 40 | 1,4 |

Die Interpolation von 100 ms lief in keinem Lauf leer; mit `--interp 50` in 0,04 % der Runden. Beim Server (`--log`):
außer den 4 Ticks vor der ersten Eingabe kein fehlender Tick über UDP; über den WebSocket mit der Marge von 2 Ticks 21–24
fehlende in 40 s (TCP: was einem verlorenen Segment folgt, wartet auf dessen Wiederholung). Daraus die Marge von 3 Ticks über
WebSocket ([oben](#die-marge-über-websocket-teil-2-nach-der-messung)). Mit Schießen kamen alle 63 Abschüsse bei jedem der drei
Clients an (Ereignisse), jeder Client verließ den Raum mit LEAVE (`3 left` beim Server).

Dabei gefunden: `game1-server` beantwortete UDP-Clients aus dem LAN nicht. `plat` prüft `UdpSocket::send_to` gegen
`net.connect`, und die PERMISSIONS des Servers erlaubten nur `localhost` (PERMDENY im Log); jetzt `net.connect *:*`.

### Der Browser als Client

`node apps/game1/tests/browser.mjs` (Playwright, headless): `game1-server --web` liefert auf 127.0.0.1 eine Kopie der
Web-Ausgabe, deren Loader `--client /game1` übergibt (der Loader von `web/` nimmt keine Argumente aus der Adresse); die Seite
spielt dann ohne Fenster zwei Clients mit dem Verstand der Bots über den WebSocket der Seite und vergleicht jede Vorhersage
mit dem Zustand des Servers (Abgleich). Chrome 157 und Firefox 153, je 12 s, ohne Bots: beide Clients im Raum, 330 bis 390
Schnappschüsse, keiner kaputt, keine fehlende Basis, jede Vorhersage exakt (0 Korrekturen), Umlauf 22–24 ms (der Takt des
Browsers), Schwankung 7 ms (Chrome) und 3 ms (Firefox); mit Bots und Schießen ebenso bestanden. Firefox trat zuerst erst nach
5,7 s bei: Die HTTP-Arbeiter des Servers (4) hingen an den offenen Verbindungen, mit denen die Seite geladen wurde (30 s
Keep-alive); jetzt 16 Arbeiter und 2 s. In einem Lauf hielt Firefox' Worker einmal rund 400 ms an (der Umlauf p99 400 ms, der
Rechner war belastet); der Netcode fing es auf.

### Räume nebeneinander

`gametest --netbench` (Release), zuletzt: 8 Räume mit je 16 Bots über die simulierte Leitung (300 Requisiten, Schießen,
typisches Wetter), 10 s, die Zeit von `NetServer::step`:

| Rechner | nacheinander: Mittel / p50 / p99 | nebeneinander: Mittel / p50 / p99 | schneller |
| --- | --- | --- | ---: |
| Ryzen 5 3600 (6 Kerne, 12 Threads), Windows, geteilt mit anderen Sitzungen | 1643 / 2617 / 3820 µs | 488 / 555 / 1442 µs | 3,4 × |

Der p50 liegt bei den Ticks mit Schnappschüssen. Ein Raum mit 16 Spielern braucht rund 200 µs je Tick (1,2 % eines Kerns bei
60 Hz); acht Räume nacheinander 10 %, nebeneinander 3 % der Zeit eines Ticks. Ein Kern trägt also viele Räume; die Threads
nehmen dem einen Thread der Schleife die Arbeit ab, wenn es viele werden. `gametest --roomsbench` misst nur diesen Teil.

## Was nicht getestet ist

- **Fedora als Server, der Mac als Client, Mobilfunk**: Das echte Netz lief zwischen dem Windows-Rechner (Kabel) und dem
  Mac (WLAN, Server). Fedora und das ThinkPad waren nicht erreichbar.
- **Android, iOS**: `lib/game` mit `game::net` baut für `aarch64-linux-android`, `aarch64-apple-ios` und
  `wasm32-unknown-unknown`; gelaufen ist dort nichts. Der statische Linux-Server von Spiel 1 läuft (WSL).
- **Mehr als 16 Spieler je Raum**, viele Räume unter echter Last mit echten Clients (die Räume auf Threads sind auf der
  simulierten Leitung gemessen).
- **Böswillige Clients über die Fuzz-Tests hinaus**: ein Client, der Eingaben weit in der Zukunft schickt, wird begrenzt (64
  Ticks), einer, der falsche Acks meldet, schadet nur sich (der Server nimmt nur Stände, die er gesendet hat), einer, der
  Ereignisse nicht bestätigt, wird nach 512 getrennt, einer, der Ereignisse flutet, ist durch netchans Rate begrenzt;
  Geschwindigkeit und Ziele prüft der Server, weil er rechnet. Schummeln mit Zielhilfen ist nicht Thema (GAME.md, „Bewusst
  nicht“).
- **Das Gefühl**: Ob 100 ms Interpolation, die Marge und das Glätten in 60 ms sich gut spielen, zeigt erst Spiel 1 mit
  Menschen.

## Was nicht gebaut ist

- Verschlüsselung über UDP (später, Entscheidung vom 18.11.2026); das Ticket für FETCH ist nur so geheim wie der Verkehr.
- Extrapolation der anderen, wenn Schnappschüsse fehlen (sie halten).
- Geschosse mit Flugzeit, Treffer auf Teile der Figur (Kopf: das Spiel), Figuren, die einander stoßen oder blockieren (die
  Vorhersage kennt die anderen nicht), Requisiten, die die Figuren aufhalten.
- Deltas gegen eine Vorhersage (Position + Geschwindigkeit × Zeit) statt gegen den Stand: spart bei laufenden Figuren
  vielleicht ein Drittel.
- Zuschauer, Wiedereinstieg nach einem Verbindungsabbruch mit derselben Figur, Teams, Runden: das ist Spiel 1 (G7h).
- Eingaben, die das Spiel selbst definiert (`NetInput` ist fest).
- Ein `NetServer` je Thread mit eigener Uhr (die Räume eines Ticks warten auf den langsamsten; bei Räumen gleicher Größe
  unwichtig).

## Offen

- **Fedora als Server, Clients auf dem Windows-Rechner und dem Mac** (der Rechner war am 20.11.2026 nicht erreichbar),
  WLAN beider Enden, ein Mobilfunk-Hotspot.
- **Die Verschlüsselung über UDP** (netchan, entschieden: ChaCha20-Poly1305 aus `crypto`).
- **Deltas gegen eine Vorhersage**.
- **Die Grenze des Lag-Ausgleichs**: 300 ms (entschieden am 19.11.2026; ob sie auf 400 ms geht, ist eine Frage an den Nutzer
  aus GAME1.md).
- **Ein UDP-Server und `net.connect`**: Ein Server, der UDP-Clients antwortet, braucht in seinen PERMISSIONS `net.connect *:*`,
  weil `plat` jedes `send_to` gegen diese Regel prüft. Ob Antworten an eine Adresse, die vorher an einen erlaubten
  lauschenden Socket schickte, ohne Regel durchgehen sollen, ist eine Frage für `plat` (docs/PERMISSIONS.md).

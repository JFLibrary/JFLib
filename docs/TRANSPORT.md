# TRANSPORT -- Netz für Spiele

Der Transport der Spiel-Engine ([GAME.md](GAME.md), Stufe G7d, Entscheidungen E6 und E8): Spiel 1 hat Mehrspieler im
Browser und auf dem Desktop, im Browser trägt WebSocket das Netz. Drei Teile:

1. **WebSocket in `netio`** (RFC 6455): die Server-Seite im HTTP-Server (der Handschlag, dann ein eigener Thread je
   Verbindung), ein Client für Tests und Desktop-Werkzeuge, der Rahmen-Parser für unsichere Bytes, Grenzen wie überall
   in `netio`.
2. **Der WebSocket des Browsers in `plat`**: `plat::net::WebSocket` über den JS-Host. Den Socket hält der Web-Worker,
   der auch die App ausführt; Rahmen gehen nie über die Seite. Die Berechtigungen der App (`net.connect`) gelten.
3. **Der Kanal für `lib/game`** im neuen Crate `lib/netchan` (`jflib-netchan`, in der Fassade `jflib::netchan`):
   Nachrichten mit Folgenummern und Bestätigungen, unzuverlässig und ungeordnet über UDP, zuverlässig über WebSocket,
   dieselbe Kopfzeile auf beiden. Ein Server nimmt UDP- und WebSocket-Clients in eine Tabelle.

Stand: G7d Teil 1 (dieser Lauf). Teil 2: `wss:` über das TLS 1.3 von `netio` geprüft, Messungen zwischen Rechnern und
über ein echtes Netz, die Grenzen nach diesen Messungen ([Für Teil 2](#für-teil-2)).

| Teil | Wo |
| --- | --- |
| WebSocket: Codec, Verbindung, Acceptor, Client | [`lib/netio/ws.rs`](../lib/netio/ws.rs); die Übergabe der Verbindung: `Body::Upgrade` in [`http.rs`](../lib/netio/http.rs), [`server.rs`](../lib/netio/server.rs) |
| Der WebSocket des Browsers | [`lib/plat/web_net.rs`](../lib/plat/web_net.rs), [`web/worker.js`](../web/worker.js) (Anfrage 32, `jfgui_ws`); die Prüfung: `net_host_allowed` in [`policy.rs`](../lib/plat/policy.rs) |
| Paketformat | [`lib/netchan/src/packet.rs`](../lib/netchan/src/packet.rs) |
| Folgenummern, Bestätigungen, Umlaufzeit | [`lib/netchan/src/conn.rs`](../lib/netchan/src/conn.rs) |
| Handschlag, Server und Client ohne Ein- und Ausgabe | [`lib/netchan/src/endpoint.rs`](../lib/netchan/src/endpoint.rs) |
| Server und Client mit Sockets, ein WebSocket-Typ für alle Ziele | [`server.rs`](../lib/netchan/src/server.rs), [`client.rs`](../lib/netchan/src/client.rs), [`ws.rs`](../lib/netchan/src/ws.rs) |
| Tests, Fuzzing, Messung, Browser-Test | [`apps/nettest`](../apps/nettest/README.md) |

## Ein Beispiel

```rust
use jflib::netchan::{Client, ClientConfig, ClientEvent, Endpoint, Server, ServerConfig, ServerEvent};

// der Server: Desktop-Spieler über UDP, Browser-Spieler über WebSocket, in einer Tabelle
let mut server = Server::new(ServerConfig { protocol: SPIEL1, ..Default::default() });
server.bind_udp(&udp_adresse)?;
server.listen_ws(&http_adresse, "/spiel")?;          // oder ws_endpoint() im eigenen netio-Handler
loop {
    server.wait(5);                                   // ohne eigenen Takt: schlafen, bis etwas kommt
    while let Some(e) = server.poll() {
        match e {
            ServerEvent::Connected { id, transport, .. } => {}
            ServerEvent::Message { id, seq, data } => { let _ = server.send(id, &data); }
            ServerEvent::Delivered { id, seq } => {}  // der Client hat das Paket seq
            ServerEvent::Lost { id, seq } => {}       // (UDP) es kam nicht an
            ServerEvent::Disconnected { id, reason } => {}
        }
    }
}

// der Client, jedes Bild: im Browser "/spiel" (der eigene Server der Seite), auf dem Desktop auch UDP
let mut client = Client::connect(Endpoint::WebSocket("/spiel".into()), ClientConfig { protocol: SPIEL1, ..Default::default() })?;
while let Some(e) = client.poll() {
    if let ClientEvent::Message { seq, data } = e { /* ein Schnappschuss */ }
}
let seq = client.send(&eingaben)?;
```

## API-Überblick

**`netio::ws`** (Windows, Linux, macOS, iOS)

- `Acceptor::new(Limits)`, `.origins(&[..])` (nur diese `Origin`, gegen fremde Seiten mit den Cookies des Besuchers),
  `.protocols(&[..])` (Unterprotokolle, das erste vom Client angebotene gewinnt), `.connections()`.
- `acceptor.accept(&req, |ws| ..) -> Response` im Handler eines `Server`: 101 und `on_open` auf einem eigenen Thread, bis
  er zurückkehrt; sonst 400 (kein WebSocket-Request), 403 (fremder Origin), 426 (andere Version, mit
  `Sec-WebSocket-Version: 13`), 503 (`max_connections`).
- `connect(url, &ClientConfig) -> WebSocket`: `ws://`, `wss://` über einen Connector wie bei `Client::with_connector`;
  `Sec-WebSocket-Accept` wird geprüft, Erweiterungen und nicht angebotene Unterprotokolle abgelehnt; alles innerhalb von
  `handshake_ms`.
- `WebSocket`: `recv()`, `recv_timeout(ms)`, `send_text`, `send_binary`, `send_fragmented(op, bytes, frame)`, `ping`,
  `close(code, reason)`, `peer`, `protocol`, `is_open`. Alles mit `&self`: ein Thread liest, andere senden
  (`Arc<WebSocket>`). Ein Ende des Gegenübers kommt als `Err(WsError::Closed(code, reason))`, ein eigenes Ende wegen
  eines Fehlers des Gegenübers als `Err(WsError::Protocol(code))`.
- `Decoder::new(Role, Limits)`, `feed`, `next() -> Frame` (Nachricht, Ping, Pong, Close) oder der Close-Code;
  `encode_frame`, `apply_mask`, `accept_key`, `close_code_valid`.
- `http::Body::Upgrade(Upgrade)`: das allgemeine Stück darunter. Eine Antwort mit 101 und diesem Body schickt der Server
  so, wie der Handler sie baute, und übergibt die Verbindung (`Upgraded`: Stream, schon gelesene Bytes, Adresse,
  Stopp-Flag) einem eigenen Thread. `Server::stop` wartet auch auf diese Threads.

**`plat::net`** (jetzt auf allen Zielen: Adressen und Fehler überall, Sockets auf dem Desktop)

- Im Browser `WebSocket::connect(url)` / `connect_with(url, WsLimits)`, `state()` (Connecting, Open, Closing, Closed),
  `send_text`, `send_binary`, `recv()` ohne zu warten, `close(code, reason)`, `close_info()`, `protocol()`, `pending()`.
  URLs `ws://`, `wss://` und `/pfad` für den eigenen Server der Seite (ws oder wss wie http oder https der Seite).

**`netchan`**

- `Client::connect(Endpoint::Udp(addr) | Endpoint::WebSocket(url), ClientConfig)`, `poll() -> ClientEvent`,
  `send(payload) -> seq`, `state()`, `reliable()`, `max_payload()`, `stats()`, `rtt_ms()`, `disconnect()`.
- `Server::new(ServerConfig)`, `bind_udp`, `listen_ws(addr, pfad)`, `ws_endpoint()` (`WsEndpoint::accept(&req)` im eigenen
  Handler, etwa neben den Dateien der Seite), `poll() -> ServerEvent`, `wait(ms)`, `send(id, payload) -> seq`,
  `disconnect(id)`, `connections()`, `ids()`, `stats(id)`, `peer(id)`, `server_stats()`.
- `WebSocket`: ein Client-Typ für alle Ziele, der nie wartet (im Browser der von `plat`, sonst der von `netio` auf zwei
  Threads); `unsent()`.
- Ohne Ein- und Ausgabe: `endpoint::ServerCore`, `endpoint::ClientCore` (die Zeit kommt vom Aufrufer), `conn::Conn`,
  `packet::parse`, `Packet::write`, `seq_newer`.

## Entscheidungen

### Ein eigenes Crate für den Kanal

Der Kanal liegt in `lib/netchan`, nicht in `netio` und nicht in `lib/game`:

- `netio` ist im Browser und auf Android nicht gebaut (dort hatte `plat` keine Sockets; TLS, Zertifikate, SSH brauchen
  sie). Der Client des Kanals muss aber im Browser laufen.
- Der eine WebSocket-Typ, den `lib/game` überall nimmt, braucht auf dem Desktop `netio` (Namen unter Linux gehen über
  `netio::dns`, `plat` allein löst dort nur Zahlen und `localhost` auf) und im Browser den JS-Host von `plat`. Das kann
  nur ein Crate über beiden je Ziel wählen: `netchan` hängt an `plat`, `crypto` und auf Windows, Linux, macOS und iOS an
  `netio`.
- `lib/game` entsteht in derselben Welle (G7b), und im Transport steckt kein Spiel. So bleibt `lib/game` ohne `cfg` für
  das Netz.

### WebSocket im HTTP-Server

Der Server von `netio` hat einen Pool von Workern, jeder bedient eine Verbindung zur Zeit. Eine WebSocket-Verbindung
lebt lange; hielte sie einen Worker, wären nach `threads` Spielern keine HTTP-Anfragen mehr möglich. Deshalb übergibt der
Server eine Antwort mit 101 und `Body::Upgrade` an einen eigenen Thread (256 KiB Stack) und ist sofort frei. `Body` ist
`#[non_exhaustive]`, die neue Variante bricht keine App. Der Acceptor zählt seine Verbindungen schon in `accept` (vor
dem Thread), damit ein Schwall von Anfragen die Grenze nicht zwischen Prüfung und Zählung überholt. Über
`Server::start_with` liegt TLS darunter, `wss:` kommt also ohne neuen Code; geprüft wird es in Teil 2.

Lesen: in Scheiben von 250 ms (`wait_readable`), damit das Stopp-Flag des Servers, der Ping nach `ping_ms` und das Ende
nach `idle_ms` ohne zweiten Thread gesehen werden. Schreiben: eine Nachricht wird in einem Stück kodiert und unter
einer Sperre geschrieben, Nachrichten über `send_frame` (64 KiB) in mehreren Rahmen. Masken des Clients aus 256 Bytes
des Zufallsgenerators des Systems, nachgefüllt alle 64 Rahmen.

### Der Browser: der Worker hält die Sockets

Die App läuft im Browser in einem Web-Worker, und WebSocket gibt es in Workern. Deshalb behandelt `web/worker.js` die
Anfrage 32 (`REQUEST_WEBSOCKET`, die Id des Sockets als Id der Anfrage, ein Op-Byte: öffnen, Text, Binär, schließen)
selbst und schickt sie nicht zur Seite: ein Weg weniger. Die Ereignisse des Browsers gehen über einen eigenen Export
`jfgui_ws(id, art, länge)` mit den Bytes im Eingangspuffer nach Rust (`jfgui_event` nimmt keine Typen über `E_DROP`).
Ein Schwall von Nachrichten endet in einem `input()` über einen `MessageChannel` (kein Timer, also keine Drosselung auf
4 ms), und `plat::wake` lässt den Lauf der UI eine Runde drehen (`E_WAKE`). Ein eigener Ereignistyp war nicht nötig.

Eine URL `/pfad` meint den eigenen Server der Seite: Der Worker setzt beim Start `jfgui_PAGE_ORIGIN`, Rust baut daraus
`ws://` oder `wss://`. Die Berechtigung prüft `policy::net_host_allowed(host, port)` vor der Anfrage an den Host: der
Host so, wie er in der URL steht (ein Name, `localhost`, eine Adresse), gegen die Regeln von `net.connect`. Der
Browser löst Namen selbst auf, es gibt also keinen Schritt dazwischen, den `plat` sehen könnte. Was ein Socket für die
App aufhebt, ist begrenzt (`WsLimits`: 1 MiB je Nachricht, 8 MiB in der Warteschlange); mehr schließt ihn, die App sieht
1009 in `close_info`.

Grenzen des Browsers: Eine Seite darf nur mit 1000 oder 3000 bis 4999 schließen. Wie viel der Browser noch zu senden
hat (`bufferedAmount`), sieht Rust nicht; `netchan` begrenzt deshalb, was das Spiel schickt, und nicht der Socket. Die
Sockets gehören dem Thread der App: Die Rechen-Worker des Builds mit Threads erreichen sie nicht.

### Paketformat

Byte 0 ist `VERSION << 4 | Art` (Version 1), danach Little-Endian:

| Art | Richtung | Inhalt | Bytes |
| --- | --- | --- | ---: |
| CONNECT | Client → Server | Protokoll-Id u32, Nonce u64 | 13, über UDP auf 64 aufgefüllt |
| CHALLENGE | Server → Client | Nonce u64, Zeit u32, MAC [16] | 29 (nur UDP) |
| RESPONSE | Client → Server | Protokoll-Id, Nonce, Zeit, MAC | 33, auf 64 aufgefüllt (nur UDP) |
| ACCEPT | Server → Client | Nonce, Token u32, größte Nutzlast u16 | 15 |
| DENY | Server → Client | Nonce, Grund u8 (voll, Protokoll, Adresse) | 10 |
| DATA | beide | Token u32, Folgenummer u16, Ack u16, Ack-Bits u32, Nutzlast | 13 + n |
| DISCONNECT | beide | Token u32, Grund u8 | 6 |

Eine Nachricht ist ein Paket: Der Kopf von DATA kostet 13 Bytes, die Nutzlast ist höchstens 1200 Bytes über UDP
(einstellbar bis 1400: 1400 + 13 + 8 + 40 bleiben unter den 1500 von Ethernet auch mit IPv6) und 64 KiB über
WebSocket. Über WebSocket trägt jede binäre Nachricht ein Paket, das Token ist 0. `packet::parse` liest genaue Größen,
ohne Allokation und ohne Panik.

### Handschlag über UDP

CONNECT (aufgefüllt) → CHALLENGE mit einem zustandslosen Cookie: die ersten 16 Bytes von HMAC-SHA256 über Adresse und
Port des Clients, seine Nonce und die Zeit in Sekunden, mit einem Geheimnis des Servers → RESPONSE (aufgefüllt) mit dem
Cookie → ACCEPT mit einem Token, das jedes DATA und DISCONNECT dann trägt. Für eine Adresse merkt sich der Server nichts,
bevor ihre RESPONSE beweist, dass sie dort empfängt; ein Cookie gilt 10 Sekunden und nur für seine Adresse. Jede
Antwort vor dem ACCEPT ist kleiner als die Anfrage, die sie auslöste (29 und 15 Bytes gegen 64): Ein gefälschter Absender
bekommt weniger zurück, als geschickt wurde. `challenges_per_s` (2000) begrenzt die Arbeit für alle Adressen zusammen
(eine HMAC je Antwort). Eine neue RESPONSE von einer Adresse mit Verbindung ersetzt die alte (der Client hat neu
gestartet); eine verlorene ACCEPT wird auf die wiederholte RESPONSE noch einmal geschickt. Der Client wiederholt CONNECT
oder RESPONSE alle 100 ms bis `connect_ms`.

Über WebSocket schickt der Client CONNECT, der Server antwortet ACCEPT oder DENY; TCP hat die Adresse bewiesen. Eine
Verbindung ohne CONNECT schließt der Server nach `handshake_ms`.

Das Token ist keine Verschlüsselung: Es hält Absender fern, die den Verkehr nicht sehen. Wer mitliest, kann Pakete
fälschen. Verschlüsselung (etwa ChaCha20-Poly1305 aus `crypto` mit Schlüsseln aus einem Austausch im Handschlag) ist
nicht gebaut.

### Folgenummern und Bestätigungen

Jedes DATA-Paket hat eine eigene Folgenummer (u16, umlaufend) und bestätigt die neueste empfangene Nummer und die 32
davor als Bits. Beide Seiten merken sich 1024 Nummern zurück.

- **Empfangen (UDP):** Eine Nummer, die schon kam, oder eine, die älter als das Fenster ist, wird verworfen; alles Neue
  wird sofort geliefert, in der Reihenfolge des Eintreffens. Der Netcode sortiert selbst nach der Nummer.
- **Bestätigt:** `Delivered(seq)`, sobald ein Ack das Paket nennt; `Lost(seq)`, sobald die Acks mehr als 32 Nummern
  darüber hinaus sind, ohne es zu nennen. Jedes gesendete Paket endet genau einmal so (die geprüfte Bilanz in
  `nettest --test sim`). Ein Paket, das noch kommt, nachdem die Acks 32 weiter sind (Umordnung um mehr als 32 Pakete),
  heißt `Lost` und wird trotzdem geliefert.
- **Ack-Pflicht:** Eine Seite, die nur empfängt, würde Pakete an den Bits vorbeiziehen lassen. Deshalb schickt eine
  UDP-Verbindung nach 16 empfangenen Paketen ohne eigenes sofort ein leeres DATA (`ACK_EVERY`). Ohne diese Regel kamen
  in der Simulation am Ende eines Schwalls falsche `Lost`.
- **Zuverlässig (WebSocket):** Der Transport ordnet und verliert nichts, also bestätigt ein Ack von n alles bis n.
  Nie `Lost`. Läuft das Fenster von 1024 ohne Antwort voll, gilt das älteste Paket als geliefert.
- **Umlaufzeit:** nur aus dem neuesten Ack (die Bits nennen ältere Pakete, deren Warten doppelt zählte), geglättet wie
  bei TCP (RFC 6298: 7/8 und 1/8, die Schwankung mit 3/4 und 1/4). Ein Ack reitet auf dem nächsten Paket der Gegenseite:
  Bei 60 Hz in beide Richtungen kommen bis zu 16 ms dazu.
- **Keep-alive:** ein leeres DATA nach `keepalive_ms` (100) ohne Senden; es trägt die Acks. Nichts empfangen seit
  `timeout_ms` (5000) beendet die Verbindung.

### Grenzen

| Grenze | Vorgabe | wo |
| --- | --- | --- |
| Rahmen, Nachricht (WebSocket) | 1 MiB, 1 MiB; 4096 Rahmen je Nachricht | `ws::Limits` |
| Nachrichten und Bytes je Sekunde (WebSocket) | 2000, 16 MiB; Stöße von zwei Sekunden | `ws::Limits` |
| Leerlauf, Ping, Handschlag, Schreiben | 60 s, 20 s, 10 s, 10 s | `ws::Limits` |
| Verbindungen je Acceptor | 1024 | `ws::Limits` |
| Nutzlast UDP / WebSocket | 1200 (höchstens 1400) / 64 KiB | `ServerConfig`, `ClientConfig` |
| Pakete und Bytes je Sekunde je Client | 300, 256 KiB (UDP: verworfen; WebSocket: getrennt) | `ServerConfig` |
| Verbindungen je Server, je Adresse | 64, 16 | `ServerConfig` |
| Antworten auf Handschläge je Sekunde | 2000 | `ServerConfig` |
| Zeitlimit, Keep-alive, CONNECT über WebSocket | 5 s, 100 ms, 5 s | `ServerConfig` |
| Ungesendetes je WebSocket-Client des Servers | 1 MiB (`send`: `Backlog`; die eigenen Pakete darüber: getrennt) | `server::LINK_UNSENT` |
| Ungesendetes des Desktop-WebSockets | 4 MiB (`send`: `Backlog`, nichts gesendet) | `WsOptions` |
| Warteschlange der lesenden Threads des Servers | 8192 Einträge (UDP verwirft, WebSocket wartet) | `server.rs` |
| Browser: Nachricht, Warteschlange, Sockets | 1 MiB, 8 MiB, 64 | `WsLimits`, `WS_MAX_SOCKETS` |

Die Vorgaben sind geschätzt; Teil 2 stellt sie nach Messungen ein. Ein Spiel mit 60 Hz Eingaben und Bestätigungen
braucht rund 70 Pakete je Sekunde je Client, 300 lassen Luft für Stöße.

## Tests

`apps/nettest` ([README](../apps/nettest/README.md)), `--check` mit allen Gruppen, auch `python build_rs.py test nettest`:

| Gruppe | Prüfungen | was |
| --- | ---: | --- |
| codec | 162 | Längen an ihren Grenzen (0, 125, 126, 65535, 65536), Masken an jedem Versatz gegen die einfache Schleife, Bytes einzeln, Steuerrahmen zwischen Fragmenten, jeder Close-Code 0 bis 5100, die Grenze des Decoders |
| ws | 63 | Echo (Text, Binär, leer, 1 MiB in 16 Rahmen), Text in Rahmen von 3 Bytes (UTF-8 über Rahmen geteilt), Server → Client in Rahmen von 1000, Ping und Pong (auch zwischen Fragmenten), Close in beide Richtungen, Unterprotokolle; 14 kaputte Rahmen mit ihrem Close-Code (unmaskiert, RSV, Opcodes, fragmentierter Ping, Fortsetzung ohne Anfang, neue Nachricht mitten in einer, UTF-8, Close-Codes, Close mit einem Byte) und 5 kaputte Köpfe (zu langer Ping, Längen nicht in kürzester Form, oberstes Bit, 4 GiB angekündigt); die Grenzen (Rahmen 1009 vor der Nutzlast, Nachricht 1009, Fragmente 1008, Nachrichten- und Byte-Rate 1008, Leerlauf 1001 nach 300 ms, Ping nach 200 ms); Fehler im Handschlag (ohne Schlüssel, kurzer Schlüssel, POST, ohne Upgrade, fremder und fehlender Origin, Version 8 mit 426, `max_connections` mit 503, Pfad ohne WebSocket) und beim Client (falsches Accept, keine Antwort in `handshake_ms`); Stopp des Servers mit offenen Verbindungen (1001); 32 Clients gleichzeitig mit je 200 Echos; Senden von einem zweiten Thread |
| sim | 211 | der Kanal auf einem simulierten Netz mit virtueller Uhr (siehe unten) |
| udp | 18 | Kanal über UDP auf Loopback (IPv4, IPv6): 2000 Echos bis 1200 Bytes, alle geliefert, Grenzen der Nutzlast, Trennen, Stopp des Servers (`BYE_SHUTDOWN`), Clients ohne `poll` laufen ab, `max_connections`, fremde Protokoll-Id, niemand da (`ConnectTimeout`), die Berechtigungen (192.0.2.1 und 0.0.0.0:45678 abgelehnt) |
| wschan | 17 | Kanal über WebSocket: 2000 Echos bis 8000 Bytes in Reihenfolge, 60 KB am Stück, Trennen; 12 UDP- und 12 WebSocket-Clients an einem Server mit je 200 Echos; falscher Pfad; Berechtigungen; der WebSocket-Typ von `netchan` gegen den Echo-Server von `netio` |
| fuzz | 4 | 2,12 Mio. Fälle mit festem Seed (siehe unten) |

Die Simulation (`sim`) ist die unabhängige Referenz für den Kanal: Ein Hauptbuch je Richtung hält fest, was unter
welcher Nummer gesendet wurde, was ankam und was die Acks behaupteten. Geprüft unter klarem Wetter, rauem Wetter (20 %
Verlust, 10 % doppelt, 20 % bis 30 ms später, 20 ms ± 5 ms; sechs Seeds), Sturm (50 % Verlust, 20 % doppelt, 30 % bis
60 ms später, 40 ms ± 20 ms), nur Duplikaten, und über den Umlauf der Nummern (70 000 Pakete): jede Nachricht höchstens
einmal und unverändert, `Delivered` nur für Pakete, die ankamen, jedes Paket genau einmal entschieden, falsche `Lost`
nur bei Umordnung (höchstens 2 %, ohne Umordnung keine), die Umlaufzeit gleich der des Netzes, zuverlässige Ströme
vollständig in Reihenfolge. Dazu: Handschlag unter 40 % Verlust (30 Seeds), keine Antwort vor ACCEPT größer als ihre
Anfrage, Keep-alives halten eine stille Verbindung 3 s (30 Pakete), ein durchgeschnittenes Kabel beendet beide Seiten
nach `timeout_ms`, die Obergrenzen, die Protokoll-Id, `ConnectTimeout`, die Rate über UDP (1000 Pakete gegen 100 je
Sekunde: rund 200 genommen, 800 verworfen, die Verbindung bleibt), gefälschte Cookies (fremde Adresse, 11 s alt), falsches
Token, CONNECT ohne Auffüllung, ein neu startender Client, Ströme ohne CONNECT, zu schnell, zu groß.

Fuzzing (`fuzz`, eigener Korpus aus einem festen Seed, keine fremden Fuzzer): 200 000 Byte-Folgen für den Rahmen-Decoder
(halb zufällig, halb gültige Ströme mit Mutationen, beide Rollen, in zufälligen Stücken gefüttert; kein Puffer über einen
Rahmen und 64 KiB, keine Nachricht über ihre Grenze, nur die vier erlaubten Close-Codes), 1,5 Mio. für den
Paket-Parser (zufällig, gültig mit Rundreise `parse` ∘ `write`, mutiert), 300 000 Datagramme und Strom-Nachrichten an
einen `ServerCore` (zufällige Adressen, nie über `max_connections`), 100 000 Läufe eines `ClientCore` in allen Phasen,
20 000 Läufe einer Verbindung mit zufälligen Nummern, Acks und Bits (nie mehr entschieden als gesendet). Nichts stürzt
ab.

Der Browser-Test (`tests/web.mjs`, Playwright über `tools/tests/playwright.mjs`): `nettest --serve` liefert die Seite des
Web-Builds, einen Echo-WebSocket auf `/ws` und den Kanal auf `/chan` (UDP auf derselben Portnummer, mit Ausweichen auf
ein zufälliges Paar, weil Windows Ports der Reihe nach vergibt). Die Seite öffnet `/ws` über `plat::net::WebSocket`,
prüft Echos (Text, 100 000 Bytes), 200 000 Bytes vom Server in Rahmen von 1000, misst 500 Umläufe und den Durchsatz,
schließt mit 1000, prüft, dass ein Host ohne Regel abgelehnt wird und es kein UDP gibt, und tauscht 300 Umläufe über
den Kanal auf `/chan`. Danach erreicht `nettest --client` denselben Server über UDP und WebSocket: Desktop und Browser
am selben Server, das „Fertig, wenn" von G7d.

Bestanden (18.11.2026): `--check` mit allen 475 Prüfungen auf Windows 11 (Ryzen 5 3600), Linux (WSL2, das statische
Programm ohne libc; `python build_rs.py test nettest` auf beiden) und macOS 27.2 (M2); der Browser-Test mit 11 Prüfungen
in Chrome 157 und Firefox 153 (Playwright, headless), danach der Desktop-Client am selben Server.

## Messwerte

Loopback, Ryzen 5 3600, Windows 11, `nettest --bench --n 2000` (Release). Beide Enden fragen ohne Pause ab, der Server
auf einem eigenen Thread; die Zahlen enthalten den Sprung vom lesenden Thread zu `poll()`, aber keinen Takt eines Spiels.

| Messung | Median | p99 | Durchsatz |
| --- | ---: | ---: | --- |
| Kanal über UDP, 32 Bytes hin und zurück | 0,046 bis 0,066 ms | 0,8 bis 4,4 ms | |
| Kanal über UDP, Flut mit 1200 Bytes | | | 8 bis 45 MB/s beim Server, 0 bis 0,9 % verloren |
| Kanal über WebSocket, 32 Bytes | 0,096 bis 0,102 ms | 0,8 bis 2,8 ms | |
| Kanal über WebSocket, Flut mit 16 KiB | | | 12 bis 232 MB/s beim Server |
| WebSocket von `netio`, 32 Bytes | 0,072 bis 0,090 ms | 0,17 bis 0,27 ms | |
| WebSocket von `netio`, Server → Client (64 KiB) | | | 761 bis 827 MB/s |
| WebSocket von `netio`, Echo (64 KiB) | | | 164 bis 253 MB/s je Richtung |

Drei Läufe; der Rechner war mit parallelen Builds anderer Sitzungen geteilt, deshalb die Spannen. Die Mediane der
Umlaufzeit sind stabil, die Fluten schwanken stark (beide Enden drehen ohne Pause auf einem geteilten Prozessor). Über
UDP gehen bei einer Flut wenige Prozent verloren (der Puffer des Sockets läuft über), über WebSocket nichts: `send`
meldet `Backlog`, bevor die Warteschlange voll ist.

Im Browser (dieselbe Maschine, Seite und Server auf Loopback, `web.mjs`; die Zeit der Seite ist in Chrome auf 0,1 ms,
in Firefox auf 1 ms gerundet):

| Messung | Chrome 157 | Firefox 153 |
| --- | --- | --- |
| WebSocket von `plat`, 32 Bytes hin und zurück (500) | Median 0,30 bis 0,40 ms, p99 1,8 bis 1,9 ms | Median 1,0 ms, p99 36 ms |
| Server → Browser, 16 MiB in Nachrichten von 64 KiB | 60 bis 88 MB/s | 85 MB/s |
| Browser → Server → Browser, 8 MiB Echo | 57 bis 69 MB/s je Richtung | 21 MB/s je Richtung |
| Kanal über WebSocket, 32 Bytes hin und zurück (300) | Median 0,40 ms, p99 2,2 ms | Median 1,0 ms, p99 3,0 ms |

Der Server dabei ist `nettest --serve` (Entwicklungs-Build), der zwischen zwei Runden mit `Server::wait` schläft; die
Seite ist der skalare Web-Build. Danach erreichte `nettest --client` denselben Server: über UDP Median 0,095 ms (p99
0,73 ms), über WebSocket 0,166 ms (p99 0,88 ms), je 500 Umläufe. Ein erster Lauf mit `plat::sleep_ms(1)` statt
`wait` im Server ergab 16 bis 45 ms: der Timer-Tick von Windows.

Auf dem Mac mini (M2, macOS 27.2, Release, dieselben Läufe):

| Messung | Median | p99 | Durchsatz |
| --- | ---: | ---: | --- |
| Kanal über UDP, 32 Bytes hin und zurück | 0,041 ms | 0,072 ms | |
| Kanal über UDP, Flut mit 1200 Bytes | | | 155 MB/s beim Server, 0,0 % verloren |
| Kanal über WebSocket, 32 Bytes | 0,045 ms | 0,108 ms | |
| Kanal über WebSocket, Flut mit 16 KiB | | | 1419 MB/s beim Server |
| WebSocket von `netio`, 32 Bytes | 0,022 ms | 0,074 ms | |
| WebSocket von `netio`, Server → Client (64 KiB) | | | 3310 MB/s |
| WebSocket von `netio`, Echo (64 KiB) | | | 2469 MB/s je Richtung |

Unter Linux in WSL2 (Ubuntu 24.04 auf demselben Ryzen, eine virtuelle Maschine, Release):

| Messung | Median | p99 | Durchsatz |
| --- | ---: | ---: | --- |
| Kanal über UDP, 32 Bytes | 0,106 ms | 16,7 ms | |
| Kanal über UDP, Flut mit 1200 Bytes | | | 23 MB/s, 15 % verloren |
| Kanal über WebSocket, 32 Bytes | 0,216 ms | 15,7 ms | |
| Kanal über WebSocket, Flut mit 16 KiB | | | 91 MB/s |
| WebSocket von `netio`, 32 Bytes | 0,107 ms | 2,1 ms | |
| WebSocket von `netio`, Server → Client, Echo | | | 184 MB/s, 109 MB/s je Richtung |

Die p99 um 16 ms sind Planungslücken der virtuellen Maschine; Linux auf echter Hardware (Fedora, ThinkPad) misst Teil 2.

## Was nicht getestet ist

- `wss:` (TLS darunter): verdrahtet (Server über `start_with`, Client über `ClientConfig::connector`), nicht geprüft
  (Teil 2).
- Ein echtes Netz und zwei Rechner: alle Messungen sind auf Loopback (Teil 2).
- Safari und der Build mit Threads im Browser (die Sockets gehören dem Thread der App; aus einem Rechen-Worker gerufen
  bleiben sie stumm).
- Viele Clients mit echter Last: 24 gleichzeitig im Test, 32 am rohen WebSocket; 16 Spieler je Raum misst G7e.
- Firefox' 1-ms-Takt verdeckt Umlaufzeiten darunter.

## Was nicht gebaut ist

- Verschlüsselung und Authentifizierung über UDP (das Token hält nur blinde Absender fern).
- Nachrichten über 1400 Bytes über UDP (keine Fragmentierung): Große, seltene Daten (die Karte, der volle Stand beim
  Beitritt) braucht der Netcode in Stücken oder über einen WebSocket daneben.
- Ein zuverlässiger, geordneter Teilkanal über UDP.
- Android (`plat` hat dort noch keine Sockets: `Error::Unsupported`, [ANDROID.md](ANDROID.md)).
- WebTransport (E8: erst mit QUIC in `netio`), `permessage-deflate`.
- Ein `wait` für den Client (Spiele fragen jedes Bild ab).

## Für G7e

- **Delta gegen den bestätigten Stand:** `Delivered(seq)` sagt, welcher Schnappschuss beim Client ist; über WebSocket
  kommt alles in Reihenfolge an, dort darf gegen den zuletzt gesendeten Stand kodiert werden.
- **Reihenfolge über UDP:** Nachrichten kommen in der Reihenfolge des Eintreffens; ein älterer Schnappschuss nach einem
  neueren ist an der Folgenummer (oder besser an der Tick-Nummer in der Nutzlast) zu erkennen und zu verwerfen.
- **Bestätigungen brauchen Gegenverkehr:** Die Ack-Pflicht nach 16 Paketen deckt eine Seite ab, die nur empfängt; bei
  Umordnung um mehr als 32 Pakete meldet der Kanal `Lost` für Pakete, die noch kommen.
- **Größe:** 1200 Bytes je UDP-Nachricht; `client.max_payload()` sagt die Grenze des Servers. Über WebSocket bis
  64 KiB, aber dieselbe Grenze für beide hält den Netcode einfach.
- **Rate:** 300 Pakete je Sekunde je Client in der Vorgabe; Bots in einem Prozess brauchen `max_per_ip` und
  `packets_per_s` passend.
- **Der Server ohne Takt:** `server.wait(ms)` schläft, bis etwas kommt; mit festem Takt genügt `poll()` je Tick. Ein
  Schlaf mit `plat::sleep_ms(1)` kostet unter Windows einen Timer-Tick (15,6 ms).
- **Zeit:** `endpoint::ServerCore` und `ClientCore` nehmen die Zeit vom Aufrufer; die simulierte Leitung von G7e kann
  `nettest/src/sim.rs` als Vorlage nehmen.

## Für Teil 2

- `wss:` mit dem TLS von `netio` prüfen (Server mit eigener CA, Client mit `trust::connector`; im Browser gegen ein
  Zertifikat, dem der Browser traut).
- Messen zwischen Rechnern (Windows ↔ Fedora, ThinkPad, Mac mini) und über WLAN; Verlust und Schwankung echter Netze.
- Die Grenzen nach diesen Messungen einstellen; prüfen, ob 1200 Bytes und 300 Pakete je Sekunde zu Spiel 1 passen.
- `bufferedAmount` des Browsers an Rust melden, wenn der Netcode im Browser Gegendruck braucht.

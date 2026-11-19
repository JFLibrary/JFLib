// net_room -- rooms, codes, links and names, and the server's join and leave driven by netchan events
// written here (no network): HELLO makes or joins a room, the name is cleaned and made unique, a full
// room, an unknown code, another version, an empty name are refused; leaving frees the figure; an empty
// room closes after its time; a connection without HELLO is ended; a WebSocket client's first snapshot
// carries the whole state, a UDP client's whose state does not fit gets a ticket

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use game::net::arena::{Arena, ArenaConfig};
use game::net::proto::{self, Hello};
use game::net::room::{clean_name, room_from_link, room_link, unique_name, RoomCode, CODE_ALPHABET};
use game::net::server::{NetServer, ServerOptions};
use netchan::endpoint::ServerEvent;
use netchan::Transport;

use crate::{Ctx, Rng};

pub fn run(c: &mut Ctx) {
    codes(c);
    names(c);
    joins(c);
}

fn codes(c: &mut Ctx) {
    let mut r = Rng::new(21);
    let mut ok = true;
    let mut distinct = alloc::collections::BTreeSet::new();
    for _ in 0..c.n(20_000) {
        let code = RoomCode::from_random(r.next_u64());
        ok &= code.valid() && RoomCode::parse(code.as_str()) == Some(code);
        ok &= RoomCode::parse(&code.as_str().to_ascii_lowercase()) == Some(code);
        distinct.insert(code);
    }
    c.check(ok, || String::from("codes: valid, parse back, any case"));
    c.check(distinct.len() * 100 >= c.n(20_000) * 99, || format!("codes: {} distinct", distinct.len()));
    c.check(!CODE_ALPHABET.iter().any(|b| b"01ILO".contains(b)), || String::from("no look-alikes in the alphabet"));
    let k = RoomCode::parse("K7M-2QX").unwrap_or(RoomCode(*b"AAAAAA"));
    c.check(k.as_str() == "K7M2QX", || format!("dashes ignored: {}", k));
    for bad in ["K7M2Q", "K7M2QXA", "K7M2Q0", "K7M2QI", "", "äöüäöü"] {
        c.check(RoomCode::parse(bad).is_none(), || format!("refused code {:?}", bad));
    }
    let link = room_link("https://spiel.example/play#old", &k);
    c.check(link == "https://spiel.example/play#K7M2QX", || format!("link {}", link));
    for l in ["https://spiel.example/play#K7M2QX", "https://spiel.example/?room=k7m2qx&x=1", "https://spiel.example/r/K7M2QX", "  k7m 2qx ", "spiel://host/r/K7M2QX?x"] {
        c.check(room_from_link(l) == Some(k), || format!("code from {:?}: {:?}", l, room_from_link(l)));
    }
    c.check(room_from_link("https://spiel.example/play").is_none(), || String::from("a link without a code"));
}

fn names(c: &mut Ctx) {
    let cases: &[(&str, Option<&str>)] = &[
        ("  Anna  ", Some("Anna")),
        ("Jörg   Müller", Some("Jörg Müller")),
        ("a\u{202e}b\u{200b}c\td", Some("abc d")),
        ("\u{1}\u{7f}", None),
        ("   ", None),
        ("ABCDEFGHIJKLMNOPQRSTUVWXYZ", Some("ABCDEFGHIJKLMNOP")),
        ("abcdefghijklmno pq", Some("abcdefghijklmno")),
        ("Ελληνικά όνομα", Some("Ελληνικά όνομα")),
    ];
    for (raw, want) in cases {
        let got = clean_name(raw);
        c.check(got.as_deref() == *want, || format!("clean_name({:?}) = {:?}, want {:?}", raw, got, want));
    }
    let taken = ["Anna", "anna 2", "ABCDEFGHIJKLMNOP"];
    c.check(unique_name("ANNA", &taken) == "ANNA 3", || format!("unique {:?}", unique_name("ANNA", &taken)));
    c.check(unique_name("Bert", &taken) == "Bert", || String::from("unique free"));
    let u = unique_name("ABCDEFGHIJKLMNOP", &taken);
    c.check(u == "ABCDEFGHIJKLMN 2" && u.chars().count() == 16, || format!("unique long {:?}", u));
}

fn server(opts: ServerOptions, props: usize) -> NetServer<Arena> {
    let cfg = ArenaConfig { props, moving: props, ..ArenaConfig::default() };
    NetServer::new(opts, 77, alloc::boxed::Box::new(move |_| Arena::new(cfg)))
}

fn outs(s: &mut NetServer<Arena>) -> (Vec<(u32, Vec<u8>)>, Vec<u32>) {
    let mut o = Vec::new();
    while let Some(x) = s.take_out() {
        o.push(x);
    }
    let mut cl = Vec::new();
    while let Some(x) = s.take_close() {
        cl.push(x);
    }
    (o, cl)
}

fn hello(s: &mut NetServer<Arena>, id: u32, ws: bool, room: Option<RoomCode>, name: &str) {
    s.on_event(&ServerEvent::Connected { id, transport: if ws { Transport::WebSocket } else { Transport::Udp }, addr: None });
    let h = Hello { version: proto::VERSION, room, name: String::from(name) };
    s.on_event(&ServerEvent::Message { id, seq: 0, data: proto::write_hello(&h) });
}

fn joins(c: &mut Ctx) {
    let opts = ServerOptions { max_players: 3, max_rooms: 2, empty_room_ms: 1000, hello_ms: 500, ws_port: 9000, ..ServerOptions::default() };
    let mut s = server(opts, 0);
    hello(&mut s, 1, false, None, "  Anna ");
    let (o, _) = outs(&mut s);
    let w = o.first().and_then(|(_, b)| proto::read_welcome(b));
    c.check(w.as_ref().is_some_and(|w| w.player == 1 && !w.fetch && w.rate == 60 && w.snapshot_every == 2), || format!("first HELLO: {:?}", w));
    let code = w.map(|w| w.room).unwrap_or(RoomCode(*b"AAAAAA"));
    c.check(s.rooms() == [code], || String::from("a room was made"));
    hello(&mut s, 2, true, Some(code), "anna");
    hello(&mut s, 3, false, Some(code), "Carl");
    let (o, _) = outs(&mut s);
    c.check(o.len() == 2 && o.iter().all(|(_, b)| proto::read_welcome(b).is_some()), || format!("two more joined: {} messages", o.len()));
    let names: Vec<String> = s.players(&code).into_iter().map(|p| p.2).collect();
    c.check(names == ["Anna", "anna 2", "Carl"], || format!("names {:?}", names));
    // refused: a full room, an unknown room, another version, an empty name
    hello(&mut s, 4, false, Some(code), "Dora");
    hello(&mut s, 5, false, Some(RoomCode(*b"ZZZZZZ")), "Eve");
    s.on_event(&ServerEvent::Connected { id: 6, transport: Transport::Udp, addr: None });
    s.on_event(&ServerEvent::Message { id: 6, seq: 0, data: proto::write_hello(&Hello { version: 99, room: None, name: String::from("F") }) });
    hello(&mut s, 7, false, None, "\u{7}");
    let (o, cl) = outs(&mut s);
    let reasons: Vec<Option<u8>> = o.iter().map(|(_, b)| proto::read_refuse(b)).collect();
    c.check(reasons == [Some(proto::REFUSE_ROOM_FULL), Some(proto::REFUSE_NO_ROOM), Some(proto::REFUSE_VERSION), Some(proto::REFUSE_NAME)] && cl == [4, 5, 6, 7], || format!("refusals {:?} closes {:?}", reasons, cl));
    // the first snapshot: the WebSocket client gets the whole state (from the map's baseline)
    s.step();
    s.step();
    let (o, _) = outs(&mut s);
    let to2 = o.iter().find(|(id, _)| *id == 2).and_then(|(_, b)| proto::read_snap_header(b).map(|h| h.1));
    c.check(to2.is_some_and(|h| h.base == proto::Base::Map && h.player == 2), || format!("first snapshot over WebSocket: {:?}", to2));
    c.check(o.len() == 3, || format!("snapshots of a tick: {}", o.len()));
    // leaving frees the figure and the place
    s.on_event(&ServerEvent::Disconnected { id: 1, reason: netchan::Reason::Timeout });
    let left = s.room(&code).map(|a| a.figures().len());
    c.check(left == Some(2), || format!("figures after a leave: {:?}", left));
    hello(&mut s, 8, false, Some(code), "Dora");
    let (o, _) = outs(&mut s);
    let w = o.first().and_then(|(_, b)| proto::read_welcome(b));
    c.check(w.as_ref().is_some_and(|w| w.player == 1), || format!("the freed place: {:?}", w));
    // a second generation of id 1: the snapshot says so
    s.step();
    s.step();
    let (_, _) = outs(&mut s);
    let g = s.room(&code).and_then(|a| a.figures().get(&1).map(|f| f.generation));
    c.check(g == Some(1), || format!("generation of the reused id: {:?}", g));
    // a connection without HELLO is ended after hello_ms; an empty room closes after empty_room_ms
    s.on_event(&ServerEvent::Connected { id: 9, transport: Transport::Udp, addr: None });
    for _ in 0..40 {
        s.step();
    }
    let (_, cl) = outs(&mut s);
    c.check(cl.contains(&9), || format!("silent connection closed: {:?}", cl));
    for id in [2, 3, 8] {
        s.on_event(&ServerEvent::Disconnected { id, reason: netchan::Reason::Timeout });
    }
    c.check(s.rooms().len() == 1, || String::from("the empty room stays a while"));
    for _ in 0..70 {
        s.step();
    }
    c.check(s.rooms().is_empty(), || format!("the empty room closed: {:?}", s.rooms()));
    // the server's rooms: max_rooms
    hello(&mut s, 10, false, None, "A");
    hello(&mut s, 11, false, None, "B");
    hello(&mut s, 12, false, None, "C");
    let (o, _) = outs(&mut s);
    let last = o.last().and_then(|(_, b)| proto::read_refuse(b));
    c.check(s.rooms().len() == 2 && last == Some(proto::REFUSE_SERVER_FULL), || format!("max_rooms: {} rooms, {:?}", s.rooms().len(), last));
    // a big state: a UDP client gets a ticket, a WebSocket client not; FETCH with it gives FULL at the next
    // snapshot tick and the connection ends; a wrong ticket is refused
    // (the props move: after two seconds they are away from the map's baseline)
    // the default (since 2026-11-19): no ticket, the snapshots fill the full state in
    let mut d = server(ServerOptions { ws_port: 9000, ..ServerOptions::default() }, 300);
    let made_d = d.create_room();
    for _ in 0..120 {
        d.step();
    }
    hello(&mut d, 1, false, made_d, "Udp");
    let (o, _) = outs(&mut d);
    let wd = o.first().and_then(|(_, b)| proto::read_welcome(b));
    c.check(wd.as_ref().is_some_and(|w| !w.fetch && w.ticket == 0 && w.full_bytes > 1200), || format!("UDP with 300 props, no fetch_full: {:?}", wd));
    // the option: a ticket for the fetch over a WebSocket
    let mut s = server(ServerOptions { ws_port: 9000, fetch_full: true, ..ServerOptions::default() }, 300);
    let made = s.create_room();
    for _ in 0..120 {
        s.step();
    }
    hello(&mut s, 1, false, made, "Udp");
    let (o, _) = outs(&mut s);
    let w = o.first().and_then(|(_, b)| proto::read_welcome(b));
    let fetch = w.as_ref().is_some_and(|w| w.fetch && w.ticket != 0 && w.ws_port == 9000 && w.full_bytes > 1200);
    c.check(fetch, || format!("UDP with 300 props: {:?}", w));
    let code = w.as_ref().map_or(RoomCode(*b"AAAAAA"), |w| w.room);
    hello(&mut s, 2, true, Some(code), "Ws");
    let (o, _) = outs(&mut s);
    let w2 = o.first().and_then(|(_, b)| proto::read_welcome(b));
    c.check(w2.as_ref().is_some_and(|w| !w.fetch), || format!("WebSocket with 300 props: {:?}", w2));
    s.on_event(&ServerEvent::Connected { id: 3, transport: Transport::WebSocket, addr: None });
    s.on_event(&ServerEvent::Message { id: 3, seq: 0, data: proto::write_fetch(w.as_ref().map_or(0, |w| w.ticket)) });
    s.on_event(&ServerEvent::Connected { id: 4, transport: Transport::WebSocket, addr: None });
    s.on_event(&ServerEvent::Message { id: 4, seq: 0, data: proto::write_fetch(12345) });
    let (o, cl) = outs(&mut s);
    c.check(o.len() == 1 && proto::read_refuse(&o[0].1) == Some(proto::REFUSE_TICKET) && cl == [4], || String::from("a wrong ticket"));
    s.step();
    s.step();
    let (o, cl) = outs(&mut s);
    let full = o.iter().find(|(id, _)| *id == 3).map(|(_, b)| (b.len(), proto::read_snap_header(b).map(|h| h.0)));
    let udp = o.iter().any(|(id, _)| *id == 1);
    let ws_first = o.iter().find(|(id, _)| *id == 2).map(|(_, b)| b.len());
    c.check(full.is_some_and(|f| f.1 == Some(proto::MSG_FULL) && f.0 > 1200) && cl.contains(&3) && !udp, || format!("FULL: {:?}, closes {:?}, a UDP snapshot as well {}", full, cl, udp));
    c.check(ws_first.is_some_and(|n| n > 1200), || format!("the WebSocket's first snapshot: {:?} bytes", ws_first));
    crate::out(&format!("  join with 300 props: FULL {} bytes, WebSocket's first snapshot {} bytes\n", full.map_or(0, |f| f.0), ws_first.unwrap_or(0)));
}

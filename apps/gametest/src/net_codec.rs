// net_codec -- the netcode's parts one by one against references of their own: the bit stream and its
// codes, the quantisation grids, the delta encoding of snapshots against a plain full encoding written
// here (and the budget's cut against its rules), malformed bytes, the input packets and messages, the
// server's input buffer, the predictor against a replay computed here, the two client clocks

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use game::net::bits::{svar_bits, uvar_bits, BitReader, BitWriter};
use game::net::clock::{InterpClock, SteeredClock};
use game::net::input::{read_inputs, write_inputs, InputBuffer, NetInput};
use game::net::predict::{Movement, Outcome, Predictor};
use game::net::proto::{self, Base, Hello, InputMsg, SnapHeader, Welcome};
use game::net::quant::{angle_f, angle_q, axis_f, axis_q, pitch_f, pitch_q, pos_f, pos_q, rot_f, rot_q};
use game::net::room::RoomCode;
use game::net::snapshot::{self, apply, decode, diff, encode, encoded_bits, select, tick_diff, Change, Ent, NetState, Op, Schema, Snapshot, MAX_FIELDS};
use game::{Quat, Vec3};

use crate::{Ctx, Rng};

pub fn run(c: &mut Ctx) {
    bits(c);
    quant(c);
    delta(c);
    budget(c);
    malformed(c);
    inputs(c);
    messages(c);
    buffer(c);
    predictor(c);
    clocks(c);
}

fn bits(c: &mut Ctx) {
    let mut r = Rng::new(1);
    let n = c.n(20_000);
    let mut bad = 0;
    for _ in 0..n {
        let k = 1 + r.below(40);
        let vals: Vec<(u32, u32)> = (0..k).map(|_| {
            let w = r.below(33) as u32;
            let v = r.next_u64() as u32;
            (if w == 32 { v } else if w == 0 { 0 } else { v & ((1u32 << w) - 1) }, w)
        }).collect();
        let mut w = BitWriter::new();
        let mut total = 0;
        for &(v, n) in &vals {
            w.bits(v, n);
            total += n as usize;
        }
        if w.bit_len() != total {
            bad += 1;
        }
        let b = w.finish();
        let mut rd = BitReader::new(&b);
        for &(v, n) in &vals {
            if rd.bits(n) != v {
                bad += 1;
            }
        }
        if !rd.ok() {
            bad += 1;
        }
    }
    c.check(bad == 0, || format!("bits: {} mismatches in {} random streams", bad, n));
    // the codes at their edges, and their sizes
    let us = [0u32, 1, 15, 16, 255, 256, 65535, 65536, u32::MAX];
    let ss = [0i32, 1, -1, 31, -32, 32, -33, 511, -512, 512, -513, 32767, -32768, 32768, i32::MAX, i32::MIN];
    let mut w = BitWriter::new();
    let mut size = 0;
    for &u in &us {
        w.uvar(u);
        size += uvar_bits(u);
    }
    for &s in &ss {
        w.svar(s);
        size += svar_bits(s);
    }
    c.check(w.bit_len() == size, || format!("uvar/svar sizes: {} written, {} computed", w.bit_len(), size));
    let b = w.finish();
    let mut rd = BitReader::new(&b);
    let ok_u = us.iter().all(|&u| rd.uvar() == u);
    let ok_s = ss.iter().all(|&s| rd.svar() == s);
    c.check(ok_u && ok_s && rd.ok(), || format!("uvar/svar round trip: {} {}", ok_u, ok_s));
    // reading past the end: zeros and the flag, never a panic
    let mut rd = BitReader::new(&[0xff]);
    let a = rd.bits(8);
    let z = rd.bits(5);
    c.check(a == 0xff && z == 0 && !rd.ok(), || format!("past the end: {} {} {}", a, z, rd.ok()));
    let mut sizes_ok = true;
    for v in [0u32, 7, 15, 16, 200, 255, 256, 1000, 65535, 65536, 1 << 30] {
        let mut w = BitWriter::new();
        w.uvar(v);
        sizes_ok &= w.bit_len() == uvar_bits(v);
    }
    c.check(sizes_ok, || String::from("uvar_bits"));
}

fn quant(c: &mut Ctx) {
    let mut r = Rng::new(2);
    let n = c.n(100_000);
    let (mut worst_pos, mut worst_ang, mut worst_pitch, mut worst_axis) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut exact = true;
    for _ in 0..n {
        let x = r.range(-1e5, 1e5);
        worst_pos = worst_pos.max((pos_f(pos_q(x)) - x).abs());
        let q = (r.next_u64() as i32) >> 4;
        exact &= pos_q(pos_f(q)) == q;
        let a = r.range(-50.0, 50.0);
        let d = game::camera::wrap_angle(angle_f(angle_q(a)) - a).abs();
        worst_ang = worst_ang.max(d);
        let p = r.range(-1.5, 1.5);
        worst_pitch = worst_pitch.max((pitch_f(pitch_q(p)) - p).abs());
        let v = r.range(-1.0, 1.0);
        worst_axis = worst_axis.max((axis_f(axis_q(v)) - v).abs());
    }
    let step_ang = core::f64::consts::TAU / 65536.0;
    c.check(worst_pos <= 0.5 / 1024.0 + 1e-12 && exact, || format!("positions: worst {:.3e} m (half a step {:.3e}), grid exact {}", worst_pos, 0.5 / 1024.0, exact));
    c.check(worst_ang <= step_ang * 0.5 + 1e-12, || format!("angles: worst {:.3e} rad, half a step {:.3e}", worst_ang, step_ang * 0.5));
    c.check(worst_pitch <= step_ang * 0.5 + 1e-12, || format!("pitch: worst {:.3e}", worst_pitch));
    c.check(worst_axis <= 0.5 / 127.0 + 1e-12, || format!("axes: worst {:.3e}", worst_axis));
    c.check(angle_q(core::f64::consts::PI) == 0x8000 && angle_f(0x8000) == core::f64::consts::PI && angle_q(f64::NAN) == 0 && pos_q(1e300) == i32::MAX, || String::from("angle and position edge values"));
    // rotations: the smallest three
    let mut worst = 0.0f64;
    let mut sign_ok = true;
    for _ in 0..c.n(20_000) {
        let q = Quat::new(r.range(-1.0, 1.0), r.range(-1.0, 1.0), r.range(-1.0, 1.0), r.range(-1.0, 1.0)).normalize();
        let back = rot_f(rot_q(q));
        let dot = (q.x * back.x + q.y * back.y + q.z * back.z + q.w * back.w).abs().min(1.0);
        let angle = 2.0 * physics::math::atan2(physics::math::sqrt(1.0 - dot * dot), dot);
        worst = worst.max(angle);
        sign_ok &= rot_q(q) == rot_q(Quat::new(-q.x, -q.y, -q.z, -q.w));
    }
    c.check(worst < 0.1f64.to_radians() && sign_ok, || format!("rotations: worst {:.4} degrees, q and -q alike {}", worst.to_degrees(), sign_ok));
}

/// a random schema and snapshots for the delta tests
fn schema3() -> Schema {
    Schema::new().with(1, 13).with(2, 7).with(3, 1).with(4, MAX_FIELDS)
}

fn rand_state(r: &mut Rng, kind: u8, s: &Schema) -> NetState {
    let n = s.fields(kind);
    let mut f = [0i32; MAX_FIELDS];
    for x in f.iter_mut().take(n) {
        *x = match r.below(4) {
            0 => 0,
            1 => r.below(64) as i32 - 32,
            2 => r.below(1 << 16) as i32 - (1 << 15),
            _ => r.next_u64() as i32,
        };
    }
    NetState { kind, fields: f }
}

fn rand_snapshot(r: &mut Rng, s: &Schema, tick: u32, n: usize) -> Snapshot {
    let mut snap = Snapshot::new(tick);
    let mut id = r.below(5) as u16;
    for _ in 0..n {
        let kind = 1 + r.below(4) as u8;
        snap.ents.push(Ent { id, generation: r.below(3) as u8, state: rand_state(r, kind, s) });
        id = id.saturating_add(1 + if r.below(10) == 0 { r.below(3000) as u16 } else { r.below(3) as u16 });
        if id == u16::MAX {
            break;
        }
    }
    snap
}

/// cur from base: some removed, some changed in a few fields, some with a new generation, some new
fn evolve(r: &mut Rng, s: &Schema, base: &Snapshot) -> Snapshot {
    let mut cur = Snapshot::new(base.tick.wrapping_add(2));
    for e in &base.ents {
        match r.below(10) {
            0 => {}
            1 => cur.ents.push(Ent { id: e.id, generation: e.generation.wrapping_add(1), state: rand_state(r, e.state.kind, s) }),
            2..=4 => {
                let mut x = *e;
                let n = s.fields(x.state.kind);
                for _ in 0..1 + r.below(3) {
                    let i = r.below(n);
                    x.state.fields[i] = x.state.fields[i].wrapping_add(r.below(2000) as i32 - 1000);
                }
                cur.ents.push(x);
            }
            _ => cur.ents.push(*e),
        }
    }
    for _ in 0..r.below(6) {
        let id = r.below(70_000).min(65_535) as u16;
        if cur.get(id).is_none() {
            let kind = 1 + r.below(4) as u8;
            cur.put(Ent { id, generation: 0, state: rand_state(r, kind, s) });
        }
    }
    cur
}

/// the reference: a plain full encoding (count, then id, generation, kind and every field in whole bytes)
fn ref_full(s: &Schema, snap: &Snapshot) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(snap.ents.len() as u32).to_le_bytes());
    for e in &snap.ents {
        v.extend_from_slice(&e.id.to_le_bytes());
        v.push(e.generation);
        v.push(e.state.kind);
        for i in 0..s.fields(e.state.kind) {
            v.extend_from_slice(&e.state.fields[i].to_le_bytes());
        }
    }
    v
}

fn ref_read(s: &Schema, b: &[u8], tick: u32) -> Snapshot {
    let mut snap = Snapshot::new(tick);
    let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let mut p = 4;
    for _ in 0..n {
        let id = u16::from_le_bytes([b[p], b[p + 1]]);
        let generation = b[p + 2];
        let kind = b[p + 3];
        p += 4;
        let mut f = [0i32; MAX_FIELDS];
        for x in f.iter_mut().take(s.fields(kind)) {
            *x = i32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]);
            p += 4;
        }
        snap.ents.push(Ent { id, generation, state: NetState { kind, fields: f } });
    }
    snap
}

fn delta(c: &mut Ctx) {
    let s = schema3();
    let mut r = Rng::new(3);
    let n = c.n(4000);
    let (mut bad, mut size_bad, mut smaller, mut total_delta, mut total_full) = (0, 0, 0usize, 0usize, 0usize);
    let mut ch: Vec<Change> = Vec::new();
    for k in 0..n {
        let cnt = r.below(120);
        let base = rand_snapshot(&mut r, &s, 1000 + k as u32, cnt);
        let cur = evolve(&mut r, &s, &base);
        let baseline = if k % 3 == 0 { Some(rand_snapshot(&mut r, &s, 0, 60)) } else { None };
        let bl = baseline.as_ref();
        // the delta against the base
        diff(&s, Some(&base), &cur, bl, &mut ch);
        let mut w = BitWriter::new();
        encode(&mut w, &s, Some(&base), &cur, bl, &ch);
        if w.bit_len() != encoded_bits(&ch) {
            size_bad += 1;
        }
        let bytes = w.finish();
        let got = decode(&mut BitReader::new(&bytes), &s, Some(&base), bl, cur.tick);
        // against the reference full encoding
        let reference = ref_read(&s, &ref_full(&s, &cur), cur.tick);
        if got.as_ref() != Some(&reference) || reference != cur {
            bad += 1;
        }
        // the full encoding of the same (against nothing) decodes to the same
        diff(&s, None, &cur, bl, &mut ch);
        let mut wf = BitWriter::new();
        encode(&mut wf, &s, None, &cur, bl, &ch);
        let full = wf.finish();
        let got_full = decode(&mut BitReader::new(&full), &s, None, bl, cur.tick);
        if got_full.as_ref() != Some(&reference) {
            bad += 1;
        }
        total_delta += bytes.len();
        total_full += full.len();
        if bytes.len() <= full.len() {
            smaller += 1;
        }
    }
    c.check(bad == 0, || format!("delta: {} of {} snapshots decoded differently from the reference", bad, n));
    c.check(size_bad == 0, || format!("delta: {} sizes measured wrongly", size_bad));
    c.check(smaller * 10 >= n * 9, || format!("delta smaller than full in {} of {}", smaller, n));
    crate::out(&format!("  delta: {} snapshot pairs, delta {} bytes, full {} bytes on average\n", n, total_delta / n.max(1), total_full / n.max(1)));
}

fn budget(c: &mut Ctx) {
    let s = schema3();
    let mut r = Rng::new(4);
    let n = c.n(3000);
    let (mut over, mut bad, mut mixed, mut complete_wrong, mut order_wrong) = (0, 0, 0, 0, 0);
    let mut ch: Vec<Change> = Vec::new();
    for k in 0..n {
        let cnt = 30 + r.below(200);
        let base = rand_snapshot(&mut r, &s, 50 + k as u32, cnt);
        let cur = evolve(&mut r, &s, &base);
        diff(&s, Some(&base), &cur, None, &mut ch);
        let all = encoded_bits(&ch);
        let budget = r.below(all + 64).max(5);
        // the most important: by a random priority
        let mut order: Vec<usize> = (0..ch.len()).collect();
        let pr: Vec<u64> = (0..ch.len()).map(|_| r.next_u64() % 100).collect();
        order.sort_by_key(|&i| (core::cmp::Reverse(pr[i]), i));
        let chosen = select(&ch, &order, budget);
        if encoded_bits(&chosen) > budget && !chosen.is_empty() {
            over += 1;
        }
        // in id order
        if !chosen.windows(2).all(|w| w[0].id < w[1].id) {
            order_wrong += 1;
        }
        let mut w = BitWriter::new();
        encode(&mut w, &s, Some(&base), &cur, None, &chosen);
        let bytes = w.finish();
        let as_sent = apply(Some(&base), &cur, &chosen);
        let got = decode(&mut BitReader::new(&bytes), &s, Some(&base), None, cur.tick);
        if got.as_ref() != Some(&as_sent) {
            bad += 1;
        }
        // every entity of the as-sent snapshot is the base's or the current one's
        for e in &as_sent.ents {
            if base.get(e.id) != Some(e) && cur.get(e.id) != Some(e) {
                mixed += 1;
            }
        }
        if (chosen.len() == ch.len()) != (as_sent == Snapshot { tick: cur.tick, ents: cur.ents.clone() }) {
            complete_wrong += 1;
        }
        // nothing left out that would still have fitted at the end of the greedy pass is checked by size only
    }
    c.check(over == 0, || format!("budget: {} selections over their budget", over));
    c.check(bad == 0, || format!("budget: {} cut snapshots decoded differently from base + chosen", bad));
    c.check(mixed == 0, || format!("budget: {} entities neither the base's nor the current one's", mixed));
    c.check(complete_wrong == 0 && order_wrong == 0, || format!("budget: complete wrong {}, order wrong {}", complete_wrong, order_wrong));
    // the change kinds: an update, an add, a remove, a new generation
    let mut b = Snapshot::new(1);
    b.ents.push(Ent { id: 3, generation: 0, state: NetState::new(2, &[1, 2, 3, 0, 0, 0, 0]) });
    b.ents.push(Ent { id: 9, generation: 0, state: NetState::new(2, &[5; 7]) });
    b.ents.push(Ent { id: 12, generation: 1, state: NetState::new(3, &[7]) });
    let mut cur = Snapshot::new(3);
    cur.ents.push(Ent { id: 3, generation: 0, state: NetState::new(2, &[1, 2, 4, 0, 0, 0, 0]) });
    cur.ents.push(Ent { id: 12, generation: 2, state: NetState::new(3, &[7]) });
    cur.ents.push(Ent { id: 20, generation: 0, state: NetState::new(1, &[9; 13]) });
    diff(&s, Some(&b), &cur, None, &mut ch);
    let ops: Vec<(u16, Op)> = ch.iter().map(|x| (x.id, x.op)).collect();
    c.check(ops == [(3, Op::Update), (9, Op::Remove), (12, Op::Add), (20, Op::Add)], || format!("diff ops: {:?}", ops));
}

fn malformed(c: &mut Ctx) {
    let s = schema3();
    let mut r = Rng::new(5);
    let n = c.n(60_000);
    let mut invalid = 0;
    let mut ch: Vec<Change> = Vec::new();
    let mut decoded = 0;
    for k in 0..n {
        let cnt = r.below(40);
        let base = rand_snapshot(&mut r, &s, 7, cnt);
        let bytes: Vec<u8> = if k % 2 == 0 {
            (0..r.below(200)).map(|_| r.next_u64() as u8).collect()
        } else {
            let cur = evolve(&mut r, &s, &base);
            diff(&s, Some(&base), &cur, None, &mut ch);
            let mut w = BitWriter::new();
            encode(&mut w, &s, Some(&base), &cur, None, &ch);
            let mut b = w.finish();
            for _ in 0..1 + r.below(4) {
                if b.is_empty() {
                    break;
                }
                match r.below(3) {
                    0 => {
                        let i = r.below(b.len());
                        b[i] ^= 1 << r.below(8);
                    }
                    1 => b.truncate(r.below(b.len())),
                    _ => b.push(r.next_u64() as u8),
                }
            }
            b
        };
        if let Some(got) = decode(&mut BitReader::new(&bytes), &s, Some(&base), None, 9) {
            decoded += 1;
            let sorted = got.ents.windows(2).all(|w| w[0].id < w[1].id);
            let kinds = got.ents.iter().all(|e| s.fields(e.state.kind) > 0 && e.state.fields[s.fields(e.state.kind)..].iter().all(|&f| f == 0));
            if !sorted || !kinds {
                invalid += 1;
            }
        }
        // the messages' readers on the same bytes
        let mut m = bytes.clone();
        m.insert(0, 1 + (k % 7) as u8);
        let _ = proto::read_hello(&m);
        let _ = proto::read_welcome(&m);
        let _ = proto::read_input(&m);
        let _ = proto::read_snap_header(&m);
        let _ = proto::read_fetch(&m);
        let _ = proto::read_refuse(&m);
    }
    c.check(invalid == 0, || format!("malformed: {} decoded snapshots unsorted or with a bad kind ({} decoded)", invalid, decoded));
    crate::out(&format!("  malformed: {} byte strings, {} still decoded (valid), none panicked\n", n, decoded));
}

fn rand_input(r: &mut Rng, prev: &NetInput) -> NetInput {
    let mut i = *prev;
    if r.below(3) == 0 {
        i.buttons = r.next_u64() as u16;
    }
    if r.below(3) == 0 {
        i.move_x = r.next_u64() as i8;
        i.move_z = r.next_u64() as i8;
    }
    if r.below(2) == 0 {
        i.yaw = i.yaw.wrapping_add(r.below(2000) as u16);
        i.pitch = r.next_u64() as i16;
    }
    if r.below(2) == 0 {
        i.view = r.next_u64() as u16;
    }
    i
}

fn inputs(c: &mut Ctx) {
    let mut r = Rng::new(6);
    let n = c.n(20_000);
    let mut bad = 0;
    let mut prev = NetInput::default();
    for k in 0..n {
        let len = 1 + r.below(16);
        let mut v = Vec::new();
        for _ in 0..len {
            prev = rand_input(&mut r, &prev);
            v.push(prev);
            if r.below(2) == 0 {
                v.push(prev);
            }
        }
        v.truncate(16);
        let newest = (r.next_u64() as u32).wrapping_add(k as u32);
        let mut w = BitWriter::new();
        write_inputs(&mut w, newest, &v);
        let b = w.finish();
        match read_inputs(&mut BitReader::new(&b)) {
            Some((t, got)) if t == newest && got == v => {}
            _ => bad += 1,
        }
    }
    c.check(bad == 0, || format!("inputs: {} of {} packets read differently", bad, n));
    // redundancy is cheap: 16 inputs, the same but for yaw, in few bytes
    let mut v = Vec::new();
    let mut i = NetInput { move_z: 127, yaw: 1000, ..NetInput::default() };
    for _ in 0..16 {
        v.push(i);
        i.yaw = i.yaw.wrapping_sub(3);
    }
    let mut w = BitWriter::new();
    write_inputs(&mut w, 5, &v);
    let len = w.finish().len();
    c.check(len <= 48, || format!("16 inputs that differ in yaw: {} bytes", len));
    crate::out(&format!("  inputs: 16 redundant inputs (yaw turning) in {} bytes, one alone in 15\n", len));
}

fn messages(c: &mut Ctx) {
    let code = RoomCode::parse("k7m2qx").unwrap_or(RoomCode(*b"AAAAAA"));
    let h = Hello { version: proto::VERSION, room: Some(code), name: String::from("Jörg Müller") };
    c.check(proto::read_hello(&proto::write_hello(&h)) == Some(h.clone()), || String::from("HELLO round trip"));
    // a name cut inside a character loses that character
    let long: String = core::iter::repeat_n('ü', 40).collect();
    let h2 = Hello { version: 1, room: None, name: long };
    let got = proto::read_hello(&proto::write_hello(&h2)).map(|x| x.name.chars().count());
    c.check(got == Some(32), || format!("HELLO with a long name: {:?} characters", got));
    let w = Welcome { room: code, player: 7, rate: 60, snapshot_every: 2, tick: u32::MAX - 3, margin: 2, max_rewind: 12, fetch: true, ticket: 0x1234_5678_9abc_def0, ws_port: 8080, full_bytes: 5000 };
    c.check(proto::read_welcome(&proto::write_welcome(&w)) == Some(w.clone()), || String::from("WELCOME round trip"));
    c.check(proto::read_refuse(&proto::write_refuse(proto::REFUSE_ROOM_FULL)) == Some(proto::REFUSE_ROOM_FULL), || String::from("REFUSE"));
    let m = InputMsg { newest: 17, inputs: alloc::vec![NetInput { buttons: 5, ..NetInput::default() }; 3], ack: Some(u32::MAX), full: None };
    c.check(proto::read_input(&proto::write_input(&m)) == Some(m.clone()), || String::from("INPUT round trip"));
    c.check(proto::read_fetch(&proto::write_fetch(99)) == Some(99), || String::from("FETCH"));
    for base in [Base::None, Base::Map, Base::Back(1), Base::Back(255)] {
        for margin in [None, Some(-128i8), Some(0), Some(127)] {
            let h = SnapHeader { tick: 0xffff_fff0, base, player: 3, have: 0xffff_ffee, margin };
            let b = proto::write_snap_header(proto::MSG_SNAPSHOT, &h).finish();
            let got = proto::read_snap_header(&b).map(|x| (x.0, x.1));
            c.check(got == Some((proto::MSG_SNAPSHOT, h)), || format!("snapshot header {:?}: {:?}", h, got));
        }
    }
}

fn buffer(c: &mut Ctx) {
    let inp = |b: u16| NetInput { buttons: b, ..NetInput::default() };
    // across the wrap of u32
    let start = u32::MAX - 5;
    let mut buf = InputBuffer::new(start);
    let newest = start.wrapping_add(3);
    buf.put_packet(newest, &[inp(4), inp(3), inp(2), inp(1)]);
    c.check(buf.have() == newest, || format!("have after 4 inputs: {} (want {})", buf.have(), newest));
    c.check(buf.take_margin() == Some(3), || String::from("margin of the first packet"));
    let mut ok = true;
    for k in 0..4u16 {
        let (i, own) = buf.take(start.wrapping_add(k as u32));
        ok &= own && i.buttons == k + 1;
    }
    c.check(ok, || String::from("take hands each tick its input across the wrap"));
    // a missed tick repeats the last; a late input is refused; one too far ahead too
    let (i, own) = buf.take(start.wrapping_add(4));
    c.check(!own && i.buttons == 4, || format!("missed: {:?} own {}", i, own));
    c.check(!buf.put(start.wrapping_add(4), inp(9)), || String::from("a late input is refused"));
    c.check(!buf.put(start.wrapping_add(5 + 64), inp(9)), || String::from("an input 64 ticks ahead is refused"));
    c.check(buf.put(start.wrapping_add(7), inp(7)) && buf.have() == start.wrapping_add(4), || format!("a gap: have {}", buf.have()));
    buf.put(start.wrapping_add(5), inp(5));
    buf.put(start.wrapping_add(6), inp(6));
    c.check(buf.have() == start.wrapping_add(7), || String::from("the gap filled"));
    let st = buf.stats();
    c.check(st.late == 1 && st.early == 1 && st.missed == 1 && st.received == 7, || format!("stats {:?}", st));
    // a jump far ahead (the server was behind) forgets the old slots
    let (_, own) = buf.take(start.wrapping_add(1000));
    c.check(!own && buf.next() == start.wrapping_add(1001), || String::from("a far jump"));
}

/// a movement for the predictor's test: integers, a wall at 100
#[derive(Clone, Copy, Default)]
struct Toy;

impl Movement for Toy {
    type State = [i32; 2];
    const FIELDS: usize = 2;
    fn step(&self, _: &physics::World, s: &[i32; 2], i: &NetInput, _: f64) -> [i32; 2] {
        let x = (s[0] + i.move_x as i32).min(100);
        [x, s[1] + (i.buttons as i32 & 1)]
    }
    fn to_fields(s: &[i32; 2], out: &mut [i32]) {
        out[..2].copy_from_slice(s);
    }
    fn from_fields(f: &[i32]) -> [i32; 2] {
        [f[0], f[1]]
    }
    fn position(s: &[i32; 2]) -> Vec3 {
        Vec3::new(s[0] as f64, 0.0, 0.0)
    }
}

fn predictor(c: &mut Ctx) {
    let map = physics::World::new();
    let mut p: Predictor<Toy> = Predictor::new();
    let mut r = Rng::new(7);
    let inputs: Vec<NetInput> = (0..200).map(|_| NetInput { move_x: r.below(7) as i8 - 3, buttons: r.below(2) as u16, ..NetInput::default() }).collect();
    // before the first state: no prediction; the first state replays what came after it
    for (k, i) in inputs.iter().enumerate().take(10) {
        p.step(&Toy, &map, 100 + k as u32, *i, 0.0);
    }
    let o = p.reconcile(&Toy, &map, 103, [0, 0], 0.0);
    let mut want = [0, 0];
    for i in &inputs[4..10] {
        want = Toy.step(&map, &want, i, 0.0);
    }
    c.check(o == Outcome::Reset && p.current() == Some(want), || format!("first state: {:?} {:?} want {:?}", o, p.current(), want));
    // predictions that are right only drop history
    for (k, i) in inputs.iter().enumerate().skip(10).take(40) {
        p.step(&Toy, &map, 100 + k as u32, *i, 0.0);
    }
    let truth = |from: [i32; 2], a: usize, b: usize| inputs[a..b].iter().fold(from, |s, i| Toy.step(&map, &s, i, 0.0));
    let s110 = truth([0, 0], 4, 11);
    let o = p.reconcile(&Toy, &map, 110, s110, 0.0);
    c.check(o == Outcome::Match && p.pending() == 39, || format!("exact prediction: {:?}, pending {}", o, p.pending()));
    // a server state that differs: back to it, the inputs since then again
    let wrong = [s110[0] + 5, s110[1]];
    let s120_server = truth(wrong, 11, 21);
    let o = p.reconcile(&Toy, &map, 120, s120_server, 0.0);
    let now_want = truth(s120_server, 21, 50);
    c.check(matches!(o, Outcome::Corrected { .. }) && p.current() == Some(now_want), || format!("reconcile: {:?} {:?} want {:?}", o, p.current(), now_want));
    let o = p.reconcile(&Toy, &map, 115, s120_server, 0.0);
    c.check(o == Outcome::Stale, || format!("an older server state: {:?}", o));
    let st = p.stats();
    c.check(st.mismatches == 1 && st.resets == 1 && st.checks == 3, || format!("predictor stats {:?} {:?} {:?}", st.mismatches, st.resets, st.checks));
}

fn clocks(c: &mut Ctx) {
    // the prediction's clock: nominal at the target, faster when inputs come late, slower when early
    let mut k = SteeredClock::new(60, 2.0);
    k.set_tick(1000);
    k.advance(0);
    let mut steps = 0;
    for ms in 1..=1000u64 {
        steps += k.advance(ms * 1_000_000);
    }
    c.check(steps == 60, || format!("one second at 60 Hz: {} ticks", steps));
    k.on_margin(2);
    c.check(k.speed_ppm() == 1_000_000, || format!("at the target: {}", k.speed_ppm()));
    let mut k2 = SteeredClock::new(60, 2.0);
    k2.on_margin(0);
    let fast = k2.speed_ppm();
    let mut k3 = SteeredClock::new(60, 2.0);
    k3.on_margin(5);
    let slow = k3.speed_ppm();
    c.check(fast > 1_000_000 && slow < 1_000_000 && fast <= 1_100_000 && slow >= 900_000, || format!("steering: late {} early {}", fast, slow));
    // far too late: a jump of ticks at once; far too early: it holds
    let mut k4 = SteeredClock::new(60, 2.0);
    k4.advance(0);
    k4.on_margin(-20);
    let n = k4.advance(1);
    c.check(n == 23 && k4.jumps() == 1, || format!("jump: {} ticks", n));
    let mut k5 = SteeredClock::new(60, 2.0);
    k5.advance(0);
    k5.on_margin(60);
    let mut held = 0;
    for ms in 1..=1000u64 {
        held += k5.advance(ms * 1_000_000);
    }
    c.check(held == 60 - 56, || format!("hold: {} ticks in a second", held));
    // the interpolation clock: delay behind the newest, follows, holds, survives the wrap
    let mut ic = InterpClock::new(60, 6.0);
    ic.advance(0);
    let t0 = u32::MAX - 30;
    ic.on_snapshot(t0);
    c.check(ic.time() == Some((t0.wrapping_sub(6), 0.0)), || format!("first snapshot: {:?}", ic.time()));
    let mut ok = true;
    for k in 1..=60u32 {
        ic.advance(k as u64 * 16_666_667);
        if k % 2 == 0 {
            ic.on_snapshot(t0.wrapping_add(k));
        }
        ok &= ic.behind() > 4.0 && ic.behind() < 8.5;
    }
    let (a, _) = ic.time().unwrap_or((0, 0.0));
    c.check(ok && tick_diff(a, t0) > 50, || format!("follows across the wrap: behind {:.2}, at {}", ic.behind(), tick_diff(a, t0)));
    // starved: it holds at the newest
    for k in 61..200u32 {
        ic.advance(k as u64 * 16_666_667);
    }
    c.check(ic.behind().abs() < 1e-9, || format!("starved: behind {:.3}", ic.behind()));
    let _ = snapshot::MAX_ENTITIES;
}

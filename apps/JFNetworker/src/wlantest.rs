// wlantest -- checks of WLAN (group wlan of --selftest; jfn_model::wlan, jfn_sim::wlan, wlankey,
// wlantrace, cli/wlan.rs, doc/wlan.rs): the radio facts (signal, rate, loss, channels, ranges),
// the keys of WPA2 and WPA3 against independently computed values, scanning and the choice of an
// access point and band, open, WPA2 and WPA3 networks with exact virtual times of every step and
// their real EAPOL frames checked from outside, a wrong passphrase with its repetitions, back-off
// and giving up, DHCP and ping over WLAN, roaming while a ping runs, the edge of the range and
// leaving it, an access point switched off, reconfigured or with its radio shut down, client
// isolation and guest networks (also on the home router), the radio cables of older projects
// (fixed associations, as before), the command lines, the problems, the file format, undo, the
// texts in German and English, determinism
//
// Times: a laptop scans 38 channels (2.4 GHz 13, 5 GHz 19, 6 GHz 6) for 20 ms each, 760 ms;
// open system authentication and the association take two management frames of 250 µs each,
// so a station is associated 761 ms after it starts (SAE: 11 ms instead of 0.5 ms for the
// authentication).  At 3 m from a JF-AP6E on 5 GHz the signal is -50 dBm (MCS 11): 2.4 Gbit/s of
// PHY rate, 1.6 Gbit/s on the link, 200 µs to get the medium plus 10 ns of flight.  An EAPOL-Key
// frame of 113 bytes (messages 1 and 4) then takes 685 ns on the air, message 2 (135 bytes) 795
// ns, message 3 (169 bytes) 965 ns; an ARP frame 420 ns, a 98-byte echo 610 ns.  Copper: 1 Gbit/s
// with 50 ns of cable (ARP 722 ns, an echo 1026 ns).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::catalog;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_model::scenario::{self, StepKind};
use jfn_model::wlan::{self as mw, Band, Role, Security, Wlan, WlanJoin, WlanNet};
use jfn_sim::wlan::{self as sw, ASSOC_ID0};
use jfn_sim::wlankey::{self, Ptk};
use jfn_sim::{Action, Engine, JobKind, Led, Note, PingEv, PingOpts, ProblemKind, Report, Severity, StaState, WNote, MS, SEC, US};
use netpkt::decode;
use netpkt::eapol::{self, EapolView, Key};
use netpkt::{Ipv4, Mac};

use crate::doc::Doc;
use crate::selftest::T;

const BUDGET: usize = 20_000_000;
const PASS: &str = "Sommer2026!";
/// a laptop is associated this long after it starts looking
const JOINED: u64 = 761 * MS;

pub(crate) fn run(t: &mut T) {
    radio(t);
    keys(t);
    open(t);
    wpa2(t);
    wrong(t);
    wpa3(t);
    choice(t);
    dhcp_ping(t);
    roaming(t);
    range(t);
    ap_changes(t);
    isolation(t);
    home(t);
    fixed(t);
    shells(t);
    problems(t);
    model(t);
    undo(t);
    texts(t);
    determinism(t);
}

// ---------------------------------------------------------------- helpers

fn s(x: &str) -> String {
    String::from(x)
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: s(port) }
}

fn ip(x: &str) -> Ipv4 {
    Ipv4::parse(x).unwrap_or(Ipv4::UNSPECIFIED)
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_to(e: &mut Engine, at: u64) {
    e.run_until(at, BUDGET);
}

fn net(ssid: &str, band: Band, security: Security, pass: &str) -> WlanNet {
    WlanNet { ssid: s(ssid), band, channel: 0, security, passphrase: s(pass), ..WlanNet::default() }
}

/// a device of `model` at (x, y) with a startup configuration
fn add(p: &mut Project, model: &str, x: i32, y: i32, cfg: &str) -> u32 {
    let id = p.add_device(model, x, y).unwrap_or(0);
    if let Some(d) = p.device_mut(id) {
        d.startup_config = s(cfg);
    }
    id
}

fn offer(p: &mut Project, ap: u32, nets: Vec<WlanNet>) {
    if let Some(d) = p.device_mut(ap) {
        d.wlan = Some(Wlan { nets, join: None });
    }
}

fn join(p: &mut Project, sta: u32, ssid: &str, pass: &str) {
    if let Some(d) = p.device_mut(sta) {
        d.wlan = Some(Wlan { nets: Vec::new(), join: Some(WlanJoin { ssid: s(ssid), passphrase: s(pass) }) });
    }
}

fn joining(ssid: &str, pass: &str) -> Option<Wlan> {
    Some(Wlan { nets: Vec::new(), join: Some(WlanJoin { ssid: s(ssid), passphrase: s(pass) }) })
}

fn dhcp(p: &mut Project, dev: u32, iface: &str) {
    if let Some(d) = p.device_mut(dev) {
        let config = agent_core::json::parse(format!("{{\"iface\": \"{}\"}}", iface).as_bytes()).unwrap_or(Value::Null);
        d.services.push(ServiceDef { kind: s("dhcp-client"), name: format!("dhcp-client@{}", iface), disabled: false, config });
    }
}

/// a JF-AP6E (AP1) at (0, 0) with `nets` and Laptop1 (10.0.0.2 on wlan0) at (x, y) that joins
/// `ssid`
fn pair_at(nets: Vec<WlanNet>, ssid: &str, pass: &str, x: i32, y: i32) -> (Project, u32, u32) {
    let mut p = Project::new("wlan", 5);
    let ap = add(&mut p, "jf-ap6e", 0, 0, "hostname AP1\n");
    let nb = add(&mut p, "laptop", x, y, "hostname Laptop1\nip addr add 10.0.0.2/24 dev wlan0\n");
    offer(&mut p, ap, nets);
    join(&mut p, nb, ssid, pass);
    (p, ap, nb)
}

/// the same 3 m away
fn pair(nets: Vec<WlanNet>, ssid: &str, pass: &str) -> (Project, u32, u32) {
    pair_at(nets, ssid, pass, 30, 0)
}

/// the WLAN decisions of a device with their times
fn wn(e: &Engine, dev: u32) -> Vec<(u64, WNote)> {
    e.trace
        .events()
        .filter(|x| x.dev == dev)
        .filter_map(|x| match &x.note {
            Note::Wlan(n) => Some((x.t, n.clone())),
            _ => None,
        })
        .collect()
}

/// ... from `t0` on
fn wn_from(e: &Engine, dev: u32, t0: u64) -> Vec<(u64, WNote)> {
    wn(e, dev).into_iter().filter(|x| x.0 >= t0).collect()
}

/// the times a decision of a device matches
fn times(e: &Engine, dev: u32, f: impl Fn(&WNote) -> bool) -> Vec<u64> {
    wn(e, dev).into_iter().filter(|x| f(&x.1)).map(|x| x.0).collect()
}

/// the first time a decision of a device matches
fn when(e: &Engine, dev: u32, f: impl Fn(&WNote) -> bool) -> Option<u64> {
    times(e, dev, f).first().copied()
}

/// the key body of an EAPOL frame
fn key_of(f: &[u8]) -> Option<Key> {
    EapolView::parse(f.get(14..)?).and_then(|v| Key::parse(v.body()))
}

/// the EAPOL PDU of a frame
fn pdu_of(f: &[u8]) -> Option<Vec<u8>> {
    EapolView::parse(f.get(14..)?).map(|v| v.pdu().to_vec())
}

fn mac_at(f: &[u8], off: usize) -> Mac {
    let mut m = [0u8; 6];
    if let Some(b) = f.get(off..off + 6) {
        m.copy_from_slice(b);
    }
    Mac(m)
}

/// the EAPOL frames of the recording: (sent, arrived, sending device, message)
fn eapol_hops(e: &Engine) -> Vec<(u64, u64, u32, u8)> {
    e.trace.hops().filter(|h| h.mask & decode::P_EAPOL != 0).map(|h| (h.t_tx, h.t_rx, h.from.0, key_of(&h.bytes).and_then(|k| k.message()).unwrap_or(0))).collect()
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

/// lines typed at a device now; its output
fn cli(e: &mut Engine, dev: u32, lines: &[&str]) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    let now = e.now();
    e.run_until(now, BUDGET);
    out(e, dev)
}

fn ping(e: &mut Engine, dev: u32, dst: &str, count: u32) -> u32 {
    e.ping(dev, ip(dst), PingOpts { count, ..PingOpts::default() }).unwrap_or(0)
}

/// what a ping job saw
fn ping_ev(e: &Engine, id: u32) -> Vec<PingEv> {
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.events.clone(),
        _ => Vec::new(),
    }
}

fn reply(seq: u16, from: &str, rtt: u64) -> PingEv {
    PingEv::Reply { seq, from: ip(from), bytes: 84, ttl: 64, rtt }
}

/// the IPv4 addresses of an interface
fn addrs(e: &Engine, dev: u32, iface: &str) -> Vec<String> {
    e.ifaces(dev).iter().filter(|f| f.name == iface).flat_map(|f| f.addrs.iter().map(|a| format!("{}", a))).collect()
}

/// the WLAN problems: (kind, severity, device, port, text), by device
fn wlan_problems(e: &Engine) -> Vec<(ProblemKind, Severity, u32, Option<u16>, String)> {
    let mut v: Vec<(ProblemKind, Severity, u32, Option<u16>, String)> = e
        .problems()
        .into_iter()
        .filter(|x| matches!(x.kind, ProblemKind::WlanNoNetwork | ProblemKind::WlanPassphrase | ProblemKind::WlanBadNet))
        .map(|x| (x.kind, x.severity, x.dev, x.port, x.text))
        .collect();
    v.sort_by(|a, b| (a.2, &a.4).cmp(&(b.2, &b.4)));
    v
}

// ---------------------------------------------------------------- the radio and the keys

fn radio(t: &mut T) {
    // -82 dBm at the edge, 30 dB per decade nearer; nearer than 1 m counts as 1 m
    t.eq("signal at 3 m", sw::signal10(35, 3000), -500);
    t.eq("signal at 10 m", sw::signal10(35, 10_000), -657);
    t.eq("signal at the edge", sw::signal10(35, 35_000), -820);
    t.eq("signal at 1 m and nearer", (sw::signal10(35, 1000), sw::signal10(35, 10)), (-357, -357));
    t.eq("signal beyond", sw::signal10(35, 60_000), -890);
    t.eq("dbm rounds halves up", (sw::dbm(-500), sw::dbm(-657), sw::dbm(-655), sw::dbm(-654), sw::dbm(-5)), (-50, -66, -65, -65, 0));
    t.eq("mcs", (sw::mcs_of(-500), sw::mcs_of(-657), sw::mcs_of(-820), sw::mcs_of(-821)), (Some((11, 1000)), Some((5, 480)), Some((0, 60)), None));
    t.eq("edge loss", (sw::edge_loss_ppm(-780), sw::edge_loss_ppm(-800), sw::edge_loss_ppm(-820), sw::edge_loss_ppm(-900)), (0, 10_000, 20_000, 20_000));
    t.eq("medium", (sw::air_ns(Band::G24), sw::air_ns(Band::G5), sw::air_ns(Band::G6)), (300 * US, 200 * US, 150 * US));
    let m = Mac([0x02, 0x4a, 0, 0, 1, 3]);
    t.eq("bssid", (sw::bssid(m, 0), sw::bssid(m, 1), sw::bssid(m, 2)), (m, Mac([0x12, 0x4a, 0, 0, 1, 3]), Mac([0x22, 0x4a, 0, 0, 1, 3])));
    if let (Some(ap), Some(home), Some(laptop), Some(sensor), Some(pc)) = (catalog::model("jf-ap6e"), catalog::model("home-router"), catalog::model("laptop"), catalog::model("iot-sensor"), catalog::model("pc")) {
        t.eq("roles", [mw::role(ap), mw::role(home), mw::role(laptop), mw::role(pc)], [Role::Ap, Role::Ap, Role::Station, Role::None]);
        t.eq("bands", (mw::ap_bands(ap), mw::ap_bands(home), mw::client_bands(laptop), mw::client_bands(sensor), mw::ap_bands(pc)), (7, 3, 7, 1, 0));
        t.eq("ranges", (mw::range_m(ap, Band::G24), mw::range_m(ap, Band::G5), mw::range_m(ap, Band::G6), mw::range_m(home, Band::G5)), (50, 35, 25, 30));
        t.eq("radio ports", (mw::radio_port(ap, Band::G24), mw::radio_port(ap, Band::G6), mw::radio_port(home, Band::G5), mw::radio_port(pc, Band::G5)), (Some(s("Dot11Radio0")), Some(s("Dot11Radio2")), Some(s("wlan0")), None));
        t.eq("station port", (mw::station_port(laptop), mw::station_port(ap)), (Some(s("wlan0")), None));
        let n = mw::new_net(ap, &[net("WLAN-1", Band::G5, Security::Open, "")]);
        t.eq("a new network", (n.ssid, n.band, n.security, mw::channel_of(&mw::new_net(ap, &[]))), (s("WLAN-2"), Band::G5, Security::Open, 36));
    } else {
        t.check("models", false);
    }
    t.eq("channels", (mw::channels(Band::G24).len(), mw::channels(Band::G5).len(), mw::channels(Band::G6).len()), (13, 19, 24));
    t.eq("frequencies", (mw::freq_mhz(Band::G24, 1), mw::freq_mhz(Band::G24, 13), mw::freq_mhz(Band::G5, 36), mw::freq_mhz(Band::G6, 37)), (2412, 2472, 5180, 6135));
    t.eq("default channels", Band::ALL.iter().map(|&b| mw::default_channel(b)).collect::<Vec<_>>(), vec![1, 36, 37]);
    t.eq("a laptop scans 760 ms", Band::ALL.iter().map(|&b| mw::scan_channels(b)).sum::<usize>() as u64 * sw::DWELL, 760 * MS);
    t.eq("fastest rates", (mw::band_max_kbps(Band::G24), mw::band_max_kbps(Band::G6)), (573_500, 2_401_900));
    t.eq("passphrases", [mw::passphrase_ok("1234567"), mw::passphrase_ok("12345678"), mw::passphrase_ok(&"x".repeat(63)), mw::passphrase_ok(&"x".repeat(64)), mw::passphrase_ok("Grüße2026")], [false, true, true, false, false]);
    let text = |m: Option<jfn_model::Msg>| m.map(|m| m.text());
    t.eq("no ssid", text(mw::check_net(&net("", Band::G5, Security::Open, ""))), Some(s("Eine SSID hat 1 bis 32 Bytes.")));
    t.eq("6 GHz takes WPA3 only", text(mw::check_net(&net("X", Band::G6, Security::Wpa2, PASS))), Some(s("Im 6-GHz-Band gibt es nur WPA3-Personal.")));
    t.eq("short passphrase", text(mw::check_net(&net("Buero", Band::G5, Security::Wpa2, "kurz"))), Some(s("Die Passphrase von \u{201e}Buero\u{201c} braucht 8 bis 63 Zeichen (ASCII).")));
    t.eq("no channel 14", text(mw::check_net(&WlanNet { channel: 14, ..net("X", Band::G24, Security::Open, "") })), Some(s("Kanal 14 gibt es im Band 2,4 GHz nicht.")));
    t.eq("no vlan 4095", text(mw::check_net(&WlanNet { vlan: 4095, ..net("X", Band::G24, Security::Open, "") })), Some(s("VLAN 4095 gibt es nicht (1 bis 4094).")));
    t.eq("a good network", (mw::check_net(&net("Buero", Band::G5, Security::Wpa2, PASS)), mw::check_net(&net("Labor", Band::G6, Security::Wpa3, PASS))), (None, None));
    t.eq("a bad join", text(mw::check_join(&WlanJoin { ssid: s("Buero"), passphrase: s("x") })), Some(s("Die Passphrase von \u{201e}Buero\u{201c} braucht 8 bis 63 Zeichen (ASCII).")));
    t.eq("an open join", mw::check_join(&WlanJoin { ssid: s("Cafe"), passphrase: String::new() }), None);
    t.eq("default scale", mw::dip_per_m(&Project::new("x", 1)), 10);
}

fn hex(b: &[u8]) -> String {
    crypto::hex(b)
}

fn keys(t: &mut T) {
    // the PRF of 12.7.1.2: the test vectors of IEEE 802.11 J.3.2
    t.eq("prf-192", hex(&wlankey::prf_sha1(&[0x0b; 20], b"prefix", b"Hi There", 192)), s("bcd4c650b30b9684951829e0d75f9d54b862175ed9f00606"));
    t.eq("prf-256", hex(&wlankey::prf_sha1(b"Jefe", b"prefix-2", b"what do ya want for nothing?", 256)), s("47c4908e30c947521ad20be9053450ecbea23d3aa604b77326d8b3825ff7475c"));
    t.eq(
        "prf-384",
        hex(&wlankey::prf_sha1(&[0xaa; 80], b"prefix-3", b"Test Using Larger Than Block-Size Key - Hash Key First", 384)),
        s("0ab6c33ccf70d0d736f4b04c8a7373255511abc5073713163bd0b8c9eeb7e1956fa066820a73ddee3f6d3bd407e0682a"),
    );
    t.eq(
        "prf-512",
        hex(&wlankey::prf_sha1(&[0x0b; 20], b"prefix-4", b"Hi There Again", 512)),
        s("248cfbc532ab38ffa483c8a2e40bf170eb542a2e0916d7bf6d97da2c4c5ca877736c53a65b03fa4b3745ce7613f6ad68e0e4a798b7cf691c96176fd634a59a49"),
    );
    // KDF-SHA256-384 of 12.7.1.6.2 (computed independently, Python's hmac)
    let ctx: Vec<u8> = (0..76u8).collect();
    t.eq(
        "kdf-sha256-384",
        hex(&wlankey::kdf_sha256(&[1; 32], b"Pairwise key expansion", &ctx, 384)),
        s("90a3366a08bea762c366893e1deb3ac0be9d3d558c9db333707404e500e6acacb24de8e0d8997740e19b9b2a804ecf7b"),
    );
    // the PMK of WPA2-Personal (J.4) and the keys of a handshake (computed independently)
    let pmk = wlankey::pmk_psk("ThisIsAPassword", "ThisIsASSID");
    t.eq("pmk", hex(&pmk), s("0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af"));
    let an: [u8; 32] = core::array::from_fn(|i| 0xe0 + i as u8);
    let sn: [u8; 32] = core::array::from_fn(|i| 0xc0 + i as u8);
    let (aa, spa) = (Mac([0xa0, 0xa1, 0xa1, 0xa3, 0xa4, 0xa5]), Mac([0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5]));
    let p = Ptk::derive(false, &pmk, aa, spa, &an, &sn);
    t.eq("ptk", (hex(&p.kck), hex(&p.kek), hex(&p.tk)), (s("e79498d36bd81b9b41819b48f3c97f96"), s("6e620972f3d263238df571d65fa2b8fa"), s("d503e8b5891f81c74422468fc2d4dd39")));
    t.eq("both sides get the same keys", Ptk::derive(false, &pmk, spa, aa, &sn, &an), p.clone());
    // a sealed message 2 checks out with these keys, not with those of another passphrase
    let m2 = wlankey::seal(&wlankey::m2(false, 1, &sn), Some(&p), false, 1);
    let other = Ptk::derive(false, &wlankey::pmk_psk("ThisIsAnother", "ThisIsASSID"), aa, spa, &an, &sn);
    t.check("mic", wlankey::mic_ok(&m2, &p, false) && !wlankey::mic_ok(&m2, &other, false));
    t.eq("mic of a fixed pdu (HMAC-SHA1-128)", {
        let mut z = vec![2u8, 3, 0, 117];
        z.extend_from_slice(&[0; 117]);
        hex(&crypto::hmac_sha1(&p.kck, &z)[..16])
    }, s("f5075482962375498df06937b8de5688"));
    // WPA3 (AKM 8): KDF-SHA256 and AES-128-CMAC
    let p3 = Ptk::derive(true, &pmk, aa, spa, &an, &sn);
    t.check("wpa3 keys differ", p3 != p);
    let m4 = wlankey::seal(&wlankey::m4(true, 2), Some(&p3), true, 1);
    t.check("cmac mic", wlankey::mic_ok(&m4, &p3, true) && !wlankey::mic_ok(&m4, &p3, false) && !wlankey::mic_ok(&m4, &p, true));
    // message 3's key data: wrapped under the KEK, the GTK comes out again
    let gtk = [0x33u8; 16];
    let w = wlankey::wrap(&p.kek, &wlankey::key_data(false, &gtk, &[0; 16]));
    t.eq("wrapped key data", (w.len(), wlankey::unwrap_gtk(&p.kek, &w), wlankey::unwrap_gtk(&other.kek, &w)), (56, Some(gtk), None));
    t.eq("wpa3 key data (with the IGTK)", wlankey::wrap(&p3.kek, &wlankey::key_data(true, &gtk, &[0x44; 16])).len(), 88);
    // SAE's stand-in: the same on both sides with one password, fresh with other random values
    let (ra, rb) = ([1u8; 32], [2u8; 32]);
    let sae = wlankey::pmk_sae("geheim2026", aa, spa, &ra, &rb);
    t.eq("sae pmk on both sides", wlankey::pmk_sae("geheim2026", spa, aa, &rb, &ra), sae);
    t.check("sae pmk differs", sae != wlankey::pmk_sae("geheim2026", aa, spa, &ra, &[3; 32]) && sae != wlankey::pmk_sae("geheim2027", aa, spa, &ra, &rb));
    t.eq("rsn elements", (wlankey::rsne(false).akm, wlankey::rsne(true).akm, wlankey::rsne(true).caps), (eapol::AKM_PSK, eapol::AKM_SAE, eapol::RSN_MFPR | eapol::RSN_MFPC));
}

// ---------------------------------------------------------------- joining

fn open(t: &mut T) {
    let (p, ap, nb) = pair(vec![net("Cafe", Band::G5, Security::Open, "")], "Cafe", "");
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    t.eq(
        "station steps",
        wn(&e, nb),
        vec![
            (0, WNote::ScanStart { ssid: s("Cafe"), bands: 7, ms: 760 }),
            (760 * MS, WNote::ScanFound { ssid: s("Cafe"), n: 1, ap: s("AP1"), band: Band::G5, ch: 36, dbm: -50 }),
            (760 * MS, WNote::AuthStart { ap: s("AP1"), sae: false, reassoc: false }),
            (JOINED, WNote::Assoc { ap: s("AP1"), aid: 1, band: Band::G5, ch: 36, dbm: -50, kbps: 2_400_000, reassoc: false }),
        ],
    );
    t.eq(
        "access point steps",
        wn(&e, ap),
        vec![
            (760 * MS, WNote::AuthBy { sta: s("Laptop1"), sae: false }),
            (JOINED, WNote::ApAssoc { sta: s("Laptop1"), aid: 1, ssid: s("Cafe"), port: 2, reassoc: false }),
            (JOINED, WNote::L2Update { sta: s("Laptop1") }),
        ],
    );
    t.eq("no handshake", eapol_hops(&e), Vec::new());
    t.eq("connected", e.wlan_station(nb).map(|v| (v.state, v.failures, v.join)), Some((StaState::Connected, 0, Some(WlanJoin { ssid: s("Cafe"), passphrase: String::new() }))));
    let a: Vec<_> = e.wlan_assocs().into_iter().map(|a| ((a.id, a.sta, a.sta_port, a.ap, a.ap_port, a.ssid), (a.channel, a.aid, a.vlan, a.dbm, a.kbps, a.authorized, a.lost, a.since))).collect();
    t.eq("the association", a, vec![((ASSOC_ID0, nb, 0, ap, 2, s("Cafe")), (36, 1, 1, -50, 2_400_000, true, false, JOINED))]);
    // a link of the engine: two thirds of the rate, 200 µs + 10 ns
    let lv = e.view().links.into_iter().find(|l| l.id == ASSOC_ID0).map(|l| (l.up, l.kbps, l.ends));
    t.eq("radio link", lv, Some((true, 1_600_000, [(nb, 0), (ap, 2)])));
    // the radio of the network sends without a cable, the 2.4 GHz one has nothing to send
    t.eq("leds", (e.port_led(ap, 2), e.port_led(ap, 1), e.port_led(nb, 0)), (Led::Up, Led::Down, Led::Up));
    t.eq("the stations of the AP", e.wlan_stations_of(ap).iter().map(|a| a.sta).collect::<Vec<_>>(), vec![nb]);
}

fn wpa2(t: &mut T) {
    let mut p = Project::new("wpa2", 5);
    let ap = add(&mut p, "jf-ap6e", 0, 0, "hostname AP1\n");
    let nb = add(&mut p, "laptop", 30, 0, "hostname Laptop1\nip addr add 10.0.0.2/24 dev wlan0\n");
    let pc = add(&mut p, "pc", 0, -300, "hostname PC1\nip addr add 10.0.0.1/24 dev eth0\n");
    t.check("cable", p.connect(&pr(pc, "eth0"), &pr(ap, "TwoGigabitEthernet0"), None).is_ok());
    offer(&mut p, ap, vec![net("Buero", Band::G5, Security::Wpa2, PASS)]);
    join(&mut p, nb, "Buero", PASS);
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    let at = |x: u64| JOINED + x;
    // 685 + 200010, 795 + 200010, 965 + 200010, 685 + 200010 ns
    t.eq("eapol frames", eapol_hops(&e), vec![(at(0), at(200_695), ap, 1), (at(200_695), at(401_500), nb, 2), (at(401_500), at(602_475), ap, 3), (at(602_475), at(803_170), nb, 4)]);
    t.eq(
        "handshake of the station",
        wn(&e, nb).into_iter().skip(4).collect::<Vec<_>>(),
        vec![
            (at(200_695), WNote::Eapol { msg: 1, out: false, peer: s("AP1") }),
            (at(200_695), WNote::Eapol { msg: 2, out: true, peer: s("AP1") }),
            (at(602_475), WNote::Eapol { msg: 3, out: false, peer: s("AP1") }),
            (at(602_475), WNote::Eapol { msg: 4, out: true, peer: s("AP1") }),
            (at(602_475), WNote::Authorized { peer: s("AP1") }),
        ],
    );
    t.eq(
        "handshake of the access point",
        wn(&e, ap).into_iter().skip(3).collect::<Vec<_>>(),
        vec![
            (at(0), WNote::Eapol { msg: 1, out: true, peer: s("Laptop1") }),
            (at(401_500), WNote::Eapol { msg: 2, out: false, peer: s("Laptop1") }),
            (at(401_500), WNote::Eapol { msg: 3, out: true, peer: s("Laptop1") }),
            (at(803_170), WNote::Eapol { msg: 4, out: false, peer: s("Laptop1") }),
            (at(803_170), WNote::Authorized { peer: s("Laptop1") }),
        ],
    );
    t.check("nothing dropped before the keys", !wn(&e, ap).iter().any(|x| matches!(x.1, WNote::NotAuthorized { .. })));
    // from outside: the PMK of the passphrase and the nonces of messages 1 and 2 give the keys;
    // they check the MICs of messages 2 to 4 and unwrap the GTK of message 3
    let frames: Vec<Vec<u8>> = e.trace.hops().filter(|h| h.mask & decode::P_EAPOL != 0).map(|h| h.bytes.to_vec()).collect();
    match frames.as_slice() {
        [f1, f2, f3, f4] => {
            t.eq("frame lengths", (f1.len(), f2.len(), f3.len(), f4.len()), (113, 135, 169, 113));
            let (aa, spa) = (mac_at(f1, 6), mac_at(f1, 0));
            t.eq("addresses", (aa, spa), (jfn_sim::node::port_mac(ap, 2), jfn_sim::node::port_mac(nb, 0)));
            match (key_of(f1), key_of(f2), key_of(f3)) {
                (Some(k1), Some(k2), Some(k3)) => {
                    let ptk = Ptk::derive(false, &wlankey::pmk_psk(PASS, "Buero"), aa, spa, &k1.nonce, &k2.nonce);
                    t.check("mics of messages 2 to 4", [f2, f3, f4].iter().all(|f| pdu_of(f).is_some_and(|x| wlankey::mic_ok(&x, &ptk, false))));
                    t.check("the gtk unwraps", wlankey::unwrap_gtk(&ptk.kek, &k3.data).is_some());
                    t.eq("replay counters", (k1.replay, k2.replay, k3.replay), (1, 1, 2));
                    t.eq("rsn of message 2", eapol::Rsne::parse(&k2.data), Some(eapol::Rsne { akm: eapol::AKM_PSK, caps: 0 }));
                    t.eq("descriptor version", (k1.version(), k3.version()), (eapol::KDV_SHA1, eapol::KDV_SHA1));
                }
                _ => t.check("key bodies", false),
            }
            t.eq("decoded", (decode::info(f1).1, decode::info(f3).1), (s("Key (Message 1 of 4)"), s("Key (Message 3 of 4)")));
        }
        _ => t.check("four eapol frames", false),
    }
    // data: ARP (2 x 200430 ns on the air, 2 x 318 ns on the cable: PC1 has 2.5 Gbit/s as the
    // AP's port), then the echo (2 x 200620 + 2 x 440 ns)
    let id = ping(&mut e, nb, "10.0.0.1", 2);
    run_to(&mut e, 5 * SEC);
    t.eq("ping over WLAN", ping_ev(&e, id), vec![reply(1, "10.0.0.1", 803_616), reply(2, "10.0.0.1", 402_120)]);
}

fn wrong(t: &mut T) {
    let (p, ap, nb) = pair(vec![net("Buero", Band::G5, Security::Wpa2, PASS)], "Buero", "Winter2026!");
    let mut e = engine(&p);
    run_to(&mut e, 120 * SEC);
    // the AP drops message 2 (its MIC is another), sends message 1 again after 100 ms, then every
    // second, four times in all, and then sends the station away (reason 15)
    let m2 = JOINED + 401_500;
    t.eq("bad mics", times(&e, ap, |n| matches!(n, WNote::MicBad { msg: 2, .. })).into_iter().take(4).collect::<Vec<_>>(), vec![m2, m2 + 100 * MS, m2 + 1100 * MS, m2 + 2100 * MS]);
    t.eq("repetitions", wn(&e, ap).into_iter().filter(|x| matches!(x.1, WNote::HsRetry { .. })).take(3).collect::<Vec<_>>(), vec![
        (861 * MS, WNote::HsRetry { msg: 1, sta: s("Laptop1"), n: 2 }),
        (1861 * MS, WNote::HsRetry { msg: 1, sta: s("Laptop1"), n: 3 }),
        (2861 * MS, WNote::HsRetry { msg: 1, sta: s("Laptop1"), n: 4 }),
    ]);
    t.eq("end of the handshake", (when(&e, ap, |n| matches!(n, WNote::HsTimeout { .. })), when(&e, nb, |n| matches!(n, WNote::Deauthed { reason: 15, .. }))), (Some(3861 * MS), Some(3861 * MS)));
    t.check("never authorized", !wn(&e, nb).iter().any(|x| matches!(x.1, WNote::Authorized { .. })));
    // it tries again after 10, 20 and 30 s and gives up after the fourth failure
    t.eq(
        "back-off",
        wn(&e, nb).into_iter().filter(|x| matches!(x.1, WNote::Backoff { .. } | WNote::GiveUp { .. })).collect::<Vec<_>>(),
        vec![
            (3861 * MS, WNote::Backoff { secs: 10, n: 1 }),
            (17_722 * MS, WNote::Backoff { secs: 20, n: 2 }),
            (41_583 * MS, WNote::Backoff { secs: 30, n: 3 }),
            (75_444 * MS, WNote::GiveUp { n: 4 }),
        ],
    );
    t.eq("scans", times(&e, nb, |n| matches!(n, WNote::ScanStart { .. })), vec![0, 13_861 * MS, 37_722 * MS, 71_583 * MS]);
    t.eq("given up", e.wlan_station(nb).map(|v| (v.state, v.failures, v.assoc.is_none())), Some((StaState::GaveUp, 4, true)));
    t.eq("the problem", wlan_problems(&e), vec![(ProblemKind::WlanPassphrase, Severity::Error, nb, Some(0), s("Die Passphrase von Laptop1 passt nicht zum WLAN \u{201e}Buero\u{201c}."))]);
    // the right passphrase: it starts again at once and gets in
    let t0 = e.now();
    t.check("new passphrase", e.apply(Action::Wlan { dev: nb, wlan: joining("Buero", PASS) }).is_ok());
    run_to(&mut e, t0 + 2 * SEC);
    t.eq("in now", (when(&e, nb, |n| matches!(n, WNote::Authorized { .. })), e.wlan_station(nb).map(|v| (v.state, v.failures))), (Some(t0 + JOINED + 602_475), Some((StaState::Connected, 0))));
    t.eq("no problem", wlan_problems(&e), Vec::new());
    // the station's log has the reason, as wpa_supplicant writes one
    t.check("log line", e.log_lines(nb).iter().any(|l| l.tag == "wpa_supplicant" && l.text.contains("the passphrase may be wrong")));
    // the other way round: a station that gave up tries again when an access point's network of
    // its SSID changes (here to the passphrase the station has)
    let (p, ap, nb) = pair(vec![net("Buero", Band::G5, Security::Wpa2, PASS)], "Buero", "Winter2026!");
    let mut e = engine(&p);
    run_to(&mut e, 80 * SEC);
    t.eq("gave up again", e.wlan_station(nb).map(|v| v.state), Some(StaState::GaveUp));
    let t0 = e.now();
    let _ = e.apply(Action::Wlan { dev: ap, wlan: Some(Wlan { nets: vec![net("Buero", Band::G5, Security::Wpa2, "Winter2026!")], join: None }) });
    run_to(&mut e, t0 + 2 * SEC);
    t.eq("the network changed: in", (when(&e, nb, |n| matches!(n, WNote::Authorized { .. })), e.wlan_station(nb).map(|v| (v.state, v.failures))), (Some(t0 + JOINED + 602_475), Some((StaState::Connected, 0))));
}

fn wpa3(t: &mut T) {
    let (p, ap, nb) = pair(vec![net("Labor", Band::G6, Security::Wpa3, PASS)], "Labor", PASS);
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    // SAE: commit and confirm both ways (four frames of 250 µs) and 10 ms of computing
    let j = 771_500 * US;
    t.eq(
        "sae",
        wn(&e, nb).into_iter().take(4).collect::<Vec<_>>(),
        vec![
            (0, WNote::ScanStart { ssid: s("Labor"), bands: 7, ms: 760 }),
            (760 * MS, WNote::ScanFound { ssid: s("Labor"), n: 1, ap: s("AP1"), band: Band::G6, ch: 37, dbm: -54 }),
            (760 * MS, WNote::AuthStart { ap: s("AP1"), sae: true, reassoc: false }),
            (j, WNote::Assoc { ap: s("AP1"), aid: 1, band: Band::G6, ch: 37, dbm: -54, kbps: 1_920_000, reassoc: false }),
        ],
    );
    // 6 GHz at 3 m: -54 dBm (MCS 9), 1.28 Gbit/s on the link, 150 µs + 10 ns; message 3 carries
    // the IGTK as well (201 bytes)
    t.eq("eapol frames", eapol_hops(&e), vec![(j, j + 150_866, ap, 1), (j + 150_866, j + 301_869, nb, 2), (j + 301_869, j + 453_285, ap, 3), (j + 453_285, j + 604_151, nb, 4)]);
    let frames: Vec<Vec<u8>> = e.trace.hops().filter(|h| h.mask & decode::P_EAPOL != 0).map(|h| h.bytes.to_vec()).collect();
    t.eq("frame lengths", frames.iter().map(|f| f.len()).collect::<Vec<_>>(), vec![113, 135, 201, 113]);
    let k2 = frames.get(1).and_then(|f| key_of(f));
    t.eq("akm 8", k2.map(|k| (k.version(), eapol::Rsne::parse(&k.data))), Some((eapol::KDV_AKM, Some(eapol::Rsne { akm: eapol::AKM_SAE, caps: eapol::RSN_MFPR | eapol::RSN_MFPC }))));
    t.eq("connected", e.wlan_station(nb).map(|v| v.state), Some(StaState::Connected));
    // a wrong password: the AP never confirms, the station gives up the authentication 1 s later
    let (p, ap, nb) = pair(vec![net("Labor", Band::G6, Security::Wpa3, PASS)], "Labor", "Falsch2026!");
    let mut e = engine(&p);
    run_to(&mut e, 5 * SEC);
    t.eq(
        "no confirm",
        wn(&e, nb).into_iter().skip(3).take(2).collect::<Vec<_>>(),
        vec![(1771 * MS, WNote::AuthFailed { ap: s("AP1") }), (1771 * MS, WNote::Backoff { secs: 10, n: 1 })],
    );
    t.eq("the AP sees why", when(&e, ap, |n| matches!(n, WNote::SaeMismatch { .. })), Some(1771 * MS));
    t.check("no association, no handshake", e.wlan_assocs().is_empty() && eapol_hops(&e).is_empty());
    t.eq("the problem", wlan_problems(&e).into_iter().map(|x| x.0).collect::<Vec<_>>(), vec![ProblemKind::WlanPassphrase]);
    // WPA2 on 6 GHz is not offered
    let (p, ap, nb) = pair(vec![net("Labor", Band::G6, Security::Wpa2, PASS)], "Labor", PASS);
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    t.eq("not offered", (e.wlan_station(nb).map(|v| v.state), e.port_led(ap, 3)), (Some(StaState::NoNetwork), Led::Down));
    t.eq(
        "its problems",
        wlan_problems(&e),
        vec![
            (ProblemKind::WlanBadNet, Severity::Error, ap, Some(3), s("Im 6-GHz-Band gibt es nur WPA3-Personal.")),
            (ProblemKind::WlanNoNetwork, Severity::Error, nb, Some(0), s("Kein Access Point bietet das WLAN \u{201e}Labor\u{201c} an, das Laptop1 sucht.")),
        ],
    );
}

fn choice(t: &mut T) {
    // one network on two bands: at 3 m 5 GHz is faster; at 40 m only 2.4 GHz reaches (-79 dBm, MCS 0)
    let dual = vec![net("Haus", Band::G24, Security::Open, ""), net("Haus", Band::G5, Security::Open, "")];
    let (p, _, nb) = pair(dual.clone(), "Haus", "");
    let mut e = engine(&p);
    run_to(&mut e, SEC);
    t.eq("near: 5 GHz", when(&e, nb, |n| matches!(n, WNote::ScanFound { n: 2, band: Band::G5, ch: 36, dbm: -50, .. })), Some(760 * MS));
    let (p, _, nb) = pair_at(dual, "Haus", "", 0, 400);
    let mut e = engine(&p);
    run_to(&mut e, SEC);
    t.eq("far: 2.4 GHz", wn(&e, nb).into_iter().filter(|x| matches!(x.1, WNote::ScanFound { .. } | WNote::Assoc { .. })).collect::<Vec<_>>(), vec![
        (760 * MS, WNote::ScanFound { ssid: s("Haus"), n: 1, ap: s("AP1"), band: Band::G24, ch: 1, dbm: -79 }),
        (JOINED, WNote::Assoc { ap: s("AP1"), aid: 1, band: Band::G24, ch: 1, dbm: -79, kbps: 34_410, reassoc: false }),
    ]);
    // two access points alike: the lower device id
    let mut p = Project::new("two", 5);
    let ap1 = add(&mut p, "jf-ap6e", 0, 0, "hostname AP1\n");
    let ap2 = add(&mut p, "jf-ap6e", 60, 0, "hostname AP2\n");
    let nb = add(&mut p, "laptop", 30, 0, "hostname Laptop1\n");
    offer(&mut p, ap1, vec![net("Buero", Band::G5, Security::Open, "")]);
    offer(&mut p, ap2, vec![WlanNet { channel: 44, ..net("Buero", Band::G5, Security::Open, "") }]);
    join(&mut p, nb, "Buero", "");
    let mut e = engine(&p);
    run_to(&mut e, SEC);
    t.eq("a tie: the first one", when(&e, nb, |n| matches!(n, WNote::ScanFound { n: 2, ap, ch: 36, .. } if ap == "AP1")), Some(760 * MS));
    // a nearer one wins
    let _ = e.apply(Action::Wlan { dev: nb, wlan: None });
    let _ = e.apply(Action::Move { dev: nb, x: 50, y: 0 });
    let t0 = e.now();
    let _ = e.apply(Action::Wlan { dev: nb, wlan: joining("Buero", "") });
    run_to(&mut e, t0 + SEC);
    t.eq("the nearer one", e.wlan_station(nb).and_then(|v| v.assoc).map(|a| (a.ap, a.channel, a.dbm)), Some((ap2, 44, -36)));
}

fn dhcp_ping(t: &mut T) {
    let mut p = Project::new("dhcp", 9);
    let r1 = add(
        &mut p,
        "jf-r4",
        0,
        -300,
        "hostname R1\n!\nip dhcp excluded-address 10.0.0.1 10.0.0.10\n!\nip dhcp pool WLAN\n network 10.0.0.0 255.255.255.0\n default-router 10.0.0.1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\nend\n",
    );
    let ap = add(&mut p, "jf-ap6e", 0, 0, "hostname AP1\n");
    let nb = add(&mut p, "laptop", 30, 0, "hostname Laptop1\n");
    t.check("cable", p.connect(&pr(r1, "Gi0/0/0"), &pr(ap, "TwoGigabitEthernet0"), None).is_ok());
    offer(&mut p, ap, vec![net("Buero", Band::G5, Security::Wpa2, PASS)]);
    join(&mut p, nb, "Buero", PASS);
    dhcp(&mut p, nb, "wlan0");
    let mut e = engine(&p);
    run_to(&mut e, 10 * SEC);
    // the client starts when the keys are in place (its link comes up), the router pings the
    // address twice before it offers it
    t.eq("lease", addrs(&e, nb, "wlan0"), vec![s("10.0.0.11/24")]);
    t.check("the router served it", e.dhcp_bindings(r1).iter().any(|b| b.ip == ip("10.0.0.11")));
    let id = ping(&mut e, nb, "10.0.0.1", 2);
    run_to(&mut e, 13 * SEC);
    let ev = ping_ev(&e, id);
    t.eq("ping the router", ev.get(1).cloned(), Some(PingEv::Reply { seq: 2, from: ip("10.0.0.1"), bytes: 84, ttl: 255, rtt: 403_292 }));
    t.check("first ping", matches!(ev.first(), Some(PingEv::Reply { seq: 1, .. })));
}

fn roaming(t: &mut T) {
    // two access points 40 m apart with one network; the laptop starts 6 m from AP1, walks to the
    // middle (20 m from both: -75 dBm each, not 8 dB better: it stays) and on to 7 m from AP2
    // (-81 dBm at AP1, -61 dBm at AP2: it roams) while it pings PC1 behind the switch every second
    let mut p = Project::new("roam", 21);
    let sw1 = add(&mut p, "jf-s24p", 200, -200, "hostname SW1\n");
    let ap1 = add(&mut p, "jf-ap6e", 0, 0, "hostname AP1\n");
    let ap2 = add(&mut p, "jf-ap6e", 400, 0, "hostname AP2\n");
    let pc = add(&mut p, "pc", 200, -400, "hostname PC1\nip addr add 10.0.0.1/24 dev eth0\n");
    let nb = add(&mut p, "laptop", 60, 0, "hostname Laptop1\nip addr add 10.0.0.2/24 dev wlan0\n");
    let ok = [
        p.connect(&pr(sw1, "gi1/0/1"), &pr(ap1, "TwoGigabitEthernet0"), None).is_ok(),
        p.connect(&pr(sw1, "gi1/0/2"), &pr(ap2, "TwoGigabitEthernet0"), None).is_ok(),
        p.connect(&pr(sw1, "gi1/0/3"), &pr(pc, "eth0"), None).is_ok(),
    ];
    t.check("cables", ok.iter().all(|x| *x));
    offer(&mut p, ap1, vec![net("Buero", Band::G5, Security::Wpa2, PASS)]);
    offer(&mut p, ap2, vec![WlanNet { channel: 44, ..net("Buero", Band::G5, Security::Wpa2, PASS) }]);
    join(&mut p, nb, "Buero", PASS);
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    t.eq("both in reach, AP1 is better", when(&e, nb, |n| matches!(n, WNote::ScanFound { n: 2, ap, dbm: -59, .. } if ap == "AP1")), Some(760 * MS));
    let id = ping(&mut e, nb, "10.0.0.1", 8);
    run_to(&mut e, 3500 * MS);
    t.check("halfway", e.apply(Action::Move { dev: nb, x: 200, y: 0 }).is_ok());
    run_to(&mut e, 5500 * MS);
    t.check("near AP2", e.apply(Action::Move { dev: nb, x: 330, y: 0 }).is_ok());
    run_to(&mut e, 12 * SEC);
    let r = 5501 * MS;
    t.eq(
        "the station",
        wn_from(&e, nb, 3 * SEC).into_iter().take(7).collect::<Vec<_>>(),
        vec![
            (3500 * MS, WNote::Rate { ap: s("AP1"), dbm: -75, kbps: 432_000 }),
            (5500 * MS, WNote::Rate { ap: s("AP1"), dbm: -81, kbps: 144_000 }),
            (5500 * MS, WNote::Roam { from: s("AP1"), to: s("AP2"), old: -81, new: -61 }),
            (5500 * MS, WNote::AuthStart { ap: s("AP2"), sae: false, reassoc: true }),
            (r, WNote::Assoc { ap: s("AP2"), aid: 1, band: Band::G5, ch: 44, dbm: -61, kbps: 1_440_000, reassoc: true }),
            (r + 201_164, WNote::Eapol { msg: 1, out: false, peer: s("AP2") }),
            (r + 201_164, WNote::Eapol { msg: 2, out: true, peer: s("AP2") }),
        ],
    );
    t.eq("authorized at AP2", when(&e, ap2, |n| matches!(n, WNote::Authorized { .. })), Some(r + 805_307));
    t.eq("AP1 lets it go", wn_from(&e, ap1, 3 * SEC), vec![(r, WNote::RoamedAway { sta: s("Laptop1"), to: s("AP2") })]);
    t.eq(
        "AP2 takes it and tells the switch",
        wn(&e, ap2).into_iter().take(3).collect::<Vec<_>>(),
        vec![(5500 * MS, WNote::AuthBy { sta: s("Laptop1"), sae: false }), (r, WNote::ApAssoc { sta: s("Laptop1"), aid: 1, ssid: s("Buero"), port: 2, reassoc: true }), (r, WNote::L2Update { sta: s("Laptop1") })],
    );
    t.eq("one association, on AP2", e.wlan_assocs().into_iter().map(|a| (a.ap, a.authorized)).collect::<Vec<_>>(), vec![(ap2, true)]);
    // every echo comes back: 6 m (201036 ns on the air each way), 20 m (203454), 7 m via AP2 (201039)
    t.eq(
        "the ping",
        ping_ev(&e, id),
        vec![
            reply(1, "10.0.0.1", 810_504),
            reply(2, "10.0.0.1", 406_176),
            reply(3, "10.0.0.1", 411_012),
            reply(4, "10.0.0.1", 411_012),
            reply(5, "10.0.0.1", 406_182),
            reply(6, "10.0.0.1", 406_182),
            reply(7, "10.0.0.1", 406_182),
            reply(8, "10.0.0.1", 406_182),
        ],
    );
}

fn range(t: &mut T) {
    // exactly at the edge of 5 GHz (35 m): -82 dBm, MCS 0
    let cafe = vec![net("Cafe", Band::G5, Security::Open, "")];
    let (p, _, nb) = pair_at(cafe.clone(), "Cafe", "", 0, 350);
    let mut e = engine(&p);
    run_to(&mut e, SEC);
    t.eq("at the edge", e.wlan_station(nb).and_then(|v| v.assoc).map(|a| (a.dbm, a.kbps)), Some((-82, 144_000)));
    // 10 cm further: nothing in reach
    let (p, ap, nb) = pair_at(cafe, "Cafe", "", 0, 351);
    let mut e = engine(&p);
    run_to(&mut e, SEC);
    t.eq("beyond", wn(&e, nb), vec![(0, WNote::ScanStart { ssid: s("Cafe"), bands: 7, ms: 760 }), (760 * MS, WNote::ScanNone { ssid: s("Cafe") })]);
    t.eq("waits", e.wlan_station(nb).map(|v| v.state), Some(StaState::NoNetwork));
    t.eq("the problem", wlan_problems(&e), vec![(ProblemKind::WlanNoNetwork, Severity::Warning, nb, Some(0), s("Das WLAN \u{201e}Cafe\u{201c} ist f\u{fc}r Laptop1 au\u{df}er Reichweite."))]);
    // it comes nearer: it looks again at once (30 m: -80 dBm)
    let _ = e.apply(Action::Move { dev: nb, x: 0, y: 300 });
    run_to(&mut e, 3 * SEC);
    t.eq("nearer", wn_from(&e, nb, SEC).into_iter().filter(|x| matches!(x.1, WNote::ScanStart { .. } | WNote::Assoc { .. })).collect::<Vec<_>>(), vec![
        (SEC, WNote::ScanStart { ssid: s("Cafe"), bands: 7, ms: 760 }),
        (SEC + JOINED, WNote::Assoc { ap: s("AP1"), aid: 1, band: Band::G5, ch: 36, dbm: -80, kbps: 144_000, reassoc: false }),
    ]);
    // and leaves: out of range at once, the connection is lost after seven beacons (716.8 ms)
    let _ = e.apply(Action::Move { dev: nb, x: 0, y: 600 });
    run_to(&mut e, 6 * SEC);
    let lost = 3 * SEC + 716_800 * US;
    t.eq(
        "it leaves",
        wn_from(&e, nb, 3 * SEC),
        vec![
            (3 * SEC, WNote::OutOfRange { ap: s("AP1") }),
            (lost, WNote::BeaconLoss { ap: s("AP1") }),
            (lost, WNote::ScanStart { ssid: s("Cafe"), bands: 7, ms: 760 }),
            (lost + 760 * MS, WNote::ScanNone { ssid: s("Cafe") }),
        ],
    );
    t.eq("the AP notices", wn_from(&e, ap, 3 * SEC), vec![(lost, WNote::StaLeft { sta: s("Laptop1"), reason: 4 })]);
    t.check("no association", e.wlan_assocs().is_empty());
}

fn ap_changes(t: &mut T) {
    let (p, ap, nb) = pair(vec![net("Buero", Band::G5, Security::Wpa2, PASS)], "Buero", PASS);
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    // switched off: the station misses the beacons, then finds nothing
    t.check("off", e.apply(Action::Power { dev: ap, on: false }).is_ok());
    run_to(&mut e, 5 * SEC);
    let lost = 2 * SEC + 716_800 * US;
    t.eq(
        "AP off",
        wn_from(&e, nb, 2 * SEC),
        vec![(lost, WNote::BeaconLoss { ap: s("AP1") }), (lost, WNote::ScanStart { ssid: s("Buero"), bands: 7, ms: 760 }), (lost + 760 * MS, WNote::ScanNone { ssid: s("Buero") })],
    );
    t.eq("a switched-off AP says nothing", wn_from(&e, ap, 2 * SEC), Vec::new());
    // on again: the station looks at once
    t.check("on", e.apply(Action::Power { dev: ap, on: true }).is_ok());
    run_to(&mut e, 8 * SEC);
    t.eq("back", times(&e, nb, |n| matches!(n, WNote::Authorized { .. })), vec![JOINED + 602_475, 5 * SEC + JOINED + 602_475]);
    // another passphrase on the AP sends its station away; with the old one it fails
    let mut nets = vec![net("Buero", Band::G5, Security::Wpa2, "Herbst2026!")];
    t.check("reconfigured", e.apply(Action::Wlan { dev: ap, wlan: Some(Wlan { nets: nets.clone(), join: None }) }).is_ok());
    run_to(&mut e, 10 * SEC);
    t.eq("sent away", wn_from(&e, nb, 8 * SEC).into_iter().take(2).collect::<Vec<_>>(), vec![(8 * SEC, WNote::Deauthed { ap: s("AP1"), reason: 3 }), (8 * SEC, WNote::ScanStart { ssid: s("Buero"), bands: 7, ms: 760 })]);
    t.eq("the AP's side", wn_from(&e, ap, 8 * SEC).first().cloned(), Some((8 * SEC, WNote::StaLeft { sta: s("Laptop1"), reason: 3 })));
    t.eq("the old passphrase fails", when(&e, ap, |n| matches!(n, WNote::MicBad { .. })).filter(|&x| x > 8 * SEC), Some(8 * SEC + JOINED + 401_500));
    // the old one again; then the radio shut down ends the association as well
    nets[0].passphrase = s(PASS);
    let _ = e.apply(Action::Wlan { dev: ap, wlan: Some(Wlan { nets, join: None }) });
    run_to(&mut e, 20 * SEC);
    t.eq("joined again", e.wlan_station(nb).map(|v| v.state), Some(StaState::Connected));
    t.check("radio shut down", e.apply(Action::PortAdmin { dev: ap, port: s("Dot11Radio1"), up: false }).is_ok());
    run_to(&mut e, 21 * SEC);
    t.eq("deauthenticated", wn_from(&e, nb, 20 * SEC).first().cloned(), Some((20 * SEC, WNote::Deauthed { ap: s("AP1"), reason: 3 })));
    t.eq("the radio is down", (e.port_led(ap, 2), e.wlan_station(nb).map(|v| v.state)), (Led::Down, Some(StaState::NoNetwork)));
    // the station's port shut down: it leaves; up again: it joins
    let _ = e.apply(Action::PortAdmin { dev: ap, port: s("Dot11Radio1"), up: true });
    run_to(&mut e, 23 * SEC);
    t.eq("radio up: back", e.wlan_station(nb).map(|v| v.state), Some(StaState::Connected));
    t.check("station port down", e.apply(Action::PortAdmin { dev: nb, port: s("wlan0"), up: false }).is_ok());
    t.eq("it leaves", (wn(&e, nb).last().cloned(), e.wlan_station(nb).map(|v| v.state)), (Some((23 * SEC, WNote::Left { ap: s("AP1") })), Some(StaState::Idle)));
    t.eq("the AP hears it", wn(&e, ap).last().cloned(), Some((23 * SEC, WNote::StaLeft { sta: s("Laptop1"), reason: 3 })));
}

fn isolation(t: &mut T) {
    // two laptops on one network of an AP and PC1 on its cable
    let mut p = Project::new("iso", 3);
    let ap = add(&mut p, "jf-ap6e", 0, 0, "hostname AP1\n");
    let a = add(&mut p, "laptop", 30, 0, "hostname Laptop1\nip addr add 10.0.0.2/24 dev wlan0\n");
    let b = add(&mut p, "laptop", 0, 30, "hostname Laptop2\nip addr add 10.0.0.3/24 dev wlan0\n");
    let pc = add(&mut p, "pc", 0, -300, "hostname PC1\nip addr add 10.0.0.1/24 dev eth0\n");
    t.check("cable", p.connect(&pr(pc, "eth0"), &pr(ap, "TwoGigabitEthernet0"), None).is_ok());
    let buero = net("Buero", Band::G5, Security::Wpa2, PASS);
    offer(&mut p, ap, vec![buero.clone()]);
    join(&mut p, a, "Buero", PASS);
    join(&mut p, b, "Buero", PASS);
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    t.eq("two stations", e.wlan_stations_of(ap).iter().map(|x| (x.sta, x.aid)).collect::<Vec<_>>(), vec![(a, 1), (b, 2)]);
    // station to station through the AP: four times the air (ARP 200430, an echo 200620 ns)
    let id = ping(&mut e, a, "10.0.0.3", 2);
    run_to(&mut e, 4 * SEC);
    t.eq("relayed", ping_ev(&e, id), vec![reply(1, "10.0.0.3", 1_604_200), reply(2, "10.0.0.3", 802_480)]);
    // client isolation: the stations stay, an echo to the other one is dropped
    t.check("isolate", e.apply(Action::Wlan { dev: ap, wlan: Some(Wlan { nets: vec![WlanNet { isolate: true, ..buero.clone() }], join: None }) }).is_ok());
    t.eq("still associated", e.wlan_assocs().len(), 2);
    let id = ping(&mut e, a, "10.0.0.3", 1);
    run_to(&mut e, 10 * SEC);
    t.eq("isolated", ping_ev(&e, id), vec![PingEv::Timeout { seq: 1 }]);
    t.eq("the AP drops it", wn_from(&e, ap, 4 * SEC).first().cloned(), Some((4 * SEC + 200_620, WNote::Isolated { mac: jfn_sim::node::port_mac(b, 0), guest: false })));
    // the wired side stays reachable
    let id = ping(&mut e, a, "10.0.0.1", 1);
    run_to(&mut e, 12 * SEC);
    t.eq("the cable", ping_ev(&e, id), vec![reply(1, "10.0.0.1", 803_616)]);
    // a guest network on a plain AP: the same between stations
    let _ = e.apply(Action::Wlan { dev: ap, wlan: Some(Wlan { nets: vec![WlanNet { guest: true, ..buero }], join: None }) });
    let id = ping(&mut e, b, "10.0.0.2", 1);
    run_to(&mut e, 20 * SEC);
    t.eq("guests apart", ping_ev(&e, id).first().map(|x| matches!(x, PingEv::Reply { .. })), Some(false));
}

fn home(t: &mut T) {
    // the home router: its LAN and its networks on one bridge; a guest reaches only the router
    let mut p = Project::new("home", 4);
    let gw = add(&mut p, "home-router", 0, 0, "hostname Gateway1\n");
    let pc = add(&mut p, "pc", 0, -300, "hostname PC1\nip addr add 192.168.1.10/24 dev eth0\n");
    let ph = add(&mut p, "smartphone", 30, 0, "hostname Smartphone1\nip addr add 192.168.1.20/24 dev wlan0\n");
    let nb = add(&mut p, "laptop", 0, 30, "hostname Laptop1\nip addr add 192.168.1.30/24 dev wlan0\n");
    t.check("cable", p.connect(&pr(pc, "eth0"), &pr(gw, "lan1"), None).is_ok());
    offer(&mut p, gw, vec![net("Heim", Band::G5, Security::Wpa2, PASS), WlanNet { guest: true, ..net("Gast", Band::G24, Security::Open, "") }]);
    join(&mut p, ph, "Gast", "");
    join(&mut p, nb, "Heim", PASS);
    let mut e = engine(&p);
    t.check("router address", e.apply(Action::IfAddr { dev: gw, iface: s("br0"), prefix: netpkt::Prefix4::parse("192.168.1.1/24").unwrap_or_default(), add: true }).is_ok());
    run_to(&mut e, 2 * SEC);
    t.eq("both on wlan0", e.wlan_stations_of(gw).iter().map(|x| (x.sta, x.ssid.clone(), x.ap_port)).collect::<Vec<_>>(), vec![(ph, s("Gast"), 5), (nb, s("Heim"), 5)]);
    // the laptop of the home network reaches the PC on the cable
    let id = ping(&mut e, nb, "192.168.1.10", 1);
    run_to(&mut e, 4 * SEC);
    t.check("home network to the LAN", matches!(ping_ev(&e, id).as_slice(), [PingEv::Reply { .. }]));
    // the guest: the router answers (2.4 GHz at 3 m: -48 dBm, 382 Mbit/s on the link, 300 µs)
    let id = ping(&mut e, ph, "192.168.1.1", 2);
    run_to(&mut e, 7 * SEC);
    t.eq("guest to the router", ping_ev(&e, id).into_iter().map(|x| matches!(x, PingEv::Reply { .. })).collect::<Vec<_>>(), vec![true, true]);
    let id = ping(&mut e, ph, "192.168.1.10", 1);
    run_to(&mut e, 12 * SEC);
    t.check("guest not to the LAN", !ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Reply { .. })));
    let id = ping(&mut e, ph, "192.168.1.30", 1);
    run_to(&mut e, 17 * SEC);
    t.check("guest not to the home network", !ping_ev(&e, id).iter().any(|x| matches!(x, PingEv::Reply { .. })));
    // its shell: a second network on the radio is wlan0-1
    let o = cli(&mut e, gw, &["iw dev"]);
    t.check(&format!("iw dev of the home router: {:?}", o), o.contains("\tInterface wlan0\n") && o.contains("\t\tssid Heim\n") && o.contains("\tInterface wlan0-1\n") && o.contains("\t\tssid Gast\n") && o.contains("\t\ttype AP\n"));
    let o = cli(&mut e, gw, &["iw dev wlan0 station dump"]);
    t.check(&format!("station dump: {:?}", o), o.matches("Station ").count() == 2 && o.contains("\tauthorized:\tyes\n"));
}

fn fixed(t: &mut T) {
    // the radio link of an older project stays a cable: no scan, no association of the engine
    let p = Project::from_text(include_str!("../tests/fixtures/office.jfnet"));
    t.check("office loads", p.is_ok());
    let Ok(p) = p else { return };
    let (ap1, nb) = (12, 13);
    let mut e = engine(&p);
    run_to(&mut e, 10 * SEC);
    t.eq("fixed", e.wlan_station(nb).map(|v| (v.state, v.assoc.is_none())), Some((StaState::Fixed, true)));
    t.check("no WLAN decisions", !e.trace.events().any(|x| matches!(x.note, Note::Wlan(_))));
    t.check("no associations", e.wlan_assocs().is_empty());
    t.eq("the AP's radio as before", e.port_led(ap1, 2), Led::Up);
    t.eq("its shell", cli(&mut e, nb, &["iw dev wlan0 link"]), s("wlan0: fixed radio link of the project to AP1 Dot11Radio1\n"));
    // a radio cable beside WLAN settings: the cable counts, also 85 m away, and its radio sends
    // no network
    let mut p = Project::new("fixed", 6);
    let ap = add(&mut p, "jf-ap6e", 0, 0, "hostname AP1\n");
    let nb = add(&mut p, "laptop", 600, 600, "hostname Laptop1\nip addr add 10.0.0.2/24 dev wlan0\n");
    let pc = add(&mut p, "pc", 0, -300, "hostname PC1\nip addr add 10.0.0.1/24 dev eth0\n");
    t.check("cable", p.connect(&pr(pc, "eth0"), &pr(ap, "TwoGigabitEthernet0"), None).is_ok());
    let radio = p.connect(&pr(nb, "wlan0"), &pr(ap, "Dot11Radio1"), None).unwrap_or(0);
    t.eq("a radio cable", p.link(radio).map(|l| (l.cable, l.kbps, l.delay_ns)), Some((jfn_model::CableKind::Wireless, 2_400_000, 0)));
    offer(&mut p, ap, vec![net("Buero", Band::G5, Security::Wpa2, PASS)]);
    join(&mut p, nb, "Buero", PASS);
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    t.eq("the cable counts", (e.wlan_station(nb).map(|v| v.state), wn(&e, nb), e.wlan_assocs().len()), (Some(StaState::Fixed), Vec::new(), 0));
    t.eq("no problem for it", wlan_problems(&e), Vec::new());
    // PC1 - AP1 (2.5 Gbit/s: ARP 268 + 50, an echo 390 + 50 ns) - the radio cable (2.4 Gbit/s,
    // no delay: ARP 280, an echo 406 ns)
    let id = ping(&mut e, pc, "10.0.0.2", 2);
    run_to(&mut e, 5 * SEC);
    t.eq("ping over the radio cable", ping_ev(&e, id), vec![reply(1, "10.0.0.2", 2888), reply(2, "10.0.0.2", 1692)]);
    // unplugged: the laptop looks for its network (out of reach there), then nearer it joins
    t.check("unplug", e.apply(Action::Disconnect(radio)).is_ok());
    run_to(&mut e, 6 * SEC);
    t.eq("it looks", wn_from(&e, nb, 5 * SEC), vec![(5 * SEC, WNote::ScanStart { ssid: s("Buero"), bands: 7, ms: 760 }), (5 * SEC + 760 * MS, WNote::ScanNone { ssid: s("Buero") })]);
    let _ = e.apply(Action::Move { dev: nb, x: 30, y: 0 });
    run_to(&mut e, 8 * SEC);
    t.eq("over the air", when(&e, nb, |n| matches!(n, WNote::Authorized { .. })), Some(6 * SEC + JOINED + 602_475));
}

// ---------------------------------------------------------------- command lines

fn shells(t: &mut T) {
    let (p, ap, nb) = pair(vec![net("Buero", Band::G5, Security::Wpa2, PASS)], "Buero", PASS);
    let mut e = engine(&p);
    run_to(&mut e, 2 * SEC);
    let bssid = jfn_sim::node::port_mac(ap, 2);
    let mac = jfn_sim::node::port_mac(nb, 0);
    t.eq(
        "iw dev",
        cli(&mut e, nb, &["iw dev"]),
        format!("phy#0\n\tInterface wlan0\n\t\tifindex 2\n\t\twdev 0x1\n\t\taddr {}\n\t\tssid Buero\n\t\ttype managed\n\t\tchannel 36 (5180 MHz)\n\t\ttxpower 20.00 dBm\n", mac),
    );
    let o = cli(&mut e, nb, &["iw dev wlan0 link"]);
    t.check(
        &format!("iw link: {:?}", o),
        o.starts_with(&format!("wlan0: connected to {} (AP1)\n\tSSID: Buero\n\tfreq: 5180\n", bssid)) && o.contains("\tsignal: -50 dBm\n\trx bitrate: 2400.0 MBit/s\n\ttx bitrate: 2400.0 MBit/s\n") && o.ends_with("\tsecurity: WPA2-Personal (PSK, CCMP)\n\tconnected time: 1 seconds\n"),
    );
    // a scan takes as long as the station's (760 ms): nothing before, then what it found
    t.eq("scan runs", cli(&mut e, nb, &["iw dev wlan0 scan"]), String::new());
    let t0 = e.now();
    run_to(&mut e, t0 + 759 * MS);
    t.eq("not yet", out(&mut e, nb), String::new());
    run_to(&mut e, t0 + 760 * MS);
    let o = out(&mut e, nb);
    t.check(
        &format!("scan: {:?}", o),
        o.starts_with(&format!("BSS {}(on wlan0) -- associated\n\tfreq: 5180\n\tbeacon interval: 100 TUs\n\tsignal: -50.00 dBm\n\tlast seen: 0 ms ago\n\tSSID: Buero\n\tchannel: 36\n\tRSN:\t * Version: 1\n", bssid)) && o.contains("\t\t * Authentication suites: PSK\n"),
    );
    // the state of the link in ip link
    t.check("ip link up", cli(&mut e, nb, &["ip link show wlan0"]).contains("state UP"));
    // leave and join from the shell (the project keeps its setting)
    t.eq("disconnect", cli(&mut e, nb, &["iw dev wlan0 disconnect"]), String::new());
    t.eq("left", (wn(&e, nb).last().map(|x| x.1.clone()), e.wlan_assocs().len()), (Some(WNote::Left { ap: s("AP1") }), 0));
    t.eq("not connected", cli(&mut e, nb, &["iw dev wlan0 link"]), s("wlan0: not connected\n"));
    t.check("ip link down", cli(&mut e, nb, &["ip link show wlan0"]).contains("NO-CARRIER"));
    let t0 = e.now();
    t.eq("connect", cli(&mut e, nb, &[&format!("iw dev wlan0 connect Buero psk {}", PASS)]), String::new());
    run_to(&mut e, t0 + 2 * SEC);
    t.eq("joined from the shell", times(&e, nb, |n| matches!(n, WNote::Assoc { .. })).last().copied(), Some(t0 + JOINED));
    t.eq("scanning", {
        let _ = cli(&mut e, nb, &["iw dev wlan0 connect Nirgendwo"]);
        cli(&mut e, nb, &["iw dev wlan0 link"])
    }, s("wlan0: not connected (scanning for Nirgendwo)\n"));
    let t1 = e.now();
    run_to(&mut e, t1 + SEC);
    t.eq("out of reach", cli(&mut e, nb, &["iw dev wlan0 link"]), s("wlan0: not connected (Nirgendwo is out of reach)\n"));
    t.eq(
        "errors",
        (cli(&mut e, nb, &["iw dev eth0 link"]), cli(&mut e, nb, &["iw dev wlan7 link"]), cli(&mut e, nb, &["iw dev wlan0 connect"]), cli(&mut e, nb, &["iw dev wlan0 connect Buero psk kurz"])),
        (s("command failed: Operation not supported (-95)\n"), s("command failed: No such device (-19)\n"), s("Usage: iw dev IF connect SSID [psk PASSPHRASE]\n"), s("iw: a passphrase has 8 to 63 printable characters\n")),
    );
    t.check("usage", cli(&mut e, nb, &["iw"]).starts_with("Usage: iw dev "));
    let _ = cli(&mut e, nb, &[&format!("iw dev wlan0 connect Buero psk {}", PASS)]);
    let t2 = e.now();
    run_to(&mut e, t2 + 2 * SEC);
    // the IOS-like access point lists its stations
    let head = format!("{:<12} {:<16} {:<15} {:<15} {:<4} {:<7} {:<12} {}\n", "Radio", "SSID", "Station", "MAC address", "AID", "Signal", "Rate", "State");
    let row = format!("{:<12} {:<16} {:<15} {:<15} {:<4} {:<7} {:<12} {}\n", "Dot11Radio1", "Buero", "Laptop1", mac.cisco(), 1, -50, "2400.0 Mb/s", "authorized");
    // (the console shows the log lines that came meanwhile after the output)
    let o = cli(&mut e, ap, &["show dot11 associations"]);
    t.check(&format!("show dot11 associations: {:?}", o), o.starts_with(&format!("{}{}", head, row)));
    t.eq("station dump of a station", cli(&mut e, nb, &["iw dev wlan0 station dump"]).lines().next().map(String::from), Some(format!("Station {} (on wlan0)", bssid)));
    let _ = cli(&mut e, nb, &["iw dev wlan0 disconnect"]);
    t.check("no stations", cli(&mut e, ap, &["show dot11 associations"]).starts_with("No station is associated with this access point.\n"));
}

// ---------------------------------------------------------------- problems, model, undo, texts

fn problems(t: &mut T) {
    let mut p = Project::new("p", 2);
    let ap = add(&mut p, "jf-ap6e", 0, 0, "hostname AP1\n");
    let a = add(&mut p, "laptop", 30, 0, "hostname Laptop1\n");
    let b = add(&mut p, "laptop", 0, 30, "hostname Laptop2\n");
    let c = add(&mut p, "laptop", 0, -30, "hostname Laptop3\n");
    let d = add(&mut p, "laptop", 900, 0, "hostname Laptop4\n");
    let f = add(&mut p, "laptop", -30, 0, "hostname Laptop5\n");
    offer(&mut p, ap, vec![net("Buero", Band::G5, Security::Wpa2, PASS), net("Labor", Band::G6, Security::Wpa2, PASS)]);
    join(&mut p, a, "Nirgendwo", "");
    join(&mut p, b, "Buero", "");
    join(&mut p, c, "Buero", "Falsch2026!");
    join(&mut p, d, "Buero", PASS);
    join(&mut p, f, "Buero", PASS);
    let e = engine(&p);
    t.eq(
        "problems",
        wlan_problems(&e),
        vec![
            (ProblemKind::WlanBadNet, Severity::Error, ap, Some(3), s("Im 6-GHz-Band gibt es nur WPA3-Personal.")),
            (ProblemKind::WlanNoNetwork, Severity::Error, a, Some(0), s("Kein Access Point bietet das WLAN \u{201e}Nirgendwo\u{201c} an, das Laptop1 sucht.")),
            (ProblemKind::WlanPassphrase, Severity::Error, b, Some(0), s("Laptop2 hat keine Passphrase f\u{fc}r das WLAN \u{201e}Buero\u{201c}.")),
            (ProblemKind::WlanPassphrase, Severity::Error, c, Some(0), s("Die Passphrase von Laptop3 passt nicht zum WLAN \u{201e}Buero\u{201c}.")),
            (ProblemKind::WlanNoNetwork, Severity::Warning, d, Some(0), s("Das WLAN \u{201e}Buero\u{201c} ist f\u{fc}r Laptop4 au\u{df}er Reichweite.")),
        ],
    );
    // the project's check warns about the network that cannot be offered
    let issues = p.check();
    t.check("check warns", issues.iter().any(|i| !i.error && i.dev == Some(ap) && i.text.contains("WPA3")));
    t.check("a station without a passphrase", e.wlan_station(b).is_some_and(|v| v.join.is_some_and(|j| j.passphrase.is_empty())));
}

fn model(t: &mut T) {
    let (mut p, ap, nb) = pair(vec![net("Buero", Band::G5, Security::Wpa2, PASS), WlanNet { vlan: 20, guest: true, ..net("Gast", Band::G24, Security::Open, "") }], "Buero", PASS);
    p.dip_per_m = 20;
    let text = p.to_text();
    t.eq("round trip", Project::from_text(&text), Ok(p.clone()));
    t.check("in the file", text.contains("\"wlan\": {") && text.contains("\"security\": \"wpa2-personal\"") && text.contains("\"band\": \"2.4\"") && text.contains("\"dip_per_m\": 20"));
    // older files have neither
    let old = Project::from_text("{\"format\": \"jfn\", \"version\": 1, \"devices\": [{\"id\": 1, \"name\": \"AP1\", \"model\": \"jf-ap6e\"}]}");
    t.check("an older file", old.is_ok_and(|p| p.devices[0].wlan.is_none() && p.dip_per_m == 0 && mw::dip_per_m(&p) == 10));
    // the engine's actions
    let mut e = engine(&p);
    t.eq("scale from the project", e.wlan_scale(), 20);
    t.check("unknown devices", e.apply(Action::Move { dev: 99, x: 0, y: 0 }).is_err() && e.apply(Action::Wlan { dev: 99, wlan: None }).is_err());
    t.eq("settings", (e.wlan_settings(ap).map(|w| w.nets.len()), e.wlan_pos(nb)), (Some(2), Some((30, 0))));
    run_to(&mut e, 2 * SEC);
    // 20 units per metre: 1.5 m; then 5: 6 m, -59 dBm (MCS 7)
    t.eq("near", e.wlan_station(nb).and_then(|v| v.assoc).map(|a| a.dbm), Some(-41));
    let t0 = e.now();
    t.check("scale", e.apply(Action::Scale { dip_per_m: 5 }).is_ok() && e.wlan_scale() == 5);
    t.eq("the rate follows", wn(&e, nb).last().cloned(), Some((t0, WNote::Rate { ap: s("AP1"), dbm: -59, kbps: 1_440_000 })));
    // scenarios move devices
    let sc = scenario::parse("move Laptop1 620 300\nat 2s move AP2 -10 5\n");
    t.eq("move steps", sc.ok().map(|x| x.steps.into_iter().map(|s| s.kind).collect::<Vec<_>>()), Some(vec![StepKind::Move { dev: s("Laptop1"), x: 620, y: 300 }, StepKind::Move { dev: s("AP2"), x: -10, y: 5 }]));
    t.check("a bad move", scenario::parse("move Laptop1 620\n").is_err() && scenario::parse("move Laptop1 a b\n").is_err());
}

fn undo(t: &mut T) {
    let (p, ap, nb) = pair(vec![net("Buero", Band::G5, Security::Wpa2, PASS)], "Buero", PASS);
    let mut doc = Doc::new();
    doc.replace(p, None);
    let eng_nets = |doc: &Doc| doc.sim.engine().wlan_settings(ap).map(|w| w.nets.iter().map(|x| x.ssid.clone()).collect::<Vec<_>>());
    t.check("add a network", crate::doc::wlan::set(&mut doc, ap, "WLAN-Netz von AP1 hinzuf\u{fc}gen", |w| w.nets.push(net("Gast", Band::G24, Security::Open, ""))));
    t.eq("project and engine", (doc.proj.device(ap).and_then(|d| d.wlan.as_ref()).map(|w| w.nets.len()), eng_nets(&doc)), (Some(2), Some(vec![s("Buero"), s("Gast")])));
    t.check("unchanged is no step", !crate::doc::wlan::set(&mut doc, ap, "x", |_| {}));
    t.check("undo", doc.undo());
    t.eq("undone", eng_nets(&doc), Some(vec![s("Buero")]));
    t.check("redo", doc.redo());
    t.eq("redone", eng_nets(&doc), Some(vec![s("Buero"), s("Gast")]));
    // leaving the network: None in the project (an empty setting is none)
    t.check("leave", crate::doc::wlan::set(&mut doc, nb, "WLAN von Laptop1 trennen", |w| w.join = None));
    t.eq("left", (doc.proj.device(nb).map(|d| d.wlan.is_none()), doc.sim.engine().wlan_station(nb).map(|v| v.join.is_none())), (Some(true), Some(true)));
    t.check("undo the leave", doc.undo() && doc.sim.engine().wlan_station(nb).is_some_and(|v| v.join.is_some()));
    // the scale
    t.check("scale", crate::doc::wlan::scale(&mut doc, 20));
    t.eq("scaled", (doc.proj.dip_per_m, doc.sim.engine().wlan_scale()), (20, 20));
    t.check("the same scale is no step", !crate::doc::wlan::scale(&mut doc, 20));
    t.check("undo the scale", doc.undo());
    t.eq("scale back", (doc.proj.dip_per_m, doc.sim.engine().wlan_scale()), (0, 10));
    t.check("the document is dirty", doc.dirty());
}

fn texts(t: &mut T) {
    let rows: Vec<(&str, &str)> = include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    let en = move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1);
    let port = |p: u16| format!("Dot11Radio{}", p);
    let mac = Mac([0x02, 0x4a, 0, 0, 3, 1]);
    let cases: Vec<(WNote, &str, &str)> = vec![
        (WNote::ScanStart { ssid: s("Buero"), bands: 7, ms: 760 }, "WLAN: sucht \u{201e}Buero\u{201c} auf 2,4 GHz, 5 GHz, 6 GHz (760 ms).", "WLAN: looking for \u{201c}Buero\u{201d} on 2.4 GHz, 5 GHz, 6 GHz (760 ms)."),
        (
            WNote::ScanFound { ssid: s("Buero"), n: 2, ap: s("AP1"), band: Band::G24, ch: 6, dbm: -71 },
            "WLAN: \u{201e}Buero\u{201c} bei 2 Access Points gefunden, der beste ist AP1 (2,4 GHz, Kanal 6, -71 dBm).",
            "WLAN: \u{201c}Buero\u{201d} found at 2 access points, the best is AP1 (2.4 GHz, channel 6, -71 dBm).",
        ),
        (
            WNote::Assoc { ap: s("AP1"), aid: 1, band: Band::G5, ch: 36, dbm: -50, kbps: 2_401_900, reassoc: false },
            "WLAN: assoziiert mit AP1 (AID 1, 5 GHz, Kanal 36, -50 dBm, 2401,9 Mbit/s).",
            "WLAN: associated with AP1 (AID 1, 5 GHz, channel 36, -50 dBm, 2401.9 Mbit/s).",
        ),
        (WNote::ApAssoc { sta: s("Laptop1"), aid: 2, ssid: s("Buero"), port: 2, reassoc: true }, "WLAN: Laptop1 wechselt an Dot11Radio2 in \u{201e}Buero\u{201c} (AID 2).", "WLAN: Laptop1 roams to Dot11Radio2 in \u{201c}Buero\u{201d} (AID 2)."),
        (WNote::Deauthed { ap: s("AP1"), reason: 15 }, "WLAN: AP1 beendet die Verbindung (Grund 15: Zeit\u{fc}berschreitung im 4-Wege-Handshake).", "WLAN: AP1 ends the connection (reason 15: 4-way handshake timeout)."),
        (WNote::StaLeft { sta: s("Laptop1"), reason: 4 }, "WLAN: Laptop1 ist abgemeldet (Grund 4: zu lange still).", "WLAN: Laptop1 has left (reason 4: silent for too long)."),
        (WNote::Backoff { secs: 10, n: 1 }, "WLAN: neuer Versuch in 10 s (1. Fehlschlag).", "WLAN: next attempt in 10 s (failure 1)."),
        (WNote::Rate { ap: s("AP2"), dbm: -75, kbps: 432_000 }, "WLAN: Signal von AP2 jetzt -75 dBm, 432,0 Mbit/s.", "WLAN: signal of AP2 now -75 dBm, 432.0 Mbit/s."),
        (WNote::MicBad { msg: 2, sta: s("Laptop2") }, "WLAN: der MIC von Nachricht 2 von Laptop2 passt nicht: die Passphrase ist eine andere. Verworfen.", "WLAN: the MIC of message 2 from Laptop2 does not match: the passphrase is a different one. Dropped."),
        (WNote::Isolated { mac, guest: true }, "WLAN: Frame an 02:4a:00:00:03:01 verworfen: das Gastnetz erreicht nur den Router.", "WLAN: frame to 02:4a:00:00:03:01 dropped: the guest network reaches the router only."),
    ];
    for (n, de, english) in cases {
        let m = n.message(&port);
        t.eq(&format!("german {:?}", n), m.text(), s(de));
        t.eq(&format!("english {:?}", n), m.text_with(&en), s(english));
    }
    let n = Note::Wlan(WNote::Eapol { msg: 1, out: true, peer: s("AP1") });
    t.eq("layer and mask", (n.layer(), n.mask() & decode::P_EAPOL, n.is_drop()), (2, decode::P_EAPOL, false));
    t.check("drops", Note::Wlan(WNote::MicBad { msg: 2, sta: s("x") }).is_drop() && Note::Wlan(WNote::Isolated { mac, guest: false }).is_drop() && !Note::Wlan(WNote::Left { ap: s("x") }).is_drop());
    // the problems in English
    let m = jfn_model::msg!("Das WLAN \u{201e}{}\u{201c} ist f\u{fc}r {} au\u{df}er Reichweite.", "Cafe", "Laptop1");
    t.eq("problem in English", m.text_with(&en), s("The WLAN \u{201c}Cafe\u{201d} is out of range for Laptop1."));
}

fn determinism(t: &mut T) {
    let p = Project::from_text(include_str!("../tests/fixtures/wlan_office.jfnet"));
    t.check("wlan office loads", p.is_ok());
    let Ok(p) = p else { return };
    let digest = |e: &Engine| -> Vec<(u64, u32, Vec<u8>)> { e.trace.hops().map(|h| (h.t_tx, h.link, h.bytes.to_vec())).collect() };
    let notes = |e: &Engine| -> Vec<(u64, u32, Note)> { e.trace.events().filter(|x| matches!(x.note, Note::Wlan(_))).map(|x| (x.t, x.dev, x.note.clone())).collect() };
    let mut a = engine(&p);
    let mut b = engine(&p);
    for e in [&mut a, &mut b] {
        run_to(e, 6 * SEC);
        let _ = e.apply(Action::Move { dev: 6, x: 620, y: 300 });
        run_to(e, 20 * SEC);
    }
    let mut c = a.clone();
    for e in [&mut a, &mut b, &mut c] {
        let _ = e.apply(Action::Move { dev: 6, x: 380, y: 300 });
        run_to(e, 40 * SEC);
    }
    t.check("two runs alike", digest(&a) == digest(&b) && notes(&a) == notes(&b) && !notes(&a).is_empty());
    t.check("a clone goes on alike", digest(&a) == digest(&c) && notes(&a) == notes(&c));
    // the office: Laptop1 and Phone1 in, Laptop2 (wrong passphrase) in its third try; Laptop1
    // roamed to AP2 and back
    let st = |e: &Engine, d: u32| e.wlan_station(d).map(|v| v.state);
    t.eq("states", (st(&a, 6), st(&a, 7), st(&a, 8)), (Some(StaState::Connected), Some(StaState::Handshake), Some(StaState::Connected)));
    t.eq("roams", notes(&a).iter().filter(|x| matches!(x.2, Note::Wlan(WNote::Roam { .. }))).count(), 2);
}

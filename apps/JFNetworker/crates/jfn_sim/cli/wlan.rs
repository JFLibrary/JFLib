// wlan -- WLAN at the command lines: an iw-like subset in the Linux shell and "show dot11
// associations" on the IOS-like access point; help texts and status lines are our own words,
// the field names of the listings follow iw
//
//   iw [help]                              what iw does here
//   iw dev                                 the WLAN interfaces: address, SSID, type, channel
//   iw dev IF info                         the same for one
//   iw dev IF link                         the connection: AP, SSID, frequency, signal, rates
//   iw dev IF scan                         waits for a scan (as long as the station's), then
//                                          one block per network found, the best first
//   iw dev IF connect SSID [psk PASS]      join a network (psk: the passphrase of WPA2 / WPA3,
//                                          our own addition: the real iw has none); runtime only,
//                                          the project keeps its setting
//   iw dev IF disconnect                   leave it
//   iw dev IF station dump                 an AP's stations (a station: its AP)
//   show dot11 associations                IOS-like: the stations of every radio
//
// Errors are iw's short ones ("command failed: No such device (-19)").  The home router (a Linux
// AP) names a second network on its radio wlan0-1, a third wlan0-2.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree};
use jfn_model::catalog::Media;
use jfn_model::wlan::{self, Band, Role, Security, WlanJoin};

use super::ios::K;
use super::print::JobOut;
use super::CliJob;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::wlan::{dbm, Bss, StaState};
use crate::wlantrace::mbit_dot;

const IW_USAGE: &str = "Usage: iw dev                          list the WLAN interfaces\n       iw dev IF info|link|scan          one interface, its connection, a scan\n       iw dev IF connect SSID [psk PASS] join a network (psk: its passphrase)\n       iw dev IF disconnect              leave it\n       iw dev IF station dump            the stations of an access point\n";

/// the commands of WLAN in an IOS tree of an access point; `m`: user and privileged mode
pub(crate) fn build_wlan(t: &mut Tree<K>, m: [ModeId; 2]) {
    for x in m {
        t.add(x, "show dot11", "WLAN radios of this access point", None, 0);
        t.add(x, "show dot11 associations", "The stations on the radios", Some(K::ShowDot11), 0);
    }
}

/// a signal in tenths of dBm as iw writes it ("-48.30")
fn dbm2(rssi10: i32) -> String {
    let a = rssi10.unsigned_abs();
    format!("{}{}.{}0", if rssi10 < 0 { "-" } else { "" }, a / 10, a % 10)
}

/// the authentication suite of a network as iw names it
fn suite(s: Security) -> &'static str {
    match s {
        Security::Open => "",
        Security::Wpa2 => "PSK",
        Security::Wpa3 => "SAE",
    }
}

impl Engine {
    /// the radio ports of node `ni`: (index, name)
    fn radio_ports(&self, ni: usize) -> Vec<(u16, String)> {
        let Some(Some(n)) = self.nodes.get(ni) else { return Vec::new() };
        n.ports.iter().enumerate().filter(|(_, p)| p.spec.media.is_radio()).map(|(i, p)| (i as u16, p.spec.name.clone())).collect()
    }

    /// "ifindex": the interface number as ip link counts, a bridge's port after them
    fn iw_ifindex(&self, ni: usize, port: u16) -> usize {
        match self.stack(ni) {
            Some(st) => match st.ifaces.iter().position(|f| f.kind == crate::ip::IfKind::Port(port)) {
                Some(i) => i + 1,
                None => st.ifaces.len() + port as usize + 1,
            },
            None => port as usize + 1,
        }
    }

    /// "iw ..." in the shell
    pub(crate) fn sh_iw(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        match args {
            [] | ["help"] | ["--help"] => {
                self.out(ni, IW_USAGE);
                Ok(())
            }
            ["dev"] => {
                let text = self.iw_dev(ni, None);
                self.out(ni, &text);
                Ok(())
            }
            ["dev", name, rest @ ..] => {
                let radios = self.radio_ports(ni);
                let Some((port, _)) = radios.iter().find(|r| r.1 == *name).cloned() else {
                    let known = self.nodes.get(ni).and_then(|n| n.as_ref()).is_some_and(|n| n.ports.iter().any(|p| p.spec.name == *name));
                    let e = if known { "command failed: Operation not supported (-95)\n" } else { "command failed: No such device (-19)\n" };
                    return self.sh_err(ni, e);
                };
                self.iw_if(ni, port, name, rest)
            }
            [w, ..] => self.sh_err(ni, &format!("iw: command {} is not known here; try \"iw help\"\n", w)),
        }
    }

    fn iw_if(&mut self, ni: usize, port: u16, name: &str, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let station = self.wlan.stas.get(&dev).is_some_and(|s| s.port == port);
        let unsupported = "command failed: Operation not supported (-95)\n";
        match args {
            ["info"] => {
                let text = self.iw_dev(ni, Some(port));
                self.out(ni, &text);
                Ok(())
            }
            ["link"] => {
                let text = self.iw_link(ni, port, name);
                self.out(ni, &text);
                Ok(())
            }
            ["scan", ..] if station => {
                let until = self.now + self.iw_scan_time(ni);
                self.next_job += 1;
                let id = self.next_job;
                let now = self.now;
                if let Some(c) = self.cdev_mut(ni) {
                    c.job = Some(CliJob { id, out: JobOut::IwScan { until, port }, start: now });
                }
                self.cli_wake_at(ni, until);
                self.cli_state(ni);
                Ok(())
            }
            ["connect", rest @ ..] if station => {
                let (ssid, pass) = match rest {
                    [s] => (*s, ""),
                    [s, "psk", p] | [s, "key", p] => (*s, *p),
                    _ => return self.sh_err(ni, "Usage: iw dev IF connect SSID [psk PASSPHRASE]\n"),
                };
                if ssid.is_empty() || ssid.len() > wlan::SSID_MAX {
                    return self.sh_err(ni, "iw: an SSID has 1 to 32 bytes\n");
                }
                if !pass.is_empty() && !wlan::passphrase_ok(pass) {
                    return self.sh_err(ni, "iw: a passphrase has 8 to 63 printable characters\n");
                }
                self.wlan_join(ni, Some(WlanJoin { ssid: String::from(ssid), passphrase: String::from(pass) }));
                Ok(())
            }
            ["disconnect"] if station => {
                self.wlan_join(ni, None);
                Ok(())
            }
            ["station", "dump"] => {
                let text = self.iw_stations(ni, port, name);
                self.out(ni, &text);
                Ok(())
            }
            ["scan", ..] | ["connect", ..] | ["disconnect"] => self.sh_err(ni, unsupported),
            _ => self.sh_err(ni, IW_USAGE),
        }
    }

    /// how long a scan of the station takes
    fn iw_scan_time(&self, ni: usize) -> Ns {
        let Some(Some(n)) = self.nodes.get(ni) else { return 0 };
        let bands = wlan::client_bands(n.model);
        let chans: usize = Band::ALL.iter().filter(|b| bands & b.bit() != 0).map(|&b| wlan::scan_channels(b)).sum();
        crate::wlan::DWELL * chans as Ns
    }

    /// the listing of "iw dev" (one port: "iw dev IF info")
    fn iw_dev(&self, ni: usize, only: Option<u16>) -> String {
        let Some(Some(n)) = self.nodes.get(ni) else { return String::new() };
        let dev = n.dev;
        let mut s = String::from("phy#0\n");
        for (port, name) in self.radio_ports(ni) {
            if only.is_some_and(|o| o != port) {
                continue;
            }
            let mac = n.ports[port as usize].mac;
            let ifindex = self.iw_ifindex(ni, port);
            if wlan::role(n.model) == Role::Ap && n.ports[port as usize].spec.media == Media::RadioAp {
                // one interface per network on this radio
                let nets: Vec<(String, Band, u16)> = self.wlan_settings(dev).map(|w| w.nets.iter().filter(|x| self.wlan_radio_index_of(dev, x.band) == Some(port)).map(|x| (x.ssid.clone(), x.band, wlan::channel_of(x))).collect()).unwrap_or_default();
                if nets.is_empty() {
                    s.push_str(&format!("\tInterface {}\n\t\tifindex {}\n\t\twdev 0x1\n\t\taddr {}\n\t\ttype AP\n\t\ttxpower 20.00 dBm\n", name, ifindex, mac));
                }
                for (k, (ssid, band, ch)) in nets.into_iter().enumerate() {
                    let ifn = if k == 0 { name.clone() } else { format!("{}-{}", name, k) };
                    let addr = crate::wlan::bssid(mac, k);
                    s.push_str(&format!("\tInterface {}\n\t\tifindex {}\n\t\twdev 0x{:x}\n\t\taddr {}\n\t\tssid {}\n\t\ttype AP\n\t\tchannel {} ({} MHz)\n\t\ttxpower 20.00 dBm\n", ifn, ifindex + k, k + 1, addr, ssid, ch, wlan::freq_mhz(band, ch)));
                }
                continue;
            }
            s.push_str(&format!("\tInterface {}\n\t\tifindex {}\n\t\twdev 0x1\n\t\taddr {}\n", name, ifindex, mac));
            let a = self.wlan.assocs.values().find(|a| a.sta == dev && a.sta_port == port);
            if let Some(a) = a {
                s.push_str(&format!("\t\tssid {}\n", a.ssid));
            }
            s.push_str("\t\ttype managed\n");
            if let Some(a) = a {
                s.push_str(&format!("\t\tchannel {} ({} MHz)\n", a.channel, wlan::freq_mhz(a.band, a.channel)));
            }
            s.push_str("\t\ttxpower 20.00 dBm\n");
        }
        s
    }

    /// the radio port of a band on an AP (by device id)
    fn wlan_radio_index_of(&self, dev: u32, band: Band) -> Option<u16> {
        let n = self.node(dev)?;
        n.port_by_name(&wlan::radio_port(n.model, band)?)
    }

    /// "iw dev IF link"
    fn iw_link(&self, ni: usize, port: u16, name: &str) -> String {
        let dev = self.dev_of(ni);
        let Some(v) = self.wlan_station(dev).filter(|v| v.port == port) else {
            return format!("{}: not connected (an access point's radio)\n", name);
        };
        if v.state == StaState::Fixed {
            // the cable of the project and its far end
            let far = self.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.ports[port as usize].link).and_then(|li| self.links.get(li).and_then(|l| l.as_ref())).map(|l| if l.ends[0] == (ni, port) { l.ends[1] } else { l.ends[0] });
            let peer = far.and_then(|(fni, fp)| self.nodes.get(fni).and_then(|n| n.as_ref()).map(|n| format!("{} {}", n.name, n.port_name(fp)))).unwrap_or_default();
            return format!("{}: fixed radio link of the project to {}\n", name, peer);
        }
        let Some(a) = v.assoc.as_ref() else {
            let why = match v.state {
                StaState::Scanning => format!(" (scanning for {})", v.join.as_ref().map_or("", |j| j.ssid.as_str())),
                StaState::Joining => String::from(" (joining)"),
                StaState::Backoff => format!(" (next try in {} s)", v.retry_at.map_or(0, |t| t.saturating_sub(self.now).div_ceil(SEC))),
                StaState::NoNetwork => format!(" ({} is out of reach)", v.join.as_ref().map_or("", |j| j.ssid.as_str())),
                StaState::NoPassphrase => String::from(" (the network wants a passphrase)"),
                StaState::GaveUp => format!(" (gave up after {} failures)", v.failures),
                _ => String::new(),
            };
            return format!("{}: not connected{}\n", name, why);
        };
        let ap = self.node(a.ap).map(|n| n.name.clone()).unwrap_or_default();
        let head = if a.authorized { format!("{}: connected to {} ({})\n", name, a.bssid, ap) } else { format!("{}: associated with {} ({}), the 4-way handshake runs\n", name, a.bssid, ap) };
        let c = self.node(dev).map(|n| n.ports[port as usize].counters).unwrap_or_default();
        let rate = mbit_dot(a.kbps);
        let mut s = head;
        s.push_str(&format!("\tSSID: {}\n\tfreq: {}\n\tRX: {} bytes ({} packets)\n\tTX: {} bytes ({} packets)\n\tsignal: {} dBm\n\trx bitrate: {} MBit/s\n\ttx bitrate: {} MBit/s\n", a.ssid, wlan::freq_mhz(a.band, a.channel), c.rx_bytes, c.rx_frames, c.tx_bytes, c.tx_frames, a.dbm, rate, rate));
        let sec = match a.security {
            Security::Open => String::from("open"),
            x => format!("{} ({}, CCMP)", x.title(), suite(x)),
        };
        s.push_str(&format!("\tsecurity: {}\n\tconnected time: {} seconds\n", sec, self.now.saturating_sub(a.since) / SEC));
        s
    }

    /// the end of "iw dev IF scan": the networks found, the best first
    pub(crate) fn iw_scan_done(&mut self, ni: usize) {
        let Some(CliJob { out: JobOut::IwScan { port, .. }, .. }) = self.cdev(ni).and_then(|c| c.job.clone()) else {
            return;
        };
        let dev = self.dev_of(ni);
        let name = self.nodes.get(ni).and_then(|n| n.as_ref()).map(|n| String::from(n.port_name(port))).unwrap_or_default();
        let mine = self.wlan.assocs.values().find(|a| a.sta == dev).map(|a| (a.ap, a.ap_port, a.ssid.clone()));
        let found: Vec<Bss> = self.wlan_survey(dev, None);
        let mut s = String::new();
        for b in &found {
            let cur = mine.as_ref().is_some_and(|m| m.0 == b.ap && m.1 == b.port && m.2 == b.ssid);
            s.push_str(&format!("BSS {}(on {}){}\n\tfreq: {}\n\tbeacon interval: 100 TUs\n\tsignal: {} dBm\n\tlast seen: 0 ms ago\n\tSSID: {}\n\tchannel: {}\n", b.bssid, name, if cur { " -- associated" } else { "" }, wlan::freq_mhz(b.band, b.channel), dbm2(b.rssi10), b.ssid, b.channel));
            if b.security != Security::Open {
                s.push_str(&format!("\tRSN:\t * Version: 1\n\t\t * Group cipher: CCMP\n\t\t * Pairwise ciphers: CCMP\n\t\t * Authentication suites: {}\n", suite(b.security)));
            }
        }
        if found.is_empty() {
            s.push_str(&format!("{}: no network in reach\n", name));
        }
        self.out(ni, &s);
        self.cli_job_done(ni);
    }

    /// "iw dev IF station dump": an AP's stations, a station's AP
    fn iw_stations(&self, ni: usize, port: u16, name: &str) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::new();
        let list: Vec<&crate::wlan::Assoc> = self.wlan.assocs.values().filter(|a| (a.ap == dev && a.ap_port == port) || (a.sta == dev && a.sta_port == port)).collect();
        for a in list {
            let station = a.sta == dev;
            let peer = if station { a.bssid } else { a.sta_mac };
            // the station's counters, seen from the side asked
            let c = self.node(a.sta).map(|n| n.ports[a.sta_port as usize].counters).unwrap_or_default();
            let (rxb, rxp, txb, txp) = if station { (c.rx_bytes, c.rx_frames, c.tx_bytes, c.tx_frames) } else { (c.tx_bytes, c.tx_frames, c.rx_bytes, c.rx_frames) };
            let rate = mbit_dot(a.kbps);
            s.push_str(&format!("Station {} (on {})\n\tinactive time:\t0 ms\n\trx bytes:\t{}\n\trx packets:\t{}\n\ttx bytes:\t{}\n\ttx packets:\t{}\n\tsignal:  \t{} dBm\n\ttx bitrate:\t{} MBit/s\n\trx bitrate:\t{} MBit/s\n\tauthorized:\t{}\n\tassociated at:\t{} ms\n", peer, name, rxb, rxp, txb, txp, dbm(a.rssi10), rate, rate, if a.sta_ok && a.ap_ok { "yes" } else { "no" }, a.since / MS));
        }
        s
    }

    /// "show dot11 associations" (IOS-like)
    pub(crate) fn ios_dot11(&mut self, ni: usize, _p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let list = self.wlan_stations_of(dev);
        let mut s = String::new();
        if list.is_empty() {
            s.push_str("No station is associated with this access point.\n");
        } else {
            s.push_str(&format!("{:<12} {:<16} {:<15} {:<15} {:<4} {:<7} {:<12} {}\n", "Radio", "SSID", "Station", "MAC address", "AID", "Signal", "Rate", "State"));
            for a in &list {
                let radio = self.node(dev).map(|n| String::from(n.port_name(a.ap_port))).unwrap_or_default();
                let name = self.node(a.sta).map(|n| n.name.clone()).unwrap_or_default();
                let mac = self.node(a.sta).map(|n| n.ports[a.sta_port as usize].mac.cisco()).unwrap_or_default();
                let state = if a.authorized { "authorized" } else if a.lost { "lost" } else { "handshake" };
                s.push_str(&format!("{:<12} {:<16} {:<15} {:<15} {:<4} {:<7} {:<12} {}\n", radio, a.ssid, name, mac, a.aid, a.dbm, format!("{} Mb/s", mbit_dot(a.kbps)), state));
            }
        }
        self.out(ni, &s);
        Ok(())
    }
}

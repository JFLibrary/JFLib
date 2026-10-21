// synth -- synthetic projects for performance tests: a campus of n devices from the catalog with
// working configurations, laid out in two rows around a core
//
//   topology(n) -> Project      n devices (MIN_DEVICES ..= MAX_DEVICES), about 1.6 cables per
//                               device; the same n always gives the same project
//
// The campus: core routers (JF-R8) in a row, joined in a ring over 100G (the cable that closes
// the ring runs below them); pods above and below, each one router (JF-R4) facing the core
// with two uplinks to the two nearest core routers, two access switches (JF-S24P) in the
// networks 10.P.1.0/24 and 10.P.2.0/24 (P = the pod's number from 1), hosts in a column on
// either side (PCs, a printer, a camera) and servers in the middle column with one port on
// each switch and an address in each network.  The pod routers form a ring of serial lines and
// pair up across the core over GigabitEthernet; those cables carry no routes.
//
// Routing is static: hosts and servers point at their pod router, a pod router has a default
// route over its first uplink and a floating one (distance 10) over the second, a core router
// knows every pod's 10.P.0.0/16 over the pod's uplink or along the shorter way round the ring
// towards a core router that has it.  There is no spanning tree yet, so every cable beyond a
// tree joins routed interfaces or a server's two networks: no layer-2 loop.  Links between
// routers are /30 networks (172.16 uplinks, the core's end .1; 172.17 the ring; 172.18
// between pods).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::project::{Device, Layout, Point, PortRef, Project, Route};

/// the most devices: pods are the networks 10.P.0.0/16 with P = 1 ..= 250
pub const MAX_DEVICES: usize = 5000;
/// the fewest: two core routers and a pod with one host
pub const MIN_DEVICES: usize = 6;

/// devices per pod (router, two switches, hosts) the plan aims at
const POD_SIZE: usize = 26;
/// host ports of an access switch (Gi1/0/24 goes to the pod router)
const SWITCH_HOSTS: usize = 23;
/// pod uplinks per core router the plan aims at, and the most it takes (Te0/0/0 ..= Te0/0/7)
const CORE_UPLINKS: usize = 6;
const CORE_PORTS: usize = 8;

// the layout in world units: the device box, the pitch of a host column, of the pods' columns,
// the distance of the pods' routers from the core row, the first host row and the switches
// below a pod router
const BOX_W: i32 = 64;
const BOX_H: i32 = 48;
const ROW: i32 = 72;
const POD_PITCH: i32 = 800;
const BAND: i32 = 200;
const HOSTS_AT: i32 = 208;
const SWITCH_AT: i32 = 112;
// x in a pod: hosts of switch A, switch A, the router and the servers, switch B, hosts of B
const COL_A: i32 = 0;
const COL_SW_A: i32 = 160;
const COL_MID: i32 = 320;
const COL_SW_B: i32 = 480;
const COL_B: i32 = 640;

const MASK30: &str = "255.255.255.252";
const MASK24: &str = "255.255.255.0";

/// one pod: hosts on switch A and on switch B, servers on both
#[derive(Clone, Copy, Debug)]
struct Pod {
    a: usize,
    b: usize,
    servers: usize,
}

/// the plan for n devices: the number of core routers and the pods
fn plan(n: usize) -> (usize, Vec<Pod>) {
    let pods = (n / POD_SIZE).max(1);
    let cores = (2 * pods).div_ceil(CORE_UPLINKS).max(2);
    let hosts = n.saturating_sub(cores + 3 * pods).max(pods);
    // the cables besides the hosts' first ones: the ring, two uplinks and two switches per pod,
    // the serial ring and the pairs across the core
    let ring = if cores > 2 { cores } else { 1 };
    let serial = match pods {
        0 | 1 => 0,
        2 => 1,
        p => p,
    };
    let infra = ring + 4 * pods + serial + pods / 2;
    // every server has a second cable: about 1.6 cables per device
    let mut servers = (n * 8 / 5).saturating_sub(infra + hosts).min(hosts);
    let mut v = Vec::with_capacity(pods);
    for p in 0..pods {
        let h = hosts / pods + usize::from(p < hosts % pods);
        let mut s = (servers / (pods - p)).min(h);
        // a switch has SWITCH_HOSTS ports for its hosts and the servers
        while s > 0 && (h - s).div_ceil(2) + s > SWITCH_HOSTS {
            s -= 1;
        }
        let a = (h - s).div_ceil(2).min(SWITCH_HOSTS - s);
        let b = (h - s - a).min(SWITCH_HOSTS - s);
        servers -= s;
        v.push(Pod { a, b, servers: s });
    }
    (cores, v)
}

/// one /30 network between two routers: (.1, .2)
fn net30(second: u8, k: usize) -> (String, String) {
    let (c, d) = (k / 64, (k % 64) * 4);
    (format!("172.{}.{}.{}", second, c, d + 1), format!("172.{}.{}.{}", second, c, d + 2))
}

/// an IOS interface block
fn iface(cfg: &mut String, name: &str, addr: &str, mask: &str) {
    cfg.push_str(&format!("interface {}\n ip address {} {}\n no shutdown\n!\n", name, addr, mask));
}

/// the project being made: devices with names counted per prefix ("PC1", "PC2", ...)
struct Builder {
    p: Project,
    names: Vec<(&'static str, u32)>,
    next: u32,
}

impl Builder {
    fn add(&mut self, model: &str, prefix: &'static str, x: i32, y: i32) -> u32 {
        let k = match self.names.iter_mut().find(|u| u.0 == prefix) {
            Some(u) => {
                u.1 += 1;
                u.1
            }
            None => {
                self.names.push((prefix, 1));
                1
            }
        };
        let id = self.next;
        self.next += 1;
        self.p.devices.push(Device { id, name: format!("{}{}", prefix, k), model: String::from(model), x, y, ..Device::default() });
        id
    }

    fn name(&self, id: u32) -> String {
        self.p.device(id).map(|d| d.name.clone()).unwrap_or_default()
    }

    fn config(&mut self, id: u32, cfg: String) {
        if let Some(d) = self.p.device_mut(id) {
            d.startup_config = cfg;
        }
    }

    fn link(&mut self, a: u32, pa: &str, b: u32, pb: &str) -> Option<u32> {
        self.p.connect(&PortRef { dev: a, port: String::from(pa) }, &PortRef { dev: b, port: String::from(pb) }, None).ok()
    }

    fn reroute(&mut self, id: Option<u32>, route: Route, wps: Vec<Point>) {
        if let Some(l) = id.and_then(|id| self.p.link_mut(id)) {
            l.route = route;
            l.waypoints = wps;
        }
    }
}

/// the devices of a pod
struct PodIds {
    router: u32,
    sw: [u32; 2],
    a: Vec<u32>,
    b: Vec<u32>,
    servers: Vec<u32>,
}

/// an uplink: the core router (index), its port and address, the pod router's port and address
struct Uplink {
    core: usize,
    core_port: String,
    core_addr: String,
    pod_port: String,
    pod_addr: String,
}

/// a project with n devices (clamped to MIN_DEVICES ..= MAX_DEVICES), see the file header
pub fn topology(n: usize) -> Project {
    let n = n.clamp(MIN_DEVICES, MAX_DEVICES);
    let (cores, pods) = plan(n);
    let mut p = Project::new(&format!("Synthetisches Netz mit {} Geräten", n), n as u64);
    p.description = String::from(
        "Erzeugt für Leistungstests: Kern-Router im Ring, darum Pods aus einem Router, zwei Access-Switches, Hosts und Servern an beiden Switches; statische Routen, keine Schleifen auf Schicht 2.",
    );
    p.layout = Layout { zoom_pm: 0, grid: 20, ..Layout::default() };
    let mut g = Builder { p, names: Vec::new(), next: 1 };
    // ---- where things go: pods above the core row (facing down) and below it (facing up, in
    // reverse order, so the serial ring runs round)
    let top = pods.len().div_ceil(2);
    let bottom = pods.len() - top;
    let width = top as i32 * POD_PITCH - (POD_PITCH - COL_B - BOX_W);
    let place = |k: usize| -> (i32, bool) { if k < top { (k as i32, true) } else { ((bottom - 1 - (k - top)) as i32, false) } };
    // a point of pod k: x from its left, y from its router away from the core
    let at = |k: usize, x: i32, y: i32| -> (i32, i32) {
        let (col, above) = place(k);
        let px = col * POD_PITCH + x;
        if above { (px, -BAND - BOX_H - y) } else { (px, BAND + y) }
    };
    let core_x = |i: usize| ((2 * i as i64 + 1) * width as i64 / (2 * cores as i64)) as i32 - BOX_W / 2;
    let centre = |x: i32| x + BOX_W / 2;
    // ---- devices: the core, then pod after pod
    let core_ids: Vec<u32> = (0..cores).map(|i| g.add("jf-r8", "CR", core_x(i), -BOX_H / 2)).collect();
    let mut ids: Vec<PodIds> = Vec::with_capacity(pods.len());
    for (k, pod) in pods.iter().enumerate() {
        let (x, y) = at(k, COL_MID, 0);
        let router = g.add("jf-r4", "R", x, y);
        let (ax, ay) = at(k, COL_SW_A, SWITCH_AT);
        let (bx, by) = at(k, COL_SW_B, SWITCH_AT);
        let sw = [g.add("jf-s24p", "SW", ax, ay), g.add("jf-s24p", "SW", bx, by)];
        let row = |i: usize| HOSTS_AT + i as i32 * ROW;
        // a printer heads the hosts of switch A, a camera those of switch B
        let a: Vec<u32> = (0..pod.a)
            .map(|i| {
                let (x, y) = at(k, COL_A, row(i));
                if i == 0 && pod.a > 1 { g.add("printer", "Printer", x, y) } else { g.add("pc", "PC", x, y) }
            })
            .collect();
        let servers: Vec<u32> = (0..pod.servers)
            .map(|i| {
                let (x, y) = at(k, COL_MID, row(i));
                g.add("server-1u", "SRV", x, y)
            })
            .collect();
        let b: Vec<u32> = (0..pod.b)
            .map(|i| {
                let (x, y) = at(k, COL_B, row(i));
                if i == 0 && pod.b > 1 { g.add("iot-camera", "Cam", x, y) } else { g.add("pc", "PC", x, y) }
            })
            .collect();
        ids.push(PodIds { router, sw, a, b, servers });
    }
    // ---- uplinks: every pod's first to the nearest core router with a port left, then every
    // pod's second to the next nearest
    let mut taken = alloc::vec![0usize; cores];
    let mut ups: Vec<Vec<Uplink>> = (0..pods.len()).map(|_| Vec::new()).collect();
    let mut u = 0usize;
    for j in 0..2 {
        for (k, v) in ups.iter_mut().enumerate() {
            let rx = centre(at(k, COL_MID, 0).0);
            let mut order: Vec<usize> = (0..cores).collect();
            order.sort_by_key(|&i| ((centre(core_x(i)) - rx).abs(), i));
            if let Some(i) = order.into_iter().find(|&i| taken[i] < CORE_PORTS && v.iter().all(|x| x.core != i)) {
                let (core_addr, pod_addr) = net30(16, u);
                u += 1;
                let (core_port, pod_port) = (format!("TenGigabitEthernet0/0/{}", taken[i]), format!("TenGigabitEthernet0/1/{}", j));
                v.push(Uplink { core: i, core_port, core_addr, pod_port, pod_addr });
                taken[i] += 1;
            }
        }
    }
    // ---- the core ring: core i's Hu0/1/0 to core i + 1's Hu0/1/1 (.1 and .2 of cable i)
    let ring: Vec<(usize, usize)> = if cores > 2 { (0..cores).map(|i| (i, (i + 1) % cores)).collect() } else { alloc::vec![(0, 1)] };
    let ring_addr: Vec<(String, String)> = (0..ring.len()).map(|r| net30(17, r)).collect();
    // ---- between pods: the serial ring, the pairs across the core
    let mut between: Vec<(usize, &str, usize, &str)> = Vec::new();
    match pods.len() {
        0 | 1 => {}
        2 => between.push((0, "Serial0/2/0", 1, "Serial0/2/1")),
        m => between.extend((0..m).map(|k| (k, "Serial0/2/0", (k + 1) % m, "Serial0/2/1"))),
    }
    between.extend((0..bottom).map(|c| (c, "GigabitEthernet0/0/2", top + bottom - 1 - c, "GigabitEthernet0/0/3")));
    let between_addr: Vec<(String, String)> = (0..between.len()).map(|s| net30(18, s)).collect();
    // ---- configurations
    let net = |k: usize, side: usize, host: usize| format!("10.{}.{}.{}", k + 1, side + 1, host);
    for (k, pod) in ids.iter().enumerate() {
        let mut cfg = format!("hostname {}\n!\n", g.name(pod.router));
        iface(&mut cfg, "GigabitEthernet0/0/0", &net(k, 0, 1), MASK24);
        iface(&mut cfg, "GigabitEthernet0/0/1", &net(k, 1, 1), MASK24);
        for x in &ups[k] {
            iface(&mut cfg, &x.pod_port, &x.pod_addr, MASK30);
        }
        for (s, &(a, pa, b, pb)) in between.iter().enumerate() {
            if a == k {
                iface(&mut cfg, pa, &between_addr[s].0, MASK30);
            }
            if b == k {
                iface(&mut cfg, pb, &between_addr[s].1, MASK30);
            }
        }
        for (j, x) in ups[k].iter().enumerate() {
            let floating = if j == 0 { "" } else { " 10" };
            cfg.push_str(&format!("ip route 0.0.0.0 0.0.0.0 {}{}\n", x.core_addr, floating));
        }
        cfg.push_str("!\nend\n");
        g.config(pod.router, cfg);
        for sw in pod.sw {
            let cfg = format!("hostname {}\n!\nend\n", g.name(sw));
            g.config(sw, cfg);
        }
        for (side, list) in [(0usize, &pod.a), (1, &pod.b)] {
            for (i, &h) in list.iter().enumerate() {
                let cfg = format!("hostname {}\nip addr add {}/24 dev eth0\nip route add default via {}\n", g.name(h), net(k, side, 10 + i), net(k, side, 1));
                g.config(h, cfg);
            }
        }
        for (i, &h) in pod.servers.iter().enumerate() {
            let cfg = format!(
                "hostname {}\nip addr add {}/24 dev eth1\nip addr add {}/24 dev eth2\nip route add default via {}\n",
                g.name(h),
                net(k, 0, 100 + i),
                net(k, 1, 100 + i),
                net(k, 0, 1)
            );
            g.config(h, cfg);
        }
    }
    for (i, &core) in core_ids.iter().enumerate() {
        let mut cfg = format!("hostname {}\n!\n", g.name(core));
        for x in ups.iter().flatten().filter(|x| x.core == i) {
            iface(&mut cfg, &x.core_port, &x.core_addr, MASK30);
        }
        for (r, &(a, b)) in ring.iter().enumerate() {
            if a == i {
                iface(&mut cfg, "HundredGigE0/1/0", &ring_addr[r].0, MASK30);
            }
            if b == i {
                iface(&mut cfg, "HundredGigE0/1/1", &ring_addr[r].1, MASK30);
            }
        }
        // every pod: over its own uplink, else the shorter way round the ring to one of its
        // core routers (clockwise, towards higher numbers, on a tie)
        for (k, v) in ups.iter().enumerate() {
            let via = match v.iter().find(|x| x.core == i) {
                Some(x) => x.pod_addr.clone(),
                None if cores == 2 => if i == 0 { ring_addr[0].1.clone() } else { ring_addr[0].0.clone() },
                None => {
                    let way = v.iter().map(|x| {
                        let (cw, ccw) = ((x.core + cores - i) % cores, (i + cores - x.core) % cores);
                        (cw.min(ccw), cw > ccw)
                    });
                    match way.min() {
                        Some((_, true)) => ring_addr[(i + cores - 1) % cores].0.clone(),
                        _ => ring_addr[i].1.clone(),
                    }
                }
            };
            cfg.push_str(&format!("ip route 10.{}.0.0 255.255.0.0 {}\n", k + 1, via));
        }
        cfg.push_str("!\nend\n");
        g.config(core, cfg);
    }
    // ---- cables
    g.p.next_id = g.next;
    for (r, &(a, b)) in ring.iter().enumerate() {
        let id = g.link(core_ids[a], "HundredGigE0/1/0", core_ids[b], "HundredGigE0/1/1");
        if cores > 2 && r == cores - 1 {
            // the cable that closes the ring runs below the row
            let y = BAND / 2 + BOX_H / 4;
            g.reroute(id, Route::Ortho, alloc::vec![Point { x: centre(core_x(a)), y }, Point { x: centre(core_x(b)), y }]);
        }
    }
    for (k, pod) in ids.iter().enumerate() {
        for x in &ups[k] {
            g.link(core_ids[x.core], &x.core_port, pod.router, &x.pod_port);
        }
        g.link(pod.router, "GigabitEthernet0/0/0", pod.sw[0], "GigabitEthernet1/0/24");
        g.link(pod.router, "GigabitEthernet0/0/1", pod.sw[1], "GigabitEthernet1/0/24");
        let mut port = [0usize; 2];
        let mut next = |side: usize| {
            port[side] += 1;
            format!("GigabitEthernet1/0/{}", port[side])
        };
        for &h in &pod.a {
            let sp = next(0);
            g.link(h, "eth0", pod.sw[0], &sp);
        }
        for &h in &pod.servers {
            let (sa, sb) = (next(0), next(1));
            g.link(h, "eth1", pod.sw[0], &sa);
            g.link(h, "eth2", pod.sw[1], &sb);
        }
        for &h in &pod.b {
            let sp = next(1);
            g.link(h, "eth0", pod.sw[1], &sp);
        }
    }
    // the cables across the core bend round the core routers
    for &(a, pa, b, pb) in &between {
        let id = g.link(ids[a].router, pa, ids[b].router, pb);
        if place(a).1 == place(b).1 {
            continue;
        }
        let mid = (centre(at(a, COL_MID, 0).0) + centre(at(b, COL_MID, 0).0)) / 2;
        let free = |x: i32| (0..cores).all(|i| x < core_x(i) - BOX_W / 2 || x > core_x(i) + BOX_W + BOX_W / 2);
        let x = [0, 120, -120, 240, -240, 360].iter().map(|d| mid + d).find(|&x| free(x)).unwrap_or(mid);
        g.reroute(id, Route::Curve, alloc::vec![Point { x, y: 0 }]);
    }
    let mut p = g.p;
    p.next_id = p.next_id.max(p.max_id() + 1);
    p
}

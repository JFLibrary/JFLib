// jfn_sim -- JFNetworker's simulator: a discrete-event engine over virtual time with devices,
// ports, cables, switching, IPv4 (ARP, routing, ICMP, ping, traceroute), UDP and TCP with
// sockets, a virtual OS with services (DHCP, DNS, syslog, NTP, TCP echo, discard and a test
// client; an HTTP server, client and load generator) and the command lines.  Deterministic: no
// clock and no OS
// randomness here -- the project seed and the order of events fix every run.  It never names
// plat, so it runs inline on the UI thread, in headless tests and on the web target alike.
//
//   kernel   Engine: events ordered by (time, sequence), step / run_until / run_idle, timers
//   node     Node, Port, PortCounters; the MAC addresses of ports
//   link     cables at run time: serialisation (with preamble, gap and minimum frame), delay,
//            jitter, loss, queues, link state; send / arrive
//   bridge   learning bridge with VLANs (access, trunk with native VLAN), FDB with aging; hub;
//            the bridge's own interfaces (SVIs)
//   ip       IPv4 interfaces (ports, 802.1Q subinterfaces, SVIs, loopbacks), addresses, ARP
//            cache, static routes, RIB
//   l3       ARP, local delivery, forwarding with TTL, fragmentation, reassembly, ICMP errors
//            (RFC 1812 rules, rate limit); serial lines with Cisco HDLC
//   jobs     ping and traceroute over virtual time (ping_host: by name)
//   os       the OS of a device: UDP sockets, Service trait and OsApi, inbox, log with syslog
//            hosts, host names, the device clock
//   resolver lookups with search domains, timeouts, retries and a TTL cache (resolv.conf)
//   dhcpc    DHCP client (dhclient on Linux, "ip address dhcp" on IOS)
//   dhcpd    DHCP server with pools, exclusions, ping check, bindings; the relay agent
//   dnsd     authoritative DNS server with zones; forwarding to the resolver
//   syslogd  syslog server
//   ntpd     NTP client and server on the device clock
//   svcact   the actions of services and services from project files
//   tcp      TCP after RFC 9293: states, windows, RTO (RFC 6298), Reno (RFC 5681), delayed
//            ACKs, Nagle, zero-window probes, RST rules, TIME-WAIT, keepalive; sockets for
//            services; what ss and "show tcp brief" show
//   tcpsvc   TCP echo and discard servers and a client that sends N bytes (for trying TCP)
//   http     HTTP/1.1 on the sockets: Serve (the server side: keep-alive, pipelining,
//            timeouts), Agent (the client side: kept-alive connections, retries, timeouts),
//            Url, Wheel (many deadlines on one timer key)
//   httpsvc  the HTTP services: web server (routes, time, workers, error rate), client, load
//            generator (open and closed loop, mix, think time, ramp) with latency windows
//   trace    Hop (a frame on a cable), DevEvent + Note (what a device decided), Trace rings
//   action   Action: topology edits, power, cables, impairments, port state, VLANs, frames,
//            IPv4 interfaces, routes and ARP entries
//   view     View for the canvas: device, port and link state (LEDs), frames in flight; the
//            interfaces, routing table and ARP cache of a device; notes as sentences; for the
//            simulation mode the frames on the cables at a moment, drops, event list rows and
//            what a device did with a frame (packet tab)
//   pcap     the recording (all, filtered, one cable, one device) as a pcapng file
//   problems Engine::problems(): duplicate addresses, gateways, masks, VLANs on trunks,
//            cables, loops without spanning tree, interfaces shut down
//   driver   Inline: realtime pacing and simulation stepping for the UI thread; stepping by
//            protocols and devices, back along the recording and live again; Report
//   load     Engine::from_project (the startup configurations run through the command lines)
//   cli      command lines: IOS-like for network devices, a Linux shell for hosts; running and
//            startup configuration; ping and traceroute as console commands; debug and logs;
//            curl, nc, ss and top as console programs on the sockets (cli/tools)
//   runner   .scn scenarios against an engine: a report with virtual times only
//   pdu      PDUs of the simulation mode: a service sends UDP datagrams (with a TTL of their
//            own) or opens TCP connections, once or again and again
//   disk     the disk of a device: access times (latency, throughput, iops) and capacity
//   fault    fault injection with marks on the timeline: cables pulled or worse, power off,
//            services stopped, slow disks, partitions; each one heals exactly what it did
//   lab      the actions of the lab and the views of its services for the user interface
//   lb       load balancer: HTTP or TCP, round robin, least connections, weighted, consistent
//            hashing; health checks, draining, numbers per backend
//   kv       key-value store: hash ring with virtual nodes, replicas, a leader per key, read
//            and write quorums, read repair, heartbeats; KvClient for other services
//   cache    HTTP cache in front of a backend: LRU with a time to live, hits and misses
//   mq       message queue (topics, at-least-once, acknowledgements, redelivery, dead
//            letters) and the workers that take from it
//   scriptsvc scripts as services: a .jfs program (lib/script) driven by events, its fuel the
//            CPU time of a single-threaded program; the host API (log, timers, net, dns, http,
//            disk, metrics); a new source that keeps the disk and numbers (ScriptSource)
//   series   time series on virtual time: every service's metrics (rates of counters, gauges,
//            latency quantiles per interval) and the cables' traffic, KPIs, fault marks for
//            charts, values over a span for scenarios, CSV
//   goal     an expectation against the engine now (judge: the runner's and the lab's, with a
//            report line and a reason as a Msg); LabRun: a lab's goals over a run
//   stp      spanning tree on the bridges: settings, the protocol that runs, BPDUs in and out,
//            port states for the bridge, MAC flushing, BPDU guard (off unless configured)
//   stp98    STP: the spanning tree algorithm of IEEE 802.1D-1998 clause 8
//   rstp     RSTP: the state machines of IEEE 802.1D-2004 clause 17 (802.1w)
//   rip      RIP (RFC 2453, version 1 after RFC 1058) as the service "rip" of IOS-like routers
//            and L3 switches: requests, periodic and triggered updates, split horizon with
//            poisoned reverse, timeout, garbage collection, holddown, equal-cost paths; its
//            routes in the RIB with distance 120; RipCfg / RipSet (Action::Rip)
//   acl      IPv4 access lists of IOS-like devices: standard and extended, numbered and named,
//            sequence numbers, remarks, hit counters; inbound and outbound on interfaces
//   nat      address translation of IOS-like devices: inside and outside interfaces, pools,
//            PAT, static addresses and ports; TCP, UDP, ICMP queries and errors (RFC 5508),
//            fragments, timeouts; the order of lists, routing and translation
//   ip6      the IPv6 part of a device (IpStack::v6): addresses with their states and lifetimes,
//            the neighbor cache, default routers, static routes, the RIB; Ip6Set (Action::Ip6);
//            off until configured, so projects without IPv6 send no IPv6 frame
//   nd       Neighbor Discovery (RFC 4861), DAD and SLAAC (RFC 4862), router advertisements,
//            the IPv6 timers; the views ip6 / ip6_routes / ip6_neighbors / addrs6
//   l3v6     IPv6 input, local delivery, forwarding with the hop limit, output with source
//            address selection and fragmentation by the sender, ICMPv6 errors with their limits
//   jobs6    ping and traceroute over IPv6 (JobKind::Ping6 / Trace6)
//   trace6   Note6: the decisions of IPv6 as notes (Note::V6) with their German sentences
//   ospf     OSPF version 2 (RFC 2328) as the service "ospf" of IOS-like routers and L3
//            switches: interfaces with DR / BDR election, neighbours from Down to Full with the
//            database exchange, flooding with acknowledgements and retransmission, aging and
//            refresh, SPF per area, inter-area routes over an ABR, AS-external routes
//            (redistribution, default route); its routes in the RIB with distance 110;
//            OspfCfg / OspfSet (Action::Ospf)
//   assess   the grading of an assessment (A1): its items against a project and its engine
//            (points, a percentage, what each item found), config lines normalised on a
//            scratch device; for authors the network captured as a target, the target's
//            settings as candidate items, the start network without what the items check
//   wlan     WLAN on layer 2: access points offer networks, stations scan, authenticate,
//            associate and run the 4-way handshake; associations as radio links the simulator
//            makes and drops, the AP's bridge to its stations (VLANs, client isolation, guest
//            networks), signal and rate from the distance, roaming, beacon loss, back-off;
//            Action::Move / Wlan / Scale; the views wlan_assocs / wlan_station / wlan_survey
//   wlankey  the keys of WPA2-Personal and WPA3-Personal (PMK, PTK, MICs, the key wrap of the
//            group key) and the EAPOL-Key messages of the 4-way handshake
//   wlantrace WNote: the decisions of WLAN as notes (Note::Wlan) with their German sentences
//   files    the files of a device at run time (Os::files, jfn_model::files::Store): reads and
//            writes of services and command lines, Report::Files keeps the project in step,
//            Action::Files gives the project's records back (load, undo)
//   xfertrace XNote: the decisions of TFTP and FTP as notes (Note::Xfer) with their German
//            sentences
//   tftpd    the TFTP server (RFC 1350, options of RFC 2347-2349): a port per transfer,
//            lock-step with retransmission, errors, quota; "tftp-server"
//   tftpc    the TFTP client of the consoles (busybox-like tftp, IOS-like copy ... tftp:)
//   ftpd     the FTP server (RFC 959, EPSV / EPRT of RFC 2428): users, anonymous login,
//            passive and active data connections, LIST, NLST, RETR, STOR ...; "ftp-server"
//   ftpc     the FTP client of the consoles (curl with ftp:// URLs, IOS-like copy ... ftp:)
//
// UDP and TCP run over both families (os.rs: sockets on ANY take both, ANY4 IPv4 only); the
// services, the resolver (A and AAAA in the order of RFC 6724) and the tools use IPv6 where a
// device has it; dig, host and nslookup are console programs too (cli/names.rs).
//
// Ids: devices and links keep the project's numeric ids; inside the engine they live in slots
// that are never reused, so events of removed devices and cables simply find nothing.
//
// Texts for the app are German; each also comes as a jfn_model::msg::Msg (a template the app's
// catalogue src/locales/engine.tsv translates): Note::message / Engine::message (describe),
// EventRow::msg, Problem::msg, ActErr::msg (Refused carries a Msg), SvcView::status_msg
// (Service::status_msg), Fail::msg, PduSender::error_msg, TcpClient::error_msg, the *_msg
// variants of the socket calls and of from_def, Verdict::reason (goal.rs).  The runner's reports
// stay German only.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod action;
pub mod bridge;
pub mod load;
pub mod driver;
pub mod ip;
pub mod jobs;
pub mod kernel;
pub mod l3;
pub mod link;
pub mod node;
pub mod pcap;
pub mod problems;
pub mod trace;
pub mod view;
pub mod dhcpc;
pub mod dhcpd;
pub mod dnsd;
pub mod ntpd;
pub mod os;
pub mod resolver;
pub mod svcact;
pub mod syslogd;
pub mod cli;
pub mod runner;
pub mod tcp;
pub mod tcpsvc;
pub mod http;
pub mod httpsvc;
pub mod pdu;
pub mod disk;
pub mod fault;
pub mod lab;
pub mod lb;
pub mod kv;
pub mod cache;
pub mod mq;
pub mod scriptsvc;
pub mod series;
pub mod goal;
pub mod stp;
pub mod stp98;
pub mod rstp;
pub mod rip;
pub mod acl;
pub mod nat;
pub mod ip6;
pub mod nd;
pub mod l3v6;
pub mod jobs6;
pub mod trace6;
pub mod ospf;
pub mod assess;
pub mod wlan;
pub mod wlankey;
pub mod wlantrace;
pub mod files;
pub mod xfertrace;
pub mod tftpd;
pub mod tftpc;
pub mod ftpd;
pub mod ftpc;

pub use action::{ActErr, Action};
pub use bridge::{Bridge, PortMode, VlanSet};
pub use driver::{Cmd, Inline, Mode, Tick};
pub use ip::{IcmpLimit, IfKind, Iface, IpStack, Route, RouteSrc};
pub use jobs::{IcmpKind, Job, JobId, JobKind, PingEv, PingOpts, TraceHop, TraceOpts, TraceProbe};
pub use kernel::{Engine, Frame, Ns, RunOut, MS, SEC, US};
pub use node::{Node, Port, PortCounters};
pub use pcap::{Pcap, PcapOpts, PcapScope};
pub use problems::{Problem, ProblemKind, Severity};
pub use trace::{DevEvent, Hop, HopDrop, HopId, Note, Trace};
pub use trace::Filter;
pub use view::{ArpRow, DevView, IfView, Led, LinkView, PortView, View};
pub use view::{DevStep, DropMark, EventRow, InFlight};
pub use dhcpc::{ClientEv, ClientState, DhcpClient, Lease, LeaseView};
pub use dhcpd::{BindState, Binding, Detect, Dhcpd, Pool, PoolCfg, PoolView};
pub use dnsd::{parse_record, DnsServer, Zone};
pub use ntpd::{Ntp, NtpPeer};
pub use os::{Clock, Ctl, Dgram, LogLine, Metric, OsApi, Service, Sock, SvcEvent, SvcView, EPOCH_UNIX_NS};
pub use resolver::{Answer, CacheRow, LatencyView, Lookup, LookupErr, ResolvConf, Source, Try, TryOut};
pub use syslogd::{SyslogMsg, SyslogServer};
pub use trace::{fmt_ns, DhcpReq, RelayDrop};
pub use bridge::{SwMode, SwSet};
pub use driver::Report;
pub use runner::{run_scenario, Outcome};
pub use tcp::{RstWhy, SockOpts, TcpCfg, TcpCounters, TcpEnd, TcpRow, TcpState, TcpStats, TcpTimer};
pub use tcpsvc::{CloseMode, TcpClient, TcpDiscard, TcpEcho};
pub use os::ConsoleEv;
pub use http::{Agent, AgentEv, Reply, Serve, ServeCfg, Url};
pub use httpsvc::{HttpClient, HttpLoad, HttpRoute, HttpServer, LoadMode, LoadView, ServerView};
pub use pdu::{PduProto, PduSender, PduSpec};
pub use disk::Disk;
pub use fault::{Fault, FaultMark, LinkOver};
pub use lb::{Algo, BackendView, Lb, LbMode, LbOp, LbView};
pub use kv::{Consistency, Entry, KvClient, KvDone, KvNode, KvView, ReplicaView, Ver};
pub use cache::{Cache, CacheView};
pub use mq::{Queue, QueueView, SlotState, TopicView, Worker, WorkerView};
pub use scriptsvc::{ScriptCfg, ScriptErr, ScriptService, ScriptState, ScriptView};
pub use series::{AggOut, Kpi, LatStat, LatWindow, MetricKind, SeriesCfg, SeriesErr, SeriesKind, SeriesView, ShardLoad, SvcSeries, Unit};
pub use goal::{judge, Found, GoalRun, LabRun, Verdict};
pub use stp::{LinkType, ModeWord, PortRole, PortState, StpPortView, StpSet, TreeView};
pub use rip::{Redist, Rip, RipCfg, RipDrop, RipHow, RipLoss, RipOut, RipSet, RipTimers};
pub use acl::{Ace, Acl, AclKind, AclOp, AclView};
pub use nat::{Nat, NatAct, NatStats, Tmo, XlateRow};
pub use ip6::{Addr6, AddrCfg, AddrState, DefRouter, If6, Ip6, Ip6Set, Limit6, NState, Neigh, NextHop6, OnLink, Origin6, PrefixCfg, RaCfg, Route6, Src6};
pub use nd::{Addr6View, Neigh6Row};
pub use jobs6::{Icmp6Kind, Ping6Ev, Ping6Job, Ping6Opts, Trace6Hop, Trace6Job, Trace6Probe};
pub use trace6::{Note6, RaWhy};
pub use ospf::{IfState, LsaDrop, LsaRef, LsuWhy, NbrEvent, NbrState, NetType, OrigWhy, Ospf, OspfCfg, OspfDrop, OspfSet, RtChange, RtKind};
pub use resolver::Family;
pub use os::{ANY, ANY4};
pub use assess::{Cand, Graded, Grader, Lines, Score};
pub use wlan::{AssocView, Bss, StaState, StaView};
pub use wlantrace::WNote;
pub use xfertrace::XNote;
pub use tftpd::TftpServer;
pub use tftpc::{Sink, Style, TftpClient};
pub use ftpd::{FtpServer, FtpUser};
pub use ftpc::FtpProgram;

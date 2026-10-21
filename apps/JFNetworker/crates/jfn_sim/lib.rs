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
//
// Ids: devices and links keep the project's numeric ids; inside the engine they live in slots
// that are never reused, so events of removed devices and cables simply find nothing.
//
// Texts for the app are German; each also comes as a jfn_model::msg::Msg (a template the app's
// catalogue src/locales/engine.tsv translates): Note::message / Engine::message (describe),
// EventRow::msg, Problem::msg, ActErr::msg (Refused carries a Msg), SvcView::status_msg
// (Service::status_msg), Fail::msg, PduSender::error_msg, TcpClient::error_msg, the *_msg
// variants of the socket calls and of from_def.  The runner's reports stay German only.
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

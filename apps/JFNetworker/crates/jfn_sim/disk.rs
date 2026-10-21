// disk -- the disk of a device (Os::disk): how long reads and writes take and how much fits;
// the data itself stays in the services that store it (kv.rs, mq.rs)
//
//   Disk { latency, bps, iops, capacity }   an access occupies the disk for its transfer time
//                                           (at least 1/iops), one after the other; its latency
//                                           comes on top and overlaps with the next accesses
//                                           (a queue of a flash disk)
//   Disk::access(now, bytes) -> Ns          when an access that starts now is done
//   Disk::text() / msg()                    "12 KiB von 1 GiB, 100 µs" (the console's German
//                                           line; as a message for the app's catalogue)
//   OsApi::disk_read(bytes) / disk_write(bytes, grow) / disk_free(bytes) / disk() /
//   disk_setup(latency, bps, iops, capacity); setup_of(config) reads {"disk": {...}}
//
// A "slow disk" fault (fault.rs) adds latency and multiplies the transfer time; it acts on
// every access that starts while it lasts.  Capacity is counted per service (a removed service
// gives its bytes back); a write that does not fit fails, the service answers it as full.
// Power off keeps what is stored, as a disk does: accesses that had not finished are the
// services' business (they lose what they had not stored yet).

use alloc::collections::BTreeMap;

use jfn_model::msg;
use jfn_model::msg::{dur, Msg};

use crate::kernel::{Ns, SEC, US};
use crate::os::OsApi;

/// the disk of a device
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    /// latency of one access
    pub latency: Ns,
    /// throughput in bytes per second
    pub bps: u64,
    /// accesses per second at most (0: no limit)
    pub iops: u64,
    /// bytes it holds at most
    pub capacity: u64,
    /// bytes stored per service id
    used: BTreeMap<u32, u64>,
    /// the transfer channel is busy until then
    busy_until: Ns,
    /// a slow-disk fault: extra latency and the transfer time in per mille (1000: normal)
    pub slow_extra: Ns,
    pub slow_pm: u32,
    pub reads: u64,
    pub writes: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    /// time the channel was busy
    pub busy_ns: u64,
    /// the longest wait of an access for the channel
    pub max_wait: Ns,
    /// writes refused because the disk was full
    pub full: u64,
}

impl Default for Disk {
    fn default() -> Disk {
        // a server's flash disk
        Disk::new(100 * US, 500_000_000, 50_000, 1 << 30)
    }
}

impl Disk {
    pub fn new(latency: Ns, bps: u64, iops: u64, capacity: u64) -> Disk {
        Disk {
            latency,
            bps: bps.max(1),
            iops,
            capacity,
            used: BTreeMap::new(),
            busy_until: 0,
            slow_extra: 0,
            slow_pm: 1000,
            reads: 0,
            writes: 0,
            bytes_read: 0,
            bytes_written: 0,
            busy_ns: 0,
            max_wait: 0,
            full: 0,
        }
    }
    /// bytes stored by every service
    pub fn used(&self) -> u64 {
        self.used.values().sum()
    }
    /// bytes stored by one service
    pub fn used_by(&self, svc: u32) -> u64 {
        self.used.get(&svc).copied().unwrap_or(0)
    }
    /// the transfer time of `bytes` (at least one slot of the iops limit), slowed by a fault
    fn transfer(&self, bytes: u64) -> Ns {
        let t = (bytes as u128 * SEC as u128 / self.bps as u128) as Ns;
        let slot = if self.iops > 0 { SEC / self.iops } else { 0 };
        (t.max(slot) as u128 * self.slow_pm as u128 / 1000) as Ns
    }
    /// an access of `bytes` that starts now: when it is done
    pub fn access(&mut self, now: Ns, bytes: u64) -> Ns {
        let start = self.busy_until.max(now);
        let t = self.transfer(bytes);
        self.busy_until = start + t;
        self.busy_ns += t;
        self.max_wait = self.max_wait.max(start - now);
        start + t + self.latency + self.slow_extra
    }
    /// accesses waiting for the channel now (in time)
    pub fn backlog(&self, now: Ns) -> Ns {
        self.busy_until.saturating_sub(now)
    }
    /// counts `bytes` more for a service; false when they do not fit
    pub fn reserve(&mut self, svc: u32, bytes: u64) -> bool {
        if self.used() + bytes > self.capacity {
            self.full += 1;
            return false;
        }
        *self.used.entry(svc).or_insert(0) += bytes;
        true
    }
    /// gives back `bytes` of a service
    pub fn free(&mut self, svc: u32, bytes: u64) {
        if let Some(u) = self.used.get_mut(&svc) {
            *u = u.saturating_sub(bytes);
            if *u == 0 {
                self.used.remove(&svc);
            }
        }
    }
    /// a removed service gives back everything
    pub fn forget(&mut self, svc: u32) {
        self.used.remove(&svc);
    }
    /// the one-line state for status texts: "12 KiB von 1 GiB, 100 µs"
    pub fn text(&self) -> alloc::string::String {
        let slow = if self.slow_extra > 0 || self.slow_pm != 1000 { " (verlangsamt)" } else { "" };
        alloc::format!("{} von {}, {}{}", size_text(self.used()), size_text(self.capacity), crate::trace::fmt_ns((self.latency + self.slow_extra) as i64), slow)
    }
    /// text() as a message for the app's catalogue (its German text is text())
    pub fn msg(&self) -> Msg {
        let lat = dur((self.latency + self.slow_extra) as i64);
        if self.slow_extra > 0 || self.slow_pm != 1000 {
            msg!("{} von {}, {} (verlangsamt)", size_text(self.used()), size_text(self.capacity), lat)
        } else {
            msg!("{} von {}, {}", size_text(self.used()), size_text(self.capacity), lat)
        }
    }
}

/// "512 B", "12 KiB", "3 MiB", "1 GiB" (rounded down)
pub fn size_text(b: u64) -> alloc::string::String {
    if b >= 1 << 30 {
        alloc::format!("{} GiB", b >> 30)
    } else if b >= 1 << 20 {
        alloc::format!("{} MiB", b >> 20)
    } else if b >= 1 << 10 {
        alloc::format!("{} KiB", b >> 10)
    } else {
        alloc::format!("{} B", b)
    }
}

/// the settings of a disk in a service's configuration: {"latency_us": 100, "mbps": 500,
/// "iops": 50000, "capacity_mb": 1024} (every member may be left out)
pub fn setup_of(c: &agent_core::jmap::Value) -> Option<(Ns, u64, u64, u64)> {
    let d = c.get("disk")?;
    let def = Disk::default();
    let lat = d.get("latency_us").and_then(|v| crate::httpsvc::decimal(v, US)).unwrap_or(def.latency);
    let bps = d.get("mbps").and_then(|v| crate::httpsvc::decimal(v, 1_000_000)).unwrap_or(def.bps).max(1);
    let iops = d.get("iops").and_then(|v| v.as_u64()).unwrap_or(def.iops);
    let cap = d.get("capacity_mb").and_then(|v| crate::httpsvc::decimal(v, 1 << 20)).unwrap_or(def.capacity);
    Some((lat, bps, iops, cap))
}

impl OsApi<'_> {
    /// the device's disk
    pub fn disk(&self) -> Option<&Disk> {
        self.eng.os(self.ni).map(|o| &o.disk)
    }
    /// sets up the device's disk (latency, bytes per second, accesses per second, capacity)
    pub fn disk_setup(&mut self, latency: Ns, bps: u64, iops: u64, capacity: u64) {
        if let Some(o) = self.eng.os_mut(self.ni) {
            o.disk.latency = latency;
            o.disk.bps = bps.max(1);
            o.disk.iops = iops;
            o.disk.capacity = capacity;
        }
    }
    /// a read of `bytes`: when it is done
    pub fn disk_read(&mut self, bytes: u64) -> Ns {
        let now = self.eng.now;
        match self.eng.os_mut(self.ni) {
            Some(o) => {
                o.disk.reads += 1;
                o.disk.bytes_read += bytes;
                o.disk.access(now, bytes)
            }
            None => now,
        }
    }
    /// a write of `bytes` that stores `grow` more bytes for this service (negative: frees
    /// them): when it is done, or None when the disk is full
    pub fn disk_write(&mut self, bytes: u64, grow: i64) -> Option<Ns> {
        let (now, id) = (self.eng.now, self.id);
        let o = self.eng.os_mut(self.ni)?;
        if grow > 0 && !o.disk.reserve(id, grow as u64) {
            return None;
        }
        if grow < 0 {
            o.disk.free(id, grow.unsigned_abs());
        }
        o.disk.writes += 1;
        o.disk.bytes_written += bytes;
        Some(o.disk.access(now, bytes))
    }
    /// counts `bytes` more for this service without an access; false when they do not fit
    pub fn disk_reserve(&mut self, bytes: u64) -> bool {
        let id = self.id;
        self.eng.os_mut(self.ni).is_some_and(|o| o.disk.reserve(id, bytes))
    }
    /// gives back bytes this service stored (a write that was lost, a deleted entry)
    pub fn disk_free(&mut self, bytes: u64) {
        let id = self.id;
        if let Some(o) = self.eng.os_mut(self.ni) {
            o.disk.free(id, bytes);
        }
    }
}

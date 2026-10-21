// linux_x11_xi -- (PN1) X11 pen tablets through XInput 2 (raw protocol)
//                                                              [src/plat/linux/x11_xi.s]
//
//   xi_init()                  QueryExtension "XInputExtension", XIQueryVersion 2.0, the
//                              valuator label atoms ("Abs Pressure", "Abs Tilt X / Y"),
//                              XI_HierarchyChanged on the root window, the device list
//   xi_select(window)          XISelectEvents Motion / ButtonPress / ButtonRelease of the pen
//                              devices on one of our windows (every window)
//   xi_event(ev)               a GenericEvent of XInput: pen samples, device changes
//                              (-> xi_deferred re-reads the list)
//   xi_deferred()              device list again, windows re-selected
//   xi_data(time) -> data      pen data (Event::data) for a core pointer event of that time,
//                              0 = a mouse event
//   xi_parse_devices(reply) -> pens found in an XIQueryDevice reply
//
// The core events stay as they are (linux_x11_input.rs): XInput 2 events are selected only
// for the slave devices that have an "Abs Pressure" valuator, so the server keeps delivering
// the master's core events.  A slave event comes before the core event of the same device
// event and carries the same time: xi_data hands its pressure (scaled from the valuator range
// to 1..PRESSURE_ONE), PTR_ERASER for devices whose name contains "eraser" (Wacom / libinput /
// Xwayland naming; else PTR_PEN) and the tilt valuators (taken as degrees) to the core event.
// Valuators missing from an event keep their last value.  The stylus buttons arrive as core
// buttons 2 / 3 (the driver's mapping).  Replies and events are not trusted: every length is
// checked against the packet.
//
// Deviation: x11_xi_deferred re-selected on every record, also on missing windows (a read
// through a null record pointer in the assembly); here only existing windows are re-selected.

use super::*;
use crate::{PRESSURE_ONE, PTR_ERASER, PTR_PEN};

const XI_QUERY_VERSION: u32 = 47; // XInput 2 minor opcodes
const XI_SELECT_EVENTS: u32 = 46;
const XI_QUERY_DEVICE: u32 = 48;
const XI_BUTTON_PRESS: u32 = 4; // event types
const XI_MOTION: u32 = 6;
const XI_HIERARCHY: u32 = 11;
const XI_SLAVE_POINTER: u32 = 3; // XIDeviceInfo.use
const XI_VALUATOR: u32 = 2; // class type
const XI_PEN_MASK: u32 = (1 << 4) | (1 << 5) | (1 << 6);
pub(super) const XD_MAX: usize = 8; // pen devices
const VAL_MAX: u32 = 64; // valuator numbers we look at
const NAME_MAX: usize = 128;
const ATOM_NAMES: [&str; 3] = ["Abs Pressure", "Abs Tilt X", "Abs Tilt Y"];

/// name_eraser(name) -> the name contains "eraser" (ASCII, any case)
fn name_eraser(name: &[u8]) -> bool {
    let n = name.len().min(NAME_MAX);
    if n < 6 {
        return false;
    }
    (0..=n - 6).any(|p| (0..6).all(|k| name[p + k] | 32 == b"eraser"[k]))
}

impl X11 {
    /// x11_xi_init: XInput 2 (if the server has it) and the pen devices
    pub(super) fn xi_init(&mut self) {
        self.xi_op = 0;
        let seq = self.query_ext_send(b"XInputExtension");
        let Some(r) = self.reply(seq) else { return };
        if r[8] == 0 || r[9] == 0 {
            return;
        }
        let op = r[9] as u32; // major opcode
        // XIQueryVersion 2.0
        let (o, seq) = self.req(op | XI_QUERY_VERSION << 8, 8);
        wr16(&mut self.out, o, 2);
        wr16(&mut self.out, o + 2, 0);
        let Some(r) = self.reply(seq) else { return };
        if rd16(&r, 8) < 2 {
            return;
        }
        self.xi_op = op;
        // the valuator labels (only if they exist: no device has them otherwise)
        let mut seqs = [0u32; 3];
        for (i, n) in ATOM_NAMES.iter().enumerate() {
            seqs[i] = self.intern_send(n.as_bytes(), true);
        }
        for i in 0..3 {
            self.xi_atoms[i] = self.atom_reply(seqs[i]);
        }
        // device changes (hotplug) on the root window
        let root = self.root;
        self.select_one(root, 0, 1 << XI_HIERARCHY);
        self.xi_query();
    }

    /// select_one(window, device, mask): XISelectEvents with one mask
    fn select_one(&mut self, win: u32, dev: u32, mask: u32) {
        let (o, _) = self.req(self.xi_op | XI_SELECT_EVENTS << 8, 20);
        let b = &mut self.out;
        wr32(b, o, win);
        wr16(b, o + 4, 1); // num_mask
        wr16(b, o + 8, dev); // deviceid
        wr16(b, o + 10, 1); // mask_len (4-byte units)
        wr32(b, o + 12, mask);
    }

    /// xi_query: XIQueryDevice(XIAllDevices) -> the pen device table
    fn xi_query(&mut self) {
        if self.xi_op == 0 {
            return;
        }
        let (o, seq) = self.req(self.xi_op | XI_QUERY_DEVICE << 8, 8);
        wr16(&mut self.out, o, 0); // XIAllDevices
        let Some(r) = self.reply(seq) else { return };
        self.xi_parse_devices(&r);
        if self.debug {
            eprint(&alloc::format!("x11: XInput 2 op {}, {} pen device(s)\n", self.xi_op, self.xi_ndev));
        }
    }

    /// x11_xi_parse_devices(reply) -> pen devices (xi_dev / xi_ndev)
    pub(super) fn xi_parse_devices(&mut self, r: &[u8]) -> u32 {
        self.xi_ndev = 0;
        let len = rd32(r, 4) as usize;
        if len > (X11_IN_CAP - 32) / 4 {
            return 0;
        }
        let end = (len * 4 + 32).min(r.len());
        let mut left = rd16(r, 8);
        let mut p = 32; // first XIDeviceInfo
        while left > 0 {
            left -= 1;
            if p + 12 > end {
                break;
            }
            let usage = rd16(r, p + 2) as u32;
            let mut classes = rd16(r, p + 6);
            let name_len = rd16(r, p + 8) as usize;
            let name = p + 12;
            let mut c = name + ((name_len + 3) & !3); // first class
            if c > end {
                break;
            }
            let (mut pnum, mut pmin, mut pmax, mut txn, mut tyn) = (-1i32, 0i32, 0i32, -1i32, -1i32);
            let mut bad = false;
            while classes > 0 {
                classes -= 1;
                if c + 4 > end {
                    bad = true;
                    break;
                }
                let cl = rd16(r, c + 2) as usize * 4; // class length
                if cl < 4 || c + cl > end {
                    bad = true;
                    break;
                }
                if rd16(r, c) as u32 == XI_VALUATOR && cl >= 44 {
                    let num = rd16(r, c + 6) as u32; // valuator number
                    let label = rd32(r, c + 8);
                    if num < VAL_MAX && label != 0 {
                        if label == self.xi_atoms[0] {
                            pnum = num as i32;
                            pmin = rd32(r, c + 12) as i32; // min.integral
                            pmax = rd32(r, c + 20) as i32; // max.integral
                        } else if label == self.xi_atoms[1] {
                            txn = num as i32;
                        } else if label == self.xi_atoms[2] {
                            tyn = num as i32;
                        }
                    }
                }
                c += cl;
            }
            if bad {
                break;
            }
            // a slave pointer with a pressure valuator of a usable range is a pen
            if usage == XI_SLAVE_POINTER && pnum >= 0 && pmax > pmin && (self.xi_ndev as usize) < XD_MAX {
                let nm = &r[name.min(end)..(name + name_len).min(end)];
                self.xi_dev[self.xi_ndev as usize] = Xd {
                    id: rd16(r, p) as u32,
                    typ: if name_eraser(nm) { PTR_ERASER } else { PTR_PEN },
                    pnum,
                    pmin,
                    pmax,
                    txn,
                    tyn,
                    lastp: 0,
                    tx: 0,
                    ty: 0,
                };
                self.xi_ndev += 1;
            }
            p = c;
        }
        self.xi_ndev
    }

    /// x11_xi_select(window): the pen devices' events on one of our windows
    pub(super) fn xi_select(&mut self, win: u32) {
        if win == 0 || self.xi_op == 0 || self.xi_ndev == 0 {
            return;
        }
        let n = (self.xi_ndev as usize).min(XD_MAX);
        let (o, _) = self.req(self.xi_op | XI_SELECT_EVENTS << 8, n * 8 + 12);
        wr32(&mut self.out, o, win);
        wr16(&mut self.out, o + 4, n as u32); // num_mask
        for k in 0..n {
            let id = self.xi_dev[k].id;
            let b = &mut self.out;
            wr16(b, o + 8 + k * 8, id);
            wr16(b, o + 10 + k * 8, 1);
            wr32(b, o + 12 + k * 8, XI_PEN_MASK);
        }
    }

    /// x11_xi_event(ev): a GenericEvent of XInput (the dispatcher checked its length)
    pub(super) fn xi_event(&mut self, b: &[u8]) {
        let et = rd16(b, 8) as u32; // evtype
        if et == XI_HIERARCHY {
            self.xi_want = true;
            return;
        }
        if et.wrapping_sub(XI_BUTTON_PRESS) > XI_MOTION - XI_BUTTON_PRESS {
            return;
        }
        // the device
        let id = rd16(b, 10) as u32;
        let n = (self.xi_ndev as usize).min(XD_MAX);
        let Some(k) = (0..n).find(|&k| self.xi_dev[k].id == id) else { return };
        // valuator mask and values, inside the event
        let extra = rd32(b, 4) as usize;
        if extra > (X11_IN_CAP - 32) / 4 {
            return;
        }
        let end = (extra * 4 + 32).min(b.len());
        if end < 80 {
            return; // no room for the valuators (the assembly reads past such an event, then
                    // finds the values outside it and returns too)
        }
        let buttons_len = rd16(b, 48) as usize;
        let vlen = rd16(b, 50) as usize; // valuators_len (mask words)
        let mask = buttons_len * 4 + 80;
        let mut vals = mask + vlen * 4; // values (FP3232)
        if vals > end {
            return;
        }
        let bits = ((vlen as u32) << 5).min(VAL_MAX);
        let d = &mut self.xi_dev[k];
        for bit in 0..bits {
            if rd32(b, mask + (bit as usize >> 5) * 4) & 1 << (bit & 31) == 0 {
                continue;
            }
            if vals + 8 > end {
                break;
            }
            let v = rd32(b, vals) as i32; // integral part
            if bit as i32 == d.pnum {
                // (v - min) * PRESSURE_ONE / (max - min), clamped
                let mut a = v.wrapping_sub(d.pmin);
                if a < 0 {
                    a = 0;
                }
                let range = d.pmax.wrapping_sub(d.pmin) as u32;
                let a = (a as u32).min(range);
                d.lastp = ((a as u64 * PRESSURE_ONE as u64) / range as u64) as u32;
            } else if bit as i32 == d.txn {
                d.tx = v;
            } else if bit as i32 == d.tyn {
                d.ty = v;
            }
            vals += 8;
        }
        self.xi_time = rd32(b, 12);
        let d = self.xi_dev[k];
        self.xi_data = crate::evq_pen_data(d.lastp as i32, d.typ, d.tx, d.ty);
        self.xi_valid = true;
    }

    /// x11_xi_data(time) -> pen data of the slave event with that time, 0 = none; a sample
    /// belongs to one core event only
    pub(super) fn xi_data(&mut self, time: u32) -> u32 {
        if !self.xi_valid || time != self.xi_time {
            return 0;
        }
        self.xi_valid = false;
        self.xi_data
    }

    /// x11_xi_deferred: devices changed -- read the list again, select on every window
    pub(super) fn xi_deferred(&mut self) {
        if !self.xi_want {
            return;
        }
        self.xi_want = false;
        self.xi_query();
        for j in 0..PW_MAX {
            let w = self.xw[j].win;
            self.xi_select(w);
        }
        self.flush();
    }
}

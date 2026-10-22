// ids -- widget ids from names: hashing and a stack of scopes
//
//   wid(name) -> u32             const: the id of a name outside every scope, for constants
//                                (const SAVE: u32 = ui::wid("save") == ui.id("save") there)
//   wid_path(&[names]) -> u32    const: the last name inside the scopes of the ones before
//                                (wid_path(&["layers", "add"]) == ui.id("add") after
//                                ui.id_push_str("layers"))
//   ui.id(name) -> u32           a name in the current scope (takes &self: ui.button(ui.id("save"), ..))
//   ui.id_n(n) -> u32            a number in the current scope (the rows of a loop)
//   ui.id_push(n) / ui.id_push_str(name) / ui.id_pop()   open / close a scope: a number (an
//                                index, a document's id) or a name
//   ui.id_scope(n, |ui| ...)     id_push, the closure, id_pop
// The names of hashed ids are kept for the frame in scripts, with jfgui_DEBUG=1 and while the
// inspector is open (it shows them: inspect.rs).
//
// A hashed id is FNV-1a over the bytes (a number: its four bytes, little endian) and a tag
// byte (1 name, 2 number), seeded with the scope, then mixed (murmur3's finaliser) into
// ID_HASH_BASE .. 0x0FFE_FFFF.  Constants written by hand stay below ID_HASH_BASE and never
// meet hashed ids; bits 28..31 stay the library's (part_id); the 64 Ki ids above the range
// leave room for widgets that also take id + 1 + i.  Two names of one pass collide with a
// chance of about 1 : 134 million per pair; scripts and jfgui_DEBUG=1 warn about two names
// with one id and about an id two widgets share (naming the hashed names involved).  The
// stack starts empty in every pass; a scope still open at the end of App::frame is closed
// (a UIWARN the first time).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::Ui;

/// the first hashed id; ids written by hand stay below it
pub const ID_HASH_BASE: u32 = 0x0800_0000;
const SPAN: u32 = 0x07FF_0000; // hashed ids: ID_HASH_BASE .. ID_HASH_BASE + SPAN
const SEED: u32 = 0x811C_9DC5; // FNV-1a offset basis: the scope outside every scope
const PRIME: u32 = 0x0100_0193;
const TAG_NAME: u32 = 1;
const TAG_NUM: u32 = 2;
const DEPTH_MAX: usize = 64; // id scopes inside each other (fixed: nesting; deeper ones warn)

const fn fnv(mut h: u32, b: &[u8], tag: u32) -> u32 {
    let mut i = 0;
    while i < b.len() {
        h = (h ^ b[i] as u32).wrapping_mul(PRIME);
        i += 1;
    }
    (h ^ tag).wrapping_mul(PRIME)
}

// a scope hash -> its id: murmur3's finaliser, into the hashed range
const fn finish(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    ID_HASH_BASE + h % SPAN
}

/// the id of a name outside every scope (the same as ui.id(name) there)
pub const fn wid(name: &str) -> u32 {
    finish(fnv(SEED, name.as_bytes(), TAG_NAME))
}

/// the id of the last name inside the scopes of the names before it (id_push_str each);
/// no names: wid("")
pub const fn wid_path(names: &[&str]) -> u32 {
    if names.is_empty() {
        return wid("");
    }
    let mut h = SEED;
    let mut i = 0;
    while i < names.len() {
        h = fnv(h, names[i].as_bytes(), TAG_NAME);
        i += 1;
    }
    finish(h)
}

/// the scope stack (Ui::ids)
#[derive(Default)]
pub(crate) struct Ids {
    stack: Vec<u32>,
    over: u32, // id_push beyond DEPTH_MAX (their id_pop are ignored)
    /// scripts and jfgui_DEBUG: the hashed ids of this frame -> their names; collisions found
    /// (reported at the end of App::frame)
    names: RefCell<BTreeMap<u32, String>>,
    clash: RefCell<Vec<String>>,
    reported: alloc::collections::BTreeSet<String>, // each collision is reported once
    leak_warned: bool, // scopes left open: reported once
}

impl Ids {
    fn seed(&self) -> u32 {
        self.stack.last().copied().unwrap_or(SEED)
    }
}

impl Ui {
    /// the id of a name in the current scope
    pub fn id(&self, name: &str) -> u32 {
        let id = finish(fnv(self.ids.seed(), name.as_bytes(), TAG_NAME));
        if self.dup_check || self.m.ins.on {
            self.id_note(id, name);
        }
        id
    }

    /// the id of a number in the current scope (the rows of a loop)
    pub fn id_n(&self, n: u32) -> u32 {
        let id = finish(fnv(self.ids.seed(), &n.to_le_bytes(), TAG_NUM));
        if self.dup_check || self.m.ins.on {
            self.id_note(id, &alloc::format!("#{}", n));
        }
        id
    }

    /// open a scope named by a number (an index, a document's id); always pair with id_pop
    pub fn id_push(&mut self, n: u32) {
        let h = fnv(self.ids.seed(), &n.to_le_bytes(), TAG_NUM);
        self.id_enter(h);
    }

    /// open a scope named by a name; always pair with id_pop
    pub fn id_push_str(&mut self, name: &str) {
        let h = fnv(self.ids.seed(), name.as_bytes(), TAG_NAME);
        self.id_enter(h);
    }

    /// close the scope of the last id_push / id_push_str
    pub fn id_pop(&mut self) {
        if self.ids.over > 0 {
            self.ids.over -= 1;
        } else if self.ids.stack.pop().is_none() {
            self.warn("id_pop without id_push");
        }
    }

    /// id_push(n), f(ui), id_pop()
    pub fn id_scope<R>(&mut self, n: u32, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.id_push(n);
        let r = f(self);
        self.id_pop();
        r
    }

    fn id_enter(&mut self, h: u32) {
        if self.ids.stack.len() >= DEPTH_MAX {
            if self.ids.over == 0 {
                self.warn("id_push: scopes nested too deeply");
            }
            self.ids.over += 1;
            return;
        }
        self.ids.stack.push(h);
    }

    // remember the name of a hashed id; another name with the same id is a collision
    fn id_note(&self, id: u32, name: &str) {
        let mut names = self.ids.names.borrow_mut();
        match names.get(&id) {
            Some(old) if old != name => {
                let msg = alloc::format!("ids of \"{}\" and \"{}\" collide ({:#X}): rename one", old, name, id);
                let mut clash = self.ids.clash.borrow_mut();
                if !clash.contains(&msg) {
                    clash.push(msg);
                }
            }
            Some(_) => {}
            None => {
                names.insert(id, String::from(name));
            }
        }
    }

    /// (run_passes, after App::frame) collisions found; scopes the app left open are closed
    pub(crate) fn ids_unwind(&mut self) {
        let clash = core::mem::take(&mut *self.ids.clash.borrow_mut());
        for m in clash {
            if self.ids.reported.insert(m.clone()) {
                self.warn(&m);
            }
        }
        if !self.ids.stack.is_empty() || self.ids.over != 0 {
            let n = self.ids.stack.len() as u32 + self.ids.over;
            self.ids.stack.clear();
            self.ids.over = 0;
            if !self.ids.leak_warned {
                self.ids.leak_warned = true;
                self.warn(&alloc::format!("id_push without id_pop ({} scope(s) open at the end of the frame)", n));
            }
        }
    }

    /// (frame_begin) a new frame: the names of the last one are forgotten
    pub(crate) fn ids_frame_begin(&mut self) {
        if self.dup_check || self.m.ins.on {
            self.ids.names.borrow_mut().clear();
        }
    }

    /// the name of a hashed id seen in this frame (dup_ids)
    pub(crate) fn id_name(&self, id: u32) -> Option<String> {
        self.ids.names.borrow().get(&id).cloned()
    }
}

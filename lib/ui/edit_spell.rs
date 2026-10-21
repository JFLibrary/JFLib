// edit_spell -- spell checking in text boxes (edit.rs includes it as `spellbox`)
//
//   TB_SPELL (cap_flags of textbox, TB_MULTI too)   the box checks its words: a red wavy line
//        under misspelled ones; a right click on one (or the menu key / Shift+F10 with the
//        caret in it) opens a menu: up to 5 suggestions (a click replaces the word, one undo
//        step), "Zum Wörterbuch hinzufügen" (the personal word list), "Ignorieren" (this
//        session).  Password, read-only and digit boxes are never checked.  The app's own
//        context_menu() for the box still opens on words that are not marked.
//        Suggestions take a while with large dictionaries (a long German compound up to a
//        few tenths of a second): Hunspell's are made on a worker (plat::Job), the menu opens
//        at once with "Vorschläge werden gesucht …" and fills when they are there (E_WAKE; a
//        menu opened with a key then focuses the first one).  The system's checker answers
//        on the UI thread.
//   spell_enable(on)            the switch for the whole UI (default on; off: no marks)
//   spell_language(tag)         the language, a BCP 47 tag; "" (default) = the user's
//                               languages (i18n::user_languages), the first with a checker
//   spell_sources(SRC_*)        where checkers come from (spell crate; 0 = the default:
//                               everything in window mode, only app:// in headless runs)
//   spell_suggest_async(on)     suggestions on a worker (default: window mode yes, headless
//                               no, so scripts stay synchronous; a script that switches it on
//                               sees the menu waiting, and its `sync` waits for them)
//   spell_set(speller)          the app's own checker (spell::Speller)
//   speller() -> Option<&mut Speller>   the checker in use (None while loading, or none);
//                               loads it when no box asked for it yet
//   spell_state() -> SPELL_*    OFF (switched off / not asked yet), LOADING, READY, NONE
//   spell_errors(id) -> ranges  the misspelled byte ranges of box id as last checked
//
// Checking: a box checks when its text changed (a hash of it) or the checker or its word
// lists did; word results are cached in the Speller, so typing re-checks only new words.  In
// window mode at most BUDGET new words and BUDGET_US of checking per frame (one word at
// least; the rest on the timer UIT_SPELL: a long German compound may take milliseconds),
// headless all at once.  The word being typed (the caret at its end while typing) is marked
// only once the caret leaves it.  Loading: window mode the system's checker on the UI thread,
// Hunspell files on a worker (plat::Job; no marks until it is read), the personal word list
// in config://spell (spell::personal_path); headless runs read app:// dictionaries at once
// and keep the personal list in memory, so scripts never depend on the machine.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use rt::sync::SpinLock;
use spell::{Dictionary, Source, Speller, Suggester, Token, SRC_ALL, SRC_APP};

use crate::consts::*;
use crate::i18n;
use crate::keys::*;
use crate::Ui;

const BUDGET: u32 = 200; // new words checked per frame (window mode)
const BUDGET_US: u64 = 6000; // ... and the time for them
const SUG_SHOWN: usize = 5;
const BOXES_MAX: usize = 64; // boxes whose results are kept (then the ones not drawn lately go)
const MOD3: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT;

/// the results of one box
#[derive(Default)]
struct BoxSpell {
    hash: u64,
    generation: u32,
    rev: u32,
    toks: Vec<Token>,
    next: usize, // the next token to check
    bad: Vec<(usize, usize)>,
    seen: u32,
}

/// the open suggestion menu
struct Menu {
    id: u32,
    pid: u32, // its popup
    start: usize,
    end: usize,
    word: String,
    sugs: Vec<String>,
    wait: u32,                    // the suggestion job it waits for (0: sugs are there)
    add_row: u32,                 // the id of its row "Zum Wörterbuch hinzufügen"
    kbd: bool,                    // opened with a key: anchored below the word
    rect: [i32; 4],               // the word (px)
    caret: Option<(usize, usize)>, // the box's caret and anchor when it had the focus
}

enum Act {
    Use(usize),
    Add,
    Ignore,
}

/// a dictionary read on a worker: (load number, the dictionary, its language, the personal list)
type Loaded = (u32, Option<Dictionary>, String);

/// suggestions made on a worker: (the job the menu waits for, its result)
type Sugs = (u32, Option<Vec<String>>);

pub(crate) struct S {
    off: bool,
    lang: String,
    sources: u32,
    state: u32,
    speller: Option<Speller>,
    generation: u32,
    load: u32, // number of the load running / last started
    job: Option<plat::Job>,
    slot: Arc<SpinLock<Option<Loaded>>>,
    boxes: BTreeMap<u32, BoxSpell>,
    budget: u32,
    budget_frame: u32,
    budget_until: u64, // plat::time_us() when the frame's checking time is up
    menu: Option<Menu>,
    sug_async: Option<bool>, // spell_suggest_async (None: the default of the mode)
    sug_n: u32,              // number of the suggestion job started last
    sug_jobs: Vec<plat::Job>,
    sug_slot: Arc<SpinLock<Sugs>>,
}

impl Default for S {
    fn default() -> S {
        S {
            off: false,
            lang: String::new(),
            sources: 0,
            state: SPELL_OFF,
            speller: None,
            generation: 0,
            load: 0,
            job: None,
            slot: Arc::new(SpinLock::new(None)),
            boxes: BTreeMap::new(),
            budget: 0,
            budget_frame: 0,
            budget_until: 0,
            menu: None,
            sug_async: None,
            sug_n: 0,
            sug_jobs: Vec::new(),
            sug_slot: Arc::new(SpinLock::new((0, None))),
        }
    }
}

impl S {
    // a new checker is due (language, sources); a load still running is let finish and dropped
    fn reset(&mut self) {
        self.speller = None;
        self.state = SPELL_OFF;
        self.load = self.load.wrapping_add(1);
        self.generation = self.generation.wrapping_add(1);
        self.menu = None;
    }

    // make the suggestions for word on a worker -> the number of the job (plat::wake() when
    // done); a job still busy with an earlier word is let finish, its result dropped
    fn sug_start(&mut self, sg: Suggester, word: &str) -> u32 {
        self.sug_n = self.sug_n.wrapping_add(1).max(1);
        let n = self.sug_n;
        *self.sug_slot.lock() = (n, None);
        let i = match self.sug_jobs.iter().position(|j| j.done()) {
            Some(i) => i,
            None => {
                self.sug_jobs.push(plat::Job::default());
                self.sug_jobs.len() - 1
            }
        };
        let slot = self.sug_slot.clone();
        let w = String::from(word);
        let job = &mut self.sug_jobs[i];
        job.set(1, move |_, _| {
            if slot.lock().0 != n {
                return; // another word was asked for before this one started
            }
            let v = sg.suggest(&w, SUG_SHOWN);
            let mut g = slot.lock();
            if g.0 == n {
                g.1 = Some(v);
            }
        });
        job.start();
        n
    }
}

// FNV-1a of a text (its revision)
fn fnv(b: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &c in b {
        h = (h ^ c as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

impl Ui {
    /// the spell checking switch for every TB_SPELL box (default on)
    pub fn spell_enable(&mut self, on: bool) {
        let s = &mut self.m.edit.spell;
        if s.off == on {
            s.off = !on;
            s.menu = None;
            self.repass();
        }
    }

    /// the language of the checker: a BCP 47 tag ("de-DE"), "" = the user's languages
    pub fn spell_language(&mut self, tag: &str) {
        let s = &mut self.m.edit.spell;
        if s.lang != tag {
            s.lang = String::from(tag);
            s.reset();
            self.repass();
        }
    }

    /// where checkers come from (spell::SRC_*; 0 = the default, see the module header)
    pub fn spell_sources(&mut self, sources: u32) {
        let s = &mut self.m.edit.spell;
        if s.sources != sources {
            s.sources = sources;
            s.reset();
            self.repass();
        }
    }

    /// use the app's own checker
    pub fn spell_set(&mut self, sp: Speller) {
        let s = &mut self.m.edit.spell;
        s.reset();
        s.speller = Some(sp);
        s.state = SPELL_READY;
        self.repass();
    }

    /// suggestions on a worker: on / off (default: on in window mode, off in headless runs)
    pub fn spell_suggest_async(&mut self, on: bool) {
        self.m.edit.spell.sug_async = Some(on);
    }

    /// SPELL_OFF / SPELL_LOADING / SPELL_READY / SPELL_NONE
    pub fn spell_state(&self) -> u32 {
        let s = &self.m.edit.spell;
        if s.off { SPELL_OFF } else { s.state }
    }

    /// the checker in use (None while it loads or when there is none)
    pub fn speller(&mut self) -> Option<&mut Speller> {
        self.spell_ensure();
        self.m.edit.spell.speller.as_mut()
    }

    /// the misspelled byte ranges of text box id, as last checked (none while switched off)
    pub fn spell_errors(&self, id: u32) -> Vec<(usize, usize)> {
        let s = &self.m.edit.spell;
        if s.off || s.speller.is_none() {
            return Vec::new();
        }
        s.boxes.get(&id).filter(|b| b.generation == s.generation).map_or_else(Vec::new, |b| b.bad.clone())
    }

    // the personal word list of a checker: config:// in window mode, memory headless
    fn spell_personal(&self, sp: &mut Speller) {
        if !self.headless {
            let p = spell::personal_path(sp.language());
            sp.set_personal_file(Some(&p));
        }
    }

    // the checker: found and loaded (see the module header)
    fn spell_ensure(&mut self) {
        if self.m.edit.spell.off {
            return;
        }
        // a finished load (one started before the last reset is dropped)
        let r = self.m.edit.spell.slot.lock().take();
        if let Some((n, dict, lang)) = r {
            if n == self.m.edit.spell.load && self.m.edit.spell.state == SPELL_LOADING {
                match dict {
                    Some(d) => {
                        let mut sp = Speller::with_dictionary(d, &lang);
                        self.spell_personal(&mut sp);
                        self.spell_ready(sp);
                    }
                    None => self.m.edit.spell.state = SPELL_NONE,
                }
                self.repass();
                return;
            }
        }
        if self.m.edit.spell.state != SPELL_OFF {
            return; // loading (the job wakes the UI), ready or none
        }
        let headless = self.headless;
        let s = &mut self.m.edit.spell;
        let sources = if s.sources != 0 {
            s.sources
        } else if headless {
            SRC_APP
        } else {
            SRC_ALL
        };
        let langs: Vec<String> = if s.lang.is_empty() { i18n::user_languages() } else { alloc::vec![s.lang.clone()] };
        let found = langs.iter().find_map(|l| spell::find(l, sources));
        match found {
            None => s.state = SPELL_NONE,
            Some(Source::System(t)) => match Speller::from_source(&Source::System(t)) {
                Some(mut sp) => {
                    self.spell_personal(&mut sp);
                    self.spell_ready(sp);
                }
                None => self.m.edit.spell.state = SPELL_NONE,
            },
            Some(Source::Hunspell { aff, dic, lang, .. }) => {
                if headless || s.job.as_ref().is_some_and(|j| !j.done()) {
                    if !headless {
                        self.timer(UIT_SPELL, 50); // an older load is still running: soon again
                        return;
                    }
                    match Dictionary::load(&aff, &dic) {
                        Some(d) => {
                            let mut sp = Speller::with_dictionary(d, &lang);
                            self.spell_personal(&mut sp);
                            self.spell_ready(sp);
                        }
                        None => self.m.edit.spell.state = SPELL_NONE,
                    }
                    return;
                }
                // window mode: read the files on a worker, plat::wake() when done
                let n = s.load;
                let slot = s.slot.clone();
                let mut job = s.job.take().unwrap_or_default();
                job.set(1, move |_, _| {
                    let d = Dictionary::load(&aff, &dic);
                    *slot.lock() = Some((n, d, lang.clone()));
                });
                job.start();
                s.job = Some(job);
                s.state = SPELL_LOADING;
            }
        }
    }

    fn spell_ready(&mut self, sp: Speller) {
        let s = &mut self.m.edit.spell;
        s.speller = Some(sp);
        s.state = SPELL_READY;
        s.generation = s.generation.wrapping_add(1);
    }

    /// (edit.rs) the text box being built checks its text -> marks can be drawn
    pub(super) fn spell_scan(&mut self, id: u32) -> bool {
        let flags = self.m.edit.tb_flags;
        if flags & TB_SPELL == 0 || flags & (TB_PASSWORD | TB_READONLY | TB_DIGITS) != 0 || self.m.edit.spell.off {
            return false;
        }
        self.spell_ensure();
        let (headless, frame) = (self.headless, self.frame);
        let e = &mut self.m.edit;
        let s = &mut e.spell;
        let Some(sp) = s.speller.as_mut() else { return false };
        let Ok(text) = core::str::from_utf8(&e.buf) else { return false };
        if s.budget_frame != frame {
            s.budget_frame = frame;
            s.budget = if headless { u32::MAX } else { BUDGET };
            s.budget_until = if headless { u64::MAX } else { plat::time_us() + BUDGET_US };
        }
        let h = fnv(text.as_bytes());
        let b = s.boxes.entry(id).or_default();
        if b.hash != h || b.generation != s.generation || b.rev != sp.revision() || b.seen == 0 {
            b.hash = h;
            b.generation = s.generation;
            b.rev = sp.revision();
            b.toks = spell::tokens(text, sp.wordchars());
            b.next = 0;
            b.bad.clear();
        }
        while b.next < b.toks.len() {
            let t = b.toks[b.next];
            if sp.cached(&text[t.start..t.end]).is_none() {
                if s.budget == 0 || (s.budget < BUDGET && plat::time_us() >= s.budget_until) {
                    break;
                }
                s.budget -= 1;
            }
            b.bad.extend(sp.token_errors(text, &t));
            b.next += 1;
        }
        b.seen = frame.max(1);
        let pending = b.next < b.toks.len();
        if s.boxes.len() > BOXES_MAX {
            s.boxes.retain(|k, b| *k == id || frame.wrapping_sub(b.seen) < 600);
        }
        if pending {
            self.timer(UIT_SPELL, 16);
        }
        true
    }

    /// (edit.rs) the marks of box id between the text indexes ls..le of a line whose first
    /// character is at x (baseline tb_base, the box's font)
    pub(super) fn spell_draw(&mut self, id: u32, x: i32, ls: usize, le: usize, editing: bool) {
        let Some(b) = self.m.edit.spell.boxes.get(&id) else { return };
        let spans: Vec<(usize, usize)> = b.bad.iter().filter(|r| r.1 > ls && r.0 < le).copied().collect();
        if spans.is_empty() {
            return;
        }
        // the word being typed stays unmarked
        let e = &self.m.edit;
        let typing = if editing && e.ed_merge && e.ed_caret == e.ed_anchor { Some(e.ed_caret) } else { None };
        let base = e.tb_base;
        let c = self.color(T_CRIT);
        self.draw_color(c);
        for (a, z) in spans {
            if typing == Some(z) {
                continue;
            }
            let (a0, z0) = (a.max(ls), z.min(le));
            let x0 = x.wrapping_add(self.text.caret_x(&self.m.edit.buf[ls..], a0 - ls));
            let x1 = x.wrapping_add(self.text.caret_x(&self.m.edit.buf[ls..], z0 - ls));
            self.spell_wave(x0, x1, base);
            let lh = self.dp(20);
            if let Some(m) = self.m.edit.spell.menu.as_mut() {
                if m.id == id && m.start == a {
                    m.rect = [x0, base - lh * 3 / 4, (x1 - x0).max(1), lh];
                }
            }
        }
    }

    // a wavy line from x0 to x1 below the baseline y (px)
    fn spell_wave(&mut self, x0: i32, x1: i32, y: i32) {
        let hp = self.dp(2).max(2); // half a wave
        let amp = self.dp(2).max(2);
        let y0 = y + self.dp(2);
        let w = self.dp(1).max(1) << 8;
        let mut x = x0;
        let mut up = false;
        while x < x1 {
            let nx = (x + hp).min(x1);
            let (ya, yb) = if up { (y0 + amp, y0) } else { (y0, y0 + amp) };
            self.draw_line(x << 8, (ya << 8) + 128, nx << 8, (yb << 8) + 128, w);
            x = nx;
            up = !up;
        }
    }

    // the popup id of box id's menu (in the id scope of the box)
    fn spell_pid(&mut self, id: u32) -> u32 {
        self.id_push(id);
        let p = self.id("spell-menu");
        self.id_pop();
        p
    }

    // the misspelled word of box id at text index i
    fn spell_at(&self, id: u32, i: usize) -> Option<(usize, usize)> {
        self.m.edit.spell.boxes.get(&id)?.bad.iter().find(|r| r.0 <= i && i <= r.1).copied()
    }

    // open the menu for the word a..z of box id (its suggestions now or from a worker)
    fn spell_open(&mut self, id: u32, a: usize, z: usize, kbd: bool) {
        let word = String::from_utf8_lossy(&self.m.edit.buf[a..z]).into_owned();
        let headless = self.headless;
        let s = &mut self.m.edit.spell;
        let Some(sp) = s.speller.as_mut() else { return };
        let sg = if s.sug_async.unwrap_or(!headless) { sp.suggester() } else { None };
        let (sugs, wait) = match sg {
            Some(sg) => (Vec::new(), s.sug_start(sg, &word)),
            None => (sp.suggest(&word, SUG_SHOWN), 0),
        };
        let pid = self.spell_pid(id);
        let e = &self.m.edit;
        let caret = (e.ed_id == id && self.wp.ctx.focus == id).then_some((e.ed_caret, e.ed_anchor));
        let rect = self.wp.ctx.last_rect;
        self.m.edit.spell.menu = Some(Menu { id, pid, start: a, end: z, word, sugs, wait, add_row: 0, kbd, rect, caret });
        self.popup_open(pid);
        if kbd {
            self.wp.ctx.focus_visible = true;
            self.pp_flag(pid, crate::popup::PPF_FOCUS);
        }
        self.repass();
    }

    /// (edit.rs) after the box is drawn: a right click at text index `hit` (or the menu key)
    /// opens the menu, the open menu is drawn and its choice applied
    pub(super) fn spell_input(&mut self, id: u32, ev: u32, hit: Option<usize>, editing: bool) {
        if ev & EV_RCLICK != 0 {
            if let Some((a, z)) = hit.and_then(|i| self.spell_at(id, i)) {
                self.wp.ctx.released &= !(1 << BTN_RIGHT); // not the app's context menu as well
                self.spell_open(id, a, z, false);
            }
        } else if editing && self.m.edit.spell.menu.is_none() {
            let caret = self.m.edit.ed_caret;
            if let Some((a, z)) = self.spell_at(id, caret) {
                let k = self.key_first();
                let open = if k & 0xFFFF == VK_APPS {
                    self.key(VK_APPS)
                } else if k & 0xFFFF == VK_F10 && (k >> 16) & MOD3 == MOD_SHIFT {
                    self.key(VK_F10)
                } else {
                    false
                };
                if open {
                    self.spell_open(id, a, z, true);
                }
            }
        }
        self.spell_menu(id);
    }

    /// (runtime E_WAKE, auto `sync`) the suggestions of a worker reach the menu waiting for
    /// them; a menu opened with a key focuses the first one (unless the focus moved on)
    pub(crate) fn spell_poll(&mut self) {
        let s = &mut self.m.edit.spell;
        let Some(m) = s.menu.as_mut().filter(|m| m.wait != 0) else { return };
        let v = {
            let mut g = s.sug_slot.lock();
            if g.0 != m.wait {
                return;
            }
            match g.1.take() {
                Some(v) => v,
                None => return,
            }
        };
        m.sugs = v;
        m.wait = 0;
        let (kbd, pid, row) = (m.kbd, m.pid, m.add_row);
        if kbd && row != 0 && self.wp.ctx.focus == row {
            self.pp_flag(pid, crate::popup::PPF_FOCUS);
        }
        self.repass();
    }

    /// (auto `sync`) a worker of the spell checker is busy (suggestions, a dictionary)
    pub(crate) fn spell_busy(&self) -> bool {
        let s = &self.m.edit.spell;
        s.sug_jobs.iter().any(|j| !j.done()) || s.job.as_ref().is_some_and(|j| !j.done())
    }

    /// (edit.rs) the menu of box id is open: the box leaves the keys to it
    pub(super) fn spell_menu_open(&self, id: u32) -> bool {
        self.m.edit.spell.menu.as_ref().is_some_and(|m| m.id == id && self.popup_is_open(m.pid))
    }

    // the menu closed: a box that gets the focus back keeps its caret (instead of starting
    // the edit again with everything selected)
    fn spell_closed(&mut self, id: u32, caret: Option<(usize, usize)>) {
        if self.wp.ctx.focus != id || self.m.edit.ed_id == id {
            return;
        }
        let Some((c, a)) = caret else { return };
        let frame = self.frame;
        let e = &mut self.m.edit;
        e.ed_id = id;
        e.ed_caret = Self::tb_fix_index(&e.buf, c);
        e.ed_anchor = Self::tb_fix_index(&e.buf, a);
        e.ed_seen = frame;
        e.ed_merge = false;
        e.ed_prefx = -1;
        self.tb_blink_reset();
    }

    // the menu of box id when it is open; its choice is applied to the box's text
    fn spell_menu(&mut self, id: u32) {
        let Some(m) = self.m.edit.spell.menu.as_ref() else { return };
        if m.id != id {
            return;
        }
        let (kbd, rect, sugs, pid, wait) = (m.kbd, m.rect, m.sugs.clone(), m.pid, m.wait != 0);
        if !self.popup_is_open(pid) {
            let caret = m.caret;
            self.m.edit.spell.menu = None;
            self.spell_closed(id, caret);
            return;
        }
        // the box stays the last widget for the app's post-modifiers (tooltip, context_menu)
        let (lr, ls) = (self.wp.ctx.last_rect, self.wp.ctx.last_state);
        if kbd {
            self.wp.ctx.last_rect = rect;
        }
        let mut act = None;
        if self.flyout_begin(pid, if kbd { FL_BELOW } else { FL_POINT }) {
            self.id_push(id);
            self.id_push_str("spell");
            if wait {
                self.next_flags(ST_DISABLED);
                let r = self.id("wait");
                self.menu_item(r, i18n::tr("Vorschläge werden gesucht …"));
            } else if sugs.is_empty() {
                self.next_flags(ST_DISABLED);
                let r = self.id("none");
                self.menu_item(r, i18n::tr("Keine Vorschläge"));
            }
            for (k, s) in sugs.iter().enumerate() {
                let r = self.id_n(k as u32);
                if self.menu_item(r, s) & EV_ACT != 0 {
                    act = Some(Act::Use(k));
                }
            }
            self.menu_sep();
            let r = self.id("add");
            if let Some(m) = self.m.edit.spell.menu.as_mut() {
                m.add_row = r;
            }
            if self.menu_item(r, i18n::tr("Zum Wörterbuch hinzufügen")) & EV_ACT != 0 {
                act = Some(Act::Add);
            }
            let r = self.id("ignore");
            if self.menu_item(r, i18n::tr("Ignorieren")) & EV_ACT != 0 {
                act = Some(Act::Ignore);
            }
            self.id_pop();
            self.id_pop();
        }
        self.flyout_end();
        self.wp.ctx.last_rect = lr;
        self.wp.ctx.last_state = ls;
        let Some(act) = act else { return };
        let Some(m) = self.m.edit.spell.menu.take() else { return };
        self.spell_closed(id, m.caret);
        match act {
            Act::Use(k) => self.spell_replace(id, &m, &sugs[k]),
            Act::Add => {
                if let Some(sp) = self.m.edit.spell.speller.as_mut() {
                    if !sp.add(&m.word) {
                        self.warn("spell: the personal word list could not be written");
                    }
                }
            }
            Act::Ignore => {
                if let Some(sp) = self.m.edit.spell.speller.as_mut() {
                    sp.ignore(&m.word);
                }
            }
        }
        self.repass();
    }

    // replace the word of the menu by a suggestion (one undo step when the box is focused)
    fn spell_replace(&mut self, id: u32, m: &Menu, with: &str) {
        let e = &self.m.edit;
        if m.end > e.buf.len() || &e.buf[m.start..m.end] != m.word.as_bytes() {
            return; // the text changed meanwhile
        }
        if e.tb_max > 0 && e.buf.len() - (m.end - m.start) + with.len() > e.tb_max {
            return; // it would not fit
        }
        let focused = e.ed_id == id && self.wp.ctx.focus == id;
        if focused {
            self.ud_push();
        }
        self.tb_delete(m.start, m.end);
        let n = self.tb_insert(m.start, with.as_bytes());
        if focused {
            let e = &mut self.m.edit;
            e.ed_caret = m.start + n;
            e.ed_anchor = m.start + n;
            e.ed_merge = false;
        }
        self.m.edit.tb_ev |= EV_ACT;
    }
}

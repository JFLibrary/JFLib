// settings_page -- the settings page of a typed schema (base::settings)
//
//   settings_page(id, &mut settings) -> true when a value changed in this pass
//
// The keys of the schema without KF_HIDDEN, grouped by section: the sections of
// Schema::sections first (with their titles), then the others in the order of their first
// key (titled by their name; keys without a section: "Allgemein").  Each section is a title
// and a card; each key a row: its label on the left, the control on the right -- bool a
// switch, int a number box (min..max, step), num a number box with its decimals, text a text
// box (at most max bytes), choice a combo box, colour a swatch and a button with the hex code
// that opens a picker -- and a reset button (disabled at the default); its help text and
// (KF_RESTART) a note below.  The label is the text right before the control, so screen
// readers name text boxes, number boxes and combo boxes by it (their own text is their
// value); the switch and the colour button carry it as their label (scripts: click "Dunkles
// Design").  The reset buttons are "Zurücksetzen: <label>".  Labels, help texts, section
// titles and choice labels go through i18n::t (the app's catalogue).  Ids: the scope of `id`,
// then the key's name ("value", "reset", "swatch", "pick" inside it; ids.rs).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::settings::{Kind, Settings, Val, KF_HIDDEN, KF_RESTART};

use super::consts::*;
use super::i18n::{t, tr};
use super::keys::IC_UNDO;
use super::value::RangeF;
use super::Ui;

const CONTROL_W: i32 = 200; // text boxes (DIP)

impl Ui {
    /// the settings page of the schema of `s` (see the module header) -> a value changed
    pub fn settings_page(&mut self, id: u32, s: &mut Settings) -> bool {
        let schema = s.schema();
        let shown = |i: usize| schema.keys[i].flags & KF_HIDDEN == 0;
        let mut secs: Vec<&'static str> = schema.sections.iter().map(|x| x.0).collect();
        for (i, d) in schema.keys.iter().enumerate() {
            let sec = d.split().0;
            if shown(i) && !secs.contains(&sec) {
                secs.push(sec);
            }
        }
        let mut changed = false;
        self.id_push(id);
        self.box_begin(0, BOX_V, 12);
        for sec in secs {
            let keys: Vec<usize> = (0..schema.keys.len()).filter(|&i| shown(i) && schema.keys[i].split().0 == sec).collect();
            if keys.is_empty() {
                continue;
            }
            let title = match schema.sections.iter().find(|x| x.0 == sec) {
                Some(x) => t(x.1),
                None if sec.is_empty() => tr("Allgemein"),
                None => sec,
            };
            self.text(TS_BODY_STRONG, title);
            self.box_begin(0, BOX_V | BOX_CARD | pad(16), 12);
            for i in keys {
                changed |= self.settings_row(s, i);
            }
            self.box_end();
        }
        self.box_end();
        self.id_pop();
        changed
    }

    // one key: label and texts, the control, the reset button -> the value changed
    fn settings_row(&mut self, s: &mut Settings, i: usize) -> bool {
        let d = s.schema().keys[i];
        let label = t(d.label);
        self.id_push_str(d.name);
        let cid = self.id("value");
        self.box_begin(0, BOX_V, 2);
        self.box_begin(0, BOX_H | BOX_XCENTER, 12);
        self.next_size(-1, 0);
        self.text(TS_BODY, label);
        let cur = s.at(i).clone();
        let mut new: Option<Val> = None;
        match (d.kind, &cur) {
            (Kind::Bool(_), Val::Bool(b)) => {
                let mut v = *b as i32;
                self.switch(cid, "", &mut v);
                self.sem_name(cid, label);
                if (v != 0) != *b {
                    new = Some(Val::Bool(v != 0));
                }
            }
            (Kind::Int { min, max, step, .. }, Val::Int(x)) => {
                let fit = |v: i64| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
                let rg = Range::new(fit(min), fit(max), fit(step).max(1), 0);
                let mut v = fit(*x);
                if self.number_box(cid, &mut v, &rg) & EV_ACT != 0 && v != fit(*x) {
                    new = Some(Val::Int(v as i64));
                }
            }
            (Kind::Num { min, max, dec, .. }, Val::Num(x)) => {
                let dec = dec.min(9);
                let step = 1.0 / (0..dec).fold(1.0, |a, _| a * 10.0);
                let mut v = *x;
                if self.number_box_f(cid, &mut v, &RangeF::new(min, max, step, dec, 0)) & EV_ACT != 0 && v != *x {
                    new = Some(Val::Num(v));
                }
            }
            (Kind::Text { max, .. }, Val::Text(txt)) => {
                let mut b = txt.clone();
                self.next_size(CONTROL_W, 0);
                self.textbox(cid, None, &mut b, (max as u32).saturating_add(1).min(0xFF_FFFF));
                if b != *txt {
                    new = Some(Val::Text(b));
                }
            }
            (Kind::Choice { items, .. }, Val::Choice(k)) => {
                let list: Vec<&str> = items.iter().map(|it| t(it.1)).collect();
                let mut sel = *k as i32;
                self.combobox(cid, &mut sel, &list);
                if sel >= 0 && sel as usize != *k {
                    new = Some(Val::Choice(sel as usize));
                }
            }
            (Kind::Color(_), Val::Color(c)) => {
                let mut argb = 0xFF00_0000 | c;
                let sw = self.swatch(self.id("swatch"), argb, false);
                self.dropdown_button(cid, &format!("#{:06X}", c));
                if sw & EV_ACT != 0 {
                    self.popup_open(cid);
                }
                self.sem_name(cid, label);
                if self.flyout_begin(cid, FL_BELOW) {
                    self.color_picker_ex(self.id("pick"), &mut argb, CP_FIELDS);
                }
                self.flyout_end();
                if argb & 0xFF_FFFF != *c {
                    new = Some(Val::Color(argb & 0xFF_FFFF));
                }
            }
            _ => {}
        }
        if cur == s.default_at(i) {
            self.next_flags(ST_DISABLED);
        }
        let reset: String = format!("{}: {}", tr("Zurücksetzen"), label);
        if self.icon_button(self.id("reset"), IC_UNDO, &reset) & EV_ACT != 0 {
            new = Some(s.default_at(i));
        }
        self.box_end();
        if !d.help.is_empty() {
            self.text(TS_CAPTION, t(d.help));
        }
        if d.flags & KF_RESTART != 0 {
            self.text(TS_CAPTION, tr("Wird nach einem Neustart wirksam."));
        }
        self.box_end();
        self.id_pop();
        match new {
            Some(v) => s.set_at(i, v),
            None => false,
        }
    }

    // the newest semantic record of widget id gets the row's label (a switch without text,
    // the colour button showing its hex code: screen readers and scripts find them by it)
    fn sem_name(&mut self, id: u32, label: &str) {
        if let Some(r) = self.sems.iter_mut().rev().find(|r| r.id == id) {
            r.label = Some(String::from(label));
        }
    }
}

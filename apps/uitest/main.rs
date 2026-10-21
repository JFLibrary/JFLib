// uitest -- self-checks of ui logic without a window: the undo history (ui::History, Snapshot,
// groups, merging, the limit, the saved state), the easing curves (ui::ease), typed settings
// (base::settings: types, limits, the file, versions and migrations), the hashed widget ids
// (ui::wid, wid_path) and Unicode segmentation (text::segment: UAX #29, UAX #14), the line
// chart's series ring and axis maths (ui::SeriesBuf, LcUnit, lc_nice, lc_time_step,
// lc_time_text).  Console program; exit code 1 when a check fails.
//
//   python build_rs.py app uitest
//   build/win/uitest.exe [--only NAME]        (build_rs.py test runs it through tests/all.uis;
//                                             --script is ignored)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::{Ini, Key, Schema, Settings};
use ui::{EASE_IN, EASE_IN_OUT, EASE_LINEAR, EASE_OUT, EASE_OUT_BACK, EASE_OUT_CUBIC, Edit, History, Snapshot, ease};
use ui::{lc_nice, lc_time_step, lc_time_text, LcUnit, SeriesBuf, SeriesCol};

struct T {
    group: &'static str,
    ok: u32,
    failed: u32,
}

impl T {
    fn eq<A: PartialEq + core::fmt::Debug>(&mut self, name: &str, got: A, want: A) {
        if got == want {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}: got {:?}, want {:?}\n", self.group, name, got, want).as_bytes());
        }
    }
    fn check(&mut self, name: &str, cond: bool) {
        self.eq(name, cond, true);
    }
}

// an edit of a list: push one value
struct Push(i32);

impl Edit<Vec<i32>> for Push {
    fn apply(&mut self, s: &mut Vec<i32>) {
        s.push(self.0);
    }
    fn revert(&mut self, s: &mut Vec<i32>) {
        s.pop();
    }
    fn label(&self) -> &str {
        "Hinzufügen"
    }
}

fn snap(label: &str, before: i32, after: i32) -> Box<dyn Edit<i32>> {
    Box::new(Snapshot::new(label, before, after))
}

fn history(t: &mut T) {
    let mut s = 0i32;
    let mut h: History<i32> = History::new(100);
    t.check("empty", !h.can_undo() && !h.can_redo() && !h.dirty() && h.is_empty());
    h.apply(&mut s, snap("eins", 0, 1));
    h.apply(&mut s, snap("zwei", 1, 2));
    t.eq("applied", (s, h.len(), h.undo_label()), (2, 2, Some("zwei")));
    t.check("undo", h.undo(&mut s) && s == 1 && h.redo_label() == Some("zwei"));
    t.check("redo", h.redo(&mut s) && s == 2 && !h.can_redo());
    h.undo(&mut s);
    h.apply(&mut s, snap("drei", 1, 3));
    t.check("a new edit drops the redo steps", s == 3 && !h.can_redo() && h.len() == 2);
    t.check("nothing to redo", !h.redo(&mut s) && s == 3);
    // merging: a drag is one step
    let mut v = 10i32;
    let mut m: History<i32> = History::new(100);
    for k in 1..=3 {
        m.apply(&mut v, Box::new(Snapshot::new("Ziehen", 10 + k - 1, 10 + k).merging(7)));
    }
    t.eq("merged steps", (v, m.len()), (13, 1));
    m.undo(&mut v);
    t.eq("one undo takes the drag back", v, 10);
    m.redo(&mut v);
    m.apply(&mut v, Box::new(Snapshot::new("Ziehen", 13, 14).merging(8)));
    t.eq("another key: a new step", m.len(), 2);
    // groups
    let mut list: Vec<i32> = Vec::new();
    let mut g: History<Vec<i32>> = History::new(100);
    g.group_begin("Drei auf einmal");
    for k in 1..=3 {
        g.apply(&mut list, Box::new(Push(k)));
    }
    g.group_end();
    t.eq("a group is one step", (g.len(), g.undo_label()), (1, Some("Drei auf einmal")));
    g.undo(&mut list);
    t.eq("undo reverts the whole group", list.len(), 0);
    g.redo(&mut list);
    t.eq("redo applies it again in order", list, alloc::vec![1, 2, 3]);
    g.group_begin("leer");
    g.group_end();
    t.eq("an empty group records nothing", g.len(), 1);
    // the limit
    let mut n = 0i32;
    let mut l: History<i32> = History::new(3);
    for k in 1..=5 {
        l.apply(&mut n, snap("x", k - 1, k));
    }
    t.eq("at most `limit` steps", l.len(), 3);
    while l.undo(&mut n) {}
    t.eq("undo stops at the oldest kept step", n, 2);
    // the saved state
    let mut d = 0i32;
    let mut sv: History<i32> = History::new(100);
    sv.apply(&mut d, snap("a", 0, 1));
    sv.mark_saved();
    t.check("saved: not dirty", !sv.dirty());
    sv.apply(&mut d, Box::new(Snapshot::new("b", 1, 2).merging(1)));
    t.check("a change: dirty", sv.dirty());
    sv.apply(&mut d, Box::new(Snapshot::new("b", 2, 3).merging(1)));
    t.eq("merging stops at the saved point (the save stays between)", sv.len(), 2);
    sv.undo(&mut d);
    t.check("back at the save: clean", !sv.dirty() && d == 1);
    sv.undo(&mut d);
    t.check("before the save: dirty", sv.dirty() && d == 0);
    sv.apply(&mut d, snap("c", 0, 5));
    t.check("the saved state is gone with the redo steps: dirty for good", sv.dirty());
    while sv.undo(&mut d) {}
    t.check("still dirty at the start", sv.dirty());
    sv.clear();
    t.check("clear keeps dirty", sv.dirty() && sv.is_empty());
}

fn easing(t: &mut T) {
    let curves = [EASE_OUT, EASE_LINEAR, EASE_IN, EASE_IN_OUT, EASE_OUT_CUBIC, EASE_OUT_BACK];
    for c in curves {
        t.check(&format!("curve {c}: 0 at the start"), ease(c, 0).abs() <= 1);
        t.eq(&format!("curve {c}: 256 at the end"), ease(c, 256), 256);
    }
    t.eq("linear", ease(EASE_LINEAR, 100), 100);
    t.eq("out (quadratic): fast start", ease(EASE_OUT, 64), 256 - (192 * 192 >> 8));
    t.eq("in (quadratic): slow start", ease(EASE_IN, 64), 16);
    t.eq("in-out: the middle", ease(EASE_IN_OUT, 128), 128);
    t.eq("in-out: symmetric", ease(EASE_IN_OUT, 64) + ease(EASE_IN_OUT, 192), 256);
    for c in [EASE_OUT, EASE_LINEAR, EASE_IN, EASE_IN_OUT, EASE_OUT_CUBIC] {
        t.check(&format!("curve {c}: never goes back"), (0..256).all(|k| ease(c, k) <= ease(c, k + 1)));
    }
    let top = (0..=256).map(|k| ease(EASE_OUT_BACK, k)).max().unwrap_or(0);
    t.check("out-back overshoots by about 10 %", (270..=290).contains(&top));
    t.eq("outside 0..256 clamps", (ease(EASE_LINEAR, -5), ease(EASE_LINEAR, 999)), (0, 256));
}

// ---- typed settings (base::settings)

const S_DARK: Key<bool> = Key::bool("view.dark", false).label("Dunkles Design");
const S_ZOOM: Key<i64> = Key::int("view.zoom", 100, 10, 800).label("Zoom (%)").step(10);
const S_SCALE: Key<f64> = Key::num("view.scale", 1.5, 0.5, 4.0, 2);
const S_NAME: Key<String> = Key::text("user.name", "Gast", 8);
const S_UNIT: Key<usize> = Key::choice("view.unit", 0, &[("px", "Pixel"), ("cm", "Zentimeter"), ("in", "Zoll")]);
const S_ACCENT: Key<u32> = Key::color("view.accent", 0x0078D4);
const S_TOP: Key<i64> = Key::int("count", 3, 0, 9).hidden();
const S_EXTRA: Key<i64> = Key::int("misc.extra", 7, 0, 100); // not in the schema

static SCHEMA1: Schema = Schema::new(1, &[S_DARK.d, S_ZOOM.d]);
static SCHEMA2: Schema = Schema::new(2, &[S_DARK.d, S_ZOOM.d, S_SCALE.d, S_NAME.d, S_UNIT.d, S_ACCENT.d, S_TOP.d])
    .sections(&[("view", "Ansicht")])
    .migrate(migrate2);

// version 1 called the zoom "view.size" and stored it in tenths
fn migrate2(from: u32, ini: &mut Ini) {
    if from < 2 {
        let tenths = ini.get_int("view", "size", -1);
        ini.remove("view", "size");
        if tenths >= 0 {
            ini.set_int("view", "zoom", tenths * 10);
        }
        base::settings::rename(ini, "view.theme", "view.dark");
    }
}

fn settings(t: &mut T) {
    // defaults, types
    let mut s = Settings::new(&SCHEMA2);
    t.eq("default bool", s.get(&S_DARK), false);
    t.eq("default int", s.get(&S_ZOOM), 100);
    t.eq("default num", s.get(&S_SCALE), 1.5);
    t.eq("default text", s.text(&S_NAME), "Gast");
    t.eq("default choice", s.get(&S_UNIT), 0usize);
    t.eq("default colour", s.get(&S_ACCENT), 0x0078D4u32);
    t.check("is default", s.is_default(&S_ZOOM) && !s.dirty());
    // set: changed / unchanged, limits
    t.check("set changes", s.set(&S_ZOOM, 120));
    t.check("same value: no change", !s.set(&S_ZOOM, 120));
    t.check("dirty", s.dirty());
    s.set(&S_ZOOM, 5000);
    t.eq("int clamped", s.get(&S_ZOOM), 800);
    s.set(&S_SCALE, -3.0);
    t.eq("num clamped", s.get(&S_SCALE), 0.5);
    s.set(&S_NAME, String::from("Jürgen-Maximilian"));
    t.eq("text cut at a character", s.text(&S_NAME), "Jürgen-");
    s.set(&S_UNIT, 9);
    t.eq("unknown choice: default", s.get(&S_UNIT), 0usize);
    s.set(&S_UNIT, 2);
    s.set(&S_ACCENT, 0xFF12_3456);
    t.eq("colour without alpha", s.get(&S_ACCENT), 0x12_3456u32);
    s.set(&S_DARK, true);
    // the file: only values that differ from the default
    let text = String::from_utf8(s.ini().to_bytes()).unwrap_or_default();
    t.check("file has zoom", text.contains("zoom=800"));
    t.check("file has choice word", text.contains("unit=in"));
    t.check("file has colour", text.contains("accent=#123456"));
    t.check("file has bool", text.contains("dark=true"));
    t.check("defaults not in the file", !text.contains("count="));
    s.reset(&S_ZOOM);
    t.check("reset removes the line", !String::from_utf8(s.ini().to_bytes()).unwrap_or_default().contains("zoom="));
    t.eq("reset value", s.get(&S_ZOOM), 100);
    let rev = s.revision();
    t.check("reset_all", s.reset_all() && s.revision() > rev && s.is_default(&S_DARK) && s.get(&S_UNIT) == 0);
    // keys outside the schema: through the file
    t.eq("unknown key default", s.get(&S_EXTRA), 7);
    t.check("unknown key set", s.set(&S_EXTRA, 42) && s.get(&S_EXTRA) == 42);
    t.eq("unknown key in the file", s.ini().get("misc", "extra"), Some("42"));

    // save and open again (mem://), the version line
    plat::delete("mem://uitest/settings.ini");
    let mut s = Settings::open("mem://uitest/settings.ini", &SCHEMA2);
    t.eq("missing file: version 0", s.file_version(), 0);
    s.set(&S_ZOOM, 250);
    s.set(&S_NAME, String::from("Ada"));
    t.check("save", s.save() && !s.dirty());
    let s = Settings::open("mem://uitest/settings.ini", &SCHEMA2);
    t.eq("read back int", s.get(&S_ZOOM), 250);
    t.eq("read back text", s.text(&S_NAME), "Ada");
    t.eq("version written", s.file_version(), 2);
    t.check("opened clean", !s.dirty());

    // invalid values: defaults (and a WARN), out of range: clamped
    let bad = Ini::parse(b"version=2\n[view]\nzoom=abc\nscale=99\nunit=furlong\naccent=#12345\ndark=vielleicht\n");
    let s = Settings::from_ini(bad, &SCHEMA2);
    t.eq("bad int", s.get(&S_ZOOM), 100);
    t.eq("num out of range", s.get(&S_SCALE), 4.0);
    t.eq("bad choice", s.get(&S_UNIT), 0usize);
    t.eq("bad colour", s.get(&S_ACCENT), 0x0078D4u32);
    t.eq("bad bool", s.get(&S_DARK), false);
    t.check("invalid values leave the file alone", !s.dirty());

    // migration 1 -> 2
    let old = Ini::parse(b"version=1\n; my comment\n[view]\nsize=15\ntheme=on\n[other]\nkeep=1\n");
    let s = Settings::from_ini(old, &SCHEMA2);
    t.eq("migrated from", s.file_version(), 1);
    t.eq("migrated value", s.get(&S_ZOOM), 150);
    t.eq("renamed key", s.get(&S_DARK), true);
    t.check("migration marks dirty", s.dirty());
    let text = String::from_utf8(s.ini().to_bytes()).unwrap_or_default();
    t.check("new version line", text.contains("version=2"));
    t.check("old key gone", !text.contains("size="));
    t.check("comment and other section kept", text.contains("; my comment") && text.contains("keep=1"));
    // no version line at all: version 0, migrated too
    let s = Settings::from_ini(Ini::parse(b"[view]\nsize=8\n"), &SCHEMA2);
    t.eq("no version line", (s.file_version(), s.get(&S_ZOOM)), (0, 80));
    // a newer file: read, version kept
    let mut s = Settings::from_ini(Ini::parse(b"version=7\n[view]\nzoom=300\n"), &SCHEMA1);
    t.eq("newer file read", s.get(&S_ZOOM), 300);
    s.set(&S_DARK, true);
    t.eq("newer version kept", s.ini().get("", "version"), Some("7"));
    // sections of an Ini
    let ini = Ini::parse(b"a=1\n[One]\nx=1\n[two]\n[ONE]\ny=2\n");
    t.eq("sections", ini.sections(), alloc::vec!["One", "two"]);
}

// ---- hashed widget ids

fn ids(t: &mut T) {
    // the values are part of saved window states: they must never change
    t.eq("wid stable", ui::wid("save"), 142_357_723);
    t.eq("wid_path stable", ui::wid_path(&["layers", "add"]), 204_816_097);
    t.eq("wid_path of one name", ui::wid_path(&["save"]), ui::wid("save"));
    t.check("different names", ui::wid("save") != ui::wid("open"));
    t.check("scopes differ", ui::wid_path(&["a", "b"]) != ui::wid_path(&["b", "a"]));
    t.eq("a known collision", ui::wid("k11513"), ui::wid("k27407"));
    // the range: above the hand-written ids, below the library's bits, room for id + 1 + i
    let mut lo = u32::MAX;
    let mut hi = 0;
    for i in 0..20000u32 {
        let id = ui::wid(&format!("w{}", i));
        lo = lo.min(id);
        hi = hi.max(id);
    }
    t.check("range low", lo >= ui::ID_HASH_BASE);
    t.check("range high", hi < 0x0FFF_0000);
    t.check("spread", hi - lo > 0x0700_0000);
}

// ---- Unicode segmentation (text::segment)

fn segment(t: &mut T) {
    use text::segment::*;
    // grapheme clusters (UAX #29)
    let g = |s: &str| graphemes(s);
    t.eq("combining mark", g("e\u{301}x"), alloc::vec![0, 3, 4]);
    t.eq("CR LF", g("a\r\nb"), alloc::vec![0, 1, 3, 4]);
    t.eq("control", g("a\u{7}b"), alloc::vec![0, 1, 2, 3]);
    t.eq("flags", g("\u{1F1E9}\u{1F1EA}\u{1F1EB}\u{1F1F7}"), alloc::vec![0, 8, 16]);
    t.eq("odd flags", g("\u{1F1E9}\u{1F1EA}\u{1F1EB}"), alloc::vec![0, 8, 12]);
    t.eq("ZWJ family", g("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}!"), alloc::vec![0, 18, 19]);
    t.eq("skin tone", g("\u{1F44D}\u{1F3FD}"), alloc::vec![0, 8]);
    t.eq("Hangul L V T", g("\u{1100}\u{1161}\u{11A8}\u{AC00}"), alloc::vec![0, 9, 12]);
    t.eq("keycap", g("1\u{FE0F}\u{20E3}2"), alloc::vec![0, 7, 8]);
    t.eq("spacing mark", g("\u{928}\u{93F}"), alloc::vec![0, 6]);
    t.eq("next inside", grapheme_next("e\u{301}x", 1), 3);
    t.eq("prev", grapheme_prev("e\u{301}x", 3), 0);
    t.eq("prev over a family", grapheme_prev("a\u{1F468}\u{200D}\u{1F469}", 12), 1);
    t.check("boundary", is_grapheme_boundary("e\u{301}x", 3) && !is_grapheme_boundary("e\u{301}x", 1));
    t.check("pictographic", pictographic_cluster("\u{1F468}\u{200D}\u{1F469}", 0, 11) && !pictographic_cluster("e\u{301}", 0, 3));
    // words (UAX #29)
    let w = |s: &str| word_bounds(s);
    t.eq("words", w("Hello, world!"), alloc::vec![0, 5, 6, 7, 12, 13]);
    t.eq("apostrophe", w("can't stop"), alloc::vec![0, 5, 6, 10]);
    t.eq("decimal", w("3.14 x"), alloc::vec![0, 4, 5, 6]);
    t.eq("hyphen", w("e-mail"), alloc::vec![0, 1, 2, 6]);
    t.eq("underscore", w("foo_bar1"), alloc::vec![0, 8]);
    t.eq("ideographs", w("日本語"), alloc::vec![0, 3, 6, 9]);
    t.eq("katakana", w("カタカナ"), alloc::vec![0, 12]);
    t.eq("spaces", w("a  b"), alloc::vec![0, 1, 3, 4]);
    t.eq("word flags", w("\u{1F1E9}\u{1F1EA}\u{1F1EB}\u{1F1F7}"), alloc::vec![0, 8, 16]);
    t.eq("umlauts", w("Größe 2"), alloc::vec![0, 7, 8, 9]);
    t.eq("word_at", word_at("Hello, world!", 8), (7, 12));
    t.eq("word_at end", word_at("ab cd", 5), (3, 5));
    t.eq("kinds", (word_kind("a, b", 0, 1), word_kind("a, b", 1, 2), word_kind("a, b", 2, 3)), (WK_WORD, WK_PUNCT, WK_SPACE));
    t.eq("word_next", (word_next("Hello, world!", 0), word_next("Hello, world!", 5), word_next("ab  cd", 0)), (5, 7, 4));
    t.eq("word_prev", (word_prev("Hello, world!", 13), word_prev("Hello, world!", 12), word_prev("ab  cd", 4)), (12, 7, 0));
    t.eq("word_next ideographs", word_next("日本語", 0), 3);
    // line break opportunities (UAX #14)
    let l = |s: &str| line_breaks(s);
    t.eq("spaces break", l("Hello world"), alloc::vec![(6, false), (11, true)]);
    t.eq("after a hyphen", l("a-b"), alloc::vec![(2, false), (3, true)]);
    t.eq("hyphen before a number", l("x 1-2"), alloc::vec![(2, false), (5, true)]);
    t.eq("ideographs break", l("日本語"), alloc::vec![(3, false), (6, false), (9, true)]);
    t.eq("no break before 。", l("日本。語"), alloc::vec![(3, false), (9, false), (12, true)]);
    t.eq("mandatory", l("a\nb"), alloc::vec![(2, true), (3, true)]);
    t.eq("no-break space", l("a\u{A0}b c"), alloc::vec![(5, false), (6, true)]);
    t.eq("parentheses", l("(ab) cd"), alloc::vec![(5, false), (7, true)]);
    t.eq("soft hyphen", l("Ver\u{AD}trag"), alloc::vec![(5, false), (9, true)]);
    t.eq("flag pairs", l("\u{1F1E9}\u{1F1EA}\u{1F1EB}\u{1F1F7}"), alloc::vec![(8, false), (16, true)]);
    t.eq("combining marks stay", l("e\u{301}\u{301} x"), alloc::vec![(6, false), (7, true)]);
    t.eq("emoji modifier", l("\u{1F44D}\u{1F3FD}\u{1F44D}"), alloc::vec![(8, false), (12, true)]);
    t.eq("numbers with currency", l("$ 10.50 €"), alloc::vec![(2, false), (8, false), (11, true)]);
    // wrapping
    let fits = |n: usize| move |a: usize, b: usize| b - a <= n;
    t.eq("wrap words", wrap_end("Hello world foo", 0, 15, &mut fits(11)), 12);
    t.eq("wrap whole", wrap_end("Hello", 0, 5, &mut fits(11)), 5);
    t.eq("wrap long word", wrap_end("abcdefgh", 0, 8, &mut fits(3)), 3);
    t.eq("wrap at a mandatory break", wrap_end("ab\ncd", 0, 5, &mut fits(10)), 3);
    t.eq("wrap ideographs", wrap_end("日本語日本語", 0, 18, &mut fits(7)), 6);
    t.eq("wrap keeps clusters", wrap_end("e\u{301}e\u{301}e\u{301}", 0, 9, &mut fits(4)), 3);
    // hyphenation (wrap_end_hyph): bytes count, a hyphen one more; the hook knows two words
    let hfits = |n: usize| move |a: usize, b: usize, h: bool| b - a + h as usize <= n;
    let mut hook = |w: &str, out: &mut Vec<usize>| {
        *out = match w.to_lowercase().as_str() {
            "silbentrennung" => alloc::vec![3, 6, 10],
            "trennung" => alloc::vec![4],
            _ => Vec::new(),
        };
    };
    let s = "Die Silbentrennung hilft";
    t.eq("hyphenated: Die Silben-", wrap_end_hyph(s, 0, s.len(), &mut hfits(12), Some(&mut hook)), (10, true));
    t.eq("without a hook: at the space", wrap_end_hyph(s, 0, s.len(), &mut hfits(12), None), (4, false));
    t.eq("the line goes on inside the word", wrap_end_hyph(s, 10, s.len(), &mut hfits(7), Some(&mut hook)), (14, true));
    t.eq("the rest of the word fits", wrap_end_hyph(s, 14, s.len(), &mut hfits(6), Some(&mut hook)), (19, false));
    t.eq("a long word: hyphenated before graphemes are cut", wrap_end_hyph("Silbentrennung", 0, 14, &mut hfits(8), Some(&mut hook)), (6, true));
    t.eq("no point fits: graphemes", wrap_end_hyph("Silbentrennung", 0, 14, &mut hfits(3), Some(&mut hook)), (3, false));
    t.eq("whole text fits", wrap_end_hyph(s, 0, s.len(), &mut hfits(40), Some(&mut hook)), (s.len(), false));
    t.eq("no hyphenation after a mandatory break", wrap_end_hyph("ab\nSilbentrennung", 0, 17, &mut hfits(10), Some(&mut hook)), (3, false));
    // soft hyphens: a break with a hyphen; words with one are not hyphenated by the hook
    let s = "Die Sil\u{AD}bentrennung";
    t.eq("soft hyphen", wrap_end_hyph(s, 0, s.len(), &mut hfits(10), None), (9, true));
    t.eq("soft hyphen, the hyphen does not fit", wrap_end_hyph(s, 0, s.len(), &mut hfits(9), Some(&mut hook)), (4, false));
    t.eq("words with digits stay whole", wrap_end_hyph("x Silbentrennung2", 0, 17, &mut hfits(12), Some(&mut hook)), (2, false));
    t.eq("wrap_end ignores soft hyphens' hyphen", wrap_end(s, 0, s.len(), &mut fits(9)), 9);
    t.check("version", unicode_version().starts_with("15."));
}

// line charts: the ring of points (order, capacity, refused times, gaps, lookups, pixel
// columns), the labels of values and ticks, nice steps, clock steps and labels
fn chart(t: &mut T) {
    let sep = ',' as u32;
    let strs = |v: &[&str]| v.iter().map(|s| String::from(*s)).collect::<Vec<String>>();
    let mut b = SeriesBuf::new(4);
    t.check("empty", b.is_empty() && b.first().is_none() && b.min_max().is_none() && b.nearest(1.0).is_none());
    let v0 = b.ver();
    t.check("push", b.push(0.0, 1.0) && b.push(1.0, 5.0) && b.push(1.0, 3.0));
    t.check("a version per change", b.ver() != v0);
    t.check("an earlier time is refused", !b.push(0.5, 9.0) && !b.push(f64::NAN, 1.0) && !b.push(f64::INFINITY, 1.0) && b.len() == 3);
    b.push(2.0, f64::NAN);
    b.push(3.0, 2.0);
    t.eq("full: the oldest went", (b.len(), b.first(), b.last(), b.capacity()), (4, Some((1.0, 5.0)), Some((3.0, 2.0)), 4));
    t.eq("iter", b.iter().map(|p| p.0).collect::<Vec<f64>>(), alloc::vec![1.0, 1.0, 2.0, 3.0]);
    t.eq("iter backwards", (b.iter().rev().next(), b.iter().len(), b.get(1)), (Some((3.0, 2.0)), 4, Some((1.0, 3.0))));
    t.eq("min_max skips gaps", b.min_max(), Some((2.0, 5.0)));
    t.eq("min_max_in", (b.min_max_in(1.5, 3.0), b.min_max_in(1.5, 2.5), b.min_max_in(4.0, 9.0)), (Some((2.0, 2.0)), None, None));
    t.eq("lower_bound", (b.lower_bound(0.0), b.lower_bound(1.0), b.lower_bound(1.5), b.lower_bound(9.0)), (0, 0, 2, 4));
    t.eq("nearest skips gaps", (b.nearest(2.0), b.nearest(2.6), b.nearest(-5.0), b.nearest(99.0)), (Some(1), Some(3), Some(0), Some(3)));
    let v1 = b.ver();
    b.clear();
    t.check("clear", b.is_empty() && b.capacity() == 4 && b.ver() != v1);
    // pixel columns: the range, first and last value per column; a gap starts an entry
    let mut c = SeriesBuf::new(100);
    for (x, v) in [(0.0, 1.0), (0.4, 3.0), (0.9, 2.0), (1.0, f64::NAN), (1.5, 4.0), (3.9, 7.0), (4.0, 8.0)] {
        c.push(x, v);
    }
    let col = |col, n, min, max, first, last, gap| SeriesCol { col, n, min, max, first, last, gap };
    t.eq(
        "columns",
        c.columns(0.0, 4.0, 4),
        alloc::vec![col(0, 3, 1.0, 3.0, 1.0, 2.0, false), col(1, 1, 4.0, 4.0, 4.0, 4.0, true), col(3, 2, 7.0, 8.0, 7.0, 8.0, false)],
    );
    let mut g = SeriesBuf::new(8);
    for (x, v) in [(0.0, 1.0), (0.2, f64::NAN), (0.4, 2.0)] {
        g.push(x, v);
    }
    let gc: Vec<(u32, u32, bool)> = g.columns(0.0, 1.0, 1).iter().map(|c| (c.col, c.n, c.gap)).collect();
    t.eq("a gap inside a column: a second entry", gc, alloc::vec![(0, 1, false), (0, 1, true)]);
    t.check("columns of nothing", c.columns(5.0, 9.0, 4).is_empty() && c.columns(1.0, 1.0, 4).is_empty() && c.columns(0.0, 4.0, 0).is_empty());
    let mut big = SeriesBuf::new(100_000);
    for i in 0..100_000u32 {
        big.push(i as f64 / 1000.0, (i % 1000) as f64);
    }
    let cols = big.columns(0.0, 100.0, 500);
    let n: usize = cols.iter().map(|c| c.n as usize).sum();
    t.eq("100 000 points in 500 columns", (cols.len(), n), (500, 100_000));
    t.check("their extremes kept", cols[0].min == 0.0 && cols.iter().all(|c| c.max <= 999.0) && cols.iter().any(|c| c.max == 999.0));
    t.check("a full ring keeps its capacity", big.push(100.0, 1.0) && big.len() == 100_000 && big.first() == Some((0.001, 1.0)));
    // values: three significant digits, the scale that fits
    let secs = [0.0482, 0.00045, 1.5, 0.12, 0.0].map(|v| LcUnit::Secs.text(v, sep));
    t.eq("seconds", secs.to_vec(), strs(&["48,2 ms", "450 µs", "1,5 s", "120 ms", "0 s"]));
    let nums = [
        LcUnit::Num("req/s").text(1234.0, sep),
        LcUnit::Num("").text(999.6, sep),
        LcUnit::Num("%").text(12.34, sep),
        LcUnit::Num("").text(-0.0123, sep),
        LcUnit::Num("").text(f64::NAN, sep),
    ];
    t.eq("numbers", nums.to_vec(), strs(&["1,23k req/s", "1k", "12,3 %", "-0,0123", "–"]));
    let si = [LcUnit::Si("B").text(1500.0, sep), LcUnit::Si("B").text(950.0, sep), LcUnit::Si("bit/s").text(1.0e8, sep), LcUnit::Si("B/s").text(2.68e7, sep)];
    t.eq("SI prefixes", si.to_vec(), strs(&["1,5 kB", "950 B", "100 Mbit/s", "26,8 MB/s"]));
    t.eq("decimal point", LcUnit::Secs.text(0.0482, '.' as u32), String::from("48.2 ms"));
    // ticks: one scale, the decimals of the step, zero with the unit
    t.eq("ticks in ms", LcUnit::Secs.ticks(0.0, 0.05, 4, sep), strs(&["0 ms", "50 ms", "100 ms", "150 ms", "200 ms"]));
    t.eq("ticks of requests", LcUnit::Num("req/s").ticks(0.0, 250.0, 4, sep), strs(&["0 req/s", "250 req/s", "500 req/s", "750 req/s", "1000 req/s"]));
    t.eq("ticks with k", LcUnit::Num("").ticks(0.0, 5000.0, 3, sep), strs(&["0", "5k", "10k", "15k"]));
    t.eq("ticks in MB", LcUnit::Si("B").ticks(0.0, 5e5, 3, sep), strs(&["0 MB", "0,5 MB", "1,0 MB", "1,5 MB"]));
    t.eq("negative ticks", LcUnit::Num("").ticks(-2.0, 1.0, 4, sep), strs(&["-2", "-1", "0", "1", "2"]));
    // nice steps: 1 / 2 / 2.5 / 5 x 10^k
    let nice = [lc_nice(0.0, 0.73, 5), lc_nice(0.0, 230.0, 5), lc_nice(3.0, 97.0, 5), lc_nice(-1.3, 2.2, 4), lc_nice(5.0, 5.0, 5), lc_nice(0.0, 1.2e6, 6), lc_nice(0.0, 120.0, 5)];
    t.eq("nice", nice.to_vec(), alloc::vec![(0.0, 0.2, 4), (0.0, 50.0, 5), (0.0, 20.0, 5), (-2.0, 2.0, 3), (5.0, 1.0, 1), (0.0, 2e5, 6), (0.0, 25.0, 5)]);
    // clock steps and labels
    let steps = [0.3, 0.8, 3.0, 7.0, 40.0, 100.0, 4000.0, 2e5, 2e-8].map(lc_time_step);
    t.eq("time steps", steps.to_vec(), alloc::vec![0.5, 1.0, 5.0, 10.0, 60.0, 120.0, 7200.0, 432000.0, 2e-8]);
    let labels = [
        lc_time_text(5.0, 0, false, sep),
        lc_time_text(60.0, 0, false, sep),
        lc_time_text(90.0, 0, false, sep),
        lc_time_text(3600.0, 0, true, sep),
        lc_time_text(83.5, 1, false, sep),
        lc_time_text(-30.0, 0, false, sep),
        lc_time_text(1.000012, 6, false, sep),
        lc_time_text(4000.0, 0, false, sep),
    ];
    t.eq("time labels", labels.to_vec(), strs(&["0:05", "1:00", "1:30", "1:00:00", "1:23,5", "-0:30", "0:01,000012", "66:40"]));
}

fn run() -> i32 {
    let only = plat::arg_value("--only").map(String::from);
    let groups: &[(&str, fn(&mut T))] = &[("history", history), ("easing", easing), ("settings", settings), ("ids", ids), ("segment", segment), ("chart", chart)];
    let (mut failed, mut total) = (0, 0);
    for (name, f) in groups {
        if only.as_deref().is_some_and(|o| o != *name) {
            continue;
        }
        let mut t = T { group: name, ok: 0, failed: 0 };
        f(&mut t);
        plat::write_stdout(format!("{:<10} {} ok, {} failed\n", name, t.ok, t.failed).as_bytes());
        failed += t.failed;
        total += t.ok + t.failed;
    }
    plat::write_stdout(format!("uitest: {} of {} checks passed\n", total - failed, total).as_bytes());
    if failed == 0 { 0 } else { 1 }
}

rt::main!(run());

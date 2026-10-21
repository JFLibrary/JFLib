// editortest -- the rich-text editor (ui/rich_edit.rs) and the code editor (ui/code_edit.rs);
// for tests/*.uis
//
//   build/win/editortest.exe --script apps/editortest/tests/rich_type.uis --verify-dirty
//   build/win/editortest.exe --test model      self-checks of RichDoc, CodeDoc and the
//                                              highlighter (model.rs); exit code = failures
//   build/win/editortest.exe                   a window
//
// Tab "Rich-Text": the toolbar over a richtext with a sample document; status lines below
// it show the document as a compact markup (`doc: ...`, see markup()), the selection, the
// revision and the last activated link.  Tab "Code": the language (Rust, JSON, INI, TOML,
// Markdown, Text, JFS), a sample per language, "100 000 Zeilen" (a generated file); the status
// line shows the caret, the find matches and the lines tokenized in the last frame.
// Diagnostics: "Prüfen" checks the text (JFS: lib/script's check(); every language: an info
// at each "TODO"), "Beim Tippen" checks after every change; a second status line (once the
// text was checked) shows the counts (`dg: E/W/I`), the diagnostic at the caret (`at: ...`,
// ui.code_diag) and where the first six are now (`ds: line:col-line:col E|W|I`, they follow
// the edits until the next check).  Commands for scripts: `cmd 900` diagnostics over lines,
// at a point and stacked on the Rust sample, `cmd 901` none, `cmd 902` one on every 7th line
// of "100 000 Zeilen" that calls compute(), `cmd 903` CE_MARKS on / off.
// Fonts: DejaVu Sans Mono (the code editor, mono_font_set; also registered as a family) and
// DejaVu Serif (registered for the rich text) from lib/doc_view/assets (licence:
// DejaVu-LICENSE.txt there).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod model;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

const DESC: AppDesc = AppDesc { title: "jfgui Editoren", flags: AF_RESIZABLE, w: 1000, h: 680, min_w: 480, min_h: 360, ..AppDesc::NONE };

static MONO: &[u8] = include_bytes!("../../lib/doc_view/assets/DejaVuSansMono.ttf");
static MONO_B: &[u8] = include_bytes!("../../lib/doc_view/assets/DejaVuSansMono-Bold.ttf");
static SERIF: &[u8] = include_bytes!("../../lib/doc_view/assets/DejaVuSerif.ttf");
static SERIF_B: &[u8] = include_bytes!("../../lib/doc_view/assets/DejaVuSerif-Bold.ttf");

const ID_TABS: u32 = wid("tabs");
const ID_RICH: u32 = 2000; // .. 2005
const ID_TOOL: u32 = 2100; // .. 2199
const ID_CODE: u32 = 3000; // .. 3015
const ID_RO: u32 = wid("readonly");
const ID_SAMPLE: u32 = wid("sample");
const ID_CLEAR: u32 = wid("clear");
const ID_JSON: u32 = wid("json");
const ID_HTML: u32 = wid("html");
const ID_LANG: u32 = wid("lang");
const ID_BIG: u32 = wid("big");
const ID_TABS_W: u32 = wid("tabs4");
const ID_USE_TABS: u32 = wid("use_tabs");
const ID_HC: u32 = wid("contrast");
const ID_IME: u32 = wid("ime");
const ID_CHECK: u32 = wid("check");
const ID_LIVE: u32 = wid("live");

// commands of the scripts (App::command)
const CMD_DIAGS: u32 = 900;
const CMD_NO_DIAGS: u32 = 901;
const CMD_BIG_DIAGS: u32 = 902;
const CMD_MARKS: u32 = 903;

const LANGS: [&str; 7] = ["Rust", "JSON", "INI", "TOML", "Markdown", "Text", "JFS"];
const JFS: i32 = 6;

struct EdTest {
    tab: i32,
    rich: RichDoc,
    ro: i32,
    link: String,
    code: CodeDoc,
    lang: i32,
    tabs8: i32,
    use_tabs: i32,
    hc: i32,
    ime: bool,
    focus_pending: bool,
    live: i32,
    // CE_MARKS (cmd 903)
    marks: bool,
    // the diagnostics' status line: shown once the document was checked (the editor keeps
    // its height in the other tests)
    diag_row: bool,
}

/// the sample document of the rich text tab
pub fn sample_rich() -> RichDoc {
    let red = Fmt { color: 0xFFC00000, ..Fmt::default() };
    let big = Fmt { size: 160, ..Fmt::default() };
    let serif = Fmt { font: String::from("DejaVu Serif"), ..Fmt::default() };
    let hl = Fmt { bg: 0xFFFFF27A, ..Fmt::default() };
    RichDoc::from_paras(alloc::vec![
        Para::new().with("Fertiges Rich-Text-Widget", Fmt::default()).attrs(RA_LEFT, LS_NONE, 0, PS_H1),
        Para::new()
            .with("Ein Absatz mit ", Fmt::default())
            .with("fett", Fmt::bold())
            .with(", ", Fmt::default())
            .with("kursiv", Fmt::italic())
            .with(", ", Fmt::default())
            .with("unterstrichen", Fmt::flags(RF_UNDER))
            .with(", ", Fmt::default())
            .with("durchgestrichen", Fmt::flags(RF_STRIKE))
            .with(", ", Fmt::default())
            .with("rot", red)
            .with(", ", Fmt::default())
            .with("groß", big)
            .with(", ", Fmt::default())
            .with("Serifen", serif)
            .with(", ", Fmt::default())
            .with("markiert", hl)
            .with(" und einem ", Fmt::default())
            .with("Link", Fmt::link("https://example.org/jflib"))
            .with(".", Fmt::default()),
        Para::new().with("Listen", Fmt::default()).attrs(RA_LEFT, LS_NONE, 0, PS_H2),
        Para::plain("Erster Punkt").attrs(RA_LEFT, LS_BULLET, 0, PS_NORMAL),
        Para::plain("Zweiter Punkt mit Unterpunkt").attrs(RA_LEFT, LS_BULLET, 0, PS_NORMAL),
        Para::plain("Unterpunkt").attrs(RA_LEFT, LS_BULLET, 1, PS_NORMAL),
        Para::plain("Schritt eins").attrs(RA_LEFT, LS_NUMBER, 0, PS_NORMAL),
        Para::plain("Schritt zwei").attrs(RA_LEFT, LS_NUMBER, 0, PS_NORMAL),
        Para::plain("Teilschritt").attrs(RA_LEFT, LS_NUMBER, 1, PS_NORMAL),
        Para::plain("Ein Zitat steht eingerückt mit einem Balken davor.").attrs(RA_LEFT, LS_NONE, 0, PS_QUOTE),
        Para::plain("Zentrierter Absatz").attrs(RA_CENTER, LS_NONE, 0, PS_NORMAL),
        Para::plain(
            "Blocksatz verteilt den Platz einer Zeile auf ihre Leerzeichen, so dass beide Ränder gerade sind; die letzte Zeile des Absatzes bleibt linksbündig, wie es sich gehört."
        )
        .attrs(RA_JUSTIFY, LS_NONE, 0, PS_NORMAL),
        Para::plain("Rechtsbündig").attrs(RA_RIGHT, LS_NONE, 0, PS_NORMAL),
    ])
}

const RUST_SAMPLE: &str = r##"// A sample of Rust: comments, /* nested /* block */ comments */, strings
#![allow(dead_code)]
use core::fmt::Write;

/// a doc comment
#[derive(Clone, Debug, PartialEq)]
pub struct Point<'a> {
    pub name: &'a str,
    x: f64,
    y: f64,
}

/* a block comment
   over /* several */ lines */
impl<'a> Point<'a> {
    pub fn new(name: &'a str, x: f64, y: f64) -> Self {
        let raw = r#"raw "string" with \n no escapes"#;
        let text = "escapes: \t \"quoted\" \u{1F600}\n";
        let c = 'x';
        let nl = '\n';
        let n = 0x1F_u32 + 0b1010 + 1_000 + 2.5e-3 as u32;
        println!("{} {} {} {} {}", raw, text, c, nl, n);
        if x > 0.0 && y >= 0.0 { Point { name, x, y } } else { Point { name, x: 0.0, y: 0.0 } }
    }
}

fn main() {
    let p = Point::new("origin", 1.0, 2.0);
    match Some(p.clone()) {
        Some(q) if q == p => {}
        _ => unreachable!(),
    }
}
"##;

const JSON_SAMPLE: &str = r#"{
    "name": "jflib",
    "version": 1.5,
    "enabled": true,
    "tags": ["ui", "no_std", "rust"],
    "nested": { "count": -42, "ratio": 3.25e2, "none": null },
    "escaped": "line\nbreak \"quote\""
}
"#;

const INI_SAMPLE: &str = r#"; INI: comments, sections, keys and values
[window]
width = 1024
height=768
title = "Editor Test"
maximized = yes

# another comment
[recent files]
file1 = C:\Users\demo\notes.txt
path: /home/demo/notes.md
"#;

const TOML_SAMPLE: &str = r#"# TOML
title = "TOML example"

[owner]
name = "Tom"
dob = 1979-05-27T07:32:00-08:00

[database]
enabled = true
ports = [ 8000, 8001, 8002 ]
temp_targets = { cpu = 79.5, case = 72.0 }
motd = """
Several lines
of text"""
literal = 'C:\path'

[[products]]
name = "Hammer"
sku = 738_594_937
"#;

const MD_SAMPLE: &str = r#"# Markdown subset

Some *emphasis*, **strong text**, `inline code` and a [link](https://example.org).
A bare URL: https://example.org/docs and an escaped \*star\*.

## Lists

- first item
- second item
  - nested item
1. numbered
2. list

> a quotation

```rust
fn main() { println!("fenced code"); }
```

---
<!-- an HTML comment -->
"#;

const TEXT_SAMPLE: &str = "Plain text without highlighting.\n\tA tab, wide characters 漢字 and an emoji 😀.\n";

/// a script of lib/script with interpolations, escapes, numbers, comments and (at the end)
/// an error, a warning and another error for "Prüfen"
pub const JFS_SAMPLE: &str = r#"// JFS: a request counter for a virtual host (lib/script)
let hits = 0
let names = ["web", "db"]

fn greet(name) {
    let n = len(name)
    return "Hallo ${name}, ${n} Zeichen: ${ {k: n}.k }\n"
}

/* the handler counts the requests
   and answers in JSON */
fn handle(req) {
    hits += 1
    if req.path == "/count" && hits > 0x10 {
        return {status: 200, body: "${hits} Aufrufe \u{2713}"}
    }
    for i in 0..3 { log("i=${i}") }
    let big = 1_000_000 + 0b1010 + 1.5e-3
    return nil ?? greet("du")
}

// TODO: limits per client
let hits = 1
pirnt(greet("Welt"))
break
"#;

fn sample_code(lang: i32) -> &'static str {
    [RUST_SAMPLE, JSON_SAMPLE, INI_SAMPLE, TOML_SAMPLE, MD_SAMPLE, TEXT_SAMPLE, JFS_SAMPLE][lang.clamp(0, 6) as usize]
}

fn highlighter(lang: i32) -> Highlighter {
    match lang {
        0 => Highlighter::rust(),
        1 => Highlighter::json(),
        2 => Highlighter::ini(),
        3 => Highlighter::toml(),
        4 => Highlighter::markdown(),
        JFS => Highlighter::jfs(),
        _ => Highlighter::plain(),
    }
}

/// the checks of the code tab: lib/script's for JFS (errors, warnings), an info at every
/// "TODO" in any language
pub fn check_code(doc: &CodeDoc, jfs: bool) -> Vec<Diag> {
    let mut d = Vec::new();
    if jfs {
        for x in script::check(&doc.text()) {
            let sev = if x.error { DIAG_ERROR } else { DIAG_WARNING };
            d.push(Diag::new(x.line as usize, x.col as usize, x.end_col as usize, sev, &x.msg));
        }
    }
    for (l, t) in doc.lines().iter().enumerate() {
        if let Some(b) = t.find("TODO") {
            let c = t[..b].chars().count() + 1;
            d.push(Diag::new(l + 1, c, c + 4, DIAG_INFO, "Offene Aufgabe"));
        }
    }
    d
}

/// where the diagnostics are: "line:col-line:col E|W|I", the first n
pub fn diag_list(doc: &CodeDoc, n: usize) -> String {
    let v: Vec<String> = (0..n)
        .map_while(|i| doc.diagnostic(i))
        .map(|x| format!("{}:{}-{}:{} {}", x.line, x.col, x.end_line, x.end_col, ["E", "W", "I"][x.severity.min(2) as usize]))
        .collect();
    v.join(", ")
}

/// a generated Rust file of n lines
pub fn big_code(n: usize) -> CodeDoc {
    let mut lines: Vec<String> = Vec::with_capacity(n);
    let mut i = 0;
    while lines.len() < n {
        if i % 50 == 0 {
            lines.push(format!("/// function {}", i / 50));
            lines.push(format!("pub fn function_{:05}(x: u32) -> u32 {{", i / 50));
        } else if i % 50 == 49 {
            lines.push(String::from("    x // the result"));
            lines.push(String::from("}"));
        } else {
            lines.push(format!("    let value_{:06} = compute(x, {}, \"text {}\"); // line {}", i, i % 97, i, lines.len() + 1));
        }
        i += 1;
    }
    lines.truncate(n);
    CodeDoc::from_lines(lines)
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}

/// the document as a compact markup: paragraphs "|"-separated, attributes in (), runs with
/// formats in [] .. [/]
pub fn markup(d: &RichDoc) -> String {
    let mut s = String::new();
    for (i, p) in d.paras().iter().enumerate() {
        if i > 0 {
            s.push_str(" | ");
        }
        let mut attrs: Vec<String> = Vec::new();
        match p.align {
            RA_CENTER => attrs.push(String::from("center")),
            RA_RIGHT => attrs.push(String::from("right")),
            RA_JUSTIFY => attrs.push(String::from("justify")),
            _ => {}
        }
        match p.list {
            LS_BULLET => attrs.push(String::from("bullet")),
            LS_NUMBER => attrs.push(String::from("number")),
            _ => {}
        }
        if p.level > 0 {
            attrs.push(format!("level{}", p.level));
        }
        match p.style {
            PS_H1 => attrs.push(String::from("h1")),
            PS_H2 => attrs.push(String::from("h2")),
            PS_H3 => attrs.push(String::from("h3")),
            PS_QUOTE => attrs.push(String::from("quote")),
            _ => {}
        }
        if !attrs.is_empty() {
            s.push('(');
            s.push_str(&attrs.join(","));
            s.push_str(") ");
        }
        for (t, f) in p.runs() {
            let mut tags: Vec<String> = Vec::new();
            for (bit, n) in [(RF_BOLD, "b"), (RF_ITALIC, "i"), (RF_UNDER, "u"), (RF_STRIKE, "s")] {
                if f.flags & bit != 0 {
                    tags.push(String::from(n));
                }
            }
            if f.color != 0 {
                tags.push(format!("c={}", hex(f.color)));
            }
            if f.bg != 0 {
                tags.push(format!("bg={}", hex(f.bg)));
            }
            if f.size != 0 {
                tags.push(format!("sz={}", f.size));
            }
            if !f.font.is_empty() {
                tags.push(format!("f={}", f.font));
            }
            if !f.link.is_empty() {
                tags.push(format!("l={}", f.link));
            }
            if tags.is_empty() {
                s.push_str(t);
            } else {
                s.push('[');
                s.push_str(&tags.join(" "));
                s.push(']');
                s.push_str(t);
                s.push_str("[/]");
            }
        }
    }
    s
}

impl EdTest {
    fn new() -> EdTest {
        let mut code = CodeDoc::from_text(RUST_SAMPLE);
        code.tab_width = 4;
        EdTest {
            tab: 0,
            rich: sample_rich(),
            ro: 0,
            link: String::new(),
            code,
            lang: 0,
            tabs8: 0,
            use_tabs: 0,
            hc: 0,
            ime: false,
            focus_pending: false,
            live: 0,
            marks: false,
            diag_row: false,
        }
    }

    fn rich_tab(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_SAMPLE, "Beispiel") & EV_ACT != 0 {
            self.rich = sample_rich();
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_CLEAR, "Leeren") & EV_ACT != 0 {
            self.rich = RichDoc::new();
            ui.focus(ID_RICH);
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_JSON, "JSON hin und zurück") & EV_ACT != 0 {
            if let Some(mut d) = RichDoc::from_json(&self.rich.to_json()) {
                d.set_selection(self.rich.selection());
                self.rich = d;
            }
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_HTML, "HTML hin und zurück") & EV_ACT != 0 {
            let mut d = RichDoc::from_html(&self.rich.to_html());
            d.set_selection(self.rich.selection());
            self.rich = d;
        }
        ui.checkbox(ID_RO, "Nur lesen", &mut self.ro);
        ui.box_end();
        if self.ro == 0 {
            ui.richtext_toolbar(ID_TOOL, ID_RICH, &mut self.rich, RTB_ALL);
        }
        ui.next_size(-1, -1);
        let flags = if self.ro != 0 { RT_READONLY } else { 0 };
        ui.richtext(ID_RICH, &mut self.rich, flags);
        if let Some(l) = ui.richtext_link(ID_RICH) {
            self.link = l;
        }
        let sel = self.rich.selection();
        let f = self.rich.current_fmt();
        let lines = [
            format!("doc: {}", markup(&self.rich)),
            format!("sel: {}:{}-{}:{}", sel.anchor.line, sel.anchor.byte, sel.caret.line, sel.caret.byte),
            format!("fmt: {}{}{}{}", if f.flags & RF_BOLD != 0 { "b" } else { "-" }, if f.flags & RF_ITALIC != 0 { "i" } else { "-" }, if f.flags & RF_UNDER != 0 { "u" } else { "-" }, if f.flags & RF_STRIKE != 0 { "s" } else { "-" }),
            format!("rev: {} undo: {}", self.rich.revision(), self.rich.can_undo() as u8),
            format!("link: {}", self.link),
        ];
        for l in &lines {
            ui.text(TS_CAPTION | TX_SECONDARY, l);
        }
    }

    fn code_tab(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        let old = self.lang;
        ui.tabs(ID_LANG, &mut self.lang, &LANGS);
        if self.lang != old {
            self.code = CodeDoc::from_text(sample_code(self.lang));
            self.code.tab_width = if self.tabs8 != 0 { 8 } else { 4 };
            self.code.use_tabs = self.use_tabs != 0;
            self.diag_row = self.live != 0;
        }
        ui.spacer(-1);
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_BIG, "100 000 Zeilen") & EV_ACT != 0 {
            self.lang = 0;
            self.code = big_code(100_000);
            self.diag_row = self.live != 0;
        }
        if ui.checkbox(ID_TABS_W, "Tabweite 8", &mut self.tabs8) & EV_ACT != 0 {
            self.code.tab_width = if self.tabs8 != 0 { 8 } else { 4 };
        }
        if ui.checkbox(ID_USE_TABS, "Tabs", &mut self.use_tabs) & EV_ACT != 0 {
            self.code.use_tabs = self.use_tabs != 0;
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_CHECK, "Prüfen") & EV_ACT != 0 {
            let d = check_code(&self.code, self.lang == JFS);
            self.code.set_diagnostics(&d);
            self.diag_row = true;
        }
        if ui.checkbox(ID_LIVE, "Beim Tippen", &mut self.live) & EV_ACT != 0 && self.live != 0 {
            self.diag_row = true;
        }
        ui.box_end();
        ui.next_size(-1, -1);
        let hl = highlighter(self.lang);
        let flags = if self.marks { CE_MARKS } else { 0 };
        if ui.code_editor(ID_CODE, &mut self.code, &hl, flags) & EV_ACT != 0 && self.live != 0 {
            // (the editor drew the old ones in this pass)
            let d = check_code(&self.code, self.lang == JFS);
            self.code.set_diagnostics(&d);
            ui.repass();
        }
        let st = ui.code_status(ID_CODE);
        let cs = self.code.selection();
        let lines = [
            format!("pos: {}:{}", st.line, st.col),
            format!("sel: {}:{}-{}:{}", cs.anchor.line, cs.anchor.byte, cs.caret.line, cs.caret.byte),
            format!("find: {}/{}", st.current, st.matches),
            format!("hl: {}", st.hl_lines),
            format!("lines: {}", st.lines),
            format!("rev: {} undo: {}", self.code.revision(), self.code.can_undo() as u8),
            format!("lang: {}", hl.name()),
        ];
        ui.box_begin(0, BOX_H, 16);
        for l in &lines {
            ui.text(TS_CAPTION | TX_SECONDARY, l);
        }
        ui.box_end();
        // the diagnostics: counts, the one at the caret, where the first ones are now
        if !self.diag_row {
            return;
        }
        let sev = |d: &Diag| ["E", "W", "I"][d.severity.min(2) as usize];
        let at = ui.code_diag(ID_CODE).map_or(String::from("-"), |d| format!("{} {}:{} {}", sev(&d), d.line, d.col, d.msg));
        let lines = [
            format!("dg: {}/{}/{}", st.diags[0], st.diags[1], st.diags[2]),
            format!("at: {}", at),
            format!("ds: {}", diag_list(&self.code, 6)),
        ];
        ui.box_begin(0, BOX_H, 16);
        for l in &lines {
            ui.text(TS_CAPTION | TX_SECONDARY, l);
        }
        ui.box_end();
    }

    // the diagnostics of the script commands (CMD_DIAGS: on the Rust sample, CMD_BIG_DIAGS:
    // on "100 000 Zeilen")
    fn test_diags(&mut self, cmd: u32) {
        let d = match cmd {
            CMD_DIAGS => alloc::vec![
                Diag::span(2, 18, 5, 4, DIAG_ERROR, "Ein Teil über vier Zeilen"),
                Diag::new(1, 200, 0, DIAG_WARNING, "Ein Punkt am Zeilenende"),
                Diag::new(6, 1, 4, DIAG_INFO, "Ein Hinweis"),
                Diag::new(6, 3, 7, DIAG_WARNING, "Eine Warnung darüber"),
            ],
            CMD_BIG_DIAGS => {
                let mut v = Vec::new();
                for (l, t) in self.code.lines().iter().enumerate().step_by(7) {
                    if let Some(b) = t.find("compute") {
                        let c = t[..b].chars().count() + 1;
                        v.push(Diag::new(l + 1, c, c + 7, (l / 7 % 3) as u8, "compute: eine Meldung"));
                    }
                }
                v
            }
            _ => Vec::new(),
        };
        self.code.set_diagnostics(&d);
        self.diag_row = true;
    }
}

impl App for EdTest {
    fn init(&mut self, ui: &mut Ui) -> bool {
        let m = ui.text.load_mem(MONO, 0);
        if m >= 0 {
            ui.mono_font_set(m as u32);
            ui.font_register("DejaVu Sans Mono", 400, false, m as u32);
        }
        let mb = ui.text.load_mem(MONO_B, 0);
        if mb >= 0 {
            ui.font_register("DejaVu Sans Mono", 700, false, mb as u32);
        }
        let s = ui.text.load_mem(SERIF, 0);
        if s >= 0 {
            ui.font_register("DejaVu Serif", 400, false, s as u32);
        }
        let sb = ui.text.load_mem(SERIF_B, 0);
        if sb >= 0 {
            ui.font_register("DejaVu Serif", 700, false, sb as u32);
        }
        self.focus_pending = true;
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | pad(12), 8);
        let old = self.tab;
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.tabs(ID_TABS, &mut self.tab, &["Rich-Text", "Code"]);
        ui.spacer(-1);
        ui.next_flags(ST_NOFOCUS);
        // an IME composition as IBus / Wayland report it (the editors draw it at the caret)
        ui.next_flags(ST_NOFOCUS);
        if ui.button(ID_IME, "IME-Vorschau") & EV_ACT != 0 {
            self.ime = !self.ime;
            if self.ime { plat::ime_preedit_set(0, "Vorschau", 3) } else { plat::ime_preedit_set(0, "", 0) }
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_HC, "Hoher Kontrast", &mut self.hc) & EV_ACT != 0 {
            ui.set_contrast(Some(self.hc != 0));
        }
        ui.box_end();
        if self.tab != old {
            self.focus_pending = true;
        }
        if self.focus_pending {
            self.focus_pending = false;
            ui.focus(if self.tab == 0 { ID_RICH } else { ID_CODE });
        }
        if self.tab == 0 {
            self.rich_tab(ui);
        } else {
            self.code_tab(ui);
        }
        ui.box_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        match id {
            CMD_DIAGS | CMD_NO_DIAGS | CMD_BIG_DIAGS => {
                self.test_diags(id);
                ui.repass();
                true
            }
            CMD_MARKS => {
                self.marks = !self.marks;
                ui.repass();
                true
            }
            _ => false,
        }
    }
}

fn editortest() -> i32 {
    if plat::arg_value("--test").as_deref() == Some("model") {
        return model::run();
    }
    ui::launch(&DESC, EdTest::new(), &style::STYLE)
}

ui::main!(editortest());

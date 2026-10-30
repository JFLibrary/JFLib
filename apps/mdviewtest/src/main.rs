// mdviewtest -- the Markdown / HTML viewer (ui.markdown_view, ui.html_view: lib/ui/md_view.rs,
// md_doc.rs) and lists whose rows have different heights (ui.vlist_begin: lib/ui/vlist.rs)
//
//   build/win/mdviewtest.exe --test unit     self-checks of the parser and the height cache
//                                            (unit.rs); exit code = failures
//   build/win/mdviewtest.exe --script apps/mdviewtest/tests/shots.uis --verify-dirty
//   build/win/mdviewtest.exe                 a window
//
// Tab "Markdown": a sample with every element (headings, emphasis, links, lists, task items,
// quotes, code with highlighting, a table, images from app:// (assets/logo.svg, raster.png)
// and one that is never fetched, rules, inline HTML, Hebrew and Arabic paragraphs).  Below
// it: the last activated link, the link under the pointer, the selection's length; the button
// "Zwischenablage" shows the clipboard's text and HTML (line ends as ⏎); "Langes Dokument"
// shows 10 000 sections (50 000 blocks) instead of the sample.  Tab "HTML": the same kind of
// document through html_view.  Tab "Hebräisch": a document in Hebrew (and a paragraph in
// Arabic, one in English): it runs right to left.  Tab "Liste": 100 000 rows of different
// heights (a title, a text of 2 .. 33 words, wrapped); Enter or a double click opens / closes a row
// (more text: it grows), "Oben einfügen" inserts five rows above (the view stays where it
// is), "Zu Zeile 50000" selects and reveals that row; the status line shows the selection and
// the first visible row.  "Hoher Kontrast", "Rechts nach links" (the mirrored window) and
// "Links öffnen" (MV_OPEN_LINKS) switch for every tab.  Code is set in DejaVu Sans Mono
// (lib/doc_view/assets, licence: DejaVu-LICENSE.txt there), so the layout is the same on every
// system; Hebrew and Arabic come from the system's fonts (set_system_fallback).
//
//   build/win/mdviewtest.exe --page form --script apps/mdviewtest/tests/form_keys.uis
//
// --page form: an HTML page with a form (text, password, check box, radios, a drop-down list, a
// text area, a hidden field, a field in a Hebrew paragraph, submit, reset) and images from the
// network (MI_ASK: the app supplies them; one more from app://).  Below it: the last field the
// user changed ("Feld: name"), the last form sent ("Senden: method action name=value&..."),
// the image addresses the view asked for ("Bilder: ..."); "Bilder liefern" answers them (two
// from app://logo.svg and raster.png, the third fails), "Hoher Kontrast" and "Rechts nach
// links" as on the tabs.
//
//   build/win/mdviewtest.exe --page dom --script apps/mdviewtest/tests/dom_steps.uis
//
// --page dom: an HTML page the app changes step by step through the document's elements (as a
// page script would: MdViewState::doc_mut, md_dom.rs) -- F5 or "Schritt" takes the next step:
// the counter in "#stand b" (every step), a list item inserted, a text field inserted before
// the focused one, the note hidden, the link's address and text, a list item removed and the
// heading's text, three paragraphs inserted above the view, the heading's HTML.  Below it: the
// step, the focused field, the selection's length, the last form sent.
//
//   build/win/mdviewtest.exe --page css --script apps/mdviewtest/tests/css_shots.uis
//
// --page css: an HTML page with a style sheet (rich_html.rs): a head bar, cards with border,
// padding and background, a note with a coloured border on the left, a list with a background,
// a dark box, a table with a border and a tinted head row, rules by tag, class, id, descendant
// and child, a hidden paragraph, a form in a card; below it the last link activated and the
// last form sent.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod unit;

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

static MONO: &[u8] = include_bytes!("../../../lib/doc_view/assets/DejaVuSansMono.ttf");

const DESC: AppDesc = AppDesc { title: "jfgui Markdown", flags: AF_RESIZABLE, w: 1000, h: 720, min_w: 480, min_h: 360, ..AppDesc::NONE };

const ID_TABS: u32 = wid("tabs");
const ID_HC: u32 = wid("contrast");
const ID_RTL: u32 = wid("rtl");
const ID_OPEN: u32 = wid("open");
const ID_MD: u32 = wid("md");
const ID_HTML: u32 = wid("html");
const ID_HE: u32 = wid("hebrew");
const ID_CLIP: u32 = wid("clip");
const ID_LONG: u32 = wid("long");
const ID_INSERT: u32 = wid("insert");
const ID_JUMP: u32 = wid("jump");
const ID_FORM: u32 = wid("form");
const ID_SUPPLY: u32 = wid("supply");
const ID_DOM: u32 = wid("dom");
const ID_CSS: u32 = wid("css");
const ID_STEP: u32 = wid("step");
// the list and its rows: ID_LIST + 1 + i
const ID_LIST: u32 = 0x0100_0000;
const ROWS: u32 = 100_000;

/// the Markdown sample (also parsed by unit.rs)
pub(crate) const SAMPLE_MD: &str = r#"# Markdown-Betrachter

Ein Absatz mit *kursiv*, **fett**, ***beidem***, ~~durchgestrichen~~, `Code im Text` und einem [Link](https://example.org/jflib "Titel"). Bare Adressen werden Links: https://example.org/docs und www.example.com. Ein harter Umbruch\
nach einem Backslash und ein weicher
Umbruch.

## Listen

- Erster Punkt
- Zweiter Punkt mit Unterliste
  - Unterpunkt
    - Noch tiefer
- Dritter Punkt

3. Schritt drei
4. Schritt vier

- [x] Erledigte Aufgabe
- [ ] Offene Aufgabe

> Ein Zitat mit **Fettdruck**
> und einer zweiten Zeile.
>
> > Ein verschachteltes Zitat.

## Code

```rust
fn main() {
    let n = 42; // ein Kommentar
    println!("Hallo, {}!", n);
}
```

    eingerückter Code
    zweite Zeile

## Tabelle

| Name | Menge | Preis |
|:-----|:-----:|------:|
| Äpfel | 3 | 1,20 € |
| Birnen mit einem langen Namen | 12 | 0,80 € |
| `a\|b` | **7** | [Preisliste](https://example.org/preis) |

## Bilder

![Logo](logo.svg) und ein Rasterbild ![Raster](raster.png)

![Nicht geladen](https://example.org/bild.png)

---

#### Überschrift 4
##### Überschrift 5
###### Überschrift 6

Escapes: \*kein Stern\*, &copy; &amp; &#x263A;. HTML im Text: <b>fett</b>, <kbd>Strg</kbd>+<kbd>C</kbd>,<br>eine neue Zeile.

[ref]: https://example.org/ref
Ein [Verweis][ref] und [ref] als Kurzform.

שלום עולם, זהו טקסט בעברית.

مرحبا بالعالم، هذا نص عربي.
"#;

/// a document in Hebrew: it runs right to left (indents, markers, quote bars on the right)
pub(crate) const SAMPLE_HE: &str = r#"# מדריך קצר

פסקה עם **הדגשה**, *נטוי*, `code` ו[קישור](https://example.org/he).

## רשימות

- פריט ראשון
- פריט שני
  - פריט מקונן
1. שלב אחד
2. שלב שניים
- [x] משימה שהושלמה
- [ ] משימה פתוחה

> ציטוט בעברית.

| שם | כמות |
|----|-----:|
| תפוחים | 3 |
| אגסים | 12 |

مرحبا بالعالم: فقرة عربية مع **نص غامق**.

English inside: the word *Hello* stays left to right.
"#;

/// the HTML sample
pub(crate) const SAMPLE_HTML: &str = r#"<h1>HTML-Betrachter</h1>
<p>Ein Absatz mit <i>kursiv</i>, <b>fett</b>, <s>durchgestrichen</s>, <u>unterstrichen</u>,
<code>Code</code>, <span style="color:#c00000">rot</span> und einem
<a href="https://example.org/html">Link</a>.</p>
<h2>Listen</h2>
<ul><li>Erster Punkt</li><li>Zweiter Punkt<ul><li>Unterpunkt</li></ul></li></ul>
<ol start="5"><li>Fünf</li><li>Sechs</li></ol>
<blockquote><p>Ein Zitat.</p></blockquote>
<pre><code class="language-json">{ "name": "jflib", "n": 1 }</code></pre>
<table>
<thead><tr><th>Spalte A</th><th align="right">Spalte B</th></tr></thead>
<tbody><tr><td>eins</td><td align="right">1</td></tr><tr><td>zwei</td><td align="right">22</td></tr></tbody>
</table>
<hr>
<p><img src="logo.svg" alt="Logo" width="64"> Ein Bild im Text.</p>
<h4>Überschrift 4</h4>
<p>Ende.</p>
"#;

/// the page with a form and images from the network (--page form)
pub(crate) const SAMPLE_FORM: &str = r#"<h2>Anmeldung</h2>
<form action="/login" method="post">
<p><label for="user">Name</label> <input id="user" name="user" placeholder="Benutzername" size="24"></p>
<p><label>Passwort <input type="password" name="pw"></label></p>
<p><label><input type="checkbox" name="keep" value="ja"> Angemeldet bleiben</label></p>
<p>Farbe: <label><input type="radio" name="farbe" value="rot" checked> Rot</label>
<label><input type="radio" name="farbe" value="gruen"> Grün</label>
<label><input type="radio" name="farbe" value="blau"> Blau</label></p>
<p><label for="land">Land</label> <select id="land" name="land"><option value="de">Deutschland</option>
<option value="at" selected>Österreich</option><option value="ch">Schweiz</option></select></p>
<p><label for="text">Nachricht</label><br><textarea id="text" name="text" rows="3" cols="30">
Hallo</textarea></p>
<p><label for="ort">עיר</label> <input id="ort" name="ort" size="10"></p>
<input type="hidden" name="token" value="abc123">
<p><button type="submit" name="aktion" value="senden">Senden</button> <input type="submit" name="aktion" value="Speichern">
<input type="reset" value="Zurücksetzen"> <a href="https://example.org/hilfe">Hilfe</a></p>
</form>
<h2>Bilder aus dem Netz</h2>
<p><img src="http://example.org/logo.svg" alt="Logo aus dem Netz" width="96" height="48">
<img src="https://example.org/raster.png" alt="Raster aus dem Netz">
<img src="https://example.org/fehlt.png" alt="Fehlt"> und lokal <img src="logo.svg" alt="Logo lokal" width="48"></p>
"#;

/// a page with a style sheet (--page css)
pub(crate) const SAMPLE_CSS: &str = r#"<style>
h1 { color: #0b3d91; border-bottom: 3px solid #0b3d91; padding-bottom: 4px }
.kopf { background: #0b3d91; color: #ffffff; padding: 6px 12px; margin-bottom: 4px }
.kopf a { color: #ffd166 }
.karte { border: 1px solid #b8c4d2; background: #f4f7fb; padding: 8px 14px; margin: 4px 0 }
.karte p { margin: 2px 0 }
.hinweis { border-left: 4px solid #e09f3e; background: #fff4e0; padding: 4px 10px }
#wichtig { color: #b00020; font-weight: bold }
ul.menu { background: #e8eef6; padding: 4px 8px }
ul.menu > li { color: #0b3d91 }
.dunkel { background: #222831; color: #eeeeee; padding: 6px 10px }
.dunkel .hell { color: #ffd166; font-style: italic }
table.daten { border: 2px solid #8899aa }
tr.kopfzeile { background: #dde6f0 }
.rechts { text-align: right }
.versteckt { display: none }
</style>
<div class="kopf"><p>Netzwerk-Labor · <a href="https://example.org/start">Start</a></p></div>
<h1>Gestaltete Seite</h1>
<div class="karte"><p>Eine Karte mit Rahmen, Innenabstand und Hintergrund.</p><p id="wichtig">Ein wichtiger Satz in Rot.</p><p>Ein <a href="https://example.org/karte">Link</a> in der Karte.</p></div>
<div class="hinweis"><p>Ein Hinweis mit einem farbigen Rand links.</p></div>
<ul class="menu"><li>Router</li><li>Switch</li><li>Server</li></ul>
<div class="dunkel"><p>Ein dunkler Kasten mit <span class="hell">hellem</span> Text.</p></div>
<table class="daten"><tr class="kopfzeile"><th>Gerät</th><th>Adresse</th></tr><tr><td>R1</td><td>10.0.0.1</td></tr></table>
<p class="rechts">Rechts ausgerichtet.</p>
<p class="versteckt">Unsichtbar.</p>
<div class="karte"><form action="/los"><p><label for="q">Suche</label> <input id="q" name="q" size="16"> <button>Los</button></p></form></div>
"#;

/// the page the app changes step by step (--page dom; the paragraphs of #kasten: dom_html)
pub(crate) const SAMPLE_DOM: &str = r#"<h2 id="kopf">Seite mit Skript</h2>
<p id="stand">Stand: <b>0</b> Schritte</p>
<ul id="liste"><li>Erster Eintrag</li><li class="zwei">Zweiter Eintrag</li></ul>
<form action="/suche"><p id="zeile"><label for="name">Name</label> <input id="name" name="name" size="16">
<label><input type="checkbox" name="merken"> Merken</label></p></form>
<p id="hinweis">Ein Hinweis, der beim vierten Schritt verschwindet.</p>
<p><a id="link" href="https://example.org/eins">Erster Link</a></p>
<div id="kasten"></div>
"#;

/// the page of --page dom: SAMPLE_DOM with 30 paragraphs in #kasten
pub(crate) fn dom_html() -> String {
    let mut s = String::from(SAMPLE_DOM);
    let mut ps = String::new();
    for k in 1..=30 {
        ps.push_str(&format!("<p>Absatz {}: {}.</p>\n", k, row_text(k)));
    }
    s = s.replace("<div id=\"kasten\"></div>", &format!("<div id=\"kasten\">\n{}</div>", ps));
    s
}

// the text of row key k: 2 .. 33 words
fn row_text(k: u32) -> String {
    const W: [&str; 8] = ["Zeilen", "haben", "verschiedene", "Höhen", "in", "einer", "virtuellen", "Liste"];
    let n = 2 + (k.wrapping_mul(2_654_435_761) >> 27) as usize;
    let mut s = String::new();
    for i in 0..n {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(W[(k as usize + i * 3) % 8]);
    }
    s
}

// a long document: 10 000 sections of a heading, a paragraph and a list (50 000 blocks), then
// three blocks that are hard to lay out
fn long_doc() -> String {
    let mut s = String::new();
    for i in 0..10_000u32 {
        s.push_str(&format!(
            "## Abschnitt {}\n\nAbsatz {} mit **fettem** Text, *Betonung* und einem [Link](https://example.org/{}): {}.\n\n- Punkt {}a\n- Punkt {}b\n\n",
            i, i, i, row_text(i), i, i
        ));
    }
    // hard cases for the layout at the end: a code line of 100 000 characters without a
    // break, a word of 20 000, a paragraph of 1 000 links
    s.push_str("```\n");
    s.push_str(&"x".repeat(100_000));
    s.push_str("\n```\n\n");
    s.push_str(&"Wort".repeat(5_000));
    s.push_str("\n\n");
    for k in 0..1_000 {
        s.push_str(&format!("[L{}](https://example.org/{}) ", k, k));
    }
    s.push('\n');
    s
}

fn row_title(k: u32) -> String {
    if k >= ROWS { format!("Neu {}", k - ROWS + 1) } else { format!("Zeile {}", k) }
}

struct MdTest {
    tab: i32,
    hc: i32,
    rtl: i32,
    open: i32,
    md: MdViewState,
    html: MdViewState,
    he: MdViewState,
    link: String,
    hover: String,
    clip: (String, String),
    // the long document (tab Markdown), the selection whose length is shown
    long: Option<String>,
    sel: ((i32, TextSel), usize),
    vl: VListState,
    // the rows' keys (inserted rows get new ones) and the open rows
    rows: Vec<u32>,
    added: u32,
    opened: BTreeSet<u32>,
    // --page form: the page, the last field changed, the last form sent, the image addresses
    // asked for (and not answered yet)
    page_form: bool,
    form: MdViewState,
    field: String,
    sent: String,
    asked: Vec<String>,
    waiting: Vec<String>,
    // --page dom: the page, the steps taken
    page_dom: bool,
    dom: MdViewState,
    step: u32,
    // --page css: the page with a style sheet
    page_css: bool,
    css: MdViewState,
}

impl MdTest {
    fn new() -> MdTest {
        let mut md = MdViewState::new();
        md.base = String::from("app://");
        md.label = String::from("Hilfe");
        let mut html = MdViewState::new();
        html.base = String::from("app://");
        let mut he = MdViewState::new();
        he.label = String::from("עברית");
        let mut form = MdViewState::new();
        form.base = String::from("app://");
        form.label = String::from("Formular");
        form.images = MI_ASK;
        let page_dom = plat::arg_value("--page").as_deref() == Some("dom");
        let mut dom = MdViewState::new();
        dom.label = String::from("Skript");
        if page_dom {
            dom.set_html(&dom_html());
        }
        let page_css = plat::arg_value("--page").as_deref() == Some("css");
        let mut css = MdViewState::new();
        css.label = String::from("Gestaltet");
        MdTest {
            tab: 0,
            hc: 0,
            rtl: 0,
            open: 0,
            md,
            html,
            he,
            link: String::new(),
            hover: String::new(),
            clip: (String::new(), String::new()),
            long: None,
            sel: ((-1, TextSel::default()), 0),
            vl: VListState::new(),
            rows: (0..ROWS).collect(),
            added: 0,
            opened: BTreeSet::new(),
            page_form: plat::arg_value("--page").as_deref() == Some("form"),
            form,
            field: String::new(),
            sent: String::new(),
            asked: Vec::new(),
            waiting: Vec::new(),
            page_dom,
            dom,
            step: 0,
            page_css,
            css,
        }
    }

    // --page css: the styled page, what it reported
    fn css_page(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.spacer(-1);
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_HC, "Hoher Kontrast", &mut self.hc) & EV_ACT != 0 {
            ui.set_contrast(Some(self.hc != 0));
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_RTL, "Rechts nach links", &mut self.rtl) & EV_ACT != 0 {
            ui.set_rtl(Some(self.rtl != 0));
        }
        ui.box_end();
        ui.next_size(-1, -1);
        let ev = ui.html_view(ID_CSS, &mut self.css, SAMPLE_CSS, 0);
        if let Some(l) = ev.link {
            self.link = l;
        }
        if let Some(s) = ev.submit {
            let pairs: Vec<String> = s.fields.iter().map(|(n, v)| format!("{}={}", n, v)).collect();
            self.sent = format!("{} {} {}", s.method, s.action, pairs.join("&"));
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.text(TS_BODY, &format!("Link: {}", if self.link.is_empty() { "-" } else { &self.link }));
        ui.text(TS_BODY, &format!("Senden: {}", if self.sent.is_empty() { "-" } else { &self.sent }));
        ui.box_end();
    }

    // --page dom: the next step, done through the document's elements as a page script would
    fn dom_step(&mut self) {
        self.step += 1;
        let k = self.step;
        let d = self.dom.doc_mut();
        // every step: the counter (a change inside one paragraph: that block alone again)
        if let Some(b) = d.select_first("#stand b") {
            d.set_text(b, &format!("{}", k));
        }
        match k {
            2 => {
                // a list item at the top of the list
                if let Some(l) = d.by_id("liste") {
                    let li = d.create_element("li");
                    let t = d.create_text("Neuer Eintrag");
                    d.insert(li, None, t);
                    let first = d.children(l).into_iter().find(|&c| d.tag(c) == "li");
                    d.insert(l, first, li);
                }
            }
            3 => {
                // a text field before the first one (the fields are numbered anew)
                if let Some(z) = d.by_id("zeile") {
                    let f = d.create_element("input");
                    d.set_attr(f, "name", "vorname");
                    d.set_attr(f, "placeholder", "Vorname");
                    d.set_attr(f, "size", "10");
                    let first = d.children(z).first().copied();
                    d.insert(z, first, f);
                    let sp = d.create_text(" ");
                    d.insert(z, first, sp);
                }
            }
            4 => {
                if let Some(h) = d.by_id("hinweis") {
                    d.set_attr(h, "hidden", "");
                }
            }
            5 => {
                if let Some(a) = d.by_id("link") {
                    d.set_attr(a, "href", "https://example.org/zwei");
                    d.set_text(a, "Zweiter Link");
                    d.add_class(a, "neu");
                }
            }
            6 => {
                if let Some(li) = d.select_first("#liste > li.zwei") {
                    d.free(li);
                }
                if let Some(h) = d.by_id("kopf") {
                    d.set_text(h, "Seite geändert");
                }
            }
            7 => {
                // three paragraphs at the top of the box (above the place the view shows)
                if let Some(b) = d.by_id("kasten") {
                    for i in (1..=3).rev() {
                        let p = d.create_element("p");
                        let t = d.create_text(&format!("Eingefügt {}", i));
                        d.insert(p, None, t);
                        let first = d.children(b).first().copied();
                        d.insert(b, first, p);
                    }
                }
            }
            8 => {
                if let Some(h) = d.by_id("kopf") {
                    d.set_inner_html(h, "Seite <i>fertig</i>");
                }
            }
            _ => {}
        }
    }

    // --page dom: the page, the step button, what the page reported
    fn dom_page(&mut self, ui: &mut Ui) {
        // (F5 before the view: the focused field keeps the focus)
        if ui.key(VK_F5) {
            self.dom_step();
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        if ui.button(ID_STEP, "Schritt") & EV_ACT != 0 {
            self.dom_step();
        }
        ui.spacer(-1);
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_HC, "Hoher Kontrast", &mut self.hc) & EV_ACT != 0 {
            ui.set_contrast(Some(self.hc != 0));
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_RTL, "Rechts nach links", &mut self.rtl) & EV_ACT != 0 {
            ui.set_rtl(Some(self.rtl != 0));
        }
        ui.box_end();
        ui.next_size(-1, -1);
        let ev = ui.doc_view(ID_DOM, &mut self.dom, 0);
        if let Some(s) = ev.submit {
            let pairs: Vec<String> = s.fields.iter().map(|(n, v)| format!("{}={}", n, v)).collect();
            self.sent = format!("{} {} {}", s.method, s.action, pairs.join("&"));
        }
        let st = &self.dom;
        let focus = st.focused_field().and_then(|i| st.doc().fields().get(i)).map_or(String::from("-"), |f| f.name.clone());
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.text(TS_BODY, &format!("Schritt: {}", self.step));
        ui.text(TS_BODY, &format!("Fokus: {}", focus));
        ui.text(TS_BODY, &format!("Auswahl: {} Zeichen", st.selected_text().chars().count()));
        ui.box_end();
        ui.text(TS_CAPTION | TX_WRAP, &format!("Senden: {}", if self.sent.is_empty() { "-" } else { &self.sent }));
    }

    // --page form: the form page, what it reported, the app as the image source
    fn form_page(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        if ui.button(ID_SUPPLY, "Bilder liefern") & EV_ACT != 0 {
            // two from app://, the third one fails
            for a in core::mem::take(&mut self.waiting) {
                let file = a.rsplit('/').next().unwrap_or("");
                let data = if file == "logo.svg" || file == "raster.png" { plat::read_file(&format!("app://{}", file)) } else { None };
                self.form.set_image(&a, data.as_deref());
            }
        }
        ui.spacer(-1);
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_HC, "Hoher Kontrast", &mut self.hc) & EV_ACT != 0 {
            ui.set_contrast(Some(self.hc != 0));
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_RTL, "Rechts nach links", &mut self.rtl) & EV_ACT != 0 {
            ui.set_rtl(Some(self.rtl != 0));
        }
        ui.box_end();
        ui.next_size(-1, -1);
        let ev = ui.html_view(ID_FORM, &mut self.form, SAMPLE_FORM, 0);
        let name = |st: &MdViewState, i: Option<usize>| i.and_then(|i| st.doc().fields().get(i)).map_or(String::from("-"), |f| f.name.clone());
        if ev.field.is_some() {
            self.field = name(&self.form, ev.field);
        }
        if let Some(s) = ev.submit {
            let pairs: Vec<String> = s.fields.iter().map(|(n, v)| format!("{}={}", n, v)).collect();
            self.sent = format!("{} {} {}", s.method, s.action, pairs.join("&"));
        }
        for a in self.form.image_requests() {
            self.asked.push(a.clone());
            self.waiting.push(a);
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.text(TS_BODY, &format!("Feld: {}", if self.field.is_empty() { "-" } else { &self.field }));
        ui.text(TS_BODY, &format!("Fokus: {}", name(&self.form, self.form.focused_field())));
        ui.box_end();
        ui.text(TS_CAPTION | TX_WRAP, &format!("Senden: {}", if self.sent.is_empty() { "-" } else { &self.sent }));
        ui.text(TS_CAPTION | TX_WRAP, &format!("Bilder: {}", self.asked.join(", ")));
    }

    // which: 0 Markdown, 1 HTML, 2 Hebrew
    fn doc(&mut self, ui: &mut Ui, which: i32) {
        let flags = if self.open != 0 { MV_OPEN_LINKS } else { 0 };
        ui.next_size(-1, -1);
        let ev = match which {
            1 => ui.html_view(ID_HTML, &mut self.html, SAMPLE_HTML, flags),
            2 => ui.markdown_view(ID_HE, &mut self.he, SAMPLE_HE, flags),
            _ => ui.markdown_view(ID_MD, &mut self.md, self.long.as_deref().unwrap_or(SAMPLE_MD), flags),
        };
        if let Some(l) = ev.link {
            self.link = l;
        }
        self.hover = ev.hover.unwrap_or_default();
        let st = match which {
            1 => &self.html,
            2 => &self.he,
            _ => &self.md,
        };
        // the selection's length (counted when it changes)
        if self.sel.0 != (which, st.selection()) {
            self.sel = ((which, st.selection()), st.selected_text().chars().count());
        }
        let n = self.sel.1;
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.text(TS_BODY, &format!("Link: {}", self.link));
        ui.text(TS_BODY, &format!("Zeiger: {}", if self.hover.is_empty() { "-" } else { &self.hover }));
        ui.text(TS_BODY, &format!("Auswahl: {} Zeichen", n));
        ui.spacer(-1);
        if which == 0 && ui.button(ID_LONG, if self.long.is_some() { "Beispiel" } else { "Langes Dokument" }) & EV_ACT != 0 {
            self.long = if self.long.is_some() { None } else { Some(long_doc()) };
        }
        if ui.button(ID_CLIP, "Zwischenablage") & EV_ACT != 0 {
            let get = |ui: &mut Ui, f: u32| ui.clip_get(f).map(|d| String::from_utf8_lossy(&d).replace('\n', "⏎").replace('\t', "⇥")).unwrap_or_default();
            self.clip = (get(ui, CLIP_TEXT), get(ui, CLIP_HTML));
        }
        ui.box_end();
        let cut = |s: &str| s.chars().take(120).collect::<String>();
        ui.text(TS_CAPTION | TX_WRAP, &format!("Text: {}", cut(&self.clip.0)));
        ui.text(TS_CAPTION | TX_WRAP, &format!("HTML: {}", cut(&self.clip.1)));
    }

    fn list(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        if ui.button(ID_INSERT, "Oben einfügen") & EV_ACT != 0 {
            let keys: Vec<u32> = (0..5).map(|i| ROWS + self.added + i).collect();
            self.added += 5;
            self.rows.splice(0..0, keys);
            self.vl.insert(0, 5);
        }
        if ui.button(ID_JUMP, "Zu Zeile 50000") & EV_ACT != 0 {
            if let Some(i) = self.rows.iter().position(|&k| k == 50_000) {
                self.vl.sel = i as i32;
                self.vl.reveal(i);
            }
        }
        let (top, off) = self.vl.first_visible();
        let sel = if self.vl.sel >= 0 { row_title(self.rows[self.vl.sel as usize]) } else { String::from("-") };
        ui.text(TS_BODY, &format!("Auswahl: {} · oben: {} +{}", sel, row_title(self.rows.get(top).copied().unwrap_or(0)), off));
        ui.box_end();
        ui.next_size(-1, -1);
        let (first, end) = ui.vlist_begin(ID_LIST, &mut self.vl, self.rows.len());
        for i in first..end {
            let k = self.rows[i];
            ui.vlist_item_begin(i);
            ui.text(TS_BODY_STRONG, &row_title(k));
            ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, &row_text(k));
            if self.opened.contains(&k) {
                ui.text(TS_CAPTION | TX_WRAP, "Aufgeklappt: diese Zeile zeigt mehr Text und wird dadurch höher; die Zeilen darunter rücken nach.");
            }
            ui.vlist_item_end();
        }
        let ev = ui.vlist_end(&mut self.vl);
        if ev & EV_DOUBLE != 0 && self.vl.sel >= 0 {
            let k = self.rows[self.vl.sel as usize];
            if !self.opened.remove(&k) {
                self.opened.insert(k);
            }
        }
    }
}

impl App for MdTest {
    fn init(&mut self, ui: &mut Ui) -> bool {
        // code in DejaVu Sans Mono on every system (the same layout everywhere)
        let m = ui.text.load_mem(MONO, 0);
        if m >= 0 {
            ui.mono_font_set(m as u32);
        }
        // Hebrew, Arabic and symbols the UI font lacks come from the system's fonts (also in
        // scripts, where the library leaves them off so that shots match everywhere)
        ui.text.set_system_fallback(true);
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | pad(12), 8);
        if self.page_form {
            self.form_page(ui);
            ui.box_end();
            return;
        }
        if self.page_dom {
            self.dom_page(ui);
            ui.box_end();
            return;
        }
        if self.page_css {
            self.css_page(ui);
            ui.box_end();
            return;
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.tabs(ID_TABS, &mut self.tab, &["Markdown", "HTML", "Hebräisch", "Liste"]);
        ui.spacer(-1);
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_HC, "Hoher Kontrast", &mut self.hc) & EV_ACT != 0 {
            ui.set_contrast(Some(self.hc != 0));
        }
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_RTL, "Rechts nach links", &mut self.rtl) & EV_ACT != 0 {
            ui.set_rtl(Some(self.rtl != 0));
        }
        ui.next_flags(ST_NOFOCUS);
        ui.checkbox(ID_OPEN, "Links öffnen", &mut self.open);
        ui.box_end();
        match self.tab {
            0..=2 => self.doc(ui, self.tab),
            _ => self.list(ui),
        }
        ui.box_end();
    }
}

fn mdviewtest() -> i32 {
    if plat::arg_value("--test").as_deref() == Some("unit") {
        return unit::run();
    }
    ui::launch(&DESC, MdTest::new(), &style::STYLE)
}

ui::main!(mdviewtest());

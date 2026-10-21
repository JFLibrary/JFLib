// model -- editortest --test model: self-checks of the document models and the highlighter:
// RichDoc (editing, formats, typing formats, paragraphs, lists, undo / redo and merging, JSON,
// HTML out and in, the clipboard fragment, plain text, links), CodeDoc (lines, auto-indent,
// block indent, closers, comments, moving lines, find / replace, columns, the change journal,
// undo merging) and the grammars (Rust, JSON, INI, TOML, Markdown: tokens and the states
// across lines) plus a timing of 100 000 lines.  Prints FAIL lines and a summary; the exit
// code is the number of failures.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

struct T {
    group: &'static str,
    ok: u32,
    failed: u32,
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

impl T {
    fn check(&mut self, cond: bool, name: &str) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            out(&format!("FAIL {}: {}\n", self.group, name));
        }
    }
    fn eq<A: PartialEq + core::fmt::Debug>(&mut self, got: A, want: A, name: &str) {
        let ok = got == want;
        self.check(ok, &if ok { String::from(name) } else { format!("{}: got {:?}, want {:?}", name, got, want) });
    }
}

fn p(l: usize, b: usize) -> TextPos {
    TextPos::new(l, b)
}

fn sel(a: TextPos, c: TextPos) -> TextSel {
    TextSel::new(a, c)
}

// the tokens of a line as "kind:text" (plain text left out)
fn toks(h: &Highlighter, line: &str, state: u32) -> (Vec<String>, u32) {
    let mut v = Vec::new();
    let st = h.line(line, state, &mut v);
    let names = ["text", "kw", "type", "fn", "str", "num", "com", "const", "punct", "meta", "key", "head", "em", "strong", "link", "code", "esc"];
    (v.iter().filter(|t| t.kind != SY_PUNCT).map(|t| format!("{}:{}", names[t.kind as usize], &line[t.start..t.end])).collect(), st)
}

fn has(v: &[String], s: &str) -> bool {
    v.iter().any(|x| x == s)
}

// ------------------------------------------------------------------ RichDoc

fn rich(t: &mut T) {
    t.group = "rich";
    let mut d = RichDoc::from_plain("Hallo Welt\nZweite Zeile");
    t.eq(d.para_count(), 2, "from_plain: two paragraphs");
    t.eq(d.text(), String::from("Hallo Welt\nZweite Zeile"), "text()");
    t.eq(d.flat(p(1, 3)), 14, "flat");
    t.eq(d.unflat(14), p(1, 3), "unflat");
    // typing, merged into one undo step
    d.set_selection(TextSel::at(p(0, 5)));
    for c in [",", " ", "d", "u"] {
        d.insert_text(c);
    }
    t.eq(String::from(d.para(0).text()), String::from("Hallo, du Welt"), "typing");
    t.eq(d.selection(), TextSel::at(p(0, 9)), "caret after typing");
    t.check(d.undo(), "undo typing");
    t.eq(String::from(d.para(0).text()), String::from("Hallo Welt"), "typing is one undo step");
    t.check(d.redo(), "redo typing");
    t.eq(String::from(d.para(0).text()), String::from("Hallo, du Welt"), "redo");
    // bold on a selection, toggled off again
    d.set_selection(sel(p(0, 10), p(0, 14)));
    d.toggle(RF_BOLD);
    let runs: Vec<(String, u8)> = d.para(0).runs().map(|(s, f)| (String::from(s), f.flags)).collect();
    t.eq(runs, alloc::vec![(String::from("Hallo, du "), 0), (String::from("Welt"), RF_BOLD)], "bold run");
    t.eq(d.current_fmt().flags & RF_BOLD, RF_BOLD, "current_fmt of a bold selection");
    d.set_selection(sel(p(0, 7), p(0, 14)));
    t.eq(d.current_fmt().flags & RF_BOLD, 0, "a partly bold selection is not bold");
    d.toggle(RF_BOLD);
    t.eq(d.para(0).runs().count(), 2, "toggle makes all of it bold");
    d.toggle(RF_BOLD);
    t.eq(d.para(0).runs().count(), 1, "toggle again: all plain, runs merged");
    // the format typed next (no selection)
    d.set_selection(TextSel::at(p(1, 0)));
    d.toggle(RF_ITALIC);
    t.eq(d.current_fmt().flags & RF_ITALIC, RF_ITALIC, "pending italic");
    d.insert_text("Die ");
    let r0 = d.para(1).runs().next().map(|(s, f)| (String::from(s), f.flags));
    t.eq(r0, Some((String::from("Die "), RF_ITALIC)), "typed text takes the pending format");
    d.insert_text("x");
    t.eq(d.para(1).runs().next().map(|(s, _)| String::from(s)), Some(String::from("Die x")), "the next character inherits");
    // Enter: split, attributes kept; in an empty list item the list ends
    let mut d = RichDoc::from_plain("eins zwei");
    d.set_selection(TextSel::at(p(0, 4)));
    d.toggle_list(LS_BULLET);
    d.split();
    t.eq(d.para_count(), 2, "split");
    t.eq((String::from(d.para(0).text()), String::from(d.para(1).text())), (String::from("eins"), String::from(" zwei")), "split text");
    t.eq((d.para(0).list, d.para(1).list), (LS_BULLET, LS_BULLET), "both halves stay list items");
    t.eq(d.selection(), TextSel::at(p(1, 0)), "caret at the new paragraph");
    d.set_selection(TextSel::at(p(1, 5)));
    d.split();
    t.eq(d.para_count(), 3, "split at the end");
    d.split();
    t.eq((d.para_count(), d.para(2).list), (3, LS_NONE), "Enter in an empty item ends the list");
    // indent and numbering
    let mut d = RichDoc::from_plain("a\nb\nc\nd\ne");
    d.set_selection(sel(p(0, 0), p(4, 1)));
    d.toggle_list(LS_NUMBER);
    d.set_selection(sel(p(1, 0), p(2, 1)));
    d.indent(1);
    let marks: Vec<String> = list_marks(d.paras()).into_iter().map(|m| m.unwrap_or_default()).collect();
    t.eq(marks, alloc::vec![String::from("1."), String::from("a."), String::from("b."), String::from("2."), String::from("3.")], "numbering by level");
    d.set_selection(TextSel::at(p(2, 0)));
    d.indent(1);
    let marks: Vec<String> = list_marks(d.paras()).into_iter().map(|m| m.unwrap_or_default()).collect();
    t.eq(marks[2].clone(), String::from("i."), "level 2 counts roman");
    t.eq(d.to_plain(), String::from("1. a\n  a. b\n    i. c\n2. d\n3. e"), "to_plain with markers");
    d.set_selection(sel(p(0, 0), p(4, 1)));
    d.toggle_list(LS_NUMBER);
    t.check(d.paras().iter().all(|q| q.list == LS_NONE), "toggle_list again: no list");
    // delete across paragraphs, undo restores formats and attributes
    let mut d = RichDoc::from_paras(alloc::vec![
        Para::new().with("Kopf", Fmt::bold()).attrs(RA_CENTER, LS_NONE, 0, PS_H1),
        Para::plain("Mitte"),
        Para::new().with("Ende", Fmt::italic()),
    ]);
    let before = d.paras().to_vec();
    d.delete(p(0, 2), p(2, 2));
    t.eq(d.para_count(), 1, "delete across paragraphs");
    t.eq(String::from(d.para(0).text()), String::from("Kode"), "joined text");
    t.eq(d.para(0).align, RA_CENTER, "the first paragraph's attributes stay");
    d.undo();
    t.check(d.paras() == &before[..], "undo restores paragraphs, formats and attributes");
    // alignment, style, link
    d.set_selection(TextSel::at(p(1, 1)));
    d.set_align(RA_RIGHT);
    d.set_style(PS_QUOTE);
    t.eq((d.para(1).align, d.para(1).style), (RA_RIGHT, PS_QUOTE), "set_align / set_style");
    d.set_selection(sel(p(1, 0), p(1, 5)));
    d.set_link("https://a.example");
    t.eq(d.link_at(p(1, 2)), Some("https://a.example"), "set_link on a selection");
    d.set_selection(TextSel::at(p(1, 2)));
    d.set_link("https://b.example");
    t.eq((d.link_at(p(1, 0)), d.link_at(p(1, 4))), (Some("https://b.example"), Some("https://b.example")), "a caret in a link edits the whole link");
    d.set_link("");
    t.eq(d.link_at(p(1, 2)), None, "set_link(\"\") removes it");
    d.set_selection(TextSel::at(p(1, 5)));
    d.set_link("https://c.example");
    t.eq(String::from(d.para(1).text()), String::from("Mittehttps://c.example"), "a link at a caret outside one inserts the url");
    t.eq(d.link_at(p(1, 6)), Some("https://c.example"), "the inserted url is linked");
    // colours, size, font, clear format
    let mut d = RichDoc::from_plain("Farbe");
    d.set_selection(sel(p(0, 0), p(0, 5)));
    d.set_color(0xFF112233);
    d.set_bg(0xFFFFFF00);
    d.set_size(145);
    d.set_font("DejaVu Serif");
    let f = d.para(0).fmt_at(0).clone();
    t.eq((f.color, f.bg, f.size, f.font.clone()), (0xFF112233, 0xFFFFFF00, 145, String::from("DejaVu Serif")), "colour, highlight, size, font");
    d.clear_format();
    t.eq(d.para(0).fmt_at(0).clone(), Fmt::default(), "clear_format");
    // JSON round trip of the sample
    let s = crate::sample_rich();
    let j = s.to_json();
    t.check(j.starts_with("{\"jfrich\":1"), "JSON header");
    match RichDoc::from_json(&j) {
        Some(b) => {
            t.check(b.paras() == s.paras(), "JSON round trip");
            t.eq(crate::markup(&b), crate::markup(&s), "JSON round trip (markup)");
        }
        None => t.check(false, "from_json of to_json"),
    }
    t.check(RichDoc::from_json("{\"x\":1}").is_none(), "from_json rejects other JSON");
    let mut e = RichDoc::from_plain("");
    e.size = 125;
    e.font = String::from("DejaVu Serif");
    let e2 = RichDoc::from_json(&e.to_json()).unwrap_or_default();
    t.eq((e2.size, e2.font.clone()), (125, String::from("DejaVu Serif")), "the document's font and size in JSON");
    // HTML
    let h = s.to_html();
    for want in [
        "<h1>Fertiges Rich-Text-Widget</h1>",
        "<b>fett</b>",
        "<i>kursiv</i>",
        "<u>unterstrichen</u>",
        "<s>durchgestrichen</s>",
        "<span style=\"color:#c00000\">rot</span>",
        "<span style=\"font-size:16pt\">groß</span>",
        "<a href=\"https://example.org/jflib\">Link</a>",
        "<ul><li>Erster Punkt</li><li>Zweiter Punkt mit Unterpunkt<ul><li>Unterpunkt</li></ul></li></ul>",
        "<ol><li>Schritt eins</li><li>Schritt zwei<ol type=\"a\"><li>Teilschritt</li></ol></li></ol>",
        "<blockquote><p>",
        "<p style=\"text-align:center\">Zentrierter Absatz</p>",
    ] {
        t.check(h.contains(want), &format!("to_html has {:?}", want));
    }
    let back = RichDoc::from_html(&h);
    t.eq(crate::markup(&back).replace("[f=DejaVu Serif]Serifen[/]", "Serifen"), crate::markup(&s).replace("[f=DejaVu Serif]Serifen[/]", "Serifen"), "from_html(to_html) keeps the formats");
    // a browser's HTML
    let web = "<html><head><style>p{color:red}</style></head><body><p>Hallo <strong>fette</strong> &amp; <em>schräge</em>   Welt&nbsp;!</p>\
        <ul><li>a</li><li>b<ul><li>c</li></ul></li></ul><h2 style=\"text-align: center\">Titel</h2>\
        <p><span style=\"color: rgb(0, 128, 0); font-size: 18px; font-weight: 700\">grün</span><br>zweite Zeile</p>\
        <script>alert(1)</script><ol><li>eins</li></ol><p><a href=\"https://x.example/?a=1&amp;b=2\">x</a></p></body></html>";
    let w = RichDoc::from_html(web);
    t.eq(
        crate::markup(&w),
        String::from(
            "Hallo [b]fette[/] & [i]schräge[/] Welt ! | (bullet) a | (bullet) b | (bullet,level1) c | (center,h2) Titel | [b c=#008000 sz=135]grün[/] | zweite Zeile | (number) eins | [l=https://x.example/?a=1&b=2]x[/]"
        ),
        "from_html of a browser's HTML",
    );
    // the clipboard's fragment is exact
    let mut c = crate::sample_rich();
    c.set_selection(sel(p(1, 11), p(4, 5)));
    let frag = c.selected_paras();
    let html_clip = RichDoc::from_paras(frag.clone()).to_html();
    t.check(html_clip.contains("<b>fett</b>"), "fragment HTML");
    let mut target = RichDoc::from_plain("[]");
    target.set_selection(TextSel::at(p(0, 1)));
    target.insert_paras(&frag);
    t.eq(target.para_count(), 4, "paste of several paragraphs");
    t.eq(String::from(target.para(0).text()), String::from("[mit fett, kursiv, unterstrichen, durchgestrichen, rot, groß, Serifen, markiert und einem Link."), "the first pasted paragraph continues the host");
    t.eq((target.para(1).style, target.para(2).list), (PS_H2, LS_BULLET), "pasted paragraphs keep their attributes");
    t.eq(String::from(target.para(3).text()), String::from("Zweit]"), "the last pasted paragraph takes the rest");
    target.undo();
    t.eq(target.text(), String::from("[]"), "undo the paste");
}

// ------------------------------------------------------------------ CodeDoc

fn code(t: &mut T) {
    t.group = "code";
    let d = CodeDoc::from_text("eins\r\nzwei\rdrei\n");
    t.eq(d.line_count(), 4, "from_text: CR LF, CR, LF");
    t.eq(d.text(), String::from("eins\nzwei\ndrei\n"), "text()");
    // typing merges, newline with indentation
    let mut d = CodeDoc::from_text("fn main() {}");
    d.set_selection(TextSel::at(p(0, 11)));
    t.check(d.newline("{[("), "newline");
    t.eq(d.text(), String::from("fn main() {\n    \n}"), "Enter between braces: the closer on a line of its own");
    t.eq(d.selection(), TextSel::at(p(1, 4)), "caret indented");
    for c in ["l", "e", "t"] {
        d.type_text(c, "}])");
    }
    t.eq(String::from(d.line(1)), String::from("    let"), "typing");
    d.undo();
    t.eq(String::from(d.line(1)), String::from("    "), "typing is one undo step");
    d.undo();
    t.eq(d.text(), String::from("fn main() {}"), "undo the newline");
    // a closer on a blank line goes out a level
    let mut d = CodeDoc::from_text("if x {\n        ");
    d.set_selection(TextSel::at(p(1, 8)));
    d.type_text("}", "}])");
    t.eq(String::from(d.line(1)), String::from("    }"), "closer dedents");
    // newline keeps the indentation, no extra level without an opener
    let mut d = CodeDoc::from_text("    abc");
    d.set_selection(TextSel::at(p(0, 7)));
    d.newline("{");
    t.eq(d.text(), String::from("    abc\n    "), "newline keeps the indentation");
    // tab, back tab, block indent
    let mut d = CodeDoc::from_text("a\nb\nc");
    d.set_selection(TextSel::at(p(0, 1)));
    d.tab();
    t.eq(String::from(d.line(0)), String::from("a   "), "Tab to the next stop");
    d.set_selection(sel(p(0, 0), p(2, 1)));
    d.tab();
    t.eq(d.text(), String::from("    a   \n    b\n    c"), "Tab indents the selected lines");
    t.eq(d.selection(), sel(p(0, 0), p(2, 5)), "the selection follows (whole lines from column 0)");
    d.back_tab();
    t.eq(d.text(), String::from("a   \nb\nc"), "Shift+Tab unindents");
    d.use_tabs = true;
    d.set_selection(sel(p(1, 0), p(2, 0)));
    d.indent(1);
    t.eq(d.text(), String::from("a   \n\tb\nc"), "a selection ending at a line start leaves that line out; tabs");
    // comments
    let mut d = CodeDoc::from_text("  a\n\n    b");
    d.set_selection(sel(p(0, 0), p(2, 5)));
    d.toggle_comment("//");
    t.eq(d.text(), String::from("  // a\n\n  //   b"), "comment on at the least indentation");
    d.toggle_comment("//");
    t.eq(d.text(), String::from("  a\n\n    b"), "comment off");
    // moving lines
    let mut d = CodeDoc::from_text("1\n2\n3");
    d.set_selection(TextSel::at(p(0, 0)));
    d.move_lines(1);
    t.eq(d.text(), String::from("2\n1\n3"), "Alt+Down");
    t.eq(d.selection().caret.line, 1, "the caret moves with the line");
    d.move_lines(-1);
    t.eq(d.text(), String::from("1\n2\n3"), "Alt+Up");
    // find and replace
    let d0 = CodeDoc::from_text("Foo foo food\nfoo_bar FOO");
    t.eq(d0.find_all("foo", 0).len(), 5, "find: case insensitive");
    t.eq(d0.find_all("foo", FIND_CASE).len(), 3, "find: case");
    t.eq(d0.find_all("foo", FIND_WORD).len(), 3, "find: whole words");
    t.eq(d0.find_all("Foo", FIND_CASE | FIND_WORD), alloc::vec![(p(0, 0), p(0, 3))], "find: case and word");
    t.eq(d0.find_next(p(1, 9), "foo", 0, false), Some((p(0, 0), p(0, 3))), "find_next wraps around");
    t.eq(d0.find_next(p(0, 1), "foo", 0, true), Some((p(0, 0), p(0, 3))), "find_next backwards");
    t.eq(d0.find_next(p(0, 0), "foo", 0, true), Some((p(1, 8), p(1, 11))), "find_next backwards wraps");
    t.eq(d0.find_all("über", 0).len(), 0, "no match");
    let du = CodeDoc::from_text("ÄBC äbc");
    t.eq(du.find_all("äbc", 0).len(), 2, "find: case folding beyond ASCII");
    let mut d = CodeDoc::from_text("a.b a.b\nab");
    t.eq(d.replace_all("a.b", "X", 0), 2, "replace_all count");
    t.eq(d.text(), String::from("X X\nab"), "replace_all");
    d.undo();
    t.eq(d.text(), String::from("a.b a.b\nab"), "replace_all is one undo step");
    d.set_selection(TextSel::at(p(0, 0)));
    d.replace_next("a.b", "Y", 0);
    t.eq(d.selection(), sel(p(0, 0), p(0, 3)), "replace_next first selects the match");
    d.replace_next("a.b", "Y", 0);
    t.eq(String::from(d.line(0)), String::from("Y a.b"), "replace_next replaces the selected match");
    t.eq(d.selection(), sel(p(0, 2), p(0, 5)), "and selects the next");
    // columns: tabs and wide characters
    let mut d = CodeDoc::from_text("\tab漢x");
    d.tab_width = 4;
    t.eq(d.col_of(0, 1), 4, "a tab to the stop");
    t.eq(d.col_of(0, 3), 6, "two letters");
    t.eq(d.col_of(0, 6), 8, "a wide character is two columns");
    t.eq(d.byte_at_col(0, 7), 6, "byte at a column (nearest boundary)");
    t.eq(d.byte_at_col(0, 1), 0, "inside a tab: the nearer boundary");
    // the change journal
    let mut d = CodeDoc::from_text("a\nb\nc\nd");
    let r0 = d.revision();
    d.set_selection(sel(p(1, 0), p(2, 1)));
    d.insert("X\nY\nZ");
    let ch = d.changes_since(r0).map(|c| c.to_vec()).unwrap_or_default();
    t.eq(ch.len(), 1, "one change");
    t.eq(ch.first().map(|c| (c.first, c.old, c.new)), Some((1, 2, 3)), "lines 1..3 became 1..4");
    t.eq(d.changes_since(d.revision()).map(|c| c.len()), Some(0), "nothing since now");
    // backspace / delete merge
    let mut d = CodeDoc::from_text("abcdef");
    d.set_selection(TextSel::at(p(0, 6)));
    for _ in 0..3 {
        let c = d.selection().caret;
        d.delete(TextPos::new(0, c.byte - 1), c);
    }
    t.eq(String::from(d.line(0)), String::from("abc"), "three deletes");
    d.undo();
    t.eq(String::from(d.line(0)), String::from("abcd"), "plain deletes are steps of their own");
}

// ------------------------------------------------------------------ the highlighter

fn grammars(t: &mut T) {
    t.group = "highlight";
    let r = Highlighter::rust();
    let (v, st) = toks(&r, "pub fn main() -> u32 { let s = \"a\\nb\"; 0x1F }", 0);
    for w in ["kw:pub", "kw:fn", "fn:main", "type:u32", "kw:let", "str:\"a", "esc:\\n", "str:b\"", "num:0x1F"] {
        t.check(has(&v, w), &format!("rust {:?} in {:?}", w, v));
    }
    t.eq(st, 0, "rust: nothing open");
    let (v, st) = toks(&r, "let a = 1; /* open /* nested", 0);
    t.check(has(&v, "com:/* open /* nested"), &format!("nested comment open: {:?}", v));
    t.check(st != 0, "a block comment stays open");
    let (v, st2) = toks(&r, "inside */ x */ after", st);
    t.check(has(&v, "com:inside */ x */"), &format!("nested comment closes after two: {:?}", v));
    t.eq(st2, 0, "closed");
    let (v, _) = toks(&r, "let c = 'x'; let l: &'a str = r#\"raw \"q\"\"#; println!(\"{}\", c);", 0);
    for w in ["str:'x'", "meta:'a", "str:r#\"raw \"q\"\"#", "fn:println!"] {
        t.check(has(&v, w), &format!("rust {:?} in {:?}", w, v));
    }
    let (v, _) = toks(&r, "#[derive(Debug)] struct P; // comment", 0);
    t.check(has(&v, "meta:#[derive(Debug)]") && has(&v, "com:// comment") && has(&v, "type:P"), &format!("attribute, comment, type: {:?}", v));
    let (_, st) = toks(&r, "let s = \"multi", 0);
    t.check(st != 0, "a string over lines");
    let (v, st) = toks(&r, "line\" + 1", st);
    t.check(has(&v, "str:line\"") && st == 0, &format!("the string ends: {:?}", v));
    let (_, st) = toks(&r, "let r = r##\"raw", 0);
    let (v, st) = toks(&r, "\"# still \"##; x", st);
    t.check(has(&v, "str:\"# still \"##") && st == 0, &format!("raw string with ## over lines: {:?}", v));
    let (v, _) = toks(&r, "x.1.5e-3f64 + 1_000u8", 0);
    t.check(has(&v, "num:1_000u8"), &format!("numbers: {:?}", v));
    // JSON
    let j = Highlighter::json();
    let (v, _) = toks(&j, "{ \"key\": \"value\", \"n\": -1.5e3, \"t\": true, \"z\": null }", 0);
    for w in ["key:\"key\"", "str:\"value\"", "key:\"n\"", "num:1.5e3", "const:true", "const:null"] {
        t.check(has(&v, w), &format!("json {:?} in {:?}", w, v));
    }
    // INI
    let i = Highlighter::ini();
    t.eq(toks(&i, "[section name]", 0).0, alloc::vec![String::from("head:[section name]")], "ini section");
    t.check(has(&toks(&i, "  key.name = 42", 0).0, "key:key.name"), "ini key");
    t.check(has(&toks(&i, "key = 42", 0).0, "num:42"), "ini number");
    t.eq(toks(&i, "; comment", 0).0, alloc::vec![String::from("com:; comment")], "ini comment");
    t.check(has(&toks(&i, "path: /usr/bin", 0).0, "key:path"), "ini key with ':'");
    // TOML
    let m = Highlighter::toml();
    t.eq(toks(&m, "[[products]]", 0).0, alloc::vec![String::from("head:[[products]]")], "toml array table");
    let (v, _) = toks(&m, "dob = 1979-05-27T07:32:00-08:00 # born", 0);
    t.check(has(&v, "key:dob") && has(&v, "num:1979-05-27T07:32:00-08:00") && has(&v, "com:# born"), &format!("toml date, comment: {:?}", v));
    let (_, st) = toks(&m, "s = \"\"\"multi", 0);
    t.check(st != 0, "toml multi-line string open");
    let (v, st) = toks(&m, "line\"\"\" # c", st);
    t.check(has(&v, "str:line\"\"\"") && st == 0, &format!("toml multi-line string closes: {:?}", v));
    // Markdown
    let md = Highlighter::markdown();
    t.eq(toks(&md, "## Title", 0).0, alloc::vec![String::from("head:## Title")], "md heading");
    let (v, _) = toks(&md, "- *em* and **strong** with `code` and [a](http://b) snake_case_word", 0);
    for w in ["kw:-", "em:*em*", "strong:**strong**", "code:`code`", "link:[a](http://b)"] {
        t.check(has(&v, w), &format!("md {:?} in {:?}", w, v));
    }
    t.check(!v.iter().any(|x| x.starts_with("em:_")), &format!("no emphasis inside snake_case: {:?}", v));
    let (_, st) = toks(&md, "```rust", 0);
    t.check(st != 0, "md fence opens");
    let (v, st) = toks(&md, "fn x() {}", st);
    t.eq(v, alloc::vec![String::from("code:fn x() {}")], "md fenced line is code");
    let (_, st) = toks(&md, "```", st);
    t.eq(st, 0, "md fence closes");
    t.eq(toks(&md, "1. item", 0).0, alloc::vec![String::from("kw:1.")], "md numbered list marker");
    // names
    t.eq(Highlighter::for_name("src/main.rs").name(), "Rust", "for_name by path");
    t.eq(Highlighter::for_name("TOML").name(), "TOML", "for_name by name");
    t.eq(Highlighter::for_name("x.cfg").name(), "INI", "for_name ini");
    t.eq(Highlighter::for_name("README.md").name(), "Markdown", "for_name md");
    t.eq(Highlighter::for_name("x.unknown").name(), "Text", "for_name plain");
    // themes
    let (l, d, h) = (CodeTheme::light(), CodeTheme::dark(), CodeTheme::high_contrast(&plat::HcColors::BLACK));
    t.check(l.bg != d.bg && l.tokens[SY_KEYWORD as usize] != d.tokens[SY_KEYWORD as usize], "light and dark differ");
    t.check(h.bg == 0xFF000000 && h.fg == 0xFFFFFFFF && h.find_frame && h.sel_text != 0, "high contrast from the scheme");
    // 100 000 lines: one pass over the states
    let big = crate::big_code(100_000);
    let t0 = plat::time_us();
    let mut st = 0;
    let mut v = Vec::new();
    let mut n = 0usize;
    for l in big.lines() {
        st = r.line(l, st, &mut v);
        n += v.len();
    }
    let us = plat::time_us() - t0;
    t.check(st == 0 && n > 500_000, &format!("100 000 lines tokenized ({} tokens)", n));
    out(&format!("model: 100 000 lines highlighted in {} ms\n", us / 1000));
    let t0 = plat::time_us();
    let found = big.find_all("value_00100", FIND_CASE).len();
    out(&format!("model: find over 100 000 lines in {} ms\n", (plat::time_us() - t0) / 1000));
    t.eq(found, 9, "find in the big file");
}

/// --test model -> failures
pub fn run() -> i32 {
    let mut t = T { group: "", ok: 0, failed: 0 };
    rich(&mut t);
    code(&mut t);
    grammars(&mut t);
    out(&format!("model: {} checks, {} failed\n", t.ok + t.failed, t.failed));
    t.failed as i32
}

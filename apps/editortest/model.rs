// model -- editortest --test model: self-checks of the document models and the highlighter:
// RichDoc (editing, formats, typing formats, paragraphs, lists, undo / redo and merging, JSON,
// HTML out and in, the clipboard fragment, plain text, links), CodeDoc (lines, auto-indent,
// block indent, closers, comments, moving lines, find / replace, columns, the change journal,
// undo merging), its diagnostics (positions in characters, clamping, counts, at / next, how
// they follow typing, Enter, deletes, points, block indent, comments, moved lines, undo,
// replace_all; 100 000 lines), what the code editor keeps of long lines through edits
// (ui::code_long_check) and the grammars (Rust, JSON, INI, TOML, Markdown, JFS: tokens
// and the states across lines; JFS also token by token against lib/script's tokens() on two
// samples) plus a timing of 100 000 lines.  Prints FAIL lines and a summary; the exit code is
// the number of failures.

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

// ------------------------------------------------------------------ diagnostics

fn diags(t: &mut T) {
    t.group = "diag";
    let ds = |d: &CodeDoc| crate::diag_list(d, 100);
    // columns in characters (a tab one), clamped to the line and the text, sorted, points
    let mut d = CodeDoc::from_text("let größe = x\n\tfoo(1)\nend");
    d.set_diagnostics(&[
        Diag::new(2, 2, 5, DIAG_WARNING, "w"),
        Diag::new(1, 5, 10, DIAG_ERROR, "e"),
        Diag::new(3, 9, 9, DIAG_INFO, "past the end of the line"),
        Diag::span(9, 1, 9, 2, DIAG_ERROR, "past the last line"),
    ]);
    t.eq(ds(&d), String::from("1:5-1:10 E, 2:2-2:5 W, 3:4-3:4 I, 3:4-3:4 E"), "set: sorted, clamped, points");
    t.eq(d.diagnostic_counts(), [2, 1, 1], "counts");
    // (bytes: größe is 4 .. 11)
    t.eq(d.diagnostics_at(p(0, 11)).len(), 1, "at: the end belongs to it");
    t.eq(d.diagnostics_at(p(0, 12)).len(), 0, "at: after it");
    t.eq(d.diagnostics_at(p(0, 3)).len(), 0, "at: before it");
    let at: Vec<u8> = d.diagnostics_at(p(2, 3)).iter().map(|x| x.severity).collect();
    t.eq(at, alloc::vec![DIAG_ERROR, DIAG_INFO], "at: most severe first");
    t.eq(d.next_diagnostic(p(0, 0), false), Some(p(0, 4)), "next");
    t.eq(d.next_diagnostic(p(0, 4), false), Some(p(1, 1)), "next after one");
    t.eq(d.next_diagnostic(p(2, 3), false), Some(p(0, 4)), "next round the end");
    t.eq(d.next_diagnostic(p(0, 4), true), Some(p(2, 3)), "previous round the start");
    t.eq(d.next_diagnostic(p(1, 2), true), Some(p(1, 1)), "previous");
    t.eq(CodeDoc::new().next_diagnostic(p(0, 0), false), None, "next without any");
    // typing: before it moves it, at its edges stays outside, inside grows it
    let mut d = CodeDoc::from_text("abc foo bar\nnext line");
    d.set_diagnostics(&[Diag::new(1, 5, 8, DIAG_ERROR, "foo"), Diag::new(2, 1, 5, DIAG_WARNING, "next")]);
    for (at, s, want, name) in [
        (0, "X", "1:6-1:9 E, 2:1-2:5 W", "typed before: moves"),
        (5, "Y", "1:7-1:10 E, 2:1-2:5 W", "typed at its start: outside"),
        (9, "Z", "1:7-1:10 E, 2:1-2:5 W", "typed at its end: outside"),
        (7, "Q", "1:7-1:11 E, 2:1-2:5 W", "typed inside: grows"),
    ] {
        d.set_selection(TextSel::at(p(0, at)));
        d.type_text(s, "");
        t.eq(ds(&d), String::from(want), name);
    }
    // Enter above and its undo; deleting a part shrinks it, deleting all of it drops it
    d.set_selection(TextSel::at(p(0, 0)));
    d.newline("");
    t.eq(ds(&d), String::from("2:7-2:11 E, 3:1-3:5 W"), "Enter above: a line down");
    d.undo();
    t.eq(ds(&d), String::from("1:7-1:11 E, 2:1-2:5 W"), "undo: back");
    d.delete(p(0, 6), p(0, 8));
    t.eq(ds(&d), String::from("1:7-1:9 E, 2:1-2:5 W"), "a part deleted: shrinks");
    d.delete(p(0, 6), p(0, 8));
    t.eq(ds(&d), String::from("2:1-2:5 W"), "its text deleted: gone");
    t.eq(d.diagnostic_counts(), [0, 1, 0], "the counts follow");
    d.set_selection(TextSel::new(p(0, 4), p(1, 2)));
    d.insert("");
    t.eq(ds(&d), String::from("1:5-1:7 W"), "a selection over lines deleted: what is left of it");
    // points: at the end of the line typed text pushes it on, inside a change it goes to its start
    let mut d = CodeDoc::from_text("call(x");
    d.set_diagnostics(&[Diag::new(1, 99, 0, DIAG_ERROR, "missing )")]);
    t.eq(ds(&d), String::from("1:7-1:7 E"), "a point at the line end");
    d.set_selection(TextSel::at(p(0, 6)));
    d.type_text(")", "");
    t.eq(ds(&d), String::from("1:8-1:8 E"), "typed at a point: it moves on");
    d.set_diagnostics(&[Diag::new(1, 3, 3, DIAG_ERROR, "at l")]);
    d.replace(p(0, 1), p(0, 3), "XYZ");
    t.eq(ds(&d), String::from("1:2-1:2 E"), "a point in a change: at its start");
    // line by line: block indent, comments; Alt+Up / Down: with the line; undo
    let mut d = CodeDoc::from_text("a = 1\nb = x\nc = 3");
    d.set_diagnostics(&[Diag::new(1, 1, 2, DIAG_INFO, "a"), Diag::new(2, 5, 6, DIAG_WARNING, "x")]);
    d.set_selection(TextSel::new(p(0, 0), p(2, 5)));
    d.tab();
    t.eq(ds(&d), String::from("1:5-1:6 I, 2:9-2:10 W"), "block indent: line by line");
    d.toggle_comment("//");
    t.eq(ds(&d), String::from("1:8-1:9 I, 2:12-2:13 W"), "comments: line by line");
    d.undo();
    d.undo();
    t.eq(ds(&d), String::from("1:1-1:2 I, 2:5-2:6 W"), "undo comments and indent");
    d.set_selection(TextSel::at(p(1, 0)));
    d.move_lines(1);
    t.eq(ds(&d), String::from("1:1-1:2 I, 3:5-3:6 W"), "Alt+Down: with its line");
    d.set_selection(TextSel::at(p(0, 0)));
    d.move_lines(1);
    t.eq(ds(&d), String::from("2:1-2:2 I, 3:5-3:6 W"), "Alt+Down of the other line");
    d.undo();
    d.undo();
    t.eq(ds(&d), String::from("1:1-1:2 I, 2:5-2:6 W"), "undo the moves");
    d.redo();
    t.eq(ds(&d), String::from("1:1-1:2 I, 3:5-3:6 W"), "redo a move");
    // a part over lines: Enter inside it, replace_all on its lines
    let mut d = CodeDoc::from_text("fn x() {\n  body\n}");
    d.set_diagnostics(&[Diag::span(1, 8, 3, 2, DIAG_ERROR, "block")]);
    t.eq(ds(&d), String::from("1:8-3:2 E"), "a part over lines");
    d.set_selection(TextSel::at(p(1, 6)));
    d.newline("{");
    t.eq(ds(&d), String::from("1:8-4:2 E"), "Enter inside: the end follows");
    let mut d = CodeDoc::from_text("x x\ny x");
    d.set_diagnostics(&[Diag::new(2, 1, 2, DIAG_WARNING, "y"), Diag::new(2, 3, 4, DIAG_ERROR, "x")]);
    d.replace_all("x", "long", 0);
    t.eq(ds(&d), String::from("2:1-2:2 W"), "replace_all: before a replaced word it stays, the replaced one goes");
    d.clear_diagnostics();
    t.eq((d.diagnostic_count(), d.diagnostics().len()), (0, 0), "clear");
    // 100 000 lines with one on every 7th: an Enter at the top and 100 characters typed
    let mut big = crate::big_code(100_000);
    let v: Vec<Diag> = (0..100_000).step_by(7).map(|l| Diag::new(l + 1, 5, 8, DIAG_WARNING, "w")).collect();
    big.set_diagnostics(&v);
    let t0 = plat::time_us();
    big.set_selection(TextSel::at(p(0, 0)));
    big.newline("");
    for _ in 0..100 {
        big.type_text("x", "");
    }
    let us = plat::time_us() - t0;
    t.eq(big.diagnostic_count(), v.len(), "big: all kept");
    t.eq(big.diagnostics().last().map(|x| x.line), Some(99_997), "big: the last one a line down");
    t.eq(big.diagnostics().first().map(|x| (x.line, x.col)), Some((2, 105)), "big: the first one moved by the typed text");
    out(&format!("model: 101 edits with {} diagnostics over 100 000 lines in {} ms\n", v.len(), us / 1000));
}

// ------------------------------------------------------------------ JFS against lib/script

// the tokens of each line as (start, end, class) with neighbours of a class merged: what the
// grammar and lib/script's lexer have to agree on
type Classes = Vec<Vec<(usize, usize, &'static str)>>;

fn merged(v: &mut Vec<(usize, usize, &'static str)>, s: usize, e: usize, class: &'static str) {
    match v.last_mut() {
        Some(l) if l.1 == s && l.2 == class => l.1 = e,
        _ if e > s => v.push((s, e, class)),
        _ => {}
    }
}

fn jfs_by_grammar(src: &str) -> Classes {
    let h = Highlighter::jfs();
    let mut st = 0;
    let mut out = Vec::new();
    let mut toks = Vec::new();
    for l in src.split('\n') {
        st = h.line(l, st, &mut toks);
        let mut v = Vec::new();
        for tk in &toks {
            let class = match tk.kind {
                SY_COMMENT => "com",
                SY_KEYWORD => "kw",
                SY_CONST => "const",
                SY_NUMBER => "num",
                SY_STRING | SY_ESCAPE => "str",
                SY_META => "interp",
                SY_PUNCT => "punct",
                SY_FUNC => "fn",
                _ => "?",
            };
            merged(&mut v, tk.start, tk.end, class);
        }
        out.push(v);
    }
    out
}

fn jfs_by_script(src: &str) -> Classes {
    let b = src.as_bytes();
    let mut starts = alloc::vec![0];
    starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
    let mut out: Classes = starts.iter().map(|_| Vec::new()).collect();
    for (s, len, kind) in script::tokens(src) {
        let (s, e) = (s as usize, (s + len) as usize);
        let class = match kind.class() {
            script::TokClass::Comment => "com",
            script::TokClass::Keyword => "kw",
            script::TokClass::Constant => "const",
            script::TokClass::Number => "num",
            script::TokClass::Str => "str",
            script::TokClass::Op | script::TokClass::Punct if kind == script::TokKind::Interp => "interp",
            script::TokClass::Op | script::TokClass::Punct => "punct",
            script::TokClass::Ident => {
                // a call: '(' after blanks on the line (the grammar's `calls`)
                let mut k = e;
                while b.get(k) == Some(&b' ') {
                    k += 1;
                }
                if b.get(k) != Some(&b'(') {
                    continue;
                }
                "fn"
            }
            _ => continue,
        };
        // a part per line it covers (a comment over lines), without the line breaks
        let first = starts.partition_point(|&x| x <= s) - 1;
        for (l, &ls) in starts.iter().enumerate().skip(first) {
            if ls >= e && l > first {
                break;
            }
            let le = starts.get(l + 1).map_or(src.len(), |&n| n - 1);
            let (a, z) = (s.max(ls), e.min(le));
            if z > a {
                merged(&mut out[l], a - ls, z - ls, class);
            }
        }
    }
    out
}

// more of the language than the editor's sample: every keyword, operator, escape, number form
const JFS_MORE: &str = r#"// every kind of token once more
let größe = 3
let m = {name: "n", "quoted key": 1, 7: true, [größe]: nil}
fn f(a, b) {
    while a < b && !(a >= 10) || b <= 0 {
        a += 2; b -= 1
        a *= 1; a /= 1; a %= 7
        if a != b { continue } else if a == b { break } else { a = a - 1 }
    }
    try { throw "boom ${a}" } catch e { log("caught: ${e} \t \" \\ \$ \0 \r \u{1F600}") }
    return "${a}${b}" + "" + "${"inner ${b} ${ {x: "}"}.x }"}" + "\${not} $ {}"
}
let r = f(1, 2) ?? 0.5 * 3 % 2 / 1 - -1
/* a comment */ let after = [1.0e+3, 2_5, 0XAB, 0B11, 1e5, 7] // the end
/* over
   three
   lines */ for k, v in m { print(k, v) }
for i in 0 .. 10 { m.list.push(i) }
"#;

fn jfs(t: &mut T) {
    t.group = "jfs";
    // token by token against lib/script's lexer
    for (name, src) in [("editor sample", crate::JFS_SAMPLE), ("more", JFS_MORE)] {
        let (g, s) = (jfs_by_grammar(src), jfs_by_script(src));
        t.eq(g.len(), s.len(), &format!("{}: lines", name));
        for (l, (a, b)) in g.iter().zip(&s).enumerate() {
            let text = src.split('\n').nth(l).unwrap_or("");
            t.check(a == b, &format!("{} line {} {:?}: grammar {:?}, lib/script {:?}", name, l + 1, text, a, b));
        }
    }
    let h = Highlighter::jfs();
    // expected tokens (punctuation left out): interpolation with braces and strings inside
    let (v, st) = toks(&h, "let s = \"a${x + {k: \"}\"}.k}b\\n\"", 0);
    t.eq(v, ["kw:let", "str:\"a", "meta:${", "str:\"}\"", "meta:}", "str:b", "esc:\\n", "str:\""].map(String::from).to_vec(), "interpolation");
    t.eq(st, 0, "a string ends with its line");
    let (v, _) = toks(&h, "\"${\"in ${x}\"}\" \"\\${no}\" \"\\u{2713}\"", 0);
    let want = [
        "str:\"", "meta:${", "str:\"in ", "meta:${", "meta:}", "str:\"", "meta:}", "str:\"", "str:\"", "esc:\\$", "str:{no}\"", "str:\"", "esc:\\u{2713}",
        "str:\"",
    ];
    t.eq(v, want.map(String::from).to_vec(), "nested strings, \\$, \\u{..}");
    let (v, _) = toks(&h, "1_000 0xFF 0b101 1.5e-3 2E10 1..5 x1", 0);
    t.eq(v, ["num:1_000", "num:0xFF", "num:0b101", "num:1.5e-3", "num:2E10", "num:1", "num:5"].map(String::from).to_vec(), "numbers");
    let (v, _) = toks(&h, "fn f(a) { return nil ?? true } print(s.split(\",\")) false", 0);
    let want = ["kw:fn", "fn:f", "kw:return", "const:nil", "const:true", "fn:print", "fn:split", "str:\",\"", "const:false"];
    t.eq(v, want.map(String::from).to_vec(), "keywords, constants, calls");
    let (v, st) = toks(&h, "let a /* open", 0);
    t.check(has(&v, "com:/* open") && st != 0, &format!("a block comment stays open: {:?}", v));
    let (v, st2) = toks(&h, "still /* not nested */ let b", st);
    t.check(has(&v, "com:still /* not nested */") && has(&v, "kw:let") && st2 == 0, &format!("it ends at the first */: {:?}", v));
    let (v, st) = toks(&h, "let s = \"open ${a + 1", 0);
    t.check(has(&v, "str:\"open ") && has(&v, "num:1") && st == 0, &format!("an open interpolation ends with the line: {:?}", v));
    let (_, st) = toks(&h, "\"${ /* in a string", 0);
    t.check(st != 0, "a comment open in an interpolation goes on in the next line");
    t.eq(Highlighter::for_name("jfs").name(), "JFS", "for_name jfs");
    t.eq(Highlighter::for_name("scripts/server.jfs").name(), "JFS", "for_name by path");
    t.eq((h.grammar().comment, h.grammar().pairs), ("//", "()[]{}"), "comment, pairs");
}

/// --test model -> failures
// ------------------------------------------------------------------ long lines

// what the code editor keeps of a long line (column checkpoints, tokens) follows edits as a
// fresh look at the line sees it (ui::code_long_check): places spread over the line, text
// that opens and closes strings, comments, interpolations, tabs, marks, emoji, ZWJ
fn long(t: &mut T) {
    t.group = "long";
    let json = String::from(crate::long_json(30_000).line(0));
    let rust = String::from(crate::long_mixed(30_000).line(0));
    let jfs = "let s = \"a ${x + f(1)} \\u{41} b\"; log(\"${s}\") // é\t".repeat(600);
    let ins = ["x", "\"", "\\", "/*", "*/", "'", "\t", "  ", "\u{301}", "\u{1F642}", "東", "{", "}", "[", "${", "\u{200D}", "//", "a = 1;", "r#\""];
    let mut seed = 12345u32;
    let mut rnd = |n: usize| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (seed >> 8) as usize % n.max(1)
    };
    let mut edits: Vec<(usize, usize, &str)> = Vec::new();
    for k in 0..60 {
        let a = rnd(32_000);
        let del = if k % 3 == 0 { rnd(40) } else { 0 };
        edits.push((a, a + del, if k % 5 == 4 { "" } else { ins[rnd(ins.len())] }));
    }
    // at the start, near it, at the end
    edits.extend([(0, 0, "\""), (0, 1, ""), (usize::MAX, usize::MAX, "/*"), (5, 9, "\t\t"), (100, 100, "*/ \"")]);
    for (name, text, h) in [("json", &json, Highlighter::json()), ("rust", &rust, Highlighter::rust()), ("jfs", &jfs, Highlighter::jfs()), ("plain", &rust, Highlighter::plain())] {
        for tw in [4u8, 8] {
            let d = code_long_check(text, &h, tw, &edits);
            t.check(d.is_empty(), &format!("{name}, tab width {tw}: kept like a fresh look ({:?})", d.first()));
        }
    }
}

pub fn run() -> i32 {
    let mut t = T { group: "", ok: 0, failed: 0 };
    rich(&mut t);
    code(&mut t);
    long(&mut t);
    diags(&mut t);
    grammars(&mut t);
    jfs(&mut t);
    out(&format!("model: {} checks, {} failed\n", t.ok + t.failed, t.failed));
    t.failed as i32
}

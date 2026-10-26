// unit -- mdviewtest --test unit: self-checks of the viewer's document (ui::MdDoc) and of the
// height cache of the variable-height list (ui::VListState) through their public API:
// Markdown blocks (headings, paragraphs, lists and their numbers, task items, tight / loose
// spacing, quotes with lazy lines, fenced and indented code, tables, rules, HTML blocks),
// inline parts (emphasis by the delimiter rules, code spans, links of every kind, images,
// bare links, breaks, entities, escapes, inline HTML), HTML documents (the rich-text reader:
// headings, lists, quotes, code, tables, images, rules), text and HTML of ranges; then
// malformed and adversarial input (deep nesting, runs of brackets, stars, backticks, tags,
// unclosed constructs, mutations of the samples) that must neither panic nor take more than
// linear time; the list's heights (Fenwick sums, positions, insertions above the anchor); the
// view's accessibility tree in a headless runtime (headings with their level); forms (fields,
// labels, choices, task boxes of list items, the values in the state, the data a form sends,
// reset) and in a headless runtime the fields' records and the images the app supplies.
// Prints FAIL lines and a summary; the exit code is the number of failures.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::{SAMPLE_HTML, SAMPLE_MD};

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

fn md(s: &str) -> MdDoc {
    MdDoc::markdown(s)
}

// block k as a compact markup: <b> <i> <s> <u> <c> around styled text, [text](url) links,
// ![alt](src) images
fn markup(d: &MdDoc, k: usize) -> String {
    let Some(b) = d.blocks().get(k) else { return String::from("<none>") };
    let mut o = String::new();
    let mut i = 0usize;
    for s in &b.spans {
        let (a, e) = (s.a as usize, s.b as usize);
        o.push_str(&b.text[i..a]);
        i = e;
        if s.image != 0 {
            let m = d.images().get(s.image as usize - 1).cloned().unwrap_or_default();
            if s.link != 0 {
                o.push_str(&format!("[![{}]({})]({})", m.alt, m.src, d.link(s.link)));
            } else {
                o.push_str(&format!("![{}]({})", m.alt, m.src));
            }
            continue;
        }
        let mut open = String::new();
        if s.link != 0 {
            open.push('[');
        }
        for (bit, t) in [(MS_BOLD, "b"), (MS_ITALIC, "i"), (MS_STRIKE, "s"), (MS_UNDER, "u"), (MS_CODE, "c")] {
            if s.style & bit != 0 {
                open.push_str(&format!("<{}>", t));
            }
        }
        // the closing tags in reverse order, the link's end last
        let mut cl = String::new();
        for (bit, t) in [(MS_CODE, "c"), (MS_UNDER, "u"), (MS_STRIKE, "s"), (MS_ITALIC, "i"), (MS_BOLD, "b")] {
            if s.style & bit != 0 {
                cl.push_str(&format!("</{}>", t));
            }
        }
        if s.link != 0 {
            cl.push_str(&format!("]({})", d.link(s.link)));
        }
        o.push_str(&open);
        o.push_str(&b.text[a..e]);
        o.push_str(&cl);
    }
    o.push_str(&b.text[i..]);
    o
}

// block k (an empty one when there is none: a failed check, no panic)
fn bk(d: &MdDoc, k: usize) -> MdBlock {
    d.blocks().get(k).cloned().unwrap_or_default()
}

fn kinds(d: &MdDoc) -> Vec<u8> {
    d.blocks().iter().map(|b| b.kind).collect()
}

fn texts(d: &MdDoc) -> Vec<String> {
    d.blocks().iter().map(|b| b.text.clone()).collect()
}

// a deterministic generator for the mutations
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

fn blocks_md(t: &mut T) {
    t.group = "blocks";
    let d = md("# Eins\n\n## Zwei ##\n\nSetext\n===\n\nNoch eine\n---\n\n####### sieben\n\n#ohne");
    t.eq(kinds(&d), alloc::vec![MB_HEADING, MB_HEADING, MB_HEADING, MB_HEADING, MB_PARA, MB_PARA], "heading kinds");
    t.eq(d.blocks().iter().map(|b| b.level).collect::<Vec<_>>(), alloc::vec![1, 2, 1, 2, 0, 0], "heading levels");
    t.eq(texts(&d), alloc::vec![String::from("Eins"), String::from("Zwei"), String::from("Setext"), String::from("Noch eine"), String::from("####### sieben"), String::from("#ohne")], "heading texts");
    let d = md("eins\nzwei\n\ndrei");
    t.eq(texts(&d), alloc::vec![String::from("eins zwei"), String::from("drei")], "paragraphs, soft break");
    t.eq(bk(&d, 1).space, 2, "a paragraph's gap");
    let d = md("---\n***\n- - -\n___");
    t.eq(kinds(&d), alloc::vec![MB_RULE; 4], "thematic breaks");
    let d = md("```rust\nfn main() {}\n  x\n```\n\n    code\n      more\n\n~~~\nunclosed\n");
    t.eq(kinds(&d), alloc::vec![MB_CODE, MB_CODE, MB_CODE], "code kinds");
    t.eq(bk(&d, 0).lang.as_str(), "rust", "fence language");
    t.eq(bk(&d, 0).text.as_str(), "fn main() {}\n  x", "fence text");
    t.eq(bk(&d, 1).text.as_str(), "code\n  more", "indented code");
    t.eq(bk(&d, 2).text.as_str(), "unclosed", "an unclosed fence runs to the end");
    let d = md("  ```\n  a\n b\nc\n  ```");
    t.eq(bk(&d, 0).text.as_str(), "a\nb\nc", "a fence's indent is removed from its lines");
    let d = md("> eins\nzwei\n>\n> > drei\n\nvier");
    t.eq(texts(&d), alloc::vec![String::from("eins zwei"), String::from("drei"), String::from("vier")], "quotes, lazy line");
    t.eq(d.blocks().iter().map(|b| b.quote).collect::<Vec<_>>(), alloc::vec![1, 2, 0], "quote depths");
    t.eq(bk(&d, 1).path, 3, "the path of a nested quote");
}

fn lists(t: &mut T) {
    t.group = "lists";
    let d = md("- a\n- b\n\n3. x\n4) y\n\n* p\n\n* q");
    t.eq(texts(&d), alloc::vec![String::from("a"), String::from("b"), String::from("x"), String::from("y"), String::from("p"), String::from("q")], "item texts");
    t.eq(d.blocks().iter().map(|b| b.marker).collect::<Vec<_>>(), alloc::vec![MK_BULLET, MK_BULLET, MK_NUMBER, MK_NUMBER, MK_BULLET, MK_BULLET], "markers");
    t.eq(d.blocks().iter().map(|b| b.number).collect::<Vec<_>>(), alloc::vec![0, 0, 3, 4, 0, 0], "numbers");
    t.eq(bk(&d, 3).delim, b')', "delimiter )");
    t.eq(d.blocks().iter().map(|b| b.space).collect::<Vec<_>>(), alloc::vec![0, 1, 2, 1, 2, 2], "tight and loose spacing");
    let d = md("- a\n  - b\n    - c\n- d\n\n  zweiter Absatz");
    t.eq(d.blocks().iter().map(|b| b.list).collect::<Vec<_>>(), alloc::vec![1, 2, 3, 1, 1], "nesting depths");
    t.eq(bk(&d, 4).marker, MK_NONE, "a second paragraph has no marker");
    t.eq(bk(&d, 4).text.as_str(), "zweiter Absatz", "the second paragraph");
    let d = md("- [ ] offen\n- [x] erledigt\n- [X] auch\n- [y] nicht");
    t.eq(d.blocks().iter().map(|b| b.task).collect::<Vec<_>>(), alloc::vec![1, 2, 2, 0], "task items");
    t.eq(bk(&d, 0).text.as_str(), "offen", "a task's text");
    t.eq(bk(&d, 3).text.as_str(), "[y] nicht", "no task");
    let d = md("Absatz\n2. kein Listenpunkt\n- aber dieser");
    t.eq(texts(&d), alloc::vec![String::from("Absatz 2. kein Listenpunkt"), String::from("aber dieser")], "only 1. interrupts a paragraph");
    let d = md("-\n  danach\n- b");
    t.eq(bk(&d, 0).text.as_str(), "danach", "an item that starts empty");
    let d = md("> - a\n> - b\n\n- > c");
    t.eq(d.blocks().iter().map(|b| (b.quote, b.list, b.path)).collect::<Vec<_>>(), alloc::vec![(1, 1, 1), (1, 1, 1), (1, 1, 2)], "quotes and lists: depths and order");
}

fn inlines(t: &mut T) {
    t.group = "inline";
    let m = |s: &str| markup(&md(s), 0);
    t.eq(m("*a* **b** ***c*** _d_ __e__ ~~f~~ ~g~"), String::from("<i>a</i> <b>b</b> <b><i>c</i></b> <i>d</i> <b>e</b> <s>f</s> <s>g</s>"), "emphasis kinds");
    t.eq(m("a*b*c a_b_c"), String::from("a<i>b</i>c a_b_c"), "intraword");
    t.eq(m("**a *b* c**"), String::from("<b>a </b><b><i>b</i></b><b> c</b>"), "nested emphasis");
    t.eq(m("*a **b**"), String::from("*a <b>b</b>"), "an unmatched opener stays text");
    t.eq(m("a * b *"), String::from("a * b *"), "no flanking");
    t.eq(m("a ~~~x~~~"), String::from("a ~~~x~~~"), "three tildes are text");
    t.eq(m("`a` `` a`b `` `x"), String::from("<c>a</c> <c>a`b</c> `x"), "code spans");
    t.eq(m("`*a*` \\`b`"), String::from("<c>*a*</c> `b`"), "no emphasis in code, escaped backtick");
    t.eq(m("[x](http://a.b \"T\") [y](<u v>) [z]()"), String::from("[x](http://a.b) [y](u v) [z]()"), "inline links");
    t.eq(m("[a [b](u) c](v)"), String::from("[a [b](u) c](v)"), "no links in links");
    t.eq(m("[**fett**](u)"), String::from("[<b>fett</b>](u)"), "emphasis in a link");
    t.eq(markup(&md("[x][r] [r][] [r] [R]\n\n[r]: /ziel \"T\""), 0), String::from("[x](/ziel) [r](/ziel) [r](/ziel) [R](/ziel)"), "reference links");
    t.eq(texts(&md("[r]: /ziel\n")), Vec::<String>::new(), "a definition alone is no paragraph");
    t.eq(m("<http://a.b/c> <x@y.de>"), String::from("[http://a.b/c](http://a.b/c) [x@y.de](mailto:x@y.de)"), "autolinks");
    t.eq(m("siehe https://x.org/a. und www.ex.com, (https://y.org/(b))"), String::from("siehe [https://x.org/a](https://x.org/a). und [www.ex.com](http://www.ex.com), ([https://y.org/(b)](https://y.org/(b)))"), "bare links");
    t.eq(m("![alt *x*](p.png \"T\") [![i](q.svg)](u)"), String::from("![alt x](p.png) [![i](q.svg)](u)"), "images");
    let d = md("a  \nb\\\nc\nd");
    t.eq(bk(&d, 0).text.as_str(), "a\nb\nc d", "hard and soft breaks");
    t.eq(m("&copy; &amp; &#65; &#x41; &nope; &#0;"), String::from("© & A A &nope; \u{FFFD}"), "entities");
    t.eq(m("\\*x\\* \\a \\\\"), String::from("*x* \\a \\"), "escapes");
    t.eq(m("<b>x</b> <em>y</em><br>z <kbd>K</kbd> <span>s</span> <!-- weg -->!"), String::from("<b>x</b> <i>y</i>\nz <c>K</c> s !"), "inline HTML");
    t.eq(m("<a href=\"u\">l</a> <img src=\"i.png\" alt=\"I\">"), String::from("[l](u) ![I](i.png)"), "HTML links and images");
    t.eq(m("a < b > c & d"), String::from("a < b > c & d"), "plain < > &");
}

fn tables(t: &mut T) {
    t.group = "tables";
    let d = md("| a | b |\n|:-|-:|\n| 1 | 2 |\n| x \\| y | `z` |\n| nur eins |\n\nnach");
    t.eq(kinds(&d), alloc::vec![MB_ROW, MB_ROW, MB_ROW, MB_ROW, MB_PARA], "rows");
    t.eq(d.tables().len(), 1, "one table");
    t.eq(d.tables().first().map(|x| x.aligns.clone()).unwrap_or_default(), alloc::vec![MA_LEFT, MA_RIGHT], "alignments");
    t.eq(d.blocks().iter().map(|b| b.head).collect::<Vec<_>>(), alloc::vec![true, false, false, false, false], "header row");
    t.eq(bk(&d, 2).text.as_str(), "x | y\tz", "cells and an escaped pipe");
    t.eq(bk(&d, 2).cells.clone(), alloc::vec![(0, 5), (6, 7)], "cell ranges");
    t.eq(markup(&d, 2), String::from("x | y\t<c>z</c>"), "inline parts in cells");
    t.eq(bk(&d, 3).text.as_str(), "nur eins\t", "missing cells are empty");
    t.eq(d.blocks().iter().map(|b| b.space).collect::<Vec<_>>(), alloc::vec![0, 0, 0, 0, 2], "rows without gaps");
    let d = md("Text davor\n| a | b |\n| --- | --- |\n| c | d |");
    t.eq(kinds(&d), alloc::vec![MB_PARA, MB_ROW, MB_ROW], "a table after a paragraph line");
    let d = md("a | b\n--- | ---\nc | d");
    t.eq(kinds(&d), alloc::vec![MB_ROW, MB_ROW], "a table without outer pipes");
    let d = md("| a |\n| - | - |");
    t.eq(kinds(&d), alloc::vec![MB_PARA], "cell counts must match");
}

fn html_md(t: &mut T) {
    t.group = "html";
    let d = md("<div>\n<p>Hallo <b>Welt</b></p>\n</div>\n\nnach");
    t.eq(markup(&d, 0), String::from("Hallo <b>Welt</b>"), "an HTML block");
    t.eq(texts(&d).last().cloned(), Some(String::from("nach")), "after the block");
    let d = MdDoc::html("<h3>T</h3><p>x <a href=\"u\">l</a></p><ol start=\"3\"><li>a</li><li>b</li></ol><ul><li>c<ul><li>d</li></ul></li></ul>");
    t.eq(d.blocks().iter().map(|b| (b.kind, b.level)).collect::<Vec<_>>(), alloc::vec![(MB_HEADING, 3), (MB_PARA, 0), (MB_PARA, 0), (MB_PARA, 0), (MB_PARA, 0), (MB_PARA, 0)], "kinds");
    t.eq(markup(&d, 1), String::from("x [l](u)"), "a link");
    t.eq(d.blocks().iter().skip(2).map(|b| (b.marker, b.number, b.list)).collect::<Vec<_>>(), alloc::vec![(MK_NUMBER, 3, 1), (MK_NUMBER, 4, 1), (MK_BULLET, 0, 1), (MK_BULLET, 0, 2)], "list items");
    let d = MdDoc::html("<table><thead><tr><th>A</th><th align=\"right\">B</th></tr></thead><tr><td> 1 </td><td><b>2</b></td></tr></table>");
    t.eq(kinds(&d), alloc::vec![MB_ROW, MB_ROW], "table rows");
    t.eq(d.blocks().iter().map(|b| b.head).collect::<Vec<_>>(), alloc::vec![true, false], "header row");
    t.eq(d.tables().first().map(|x| x.aligns.clone()).unwrap_or_default(), alloc::vec![MA_NONE, MA_RIGHT], "alignment");
    let r = bk(&d, 1);
    t.eq(r.cells.iter().map(|c| &r.text[c.0 as usize..c.1 as usize]).collect::<Vec<_>>(), alloc::vec!["1", "2"], "cells");
    let d = MdDoc::html("<pre><code class=\"language-rust\">fn a() {}\n\tb\n</code></pre><hr><p>x<img src=\"i.png\" alt=\"I\" width=\"20\">y</p><blockquote><blockquote><p>q</p></blockquote></blockquote>");
    t.eq(kinds(&d), alloc::vec![MB_CODE, MB_RULE, MB_PARA, MB_PARA], "code, rule, image, quote");
    t.eq(bk(&d, 0).lang.as_str(), "rust", "the code's language");
    t.eq(bk(&d, 0).text.as_str(), "fn a() {}\n    b", "the code's lines");
    t.eq(markup(&d, 2), String::from("x![I](i.png)y"), "an image");
    t.eq(d.images().first().map_or(0, |m| m.w), 20, "the image's width");
    t.eq(bk(&d, 3).quote, 2, "quote depth");
    // the rich-text editor reads the same HTML as before
    let r = RichDoc::from_html("<p>a<img src=x>b</p><hr><table><tr><td>c</td><td>d</td></tr></table>");
    t.eq(r.text(), String::from("ab\ncd"), "the rich-text reader is unchanged");
}

fn export(t: &mut T) {
    t.group = "export";
    let d = md("# T\n\n- a\n- b\n\n| x | y |\n|---|---|\n| 1 | 2 |\n\n1. [x] *i* ![B](b.png)");
    t.eq(d.plain(), String::from("T\n\n- a\n- b\n\nx\ty\n1\t2\n\n1. [x] i B"), "plain text");
    t.eq(
        d.to_html(),
        String::from("<h1>T</h1><ul><li><p>a</p></li><li><p>b</p></li></ul><table><tr><th>x</th><th>y</th></tr><tr><td>1</td><td>2</td></tr></table><ol><li><input type=\"checkbox\" checked disabled> <p><em>i</em> <img src=\"b.png\" alt=\"B\"></p></li></ol>"),
        "HTML",
    );
    let d = md("Hallo *Welt*\n\n```\ncode\n```");
    t.eq(d.plain_range(TextPos::new(0, 1), TextPos::new(0, 8)), String::from("allo We"), "a part of a block");
    t.eq(d.html_range(TextPos::new(0, 6), TextPos::new(1, 2)), String::from("<p><em>Welt</em></p><pre><code>co</code></pre>"), "HTML of a range");
    t.eq(d.plain_range(TextPos::new(1, 2), TextPos::new(0, 6)), String::from("Welt\n\nco"), "reversed ends");
    let d = md("[a](u) und [b](v)\n\n<https://x.y>");
    t.eq(d.link_spots().iter().map(|s| (s.block, s.a, s.b)).collect::<Vec<_>>(), alloc::vec![(0, 0, 1), (0, 6, 7), (1, 0, 11)], "links in reading order");
    t.eq(d.link(d.link_spots().get(1).map_or(0, |s| s.link)), "v", "a spot's target");
}

fn samples(t: &mut T) {
    t.group = "samples";
    let d = md(SAMPLE_MD);
    t.check(!d.truncated(), "the sample is whole");
    t.check(d.blocks().iter().any(|b| b.kind == MB_ROW), "the sample has a table");
    t.check(d.blocks().iter().any(|b| b.kind == MB_CODE && b.lang == "rust"), "the sample has Rust code");
    t.eq(d.images().len(), 3, "the sample's images");
    t.check(d.blocks().iter().any(|b| b.text.starts_with("שלום")), "Hebrew");
    let h = MdDoc::html(SAMPLE_HTML);
    t.check(h.blocks().iter().any(|b| b.kind == MB_ROW && b.head), "the HTML sample's table");
    t.check(h.blocks().iter().any(|b| b.kind == MB_CODE && b.lang == "json"), "the HTML sample's code");
}

// a case of adversarial input: parsed within `ms` milliseconds without a panic
fn timed(t: &mut T, name: &str, ms: u64, f: &dyn Fn() -> usize) {
    let t0 = plat::time_us();
    let n = f();
    let dt = (plat::time_us() - t0) / 1000;
    t.check(dt <= ms, &format!("{}: {} ms (limit {} ms, {} blocks)", name, dt, ms, n));
}

fn rep(s: &str, n: usize) -> String {
    let mut o = String::with_capacity(s.len() * n);
    for _ in 0..n {
        o.push_str(s);
    }
    o
}

fn adversarial(t: &mut T) {
    t.group = "adversarial";
    let d = md(&format!("{}x", rep("> ", 10_000)));
    t.check(d.blocks().iter().all(|b| b.quote <= 32), "nesting of quotes capped");
    let d = md(&format!("{}x", rep("- ", 10_000)));
    t.check(d.blocks().iter().all(|b| b.list <= 32), "nesting of lists capped");
    let d = md(&rep("1. a\n   ", 5000));
    t.check(d.blocks().iter().all(|b| b.list <= 32), "nesting of numbered items capped");
    let lim = 2500;
    let cases: Vec<(&str, String)> = alloc::vec![
        ("brackets", rep("[", 200_000)),
        ("closing brackets", rep("]", 200_000)),
        ("link openers", rep("[a](", 60_000)),
        ("image openers", rep("![", 100_000)),
        ("nested images", format!("{}{}", rep("![", 30_000), rep("](x)", 30_000))),
        ("nested links", format!("{}{}", rep("[", 30_000), rep("](x)", 30_000))),
        ("stars", rep("*", 200_000)),
        ("star words", rep("*a ", 80_000)),
        ("underscores", rep("_a_b ", 60_000)),
        ("tildes", rep("~~a ", 60_000)),
        ("backticks", rep("` `` ``` ", 30_000)),
        ("tags", rep("<a ", 100_000)),
        ("quoted tags", rep("<a '", 100_000)),
        ("comments", rep("<!--", 60_000)),
        ("entities", rep("&", 200_000)),
        ("entity names", rep("&abcdefgh", 30_000)),
        ("backslashes", rep("\\", 200_000)),
        ("autolinks", rep("<http://", 40_000)),
        ("bare links", rep("https://a.b ", 40_000)),
        ("reference labels", rep("[a][", 60_000)),
        ("definitions", rep("[a]: <", 40_000)),
        ("parens", format!("[a]({}", rep("(", 100_000))),
        ("table rows", format!("| a | b |\n|---|---|\n{}", rep("| x | y |\n", 40_000))),
        ("unclosed fence", format!("```\n{}", rep("x\n", 100_000))),
        ("lines", rep("text\n", 200_000)),
        ("long line", rep("wort ", 200_000)),
        ("html block", format!("<div>\n{}", rep("<div><b>x", 30_000))),
        ("emphasis mix", rep("**a*b_c__d~~e", 30_000)),
    ];
    for (name, s) in &cases {
        timed(t, name, lim, &|| md(s).blocks().len());
    }
    let html_cases: Vec<(&str, String)> = alloc::vec![
        ("html divs", rep("<div>", 100_000)),
        ("html formats", rep("<b><i>", 60_000)),
        ("html lists", rep("<ul><li>", 50_000)),
        ("html cells", rep("<table><tr><td>", 30_000)),
        ("html pre", rep("<pre>x\n", 30_000)),
        ("html images", rep("<img src=x>", 50_000)),
        ("html unclosed tag", format!("<p {}", rep("a", 200_000))),
        ("html inputs", rep("<input name=a>", 80_000)),
        ("html selects", rep("<select><option>x", 40_000)),
        ("html options", format!("<select>{}", rep("<option value=v>t", 60_000))),
        ("html buttons", rep("<button><b>x", 40_000)),
        ("html text areas", rep("<textarea>x", 40_000)),
        ("html labels", rep("<label for=a>x<input id=a>", 30_000)),
        ("html forms", rep("<form action=x><input></form>", 30_000)),
    ];
    for (name, s) in &html_cases {
        timed(t, name, lim, &|| MdDoc::html(s).blocks().len());
    }
    let d = MdDoc::html(&rep("<input>", 70_000));
    t.check(d.fields().len() <= 0xFFFE && valid(&d), "fields capped");
    let d = MdDoc::html(&format!("<select>{}", rep("<option>t", 10_000)));
    t.check(d.fields().first().is_some_and(|f| f.options.len() <= 4096), "choices capped");
    // prefixes and mutations of the samples: no panics, ranges inside the texts
    let mut rng = Lcg(0x5EED);
    let syms = b"*_~`[]()<>!#|-+>\\&;:\n \t1.x";
    for (si, src) in [SAMPLE_MD, SAMPLE_HTML].iter().enumerate() {
        let b = src.as_bytes();
        let mut ok = true;
        for cut in (0..b.len()).step_by(7) {
            let mut k = cut;
            while !src.is_char_boundary(k) {
                k -= 1;
            }
            let d = if si == 0 { md(&src[..k]) } else { MdDoc::html(&src[..k]) };
            ok &= valid(&d);
        }
        t.check(ok, &format!("prefixes of sample {}", si));
        let mut ok = true;
        for _ in 0..300 {
            let mut v = b.to_vec();
            for _ in 0..1 + rng.next() % 12 {
                let p = rng.next() as usize % v.len();
                match rng.next() % 3 {
                    0 => v[p] = syms[rng.next() as usize % syms.len()],
                    1 => v.insert(p, syms[rng.next() as usize % syms.len()]),
                    _ => {
                        v.remove(p);
                    }
                }
            }
            let s = String::from_utf8_lossy(&v);
            let d = if si == 0 { md(&s) } else { MdDoc::html(&s) };
            ok &= valid(&d);
            let _ = d.plain();
            let _ = d.to_html();
        }
        t.check(ok, &format!("mutations of sample {}", si));
    }
    let d = md("\0a\u{FEFF}b\r\nc\rd");
    t.eq(texts(&d), alloc::vec![String::from("\u{FFFD}a\u{FEFF}b c d")], "NUL, BOM, CR line ends");
}

// spans inside the text, ascending, on character boundaries; links and images exist
fn valid(d: &MdDoc) -> bool {
    d.blocks().iter().all(|b| {
        let mut prev = 0;
        b.spans.iter().all(|s| {
            let ok = s.a <= s.b
                && s.a >= prev
                && (s.b as usize) <= b.text.len()
                && b.text.is_char_boundary(s.a as usize)
                && b.text.is_char_boundary(s.b as usize)
                && (s.link as usize) <= d.links().len()
                && (s.image as usize) <= d.images().len()
                && (s.field as usize) <= d.fields().len();
            prev = s.b;
            ok
        }) && b.cells.iter().all(|c| c.0 <= c.1 && (c.1 as usize) <= b.text.len())
    })
}

fn heights(t: &mut T) {
    t.group = "vlist";
    let mut st = VListState::new();
    st.set_count(1000);
    t.eq(st.total_height(), 40_000, "estimates");
    st.set_height(0, 100);
    st.set_height(999, 10);
    t.eq(st.total_height(), 40_030, "given heights");
    st.scroll_to(10);
    t.eq(st.scroll_pos(), 100 + 9 * 40, "the position of a row");
    st.insert(5, 3);
    t.eq(st.first_visible(), (13, 0), "rows inserted above keep the anchor");
    t.eq(st.total_height(), 40_150, "inserted rows count");
    t.eq((st.row_height(0), st.row_height(5), st.row_height(1002)), (100, 40, 10), "heights move with their rows");
    st.remove(0, 2);
    t.eq(st.first_visible(), (11, 0), "removed rows above");
    st.remove(10, 5);
    t.eq(st.first_visible(), (10, 0), "the anchor's row removed");
    st.sel = 500;
    st.insert(0, 1);
    t.eq(st.sel, 501, "the selection stays on its row");
    // 100 000 rows: positions through the Fenwick sums
    let mut st = VListState::new();
    let n = 100_000usize;
    st.set_count(n);
    let mut sum = 0i64;
    for i in 0..n {
        let h = 20 + (i % 7) as i32 * 9;
        st.set_height(i, h);
        sum += h as i64;
    }
    t.eq(st.total_height(), sum, "the total of 100 000 rows");
    let mut ok = true;
    let mut rng = Lcg(7);
    for _ in 0..2000 {
        let p = (rng.next() as i64 * 7919) % (sum - 1);
        st.set_scroll_pos(p);
        ok &= st.scroll_pos() == p;
        let (top, off) = st.first_visible();
        ok &= off >= 0 && off < st.row_height(top);
    }
    t.check(ok, "positions round-trip");
    st.set_scroll_pos(sum * 2);
    t.eq(st.first_visible().0, n - 1, "beyond the end: the last row");
}

// the view in a headless runtime as screen readers get it (ui/a11y.rs): headings with their
// level inside the document's group, the paragraph a label, the link a link
fn a11y(t: &mut T) {
    use plat::a11y::Role;
    t.group = "a11y";
    struct View(MdViewState);
    impl App for View {
        fn frame(&mut self, ui: &mut Ui) {
            ui.next_size(-1, -1);
            ui.markdown_view(1, &mut self.0, "# Eins\n\nEin [Link](https://example.org).\n\n### Drei\n\n<h5>Fünf</h5>", MV_NO_FRAME);
        }
    }
    const D: AppDesc = AppDesc { title: "a11y", w: 480, h: 320, ..AppDesc::NONE };
    let Ok(mut rt) = ui::Runtime::new(&D, View(MdViewState::new()), &style::STYLE) else {
        t.check(false, "the view starts headless");
        return;
    };
    rt.step(50, &[]);
    let s = rt.ui().a11y_tree();
    let Some(w) = s.wins.first() else { return t.check(false, "a window") };
    let heads: Vec<(String, u32)> = w.nodes.iter().filter(|n| n.role == Role::Heading).map(|n| (n.name.clone(), n.level())).collect();
    t.eq(heads, alloc::vec![(String::from("Eins"), 1), (String::from("Drei"), 3), (String::from("Fünf"), 5)], "headings and their levels");
    let doc = w.nodes.iter().position(|n| n.role == Role::Group && n.name == "Dokument");
    t.check(
        doc.is_some_and(|d| w.nodes.iter().filter(|n| n.role == Role::Heading).all(|n| n.parent == d as u32)),
        "the headings lie in the document's group",
    );
    t.check(w.nodes.iter().any(|n| n.role == Role::Label && n.name == "Ein Link."), "a paragraph is a label");
    t.check(w.nodes.iter().any(|n| n.role == Role::Link && n.name == "Link"), "a link");
}

fn forms(t: &mut T) {
    t.group = "forms";
    let d = MdDoc::html(concat!(
        "<p>vor</p><form action=\" /a \" method=\"POST\" id=\"f\"><p><label for=\"n\">Name</label> <input id=\"n\" name=\"n\" value=\"x\" size=\"10\" maxlength=\"5\"></p>",
        "<p><label><input type=checkbox name=c checked> Ja</label> <input type=radio name=r value=1> <input type=radio name=r value=2 checked aria-label=\"Zwei\"></p>",
        "<select name=s><option>A</option><option value=b selected> B  b </option></select><textarea name=t rows=4 cols=9>\nz &amp; <b>y</b></textarea>",
        "<input type=hidden name=h value=v><button name=go value=1>Los <b>jetzt</b></button><input type=submit><input type=reset>",
        "<input type=password name=p title=\"Geheim\"></form><p>nach</p><input name=out form=f><form><input name=inner></form>"
    ));
    t.eq(d.forms().len(), 2, "two forms");
    t.eq(d.forms().first().map(|f| (f.action.clone(), f.method.clone())), Some((String::from("/a"), String::from("post"))), "action and method");
    t.eq(d.fields().iter().map(|f| f.kind).collect::<Vec<_>>(), alloc::vec![MF_TEXT, MF_CHECKBOX, MF_RADIO, MF_RADIO, MF_SELECT, MF_TEXTAREA, MF_HIDDEN, MF_SUBMIT, MF_SUBMIT, MF_RESET, MF_PASSWORD, MF_TEXT, MF_TEXT], "kinds");
    t.eq(d.fields().iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), alloc::vec!["Name", "Ja", "", "Zwei", "", "", "", "Los jetzt", "", "", "Geheim", "", ""], "labels, aria-label, title, captions");
    t.eq(d.fields().iter().map(|f| f.form).collect::<Vec<_>>(), alloc::vec![1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2], "forms (form= the last but one)");
    t.eq(d.fields().iter().map(|f| f.value.as_str()).collect::<Vec<_>>(), alloc::vec!["x", "on", "1", "2", "", "z & <b>y</b>", "v", "1", "", "", "", "", ""], "values (a text area's text raw)");
    let s = d.fields().get(4).cloned().unwrap_or_default();
    t.eq(s.options.iter().map(|o| (o.value.as_str(), o.label.as_str())).collect::<Vec<_>>(), alloc::vec![("A", "A"), ("b", "B b")], "choices");
    t.eq(s.selected, 1, "the chosen one");
    t.eq((d.fields()[0].size, d.fields()[0].maxlength, d.fields()[5].rows, d.fields()[5].size), (10, 5, 4, 9), "sizes");
    t.eq(d.field_spots().len(), 12, "visible fields (not the hidden one)");
    t.check(d.field_spots().windows(2).all(|w| (w[0].block, w[0].a) < (w[1].block, w[1].a)), "field spots in reading order");
    t.check(d.blocks().iter().any(|b| b.text == "vor") && d.blocks().iter().any(|b| b.text == "nach"), "a form is a block of its own");
    t.check(!d.fields()[8].caption().is_empty() && d.fields()[7].caption() == "Los jetzt", "captions");
    t.check(d.plain().contains("Los jetzt") && !d.plain().contains('\u{FFFC}'), "plain text: captions, no field characters");
    t.check(!d.to_html().contains("<input"), "HTML: no fields");
    // a disabled check box at the start of a list item is its task box (html_range writes them)
    let d = MdDoc::html("<ul><li><input type=checkbox disabled checked> fertig</li><li><input type=\"checkbox\" disabled> offen</li></ul>");
    t.eq(d.blocks().iter().map(|b| (b.task, b.text.clone())).collect::<Vec<_>>(), alloc::vec![(2, String::from("fertig")), (1, String::from("offen"))], "task boxes");
    t.eq(d.fields().len(), 0, "task boxes are no fields");
    let d = MdDoc::html(&md("- [x] a\n- [ ] b").to_html());
    t.eq(d.blocks().iter().map(|b| b.task).collect::<Vec<_>>(), alloc::vec![2, 1], "task items round trip through HTML");
    // fields in Markdown's HTML blocks, the rich-text reader unchanged
    let d = md("Text\n\n<form action=\"/q\">\n<input name=q> <button>Los</button>\n</form>\n\n*nach*");
    t.eq((d.forms().len(), d.fields().len(), d.fields().first().map_or(0, |f| f.form)), (1, 2, 1), "a form in an HTML block");
    t.eq(RichDoc::from_html("<p>a<input value=x>b<select><option>o</option></select></p>").text(), String::from("abo"), "the rich-text reader ignores fields");
    // the values in the view's state
    let mut st = MdViewState::new();
    st.set_html("<form action=/s><input name=a value=1><input type=radio name=r value=x checked><input type=radio name=r value=y><select name=s><option>p<option>q</select><input type=checkbox name=c><button name=b value=k>B</button><input disabled name=d value=no></form>");
    t.eq((st.field_text(0), st.field_checked(1), st.field_checked(2), st.field_selected(3)), ("1", true, false, 0), "first values");
    st.set_field_text(0, "neu");
    st.set_field_checked(2, true);
    st.set_field_checked(4, true);
    st.set_field_selected(3, 1);
    t.eq((st.field_checked(1), st.field_checked(2)), (false, true), "one radio of a group");
    let pairs = |v: Vec<(String, String)>| v.iter().map(|(n, v)| format!("{}={}", n, v)).collect::<Vec<_>>().join("&");
    t.eq(pairs(st.form_data(0, Some(5))), String::from("a=neu&r=y&s=q&c=on&b=k"), "form data with its button");
    t.eq(pairs(st.form_data(0, None)), String::from("a=neu&r=y&s=q&c=on"), "form data without a button");
    st.reset_form(0);
    t.eq(pairs(st.form_data(0, None)), String::from("a=1&r=x&s=p"), "reset");
    st.set_field_selected(3, 7);
    t.eq(st.field_selected(3), -1, "a choice out of range");
}

// the view in a headless runtime: forms as screen readers get them (names from the labels,
// the password protected and dotted), images the app supplies (asked once, a placeholder of
// the given size, the picture's size when the bytes come, the alt text when they fail)
fn form_view(t: &mut T) {
    use alloc::rc::Rc;
    use core::cell::RefCell;
    use plat::a11y::{Role, S_CHECKED, S_PROTECTED};
    t.group = "form view";
    const PAGE: &str = concat!(
        "<form><p><label for=a>Name</label> <input id=a name=a></p><p><label>Kennwort <input type=password name=p value=\"geheim\"></label></p>",
        "<p><label><input type=checkbox name=c checked> Merken</label></p><p><select aria-label=\"Land\"><option>A</option></select> <button>Los</button></p></form>",
        "<p><img src=\"https://x.org/a.png\" alt=\"A\" width=\"20\" height=\"10\"><img src=\"https://x.org/b.svg\" alt=\"B\"><img src=\"data:image/png;base64,xx\" alt=\"C\"><img src=\"http://x.org/b.svg\" alt=\"D\"></p>"
    );
    #[derive(Default)]
    struct Io {
        asked: Vec<String>,
        give: Vec<(String, Option<Vec<u8>>)>,
    }
    struct View(MdViewState, Rc<RefCell<Io>>);
    impl App for View {
        fn frame(&mut self, ui: &mut Ui) {
            for (a, d) in core::mem::take(&mut self.1.borrow_mut().give) {
                self.0.set_image(&a, d.as_deref());
            }
            ui.next_size(-1, -1);
            ui.html_view(1, &mut self.0, PAGE, MV_NO_FRAME);
            self.1.borrow_mut().asked.extend(self.0.image_requests());
        }
    }
    const D: AppDesc = AppDesc { title: "forms", w: 640, h: 480, ..AppDesc::NONE };
    let io = Rc::new(RefCell::new(Io::default()));
    let mut st = MdViewState::new();
    st.images = MI_ASK;
    let Ok(mut rt) = ui::Runtime::new(&D, View(st, io.clone()), &style::STYLE) else {
        t.check(false, "the view starts headless");
        return;
    };
    rt.step(50, &[]);
    let tree = |rt: &ui::Runtime| rt.ui().a11y_tree().wins.first().map(|w| w.nodes.clone()).unwrap_or_default();
    let nodes = tree(&rt);
    let find = |role: Role, name: &str| nodes.iter().find(|n| n.role == role && n.name == name).cloned();
    t.check(find(Role::TextBox, "Name").is_some(), "a text box named by its label");
    let pw = find(Role::TextBox, "Kennwort");
    t.check(pw.as_ref().is_some_and(|n| n.state & S_PROTECTED != 0 && !n.text.contains("geheim")), "a password field: protected, its text never read out");
    t.check(find(Role::CheckBox, "Merken").is_some_and(|n| n.state & S_CHECKED != 0), "a check box named by its label");
    t.check(find(Role::Combo, "Land").is_some(), "a list named by aria-label");
    t.check(find(Role::Button, "Los").is_some(), "a button");
    t.eq(io.borrow().asked.clone(), alloc::vec![String::from("https://x.org/a.png"), String::from("https://x.org/b.svg"), String::from("data:image/png;base64,xx"), String::from("http://x.org/b.svg")], "the addresses asked, each once");
    let img_w = |rt: &ui::Runtime, name: &str| tree(rt).iter().find(|n| n.role == Role::Image && n.name == name).map_or(0, |n| n.rect[2]);
    t.eq(img_w(&rt, "A"), 20, "a placeholder of the size the HTML gives");
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"30\" height=\"12\"><rect width=\"30\" height=\"12\"/></svg>".to_vec();
    io.borrow_mut().give = alloc::vec![(String::from("https://x.org/b.svg"), Some(svg)), (String::from("https://x.org/a.png"), None)];
    // (the bytes came outside a frame: the app asks for one)
    rt.command(1, 0);
    rt.step(100, &[]);
    t.eq(img_w(&rt, "B"), 30, "the picture when its bytes come");
    t.check(img_w(&rt, "A") != 20 && img_w(&rt, "A") > 0, "the alt text when they fail");
    t.eq(io.borrow().asked.len(), 4, "nothing asked twice (http: and https: are two addresses)");
}

pub fn run() -> i32 {
    let mut t = T { group: "", ok: 0, failed: 0 };
    let t0 = plat::time_us();
    a11y(&mut t);
    forms(&mut t);
    form_view(&mut t);
    blocks_md(&mut t);
    lists(&mut t);
    inlines(&mut t);
    tables(&mut t);
    html_md(&mut t);
    export(&mut t);
    samples(&mut t);
    adversarial(&mut t);
    heights(&mut t);
    let dt = (plat::time_us() - t0) / 1000;
    out(&format!("mdviewtest --test unit: {} passed, {} failed ({} ms)\n", t.ok, t.failed, dt));
    t.failed as i32
}

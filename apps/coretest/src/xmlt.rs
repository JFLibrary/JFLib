// xmlt -- checks of formats::xml's two readers: the strict Reader (events, namespaces, errors and their
// offsets, skip, the declaration, limits, no recursion), the Lenient tokenizer (attributes, references,
// a DOCTYPE's entities and their budget, broken markup), the tree on top; and a fixed-seed corpus of
// random and mutated documents through all three: nothing panics, offsets stay in the text, the lenient
// reader always comes to an end

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use formats::xml::{self, Error, Event, Lenient, Reader, Token};

use crate::T;

// every event of a document, or the error and its offset
fn events(s: &str) -> Result<Vec<String>, (Error, usize)> {
    let mut r = Reader::new(s, 64, 64);
    let mut v = Vec::new();
    loop {
        match r.next() {
            Ok(Event::Eof) => return Ok(v),
            Ok(Event::Start(st)) => {
                let attrs: Vec<String> = st.attrs.iter().map(|a| format!("{{{}}}{}={}", a.ns, a.local, a.value)).collect();
                v.push(format!("<{{{}}}{} {}{}>", st.ns, st.local, attrs.join(" "), if st.empty { "/" } else { "" }));
            }
            Ok(Event::End { .. }) => v.push(String::from("</>")),
            Ok(Event::Text(t)) => v.push(format!("t:{t}")),
            Ok(Event::CData(t)) => v.push(format!("c:{t}")),
            Ok(Event::Comment(c)) => v.push(format!("#:{c}")),
            Ok(Event::Pi { target, data }) => v.push(format!("?:{target}:{data}")),
            Ok(_) => v.push(String::from("?")),
            Err(e) => return Err((e.error, e.at)),
        }
    }
}

fn lenient(s: &str) -> Vec<String> {
    let mut l = Lenient::new(s);
    let mut v = Vec::new();
    loop {
        match l.next() {
            Token::Start { name, attrs, empty } => {
                let a: Vec<String> = attrs.iter().map(|(n, raw)| {
                    let mut d = String::new();
                    l.decode(raw, &mut d);
                    format!("{n}={d}")
                }).collect();
                v.push(format!("<{name} {}{}>", a.join(" "), if empty { "/" } else { "" }));
            }
            Token::End { name } => v.push(format!("</{name}>")),
            Token::Text(raw) => {
                let mut d = String::new();
                l.decode(raw, &mut d);
                v.push(format!("t:{d}"));
            }
            Token::CData(raw) => v.push(format!("c:{raw}")),
            Token::Eof => return v,
            _ => v.push(String::from("?")),
        }
    }
}

fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| String::from(*s)).collect()
}

pub fn run(t: &mut T) {
    // the strict reader: namespaces, references, white space, CDATA, comments, processing instructions
    let doc = "\u{feff}<?xml version='1.0' encoding='utf-8'?>\n<!-- c --><a xmlns='urn:a' xmlns:p='urn:p' p:x='1&amp;2' y=\"a\tb\r\nc\">\
               t&lt;&#x41;&#66;\r\n<p:b/><![CDATA[<raw>]]><?pi data?></a>\n";
    t.eq("events", events(doc), Ok(sv(&[
        "#: c ",
        "<{urn:a}a {http://www.w3.org/2000/xmlns/}=urn:a {http://www.w3.org/2000/xmlns/}p=urn:p {urn:p}x=1&2 {}y=a b c>",
        "t:t<AB\n",
        "<{urn:p}b />",
        "</>",
        "c:<raw>",
        "?:pi:data",
        "</>",
    ])));
    // text is borrowed when nothing had to be replaced
    let mut r = Reader::new("<a>plain</a>", 4, 4);
    let _ = r.next();
    t.check("plain text is borrowed", matches!(r.next(), Ok(Event::Text(Cow::Borrowed("plain")))));
    // the attribute helpers
    let mut r = Reader::new("<p:a xmlns:p='urn:p' p:k='v' k='w'/>", 4, 8);
    let st = match r.next() {
        Ok(Event::Start(s)) => Some(s),
        _ => None,
    };
    t.check("Start::is and attr", st.as_ref().is_some_and(|s| s.is("urn:p", "a") && s.attr("urn:p", "k") == Some("v") && s.attr("", "k") == Some("w") && s.attr("", "p").is_none()));
    // errors and where they are
    let bad: &[(&str, Error, usize)] = &[
        ("<a>", Error::Syntax, 3),
        ("<a></b>", Error::Syntax, 6),
        ("<a/><b/>", Error::Syntax, 4),
        ("text<a/>", Error::Syntax, 0),
        ("<a>&nbsp;</a>", Error::Syntax, 3),
        ("<a>]]></a>", Error::Syntax, 3),
        ("<a x='1' x='2'/>", Error::Syntax, 0),
        ("<a xmlns:p='u' xmlns:q='u' p:x='1' q:x='2'/>", Error::Syntax, 0),
        ("<p:a/>", Error::Syntax, 0),
        ("<a:1 xmlns:a='u'/>", Error::Syntax, 4),
        ("<!DOCTYPE a><a/>", Error::Unsupported, 0),
        ("<?xml version='1.0' encoding='ISO-8859-1'?><a/>", Error::Unsupported, 0),
        ("<?xml version='1.1'?><a/>", Error::Unsupported, 0),
        ("<a/><?xml version='1.0'?>", Error::Syntax, 4),
        ("<a><!-- a -- b --></a>", Error::Syntax, 3),
        ("<a><?></a>", Error::Syntax, 3),
        ("<?><a/>", Error::Syntax, 0),
        ("<a>\u{1}</a>", Error::Syntax, 3),
        ("<a xmlns:xml='urn:x'/>", Error::Syntax, 0),
    ];
    for (s, e, at) in bad {
        t.eq(&format!("error of {s:?}"), events(s).err(), Some((*e, *at)));
    }
    // the limits: depth (5000 levels, no recursion) and attributes
    let deep = format!("{}{}", "<a>".repeat(5000), "</a>".repeat(5000));
    let mut r = Reader::new(&deep, 100, 8);
    let mut n = 0;
    let err = loop {
        match r.next() {
            Ok(Event::Eof) => break None,
            Ok(_) => n += 1,
            Err(e) => break Some(e),
        }
    };
    t.eq("depth 100 of 5000", (n, err.map(|e| (e.error, e.at))), (100, Some((Error::Depth, 300))));
    let mut r = Reader::new(&deep, 6000, 8);
    let mut ok = true;
    loop {
        match r.next() {
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => {
                ok = false;
                break;
            }
        }
    }
    t.check("5000 levels read without recursion", ok);
    t.eq("attributes", events("<a a='' b='' c='' d=''/>").map(|v| v.len()), Ok(2));
    let mut r = Reader::new("<a a='' b='' c=''/>", 4, 2);
    t.eq("over the attribute limit", r.next().err().map(|e| e.error), Some(Error::Attributes));
    // skip: the raw text of an element
    let src = "<r><keep x='1'>a<b/>&amp;<c>d</c></keep><next/></r>";
    let mut r = Reader::new(src, 8, 8);
    let _ = r.next();
    let raw = match r.next() {
        Ok(Event::Start(s)) => r.skip(s.at).ok(),
        _ => None,
    };
    t.eq("skip returns the element as written", raw, Some("<keep x='1'>a<b/>&amp;<c>d</c></keep>"));
    t.check("... and goes on after it", matches!(r.next(), Ok(Event::Start(s)) if s.local == "next"));

    // the lenient reader
    t.eq("lenient: entities of the DOCTYPE", lenient("<!DOCTYPE s [<!ENTITY a 'Alpha'><!ENTITY b \"&a; &#66;\"><!ENTITY % p 'x'>]><t>&a;|&b;|&p;|&c;</t>"),
         sv(&["<t >", "t:Alpha|&a; B|&p;|&c;", "</t>"]));
    t.eq("lenient: broken markup", lenient("<a x=1 y z='q' x='dup' \" =/ w=\"v\"><b <c/>AT&T &amp; &#0;&#xZZ;< d</a"),
         sv(&["<a x=1 y= z=q w=v>", "<b >", "<c />", "t:AT&T & \u{FFFD}\u{FFFD}", "t:<", "t: d", "</a>"]));
    t.eq("lenient: CDATA, comments, declarations", lenient("<?xml?><!--x--><a><![CDATA[&amp;]]><!ELEMENT a><?pi?>b</a>"),
         sv(&["<a >", "c:&amp;", "t:b", "</a>"]));
    t.eq("lenient: unterminated", lenient("<a>x<![CDATA[y"), sv(&["<a >", "t:x", "c:y"]));
    let mut l = Lenient::new("<a b='c");
    let _ = l.next();
    t.check("lenient: the end inside a tag is truncated", l.truncated());
    // the entities' budget: a billion laughs stops at it
    let mut bomb = String::from("<!DOCTYPE s [<!ENTITY a0 'xxxxxxxxxxxxxxxxxxxx'>");
    for i in 1..10 {
        let refs: String = (0..10).map(|_| format!("&a{};", i - 1)).collect();
        bomb.push_str(&format!("<!ENTITY a{i} '{refs}'>"));
    }
    bomb.push_str("]><t>&a9;");
    bomb.push_str(&"&a0;".repeat(100));
    bomb.push_str("</t>");
    let mut l = Lenient::with_limits(&bomb, 16, 256, 1000);
    let mut text = String::new();
    loop {
        match l.next() {
            Token::Text(raw) => l.decode(raw, &mut text),
            Token::Eof => break,
            _ => {}
        }
    }
    t.check("lenient: entity values are not expanded again (a9 is its references as written)", text.starts_with("&a8;&a8;"));
    t.check("lenient: the budget of entity bytes holds", text.len() <= 1000 && l.truncated());

    // the tree on the strict reader
    t.check("tree: a comment is refused", xml::parse("<a><!-- x --></a>") == Err(Error::Unsupported));
    t.check("tree: an unknown entity is no well-formed XML", xml::parse("<a>&nbsp;</a>") == Err(Error::Syntax));
    t.check("tree: nesting beyond the limit", xml::parse_with(&"<a>".repeat(60), xml::Limits { depth: 48, ..xml::Limits::default() }) == Err(Error::Limit));
    t.check("tree: names beyond ASCII", xml::parse("<p:größe xmlns:p='urn:x' p:länge='1'/>").is_ok_and(|n| n.is("urn:x", "größe") && n.attr("urn:x", "länge") == Some("1")));

    // a fixed-seed corpus through all three: no panic, offsets in the text, the lenient reader ends
    let seeds = [doc, src, "<!DOCTYPE s [<!ENTITY e 'v'>]><svg:svg><svg:g fill=red><text>&e;&#x41;</text></svg:g></svg:svg>",
                 "<w:r xmlns:w='urn:w'><w:t xml:space='preserve'> a </w:t></w:r>"];
    let tokens: [&str; 16] = ["<", ">", "&", ";", "'", "\"", "=", "/", "</", "/>", "<![CDATA[", "]]>", "<!--", "-->", "&e;", "\u{feff}"];
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move || {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x9e37_79b9_7f4a_7c15)
    };
    let (mut cases, mut bad_pos, mut stuck) = (0u64, 0u64, 0u64);
    for k in 0..30_000usize {
        let mut s = String::from(seeds[k % seeds.len()]);
        for _ in 0..1 + rnd() % 6 {
            let mut at = (rnd() as usize) % (s.len() + 1);
            while !s.is_char_boundary(at) {
                at -= 1;
            }
            match rnd() % 3 {
                0 if at < s.len() => {
                    let mut e = at + 1;
                    while !s.is_char_boundary(e) {
                        e += 1;
                    }
                    s.replace_range(at..e, "");
                }
                _ => s.insert_str(at, tokens[(rnd() as usize) % tokens.len()]),
            }
        }
        let mut r = Reader::new(&s, 32, 16);
        for _ in 0..10_000 {
            match r.next() {
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => {
                    if e.at > s.len() {
                        bad_pos += 1;
                    }
                    break;
                }
            }
        }
        let mut l = Lenient::new(&s);
        let mut steps = 0;
        loop {
            match l.next() {
                Token::Eof => break,
                Token::Text(raw) => {
                    let mut d = String::new();
                    l.decode(raw, &mut d);
                }
                _ => {}
            }
            steps += 1;
            if steps > s.len() + 2 {
                stuck += 1;
                break;
            }
        }
        let _ = xml::parse(&s);
        cases += 1;
    }
    t.eq("corpus: offsets of errors within the text, the lenient reader never stuck", (bad_pos, stuck, cases > 0), (0, 0, true));
}

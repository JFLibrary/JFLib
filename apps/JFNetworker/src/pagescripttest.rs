// pagescripttest -- checks of the page scripts (group pagescript of --selftest): the host of
// devwin/pagescript.rs on a document without a window: finding, reading and changing elements,
// events and what stops them, timers on a given clock, fetch and src through an engine's HTTP client
// (fixtures/web_scripts.jfnet), location, fuel and limits, errors with file, line and column, the
// order of scripts, what a page cannot reach, the same result twice

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::project::Project;
use jfn_sim::{Action, Engine, Report, WebReq, WebResp, MS, SEC};
use netpkt::http::Headers;
use ui::{MdEv, MdNode, MdViewState, MF_CHECKBOX};

use crate::selftest::T;
use crate::workspace::devwin::pagescript::{ConKind, PageScript, CALL_FUEL, PAGE_NODES};

const SITE: &str = include_str!("../tests/fixtures/web_scripts.jfnet");
const URL: &str = "http://web.lab/seite.html";
const PC1: u32 = 1;

// a page without a window: its view, its scripts, the clock, the numbers of its requests
struct Page {
    view: MdViewState,
    p: alloc::boxed::Box<PageScript>,
    now: u64,
    seq: u64,
    sent: Vec<WebReq>,
    nav: Option<String>,
    reload: bool,
}

fn page_at(url: &str, html: &str, seed: u64) -> Option<Page> {
    let mut view = MdViewState::new();
    view.set_html(html);
    let p = PageScript::of(&view, html, url, seed)?;
    Some(Page { view, p, now: SEC, seq: 0, sent: Vec::new(), nav: None, reload: false })
}

fn page(html: &str) -> Page {
    let mut x = page_at(URL, html, 1).expect("a page with scripts");
    x.pump();
    x
}

fn script(src: &str) -> Page {
    page(&format!("<body><p id=\"a\" class=\"x y\">Text <b id=\"b\">fett</b></p><ul id=\"l\"><li>eins</li><li class=\"z\">zwei</li></ul><script type=\"text/jfs\">{}</script></body>", src))
}

impl Page {
    fn pump(&mut self) {
        self.p.pump(&mut self.view, self.now, &mut self.seq);
        self.take();
    }

    fn take(&mut self) {
        let fx = self.p.take_effects();
        self.sent.extend(fx.fetch);
        if fx.nav.is_some() {
            self.nav = fx.nav;
        }
        self.reload |= fx.reload;
    }

    fn ev(&mut self, e: MdEv) -> (Option<String>, Option<ui::MdSubmit>) {
        let r = self.p.events(&mut self.view, e, self.now, &mut self.seq);
        self.take();
        r
    }

    fn click(&mut self, id: &str) -> Option<String> {
        let n = self.view.doc().by_id(id).unwrap_or_default();
        let href = self.view.doc().attr(n, "href").map(String::from);
        self.ev(MdEv { click: Some(n), link: href, ..MdEv::default() }).0
    }

    fn text(&mut self, id: &str) -> String {
        self.view.update();
        let d = self.view.doc();
        d.by_id(id).map_or(String::from("(fehlt)"), |n| d.text_of(n))
    }

    fn console(&self) -> String {
        self.p.console().text()
    }

    fn at(&mut self, ms: u64) {
        self.now += ms * MS;
        self.pump();
    }
}

// a console holds this text (else it shows what it holds)
fn has(t: &mut T, name: &str, console: &str, what: &str) {
    if !console.contains(what) {
        t.eq(name, console, what);
    } else {
        t.check(name, true);
    }
}

// ---------------------------------------------------------------- the document

fn dom(t: &mut T) {
    let mut x = script(r##"
        let a = document.by_id("a")
        print(a.tag, a.kind, a.text(), a.html())
        print(document.by_id("nicht") == nil, document.find("#b").text(), document.find("li.z").text(), len(document.find_all("li")))
        print(a.find("b").tag, len(a.find_all("*")), a.find("i") == nil, a.matches("p.x"), a.matches("div"))
        let l = document.by_id("l")
        let k = l.children()
        print(len(k), k[0].next().text(), k[1].prev().text(), k[1].next() == nil, k[0].parent().attr("id"), document.find("body").parent() == nil)
        print(a.attr("id"), a.attr("nicht") == nil, a.has_class("x"), a.has_class("q"))
        a.add_class("neu")
        a.remove_class("x")
        print(a.attr("class"), a.toggle_class("t"), a.toggle_class("t"))
        a.set_attr("title", 5)
        print(a.attr("title"))
        a.remove_attr("title")
        print(a.attr("title") == nil)
        document.by_id("b").set_text(42)
        print(a.text())
        let li = document.create("li")
        li.set_text("drei")
        l.append(li)
        let vorn = document.create("li")
        vorn.set_html("<i>null</i>")
        l.insert(vorn, k[0])
        print(l.html())
        print(document.find_all("li")[3].text(), li == document.find_all("li")[3], li.same(document.find_all("li")[3]))
        k[1].remove()
        print(len(l.children()), k[1].text())
        l.append(k[1])
        print(l.children()[3].text())
        let t = document.create_text(" und mehr")
        document.by_id("a").append(t)
        print(t.kind, t.text(), a.text())
        print(document.title(), document.body().tag)
        document.set_title("Neu")
        print(document.title())
        l.set_text("leer")
        print(l.text(), len(l.children()))
    "##);
    let c = x.console();
    let want = [
        "p element Text fett Text <b id=\"b\">fett</b>",
        "true fett zwei 2",
        "b 1 true true false",
        "2 zwei eins true l true",
        "a true true false",
        "y neu true false",
        "5",
        "true",
        "Text 42",
        "<li><i>null</i></li><li>eins</li><li class=\"z\">zwei</li><li>drei</li>",
        "drei true true",
        "3 zwei",
        "zwei",
        "text  und mehr Text 42 und mehr",
        " body",
        "Neu",
        "leer 0",
    ];
    t.eq("the document's API", c.as_str(), want.join("\n").as_str());
    t.eq("no errors", x.p.console().errors(), 0);
    t.eq("the view has the changes", x.text("a").as_str(), "Text 42 und mehr");
    // an element whose node went answers with nothing, a method no element has is an error
    let x = script(r##"
        let b = document.by_id("b")
        document.by_id("a").set_text("weg")
        print(b.text() == "", b.attr("id") == nil, b.parent() == nil, b.children() == nil)
        let m = {kind: "element", node: 12345}
        print(m.text() == "")
        print({a: 1}.text())
    "##);
    let c = x.console();
    t.check("a node that is gone answers with nothing", c.starts_with("true true true false\ntrue\n"));
    t.check("another map has no method", c.contains("Die Map hat keine Funktion text"));
    // a wrong use throws, a script can catch it
    let x = script(r##"
        try {
            document.by_id("l").append(document.by_id("l"))
        } catch e {
            print("gefangen: ${e}")
        }
        try { document.by_id(1) } catch e { print(e) }
        try { document.by_id("a").on("wackeln", fn(ev) { }) } catch e { print(e) }
        try { timer.after(-1, fn() { }) } catch e { print(e) }
    "##);
    let c = x.console();
    t.check("an element into itself", c.contains("gefangen: Das Element lässt sich hier nicht einfügen."));
    t.check("an id must be a text", c.contains("document.by_id: hier muss ein Text stehen"));
    t.check("the events", c.contains("on: das Ereignis muss click, input oder submit sein"));
    t.check("a time", c.contains("timer.after: die Zeit muss eine Zahl ab 0 sein"));
}

// ---------------------------------------------------------------- events

fn events(t: &mut T) {
    let html = r##"<body><h1 id="h">Kopf</h1><div id="d"><p id="p">Text <span id="s">Teil</span></p></div>
        <p><a id="stop" href="weg.html">Bleibt</a> <a id="geht" href="weg.html">Geht</a></p>
        <form id="f" action="/senden" method="post"><input id="n" name="n" value="1"><input type="checkbox" id="c" name="c" value="ja"><input type="submit" id="los" value="Los"></form>
        <button id="b">Knopf</button><select id="w" name="w"><option value="a">A</option><option value="b" selected>B</option></select>
        <script type="text/jfs">
        fn spur(x) { print(x) }
        document.by_id("s").on("click", fn(ev) { spur("s ${ev.type} ${ev.target.attr("id")} ${ev.current.attr("id")}") })
        document.by_id("p").on("click", fn(ev) { spur("p ${ev.target.attr("id")} ${ev.current.attr("id")}") })
        document.by_id("d").on("click", fn(ev) { spur("d") })
        document.on("click", fn(ev) { spur("dok ${ev.current.kind}") })
        document.by_id("stop").on("click", fn(ev) { ev.prevent() })
        document.by_id("b").on("click", fn(ev) { ev.stop()
          spur("b") })
        document.by_id("n").on("input", fn(ev) { spur("input ${ev.target.value()}") })
        document.by_id("c").on("input", fn(ev) { spur("box ${ev.target.checked()} ${ev.target.value()}") })
        document.by_id("w").on("input", fn(ev) { spur("wahl ${ev.target.value()}") })
        let aus = fn(ev) { spur("aus") }
        document.by_id("h").on("click", aus)
        document.by_id("h").on("click", fn(ev) { spur("h2") })
        document.by_id("h").off("click", aus)
        </script></body>"##;
    let mut x = page(html);
    // the way up: the target, its parents, the document
    x.click("s");
    t.eq("up from an inline element", x.console().as_str(), "s click s s\np s p\nd\ndok document");
    x.p.clear_console();
    x.click("p");
    t.eq("from the paragraph", x.console().as_str(), "p p p\nd\ndok document");
    x.p.clear_console();
    // stop ends the way up, the other handlers there still ran
    x.click("b");
    t.eq("stop", x.console().as_str(), "b");
    x.p.clear_console();
    x.click("h");
    t.eq("off takes one handler, the others stay", x.console().as_str(), "h2\ndok document");
    x.p.clear_console();
    // a link's default action
    t.eq("prevent stops a link", x.click("stop"), None);
    t.eq("a link without a handler goes", x.click("geht"), Some(String::from("weg.html")));
    // fields
    let find = |x: &Page, id: &str| {
        let d = x.view.doc();
        d.by_id(id).and_then(|n| d.field_of(n)).unwrap_or(0)
    };
    let i = find(&x, "n");
    x.p.clear_console();
    x.view.set_field_text(i, "zwei");
    x.ev(MdEv { field: Some(i), ..MdEv::default() });
    t.eq("input of a text field", x.console().as_str(), "input zwei");
    x.p.clear_console();
    let i = find(&x, "c");
    t.eq("a check box", x.view.doc().fields()[i].kind, MF_CHECKBOX);
    x.view.set_field_checked(i, true);
    x.ev(MdEv { field: Some(i), ..MdEv::default() });
    t.eq("click and input of a check box", x.console().as_str(), "dok document\nbox true ja");
    x.p.clear_console();
    let i = find(&x, "w");
    x.view.set_field_selected(i, 0);
    x.ev(MdEv { field: Some(i), ..MdEv::default() });
    t.eq("input of a list", x.console().as_str(), "wahl a");

    // submit: stopped, allowed, with what the handler changed
    let html = r##"<form id="f" action="/senden" method="post"><input id="n" name="n" value=""><input type="submit" id="los" value="Los"></form>
        <script type="text/jfs">
        document.by_id("los").on("click", fn(ev) { print("knopf") })
        document.by_id("f").on("submit", fn(ev) {
            print(ev.type, ev.target.tag, ev.data.n)
            if ev.data.n == "" {
                ev.prevent()
                return
            }
            document.by_id("n").set_value("${ev.data.n}!")
            document.by_id("f").set_attr("action", "/anders")
        })
        </script>"##;
    let mut x = page(html);
    let sub = |x: &mut Page| {
        let d = x.view.doc();
        let b = d.by_id("los").and_then(|n| d.field_of(n));
        let s = ui::MdSubmit { form: 0, action: d.forms()[0].action.clone(), method: d.forms()[0].method.clone(), fields: x.view.form_data(0, b), button: b };
        x.ev(MdEv { submit: Some(s), ..MdEv::default() }).1
    };
    t.check("a stopped form does not go", sub(&mut x).is_none());
    t.eq("its button's click comes first", x.console().as_str(), "knopf\nsubmit form ");
    x.p.clear_console();
    let i = x.view.doc().by_id("n").and_then(|n| x.view.doc().field_of(n)).unwrap_or(0);
    x.view.set_field_text(i, "anna");
    let s = sub(&mut x);
    t.eq("it goes with the handler's value", s.as_ref().map(|s| s.fields[0].clone()), Some((String::from("n"), String::from("anna!"))));
    t.eq("and its action", s.as_ref().map(|s| s.action.clone()), Some(String::from("/anders")));
    // a later ev.prevent() does nothing
    let mut x = script(r##"
        let alt = nil
        document.by_id("a").on("click", fn(ev) { alt = ev })
        timer.after(1, fn() { alt.prevent()
          print("spät") })
    "##);
    t.eq("the link goes", x.ev(MdEv { click: x.view.doc().by_id("a"), link: Some(String::from("x")), ..MdEv::default() }).0, Some(String::from("x")));
    x.at(5);
    t.eq("prevent after the event: no error", x.console().as_str(), "spät");
    // the same element is the same map
    let mut x = script(r##"
        let a = document.by_id("a")
        a.on("click", fn(ev) { print(ev.target == a, ev.current == a, ev.target.same(a)) })
        document.on("click", fn(ev) { print(ev.current == document, ev.target == a) })
    "##);
    x.click("a");
    t.eq("event and element", x.console().as_str(), "true true true\ntrue true");
    // no handlers: no cost, everything goes on
    let mut x = script("print(\"ohne\")");
    t.eq("a page without handlers", x.ev(MdEv { click: Some(MdNode::NONE), link: Some(String::from("x")), ..MdEv::default() }).0, Some(String::from("x")));
}

// ---------------------------------------------------------------- time and chance

fn clock(t: &mut T) {
    let mut x = script(r##"
        let n = 0
        let e = timer.every(100, fn() { n += 1
            print("takt ${n} ${now()}") })
        timer.after(250, fn() { print("nach 250") })
        let weg = timer.after(50, fn() { print("nie") })
        print(timer.cancel(weg), timer.cancel(weg), timer.cancel("x"))
        timer.after(450, fn() { timer.cancel(e)
            print("aus") })
    "##);
    t.eq("nothing before its time", x.console().as_str(), "true false false");
    t.eq("three timers wait", x.p.timers(), 3);
    x.p.clear_console();
    x.at(99);
    t.eq("99 ms", x.console().as_str(), "");
    x.at(1);
    t.eq("100 ms", x.console().as_str(), "takt 1 1100.0");
    x.p.clear_console();
    // the clock stands: nothing happens however often it is asked
    x.pump();
    x.pump();
    t.eq("the clock stands", x.console().as_str(), "");
    x.at(150);
    t.eq("one late round, by time", x.console().as_str(), "takt 2 1250.0\nnach 250");
    x.p.clear_console();
    // a long pause (the page was not shown): a timer.every once, its next round from now
    x.at(1000);
    t.eq("rounds missed go", x.console().as_str(), "takt 3 2250.0\naus");
    t.eq("all gone", x.p.timers(), 0);
    // the wall clock's wait for the next timer
    let mut x = script("timer.after(1000, fn() { })\ntimer.after(2000, fn() { })");
    t.eq("at real speed", x.p.wake(x.now, 1000), Some(1000));
    t.eq("at double speed", x.p.wake(x.now, 2000), Some(500));
    t.eq("while the clock stands: no frame", x.p.wake(x.now, 0), None);
    x.at(1000);
    t.eq("the next one", x.p.wake(x.now, 1000), Some(1000));
    // a timer of 0 from a timer waits for the next pass (a chain of them cannot hold a frame)
    let mut x = script("let n = 0\nfn weiter() {\n  n += 1\n  print(n)\n  timer.after(0, weiter)\n}\ntimer.after(0, weiter)");
    t.eq("a timer of 0 runs in the pass", x.console().as_str(), "1");
    t.eq("the next one wants a frame", x.p.wake(x.now, 0), Some(1));
    x.pump();
    t.eq("and runs in it", x.console().as_str(), "1\n2");
    // a past state became the present: what is left stays
    let mut x = script("timer.after(1000, fn() { print(\"da\") })");
    x.now -= 500 * MS;
    x.pump();
    x.at(999);
    t.eq("the clock went back", x.console().as_str(), "");
    x.at(1);
    t.eq("what was left", x.console().as_str(), "da");
    // an every of 0 is at least 1 ms
    let mut x = script("let n = 0\ntimer.every(0, fn() { n += 1\n print(n) })");
    x.at(1);
    x.at(1);
    t.eq("every: 1 ms at least", x.console().as_str(), "1\n2");
    // chance: the seed decides, the same twice
    let src = "<script type=\"text/jfs\">print(rand(1000000), rand(5, 9), rand() < 1)</script>";
    let a = page_at(URL, src, 7).map(|mut p| {
        p.pump();
        p.console()
    });
    let b = page_at(URL, src, 7).map(|mut p| {
        p.pump();
        p.console()
    });
    let c = page_at(URL, src, 8).map(|mut p| {
        p.pump();
        p.console()
    });
    let d = page_at("http://web.lab/andere.html", src, 7).map(|mut p| {
        p.pump();
        p.console()
    });
    t.eq("chance: the same seed", a.clone(), b);
    t.check("another seed", a != c);
    t.check("another page", a != d);
    t.check("the host's own rand", a.as_deref().is_some_and(|s| s.ends_with(" true")));
    let x = script("try { rand(0) } catch e { print(\"rand: ${len(e) > 0}\") }");
    t.eq("a wrong rand throws", x.console().as_str(), "rand: true");
}

// ---------------------------------------------------------------- limits

fn limits(t: &mut T) {
    // an endless loop ends with an error at its place; the next script runs, handlers too
    let mut x = page("<p id=\"a\">x</p><script type=\"text/jfs\">\nwhile true {\n}\n</script><script type=\"text/jfs\">\nlet n = 0\ndocument.by_id(\"a\").on(\"click\", fn(ev) {\n  n += 1\n  if n == 1 {\n    while true { }\n  }\n  print(\"klick ${n}\")\n})\nprint(\"zweites\")\n</script>");
    let c = x.console();
    has(t, "the loop's place", &c, &format!("{}:3:1:", URL));
    has(t, "the loop's error", &c, &format!("mehr als {} Schritte", CALL_FUEL));
    t.check("the next script ran", c.ends_with("zweites"));
    t.eq("one error", x.p.console().errors(), 1);
    x.p.clear_console();
    x.click("a");
    t.check("a handler's loop: its line", x.console().contains(":9:") && x.p.console().errors() == 1);
    x.p.clear_console();
    x.click("a");
    t.eq("the page goes on", x.console().as_str(), "klick 2");
    // a timer that loops: each round its own error, the rest of the page goes on, a frame's fuel
    // ends the frame
    let mut x = script("let n = 0\ntimer.every(1, fn() { while true { } })\ntimer.every(1, fn() { while true { } })\ntimer.every(1, fn() { while true { } })\ntimer.every(1, fn() { n += 1 })\ndocument.by_id(\"a\").on(\"click\", fn(ev) { print(\"noch da\") })");
    x.at(1);
    t.eq("a frame runs at most its fuel", x.p.console().errors(), 2);
    x.p.clear_console();
    x.click("a");
    t.eq("a click still gets through", x.console().as_str(), "noch da");
    // endless recursion, a heap that grows, output that grows
    let x = script("fn f(n) { return f(n + 1) + 1 }\nf(0)");
    t.check("depth", x.console().contains("Zu viele Aufrufe ineinander") && x.console().contains(":"));
    let x = script("let l = []\nlet s = \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"\nwhile true {\n  l.push(\"${s}${s}${s}${s}${len(l)}\")\n}");
    has(t, "heap", &x.console(), "Speichergrenze erreicht");
    has(t, "heap: the place", &x.console(), ":4:");
    let x = script("let n = 0\nwhile n < 100000 {\n  print(\"Zeile ${n}\")\n  n += 1\n}");
    t.eq("no error for much output", x.p.console().errors(), 0);
    t.check("the console keeps its last lines", x.p.console().lines().count() <= 1000);
    // the document: nodes, depth, a selector's cost in a loop
    let x = script("let b = document.body()\ntry {\n  while true {\n    b.append(document.create(\"p\"))\n  }\n} catch e {\n  print(\"Grenze: ${e}\")\n}");
    t.check("fuel or the cap ends a growing document", x.p.console().errors() == 1 || x.console().contains("Grenze:"));
    let mut x = script("fn wachse() {\n  let b = document.body()\n  let i = 0\n  while i < 30000 {\n    b.append(document.create(\"p\"))\n    i += 1\n  }\n}\ndocument.by_id(\"a\").on(\"click\", fn(ev) {\n  try {\n    wachse()\n  } catch e {\n    print(\"voll\")\n  }\n})");
    for _ in 0..24 {
        x.click("a");
        x.at(1);
    }
    has(t, "the cap of nodes holds", &x.console(), "voll");
    t.check("how large it got", x.view.doc().dom_size().0 <= PAGE_NODES + 40_000);
    let x = script("let n = document.by_id(\"a\")\nlet i = 0\nwhile i < 3000 {\n  let m = document.create(\"div\")\n  n.append(m)\n  n = m\n  i += 1\n}");
    t.check("the depth of the document", x.console().contains("zu tief verschachtelt"));
    let x = script("let i = 0\nwhile i < 5000 {\n  timer.after(100000, fn() { })\n  i += 1\n}");
    t.check("too many timers", x.console().contains("Zu viele Zeitgeber"));
    let x = script("let i = 0\nlet a = document.by_id(\"a\")\nwhile i < 5000 {\n  a.on(\"click\", fn(ev) { })\n  i += 1\n}");
    t.check("too many handlers", x.console().contains("Zu viele Ereignis-Funktionen"));
    let x = script("let i = 0\nwhile i < 300 {\n  fetch(\"/x\", fn(e, r) { })\n  i += 1\n}");
    t.check("too many requests", x.console().contains("Zu viele offene Anfragen"));
    // nothing of the device or the machine
    let x = script("print(import(\"/etc/hosts\"))");
    t.check("no import", x.console().contains("import geht hier nicht") && x.console().contains(":1:"));
    for name in ["http", "net", "disk", "dns", "metrics", "config"] {
        let x = script(&format!("print({})", name));
        t.check(&format!("no {}", name), x.console().contains(&format!("Unbekannter Name {}", name)));
    }
}

// ---------------------------------------------------------------- errors and order

fn order(t: &mut T) {
    // positions: lines and columns of the page; the scripts' own names
    let html = "<html><body>\n<p>x</p>\n<script type=\"text/jfs\">\n  let a = 1\n  let b = = 2\n</script>\n  <script type=\"text/jfs\">print(1 / 0)</script>\n<script type=\"text/jfs\">\nfn f() {\n  throw \"weg\"\n}\nf()\n</script>\n<script type=\"text/jfs\">\nunbekannt()\n</script></body></html>\n";
    let x = page(html);
    let c = x.console();
    has(t, "a syntax error: page and place", &c, &format!("{}:5:11:", URL));
    has(t, "a runtime error on a line with others", &c, &format!("{}:7:35:", URL));
    has(t, "a throw", &c, &format!("{}:10:3: weg", URL));
    t.check("its trace", c.contains(&format!("  f ({}:12:1)", URL)) || c.contains(&format!("({}:12:1)", URL)));
    t.check("an unknown name", c.contains(&format!("{}:15:1: Unbekannter Name unbekannt", URL)));
    t.eq("every error kind counts", x.p.console().errors() >= 4, true);
    // a page with CRLF lines
    let x = page("<p>x</p>\r\n<p>y</p>\r\n<script type=\"text/jfs\">\r\nlet a = 1\r\n  print(1 / 0)\r\n</script>\r\n");
    has(t, "lines of a page with CRLF", &x.console(), &format!("{}:5:11:", URL));
    // the order: inline, src waits, the rest after it; names shared
    let html = "<script type=\"text/jfs\">let a = 1\nprint(\"eins\", a)</script><script type=\"text/jfs\" src=\"lib/b.jfs\"></script><script type=\"text/jfs\">print(\"drei\", b(a))</script><script type=\"text/jfs\" src=\"weg.jfs\"></script><script type=\"text/jfs\">print(\"vier\")</script><script type=\"application/x\">print(\"nie\")</script><script>print(\"nie\")</script><script src=\"nie.jfs\"></script>";
    let mut x = page(html);
    t.eq("up to the src script", x.console().as_str(), "eins 1");
    t.eq("it is fetched, resolved against the page", x.sent.len(), 1);
    t.eq("its address", x.sent[0].url.as_str(), "http://web.lab/lib/b.jfs");
    t.check("the page waits for it", x.p.running());
    x.pump();
    t.eq("still waiting", x.console().as_str(), "eins 1");
    let id = x.sent[0].id;
    x.p.answer(id, Ok(WebResp { url: x.sent[0].url.clone(), status: 200, reason: String::from("OK"), headers: Headers::new(), body: b"fn b(x) { return x + 1 }\nprint(\"zwei\")\nprint(1 / 0)".to_vec(), redirects: 0, took: 0, ..WebResp::default() }));
    x.pump();
    let c = x.console();
    t.check("the src script ran, its error names it", c.contains("zwei\nhttp://web.lab/lib/b.jfs:3:"));
    t.check("the next one sees its names", c.ends_with("drei 2"));
    t.eq("the next src script is fetched", x.sent.len(), 2);
    let id = x.sent[1].id;
    x.p.answer(id, Err(String::from("Nichts da.")));
    x.pump();
    let c = x.console();
    t.check("a failed fetch is a console error", c.contains("http://web.lab/weg.jfs: das Skript kam nicht: Nichts da."));
    t.check("and the next one runs", c.ends_with("vier"));
    t.check("the page is through", !x.p.running());
    t.check("other types did not run", !c.contains("nie"));
    // 404, too large
    let mut x = page("<script type=\"text/jfs\" src=\"a.jfs\"></script><script type=\"text/jfs\" src=\"b.jfs\"></script>");
    let id = x.sent[0].id;
    x.p.answer(id, Ok(WebResp { url: String::new(), status: 404, reason: String::from("Not Found"), headers: Headers::new(), body: b"print(1)".to_vec(), redirects: 0, took: 0, ..WebResp::default() }));
    x.pump();
    t.check("an error status", x.console().contains("http://web.lab/a.jfs: das Skript kam nicht: 404 Not Found"));
    let id = x.sent[1].id;
    x.p.answer(id, Ok(WebResp { url: String::new(), status: 200, reason: String::from("OK"), headers: Headers::new(), body: alloc::vec![b' '; (1 << 20) + 1], redirects: 0, took: 0, ..WebResp::default() }));
    x.pump();
    t.check("too large", x.console().contains("das Skript ist zu groß"));
    // no scripts: no VM; scripts that come later do not run
    t.check("a page without scripts has none", page_at(URL, "<p>x</p><script>print(1)</script>", 1).is_none());
    t.check("empty ones do not count", page_at(URL, "<script type=\"text/jfs\">  \n </script>", 1).is_none());
    t.check("the type's case and blanks", page_at(URL, "<script type=\" TEXT/JFS \">print(1)</script>", 1).is_some());
    let x = script("document.by_id(\"a\").set_html(\"<script type='text/jfs'>print(99)</\" + \"script>\")\nlet s = document.create(\"script\")\ns.set_attr(\"type\", \"text/jfs\")\ns.set_text(\"print(98)\")\ndocument.body().append(s)");
    t.eq("scripts that come later do not run", x.console().as_str(), "");
}

// ---------------------------------------------------------------- location

fn location(t: &mut T) {
    let mut x = page_at("http://web.lab:8080/a/b.html?x=1&y=2#teil", "<a id=\"a\">x</a><script type=\"text/jfs\">print(location.href)\nprint(location.scheme, location.host, location.hostname, location.port, location.path, location.query, location.fragment)\nprint(type(location.assign))\ndocument.by_id(\"a\").on(\"click\", fn(ev) { location.assign(\"c.html\") })</script>", 1).expect("a page");
    x.pump();
    let c = x.console();
    t.check("the address", c.starts_with("http://web.lab:8080/a/b.html?x=1&y=2#teil\n"));
    t.check("its parts", c.contains("http web.lab:8080 web.lab 8080 /a/b.html x=1&y=2 teil"));
    x.click("a");
    t.eq("assign: after the callback", x.nav.take(), Some(String::from("c.html")));
    let mut x = page_at("http://web.lab/", "<script type=\"text/jfs\">print(location.port == nil, location.path, location.query == \"\", location.fragment == \"\")\nlet n = 0\ntimer.after(1, fn() { location.href = \"neu.html\" })\ntimer.after(2, fn() { location.reload() })\ntimer.after(3, fn() { print(location.href) })</script>", 1).expect("a page");
    x.pump();
    t.eq("no port", x.console().as_str(), "true / true true");
    x.at(1);
    t.eq("href = navigates", x.nav.take(), Some(String::from("neu.html")));
    x.at(1);
    t.check("reload", x.reload);
    x.at(1);
    t.check("the address stays what the page is", x.console().ends_with("http://web.lab/"));
    // a fragment link is an entry of its own over the same document
    x.p.set_location("http://web.lab/#unten");
    let mut y = script("timer.after(1, fn() { print(location.href, location.fragment) })");
    y.p.set_location("http://web.lab/seite.html#unten");
    y.at(1);
    t.eq("the entry shown", y.console().as_str(), "http://web.lab/seite.html#unten unten");
    // relative addresses of fetch resolve against the page shown
    let x = page_at("http://web.lab/a/b.html", "<script type=\"text/jfs\">fetch(\"c.json\", fn(e, r) { })\nfetch({url: \"/d\", method: \"post\", body: \"x=1\", headers: {\"Content-Type\": \"text/plain\"}}, fn(e, r) { })</script>", 1).map(|mut p| {
        p.pump();
        p.sent
    });
    let x = x.unwrap_or_default();
    t.eq("fetch: two requests", x.len(), 2);
    t.eq("a relative address", x[0].url.as_str(), "http://web.lab/a/c.json");
    t.eq("options: address, method, body", (x[1].url.as_str(), x[1].method.as_str(), x[1].body.as_slice()), ("http://web.lab/d", "POST", b"x=1".as_slice()));
    t.eq("options: headers", x[1].headers.get("content-type"), Some("text/plain"));
    t.check("an id of its own", x[0].id != x[1].id);
    let x = script("try { fetch(5, fn(e, r) { }) } catch e { print(e) }\ntry { fetch(\"/a\", 5) } catch e { print(e) }");
    t.eq("fetch needs both", x.console().as_str(), "fetch: die URL fehlt\nfetch: hier muss eine Funktion stehen");
    // the answers: in the order they came, to their callbacks; err as text
    let mut x = script("fetch(\"/a\", fn(e, r) { print(\"a\", e, r.status, r.body, r.headers.x, r.reason, r.url, r.redirects) })\nfetch(\"/b\", fn(e, r) { print(\"b\", e, r) })");
    let mut h = Headers::new();
    h.add("X", "y");
    let (a, b) = (x.sent[0].id, x.sent[1].id);
    x.p.answer(b, Err(String::from("kein Weg")));
    x.p.answer(a, Ok(WebResp { url: String::from("http://web.lab/a"), status: 404, reason: String::from("Not Found"), headers: h, body: b"leer".to_vec(), redirects: 1, took: 0, ..WebResp::default() }));
    t.eq("an answer wants a frame", x.p.wake(x.now, 0), Some(1));
    t.eq("an answer waits for a pass", x.console().as_str(), "");
    x.pump();
    t.eq("two answers", x.console().as_str(), "b kein Weg nil\na nil 404 leer y Not Found http://web.lab/a 1");
    x.p.answer(a, Err(String::new()));
    x.pump();
    t.eq("an unknown request is ignored", x.p.console().lines().count(), 2);
    t.eq("no console kinds but text", x.p.console().lines().all(|l| l.kind == ConKind::Text), true);
}

// ---------------------------------------------------------------- the real thing: the fixture

fn engine() -> Engine {
    let p = Project::from_text(SITE).unwrap_or_else(|_| Project::new("web", 1));
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let t = e.now() + 2 * SEC;
    e.run_until(t, 8_000_000);
    e
}

// the requests a page wants through the engine, the answers back to it
fn serve(e: &mut Engine, x: &mut Page) {
    for req in core::mem::take(&mut x.sent) {
        let _ = e.apply(Action::WebFetch { dev: PC1, req });
    }
    let t = e.now() + 500 * MS;
    e.run_until(t, 8_000_000);
    x.now = e.now();
    for r in e.take_reports() {
        if let Report::Web { id, result, .. } = r {
            x.p.answer(id, result.map_err(|f| String::from(f.msg.text())));
        }
    }
    x.pump();
}

// a page of the fixture
fn open(e: &mut Engine, path: &str) -> Option<Page> {
    let url = format!("http://web.lab/{}", path);
    let mut h = Headers::new();
    h.add("Accept", "text/html");
    let _ = e.take_reports();
    let _ = e.apply(Action::WebFetch { dev: PC1, req: WebReq { id: 1, method: String::from("GET"), url: url.clone(), headers: h, body: Vec::new(), ..WebReq::default() } });
    let t = e.now() + 500 * MS;
    e.run_until(t, 8_000_000);
    let mut body = None;
    for r in e.take_reports() {
        if let Report::Web { id: 1, result: Ok(a), .. } = r {
            body = Some(String::from_utf8_lossy(&a.body).into_owned());
        }
    }
    let mut x = page_at(&url, &body?, e.seed())?;
    x.now = e.now();
    x.seq = 100;
    x.pump();
    Some(x)
}

fn fixture(t: &mut T) {
    let mut e = engine();
    // a src script and an inline one share names
    let Some(mut x) = open(&mut e, "zaehler.html") else { return t.check("the counter", false) };
    serve(&mut e, &mut x);
    t.eq("the counter is ready", x.console().as_str(), "bereit");
    x.click("plus");
    x.click("plus");
    t.eq("click, click", x.text("stand").as_str(), "2");
    x.click("null");
    t.eq("back to zero", x.text("stand").as_str(), "0");
    t.check("log.warn", x.p.console().lines().any(|l| l.kind == ConKind::Warn && l.text == "zurück auf null"));
    // fetch of JSON from a script with http.serve
    let Some(mut x) = open(&mut e, "daten.html") else { return t.check("the data page", false) };
    x.click("holen");
    t.eq("while it comes", x.text("stand").as_str(), "Kommt …");
    serve(&mut e, &mut x);
    t.eq("the data", x.text("stand").as_str(), "Status 200: 3 Waren");
    t.eq("the list", x.text("waren").as_str(), "RoggenbrotWeizenbrotBrezel");
    x.click("falsch");
    serve(&mut e, &mut x);
    let t2 = e.now() + 40 * SEC;
    e.run_until(t2, 8_000_000);
    serve(&mut e, &mut x);
    t.eq("a request with no answer", x.text("stand").as_str(), "Fehler beim Holen: true");
    // a broken page goes on after each error
    let Some(mut x) = open(&mut e, "kaputt.html") else { return t.check("the broken page", false) };
    serve(&mut e, &mut x);
    let c = x.console();
    t.check("a compile error with its place", c.contains("http://web.lab/kaputt.html:10:"));
    has(t, "a runtime error with its place", &c, "http://web.lab/kaputt.html:14:9:");
    t.check("src that failed", c.contains("http://web.lab/gibtsnicht.jfs: das Skript kam nicht: 404"));
    t.check("no import", c.contains("import geht hier nicht"));
    t.check("no http", c.contains("Unbekannter Name http"));
    t.eq("the rest ran", (x.text("eins").as_str().to_string(), x.text("zwei").as_str().to_string(), x.text("drei").as_str().to_string(), x.text("vier").as_str().to_string()), (String::from("Skript eins lief."), String::from("Skript zwei lief."), String::from("Skript drei lief."), String::from("Skript vier lief.")));
    t.eq("other types do not run", x.text("fremd").as_str(), "Skripte anderer Art laufen nicht.");
    // determinism: the same page in a new engine, twice
    let run = |path: &str| -> String {
        let mut e = engine();
        let Some(mut x) = open(&mut e, path) else { return String::from("keine Seite") };
        serve(&mut e, &mut x);
        x.click("kopf");
        x.click("teil");
        let t = e.now() + 700 * MS;
        e.run_until(t, 8_000_000);
        x.now = e.now();
        x.pump();
        format!("{}|{}|{}", x.console(), x.text("spur"), x.text("zufall"))
    };
    let a = run("klicks.html");
    t.eq("the same result twice", a.clone(), run("klicks.html"));
    t.check("the page ran", a.contains("Spur: dok teil absatz:teil dok"));
    // a clock on the simulation's time, the same twice
    let run = || -> String {
        let mut e = engine();
        let Some(mut x) = open(&mut e, "uhr.html") else { return String::from("keine Seite") };
        for _ in 0..5 {
            let t = e.now() + 500 * MS;
            e.run_until(t, 8_000_000);
            x.now = e.now();
            x.pump();
        }
        format!("{}|{}", x.text("takte"), x.text("spaeter"))
    };
    let a = run();
    t.eq("the clock", a.as_str(), "Takte: 5|Zwei Sekunden sind um.");
    t.eq("the clock twice", a, run());
}

pub(crate) fn run(t: &mut T) {
    dom(t);
    events(t);
    clock(t);
    limits(t);
    order(t);
    location(t);
    fixture(t);
}

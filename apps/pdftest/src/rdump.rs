// rdump -- `pdftest --read FILE [--password PW]`: what lib/pdf's reader (pdf::read) finds in a
// file, as one JSON object on stdout, for tools/pdf_read_check.py to compare with Poppler:
// version, encryption, pages with sizes, rotation and labels, the information, outlines, links,
// annotations, form fields, attachments and the text of every page.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use pdf::read::{Action, Document, Error, FieldKind, Options, Outline};

fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn opt(s: &Option<String>) -> String {
    s.as_deref().map(js).unwrap_or_else(|| "null".into())
}

fn date(d: &Option<pdf::Date>) -> String {
    match d {
        Some(d) => js(&format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
            d.year,
            d.month,
            d.day,
            d.hour,
            d.minute,
            d.second,
            if d.offset_minutes < 0 { '-' } else { '+' },
            d.offset_minutes.unsigned_abs() / 60,
            d.offset_minutes.unsigned_abs() % 60
        )),
        None => "null".into(),
    }
}

fn num(v: f64) -> String {
    if v.is_finite() { format!("{:.2}", v) } else { "null".into() }
}

fn action(a: &Option<Action>) -> String {
    match a {
        None => "null".into(),
        Some(Action::GoTo(d)) => format!("{{\"page\":{}}}", d.page),
        Some(Action::Uri(u)) => format!("{{\"uri\":{}}}", js(u)),
        Some(Action::GoToFile { file, page }) => format!("{{\"file\":{},\"page\":{}}}", js(file), page.map(|p| format!("{p}")).unwrap_or("null".into())),
        Some(Action::Launch(f)) => format!("{{\"launch\":{}}}", js(f)),
        Some(Action::Named(n)) => format!("{{\"named\":{}}}", js(n)),
        Some(Action::Other(s)) => format!("{{\"other\":{}}}", js(s)),
    }
}

fn outlines(list: &[Outline], level: u32, out: &mut Vec<String>) {
    for o in list {
        out.push(format!("{{\"title\":{},\"level\":{},\"open\":{},\"action\":{}}}", js(&o.title), level, o.open, action(&o.action)));
        outlines(&o.children, level + 1, out);
    }
}

/// `--glyphs PAGE`: every glyph of a page with its position, size and font
fn glyphs(doc: &Document, page: usize) -> i32 {
    match doc.page_text(page) {
        Ok(t) => {
            for it in &t.items {
                plat::write_stdout(
                    format!("{:8.2} {:8.2} +{:7.3},{:7.3} {:6.2} {:>3} {}{}\n", it.x, it.y, it.dx, it.dy, it.size, it.font, js(&it.text), if it.invisible { " (invisible)" } else { "" }).as_bytes(),
                );
            }
            for (i, f) in t.fonts.iter().enumerate() {
                plat::write_stdout(format!("font {i}: {f}\n").as_bytes());
            }
            0
        }
        Err(e) => {
            plat::write_stdout(format!("error: {e}\n").as_bytes());
            1
        }
    }
}

pub fn dump(path: &str, password: Option<String>) -> i32 {
    let Some(bytes) = plat::read_file(path) else {
        plat::write_stdout(format!("{{\"error\":{}}}\n", js("cannot read the file")).as_bytes());
        return 1;
    };
    let opt_ = Options { password: password.unwrap_or_default(), ..Options::default() };
    let doc = match Document::open_with(bytes, &opt_) {
        Ok(d) => d,
        Err(e) => {
            let kind = if e == Error::Password { "password" } else { "open" };
            plat::write_stdout(format!("{{\"error\":{},\"kind\":{}}}\n", js(&format!("{e}")), js(kind)).as_bytes());
            return 1;
        }
    };
    if let Some(n) = plat::arg_value("--object").and_then(|p| p.parse::<u32>().ok()) {
        let o = doc.get(pdf::read::Ref { num: n, generation: 0 });
        plat::write_stdout(format!("{o:?}\n").as_bytes());
        if let pdf::read::Object::Stream(s) = &o {
            match doc.decode(s) {
                Ok(d) => plat::write_stdout(&d),
                Err(e) => plat::write_stdout(format!("decode: {e}\n").as_bytes()),
            }
        }
        return 0;
    }
    if let Some(p) = plat::arg_value("--content").and_then(|p| p.parse::<usize>().ok()) {
        return match doc.contents(p.saturating_sub(1)) {
            Ok(c) => {
                plat::write_stdout(&c);
                0
            }
            Err(e) => {
                plat::write_stdout(format!("error: {e}\n").as_bytes());
                1
            }
        };
    }
    if let Some(p) = plat::arg_value("--glyphs").and_then(|p| p.parse::<usize>().ok()) {
        return glyphs(&doc, p.saturating_sub(1));
    }
    let mut f: Vec<String> = Vec::new();
    let (ma, mi) = doc.version();
    f.push(format!("\"version\":\"{ma}.{mi}\""));
    f.push(format!("\"encrypted\":{}", doc.is_encrypted()));
    f.push(format!("\"revision\":{}", doc.encryption_revision().map(|r| format!("{r}")).unwrap_or("null".into())));
    f.push(format!("\"owner\":{}", doc.opened_as_owner()));
    f.push(format!("\"permissions\":{}", doc.permissions().map(|p| format!("{}", p as i32)).unwrap_or("null".into())));
    f.push(format!("\"repaired\":{}", doc.was_repaired()));
    f.push(format!("\"pages\":{}", doc.page_count()));
    let i = doc.info();
    f.push(format!(
        "\"info\":{{\"title\":{},\"author\":{},\"subject\":{},\"keywords\":{},\"creator\":{},\"producer\":{},\"created\":{},\"modified\":{}}}",
        opt(&i.title),
        opt(&i.author),
        opt(&i.subject),
        opt(&i.keywords),
        opt(&i.creator),
        opt(&i.producer),
        date(&i.created),
        date(&i.modified)
    ));
    f.push(format!("\"language\":{}", opt(&doc.language())));
    f.push(format!("\"metadata\":{}", doc.metadata().map(|m| format!("{}", m.len())).unwrap_or("null".into())));
    let mut sizes = Vec::new();
    let mut labels = Vec::new();
    let mut texts = Vec::new();
    let mut links = Vec::new();
    let mut annots = Vec::new();
    let mut glyphs = 0usize;
    for p in 0..doc.page_count() {
        let pg = doc.page(p).expect("page");
        let (w, h) = pg.size();
        sizes.push(format!("[{},{},{}]", num(w), num(h), pg.rotate));
        labels.push(opt(&doc.page_label(p)));
        match doc.page_text(p) {
            Ok(t) => {
                glyphs += t.items.len();
                texts.push(js(&t.text()));
            }
            Err(e) => texts.push(js(&format!("<error: {e}>"))),
        }
        if let Ok(a) = doc.annotations(p) {
            for an in &a {
                annots.push(format!("[{},{}]", p, js(&an.subtype)));
            }
        }
        if let Ok(l) = doc.links(p) {
            for (r, a) in l {
                links.push(format!("[{},[{},{},{},{}],{}]", p, num(r[0]), num(r[1]), num(r[2]), num(r[3]), action(&Some(a))));
            }
        }
    }
    f.push(format!("\"sizes\":[{}]", sizes.join(",")));
    f.push(format!("\"labels\":[{}]", labels.join(",")));
    let mut ol = Vec::new();
    outlines(&doc.outlines(), 0, &mut ol);
    f.push(format!("\"outlines\":[{}]", ol.join(",")));
    f.push(format!("\"links\":[{}]", links.join(",")));
    f.push(format!("\"annotations\":[{}]", annots.join(",")));
    let fields: Vec<String> = doc
        .fields()
        .iter()
        .map(|fl| {
            let kind = match fl.kind {
                FieldKind::Text => "text",
                FieldKind::CheckBox => "checkbox",
                FieldKind::Radio => "radio",
                FieldKind::PushButton => "button",
                FieldKind::ComboBox => "combo",
                FieldKind::ListBox => "list",
                FieldKind::Signature => "signature",
                FieldKind::Unknown => "unknown",
            };
            let vals: Vec<String> = fl.value.iter().map(|v| js(v)).collect();
            format!("{{\"name\":{},\"kind\":\"{kind}\",\"value\":[{}],\"widgets\":{},\"signed\":{}}}", js(&fl.name), vals.join(","), fl.widgets.len(), fl.signature.is_some())
        })
        .collect();
    f.push(format!("\"fields\":[{}]", fields.join(",")));
    let att: Vec<String> = doc
        .attachments()
        .iter()
        .map(|a| {
            let len = a.stream.as_ref().and_then(|s| doc.decode(s).ok()).map(|d| format!("{}", d.len())).unwrap_or("null".into());
            format!("{{\"name\":{},\"mime\":{},\"size\":{},\"len\":{},\"relationship\":{}}}", js(&a.name), opt(&a.mime), a.size.map(|s| format!("{s}")).unwrap_or("null".into()), len, opt(&a.relationship))
        })
        .collect();
    f.push(format!("\"attachments\":[{}]", att.join(",")));
    f.push(format!("\"glyphs\":{glyphs}"));
    f.push(format!("\"text\":[{}]", texts.join(",")));
    plat::write_stdout(format!("{{{}}}\n", f.join(",")).as_bytes());
    0
}

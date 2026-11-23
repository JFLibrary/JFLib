// windows -- the drawing window of a document's windows part (DF-10): the page it shows, the
// zoom, the centre and what is shown, so that a file opens as it was saved
//
//   windows::drawing_window(&doc) -> Option<DrawingWindow>    the first window of type Drawing
//   windows::set_drawing_window(&mut doc, &w)   that window replaced by `w` (its ID and state
//                                kept), added before the others when there is none, a part of
//                                its own when the document has none; the other windows (stencils,
//                                panes) stay as written
//   DrawingWindow { page, view_scale, center, show_* }   None: not written / not in the file
//
// ViewScale is the zoom (1 = 100 %); Visio writes -1 for "whole page" and -2 for "page width".
// ViewCenterX/Y are the point in the window's middle, in inches from the page's lower left
// corner.  The stencil windows are template.rs's (stencil_windows, set_stencil_windows).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{Document, Part};

use crate::vsdx::CT_WINDOWS;
use crate::xml::{self, Event, Ns, Reader, NS_REL, NS_VISIO};

const REL_WINDOWS: &str = "http://schemas.microsoft.com/visio/2010/relationships/windows";
const DECL: &str = "<?xml version='1.0' encoding='utf-8' ?>\r\n";
/// the state Visio gives a maximized drawing window
const WS_MAXIMIZED: u32 = 0x4000_0000;

/// what the drawing window shows
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawingWindow {
    /// the page's ID (Page)
    pub page: Option<u32>,
    /// the zoom, 1 = 100 %; -1 the whole page, -2 the page's width (ViewScale)
    pub view_scale: Option<f64>,
    /// the point in the window's middle, inches from the page's lower left corner
    pub center: Option<(f64, f64)>,
    /// ShowRulers
    pub show_rulers: Option<bool>,
    /// ShowGrid
    pub show_grid: Option<bool>,
    /// ShowPageBreaks
    pub show_page_breaks: Option<bool>,
    /// ShowGuides
    pub show_guides: Option<bool>,
    /// ShowConnectionPoints
    pub show_connection_points: Option<bool>,
}

// one window as written: its text, ID, type and state
struct Win {
    raw: String,
    id: Option<u32>,
    kind: String,
    state: Option<u32>,
    attrs: Vec<(String, String)>,
    // the children with text only: (name, text)
    items: Vec<(String, String)>,
}

fn part_of(doc: &Document) -> Option<usize> {
    doc.parts.iter().position(|p| doc.str(p.content_type) == CT_WINDOWS)
}

// the root's attributes and its windows; None when the part is no windows part this reads
fn parse(data: &[u8]) -> Option<(Vec<(String, String)>, Vec<Win>)> {
    let text = xml::text_of(data)?;
    let mut r = Reader::new(&text, 64, 256);
    let root = match r.next().ok()? {
        Event::Start(s) if s.local == "Windows" => s,
        _ => return None,
    };
    let attrs = root.attrs.iter().map(|a| (String::from(a.qname), String::from(a.value.as_ref()))).collect();
    let mut out = Vec::new();
    if root.empty {
        return Some((attrs, out));
    }
    loop {
        match r.next().ok()? {
            Event::Start(s) => {
                let own: Vec<(String, String)> = s.attrs.iter().filter(|a| a.ns == Ns::None).map(|a| (String::from(a.local), String::from(a.value.as_ref()))).collect();
                let get = |n: &str| own.iter().find(|a| a.0 == n).map(|a| a.1.clone());
                let mut w = Win {
                    raw: String::new(),
                    id: get("ID").and_then(|v| v.trim().parse().ok()),
                    kind: if s.local == "Window" { get("WindowType").unwrap_or_default() } else { String::new() },
                    state: get("WindowState").and_then(|v| v.trim().parse().ok()),
                    attrs: own,
                    items: Vec::new(),
                };
                let mut depth = 0usize;
                let mut cur: Option<(String, String)> = None;
                let end = loop {
                    match r.next().ok()? {
                        Event::Start(c) => {
                            depth += 1;
                            cur = (depth == 1).then(|| (String::from(c.local), String::new()));
                        }
                        Event::Text(t) => {
                            if let Some(c) = cur.as_mut() {
                                c.1.push_str(&t);
                            }
                        }
                        Event::End { end } => {
                            if depth == 0 {
                                break end;
                            }
                            if depth == 1 {
                                if let Some(c) = cur.take() {
                                    w.items.push(c);
                                }
                            }
                            depth -= 1;
                        }
                        Event::Eof => return None,
                    }
                };
                w.raw = String::from(text.get(s.at..end)?);
                if out.len() >= 100_000 {
                    return None;
                }
                out.push(w);
            }
            Event::End { .. } => return Some((attrs, out)),
            Event::Text(_) => {}
            Event::Eof => return None,
        }
    }
}

fn flag(v: Option<&String>) -> Option<bool> {
    match v.map(|s| s.trim()) {
        Some("1") | Some("true") => Some(true),
        Some("0") | Some("false") => Some(false),
        _ => None,
    }
}

/// the document's drawing window (the first window of type Drawing), None without one
pub fn drawing_window(doc: &Document) -> Option<DrawingWindow> {
    let (_, list) = part_of(doc).and_then(|i| parse(&doc.parts[i].data))?;
    let w = list.iter().find(|w| w.kind.eq_ignore_ascii_case("Drawing"))?;
    let attr = |n: &str| w.attrs.iter().find(|a| a.0 == n).map(|a| a.1.clone());
    let num = |n: &str| attr(n).and_then(|v| v.trim().parse::<f64>().ok()).filter(|x| x.is_finite());
    let item = |n: &str| flag(w.items.iter().find(|i| i.0 == n).map(|i| &i.1));
    Some(DrawingWindow {
        page: attr("Page").and_then(|v| v.trim().parse().ok()),
        view_scale: num("ViewScale"),
        center: num("ViewCenterX").zip(num("ViewCenterY")),
        show_rulers: item("ShowRulers"),
        show_grid: item("ShowGrid"),
        show_page_breaks: item("ShowPageBreaks"),
        show_guides: item("ShowGuides"),
        show_connection_points: item("ShowConnectionPoints"),
    })
}

fn attr(s: &mut String, name: &str, v: &str) {
    s.push(' ');
    s.push_str(name);
    s.push_str("='");
    crate::props::escape(v, s);
    s.push('\'');
}

// a number as the file writes it: finite, no exponent for the usual sizes
fn num(x: f64) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        String::from("0")
    }
}

/// make the document's drawing window `w` (see the module comment)
pub fn set_drawing_window(doc: &mut Document, w: &DrawingWindow) {
    let at = part_of(doc);
    let parsed = at.and_then(|i| parse(&doc.parts[i].data));
    let (root, list) = match parsed {
        Some(p) => p,
        None => (
            alloc::vec![
                (String::from("ClientWidth"), String::from("1600")),
                (String::from("ClientHeight"), String::from("900")),
                (String::from("xmlns"), String::from(NS_VISIO)),
                (String::from("xmlns:r"), String::from(NS_REL)),
                (String::from("xml:space"), String::from("preserve")),
            ],
            Vec::new(),
        ),
    };
    let old = list.iter().position(|x| x.kind.eq_ignore_ascii_case("Drawing"));
    let id = old.and_then(|i| list[i].id).unwrap_or_else(|| list.iter().filter_map(|x| x.id).max().map_or(0, |m| m.saturating_add(1)));
    let state = old.and_then(|i| list[i].state).unwrap_or(WS_MAXIMIZED);
    let mut win = format!("<Window ID='{id}' WindowType='Drawing' WindowState='{state}'");
    // the old window's other attributes (size, position) stay
    if let Some(i) = old {
        for (k, v) in &list[i].attrs {
            if !["ID", "WindowType", "WindowState", "ContainerType", "Page", "ViewScale", "ViewCenterX", "ViewCenterY"].contains(&k.as_str()) {
                attr(&mut win, k, v);
            }
        }
    }
    win.push_str(" ContainerType='Page'");
    if let Some(p) = w.page {
        win.push_str(&format!(" Page='{p}'"));
    }
    if let Some(z) = w.view_scale {
        win.push_str(&format!(" ViewScale='{}'", num(z)));
    }
    if let Some((x, y)) = w.center {
        win.push_str(&format!(" ViewCenterX='{}' ViewCenterY='{}'", num(x), num(y)));
    }
    win.push('>');
    let shows = [
        ("ShowRulers", w.show_rulers),
        ("ShowGrid", w.show_grid),
        ("ShowPageBreaks", w.show_page_breaks),
        ("ShowGuides", w.show_guides),
        ("ShowConnectionPoints", w.show_connection_points),
    ];
    // the old window's text children in Visio's order: the Show* ones from `w` where it says them
    let mut items: Vec<(String, String)> = old.map(|i| list[i].items.clone()).unwrap_or_default();
    for (name, v) in shows.iter().rev() {
        let Some(v) = v else { continue };
        let text = String::from(if *v { "1" } else { "0" });
        match items.iter_mut().find(|i| i.0 == *name) {
            Some(i) => i.1 = text,
            None => items.insert(0, (String::from(*name), text)),
        }
    }
    let order = |n: &str| shows.iter().position(|s| s.0 == n).unwrap_or(shows.len());
    items.sort_by_key(|i| order(&i.0));
    for (n, t) in &items {
        win.push('<');
        win.push_str(n);
        if t.is_empty() {
            win.push_str("/>");
        } else {
            win.push('>');
            crate::props::escape(t, &mut win);
            win.push_str("</");
            win.push_str(n);
            win.push('>');
        }
    }
    win.push_str("</Window>");
    let mut s = String::from(DECL);
    s.push_str("<Windows");
    for (q, v) in &root {
        attr(&mut s, q, v);
    }
    s.push('>');
    if old.is_none() {
        s.push_str(&win);
    }
    for (k, x) in list.iter().enumerate() {
        if Some(k) == old {
            s.push_str(&win);
        } else {
            s.push_str(&x.raw);
        }
    }
    s.push_str("</Windows>");
    let data = s.into_bytes();
    match at {
        Some(i) => doc.parts[i].data = data,
        None => {
            let part = Part { name: doc.intern("/visio/windows.xml"), content_type: doc.intern(CT_WINDOWS), rel_type: doc.intern(REL_WINDOWS), data };
            doc.parts.push(part);
        }
    }
}

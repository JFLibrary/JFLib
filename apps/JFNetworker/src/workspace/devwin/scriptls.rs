// scriptls -- the language services of the IDE's editor: apps/jfs/crates/script's services (script::ls,
// script::fmt) for the code editor of a draft, with the names the simulator's ScriptService
// gives scripts
//
//   host_items() -> Vec<HostItem>   the host's globals (jfn_sim/scriptsvc.rs: log, now, rand,
//                                   config, timer, net, dns, http, db, disk, metrics) with their
//                                   parameters and a short description in the window's
//                                   language (our own words, t())
//   Assist                          ui::CodeAssist on script::ls with those names: completion,
//                                   hover, signature help (ui.code_editor_with);
//                                   Assist::with_loader(&load) also reads the files a script
//                                   imports (completion of a module's names; the IDE)
//   folds(doc)                      the folding ranges of the text into the document
//   outline(text) -> Vec<Entry>     the fns (the ones declared inside indented) and the
//                                   top-level lets, for the list beside the editor
//   rename(doc, pos, to) -> Result<usize, String>   every place of the name at pos renamed as
//                                   one undo step of the draft -> how many; refused with
//                                   apps/jfs/crates/script's German reason (a keyword, no valid name, a
//                                   clash, syntax errors)
//   format(doc, unit) -> Result<bool, String>   the formatted text as one undo step (false:
//                                   it was formatted already); refused with the first syntax
//                                   error
//   name_at(doc, pos) -> String     the word at a position (the rename flyout's start)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use script::ls::{self, CompleteCx, HostItem, ItemKind};
use ui::*;

use crate::app::tr::t;

/// the names of ScriptService with parameters (None: a value or a namespace) and a short
/// description
pub(crate) fn host_items() -> Vec<HostItem<'static>> {
    let f = |name: &'static str, params: &'static str, doc: &'static str| HostItem { name, params: Some(params), doc };
    let v = |name: &'static str, doc: &'static str| HostItem { name, params: None, doc };
    alloc::vec![
        f("log", "x, ..", t("Schreibt eine Zeile ins Protokoll des Geräts, mit dem Namen des Dienstes davor.")),
        f("log.debug", "x, ..", t("Eine Zeile der Stufe debug ins Protokoll des Geräts.")),
        f("log.info", "x, ..", t("Eine Zeile der Stufe info ins Protokoll des Geräts.")),
        f("log.warn", "x, ..", t("Eine Warnung ins Protokoll des Geräts.")),
        f("log.error", "x, ..", t("Ein Fehler ins Protokoll des Geräts.")),
        f("now", "", t("Die Zeit der Simulation in Millisekunden (float), samt der Rechenzeit bisher.")),
        f("rand", "a, b", t("Eine Zufallszahl aus dem Strom des Dienstes: ohne Argument 0 bis unter 1, mit n 0 bis n-1, mit a, b a bis b.")),
        v("config", t("Die Einstellungen des Dienstes ohne den Quelltext, eine Map (z. B. config.port).")),
        v("timer", t("Zeitgeber: after, every, cancel.")),
        f("timer.after", "ms, f", t("Ruft f einmal nach ms Millisekunden auf; gibt eine Nummer für cancel zurück.")),
        f("timer.every", "ms, f", t("Ruft f alle ms Millisekunden auf (mindestens alle 10 µs); gibt eine Nummer für cancel zurück.")),
        f("timer.cancel", "id", t("Hält einen Zeitgeber von after oder every an.")),
        v("net", t("TCP und UDP: listen, connect, udp.")),
        f("net.listen", "port, handlers", t("Wartet auf TCP-Verbindungen; handlers ist eine Map mit accept, data, close und binary.")),
        f("net.connect", "addr, port, handlers", t("Baut eine TCP-Verbindung auf; handlers ist eine Map mit connect, data, close und binary.")),
        f("net.udp", "port, f, opts", t("Öffnet einen UDP-Port (0: einen freien); f(sock, msg) für jedes Datagramm.")),
        v("dns", t("Der Resolver des Geräts: resolve.")),
        f("dns.resolve", "name, f", t("Löst einen Namen über den Resolver des Geräts auf; f(err, addrs).")),
        v("http", t("HTTP: serve, request.")),
        f("http.serve", "port, f", t("Ein HTTP-Server (einmal je Skript); f(req, res), die Rückgabe oder res.send / res.json antwortet.")),
        f("http.request", "url, f", t("Eine HTTP-Anfrage an url (oder eine Map mit url, method, headers, body, timeout_ms); f(err, resp).")),
        v("db", t("Eine Datenbank des Labors: connect.")),
        f("db.connect", "addr, port", t("Verbindet mit einer Datenbank (Port 7100, wenn keiner dasteht); die Verbindung kennt query, begin, commit, rollback und close.")),
        v("disk", t("JSON-Werte auf der Disk des Geräts: get, put, delete, scan; sie überstehen Neustarts.")),
        f("disk.get", "key, f", t("Liest den Wert zu key; f(err, value).")),
        f("disk.put", "key, value, f", t("Schreibt value unter key; f(err).")),
        f("disk.delete", "key, f", t("Löscht key; f(err, had).")),
        f("disk.scan", "prefix, f", t("Alle Einträge, deren Schlüssel mit prefix anfängt; f(err, items).")),
        v("metrics", t("Kennzahlen des Dienstes: inc, gauge, observe.")),
        f("metrics.inc", "name, n", t("Zählt eine Kennzahl um n hoch (in der Zeitreihe eine Rate).")),
        f("metrics.gauge", "name, v", t("Setzt einen Messwert (in der Zeitreihe ein Stand).")),
        f("metrics.observe", "name, v", t("Ein Messwert für Quantile: NAME_p50, _p95, _p99 und _max über fünf Minuten.")),
    ]
}

/// the kind of a completion item for the editor
fn kind_of(k: ItemKind) -> u8 {
    match k {
        ItemKind::Variable => CI_VARIABLE,
        ItemKind::Param => CI_PARAM,
        ItemKind::Function => CI_FUNCTION,
        ItemKind::Method => CI_METHOD,
        ItemKind::Module => CI_MODULE,
        ItemKind::Constant => CI_CONSTANT,
        ItemKind::Value => CI_VALUE,
        ItemKind::Keyword => CI_KEYWORD,
    }
}

/// apps/jfs/crates/script's services with the host's names (see the module comment)
pub(crate) struct Assist<'a> {
    host: Vec<HostItem<'static>>,
    /// the text of an imported file by the path written in import("…")
    load: Option<&'a dyn Fn(&str) -> Option<String>>,
}

impl<'a> Assist<'a> {
    /// the services that read imported files through `load`
    pub(crate) fn with_loader(load: &'a dyn Fn(&str) -> Option<String>) -> Assist<'a> {
        Assist { host: host_items(), load: Some(load) }
    }

    fn cx(&self) -> CompleteCx<'_> {
        CompleteCx { host: &self.host, load: self.load }
    }
}

impl CodeAssist for Assist<'_> {
    fn complete(&mut self, doc: &CodeDoc, at: TextPos, _: bool) -> Option<CompList> {
        let c = ls::complete(&doc.text(), doc.flat(at) as u32, &self.cx());
        let items = c.items.into_iter().map(|i| CompItem { label: i.label, detail: i.detail, doc: i.doc, kind: kind_of(i.kind), insert: String::new() }).collect();
        Some(CompList { start: doc.unflat(c.start as usize), items })
    }

    fn hover(&mut self, doc: &CodeDoc, at: TextPos) -> Option<String> {
        ls::hover(&doc.text(), doc.flat(at) as u32, &self.cx()).map(|h| h.text())
    }

    fn signature(&mut self, doc: &CodeDoc, at: TextPos) -> Option<SigHelp> {
        let s = ls::signature(&doc.text(), doc.flat(at) as u32, &self.cx())?;
        Some(SigHelp { label: s.label, params: s.params.iter().map(|&(a, b)| (a as usize, b as usize)).collect(), active: s.active, doc: s.doc })
    }
}

/// the folding ranges of the draft's text into its document
pub(crate) fn folds(doc: &mut CodeDoc) {
    let f: Vec<(usize, usize)> = ls::folding(&doc.text()).iter().map(|x| (x.line as usize - 1, x.end as usize - 1)).collect();
    doc.set_fold_ranges(&f);
}

/// an entry of the outline: its text ("fn handle(req)", "let hits"), the level (nested fns
/// deeper), where the name is (line and column from 1)
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub text: String,
    pub level: usize,
    pub line: usize,
    pub col: usize,
}

/// the outline of a text (see the module comment)
pub(crate) fn outline(text: &str) -> Vec<Entry> {
    fn add(out: &mut Vec<Entry>, s: &ls::DocSymbol, level: usize) {
        out.push(Entry { text: s.detail.clone(), level, line: s.span.line as usize, col: s.span.col as usize });
        for c in &s.children {
            add(out, c, level + 1);
        }
    }
    let mut out = Vec::new();
    for s in ls::symbols(text) {
        add(&mut out, &s, 0);
    }
    out
}

/// the word at a position ("" when there is none)
pub(crate) fn name_at(doc: &CodeDoc, pos: TextPos) -> String {
    let t = doc.line(pos.line);
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let b = pos.byte.min(t.len());
    let a = t[..b].char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(b, |(i, _)| i);
    let e = t[b..].char_indices().find(|(_, c)| !word(*c)).map_or(t.len(), |(i, _)| b + i);
    String::from(&t[a..e])
}

/// rename the name at pos to `to` (one undo step) -> the places changed
pub(crate) fn rename(doc: &mut CodeDoc, pos: TextPos, to: &str) -> Result<usize, String> {
    let edits = ls::rename(&doc.text(), doc.flat(pos) as u32, to.trim())?;
    let te: Vec<TextEdit> = edits.iter().map(|e| TextEdit::new(doc.unflat(e.start as usize), doc.unflat(e.end as usize), &e.text)).collect();
    doc.apply_edits(&te, t("Umbenennen"));
    Ok(te.len())
}

/// format the text (one undo step; unit: one level of indentation) -> changed
pub(crate) fn format(doc: &mut CodeDoc, unit: &str) -> Result<bool, String> {
    match script::fmt::format_with(&doc.text(), unit) {
        Ok(f) => Ok(doc.replace_text(&f, t("Formatieren"))),
        Err(d) => Err(d.first().map_or(String::new(), |x| format!("{}:{}: {}", x.line, x.col, x.msg))),
    }
}

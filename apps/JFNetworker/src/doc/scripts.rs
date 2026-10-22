// scripts -- the scripts of a device in the project (its services of kind "script") and their
// changes: a new source, a new script from a template, a new name, removal -- each one undo
// step that keeps the running device (Doc::apply_kept: Sim::keep turns the step into
// Action::ScriptSource, Service, RenameService or RemoveService, so a script keeps its disk,
// numbers and errors through edits, undo and redo); start, stop and restart in the simulator
//
//   KIND                              "script"
//   source_of(def) -> String          the source as the editor shows it: a list of lines
//                                     joined by '\n', a text as it is
//   with_source(def, text) -> Value   the definition's config with a new source (a list of
//                                     lines stays a list, a text a text)
//   of(d) / find(d, name)             the device's scripts in the project's order, one by name
//   first_error(text)                 what keeps a source from compiling (script::check):
//                                     (line, column, message) of the first error
//   check_name(d, name, old)          a name for a script: one word of letters, digits, - _
//                                     and . (at most 32), free on the device -> the name
//   free_name(d, base)                base, base2, base3 … whichever is free
//   apply(doc, dev, name, text)       the new source (Ok(false): it was the same); a source that
//                                     does not compile is refused: the project only takes what
//                                     the simulator can run
//   add(doc, dev, k) / rename(doc, dev, name, to) / remove(doc, dev, name)
//   run(doc, dev, name, on) / restart(doc, dev, name)   the simulator only, as the shell's
//                                     "script start|stop|restart": whether the script starts
//                                     with the device stays the project's (the Dienste tab)
//   TEMPLATES / template(k, t)        the templates of "Neu" (our own words): an empty script,
//                                     a UDP echo, an HTTP server, a client that polls, a
//                                     key-value store on the disk; comments and texts through
//                                     the catalogue `t`
//
// A new script's source is a list of lines in the project file, as people write them there.
// Removing a script drops what it stored on the disk; an undo brings the definition back, and
// the script starts with an empty disk (the simulator keeps no copy of a removed service).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::{Device, ServiceDef};
use jfn_sim::{Action, Cmd};

use super::{edits, Doc};
use crate::app::tr::{t, tf};

/// the kind of a script service
pub(crate) const KIND: &str = "script";
/// the longest name of a script
const NAME_MAX: usize = 32;

/// the source of a definition as the editor shows it
pub(crate) fn source_of(def: &ServiceDef) -> String {
    match def.config.get("source") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(lines)) => {
            let v: Vec<&str> = lines.iter().map(|l| l.as_str().unwrap_or("")).collect();
            v.join("\n")
        }
        _ => String::new(),
    }
}

/// the config of a definition with another source: a list of lines stays one (a new one is
/// one), a text stays a text
pub(crate) fn with_source(def: &ServiceDef, text: &str) -> Value {
    let mut c = match &def.config {
        Value::Object(_) => def.config.clone(),
        _ => Value::Object(Vec::new()),
    };
    let src = match def.config.get("source") {
        Some(Value::String(_)) => Value::string(text),
        _ => Value::Array(text.split('\n').map(Value::string).collect()),
    };
    c.insert("source", src);
    c
}

/// the device's scripts in the project's order
pub(crate) fn of(d: &Device) -> Vec<&ServiceDef> {
    d.services.iter().filter(|s| s.kind == KIND).collect()
}

/// the index of script `name` among the device's services
pub(crate) fn find(d: &Device, name: &str) -> Option<usize> {
    d.services.iter().position(|s| s.kind == KIND && s.name == name)
}

/// the first error that keeps `text` from compiling: line, column, message (German, as
/// lib/script words it)
pub(crate) fn first_error(text: &str) -> Option<(u32, u32, String)> {
    script::check(text).into_iter().find(|d| d.error).map(|d| (d.line, d.col, d.msg))
}

/// a name for a script of device `d` (`old`: the script's name now, None for a new one)
pub(crate) fn check_name(d: &Device, name: &str, old: Option<&str>) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(String::from(t("Ein Skript braucht einen Namen.")));
    }
    let first = n.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    if !first || n.len() > NAME_MAX || !n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err(tf("„{}“ geht nicht als Name: Buchstaben, Ziffern, - _ und . (am Anfang ein Buchstabe oder eine Ziffer, höchstens 32 Zeichen).", &[n]));
    }
    if d.services.iter().any(|s| s.name.eq_ignore_ascii_case(n) && old.is_none_or(|o| !s.name.eq_ignore_ascii_case(o))) {
        return Err(tf("Auf {} gibt es schon einen Dienst „{}“.", &[&d.name, n]));
    }
    Ok(String::from(n))
}

/// `base`, else base2, base3 … whichever no service of the device has
pub(crate) fn free_name(d: &Device, base: &str) -> String {
    let taken = |n: &str| d.services.iter().any(|s| s.name.eq_ignore_ascii_case(n));
    if !taken(base) {
        return String::from(base);
    }
    let mut k = 2;
    loop {
        let n = format!("{}{}", base, k);
        if !taken(&n) {
            return n;
        }
        k += 1;
    }
}

fn device(doc: &Doc, dev: u32) -> Result<Device, String> {
    doc.proj.device(dev).cloned().ok_or_else(|| tf("Das Gerät {} gibt es nicht.", &[&dev.to_string()]))
}

fn script_of(d: &Device, name: &str) -> Result<usize, String> {
    find(d, name).ok_or_else(|| tf("Das Skript „{}“ gibt es auf {} nicht.", &[name, &d.name]))
}

/// the new source of script `name`: one undo step, the running script takes it and keeps its
/// disk and numbers; Ok(false) when it is the same, a refusal when it does not compile
pub(crate) fn apply(doc: &mut Doc, dev: u32, name: &str, text: &str) -> Result<bool, String> {
    let d = device(doc, dev)?;
    let j = script_of(&d, name)?;
    if source_of(&d.services[j]) == text {
        return Ok(false);
    }
    if let Some((line, col, msg)) = first_error(text) {
        return Err(tf("Das Skript übersetzt nicht: Zeile {}, Spalte {}: {}", &[&line.to_string(), &col.to_string(), &msg]));
    }
    let config = with_source(&d.services[j], text);
    let label = format!("Skript {} auf {} übernehmen", name, d.name);
    if let Some(e) = edits::device_edit(&doc.proj, dev, &label, |x| x.services[j].config = config) {
        doc.apply_kept(e, dev);
    }
    Ok(true)
}

/// a new script from template k (a free name after the template's): one undo step, it starts
/// at once -> its name
pub(crate) fn add(doc: &mut Doc, dev: u32, k: usize) -> Result<String, String> {
    let d = device(doc, dev)?;
    let tpl = TEMPLATES.get(k).ok_or_else(|| String::from(t("Diese Vorlage gibt es nicht.")))?;
    let name = free_name(&d, tpl.name);
    let lines: Vec<Value> = template(k, &t).iter().map(|l| Value::string(l)).collect();
    let config = Value::object([("source", Value::Array(lines))]);
    let def = ServiceDef { kind: String::from(KIND), name: name.clone(), disabled: false, config };
    let label = format!("Skript {} auf {} anlegen", name, d.name);
    if let Some(e) = edits::device_edit(&doc.proj, dev, &label, |x| x.services.push(def)) {
        doc.apply_kept(e, dev);
    }
    Ok(name)
}

/// script `name` is called `to` from now on: one undo step, it goes on running -> the name
pub(crate) fn rename(doc: &mut Doc, dev: u32, name: &str, to: &str) -> Result<String, String> {
    let d = device(doc, dev)?;
    let j = script_of(&d, name)?;
    let to = check_name(&d, to, Some(name))?;
    if to == name {
        return Ok(to);
    }
    let label = format!("Skript {} auf {} umbenennen", name, d.name);
    let new = to.clone();
    if let Some(e) = edits::device_edit(&doc.proj, dev, &label, |x| x.services[j].name = new) {
        doc.apply_kept(e, dev);
    }
    Ok(to)
}

/// script `name` goes: one undo step (what it stored on the disk goes with it)
pub(crate) fn remove(doc: &mut Doc, dev: u32, name: &str) -> Result<(), String> {
    let d = device(doc, dev)?;
    let j = script_of(&d, name)?;
    let label = format!("Skript {} auf {} löschen", name, d.name);
    if let Some(e) = edits::device_edit(&doc.proj, dev, &label, |x| {
        x.services.remove(j);
    }) {
        doc.apply_kept(e, dev);
    }
    Ok(())
}

/// starts or stops script `name` in the simulator (a start begins with a fresh program; the
/// disk and the numbers stay)
pub(crate) fn run(doc: &mut Doc, dev: u32, name: &str, on: bool) -> Result<(), String> {
    doc.sim.send(Cmd::Apply(Action::ServiceRun { dev, name: String::from(name), on })).map(|_| ()).map_err(|e| crate::app::tr::msg(&e.msg()))
}

/// stops and starts script `name` again
pub(crate) fn restart(doc: &mut Doc, dev: u32, name: &str) -> Result<(), String> {
    run(doc, dev, name, false)?;
    run(doc, dev, name, true)
}

// ---------------------------------------------------------------- templates

/// a template of "Neu": its title in the menu, the name the script gets
pub(crate) struct Template {
    pub title: &'static str,
    pub name: &'static str,
}

// i18n: title
pub(crate) const TEMPLATES: [Template; 5] = [
    Template { title: "Leeres Skript", name: "script" },
    Template { title: "UDP-Echo", name: "echo" },
    Template { title: "HTTP-Server", name: "web" },
    Template { title: "Abfrage (fragt regelmäßig eine URL ab)", name: "poll" },
    Template { title: "Schlüssel-Wert-Speicher auf der Disk", name: "kv" },
];

/// the lines of template k, its comments and texts through `t` (the catalogue: the window's
/// language)
pub(crate) fn template(k: usize, t: &dyn Fn(&'static str) -> &'static str) -> Vec<String> {
    let c = |s: &str| format!("// {}", s);
    let l = |s: &str| String::from(s);
    match k {
        1 => alloc::vec![
            c(t("UDP-Echo: schickt jedes Datagramm an seinen Absender zurück.")),
            l("let port = config.port ?? 7"),
            l("net.udp(port, fn(sock, msg) {"),
            l("  metrics.inc(\"echos\")"),
            l("  sock.send(msg.from, msg.port, msg.data)"),
            l("})"),
            format!("log(\"{}\")", t("Echo bereit auf UDP-Port ${port}")),
        ],
        2 => alloc::vec![
            c(t("HTTP-Server: GET / grüßt, /status nennt die Zahlen als JSON, alles andere ist 404.")),
            l("let port = config.port ?? 80"),
            l("let hits = 0"),
            l("http.serve(port, fn(req, res) {"),
            l("  hits += 1"),
            l("  metrics.inc(\"requests\")"),
            l("  if req.path == \"/status\" {"),
            l("    return {hits: hits, uptime_ms: now()}"),
            l("  }"),
            l("  if req.path != \"/\" {"),
            l("    return 404"),
            l("  }"),
            format!("  return \"{}\\n\"", t("Hallo ${req.peer}, das ist Anfrage ${hits}.")),
            l("})"),
            format!("log(\"{}\")", t("Webserver bereit auf Port ${port}")),
        ],
        3 => alloc::vec![
            c(t("Abfrage: fragt alle config.every_ms Millisekunden config.url ab und misst die Antwortzeit.")),
            l("let url = config.url ?? \"http://10.0.0.1/status\""),
            l("let every = config.every_ms ?? 1000"),
            l("timer.every(every, fn() {"),
            l("  let t0 = now()"),
            l("  http.request(url, fn(err, resp) {"),
            l("    if err {"),
            l("      metrics.inc(\"failed\")"),
            l("      log.warn(\"${url}: ${err}\")"),
            l("      return"),
            l("    }"),
            l("    metrics.inc(\"ok\")"),
            l("    metrics.observe(\"latency_us\", int((now() - t0) * 1000))"),
            format!("    print(\"{}\")", t("${resp.status} nach ${now() - t0} ms")),
            l("  })"),
            l("})"),
        ],
        4 => alloc::vec![
            c(t("Schlüssel-Wert-Speicher: GET, PUT und DELETE auf /kv/SCHLÜSSEL, die Werte liegen auf der Disk")),
            c(t("des Geräts und überstehen Neustarts und Stromausfälle.")),
            l("let port = config.port ?? 80"),
            l("http.serve(port, fn(req, res) {"),
            l("  if !req.path.starts_with(\"/kv/\") {"),
            l("    return 404"),
            l("  }"),
            l("  let key = req.path.slice(4)"),
            l("  if req.method == \"GET\" {"),
            l("    disk.get(key, fn(err, v) {"),
            format!("      if v == nil {{ return res.send(404, \"{}\\n\") }}", t("kein Wert")),
            l("      res.send(200, \"${v}\\n\")"),
            l("    })"),
            l("  } else if req.method == \"PUT\" {"),
            l("    disk.put(key, req.body, fn(err) {"),
            l("      if err { return res.send(507, \"${err}\\n\") }"),
            l("      metrics.inc(\"puts\")"),
            l("      res.send(204, nil)"),
            l("    })"),
            l("  } else if req.method == \"DELETE\" {"),
            l("    disk.delete(key, fn(err, had) {"),
            l("      if had { return res.send(204, nil) }"),
            format!("      res.send(404, \"{}\\n\")", t("kein Wert")),
            l("    })"),
            l("  } else {"),
            l("    return 405"),
            l("  }"),
            l("})"),
            format!("log(\"{}\")", t("Speicher bereit auf Port ${port}")),
        ],
        _ => alloc::vec![
            c(t("Ein Skript reagiert auf Ereignisse: net.udp, net.listen, http.serve, timer.every …")),
            c(t("Strg+S übernimmt es ins Projekt und in den laufenden Simulator.")),
            format!("log(\"{}\")", t("Skript gestartet")),
        ],
    }
}

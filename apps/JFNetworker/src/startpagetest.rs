// startpagetest -- checks of the start page without a window (part of --selftest, group
// "startpage"): the templates of assets/templates (listed as the page lists them, no problems, the
// German texts of the table are the file's, both languages, every one running: the hosts reach what
// they should), the setting and when the page shows, the recent rows (a file, a missing one, one
// the app may not look at), the times, the project's info in the file and as an undo step, an old
// file without it, a simulator without a project that arms no timer

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use datetime::Zone;
use jfn_model::project::Project;
use jfn_sim::goal::judge;
use jfn_sim::{Cmd, SEC};

use crate::app::constants::RECENT_MAX;
use crate::doc::sim::Sim;
use crate::selftest::T;
use crate::workspace::startpage::{at_start, rows_of, when_text, Reach, TEMPLATES, TEMPLATE_DIR};

const HOME: &str = include_str!("../assets/templates/heimnetz.jfnet");
const OFFICE: &str = include_str!("../assets/templates/buero-vlans.jfnet");
const CAMPUS: &str = include_str!("../assets/templates/campus.jfnet");
const DC: &str = include_str!("../assets/templates/rechenzentrum.jfnet");
const WEB: &str = include_str!("../assets/templates/webdienst.jfnet");
const STUDY: &str = include_str!("../assets/templates/groesser-oder-mehr.jfnet");
const TWO_PCS: &str = include_str!("../tests/fixtures/two_pcs.jfnet");

/// per template: its file, how long it runs (s), what must hold then (the words of "expect ..."
/// lines of a scenario)
const PROOFS: [(&str, &str, u64, &[&str]); 6] = [
    ("heimnetz.jfnet", HOME, 20, &["PC1 reaches 192.168.1.1", "PC1 reaches 203.0.113.10", "PC1 reaches 192.168.1.20", "Laptop1 reaches 203.0.113.10", "Phone1 reaches 192.168.1.1", "Phone1 reaches 203.0.113.10", "WEB1 reaches 192.168.1.20"]),
    ("buero-vlans.jfnet", OFFICE, 20, &["PC1 reaches 10.0.10.1", "PC1 reaches 10.0.30.10", "PC1 reaches intranet.buero.lab", "PC3 reaches 10.0.10.20", "PC4 reaches drucker.buero.lab", "SRV1 reaches 10.0.20.1"]),
    ("campus.jfnet", CAMPUS, 90, &["PC1 reaches 10.20.10.1", "PC1 reaches www.campus.lab", "PC2 reaches 10.10.100.10", "Laptop1 reaches www.campus.lab", "PC3 reaches www.campus.lab", "PC4 reaches 10.10.100.10", "Laptop2 reaches 10.20.10.1", "R1 route 10.30.20.0/24 via 10.10.0.6", "R1 route 10.20.10.0/24 via 10.10.0.2"]),
    ("rechenzentrum.jfnet", DC, 60, &["SRV1 reaches 10.1.1.12", "SRV1 reaches 10.1.2.11", "SRV1 reaches 10.1.4.12", "SRV8 reaches 10.1.1.11", "SRV3 reaches 10.1.3.11", "SRV3 svc web metric requests >= 1", "Leaf1 route 10.1.4.0/24 via 10.0.11.1", "Leaf1 route 10.1.4.0/24 via 10.0.21.1"]),
    ("webdienst.jfnet", WEB, 20, &["PC1 reaches 10.0.1.1", "LB1 reaches 10.0.2.13", "WEB1 svc web metric requests >= 20", "WEB2 svc web metric requests >= 20", "WEB3 svc web metric requests >= 20", "GEN1 svc load metric errors == 0"]),
    ("groesser-oder-mehr.jfnet", STUDY, 10, &["LB reaches 10.0.0.11", "WEB1 svc web metric requests >= 20", "GEN svc load metric errors == 0"]),
];

pub(crate) fn run(t: &mut T) {
    templates(t);
    proofs(t);
    setting(t);
    recent(t);
    times(t);
    info(t);
    idle(t);
}

// the English row of a text of the window's catalogue
fn en(k: &str) -> Option<&'static str> {
    include_str!("locales/ui.tsv").lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once('\t')).find(|r| r.0 == k).map(|r| r.1)
}

fn templates(t: &mut T) {
    // the app's list: every file of the folder is on the page, and in its order the table's
    let mut names: Vec<String> = plat::fs::read_dir(TEMPLATE_DIR).unwrap_or_default().into_iter().map(|e| e.name).collect();
    names.sort();
    let mut table: Vec<String> = TEMPLATES.iter().map(|x| String::from(x.file)).collect();
    table.sort();
    t.eq("the folder is the table", names, table);
    t.eq("the proofs follow the table", PROOFS.iter().map(|p| p.0).collect::<Vec<_>>(), TEMPLATES.iter().map(|x| x.file).collect::<Vec<_>>());
    for (x, proof) in TEMPLATES.iter().zip(PROOFS.iter()) {
        let path = format!("{}/{}", TEMPLATE_DIR, x.file);
        let Ok(p) = crate::doc::file::read(&path) else {
            t.check(&format!("{} loads", x.file), false);
            continue;
        };
        t.eq(&format!("{}: the bundle's bytes", x.file), plat::read_file(&path).map(|b| String::from_utf8_lossy(&b).into_owned()), Some(String::from(proof.1)));
        t.eq(&format!("{}: its title is the table's", x.file), p.title.as_str(), x.name);
        t.eq(&format!("{}: its description is the table's", x.file), p.description.as_str(), x.text);
        t.check(&format!("{}: a free project", x.file), p.lab.is_none());
        t.eq(&format!("{}: no problems of the project", x.file), p.check().iter().map(|i| i.text.clone()).collect::<Vec<_>>(), Vec::<String>::new());
        t.check(&format!("{}: runs whole", x.file), Sim::new(&p).errors.is_empty());
        // English on both texts
        t.check(&format!("{}: an English name", x.file), en(x.name).is_some_and(|e| !e.is_empty()));
        t.check(&format!("{}: an English text", x.file), en(x.text).is_some_and(|e| !e.is_empty() && e != x.text));
    }
}

// a template runs: the simulator takes it, and after its time everything of its proof holds, the
// engine finds nothing wrong
fn proofs(t: &mut T) {
    for (file, text, secs, checks) in PROOFS {
        let Ok(p) = Project::from_text(text) else {
            t.check(&format!("{} parses", file), false);
            continue;
        };
        let mut sim = Sim::new(&p);
        for _ in 0..secs {
            let _ = sim.send(Cmd::RunFor(SEC));
            sim.take_reports();
        }
        for c in checks {
            match jfn_model::scenario::parse_expectation(c) {
                Ok(x) => {
                    let v = judge(&p, sim.engine(), &x, &|_| String::new());
                    t.check(&format!("{}: {} ({})", file, c, v.report()), v.held);
                }
                Err(e) => t.check(&format!("{}: {} parses: {:?}", file, c, e), false),
            }
        }
        let problems: Vec<String> = sim.engine().problems().iter().map(|x| x.msg.text()).collect();
        t.eq(&format!("{}: the simulator finds nothing", file), problems, Vec::<String>::new());
    }
    // the template of the lab has its experiment, and it fits
    let study = Project::from_text(STUDY).unwrap_or_default();
    t.eq("the study's experiment", study.experiments.iter().map(|x| x.problems(&study).len()).collect::<Vec<_>>(), alloc::vec![0]);
}

fn setting(t: &mut T) {
    use crate::app::prefs::Prefs;
    t.check("the page by default", Prefs::new().start_page);
    // a user's file: off, then on again through the settings
    let path = "mem://jfn-selftest/start.ini";
    plat::write_file(path, b"[start]\nshow=0\n");
    let mut q = Prefs::new();
    q.s = base::Settings::open(path, q.s.schema());
    q.pull();
    t.check("the setting off", !q.start_page);
    plat::write_file(path, b"[view]\ngrid=1\n");
    let mut q = Prefs::new();
    q.s = base::Settings::open(path, q.s.schema());
    q.pull();
    t.check("a file from before: on", q.start_page);
    // when it shows
    t.check("a window that opened nothing", at_start(false, false, false, true));
    t.check("a project from the command line", !at_start(true, false, false, true));
    t.check("the setting off keeps it away", !at_start(false, false, false, false));
    t.check("the window scripts do not see it", !at_start(false, true, false, true));
    t.check("unless they ask", at_start(false, true, true, true));
    t.check("and a project goes first there too", !at_start(true, true, true, true));
}

fn recent(t: &mut T) {
    let dir = "mem://jfn-selftest/recent";
    plat::write_file(&format!("{}/da.jfnet", dir), b"{}");
    let list = [format!("{}/da.jfnet", dir), format!("{}/weg.jfnet", dir)];
    let rows = rows_of(&list);
    t.eq("the rows' names", rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), alloc::vec!["da.jfnet", "weg.jfnet"]);
    t.eq("their folder", rows[0].dir.as_str(), dir);
    t.eq("a file that is there, one that is gone", (rows[0].reach, rows[1].reach), (Reach::Here, Reach::Gone));
    t.check("the file's time, none of the other", rows[0].mtime_ns.is_some() && rows[1].mtime_ns.is_none());
    t.eq("no rows without files", rows_of(&[]).len(), 0);
    t.check("as many as the menu", RECENT_MAX == 8);
}

fn times(t: &mut T) {
    let z = Zone::utc();
    let day = 86_400_000_000_000i64;
    let noon = 20_000 * day + 12 * 3_600_000_000_000;
    t.eq("today", when_text(noon + 5 * 60_000_000_000, noon + 3_600_000_000_000, &z), String::from("Heute 12:05"));
    t.eq("yesterday", when_text(noon - day, noon, &z), String::from("Gestern 12:00"));
    t.eq("before that: the date", when_text(noon - 3 * day, noon, &z), String::from("01.10.2024"));
}

fn info(t: &mut T) {
    // in the file, as members with their defaults
    let mut p = Project::new("Büro", 3);
    p.description = String::from("Zeile 1\nZeile 2");
    p.author = String::from("Jo Beispiel");
    let text = p.to_text();
    t.check("written with the file", text.contains("\"author\": \"Jo Beispiel\"") && text.contains("\"description\": \"Zeile 1\\nZeile 2\""));
    t.eq("round trip", Project::from_text(&text), Ok(p.clone()));
    // an old file has no author and stays as it is
    let old = Project::from_text(TWO_PCS);
    t.check("an old file loads", old.as_ref().is_ok_and(|o| o.author.is_empty() && !o.description.is_empty() && o.devices.len() == 3));
    t.check("the old fixture has no author member", !TWO_PCS.contains("\"author\""));
    // an undo step that goes both ways; nothing changed, no step
    let mut q = Project::new("", 1);
    t.check("no change, no step", crate::doc::edits::info(&q, "", "  ").is_none());
    if let Some(mut e) = crate::doc::edits::info(&q, "Worum es geht\n", " Jo ") {
        e.apply(&mut q);
        t.eq("applied, trimmed", (q.description.as_str(), q.author.as_str()), ("Worum es geht", "Jo"));
        e.revert(&mut q);
        t.eq("undone", (q.description.as_str(), q.author.as_str()), ("", ""));
    } else {
        t.check("a step", false);
    }
}

// the page's project: after the first tick the simulator asks for no timer, nothing wakes the window
fn idle(t: &mut T) {
    let p = Project::new("", 1);
    let mut sim = Sim::new(&p);
    sim.start();
    t.check("the first tick", sim.arm().is_some());
    sim.tick();
    t.eq("an empty project arms nothing", sim.arm(), None);
    t.eq("still nothing", sim.arm(), None);
}

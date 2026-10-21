// uitest -- the selftest's group "ui": what U5 adds around the window without a window
//
//   contrast     the colours of the simulation's categories and of the cables against the
//                canvas of the light and the dark theme and of high contrast (white on
//                black, black on white; WCAG 3:1 for graphical objects)
//   prefs        the settings: defaults, a user's file (clamped values, unknown choices), the
//                typed values, the recent files beside them
//   autosave     copies of other sessions: newest first, a running session's (locked) copy
//                is never offered, a recovered or dropped copy goes
//   catalogue    every label of the command table, the menus, the settings page and the
//                tables marked "// i18n" has an English row; English differs where it should

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::consts::T_BG;
use ui::i18n::Catalog;
use ui::{contrast_ratio, Mi, Style};

use crate::selftest::T;

pub(crate) fn run(t: &mut T) {
    contrast(t);
    prefs(t);
    autosave(t);
    catalogue(t);
}

// the canvas colour of a theme of the linked style (netlab)
fn canvas_bg(dark: bool) -> u32 {
    style::STYLE.tokens(dark)[(T_BG / 4) as usize] | 0xFF00_0000
}

fn contrast(t: &mut T) {
    use crate::workspace::sim::{cat_color, CATS};
    // the themes, then high contrast without the system's scheme (white on black, black on
    // white: the canvas takes the scheme's background, the colours follow its brightness)
    let themes = [
        (false, canvas_bg(false), "hell"),
        (true, canvas_bg(true), "dunkel"),
        (true, 0xFF00_0000 | plat::HcColors::BLACK.bg, "kontrast schwarz"),
        (false, 0xFF00_0000 | plat::HcColors::WHITE.bg, "kontrast weiß"),
    ];
    for (dark, bg, theme) in themes {
        for (k, c) in CATS.iter().enumerate() {
            let fill = cat_color(k, dark);
            let r = contrast_ratio(fill, bg);
            t.check(&format!("contrast {} {}: {:.2} >= 3", theme, c.name, r), r >= 3.0);
        }
        for &k in jfn_model::CableKind::ALL.iter() {
            let c = crate::workspace::canvas::paint::cable_color(k, dark, false, bg);
            let r = contrast_ratio(c, bg);
            t.check(&format!("contrast {} cable {:?}: {:.2} >= 3", theme, k, r), r >= 3.0);
        }
    }
    // the reference values of WCAG: black on white 21, grey #767676 on white 4.54
    t.check("contrast black on white", (contrast_ratio(0x000000, 0xFFFFFF) - 21.0).abs() < 0.01);
    t.check("contrast #767676 on white", (contrast_ratio(0x767676, 0xFFFFFF) - 4.54).abs() < 0.02);
}

fn prefs(t: &mut T) {
    use crate::app::prefs::{Lang, Motion, Prefs};
    let p = Prefs::new();
    t.eq("prefs default grid", (p.grid, p.snap, p.minimap, p.grid_step), (true, true, false, 20));
    t.eq("prefs default tempo, autosave", (p.tempo, p.autosave_s), (2, 120));
    t.check("prefs default motion, language", p.motion == Motion::System && p.lang == Lang::System);
    t.check("prefs not kept (memory)", !p.kept);
    // a user's file: the old 0/1 toggles, a step out of range, a choice by its word, an unknown
    // one, the recent files beside the schema
    let path = "mem://jfn-selftest/settings.ini";
    plat::write_file(path, b"[view]\ngrid=0\nsnap=1\nminimap=1\ngrid_step=500\nmotion=reduced\n[app]\nlang=klingonisch\n[sim]\ntempo=4\n[files]\nautosave=0\nrecent=a.jfnet|b.jfnet\n");
    let mut q = Prefs::new();
    q.s = base::Settings::open(path, q.s.schema());
    q.pull();
    t.eq("prefs file toggles", (q.grid, q.snap, q.minimap), (false, true, true));
    t.eq("prefs file step clamped", q.grid_step, 200);
    t.check("prefs file motion", q.motion == Motion::Reduced);
    t.check("prefs file unknown language", q.lang == Lang::System);
    t.eq("prefs file tempo 4x", q.tempo, 4);
    t.eq("prefs file autosave off", q.autosave_s, 0);
    t.eq("prefs file recent", q.recent(), alloc::vec![String::from("a.jfnet"), String::from("b.jfnet")]);
    // the toggles go back into the settings
    q.grid = true;
    q.push();
    q.pull();
    t.check("prefs push", q.grid);
}

fn autosave(t: &mut T) {
    use crate::doc::autosave::orphans;
    let dir = "mem://jfn-selftest/autosave";
    let copy = |id: &str, name: &str, at: u64| {
        plat::write_file(&format!("{}/{}.jfnet", dir, id), b"{}");
        let ini = format!("[autosave]\npath=\nname={}\nat={}\nrev=7\n", name, at);
        plat::write_file(&format!("{}/{}.ini", dir, id), ini.as_bytes());
    };
    t.check("autosave nothing", orphans(dir).is_empty());
    copy("aaaa", "Alt.jfnet", 5);
    copy("bbbb", "Neu.jfnet", 9);
    copy("cccc", "Läuft.jfnet", 12);
    // a running session holds its lock: its copy is not offered
    let held = plat::FileLock::acquire(&format!("{}/cccc.lock", dir));
    t.check("autosave lock taken", held.is_some());
    let o = orphans(dir);
    let names: Vec<&str> = o.iter().map(|x| x.name.as_str()).collect();
    t.eq("autosave newest first, running skipped", names, alloc::vec!["Neu.jfnet", "Alt.jfnet"]);
    t.check("autosave no path", o.iter().all(|x| x.path.is_none()));
    drop(held);
    t.eq("autosave lock released", orphans(dir).len(), 3);
    // Wiederherstellen adopts a copy: held (no other window offers it) until it is released
    let mut a = crate::doc::autosave::Autosave::new();
    a.dir = String::from(dir);
    a.adopt("bbbb");
    let ids: Vec<String> = orphans(dir).into_iter().map(|x| x.id).collect();
    t.eq("autosave adopted copy held", ids, alloc::vec![String::from("cccc"), String::from("aaaa")]);
    a.release();
    t.check("autosave adopted copy removed", plat::read_file(&format!("{}/bbbb.jfnet", dir)).is_none());
    t.check("autosave adopted ini removed", plat::read_file(&format!("{}/bbbb.ini", dir)).is_none());
    t.eq("autosave after release", orphans(dir).len(), 2);
    for id in ["aaaa", "bbbb", "cccc"] {
        for ext in ["jfnet", "ini", "lock"] {
            plat::delete(&format!("{}/{}.{}", dir, id, ext));
        }
    }
    t.check("autosave cleaned", orphans(dir).is_empty());
}

// the labels of a menu (recursive)
fn menu_labels(m: &[Mi], out: &mut Vec<&'static str>) {
    for it in m {
        match *it {
            Mi::Menu(l, sub) | Mi::Sub(l, sub) => {
                out.push(l);
                menu_labels(sub, out);
            }
            Mi::Item(_, l, _) | Mi::List(_, _, l) => out.push(l),
            _ => {}
        }
    }
}

fn catalogue(t: &mut T) {
    let ui_tsv: &'static str = include_str!("locales/ui.tsv");
    let cat = Catalog::from_tsv(ui_tsv);
    t.eq("catalogue languages", cat.languages().to_vec(), alloc::vec![String::from("de"), String::from("en")]);
    let mut labels: Vec<&'static str> = crate::app::commands::CMDS.iter().map(|c| c.label).collect();
    menu_labels(&crate::app::commands::MENUBAR, &mut labels);
    for m in [&crate::app::commands::M_CANVAS[..], &crate::app::commands::M_DEVICE[..], &crate::app::commands::M_CABLE[..]] {
        menu_labels(m, &mut labels);
    }
    // the settings page: sections, labels, help texts, choices
    let schema = crate::app::prefs::Prefs::new().s.schema();
    for (_, title) in schema.sections {
        labels.push(title);
    }
    for d in schema.keys {
        labels.push(d.label);
        if !d.help.is_empty() {
            labels.push(d.help);
        }
        if let base::settings::Kind::Choice { items, .. } = d.kind {
            labels.extend(items.iter().map(|it| it.1));
        }
    }
    // texts that stay as they are ("1×", "Terminal") need a row too: a missing row is a gap
    let mut missing = 0;
    for l in &labels {
        if !l.chars().any(|c| c.is_alphabetic()) {
            continue;
        }
        if !cat.has(l) {
            missing += 1;
            plat::write_stdout(format!("  no English: {:?}\n", l).as_bytes());
        }
    }
    t.eq("catalogue: every label of commands, menus and settings has a row", missing, 0);
    // the placeholders agree
    let mut bad = 0;
    for line in ui_tsv.lines().skip(1) {
        let Some((de, en)) = line.split_once('\t') else { continue };
        if line.starts_with('#') {
            continue;
        }
        if de.matches("{}").count() != en.matches("{}").count() {
            bad += 1;
            plat::write_stdout(format!("  placeholders differ: {:?}\n", de).as_bytes());
        }
    }
    t.eq("catalogue: the same placeholders", bad, 0);
}

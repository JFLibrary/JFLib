// sec_stencil -- the Shapes window (ui::stencil_panel, lib/ui/stencil.rs): test hook 9106
// (--stencils) shows only it with four stencils -- "Allgemein" and "Flussdiagramm" with quick
// shapes, the editable "Favoriten", "Viele Shapes" (300 tiles, with the check box) -- and lines
// that show what it reported: the last event, the active title, the favourites in their order,
// what was dropped onto the page.  A click arms a tile (TL_ARMED; once more: disarmed), right
// clicks open small menus (a tile: to / from the favourites; a title: the view, close), "Weitere
// Shapes" a menu with a submenu at the button (StencilEvent::More).  The search matches the names
// (any case).  apps/gallery/tests/stencil*.uis drive it.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;
use ui::i18n::{t, tf};

use crate::Gal;

const CMD_STENCILS: u32 = 9106; // test hook: the Shapes window on / off
const ID_SH: u32 = 0x0006_0000; // the panel (its parts up to + 0x1000 + tiles)
const ID_SH_MORE: u32 = 7500; // "Weitere Shapes"
const ID_SH_EMPTY: u32 = 7501; // check box: no stencils
const ID_SH_MANY: u32 = 7502; // check box: the stencil of 300 tiles
const ID_SH_VIEW: u32 = 7503; // radios 7503 .. 7506: the view modes
const ID_SH_PAGE: u32 = 7510; // the page: takes tile drags
const CMD_SH_FAV_ADD: u32 = 7520; // tile menu
const CMD_SH_FAV_DEL: u32 = 7521;
const CMD_SH_VIEW: u32 = 7522; // .. 7525: title menu "Ansicht"
const CMD_SH_CLOSE: u32 = 7526; // title menu: close the stencil
const CMD_SH_OPEN: u32 = 7527; // "Weitere Shapes" menu
const CMD_SH_MINE: u32 = 7528;
const CMD_SH_NEW: u32 = 7529;
const DT_SHAPE: u32 = DT_USER0; // a tile's drag
const K_GENERAL: u32 = 1;
const K_FLOW: u32 = 2;
const K_FAV: u32 = 3;
const K_MANY: u32 = 4;
const VIEWS: [&str; 4] = ["Symbole und Namen", "Nur Symbole", "Nur Namen", "Symbole und Details"]; // i18n

static M_VIEW: [Mi; 4] = [
    Mi::Item(CMD_SH_VIEW, "Symbole und Namen", 0),
    Mi::Item(CMD_SH_VIEW + 1, "Nur Symbole", 0),
    Mi::Item(CMD_SH_VIEW + 2, "Nur Namen", 0),
    Mi::Item(CMD_SH_VIEW + 3, "Symbole und Details", 0),
];
static M_STENCIL: [Mi; 3] = [Mi::Sub("Ansicht", &M_VIEW), Mi::Sep, Mi::Item(CMD_SH_CLOSE, "Schablone schließen", 0)];
static M_TILE: [Mi; 2] = [Mi::Item(CMD_SH_FAV_ADD, "Zu Favoriten hinzufügen", IC_STAR), Mi::Item(CMD_SH_FAV_DEL, "Aus Favoriten entfernen", 0)];
static M_CATS: [Mi; 1] = [Mi::Item(CMD_SH_OPEN, "Alle Schablonen öffnen", 0)];
static M_MORE: [Mi; 4] = [
    Mi::Sub("Kategorien", &M_CATS),
    Mi::Sep,
    Mi::Item(CMD_SH_MINE, "Meine Shapes", 0),
    Mi::Item(CMD_SH_NEW, "Neue Schablone", 0),
];

// a preview: a few document shapes in fixed colours (they do not follow the theme)
#[derive(Clone, Copy)]
enum Kind {
    Rect,
    Round,
    Ellipse,
    Circle,
    Triangle,
    Diamond,
    Star,
    Arrow,
    Data,
    Document,
    Cylinder,
}

// name, QuickInfo, preview, flags
type Def = (&'static str, &'static str, Option<Kind>, u32);

const GENERAL: [Def; 9] = [
    ("Rechteck", "Ein Rechteck mit geraden Ecken", Some(Kind::Rect), TL_QUICK), // i18n
    ("Abgerundetes Rechteck", "Ein Rechteck mit runden Ecken", Some(Kind::Round), TL_QUICK), // i18n
    ("Ellipse", "Eine Ellipse", Some(Kind::Ellipse), TL_QUICK), // i18n
    ("Kreis", "Ein Kreis", Some(Kind::Circle), 0), // i18n
    ("Dreieck", "Ein gleichschenkliges Dreieck", Some(Kind::Triangle), 0), // i18n
    ("Raute", "Eine auf die Spitze gestellte Raute", Some(Kind::Diamond), 0), // i18n
    ("Fünfzackiger Stern", "Ein Stern mit fünf Zacken", Some(Kind::Star), 0), // i18n
    ("Pfeil nach rechts", "Ein Blockpfeil", Some(Kind::Arrow), 0), // i18n
    ("Textfeld", "Ein Feld für Text, ohne Vorschau", None, 0), // i18n
];
const FLOW: [Def; 7] = [
    ("Prozess", "Ein Schritt im Ablauf", Some(Kind::Rect), TL_QUICK), // i18n
    ("Entscheidung", "Eine Verzweigung mit Ja und Nein", Some(Kind::Diamond), TL_QUICK), // i18n
    ("Start/Ende", "Anfang oder Ende des Ablaufs", Some(Kind::Round), TL_QUICK), // i18n
    ("Daten", "Ein- oder Ausgabe von Daten", Some(Kind::Data), 0), // i18n
    ("Dokument", "Ein Dokument oder Bericht", Some(Kind::Document), 0), // i18n
    ("Datenbank", "Gespeicherte Daten", Some(Kind::Cylinder), 0), // i18n
    ("Externe Daten", "Nicht verfügbar", Some(Kind::Data), TL_DISABLED), // i18n
];

struct Preview(Kind);

const FILL: u32 = 0xFFDCE8F8;
const INK: u32 = 0xFF2F6BB3;

// the outline of a polygon (points in 0..1 of r), filled and stroked
fn poly(g: &mut Gfx, r: [i32; 4], pts: &[(f32, f32)], t: i32) {
    let (x, y, w, h) = (r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32);
    let sh = g.shapes();
    sh.path.begin(sh.dc);
    sh.path.move_f(x + pts[0].0 * w, y + pts[0].1 * h);
    for p in &pts[1..] {
        sh.path.line_f(x + p.0 * w, y + p.1 * h);
    }
    sh.path.close();
    sh.dc.set_color(FILL);
    sh.path.fill(sh.ras, sh.dc, sh.sf, 0);
    sh.dc.set_color(INK);
    sh.path.stroke(sh.ras, sh.dc, sh.sf, t, 0);
}

impl Draw for Preview {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        // a square of the shorter side in the middle, 2 px margin
        let s = (r[2].min(r[3]) - 4).max(4);
        let q = [r[0] + (r[2] - s) / 2, r[1] + (r[3] - s) / 2, s, s];
        let wide = [q[0], q[1] + s / 6, s, s - s / 3];
        let t = (s * 256 / 24).max(256); // the stroke: 24.8
        let px = (s / 24).max(1);
        match self.0 {
            Kind::Rect | Kind::Round => {
                let rad = if matches!(self.0, Kind::Round) { s / 5 } else { 0 };
                g.dc.set_color(FILL);
                g.shapes().fill_rrect(wide[0], wide[1], wide[2], wide[3], rad);
                g.dc.set_color(INK);
                g.shapes().stroke_rrect(wide[0], wide[1], wide[2], wide[3], rad, px);
            }
            Kind::Ellipse | Kind::Circle => {
                let b = if matches!(self.0, Kind::Circle) { q } else { wide };
                let (cx, cy, rx, ry) = ((b[0] * 2 + b[2]) << 7, (b[1] * 2 + b[3]) << 7, b[2] << 7, b[3] << 7);
                let sh = g.shapes();
                sh.path.begin(sh.dc);
                sh.path.ellipse_fx(cx, cy, rx - t / 2, ry - t / 2);
                sh.dc.set_color(FILL);
                sh.path.fill(sh.ras, sh.dc, sh.sf, 0);
                sh.dc.set_color(INK);
                sh.path.stroke(sh.ras, sh.dc, sh.sf, t, 0);
            }
            Kind::Triangle => poly(g, q, &[(0.5, 0.08), (0.94, 0.9), (0.06, 0.9)], t),
            Kind::Diamond => poly(g, q, &[(0.5, 0.05), (0.95, 0.5), (0.5, 0.95), (0.05, 0.5)], t),
            Kind::Star => {
                let p = [
                    (0.0, -1.0), (0.2245, -0.309), (0.9511, -0.309), (0.3633, 0.118), (0.5878, 0.809),
                    (0.0, 0.382), (-0.5878, 0.809), (-0.3633, 0.118), (-0.9511, -0.309), (-0.2245, -0.309),
                ];
                let pts: Vec<(f32, f32)> = p.iter().map(|&(x, y)| (0.5 + x * 0.46, 0.54 + y * 0.46)).collect();
                poly(g, q, &pts, t);
            }
            Kind::Arrow => poly(g, q, &[(0.05, 0.35), (0.6, 0.35), (0.6, 0.15), (0.95, 0.5), (0.6, 0.85), (0.6, 0.65), (0.05, 0.65)], t),
            Kind::Data => poly(g, wide, &[(0.22, 0.05), (0.98, 0.05), (0.78, 0.95), (0.02, 0.95)], t),
            Kind::Document => poly(g, q, &[(0.12, 0.1), (0.88, 0.1), (0.88, 0.82), (0.66, 0.74), (0.4, 0.9), (0.12, 0.82)], t),
            Kind::Cylinder => {
                poly(g, q, &[(0.15, 0.2), (0.85, 0.2), (0.85, 0.82), (0.5, 0.92), (0.15, 0.82)], t);
                let sh = g.shapes();
                let (cx, cy) = ((q[0] * 2 + q[2]) << 7, (q[1] << 8) + q[3] * 52);
                sh.path.begin(sh.dc);
                sh.path.ellipse_fx(cx, cy, q[2] * 90, q[3] * 30);
                sh.dc.set_color(FILL);
                sh.path.fill(sh.ras, sh.dc, sh.sf, 0);
                sh.dc.set_color(INK);
                sh.path.stroke(sh.ras, sh.dc, sh.sf, t, 0);
            }
        }
    }
}

/// the section's state
#[derive(Default)]
pub(crate) struct S {
    /// the Shapes window is shown (test hook 9106, --stencils)
    pub(crate) on: bool,
    arg: bool,
    st: StencilState,
    empty: i32,
    many: i32,
    /// the favourites: (stencil, tile) of the tile they copy, in quick shapes
    fav: Vec<(u32, u32, bool)>,
    /// the closed stencils
    closed: Vec<u32>,
    armed: Option<(u32, u32)>,
    /// the tile / title whose menu is open
    menu_tile: Option<(u32, u32)>,
    menu_stencil: u32,
    dragging: bool,
    last: String,
    /// the last StencilEvent::Dropped
    dropped: String,
    page: String,
    many_names: Vec<String>,
    previews: Vec<DrawRef>,
}

/// the Shapes window is on (the frame shows only it)
pub(crate) fn active(g: &mut Gal) -> bool {
    let s = &mut g.stencil;
    if !s.arg {
        s.arg = true;
        s.on = plat::args().iter().skip(1).any(|a| a == "--stencils");
    }
    s.on
}

// the preview drawings, one per kind in Kind's order (made once)
fn previews_make(s: &mut S) {
    if s.previews.is_empty() {
        let all = [
            Kind::Rect, Kind::Round, Kind::Ellipse, Kind::Circle, Kind::Triangle, Kind::Diamond, Kind::Star, Kind::Arrow, Kind::Data, Kind::Document,
            Kind::Cylinder,
        ];
        s.previews = all.iter().map(|&k| -> DrawRef { Rc::new(Preview(k)) }).collect();
    }
}

// the name of a tile of the demo's stencils
fn name_of(s: &S, stencil: u32, tile: u32) -> String {
    let def = |d: &[Def], k: u32| d.get(k as usize).map_or("?", |d| t(d.0));
    String::from(match stencil {
        K_GENERAL => def(&GENERAL, tile),
        K_FLOW => def(&FLOW, tile),
        K_FAV => return name_of(s, tile >> 16, tile & 0xFFFF),
        K_MANY => s.many_names.get(tile as usize).map_or("?", |n| n.as_str()),
        _ => "?",
    })
}

/// the panel and the lines beside it
pub(crate) fn frame(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.stencil;
    if s.many_names.is_empty() {
        s.many_names = (1..=300).map(|k| tf("Shape {}", &[&format!("{}", k)])).collect();
    }
    let armed = s.armed;
    let flags = |sk: u32, k: u32, f: u32| if armed == Some((sk, k)) { f | TL_ARMED } else { f };
    // (the tiles are built every frame, as an app's model would hand them in)
    previews_make(s);
    let prev = s.previews.clone();
    let tiles_of = |d: &[Def], sk: u32| -> Vec<StencilTile<'static>> {
        d.iter()
            .enumerate()
            .map(|(k, d)| StencilTile::new(k as u32, t(d.0)).tip(t(d.1)).preview(d.2.map(|p| prev[p as usize].clone()), 1).flags(flags(sk, k as u32, d.3)))
            .collect()
    };
    let general = tiles_of(&GENERAL, K_GENERAL);
    let flow = tiles_of(&FLOW, K_FLOW);
    let find = |sk: u32, k: u32| -> Option<&Def> {
        match sk {
            K_GENERAL => GENERAL.get(k as usize),
            K_FLOW => FLOW.get(k as usize),
            _ => None,
        }
    };
    let fav: Vec<StencilTile<'static>> = s
        .fav
        .iter()
        .filter_map(|&(sk, k, q)| {
            let d = find(sk, k)?;
            let key = sk << 16 | k;
            Some(StencilTile::new(key, t(d.0)).tip(t(d.1)).preview(d.2.map(|p| prev[p as usize].clone()), 1).flags(flags(K_FAV, key, if q { TL_QUICK } else { 0 })))
        })
        .collect();
    let many: Vec<StencilTile<'_>> = s
        .many_names
        .iter()
        .enumerate()
        .map(|(k, n)| StencilTile::new(k as u32, n).preview(Some(prev[k % prev.len()].clone()), 1).flags(flags(K_MANY, k as u32, 0)))
        .collect();
    let mut defs: Vec<StencilDef<'_>> = Vec::new();
    if s.empty == 0 {
        for (key, title, tiles, f) in [
            (K_GENERAL, t("Allgemein"), &general[..], 0),
            (K_FLOW, t("Flussdiagramm"), &flow[..], 0),
            (K_FAV, t("Favoriten"), &fav[..], SD_EDITABLE),
            (K_MANY, t("Viele Shapes"), &many[..], 0),
        ] {
            if (key != K_MANY || s.many != 0) && !s.closed.contains(&key) {
                defs.push(StencilDef { key, title, tiles, flags: f });
            }
        }
    }
    // the search: names that contain the query (any case)
    let q = s.st.query.trim().to_lowercase();
    let hits: Vec<StencilHit<'_>> = if q.is_empty() {
        Vec::new()
    } else {
        defs.iter().flat_map(|d| d.tiles.iter().filter(|t| t.name.to_lowercase().contains(&q)).map(move |t| StencilHit { stencil: d.key, tile: t.clone() })).collect()
    };
    let panel = StencilPanel {
        stencils: &defs,
        results: (!q.is_empty()).then_some(&hits[..]),
        drag_kind: DT_SHAPE,
        more_id: ID_SH_MORE,
        labels: StencilLabels {
            panel: t("Shapes"),
            search: t("Shapes suchen"),
            history: t("Letzte Suchen"),
            more: t("Weitere Shapes"),
            quick: t("Quick-Shapes"),
            results: t("Suchergebnisse ({})"),
            quick_empty: t("Hier Quick-Shapes ablegen"),
            drop_hint: t("Shapes hierher ziehen"),
            empty_title: t("Keine Schablone geöffnet"),
            empty_text: t("Öffnen Sie eine Schablone mit „Weitere Shapes“."),
            no_results_title: t("Keine Shapes gefunden"),
            no_results_text: t("Die Suche prüft die Namen aller offenen Schablonen."),
        },
    };
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 0);
    ui.next_size(260, -1);
    let events = ui.stencil_panel(ID_SH, &mut s.st, &panel);
    let title_of = |key: u32| -> String {
        match key {
            SK_QUICK => String::from(t("Quick-Shapes")),
            SK_SEARCH => String::from(t("Suchergebnisse")),
            _ => defs.iter().find(|d| d.key == key).map_or(String::from("?"), |d| String::from(d.title)),
        }
    };
    let active = title_of(s.st.active);
    // what the panel reported
    for e in events {
        let line = match e {
            StencilEvent::Activate { stencil, tile } => {
                s.armed = if s.armed == Some((stencil, tile)) { None } else { Some((stencil, tile)) };
                tf("Aktivieren: {}", &[&name_of(s, stencil, tile)])
            }
            StencilEvent::Place { stencil, tile } => tf("Platzieren: {}", &[&name_of(s, stencil, tile)]),
            StencilEvent::TileMenu { stencil, tile, at } => {
                s.menu_tile = Some((stencil, tile));
                ui.menu_popup_at(i18n::menu(&M_TILE), at[0], at[1]);
                tf("Menü: {}", &[&name_of(s, stencil, tile)])
            }
            StencilEvent::StencilMenu { stencil, at } => {
                s.menu_stencil = stencil;
                ui.menu_popup_at(i18n::menu(&M_STENCIL), at[0], at[1]);
                tf("Schablonenmenü: {}", &[&title_of(stencil)])
            }
            StencilEvent::Search => tf("Suche: {}", &[&s.st.query]),
            StencilEvent::Dropped { stencil: _, before, quick, from_stencil, from_tile } => {
                let name = name_of(s, from_stencil, from_tile);
                // a favourite moves, another tile is copied in (once)
                let src = if from_stencil == K_FAV { (from_tile >> 16, from_tile & 0xFFFF) } else { (from_stencil, from_tile) };
                let mut at = before.min(s.fav.len());
                if let Some(i) = s.fav.iter().position(|f| (f.0, f.1) == src) {
                    s.fav.remove(i);
                    if i < at {
                        at -= 1;
                    }
                }
                s.fav.insert(at, (src.0, src.1, quick));
                s.dropped = tf("{} an {}", &[&name, &format!("{}{}", before, if quick { " (Quick)" } else { "" })]);
                tf("Abgelegt: {}", &[&s.dropped])
            }
            StencilEvent::More { at } => {
                ui.menu_popup_at(i18n::menu(&M_MORE), at[0], at[1]);
                String::from(t("Weitere Shapes"))
            }
            StencilEvent::DragStart { stencil, tile } => {
                s.dragging = true;
                tf("Ziehen: {}", &[&name_of(s, stencil, tile)])
            }
            StencilEvent::DragEnd => {
                s.dragging = false;
                String::from(t("Ziehen beendet"))
            }
        };
        s.last = line;
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | PAD_16, 8);
    ui.text(TS_SUBTITLE, t("Shapes"));
    ui.text(TS_BODY, &tf("Ereignis: {}", &[if s.last.is_empty() { "–" } else { &s.last }]));
    ui.text(TS_BODY, &tf("Aktiv: {}", &[&active]));
    let favs: Vec<String> = s.fav.iter().map(|&(sk, k, q)| format!("{}{}", name_of(s, sk, k), if q { "*" } else { "" })).collect();
    ui.text(TS_BODY, &tf("Favoriten: {}", &[&favs.join(", ")]));
    ui.text(TS_BODY, &tf("Abgelegt: {}", &[if s.dropped.is_empty() { "–" } else { &s.dropped }]));
    ui.text(TS_BODY, &tf("Ziehen: {}", &[if s.dragging { t("ja") } else { t("nein") }]));
    ui.text(TS_BODY, &tf("Zeichenblatt: {}", &[if s.page.is_empty() { "–" } else { &s.page }]));
    let mut view = s.st.view as i32;
    for (k, &l) in VIEWS.iter().enumerate() {
        ui.radio(ID_SH_VIEW + k as u32, t(l), &mut view, k as i32);
    }
    s.st.view = view as u8;
    ui.checkbox(ID_SH_EMPTY, t("Ohne Schablonen"), &mut s.empty);
    ui.checkbox(ID_SH_MANY, t("Viele Shapes"), &mut s.many);
    // the page: takes tiles
    ui.next_size(-1, -1);
    ui.box_begin(ID_SH_PAGE, BOX_V | BOX_CARD | PAD_16 | BOX_CENTER | BOX_XCENTER, 0);
    if ui.drop_target(ID_SH_PAGE, DT_SHAPE) & DS_DROP != 0 {
        if let Some((sk, k)) = stencil_drag_decode(&ui.drop_data().bytes) {
            s.page = name_of(s, sk, k);
        }
    }
    ui.text(TS_BODY | TX_SECONDARY, t("Zeichenblatt"));
    ui.box_end();
    ui.box_end();
    ui.box_end();
}

/// the test hook and the menus' rows -> handled
pub(crate) fn cmd(g: &mut Gal, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
    let s = &mut g.stencil;
    match id {
        CMD_STENCILS => s.on = !s.on,
        CMD_SH_FAV_ADD => {
            if let Some((sk, k)) = s.menu_tile {
                let src = if sk == K_FAV { (k >> 16, k & 0xFFFF) } else { (sk, k) };
                if !s.fav.iter().any(|f| (f.0, f.1) == src) {
                    s.fav.push((src.0, src.1, false));
                }
            }
        }
        CMD_SH_FAV_DEL => {
            if let Some((sk, k)) = s.menu_tile {
                let src = if sk == K_FAV { (k >> 16, k & 0xFFFF) } else { (sk, k) };
                s.fav.retain(|f| (f.0, f.1) != src);
            }
        }
        _ if (CMD_SH_VIEW..CMD_SH_VIEW + 4).contains(&id) => s.st.view = (id - CMD_SH_VIEW) as u8,
        CMD_SH_CLOSE => {
            if s.menu_stencil < SK_QUICK - 1 && !s.closed.contains(&s.menu_stencil) {
                s.closed.push(s.menu_stencil);
            }
        }
        CMD_SH_OPEN => {
            s.closed.clear();
            s.empty = 0;
        }
        CMD_SH_MINE | CMD_SH_NEW => s.last = String::from(if id == CMD_SH_MINE { t("Meine Shapes") } else { t("Neue Schablone") }),
        _ => return false,
    }
    true
}

// desktop -- the Desktop tab of a host's device window: a small PC -- the programs in windows of
// their own inside the tab (lib/ui desk.rs: moving, resizing, maximising, minimising, snapping,
// Alt+F6 ...), icons on a background of our own, a taskbar with Start and its menu, a button per
// open window, full screen and the device's clock
//
//   Desk::new(dev)                    a device's desktop with no program open.  DevWins keeps it
//                                     per device for the session (devwin/mod.rs): closing and
//                                     opening the device window again brings it back as it was;
//                                     a new or loaded project starts over.  It is no part of the
//                                     project, of its undo steps or of the simulator.
//   tab(n, ui, k, id, dev)            the desktop of device window k (id: the window's desktop
//                                     block, where IP-Konfiguration and Webbrowser keep their ids)
//   Desk::open_prog(p)                a program's window opens, or comes to the front
//   Desk::console_at                  the window that draws the device's console now: the
//                                     Terminal or the MIB-Browser, whichever is higher and not
//                                     minimised (devwin::shows: the Terminal panel says so)
//   console_or_note(n, ui, k, dev, p) the console in program p's window, or a note where it is
//   terminal_window(n, ui, k, dev, s) a Terminal window's content: the device's console (None,
//                                     the program Terminal) or a further session of its own
//                                     (Some, "Weiteres Terminal": console::view_session)
//   ide_window(n, ui, k, dev)         the IDE's window: the IDE of workspace/ide (ide::view) with
//                                     the window's box; its Terminal pane is a command-line
//                                     session of its own (Desk::ide_term, ended with the window)
//   open_ide(n, ui, dev)              the device's window on its desktop with the IDE in front
//                                     (CMD_SCRIPTS, the context menu's "IDE …")
//   open_with(n, k, dev, path)        a file of the device in its program: .jfs files in the
//                                     IDE (open_jfs(n, k, path)), the others in the Texteditor
//   bg_colors(bg, dark) / BG_LABEL    the backgrounds and their labels' colours (selftest desk)
//
// The programs (Prog): Terminal, Dateimanager (filemgr.rs), Texteditor (editor.rs),
// IP-Konfiguration (progs.rs), Webbrowser (browser.rs), E-Mail (email.rs), MIB-Browser (mib.rs),
// Systemmonitor (sysmon.rs), Einstellungen (here), IDE.  Each has one window (key: its index + 1)
// with a first and a smallest size of its own.  The Terminal shows the device's console; "Weiteres
// Terminal" in the start menu opens one more Terminal window with a command-line session of its
// own (jfn_sim cli/ses.rs: Linux "pts/N", IOS-like "tty N"; key TERM_KEY0 + the session, title
// "Terminal (2)"), closing the window ends the session.  A new window opens right of the icons, cascaded,
// as large as fits (IDE, E-Mail and MIB-Browser start maximised, the device window being a small
// screen).  The icons stand in columns from the top left: a click chooses one, a double click
// or Enter opens it; they are one Tab stop (the arrows move between them, the choice follows)
// and a list "Desktop" for screen readers.  Start opens the programs as a menu in groups
// (Netzwerk, System, Entwicklung).  The taskbar: Start, a button per window (desk_taskbar),
// Vollbild (the device window's full screen, also F11) and the clock (the device's clock in the
// simulation, as Einstellungen says: hours and minutes, with seconds or with the date).
// Einstellungen: the background (Schiefer, Salbei, Sand, Nachtblau, Punktraster, Netz), the
// icons' size and the clock -- the user's settings (app/prefs.rs, section Desktop, the same in
// Werkzeuge › Einstellungen) for every host's desktop; high contrast shows the window's own
// background without a pattern.
//
// Ids: ID_DESK0 + 0x1000 k (app/constants.rs: + 0 the area, + 1 Start, + 2 its menu, + 3
// Vollbild, + 0x10 + p the menu's rows, + 0x20 the icons' group, + 0x30 + p the icons, + 0x40 the
// background, + 0x600 Einstellungen, + 0x700 IDE, + 0x710 Terminal, + 0x720 the MIB-Browser's
// note); IP-Konfiguration id + 0x10, Webbrowser id + 0x30 (id: devwin base + 0x80; its page
// ID_BROWSER0 + 0x100 k), E-Mail ID_MAIL0 + 0x400 k, MIB-Browser ID_MIB0 + 0x400 k.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::constants::*;
use crate::app::tr::t;
use crate::app::Net;
use crate::doc::console;
use crate::workspace::icons::*;

/// the taskbar's height (DIP), the strip the window area keeps free
const TASK_H: i32 = 40;
/// the window key of a further Terminal window: TERM_KEY0 + its session (the programs' are 1 ..)
const TERM_KEY0: u32 = 0x100;

/// the programs of the desktop
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Prog {
    Terminal,
    Files,
    Editor,
    IpConfig,
    Browser,
    Mail,
    Mib,
    Monitor,
    Settings,
    Ide,
}

/// the programs: name, what it does (tooltip), group (0 Netzwerk, 1 System, 2 Entwicklung),
/// first and smallest content size (DIP)
// i18n
const PROGS: [(Prog, &str, &str, usize, [i32; 2], [i32; 2]); 10] = [
    (Prog::Terminal, "Terminal", "Die Shell des Geräts", 1, [560, 300], [300, 160]),
    (Prog::Files, "Dateimanager", "Ordner und Dateien des Geräts", 1, [640, 360], [420, 220]),
    (Prog::Editor, "Texteditor", "Notizen und Dateien des Geräts bearbeiten", 2, [540, 340], [320, 200]),
    (Prog::IpConfig, "IP-Konfiguration", "Adresse, Maske und Gateway oder DHCP", 0, [460, 380], [340, 220]),
    (Prog::Browser, "Webbrowser", "Seiten aus dem simulierten Netz", 0, [640, 420], [360, 240]),
    (Prog::Mail, "E-Mail", "Nachrichten senden und abrufen", 0, [680, 460], [420, 300]),
    (Prog::Mib, "MIB-Browser", "SNMP-Objekte eines Agenten lesen und setzen", 0, [720, 480], [460, 320]),
    (Prog::Monitor, "Systemmonitor", "Prozesse, Dienste, CPU und Speicher", 1, [580, 380], [380, 260]),
    (Prog::Settings, "Einstellungen", "Hintergrund, Symbole und Uhr des Desktops", 1, [440, 400], [320, 240]),
    (Prog::Ide, "IDE", "Skripte des Geräts entwickeln", 2, [640, 400], [360, 220]),
];
// i18n
const GROUPS: [&str; 3] = ["Netzwerk", "System", "Entwicklung"];

impl Prog {
    pub(crate) const ALL: [Prog; 10] = [Prog::Terminal, Prog::Files, Prog::Editor, Prog::IpConfig, Prog::Browser, Prog::Mail, Prog::Mib, Prog::Monitor, Prog::Settings, Prog::Ide];

    fn row(self) -> &'static (Prog, &'static str, &'static str, usize, [i32; 2], [i32; 2]) {
        &PROGS[self as usize]
    }
    // the row of a window's key: its program's, the Terminal's for a further Terminal window
    fn row_of(key: u32) -> &'static (Prog, &'static str, &'static str, usize, [i32; 2], [i32; 2]) {
        Prog::of_key(key).unwrap_or(Prog::Terminal).row()
    }
    /// its window's key in the DeskState (!= 0)
    pub(crate) fn key(self) -> u32 {
        self as u32 + 1
    }
    pub(crate) fn of_key(k: u32) -> Option<Prog> {
        Prog::ALL.get((k as usize).wrapping_sub(1)).copied()
    }
    /// its name in the current language
    pub(crate) fn name(self) -> &'static str {
        t(self.row().1)
    }
    pub(crate) fn icon(self) -> u32 {
        match self {
            Prog::Terminal => JN_TERMINAL,
            Prog::Files => JN_APP_FILES,
            Prog::Editor => JN_APP_EDITOR,
            Prog::IpConfig => JN_APP_IPCONF,
            Prog::Browser => JN_APP_BROWSER,
            Prog::Mail => JN_APP_MAIL,
            Prog::Mib => JN_APP_MIB,
            Prog::Monitor => JN_APP_MONITOR,
            Prog::Settings => JN_APP_SETTINGS,
            Prog::Ide => JN_APP_IDE,
        }
    }
    // the window as desk_begin takes it
    fn win(self, title: &str) -> DeskWin<'_> {
        let r = self.row();
        let w = DeskWin::new(self.key(), title, self.icon()).size(r.4[0], r.4[1]).min(r.5[0], r.5[1]);
        // the large programs start maximised (the device window is a small screen)
        if matches!(self, Prog::Ide | Prog::Mail | Prog::Mib) { w.flags(DWF_MAXIMIZED) } else { w }
    }
}

// a further Terminal window (session s) as desk_begin takes it
fn term_win(s: u32, title: &str) -> DeskWin<'_> {
    let r = Prog::Terminal.row();
    DeskWin::new(TERM_KEY0 + s, title, Prog::Terminal.icon()).size(r.4[0], r.4[1]).min(r.5[0], r.5[1])
}

/// a device's desktop: the windows, the programs' states (the session's)
pub(crate) struct Desk {
    pub dev: u32,
    /// the windows' places, z-order and states
    pub desk: DeskState,
    /// the open programs in the taskbar's order
    pub open: Vec<Prog>,
    /// windows (keys) that still need their first place (beside the icons, as large as fits)
    place: Vec<u32>,
    /// the icon chosen
    sel: Option<Prog>,
    /// a window (key) to bring to the front after the frame
    raise: Option<u32>,
    /// the window that draws the device's console (this frame)
    pub console_at: Option<Prog>,
    /// the key of the active window (this frame): a program's shortcuts are its own then
    pub front: u32,
    /// the sessions of the further Terminal windows (cli/ses.rs), in the taskbar's order after
    /// the programs
    pub terms: Vec<u32>,
    /// the session of the IDE's Terminal pane (opened when it is first wanted, ended with the
    /// IDE's window)
    pub ide_term: Option<u32>,
    // the programs
    pub ip: super::progs::IpState,
    pub web: super::browser::Browser,
    pub edit: super::editor::Editor,
    pub fm: super::filemgr::Fm,
    pub mon: super::sysmon::Mon,
    pub mail: super::email::MailUi,
    pub mib: super::mib::MibState,
}

impl Desk {
    pub(crate) fn new(dev: u32) -> Desk {
        let mut desk = DeskState::new();
        desk.set_reserve([0, 0, 0, TASK_H]);
        Desk {
            dev,
            desk,
            open: Vec::new(),
            place: Vec::new(),
            sel: None,
            raise: None,
            console_at: None,
            front: 0,
            terms: Vec::new(),
            ide_term: None,
            ip: super::progs::IpState::new(),
            web: super::browser::Browser::new(),
            edit: super::editor::Editor::new(),
            fm: super::filemgr::Fm::new(),
            mon: super::sysmon::Mon::new(),
            mail: super::email::MailUi::new(),
            mib: super::mib::MibState::new(),
        }
    }

    /// a program's window opens (new: beside the icons) or comes to the front after this frame
    pub(crate) fn open_prog(&mut self, p: Prog) {
        if !self.open.contains(&p) {
            self.open.push(p);
            self.place.push(p.key());
        }
        self.raise = Some(p.key());
    }

    /// a further Terminal window with session s of the device (console::open_session) opens
    pub(crate) fn open_term(&mut self, s: u32) {
        let key = TERM_KEY0 + s;
        if !self.terms.contains(&s) {
            self.terms.push(s);
            self.place.push(key);
        }
        self.raise = Some(key);
    }

    /// the window that draws the console: the Terminal or the MIB-Browser, the higher one
    /// that is not minimised (windows the state does not know yet come on top, in the order
    /// they were opened)
    pub(crate) fn console_owner(&self) -> Option<Prog> {
        let order = self.desk.order();
        let height = |p: &Prog| match order.iter().position(|&k| k == p.key()) {
            Some(i) => i,
            None => order.len() + self.open.iter().position(|o| o == p).unwrap_or(0),
        };
        [Prog::Terminal, Prog::Mib].into_iter().filter(|p| self.open.contains(p) && self.desk.state(p.key()) & DW_MIN == 0).max_by_key(height)
    }
}

/// the first id of device window k's desktop block
pub(crate) fn desk_base(k: usize) -> u32 {
    ID_DESK0 + 0x1000 * k as u32
}

// ---------------------------------------------------------------- the look

const WHITE: u32 = 0xFFFF_FFFF;
const INK: u32 = 0xFF1C_1C1C;
// the patterns
const P_NONE: u8 = 0;
const P_DOTS: u8 = 1;
const P_NET: u8 = 2;
/// the backgrounds (prefs DESK_BGS): colour in the light and the dark theme, the labels'
/// colour on them, the pattern
const BGS: [([u32; 2], [u32; 2], u8); 6] = [
    ([0xFF4A_5A6B, 0xFF2A_3440], [WHITE, WHITE], P_NONE),
    ([0xFF4E_6656, 0xFF2C_3A31], [WHITE, WHITE], P_NONE),
    ([0xFFD6_C8A8, 0xFF4A_4232], [INK, WHITE], P_NONE),
    ([0xFF1E_3250, 0xFF12_1E30], [WHITE, WHITE], P_NONE),
    ([0xFF46_576A, 0xFF25_2E38], [WHITE, WHITE], P_DOTS),
    ([0xFF3A_5163, 0xFF1D_2A34], [WHITE, WHITE], P_NET),
];
/// the tiles of the icons by group (light, dark): steel blue, teal, violet
const TILES: [[u32; 2]; 3] = [[0xFF4F_7396, 0xFF3B_5670], [0xFF3E_8A86, 0xFF2E_6764], [0xFF6A_6390, 0xFF50_4A6C]];

/// colour a mixed toward b by f / 256 (alpha from a)
fn mix(a: u32, b: u32, f: u32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = ((a >> s) & 255, (b >> s) & 255);
        ((x * (256 - f) + y * f) >> 8) << s
    };
    (a & 0xFF00_0000) | ch(16) | ch(8) | ch(0)
}

/// background bg (prefs index) in the light or dark theme: its colour, the labels' colour, the
/// colours of its pattern (lines, then dots and nodes; the background's own without one) --
/// all opaque
pub(crate) fn bg_colors(bg: usize, dark: bool) -> (u32, u32, [u32; 2]) {
    let (c, fg, pat) = BGS[bg.min(BGS.len() - 1)];
    let d = dark as usize;
    let toward = fg[d];
    let pc = if pat == P_NONE { [c[d], c[d]] } else { [mix(c[d], toward, 22), mix(c[d], toward, 40)] };
    (c[d], fg[d], pc)
}

/// the number of backgrounds (the selftest walks them)
pub(crate) const BG_LABEL: usize = BGS.len();

/// what the desktop looks like this frame (px)
struct Look {
    hc: bool,
    dark: bool,
    bg: u32,
    fg: u32,
    pat: u8,
    pat_c: [u32; 2],
    /// the tile, the glyph and the cell (px)
    tile: i32,
    glyph: i32,
    cell: [i32; 2],
}

impl Look {
    fn of(n: &Net, ui: &Ui) -> Look {
        let hc = ui.high_contrast();
        let dark = ui.theme_dark();
        let (bg, fg, pat_c) = bg_colors(n.prefs.desk_bg, dark);
        let pat = if hc { P_NONE } else { BGS[n.prefs.desk_bg.min(BGS.len() - 1)].2 };
        let (bg, fg) = if hc { (ui.color(T_BG), ui.color(T_TEXT)) } else { (bg, fg) };
        // small, medium, large: tile, glyph, cell (DIP)
        let (tile, glyph, cw, ch) = match n.prefs.desk_icons {
            0 => (32, 20, 78, 66),
            2 => (54, 34, 106, 98),
            _ => (42, 26, 90, 80),
        };
        Look { hc, dark, bg, fg, pat, pat_c, tile: ui.dp(tile), glyph: ui.dp(glyph), cell: [ui.dp(cw), ui.dp(ch)] }
    }
}

// the background with its pattern over the area (px)
fn paint_bg(ui: &mut Ui, a: [i32; 4], lk: &Look) {
    ui.draw_color(lk.bg);
    ui.draw_rect(a[0], a[1], a[2], a[3]);
    match lk.pat {
        P_DOTS => {
            let step = ui.dp(24);
            let r = ui.dp(3) << 7; // 1.5 DIP, 24.8
            ui.draw_color(lk.pat_c[1]);
            let mut y = a[1] + step / 2;
            while y < a[1] + a[3] {
                let mut x = a[0] + step / 2;
                while x < a[0] + a[2] {
                    ui.draw_circle(x << 8, y << 8, r, 0);
                    x += step;
                }
                y += step;
            }
        }
        P_NET => {
            // a tile of five nodes and their links, repeated (DIP in a 120 tile)
            const NODES: [(i32, i32); 5] = [(16, 22), (72, 12), (106, 58), (54, 76), (22, 104)];
            const LINKS: [(usize, usize); 6] = [(0, 1), (1, 2), (2, 3), (3, 0), (3, 4), (1, 3)];
            let s = ui.dp(120);
            let lw = ui.dp(1).max(1) << 8;
            let nr = ui.dp(5) << 7; // 2.5 DIP
            let pts: Vec<(i32, i32)> = NODES.iter().map(|&(x, y)| (ui.dp(x), ui.dp(y))).collect();
            let mut ty = a[1];
            while ty < a[1] + a[3] {
                let mut tx = a[0];
                while tx < a[0] + a[2] {
                    let p = |i: usize| ((tx + pts[i].0) << 8, (ty + pts[i].1) << 8);
                    ui.draw_color(lk.pat_c[0]);
                    for &(i, j) in &LINKS {
                        let (x0, y0) = p(i);
                        let (x1, y1) = p(j);
                        ui.draw_line(x0, y0, x1, y1, lw);
                    }
                    ui.draw_color(lk.pat_c[1]);
                    for i in 0..NODES.len() {
                        let (x, y) = p(i);
                        ui.draw_circle(x, y, nr, 0);
                    }
                    tx += s;
                }
                ty += s;
            }
        }
        _ => {}
    }
}

// a label of at most two lines in a cell (px): split after a hyphen or space when too wide,
// each line cut with "…"
fn label_lines(ui: &mut Ui, s: &str, maxw: i32) -> Vec<String> {
    let fit = |ui: &mut Ui, s: &str| -> String {
        if ui.text.width(s) <= maxw {
            return String::from(s);
        }
        let mut v: Vec<char> = s.chars().collect();
        while !v.is_empty() {
            v.pop();
            let c: String = v.iter().collect::<String>() + "…";
            if ui.text.width(&c) <= maxw {
                return c;
            }
        }
        String::from("…")
    };
    if ui.text.width(s) <= maxw {
        return alloc::vec![String::from(s)];
    }
    match s.find(['-', ' ']) {
        Some(i) => {
            let cut = i + 1;
            let (a, b) = (s[..cut].trim_end(), s[cut..].trim_start());
            alloc::vec![fit(ui, a), fit(ui, b)]
        }
        None => alloc::vec![fit(ui, s)],
    }
}

/// what the icons asked for this frame
#[derive(Default)]
struct IconAct {
    /// Some(None): nothing chosen any more
    select: Option<Option<Prog>>,
    open: Option<Prog>,
}

// the icons in columns from the top left of the work area (px a, its height without the
// taskbar wh) -> what happened, and the width they take (DIP)
fn icons(ui: &mut Ui, base: u32, a: [i32; 4], wh: i32, lk: &Look, sel: Option<Prog>, names: &[&str]) -> (IconAct, i32) {
    let mut act = IconAct::default();
    // a press on the background takes the choice away
    let ev = ui.hit_rect(base + 0x40, a[0], a[1], a[2], wh, 0);
    if ev & EV_ACT != 0 && sel.is_some() {
        act.select = Some(None);
    }
    let m = ui.dp(8);
    let rows = ((wh - 2 * m) / lk.cell[1]).max(1) as usize;
    let cols = Prog::ALL.len().div_ceil(rows);
    let width = ui.px2dip(m * 2 + cols as i32 * lk.cell[0]);
    // the labels in the caption font (the font before comes back after them)
    let font = ui.text.get_font();
    ui.ts_font(TS_CAPTION);
    let (asc, desc) = (ui.text.ascent(), ui.text.descent());
    let lh = asc + desc;
    ui.focus_group_begin(base + 0x20);
    let mut bbox = [0i32; 4];
    for (i, &p) in Prog::ALL.iter().enumerate() {
        let (c, r) = (i / rows, i % rows);
        let cell = [a[0] + m + c as i32 * lk.cell[0], a[1] + m + r as i32 * lk.cell[1], lk.cell[0], lk.cell[1]];
        bbox = if i == 0 { cell } else { union(bbox, cell) };
        let id = base + 0x30 + p as u32;
        let chosen = sel == Some(p);
        let ev = ui.hit_rect(id, cell[0], cell[1], cell[2], cell[3], HB_FOCUS | if chosen { HB_CURRENT } else { 0 });
        let state = ui.last_state();
        ui.tooltip(t(PROGS[p as usize].2));
        // a click chooses it; a double click, Enter or a screen reader's click opens it; the
        // arrows choose the icon they move to
        if ev & EV_DOUBLE != 0 || (ev & EV_ACT != 0 && ev & EV_COMMIT == 0) {
            act.open = Some(p);
            act.select = Some(Some(p));
        } else if (ev & EV_ACT != 0 || ui.focus_ring(ev)) && !chosen {
            // (the keyboard's focus is the choice: the arrows, Tab)
            act.select = Some(Some(p));
        }
        // paint: the choice, the hover, the tile with its glyph, the label
        let inset = ui.dp(2);
        let rad = ui.dp(6);
        let cr = [cell[0] + inset, cell[1] + inset, cell[2] - 2 * inset, cell[3] - 2 * inset];
        let (sel_fill, sel_line) = if lk.hc {
            (ui.color(T_ACCENT), ui.color(T_ACCENT))
        } else if lk.fg == WHITE {
            (0x40FF_FFFF, 0x99FF_FFFF)
        } else {
            (0x26_000000, 0x66_000000)
        };
        if chosen {
            ui.cmd_rrect(cr[0], cr[1], cr[2], cr[3], rad, ui.dp(1).max(1), sel_fill, sel_line, sel_line);
        } else if ev & EV_HOVER != 0 && !lk.hc {
            ui.draw_color(if lk.fg == WHITE { 0x1FFF_FFFF } else { 0x14_000000 });
            ui.draw_rrect(cr[0], cr[1], cr[2], cr[3], rad);
        }
        if ui.focus_ring(ev) {
            ui.draw_color(if lk.hc { ui.color(T_FOCUS_OUT) } else { lk.fg });
            ui.draw_frame(cell[0], cell[1], cell[2], cell[3], rad + inset, ui.dp(2).max(1));
        }
        let tx = cell[0] + (cell[2] - lk.tile) / 2;
        let ty = cell[1] + ui.dp(6);
        let g = PROGS[p as usize].3;
        if lk.hc {
            let line = if chosen { ui.color(T_ON_ACCENT) } else { ui.color(T_TEXT) };
            ui.cmd_rrect(tx, ty, lk.tile, lk.tile, lk.tile / 4, ui.dp(2).max(1), 0, line, line);
            ui.draw_color(line);
        } else {
            ui.draw_color(TILES[g][lk.dark as usize]);
            ui.draw_rrect(tx, ty, lk.tile, lk.tile, lk.tile / 4);
            ui.draw_color(WHITE);
        }
        ui.draw_icon(p.icon(), tx + (lk.tile - lk.glyph) / 2, ty + (lk.tile - lk.glyph) / 2, lk.glyph << 8);
        let tc = if lk.hc && chosen { ui.color(T_ON_ACCENT) } else { lk.fg };
        let lines = label_lines(ui, names[i], cell[2] - ui.dp(6));
        let mut y = ty + lk.tile + ui.dp(4);
        for l in &lines {
            let w = ui.text.width(l);
            ui.emit_text(cell[0] + (cell[2] - w) / 2, y + asc, l, tc);
            y += lh;
        }
        // for screen readers a row of the list "Desktop"
        ui.last_set(cell, state | if chosen { SS_SELECTED as u32 } else { 0 });
        ui.sem(K_LISTITEM, id, Some(names[i]), i as i32);
    }
    ui.focus_group_end();
    ui.text.set_font(font.0, font.1);
    let si = sel.map_or(-1, |p| p as i32);
    ui.last_set(bbox, 0);
    ui.sem(K_LIST, base + 0x20, Some(t("Desktop")), si).max = Prog::ALL.len() as i32;
    (act, width)
}

fn union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let (x0, y0) = (a[0].min(b[0]), a[1].min(b[1]));
    let (x1, y1) = ((a[0] + a[2]).max(b[0] + b[2]), (a[1] + a[3]).max(b[1] + b[3]));
    [x0, y0, x1 - x0, y1 - y0]
}

/// the clock of the taskbar: the device's clock in the simulation (form: prefs DESK_CLOCKS)
pub(crate) fn clock_text(n: &Net, dev: u32, form: usize) -> String {
    let Some(ns) = n.doc.sim.state().clock(dev) else { return String::from("–") };
    let c = datetime::Civil::from_unix(ns.div_euclid(1_000_000_000));
    match form {
        1 => format!("{:02}:{:02}:{:02}", c.hour, c.minute, c.second),
        2 => format!("{:04}-{:02}-{:02} {:02}:{:02}", c.year, c.month, c.day, c.hour, c.minute),
        _ => format!("{:02}:{:02}", c.hour, c.minute),
    }
}

// new windows (DIP): right of the icons, cascaded, as large as the work area lets them be
fn place_new(st: &mut DeskState, list: &[u32], work: [i32; 2], icons_w: i32) {
    let known = st.order().len().saturating_sub(list.len());
    for (i, &key) in list.iter().enumerate() {
        // (a maximised one: where it goes when restored)
        let Some(r) = st.rect(key) else { continue };
        let row = Prog::row_of(key);
        let tbh = (r[3] - row.4[1]).max(0);
        let (mw, mh) = (row.5[0], row.5[1] + tbh);
        let c = ((known + i) % 6) as i32;
        let (x0, y0) = (icons_w + 8 + c * 24, 8 + c * 24);
        let w = r[2].min(work[0] - x0 - 8).max(mw);
        let h = r[3].min(work[1] - y0 - 8).max(mh);
        let x = x0.min(work[0] - w - 4).max(4);
        let y = y0.min(work[1] - h - 4).max(0);
        st.set_rect(key, [x, y, w, h]);
    }
}

// ---------------------------------------------------------------- the tab

/// the Desktop tab of device window k (id: the window's desktop block)
pub(crate) fn tab(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let base = desk_base(k);
    let win_id = super::win_id(k);
    let lk = Look::of(n, ui);
    let clock = clock_text(n, dev, n.prefs.desk_clock);
    let on = n.doc.proj.device(dev).is_some_and(|d| !d.off);
    let Some(w) = n.devwin.get(k) else { return };
    let d = &mut w.desk;
    // the sessions that end: of the windows closed this frame, of a desktop given up
    let mut ended: Vec<(u32, u32)> = Vec::new();
    if d.dev != dev {
        ended.extend(d.terms.iter().chain(&d.ide_term).map(|&s| (d.dev, s)));
        *d = Desk::new(dev);
    }
    let mut titles: Vec<String> = d.open.iter().map(|p| if *p == Prog::Browser { d.web.window_title(p.name()) } else { String::from(p.name()) }).collect();
    titles.extend(d.terms.iter().map(|s| format!("{} ({})", Prog::Terminal.name(), s + 1)));
    let names: Vec<&'static str> = Prog::ALL.iter().map(|p| p.name()).collect();
    let np = d.open.len();
    let wins: Vec<DeskWin> = d.open.iter().zip(&titles).map(|(p, t)| p.win(t)).chain(d.terms.iter().zip(&titles[np..]).map(|(&s, t)| term_win(s, t))).collect();
    d.console_at = d.console_owner();
    d.front = d.desk.active();
    let sel = d.sel;
    let mut st = core::mem::take(&mut d.desk);
    let place = core::mem::take(&mut d.place);
    ui.next_size(-1, -1);
    ui.desk_begin(base, &mut st, &wins);
    let a = ui.box_rect();
    let wh = (a[3] - ui.dp(TASK_H)).max(1);
    paint_bg(ui, a, &lk);
    let (act, icons_w) = icons(ui, base, a, wh, &lk, sel, &names);
    // the first places of new windows (once the area has its size)
    let place = if a[2] > 0 && a[3] > 0 {
        place_new(&mut st, &place, [ui.px2dip(a[2]), ui.px2dip(wh)], icons_w);
        Vec::new()
    } else {
        place
    };
    while let Some(key) = ui.desk_next(&mut st) {
        if let Some(p) = Prog::of_key(key) {
            content(n, ui, k, id, dev, p);
        } else if let Some(s) = key.checked_sub(TERM_KEY0) {
            terminal_window(n, ui, k, dev, Some(s));
        }
    }
    // the taskbar: Start and its menu, the windows, Vollbild, the clock
    ui.desk_taskbar_begin(base);
    ui.next_icon(JN_START);
    if ui.button(base + 1, t("Start")) & EV_ACT != 0 {
        ui.popup_open(base + 2);
    }
    ui.tooltip(t("Die Programme des Geräts"));
    let (started, more) = start_menu(ui, base, on);
    ui.desk_taskbar(base, &mut st, 0);
    let mut full = (ui.window_state(win_id) & UWS_FULLSCREEN != 0) as i32;
    ui.next_icon(JN_FULLSCREEN);
    ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
    if ui.toggle_button(base + 3, t("Vollbild"), &mut full) & EV_ACT != 0 {
        ui.window_fullscreen(win_id, full != 0);
    }
    ui.tooltip(t("Vollbild (F11)"));
    ui.text(TS_CAPTION, &clock);
    ui.desk_taskbar_end();
    let evs = ui.desk_end(&mut st);
    // back into the desktop: closed windows, the icons' and the menu's wishes
    let Some(w) = n.devwin.get(k) else { return };
    let d = &mut w.desk;
    d.desk = st;
    d.place.extend(place);
    let mut web_gone: Vec<u64> = Vec::new();
    for ev in evs {
        if let DeskEv::Close(key) = ev {
            if let Some(p) = Prog::of_key(key) {
                d.open.retain(|&o| o != p);
                // the browser's state goes with its window
                if p == Prog::Browser {
                    web_gone = d.web.close();
                }
                // the IDE's terminal ends with it
                if p == Prog::Ide
                    && let Some(s) = d.ide_term.take()
                {
                    ended.push((dev, s));
                }
            } else if let Some(s) = key.checked_sub(TERM_KEY0) {
                d.terms.retain(|&o| o != s);
                ended.push((dev, s));
            } else {
                continue;
            }
            d.place.retain(|&o| o != key);
            d.desk.forget(key);
            ui.repass();
        }
    }
    if let Some(s) = act.select
        && s != d.sel
    {
        d.sel = s;
        ui.repass();
    }
    let opened = act.open.or(started);
    if let Some(p) = opened {
        d.open_prog(p);
        // the device window takes the keys (the program's content gets the focus)
        if ui.window_state(win_id) & UWS_ACTIVE == 0 {
            ui.window_activate(win_id);
        }
    }
    if let Some(key) = d.raise.take() {
        if d.desk.contains(key) {
            d.desk.activate(key);
        }
        ui.repass();
    }
    if opened == Some(Prog::Terminal) {
        n.cons.focus = Some(dev);
    }
    for (dv, s) in ended {
        console::close_session(n, dv, s);
    }
    for req in web_gone {
        let _ = n.doc.sim.send(jfn_sim::Cmd::Apply(jfn_sim::Action::WebCancel { dev, id: req }));
    }
    // "Weiteres Terminal": a session of its own in a window of its own
    if more {
        match console::open_session(n, dev) {
            Some(s) => {
                if let Some(w) = n.devwin.get(k) {
                    w.desk.open_term(s);
                }
                n.cons.focus_ses = Some((dev, s));
                if ui.window_state(win_id) & UWS_ACTIVE == 0 {
                    ui.window_activate(win_id);
                }
            }
            None => {
                let name = n.doc.proj.device(dev).map(|d| d.name.clone()).unwrap_or_default();
                n.file.msg = Some((IB_INFO, String::from(t("Keine weitere Sitzung")), crate::app::tr::tf("{} nimmt keine weitere Sitzung an: es hat keine Befehlszeile oder keinen Strom.", &[&name])));
            }
        }
        ui.repass();
    }
}

// the start menu (a flyout above Start): the programs in their groups -> the one chosen, and
// whether "Weiteres Terminal" was (on: the device has power)
fn start_menu(ui: &mut Ui, base: u32, on: bool) -> (Option<Prog>, bool) {
    let mut chosen = None;
    let mut more = false;
    if ui.flyout_begin(base + 2, FL_ABOVE) {
        for (g, title) in GROUPS.iter().enumerate() {
            ui.box_begin(0, BOX_V | pad(4), 0);
            ui.text(TS_CAPTION | TX_SECONDARY, t(title));
            ui.box_end();
            for p in Prog::ALL.iter().filter(|p| PROGS[**p as usize].3 == g) {
                ui.next_icon(p.icon());
                if ui.menu_item(base + 0x10 + *p as u32, p.name()) & EV_ACT != 0 {
                    chosen = Some(*p);
                }
                // one more Terminal window, with a session of its own
                if *p == Prog::Terminal {
                    ui.next_icon(JN_TERMINAL);
                    ui.next_flags(if on { 0 } else { ST_DISABLED });
                    if ui.menu_item(base + 0x1F, t("Weiteres Terminal")) & EV_ACT != 0 && on {
                        more = true;
                    }
                }
            }
            if g + 1 < GROUPS.len() {
                ui.menu_sep();
            }
        }
    }
    ui.flyout_end();
    (chosen, more)
}

// a program's window content
fn content(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32, p: Prog) {
    let base = desk_base(k);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | if matches!(p, Prog::Terminal | Prog::Ide) { 0 } else { pad(8) }, 8);
    match p {
        Prog::Terminal => terminal_window(n, ui, k, dev, None),
        Prog::Files => super::filemgr::program(n, ui, k, base + 0x200, dev),
        Prog::Editor => super::editor::program(n, ui, k, base + 0x100, dev),
        Prog::IpConfig => super::progs::ipconfig(n, ui, k, id + 0x10, dev),
        Prog::Browser => super::browser::program(n, ui, k, id + 0x30, dev),
        Prog::Mail => super::email::program(n, ui, k, ID_MAIL0 + 0x400 * k as u32, dev),
        Prog::Mib => super::mib::browser(n, ui, k, dev),
        Prog::Monitor => super::sysmon::program(n, ui, k, base + 0x400, dev),
        Prog::Settings => settings(n, ui, base + 0x600),
        Prog::Ide => ide_window(n, ui, k, dev),
    }
    ui.box_end();
}

// ---------------------------------------------------------------- the console's windows

/// the console of the device in program p's window (Terminal, MIB-Browser), or a note where it
/// is shown instead: a console is drawn in one place at a time
pub(crate) fn console_or_note(n: &mut Net, ui: &mut Ui, k: usize, dev: u32, p: Prog) {
    let at = n.devwin.get(k).and_then(|w| w.desk.console_at);
    if at == Some(p) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V, 0);
        console::view(n, ui, dev);
        ui.box_end();
        return;
    }
    let base = desk_base(k);
    let (nid, other) = if p == Prog::Terminal { (base + 0x710, Prog::Mib) } else { (base + 0x720, Prog::Terminal) };
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(8), 8);
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP | TX_CENTER, &crate::app::tr::tf("Die Konsole des Geräts ist im Fenster „{}“ offen.", &[other.name()]));
    if ui.button(nid, t("Zeigen")) & EV_ACT != 0
        && let Some(w) = n.devwin.get(k)
    {
        w.desk.raise = Some(other.key());
        n.cons.focus = Some(dev);
    }
    ui.box_end();
}

/// a Terminal window's content: the device's console (s None: the program Terminal) or a further
/// session of its own (a session that ended says so and takes no input)
pub(crate) fn terminal_window(n: &mut Net, ui: &mut Ui, k: usize, dev: u32, s: Option<u32>) {
    let Some(s) = s else {
        console_or_note(n, ui, k, dev, Prog::Terminal);
        return;
    };
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    console::view_session(n, ui, dev, s);
    ui.box_end();
}

/// the IDE's window (it starts maximised): the IDE of workspace/ide in the window's box
pub(crate) fn ide_window(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(4), 0);
    crate::workspace::ide::view(n, ui, k, dev);
    ui.box_end();
}

/// the device's window on its desktop with the IDE's window in front (CMD_SCRIPTS)
pub(crate) fn open_ide(n: &mut Net, ui: &mut Ui, dev: u32) {
    super::open_on(n, ui, dev, super::Tab::Desktop);
    if let Some(w) = n.devwin.wins.iter_mut().flatten().find(|w| w.dev == dev && w.open.get() != 0) {
        w.desk.open_prog(Prog::Ide);
    }
    ui.repass();
}

/// a file of the device in the program for it (the Dateimanager's Öffnen): a script in
/// open_jfs, every other file in the Texteditor
pub(crate) fn open_with(n: &mut Net, k: usize, dev: u32, path: &str) {
    if path.ends_with(".jfs") {
        open_jfs(n, k, path);
    } else {
        super::editor::open_file(n, k, dev, Some(path));
    }
}

/// a .jfs script of the device: a tab of the IDE, whose window comes to the front
pub(crate) fn open_jfs(n: &mut Net, k: usize, path: &str) {
    crate::workspace::ide::open_later(n, k, path);
    if let Some(w) = n.devwin.get(k) {
        w.desk.open_prog(Prog::Ide);
    }
}

// ---------------------------------------------------------------- Einstellungen

// Einstellungen: background, icons and clock of the desktops (the user's settings)
fn settings(n: &mut Net, ui: &mut Ui, id: u32) {
    use crate::app::prefs::{desk_names, DESK_BG_COUNT, DESK_CLOCK_COUNT, DESK_ICON_COUNT};
    let dark = ui.theme_dark();
    let (mut bg, mut ic, mut cl) = (n.prefs.desk_bg as i32, n.prefs.desk_icons as i32, n.prefs.desk_clock as i32);
    let (b0, i0, c0) = (bg, ic, cl);
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 8);
    ui.text(TS_BODY_STRONG, t("Hintergrund"));
    let bgs = desk_names(0);
    ui.gridbox_begin(0, &[-1, -1], 0, 6);
    for i in 0..DESK_BG_COUNT {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        let r = ui.place(ui.dp(30), ui.dp(20));
        // a swatch of the background (with a dot of its pattern)
        let (c, _, pc) = bg_colors(i, dark);
        let line = ui.color(T_CTRL_STROKE);
        ui.cmd_rrect(r[0], r[1], r[2], r[3], ui.dp(3), ui.dp(1).max(1), c, line, line);
        if pc[1] != c {
            ui.draw_color(pc[1]);
            ui.draw_circle((r[0] + r[2] / 2) << 8, (r[1] + r[3] / 2) << 8, ui.dp(3) << 7, 0);
        }
        ui.radio(id + 1 + i as u32, t(bgs[i]), &mut bg, i as i32);
        ui.box_end();
    }
    ui.gridbox_end();
    ui.text(TS_BODY_STRONG, t("Größe der Symbole"));
    ui.box_begin(0, BOX_H | BOX_WRAP, 12);
    for (i, name) in desk_names(1).iter().enumerate().take(DESK_ICON_COUNT) {
        ui.radio(id + 0x10 + i as u32, t(name), &mut ic, i as i32);
    }
    ui.box_end();
    ui.text(TS_BODY_STRONG, t("Uhr der Taskleiste"));
    ui.box_begin(0, BOX_V, 4);
    for (i, name) in desk_names(2).iter().enumerate().take(DESK_CLOCK_COUNT) {
        ui.radio(id + 0x20 + i as u32, t(name), &mut cl, i as i32);
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Sie gelten für die Desktops aller Hosts und stehen auch unter Werkzeuge › Einstellungen › Desktop; das Projekt und die Geräte ändern sie nicht."));
    ui.box_end();
    ui.scroll_end();
    if (bg, ic, cl) != (b0, i0, c0) {
        n.prefs.desk_bg = bg.max(0) as usize;
        n.prefs.desk_icons = ic.max(0) as usize;
        n.prefs.desk_clock = cl.max(0) as usize;
        n.prefs.push();
        ui.repass();
    }
}

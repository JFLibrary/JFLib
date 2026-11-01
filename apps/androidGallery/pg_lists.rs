// "Listen & Bäume": a virtualized list of 200 rows, reorderable rows with thumbnails and check
// boxes (LF_REORDER; apps/gallery/sec_list.rs), a tree with check boxes, a canvas redrawn through
// canvas_damage alone (the marching ants).  Rows are 48 DIP high: fingers.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

use ui::*;

const ID_LIST: u32 = 4000; // rows 4001 .. 4200
const ID_RLIST: u32 = 4300; // rows 4301 ..
const ID_RL_CB: u32 = 4350; // check boxes 4350 + item, thumbnails 4370 + item
const ID_TREE: u32 = 4400;
const ID_ANTS: u32 = 4500;
const ID_ANTS_STEP: u32 = 4501;
const RL_N: usize = 12;
const RL_COLORS: [u32; RL_N] = [
    0xFFE81123, 0xFFFF8C00, 0xFFFFB900, 0xFF107C10, 0xFF00B7C3, 0xFF0078D4, 0xFF8764B8, 0xFFE3008C, 0xFF7A7574, 0xFF498205,
    0xFF038387, 0xFF881798,
];
const TREE: [(u64, u16, bool, u32, &str, Option<&str>); 8] = [
    (1, 0, true, IC_FOLDER, "Fotos", Some("1,2 GB")),
    (2, 1, true, IC_FOLDER, "Urlaub", Some("840 MB")),
    (3, 2, false, IC_PHOTO, "Strand.jpg", Some("2,1 MB")),
    (4, 2, false, IC_PHOTO, "Berge.jpg", Some("3,4 MB")),
    (5, 1, false, IC_FOLDER, "Katze", Some("120 MB")),
    (6, 0, true, IC_FOLDER, "Dokumente", Some("64 MB")),
    (7, 1, false, IC_DOCUMENT, "Brief.txt", Some("2 KB")),
    (8, 1, false, IC_DOCUMENT, "Plan.pdf", Some("640 KB")),
];

/// a dashed rectangle whose dashes shift with the phase, read when it renders
struct Ants {
    phase: Cell<i32>,
}

impl Draw for Ants {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        g.dc.set_color(0xFFFF_FFFF);
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        g.dc.set_color(0xFF00_0000);
        let ph = self.phase.get();
        for x in 0..r[2] {
            if (x.wrapping_add(ph) & 15) < 8 {
                g.dc.fill(g.sf, r[0] + x, r[1] + 8, 1, 1);
                g.dc.fill(g.sf, r[0] + x, r[1] + r[3] - 9, 1, 1);
            }
        }
    }
}

pub struct Lists {
    sel: i32,
    double: i32,
    rl_sel: i32,
    rl_vis: [i32; RL_N],
    rl_order: [u8; RL_N],
    rl_moves: i32,
    tree: TreeState,
    checks: [u8; 8],
    ants: Rc<Ants>,
}

impl Default for Lists {
    fn default() -> Self {
        let mut tree = TreeState::default();
        tree.expanded.insert(1);
        Self {
            sel: 2,
            double: 0,
            rl_sel: 0,
            rl_vis: [1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            rl_order: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            rl_moves: 0,
            tree,
            checks: [0, 1, 0, 0, 0, 0, 1, 0],
            ants: Rc::new(Ants { phase: Cell::new(0) }),
        }
    }
}

impl Lists {
    pub fn page(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY_STRONG, "200 Einträge");
        ui.next_size(0, 240);
        let (first, end) = ui.list_begin(ID_LIST, &mut self.sel, 200, 48);
        for i in first..end {
            if ui.list_item(i, &format!("Eintrag {}", i + 1)) & EV_DOUBLE != 0 {
                self.double += 1;
            }
        }
        ui.list_end(&mut self.sel);
        ui.text(TS_BODY | TX_SECONDARY, &format!("Auswahl: {}   Doppelt getippt: {}", self.sel, self.double));

        ui.text(TS_BODY_STRONG, "Ebenen zum Ziehen");
        ui.next_size(0, 240);
        let (first, end) = ui.list_begin(ID_RLIST, &mut self.rl_sel, RL_N as i32, 48 | LF_REORDER);
        for row in first..end {
            ui.list_item_begin(row);
            // the handle: a vertical move here drags the row, elsewhere it rolls the list
            ui.icon(IC_MENU, 20);
            ui.touch_drag();
            let it = self.rl_order[row as usize] as usize;
            ui.next_size(28, 28);
            ui.next_flags(ST_NOFOCUS); // a picture: no Tab stop
            ui.swatch(ID_RL_CB + 20 + it as u32, RL_COLORS[it], false);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &format!("Ebene {}", it + 1));
            ui.checkbox(ID_RL_CB + it as u32, "", &mut self.rl_vis[it]);
            ui.list_item_end();
        }
        let (ev, v) = ui.list_end(&mut self.rl_sel);
        if ev & EV_DRAG != 0 {
            let (from, to) = ((v as u32 & 0xFFFF) as usize, (v as u32 >> 16) as usize);
            let item = self.rl_order[from];
            if from < to {
                self.rl_order.copy_within(from + 1..=to, from);
            } else {
                self.rl_order.copy_within(to..from, to + 1);
            }
            self.rl_order[to] = item;
            self.rl_moves += 1;
        }
        let mut order = String::from("Reihenfolge:");
        for o in self.rl_order {
            order.push_str(&format!(" {}", o as u32 + 1));
        }
        ui.text(TS_BODY | TX_WRAP, &order);
        ui.text(TS_BODY | TX_SECONDARY, &format!("Auswahl: {}   Verschoben: {}", self.rl_sel, self.rl_moves));

        ui.text(TS_BODY_STRONG, "Baum");
        self.tree(ui);

        ui.text(TS_BODY_STRONG, "Zeichenfläche");
        ui.next_size(SIZE_FILL, 64);
        let d: DrawRef = self.ants.clone();
        ui.canvas(ID_ANTS, Some(d), 0, 1);
        ui.next_size(SIZE_FILL, 44);
        if ui.button(ID_ANTS_STEP, "Ameisen laufen lassen") & EV_ACT != 0 {
            let p = &self.ants.phase;
            p.set(p.get().wrapping_add(3));
            // only the edges changed: the version stays
            ui.canvas_damage(ID_ANTS, 0, 0, 1000, 16);
            ui.canvas_damage(ID_ANTS, 0, 40, 1000, 400);
        }
    }

    fn tree(&mut self, ui: &mut Ui) {
        let mut rows = Vec::new();
        let mut hidden_below: Option<u16> = None;
        for (k, (key, depth, kids, icon, label, detail)) in TREE.iter().enumerate() {
            if let Some(d) = hidden_below {
                if *depth > d {
                    continue;
                }
                hidden_below = None;
            }
            rows.push(TreeRow { key: *key, depth: *depth, has_children: *kids, check: Some(self.checks[k]), icon: *icon, label, detail: *detail });
            if *kids && !self.tree.expanded.contains(key) {
                hidden_below = Some(*depth);
            }
        }
        ui.next_size(0, 8 * 44);
        let ev = ui.tree(ID_TREE, &mut self.tree, &rows, 44);
        if let Some((k, v)) = ev.check {
            if let Some(p) = TREE.iter().position(|r| r.0 == k) {
                self.checks[p] = v;
            }
        }
        let cur = self.tree.cur.and_then(|k| TREE.iter().find(|r| r.0 == k)).map_or("–", |r| r.4);
        ui.text(TS_BODY | TX_SECONDARY, &format!("Knoten: {cur}"));
    }
}

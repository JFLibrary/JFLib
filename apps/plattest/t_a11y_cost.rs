// t_a11y_cost -- what a web page's accessibility tree costs per frame (section 19 of the guide)
//
//   plattest --test a11y-cost [--records N]   (headless; not in "all": timings) a page of N rich
//                              records (default 3400, BrowserGUI's large page): landmarks,
//                              paragraphs, a table of rows and cells, links, each with an id64,
//                              its parent and some a description, below a canvas.  Times per
//                              frame (best and median of 41): the records (sem_rect and the
//                              setters, inside App::frame), the tree built from them
//                              (ui.a11y_tree), its publish (plat::a11y::publish: the differences
//                              to the last tree), the key of the tree's inputs and what
//                              a11y_frame does while a client listens (the key; the tree and its
//                              publish only when it changed), for an unchanged page and for one
//                              whose text changes in a row.  Measure it in a release build
//                              (build_rs.py app plattest --release).

use alloc::format;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::SeqCst};

use ui::{App, AppDesc, Ui};

use crate::T;

const DESC: AppDesc = AppDesc { title: "a11y-Kosten", w: 1200, h: 900, ..AppDesc::NONE };
const ID_PAGE: u32 = 0x500;

// the records per frame, whether a row's text changes, the last frame's time in App::frame (us)
static RECORDS: AtomicU32 = AtomicU32::new(3400);
static CHANGE: AtomicBool = AtomicBool::new(false);
static FRAME_US: AtomicU64 = AtomicU64::new(0);

struct Page {
    turn: u32,
}

impl App for Page {
    fn frame(&mut self, ui: &mut Ui) {
        let n = RECORDS.load(SeqCst);
        ui.next_size(-1, -1);
        let r = ui.canvas(ID_PAGE, None, 0, 0).rect;
        let t0 = plat::time_us();
        let page = ID_PAGE as u64;
        if CHANGE.load(SeqCst) {
            self.turn = self.turn.wrapping_add(1);
        }
        // a main landmark and a table, then groups of 15 records: a paragraph, a table row of 3 cells with a
        // link in each, a list of 3 items with a link and a description each
        let mut id = 1u64;
        let mut next = || {
            id += 1;
            id
        };
        let main = next();
        ui.sem_rect(ui::K_LANDMARK, 0, r, Some("Inhalt"), ui::LM_MAIN).id64(main).parent(page);
        let table = next();
        ui.sem_rect(ui::K_TABLE, 0, r, Some("Preise"), 0).id64(table).parent(main).grid(n / 15, 3);
        let mut made = 2;
        let mut row = 0u32;
        while made + 15 <= n {
            let y = r[1] + (row as i32 % 40) * 20;
            let rr = [r[0], y, r[2], 20];
            let p = next();
            let text = if row == 7 { format!("Absatz {row}: Stand {}", self.turn) } else { format!("Absatz {row}: ein Text, wie ihn eine Seite hat") };
            ui.sem_rect(ui::K_TEXT, 0, rr, Some(&text), 0).id64(p).parent(main);
            let tr = next();
            ui.sem_rect(ui::K_ROW, 0, rr, Some("Zeile"), 0).id64(tr).parent(table);
            for c in 0..3u32 {
                let cr = [r[0] + c as i32 * 100, y, 100, 20];
                let cell = next();
                ui.sem_rect(ui::K_CELL, 0, cr, Some("12,50 €"), 0).id64(cell).parent(tr).cell(row, c);
                ui.sem_rect(ui::K_LINK, 0, cr, Some("Mehr"), 0).id64(next()).parent(cell).url("https://example.org/mehr");
            }
            let list = next();
            ui.sem_rect(ui::K_LIST, 0, rr, Some("Liste"), 0).id64(list).parent(main);
            for k in 0..3u32 {
                let item = next();
                ui.sem_rect(ui::K_LISTITEM, 0, rr, Some("Eintrag"), 0).id64(item).parent(list).set_pos(k + 1, 3);
                ui.sem_rect(ui::K_LINK, 0, rr, Some("Weiter"), 0).id64(next()).parent(item).desc("Öffnet die nächste Seite");
            }
            made += 15;
            row += 1;
        }
        FRAME_US.store(plat::time_us() - t0, SeqCst);
    }
}

// best and median of the times
fn stats(mut v: Vec<u64>) -> (u64, u64) {
    v.sort_unstable();
    (v.first().copied().unwrap_or(0), v.get(v.len() / 2).copied().unwrap_or(0))
}

pub fn t_a11y_cost(t: &mut T) {
    if let Some(n) = plat::arg_value("--records").and_then(|v| v.parse::<u32>().ok()) {
        RECORDS.store(n.max(17), SeqCst);
    }
    let mut rt = match ui::Runtime::new(&DESC, Page { turn: 0 }, &style::STYLE) {
        Ok(rt) => rt,
        Err(_) => {
            t.check(false, "a11y-cost: the page starts headless");
            return;
        }
    };
    let wake = [plat::Event { typ: plat::E_WAKE, ..Default::default() }];
    let mut now = 0u32;
    for change in [false, true] {
        CHANGE.store(change, SeqCst);
        let (mut rec, mut key, mut tree, mut publish, mut step, mut nodes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), 0usize);
        let mut last = 0u64;
        for _ in 0..41 {
            now += 50;
            rt.step(now, &wake);
            rec.push(FRAME_US.load(SeqCst));
            let ui = rt.ui();
            // what a11y_frame does once a client listens: the key, and the tree and its publish
            // only when the key differs from the last one
            let t0 = plat::time_us();
            let k = ui.a11y_tree_key();
            let t1 = plat::time_us();
            if k != last {
                last = k;
                plat::a11y::publish(ui.a11y_tree());
            }
            let t2 = plat::time_us();
            // the tree and its publish whatever the key says (before charge 8: every frame)
            let s = ui.a11y_tree();
            let t3 = plat::time_us();
            nodes = s.wins.iter().map(|w| w.nodes.len()).sum();
            plat::a11y::publish(s);
            let t4 = plat::time_us();
            key.push(t1 - t0);
            step.push(t2 - t0);
            tree.push(t3 - t2);
            publish.push(t4 - t3);
        }
        let (r, k, tr, p, st) = (stats(rec), stats(key), stats(tree), stats(publish), stats(step));
        let what = if change { "a row's text changes" } else { "unchanged" };
        t.info(&format!(
            "  a11y-cost ({what}): {} records, {nodes} nodes: records {} / {} us, tree {} / {} us, publish {} / {} us, key {} / {} us, \
             a11y_frame (key, tree and publish when it differs) {} / {} us (best / median)\n",
            RECORDS.load(SeqCst), r.0, r.1, tr.0, tr.1, p.0, p.1, k.0, k.1, st.0, st.1
        ));
        t.check(nodes + 15 >= RECORDS.load(SeqCst) as usize, "a11y-cost: every record is a node");
    }
}

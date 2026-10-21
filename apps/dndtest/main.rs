// dndtest -- drag and drop (ui/dnd.rs, ui/drop.rs, plat/dnd.rs): widgets as drag sources and
// drop targets, drops of other programs (text, HTML, URLs, images, files), dragging out of the
// app, for tests/*.uis (and by hand against Explorer, a browser, a file manager)
//
//   build/win/dndtest.exe                         by hand: drag in and out of the window
//   build/win/dndtest.exe --script apps/dndtest/tests/widgets.uis
//
// Three fruits (#101..#103) can be dragged into two baskets (#201: fruits and text, #202:
// fruits only) and back to the shelf (#200); each carries the app's own kind DT_FRUIT (its
// index) and its name as text, so it can also be dropped onto the text target or out of the
// window.  The targets #301..#305 take text, HTML, URLs, an image, files; outside them the
// window takes files and text (drop_accept) as UIC_DROP.  Three buttons drag content that only
// other programs take: a text, a PNG (with a BMP copy for DIB targets) and a temporary file.
// Status lines show what the targets saw: "Über", "Abgelegt", "Ende", "UIC_DROP".
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use ui::*;

const DESC: AppDesc = AppDesc { title: "jfgui Drag & Drop", flags: AF_RESIZABLE, w: 760, h: 560, ..AppDesc::NONE };

const DT_FRUIT: u32 = DT_USER0; // the app's own kind: a fruit (bytes: its index)
const FRUITS: [&str; 3] = ["Apfel", "Birne", "Kirsche"];
const ID_FRUIT0: u32 = 101;
const ID_SHELF: u32 = 200;
const ID_BASKET_A: u32 = 201;
const ID_BASKET_B: u32 = 202;
const ID_T_TEXT: u32 = 301; // .. 305: text, HTML, URL, image, files
const ID_OUT_TEXT: u32 = 401;
const ID_OUT_IMAGE: u32 = 402;
const ID_OUT_FILE: u32 = 403;
const TARGET_NAMES: [&str; 5] = ["Text", "HTML", "URL", "Bild", "Dateien"];
const TARGET_TYPES: [u32; 5] = [DT_TEXT, DT_HTML, DT_URL, DT_IMAGE, DT_FILES];

struct Dnd {
    /// where each fruit is: 0 shelf, 1 basket A, 2 basket B
    place: [u8; 3],
    /// what each target received
    got: [String; 5],
    over: String,
    dropped: String,
    ended: String,
    uic: String,
    ends: u32,
    /// a fruit dropped into a place: moved at the end of the frame (it was drawn already)
    moving: Option<(usize, u8)>,
    /// jfgui_DNDTEST_PRINT=1: the status lines go to stdout when they change (dnd_x11.py --app)
    print: bool,
    printed: String,
}

impl Default for Dnd {
    fn default() -> Dnd {
        Dnd {
            place: [0; 3],
            got: Default::default(),
            over: String::from("-"),
            dropped: String::from("-"),
            ended: String::from("-"),
            uic: String::from("-"),
            ends: 0,
            moving: None,
            print: plat::env("jfgui_DNDTEST_PRINT").is_some_and(|v| v == "1"),
            printed: String::new(),
        }
    }
}

fn effect_name(e: u32) -> &'static str {
    match e {
        DE_COPY => "kopieren",
        DE_MOVE => "verschieben",
        DE_LINK => "verknüpfen",
        _ => "keine",
    }
}

fn kinds(t: u32) -> String {
    let mut v: Vec<&str> = Vec::new();
    for (k, n) in [(DT_FILES, "Dateien"), (DT_TEXT, "Text"), (DT_URL, "URL"), (DT_HTML, "HTML"), (DT_IMAGE, "Bild"), (DT_FRUIT, "Frucht")] {
        if t & k != 0 {
            v.push(n);
        }
    }
    if v.is_empty() { String::from("-") } else { v.join("+") }
}

// "PNG 4x3" / "BMP 4x3" / "? 12 Bytes" of an image file
fn image_info(b: &[u8]) -> String {
    let be = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let le = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    if b.len() >= 24 && b.starts_with(b"\x89PNG") {
        return format!("PNG {}x{}", be(16), be(20));
    }
    if b.len() >= 26 && b.starts_with(b"BM") {
        return format!("BMP {}x{}", le(18), (le(22) as i32).unsigned_abs());
    }
    format!("? {} Bytes", b.len())
}

// a 4 x 3 PNG (red, green, blue, white columns) and the same image as BMP
fn sample_image() -> (Vec<u8>, Vec<u8>) {
    let (w, h) = (4usize, 3usize);
    let cols: [[u8; 4]; 4] = [[0, 0, 255, 255], [0, 255, 0, 255], [255, 0, 0, 255], [255, 255, 255, 255]]; // B G R A
    let mut px = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&cols[x]);
        }
    }
    let png = img_png::encode(&px, w, h, w * 4, false);
    let mut sf = gfx::Surface::new(w as i32, h as i32, gfx::FMT_ARGB);
    let ds = sf.stride as usize;
    for y in 0..h {
        sf.px[y * ds..y * ds + w * 4].copy_from_slice(&px[y * w * 4..(y + 1) * w * 4]);
    }
    (png, clip_bmp_from_sf(&sf).unwrap_or_default())
}

impl Dnd {
    // a fruit as a draggable card
    fn fruit(&mut self, ui: &mut Ui, i: usize) {
        let id = ID_FRUIT0 + i as u32;
        let ev = ui.button(id, FRUITS[i]);
        match ui.drag_source(id, ev) {
            DS_START => {
                let mut d = DragData::of_app(DT_FRUIT, &[i as u8]);
                d.text = Some(String::from(FRUITS[i]));
                d.effects = DE_MOVE | DE_COPY;
                ui.drag_set(d);
            }
            DS_END => {
                self.ends += 1;
                self.ended = format!("{} {} ({})", FRUITS[i], effect_name(ui.drag_effect()), self.ends);
            }
            _ => {}
        }
    }

    // a place for fruits: the shelf or a basket
    fn place(&mut self, ui: &mut Ui, id: u32, k: u8, title: &str, accept: u32) {
        ui.next_size(220, 120);
        ui.box_begin(id, BOX_V | BOX_CARD | PAD_8, 6);
        match ui.drop_target(id, accept) {
            DS_HOVER => self.over = String::from(title),
            DS_DROP => {
                let d = ui.drop_data().clone();
                if d.app & DT_FRUIT != 0 {
                    let i = d.bytes.first().copied().unwrap_or(0) as usize % FRUITS.len();
                    self.moving = Some((i, k));
                    self.dropped = format!("{} -> {} ({})", FRUITS[i], title, effect_name(ui.drop_effect()));
                } else {
                    self.dropped = format!("Text {:?} -> {}", d.text.as_deref().unwrap_or(""), title);
                }
            }
            _ => {}
        }
        ui.text(TS_BODY_STRONG, title);
        ui.box_begin(0, BOX_H, 6);
        for i in 0..FRUITS.len() {
            if self.place[i] == k {
                self.fruit(ui, i);
            }
        }
        ui.box_end();
        ui.box_end();
    }

    // a target of one kind of content
    fn target(&mut self, ui: &mut Ui, k: usize) {
        let id = ID_T_TEXT + k as u32;
        ui.next_size(132, 90);
        ui.box_begin(id, BOX_V | BOX_CARD | PAD_8, 4);
        match ui.drop_target(id, TARGET_TYPES[k]) {
            DS_HOVER => self.over = format!("{}-Ziel", TARGET_NAMES[k]),
            DS_DROP => {
                let d = ui.drop_data();
                self.got[k] = match k {
                    0 => d.text.clone().unwrap_or_default(),
                    1 => d.html.clone().unwrap_or_default(),
                    2 => d.urls.join(" "),
                    3 => d.image.as_deref().map(image_info).unwrap_or_default(),
                    _ => format!("{} Datei(en): {}", d.files.len(), d.files.first().map(|f| f.rsplit(['/', '\\']).next().unwrap_or(f)).unwrap_or("")),
                };
                self.dropped = format!("{} -> {}-Ziel ({})", kinds(d.types()), TARGET_NAMES[k], effect_name(ui.drop_effect()));
            }
            _ => {}
        }
        ui.text(TS_BODY_STRONG, &format!("{}-Ziel", TARGET_NAMES[k]));
        let g = self.got[k].clone();
        ui.text(TS_CAPTION | TX_ELLIPSIS, &format!("{}: {}", TARGET_NAMES[k], if g.is_empty() { "-" } else { &g }));
        ui.box_end();
    }

    // a button that drags content out (only other programs or the targets take it)
    fn out_button(&mut self, ui: &mut Ui, id: u32, label: &str) {
        let ev = ui.button(id, label);
        match ui.drag_source(id, ev) {
            DS_START => {
                let d = match id {
                    ID_OUT_TEXT => DragData { text: Some(String::from("Hallo aus jfgui")), ..DragData::default() },
                    ID_OUT_IMAGE => {
                        let (png, bmp) = sample_image();
                        DragData { image: Some(png), bmp: Some(bmp), label: String::from("Bild 4x3"), ..DragData::default() }
                    }
                    _ => {
                        // a file written for the drag (the app's temporary folder)
                        let dir = plat::temp_dir().unwrap_or_else(|| String::from("."));
                        let path = format!("{}/jfgui-dndtest.txt", dir.trim_end_matches(['/', '\\']));
                        plat::write_file(&path, b"jfgui drag and drop\n");
                        DragData { files: vec![path], ..DragData::default() }
                    }
                };
                ui.drag_set(d);
            }
            DS_END => {
                self.ends += 1;
                self.ended = format!("{} {} ({})", label, effect_name(ui.drag_effect()), self.ends);
            }
            _ => {}
        }
    }
}

impl App for Dnd {
    fn init(&mut self, ui: &mut Ui) -> bool {
        ui.drop_accept(DT_FILES | DT_TEXT); // outside the targets: UIC_DROP
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        self.over = String::from("-");
        ui.box_begin(0, BOX_V | PAD_16, 10);
        ui.text(TS_SUBTITLE, "Drag & Drop");
        ui.box_begin(0, BOX_H, 10);
        self.place(ui, ID_SHELF, 0, "Regal", DT_FRUIT);
        self.place(ui, ID_BASKET_A, 1, "Korb A", DT_FRUIT | DT_TEXT);
        self.place(ui, ID_BASKET_B, 2, "Korb B", DT_FRUIT);
        ui.box_end();
        ui.box_begin(0, BOX_H, 8);
        for k in 0..5 {
            self.target(ui, k);
        }
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.out_button(ui, ID_OUT_TEXT, "Text ziehen");
        self.out_button(ui, ID_OUT_IMAGE, "Bild ziehen");
        self.out_button(ui, ID_OUT_FILE, "Datei ziehen");
        ui.box_end();
        let (t, x, y) = ui.drag_over();
        let over = if t == 0 { String::from("-") } else { format!("{} bei {},{}", kinds(t), x, y) };
        ui.text(TS_BODY, &format!("Ziehen: {}  Über: {}", over, self.over));
        ui.text(TS_BODY, &format!("Abgelegt: {}", self.dropped));
        ui.text(TS_BODY, &format!("Ende: {}", self.ended));
        ui.text(TS_BODY, &format!("UIC_DROP: {}", self.uic));
        ui.box_end();
        // the pass drew the fruit at its old place already: move it now, build again
        if let Some((i, k)) = self.moving.take() {
            self.place[i] = k;
            ui.repass();
        }
        if self.print {
            let s = format!("Abgelegt: {}\nEnde: {}\nUIC_DROP: {}\n", self.dropped, self.ended, self.uic);
            if s != self.printed {
                plat::write_stdout(s.as_bytes());
                self.printed = s;
            }
        }
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            UIC_DROP => {
                let d = ui.drop_data();
                let what = if !d.files.is_empty() {
                    format!("{} Datei(en)", ui.drop_count())
                } else {
                    format!("Text {:?}", d.text.as_deref().unwrap_or(""))
                };
                let (x, y) = ui.drop_pos();
                self.uic = format!("{} bei {},{} (Fenster {})", what, x, y, arg);
            }
            UIC_DRAG_END => {} // the sources report through DS_END
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, Dnd::default(), &style::STYLE));

// "Texteingabe": what the big gallery has of text (apps/gallery/sec_edit.rs, gal_u2, gal_gridbox,
// gal_texts): text boxes, a password, number boxes, multi-line boxes, a form in a grid, wrapped and
// cut texts.  A box that gets the keyboard stays above it (main.rs keeps the page scrolled).

use alloc::format;
use alloc::string::String;

use ui::*;
use ui::i18n::{t, tf};

const ID_NAME: u32 = 3000;
const ID_PW: u32 = 3001;
const ID_NUM: u32 = 3002;
const ID_NUMF: u32 = 3003;
const ID_MULTI: u32 = 3004;
const ID_WRAP: u32 = 3005;
const ID_GRID: u32 = 3010;
const ID_GRID_NAME: u32 = 3011;
const ID_GRID_DESC: u32 = 3012;
const ID_GRID_SAVE: u32 = 3013;
const ID_GRID_SHARE: u32 = 3014;
const ID_GRID_OK: u32 = 3015;

const RANGE_NUM: Range = Range::new(0, 1000, 1, 0);
const RANGE_NUMF: RangeF = RangeF::new(0.0, 100.0, 0.5, 1, 0);

const S_ASK: &str = "„Unbenannt.png“ hat nicht gespeicherte Änderungen. Möchten Sie die Änderungen speichern, bevor Sie das Bild schließen?"; // i18n
const S_LONG: &str = "Donaudampfschifffahrtsgesellschaftskapitänsmützenabzeichen passen in keine Zeile."; // i18n
const S_CUT: &str = "Dieser Hinweis ist länger als zwei Zeilen und wird deshalb am Ende mit Auslassungspunkten gekürzt."; // i18n

pub struct Texts {
    name: String,
    pw: String,
    commits: i32,
    num: i32,
    numf: f64,
    multi: String,
    wrap: String,
    grid_name: String,
    grid_desc: String,
    grid_opt: [i32; 2],
    grid_ok: i32,
}

impl Default for Texts {
    fn default() -> Self {
        Self {
            name: String::new(),
            pw: String::new(),
            commits: 0,
            num: 7,
            numf: 3.5,
            multi: (1..=3).map(|n| tf("Zeile {}", &[&format!("{}", n)])).collect::<alloc::vec::Vec<_>>().join("\n"),
            wrap: format!("{}\n{}", t("Ein langer Absatz, der am Rand des Textfelds umgebrochen wird, damit alles sichtbar bleibt."), t("Zweiter Absatz.")),
            grid_name: String::new(),
            grid_desc: String::new(),
            grid_opt: [1, 0],
            grid_ok: 0,
        }
    }
}

impl Texts {
    pub fn page(&mut self, ui: &mut Ui, reveal: bool, scroll: u32) {
        // Android's keyboard shrinks the window: the box with the focus stays above it
        let keep = |ui: &mut Ui, id: u32| {
            if reveal && ui.focused() == id {
                let rect = ui.last_rect();
                let delta = ui.px2dip(rect[1] + rect[3]) + 16 - ui.client_size().1;
                if delta > 0 {
                    let (_, y) = ui.scroll_pos(scroll);
                    ui.scroll_to(scroll, -1, y + delta);
                }
            }
        };
        ui.text(TS_BODY_STRONG, t("Eine Zeile"));
        ui.next_size(SIZE_FILL, 44);
        if ui.textbox(ID_NAME, Some(t("Name eingeben")), &mut self.name, 64) & EV_COMMIT != 0 {
            self.commits += 1;
        }
        keep(ui, ID_NAME);
        ui.next_size(SIZE_FILL, 44);
        ui.textbox(ID_PW, Some(t("Kennwort")), &mut self.pw, 32 | TB_PASSWORD);
        keep(ui, ID_PW);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, &tf("Eingabe: {}   (übernommen: {})", &[&self.name, &format!("{}", self.commits)]));

        ui.text(TS_BODY_STRONG, t("Zahlen"));
        ui.box_begin(0, BOX_H, 8);
        ui.next_size(SIZE_FILL, 44);
        ui.number_box(ID_NUM, &mut self.num, &RANGE_NUM);
        ui.next_size(SIZE_FILL, 44);
        ui.number_box_f(ID_NUMF, &mut self.numf, &RANGE_NUMF);
        ui.box_end();
        keep(ui, ID_NUMF);
        let f = ui.fmt_f64(self.numf, 1);
        ui.text(TS_BODY | TX_SECONDARY, &tf("Werte: {} und {}", &[&format!("{}", self.num), &f]));

        ui.text(TS_BODY_STRONG, t("Mehrere Zeilen"));
        ui.next_size(SIZE_FILL, 96);
        ui.textbox(ID_MULTI, Some(t("Mehrzeilig")), &mut self.multi, 512 | TB_MULTI);
        keep(ui, ID_MULTI);
        ui.next_size(SIZE_FILL, 120);
        ui.textbox(ID_WRAP, Some(t("Umbruch")), &mut self.wrap, 1024 | TB_MULTI | TB_WRAP);
        keep(ui, ID_WRAP);

        ui.text(TS_BODY_STRONG, t("Formular im Raster"));
        ui.gridbox_begin(ID_GRID, &[0, -1], BOX_CARD | pad(12), 8);
        ui.text(TS_BODY, t("Name"));
        ui.next_size(SIZE_FILL, 44);
        ui.textbox(ID_GRID_NAME, Some(t("Vorname Nachname")), &mut self.grid_name, 64);
        keep(ui, ID_GRID_NAME);
        ui.text(TS_BODY, t("Worum"));
        ui.next_size(SIZE_FILL, 44);
        ui.textbox(ID_GRID_DESC, Some(t("Worum es geht")), &mut self.grid_desc, 256);
        keep(ui, ID_GRID_DESC);
        ui.next_align(AL_START, AL_START);
        ui.text(TS_BODY, t("Optionen"));
        ui.box_begin(0, BOX_V, 4);
        let [a, b] = &mut self.grid_opt;
        ui.next_size(0, 40);
        ui.checkbox(ID_GRID_SAVE, t("Speichern"), a);
        ui.next_size(0, 40);
        ui.checkbox(ID_GRID_SHARE, t("Teilen"), b);
        ui.box_end();
        ui.next_span(2, 1);
        ui.next_align(AL_END, AL_DEFAULT);
        ui.next_size(120, 44);
        if ui.accent_button(ID_GRID_OK, "OK") & EV_ACT != 0 {
            self.grid_ok += 1;
        }
        ui.gridbox_end();
        ui.text(TS_BODY | TX_SECONDARY, &tf("Abgeschickt: {}", &[&format!("{}", self.grid_ok)]));

        ui.text(TS_BODY_STRONG, t("Umbruch und Kürzung"));
        ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 8);
        ui.text(TS_BODY | TX_WRAP, t(S_ASK));
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY | TX_CENTER, t(S_LONG));
        ui.text(TS_BODY | TX_WRAP | TX_RIGHT, &format!("{}\n{}", t("Erste Zeile"), t("Zweite Zeile, länger als die Karte breit ist")));
        ui.next_size(SIZE_FILL, 40);
        ui.text(TS_BODY | TX_WRAP | TX_ELLIPSIS, t(S_CUT));
        ui.next_size(SIZE_FILL, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, t(S_CUT));
        ui.box_end();
    }
}

// sec_edit -- the gallery's text input sections
//   [apps/gallery/src/main.s: the text input section of gal_frame, the multi-line boxes of
//    gal_texts; u2demo.s: the number boxes of gal_u2, gal_u2_cmd]

use alloc::format;
use alloc::string::String;

use ui::*;

use crate::{cap, Gal};

const ID_TB_NAME: u32 = 700;
const ID_TB_PW: u32 = 701;
const ID_TB_NUM: u32 = 702;
const ID_TXT_MULTI: u32 = 7960;
const ID_TXT_WRAP: u32 = 7961;
const ID_U2_NUM: u32 = 8610;
const ID_U2_NUMF: u32 = 8611;
const CMD_U2_SET: u32 = 9110;
const CMD_U2_INC: u32 = 9111;

const RANGE_TB_NUM: Range = Range::new(0, 10, 1, 0);
const RG_U2_NUM: Range = Range::new(0, 1000, 1, 0);
const RG_U2_NUMF: RangeF = RangeF::new(0.0, 100.0, 0.5, 1, 0);

/// the section's state
pub(crate) struct S {
    tb_name: String,
    tb_pw: String,
    tb_num: i32,
    tb_commits: i32,
    ml_text: String,
    ml_wrap: String,
    ml_commits: i32,
    u2_num: i32,
    u2_numf: f64,
    u2_commits: i32,
}

impl Default for S {
    fn default() -> S {
        S {
            tb_name: String::new(),
            tb_pw: String::new(),
            tb_num: 5,
            tb_commits: 0,
            ml_text: String::from("Zeile 1\nZeile 2\nZeile 3"),
            ml_wrap: String::from(
                "Ein langer Absatz, der am Rand des Textfelds umgebrochen wird, damit alles sichtbar bleibt.\nZweiter Absatz.",
            ),
            ml_commits: 0,
            u2_num: 7,
            u2_numf: 3.5,
            u2_commits: 0,
        }
    }
}

/// gal_frame's "Texteingabe": the subtitle, a row with two text boxes and a number box,
/// the "Eingabe: ..." line
pub(crate) fn text_input(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.edit;
    ui.text(TS_SUBTITLE, "Texteingabe");
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.next_size(240, 0);
    if ui.textbox(ID_TB_NAME, Some("Name eingeben"), &mut s.tb_name, 64) & EV_COMMIT != 0 {
        s.tb_commits += 1;
    }
    ui.next_size(180, 0);
    ui.textbox(ID_TB_PW, Some("Kennwort"), &mut s.tb_pw, 32 | TB_PASSWORD);
    ui.number_box(ID_TB_NUM, &mut s.tb_num, &RANGE_TB_NUM);
    ui.box_end();
    let t = cap(format!("Eingabe: {}   (übernommen: {}, Zahl: {})", s.tb_name, s.tb_commits, s.tb_num), 192);
    ui.text(TS_BODY | TX_SECONDARY, &t);
}

/// gal_texts' multi-line boxes: a row with "Mehrzeilig" (TB_MULTI) and "Umbruch" (TB_MULTI
/// | TB_WRAP), the "Übernommen" line
pub(crate) fn text_boxes(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.edit;
    ui.box_begin(0, BOX_H, 16);
    ui.next_size(300, 0);
    if ui.textbox(ID_TXT_MULTI, Some("Mehrzeilig"), &mut s.ml_text, 512 | TB_MULTI) & EV_COMMIT != 0 {
        s.ml_commits += 1;
    }
    ui.next_size(240, 120);
    ui.textbox(ID_TXT_WRAP, Some("Umbruch"), &mut s.ml_wrap, 1024 | TB_MULTI | TB_WRAP);
    ui.box_end();
    let t = cap(format!("Übernommen: {}", s.ml_commits), 64);
    ui.text(TS_BODY | TX_SECONDARY, &t);
}

/// gal_u2's number boxes: the row "Zahl:", number_box, number_box_f, "Wert %d, Übernahmen %d"
pub(crate) fn u2_numbers(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.edit;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY, "Zahl:");
    ui.next_size(100, 0);
    if ui.number_box(ID_U2_NUM, &mut s.u2_num, &RG_U2_NUM) & EV_COMMIT != 0 {
        s.u2_commits += 1;
    }
    ui.next_size(100, 0);
    if ui.number_box_f(ID_U2_NUMF, &mut s.u2_numf, &RG_U2_NUMF) & EV_COMMIT != 0 {
        s.u2_commits += 1;
    }
    let t = cap(format!("Wert {}, Übernahmen {}", s.u2_num, s.u2_commits), 64);
    ui.text(TS_BODY | TX_SECONDARY, &t);
    ui.box_end();
}

/// gal_u2_cmd(id, arg) -> handled: test hooks CMD_U2_SET (9110) sets the number box values
/// from outside (42 / 12,5), CMD_U2_INC (9111) adds 1 to both
pub(crate) fn u2_cmd(g: &mut Gal, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
    let s = &mut g.edit;
    match id {
        CMD_U2_SET => {
            s.u2_num = 42;
            s.u2_numf = 12.5;
            true
        }
        CMD_U2_INC => {
            s.u2_num += 1;
            s.u2_numf += 1.0;
            true
        }
        _ => false,
    }
}

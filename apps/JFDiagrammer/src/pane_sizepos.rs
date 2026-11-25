// pane_sizepos -- (JD8 rest) the task pane Größe und Position (AB-04): the measures of the single
// selection, each a field to change
//
//   pane(app, ui, id)        2D: X, Y (the pin), Breite, Höhe, Winkel, Drehpunktposition (nine
//                            places on the frame or Benutzerdefiniert); 1D: Anfang X/Y, Ende X/Y,
//                            Länge, Winkel
//   parse(text, metric)      what was typed: a number with the region's decimal sign or a dot,
//                            optionally a unit (mm, cm, m, in or ", pt, ft; since JD15 also km,
//                            yd, mi, p / pc / pica, c / cicero, dd / didot, PG-07; deg or ° for
//                            angles); without one the page's unit (mm metric, in US)
//
// A field shows the cell's value while it does not have the keyboard; typing edits a copy, Enter
// or leaving the field writes it (one undo step "Größe und Position"), Esc or a value that does
// not parse keeps the shape as it was.  The cells go through the engine (Engine::user_set: GUARD
// and the locks are respected, formulas that depend on them follow); connectors glued to the
// shape follow it (edit_canvas::follow).  A 1D shape's ends move through glue::glue as free ends,
// as dragging them does: a glued end typed elsewhere comes loose.  Visio's window also shows
// guides and offers the length/angle forms for 1D; guides are left out here (P1 lists them, the
// guide shapes keep PinX/PinY/Angle which X, Y, Winkel already change).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::{math, Input};
use diagram_core::glue::{self, End, Target};
use diagram_core::{ops, CellAddr, Document, EditError, Owner, ShapeRef, SheetRef, Unit};
use ui::i18n::{self, t};
use ui::*;

use crate::edit_canvas::{self, shape_of};
use crate::{edit_doc, Diagrammer};

/// the field being typed into: its id and the text so far
#[derive(Default)]
pub(crate) struct SizePos {
    field: u32,
    buf: String,
    pin: i32,
}

// i18n
const PIN_PLACES: [&str; 10] = ["Oben links", "Oben Mitte", "Oben rechts", "Mitte links", "Mitte", "Mitte rechts", "Unten links", "Unten Mitte", "Unten rechts", "Benutzerdefiniert"];

// the place k (0..9) of the pin on the frame: fractions of width and height (y up)
fn pin_place(k: usize) -> [f64; 2] {
    [[0.0, 0.5, 1.0][k % 3], [1.0, 0.5, 0.0][(k / 3).min(2)]]
}

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num()).filter(|v| v.is_finite())
}

/// a length or angle as typed -> (value, unit); None: not a number
pub(crate) fn parse(text: &str, angle: bool, metric: bool) -> Option<(f64, Unit)> {
    let s = text.trim().replace(',', ".");
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(s.len());
    let (n, u) = s.split_at(split);
    let v: f64 = n.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    let u = u.trim().to_ascii_lowercase();
    let unit = match u.as_str() {
        "" if angle => Unit::Degrees,
        "" if metric => Unit::Millimeters,
        "" => Unit::Inches,
        "°" | "deg" | "grad" => Unit::Degrees,
        _ if angle => return None,
        "mm" => Unit::Millimeters,
        "cm" => Unit::Centimeters,
        "m" => Unit::Meters,
        "in" | "\"" | "zoll" => Unit::Inches,
        "pt" => Unit::Points,
        "ft" | "'" => Unit::Feet,
        // (JD15, PG-07) the other lengths of Visio's list
        "km" => Unit::Kilometers,
        "yd" => Unit::Yards,
        "mi" => Unit::Miles,
        "p" | "pc" | "pica" => Unit::Picas,
        "c" | "cicero" => Unit::Ciceros,
        "dd" | "didot" => Unit::Didots,
        _ => return None,
    };
    Some((v, unit))
}

// a value in `unit` to inches
fn to_inches(v: f64, unit: Unit) -> f64 {
    unit.to_internal(v)
}

// a length (inches) in the page's unit, as the status bar shows it
fn length(v: f64, metric: bool) -> String {
    let s = if metric { format!("{:.1} mm", v * 25.4) } else { format!("{:.3} in", v) };
    let s = s.replace(".0 mm", " mm");
    let dec = i18n::locale().decimal();
    s.replace('.', dec.encode_utf8(&mut [0; 4]))
}

fn degrees(rad: f64) -> String {
    let d = rad.to_degrees();
    let d = if d.abs() < 5e-7 { 0.0 } else { d };
    let s = format!("{d:.1}°").replace(".0°", "°");
    let dec = i18n::locale().decimal();
    s.replace('.', dec.encode_utf8(&mut [0; 4]))
}

// what a field changes
#[derive(Clone, Copy, PartialEq)]
enum Field {
    PinX,
    PinY,
    Width,
    Height,
    Angle,
    Begin(usize),
    End(usize),
    Length,
    LineAngle,
}

/// the pane
pub(crate) fn pane(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V | BOX_V | pad(10));
    let metric = edit_canvas::metric(app);
    let one = single(app);
    let rows: Vec<(String, String, Field)> = match (one, app.opened.as_ref()) {
        (Some(s), Some(o)) => rows_of(&o.doc, s, metric),
        _ => Vec::new(),
    };
    if rows.is_empty() {
        // the empty fields of a 2D shape
        for (k, label) in ["X", "Y", t("Breite"), t("Höhe"), t("Winkel")].iter().enumerate() {
            field_row(ui, id + 1 + k as u32, label, &mut String::new(), false);
        }
        ui.spacer(6);
        let msg = if app.se.sel.len() > 1 { t("Mehrere Shapes ausgewählt.") } else { t("Kein Shape ausgewählt.") };
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, msg);
        ui.scroll_end();
        return;
    }
    let mut commit: Option<(Field, String)> = None;
    for (k, (label, value, f)) in rows.iter().enumerate() {
        let fid = id + 1 + k as u32;
        let focused = ui.focused() == fid;
        if focused && app.sizepos.field != fid {
            app.sizepos.field = fid;
            app.sizepos.buf = value.clone();
        }
        let mut s = if focused { app.sizepos.buf.clone() } else { value.clone() };
        let ev = field_row(ui, fid, label, &mut s, true);
        if focused || ev & EV_COMMIT != 0 {
            app.sizepos.buf = s.clone();
        }
        if ev & EV_COMMIT != 0 && s != *value {
            commit = Some((*f, s));
        }
        // Enter (the field still has the keyboard): the canvas takes it back, so Ctrl+Z takes
        // the change back and the arrows move the shape again
        if ev & EV_COMMIT != 0 && focused {
            ui.focus(crate::panes::ID_CANVAS);
        }
        if !focused && app.sizepos.field == fid && ev & EV_COMMIT == 0 {
            app.sizepos.field = 0;
        }
    }
    // the pin's place (2D)
    let two_d = rows.iter().any(|r| r.2 == Field::Width);
    if let (true, Some(s), Some(o)) = (two_d, one, app.opened.as_ref()) {
        let f = ops::frame(&o.doc, s);
        let sh = SheetRef::Shape(s);
        let lp = [num(&o.doc, sh, "LocPinX").unwrap_or(f.w * 0.5), num(&o.doc, sh, "LocPinY").unwrap_or(f.h * 0.5)];
        let cur = (0..9).find(|&k| {
            let p = pin_place(k);
            (lp[0] - p[0] * f.w).abs() < 1e-6 && (lp[1] - p[1] * f.h).abs() < 1e-6
        });
        app.sizepos.pin = cur.map_or(9, |k| k as i32);
        let names: Vec<&str> = PIN_PLACES.iter().map(|s| t(s)).collect();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(130, 0);
        ui.text(TS_BODY, t("Drehpunktposition"));
        let before = app.sizepos.pin;
        ui.next_size(-1, 0);
        ui.combobox(id + 0x10, &mut app.sizepos.pin, &names);
        ui.box_end();
        if app.sizepos.pin != before && (0..9).contains(&app.sizepos.pin) {
            let k = app.sizepos.pin as usize;
            set_pin_place(app, ui, s, k);
        }
    }
    ui.scroll_end();
    if let (Some((f, text)), Some(s)) = (commit, one) {
        // the field shows the cell again (its new value) from the next pass
        app.sizepos.field = 0;
        apply(app, ui, s, f, &text, metric);
    }
}

// a label and its field
fn field_row(ui: &mut Ui, id: u32, label: &str, s: &mut String, on: bool) -> u32 {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(130, 0);
    ui.text(TS_BODY, label);
    ui.next_size(-1, 0);
    ui.next_flags(if on { 0 } else { ST_DISABLED });
    let ev = ui.textbox(id, None, s, 32);
    ui.box_end();
    ev
}

// the single selected shape
fn single(app: &Diagrammer) -> Option<ShapeRef> {
    let page = app.model.page?;
    app.opened.as_ref()?;
    if app.se.sel.len() != 1 {
        return None;
    }
    Some(shape_of(page, *app.se.sel.iter().next()?))
}

// the rows of a shape: label, value as text, what it changes
fn rows_of(doc: &Document, s: ShapeRef, metric: bool) -> Vec<(String, String, Field)> {
    let sh = SheetRef::Shape(s);
    let n = |c: &str| num(doc, sh, c).unwrap_or(0.0);
    if glue::is_one_d(doc, s) {
        let (b, e) = glue::ends(doc, s).unwrap_or(([n("BeginX"), n("BeginY")], [n("EndX"), n("EndY")]));
        let (dx, dy) = (e[0] - b[0], e[1] - b[1]);
        let len = math::sqrt(dx * dx + dy * dy);
        let ang = if len > 0.0 { math::atan2(dy, dx) } else { 0.0 };
        alloc::vec![
            (String::from(t("Anfang X")), length(b[0], metric), Field::Begin(0)),
            (String::from(t("Anfang Y")), length(b[1], metric), Field::Begin(1)),
            (String::from(t("Ende X")), length(e[0], metric), Field::End(0)),
            (String::from(t("Ende Y")), length(e[1], metric), Field::End(1)),
            (String::from(t("Länge")), length(len, metric), Field::Length),
            (String::from(t("Winkel")), degrees(ang), Field::LineAngle),
        ]
    } else {
        let f = ops::frame(doc, s);
        alloc::vec![
            (String::from("X"), length(n("PinX"), metric), Field::PinX),
            (String::from("Y"), length(n("PinY"), metric), Field::PinY),
            (String::from(t("Breite")), length(f.w.abs(), metric), Field::Width),
            (String::from(t("Höhe")), length(f.h.abs(), metric), Field::Height),
            (String::from(t("Winkel")), degrees(n("Angle")), Field::Angle),
        ]
    }
}

// write a field's new value (one undo step); a value that does not parse changes nothing
fn apply(app: &mut Diagrammer, ui: &mut Ui, s: ShapeRef, f: Field, text: &str, metric: bool) {
    let angle = matches!(f, Field::Angle | Field::LineAngle);
    let Some((v, unit)) = parse(text, angle, metric) else { return };
    let Some(page) = app.model.page else { return };
    let owner = Owner::Page(page);
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, t("Größe und Position"), true, |tx, eng| {
        let sheet = SheetRef::Shape(s);
        match f {
            Field::PinX | Field::PinY | Field::Width | Field::Height | Field::Angle => {
                let cell = match f {
                    Field::PinX => "PinX",
                    Field::PinY => "PinY",
                    Field::Width => "Width",
                    Field::Height => "Height",
                    _ => "Angle",
                };
                if matches!(f, Field::Width | Field::Height) && v < 0.0 {
                    return Err(EditError::Invalid);
                }
                let a = tx.single(cell);
                eng.user_set(tx, sheet, &a, Input::Length(v, unit))?;
            }
            _ => {
                let doc = tx.doc();
                let (b, e) = glue::ends(doc, s).ok_or(EditError::Invalid)?;
                let inch = to_inches(v, unit);
                let (end, at) = match f {
                    Field::Begin(k) => {
                        let mut p = b;
                        p[k] = inch;
                        (End::Begin, p)
                    }
                    Field::End(k) => {
                        let mut p = e;
                        p[k] = inch;
                        (End::End, p)
                    }
                    Field::Length => {
                        let (dx, dy) = (e[0] - b[0], e[1] - b[1]);
                        let len = math::sqrt(dx * dx + dy * dy);
                        let (ux, uy) = if len > 1e-12 { (dx / len, dy / len) } else { (1.0, 0.0) };
                        (End::End, [b[0] + ux * inch, b[1] + uy * inch])
                    }
                    _ => {
                        let (dx, dy) = (e[0] - b[0], e[1] - b[1]);
                        let len = math::sqrt(dx * dx + dy * dy);
                        let r = v.to_radians();
                        (End::End, [b[0] + len * math::cos(r), b[1] + len * math::sin(r)])
                    }
                };
                glue::glue(tx, eng, s, end, at, Target::None)?;
            }
        }
        edit_canvas::follow(tx, eng, owner, &[s])
    });
    ui.repass();
}

// the pin to place k of the frame (the shape stays where it is)
fn set_pin_place(app: &mut Diagrammer, ui: &mut Ui, s: ShapeRef, k: usize) {
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, t("Drehpunktposition"), true, |tx, eng| {
        let f = ops::frame(tx.doc(), s);
        let p = pin_place(k);
        let at = f.at([p[0] * f.w, p[1] * f.h]);
        ops::set_pin(tx, eng, s, at)
    });
    ui.repass();
}

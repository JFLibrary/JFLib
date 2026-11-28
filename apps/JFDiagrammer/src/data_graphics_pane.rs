// data_graphics_pane -- (JD19, DA-08, AB-12, RD-08) the task pane Datengrafikfelder (P_DG_FIELDS)
//
//   pane(app, ui, id)          the fields of the data linked to the shapes the commands work on
//                              (data_graphics::targets: the selection, else every shape of the
//                              page with linked data), one row each: a check box -- on while their
//                              data graphic has an item for the field; a click adds one (a number
//                              as a data bar, else as text; the gallery changes it) or takes it
//                              away -- and the field's
//                              name; the row chosen (a click on it, or the field just switched on)
//                              is the one the gallery, Position and Konfigurieren work on
//
// The heading names the data graphic the shapes show ("Keine Datengrafik" when none).  A data
// graphic is made the first time a field is switched on ("Daten – <Quelle>"), put on the shapes
// and changed afterwards for every shape that shows it (Visio's behaviour).

use alloc::string::String;

use diagram_core::datagraphic::{self as dg, Kind};
use ui::i18n::{t, tf};
use ui::*;

use crate::data_graphics::{self, change, chosen, default_callout, new_item};
use crate::Diagrammer;

/// the pane's box (panes.rs)
pub(crate) const ID_PANE_BOX: u32 = 0x7D10_0F00;
const ID_LIST: u32 = 0x7D10_0000;
const ID_CHECK: u32 = 0x7D10_1000;

/// the pane
pub(crate) fn pane(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let fields = data_graphics::fields(app);
    let graphic = data_graphics::graphic_of(app).and_then(|m| app.opened.as_ref().map(|o| dg::master_name(&o.doc, m)));
    ui.next_size(-1, -1);
    ui.box_begin(id, BOX_V | pad(10), 6);
    if fields.is_empty() {
        ui.empty_state(crate::icons::JD_DG_FIELDS, t("Keine Datenfelder"), t("Verknüpfen Sie Shapes mit Daten, um ihre Felder hier zu sehen."));
        ui.box_end();
        return;
    }
    ui.next_size(-1, 0);
    let head = match &graphic {
        Some(n) => tf("Datengrafik: {}", &[n]),
        None => String::from(t("Keine Datengrafik")),
    };
    ui.text(TS_BODY_STRONG | TX_WRAP, &head);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ein Haken zeigt das Feld als Datengrafik. Die Galerie, Position und Konfigurieren ändern das gewählte Feld."));
    let cur = chosen(app);
    let mut sel = cur.as_ref().and_then(|c| fields.iter().position(|f| &f.label == c)).map_or(-1, |k| k as i32);
    let mut toggled: Option<usize> = None;
    ui.next_size(-1, -1);
    let (a, b) = ui.list_begin(ID_LIST, &mut sel, fields.len() as i32, 30);
    for i in a..b {
        let f = &fields[i as usize];
        ui.list_item_begin(i);
        let mut v = f.on as i32;
        // the check box as wide as its label: a click beside it chooses the row only
        ui.ts_font(TS_BODY);
        let w = ui.text.width(&f.label);
        ui.next_size(ui.px2dip(w) + 30, 0);
        if ui.checkbox(ID_CHECK + i as u32, &f.label, &mut v) & EV_ACT != 0 {
            toggled = Some(i as usize);
        }
        ui.tooltip(&tf("„{}“ als Datengrafik zeigen", &[&f.label]));
        ui.list_item_end();
    }
    let (_, _) = ui.list_end(&mut sel);
    ui.box_end();
    if sel >= 0 {
        if let Some(f) = fields.get(sel as usize) {
            if app.dg.field.as_deref() != Some(f.label.as_str()) {
                app.dg.field = Some(f.label.clone());
                ui.repass();
            }
        }
    }
    if let Some(i) = toggled {
        let f = &fields[i];
        let label = f.label.clone();
        app.dg.field = Some(label.clone());
        if f.on {
            change(app, ui, t("Datengrafikfeld entfernen"), |g| {
                if let Some(k) = g.item_of(&dg::field_expr(&label)) {
                    g.items.remove(k);
                }
            });
        } else {
            // a number as a data bar, anything else as text (the gallery changes it)
            let kind = if f.numeric { Kind::DataBar } else { Kind::Text };
            let item = new_item(app, &label, kind, default_callout(kind));
            change(app, ui, t("Datengrafikfeld hinzufügen"), |g| {
                g.push(item);
            });
        }
    }
}

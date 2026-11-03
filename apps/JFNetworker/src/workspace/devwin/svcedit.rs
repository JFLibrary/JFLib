// svcedit -- the settings of a service in the Dienste tab of a device window: a form where its kind has
// one (svcform.rs), else its configuration as JSON in a code editor, checked before it applies, one
// undo step
//
//   SvcEdit::new()                     nothing open
//   button(n, ui, id, dev, j)          "Einstellungen …" on the card of the project's service
//                                      j: opens its form or editor, or closes it again
//   editor(n, ui, dev, j)              below that card while it is open: the form, or the
//                                      configuration (indented JSON), Übernehmen, Abbrechen,
//                                      what Übernehmen would do and why the text was refused
//   open_form(dev, &def) / close()     the form of a service opens; nothing stays open
//   to_json(n, &def)                   from the form to the editor, with what the form has so far
//   trial(&device, &def) -> Result     a simulator's verdict on a service's config (the form's too)
//
// Übernehmen reads the text as JSON (an error is marked in the editor and named with its line
// and column), then lets a simulator of its own build the service on a bare device of the same
// model (Engine::service_def), so a configuration the simulator refuses never reaches the
// project; then the change is one undo step ("Dienst web auf WEB1 einstellen"): the running service
// takes it, or the device starts again with it (doc/sim.rs, svcform::takes).  One editor is
// open at a time; it closes when its device or service goes.  The labs use it (LB backends,
// replication factors, cache sizes, workers: crates/jfn_sim's services name their members).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::json;
use jfn_model::project::{file_limits, Device, ServiceDef};
use jfn_model::Msg;
use jfn_sim::{Action, Engine};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;

/// the open editor: its device and service, the text, why it was refused; the form while it shows
/// instead of the text; the dialog "Dienst hinzufügen"
pub(crate) struct SvcEdit {
    pub at: Option<(u32, String)>,
    pub doc: CodeDoc,
    pub err: String,
    pub form: Option<super::svcform::Form>,
    pub add: super::svcform::AddDlg,
}

impl SvcEdit {
    pub(crate) fn new() -> SvcEdit {
        SvcEdit { at: None, doc: CodeDoc::new(), err: String::new(), form: None, add: super::svcform::AddDlg::new() }
    }

    /// the form of this service opens
    pub(crate) fn open_form(&mut self, dev: u32, def: &ServiceDef) {
        self.at = Some((dev, def.name.clone()));
        self.form = Some(super::svcform::Form::of(def));
        self.err.clear();
    }

    /// nothing stays open
    pub(crate) fn close(&mut self) {
        self.at = None;
        self.form = None;
    }

    /// the editor is open on service `name` of device `dev`
    pub(crate) fn is(&self, dev: u32, name: &str) -> bool {
        self.at.as_ref().is_some_and(|(d, s)| *d == dev && s.eq_ignore_ascii_case(name))
    }
}

/// the configuration as the editor shows it (indented JSON; nothing: an empty object)
fn text_of(def: &ServiceDef) -> String {
    let v = if def.config.as_object().is_some() { def.config.clone() } else { agent_core::jmap::empty_object() };
    json::stringify_pretty(&v, file_limits(), 2).unwrap_or_default()
}

/// a simulator of its own builds the service on a bare device of the model: Err with the
/// simulator's reason when it refuses the configuration
pub(crate) fn trial(d: &Device, def: &ServiceDef) -> Result<(), Msg> {
    let mut e = Engine::new(1);
    let bare = Device { services: Vec::new(), startup_config: String::new(), off: false, ..d.clone() };
    e.apply(Action::AddDevice(bare)).map_err(|x| x.msg())?;
    e.service_def(d.id, def).map_err(|x| x.msg())
}

/// "Einstellungen …" on the card of the project's service j (button id `id`)
pub(crate) fn button(n: &mut Net, ui: &mut Ui, id: u32, dev: u32, j: usize) {
    let Some(def) = n.doc.proj.device(dev).and_then(|d| d.services.get(j)).cloned() else { return };
    let open = n.devwin.svc.is(dev, &def.name);
    ui.next_flags(ST_SUBTLE | ST_COMPACT);
    ui.next_icon(IC_EDIT);
    if ui.button(id, if open { t("Einstellungen schließen") } else { t("Einstellungen …") }) & EV_ACT != 0 {
        let ed = &mut n.devwin.svc;
        if open {
            ed.close();
        } else if super::svcform::form_of(&def.kind).is_some() {
            ed.open_form(dev, &def);
        } else {
            ed.close();
            ed.at = Some((dev, def.name.clone()));
            ed.doc = CodeDoc::from_text(&text_of(&def));
            ed.err.clear();
            ui.focus(ID_SVC_EDIT);
        }
        ui.repass();
    }
}

/// from the form to the editor: the text has what the form has so far, if that makes a config
pub(crate) fn to_json(n: &mut Net, def: &ServiceDef) {
    let ed = &mut n.devwin.svc;
    let c = ed.form.as_ref().and_then(|f| super::svcform::merged(&def.kind, &def.config, &f.vals).ok());
    let shown = ServiceDef { config: c.unwrap_or_else(|| def.config.clone()), ..def.clone() };
    ed.form = None;
    ed.doc = CodeDoc::from_text(&text_of(&shown));
    ed.err.clear();
}

/// what the text would do, for the line above the editor
fn text_says(n: &Net, d: &Device, j: usize, def: &ServiceDef, text: &str) -> String {
    let Ok(v) = json::parse_with_limits(text.as_bytes(), file_limits()) else { return String::new() };
    match super::svcform::takes(&n.doc, d.id, j, &v) {
        super::svcform::Takes::Same => String::new(),
        super::svcform::Takes::Live => tf("Gilt sofort: {} läuft weiter, ohne Neustart.", &[&def.name]),
        super::svcform::Takes::Restart => tf("{} startet dafür neu.", &[&d.name]),
    }
}

/// the editor of the project's service j of device `dev` (below its card while it is open)
pub(crate) fn editor(n: &mut Net, ui: &mut Ui, dev: u32, j: usize) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let Some(def) = d.services.get(j).cloned() else { return };
    if !n.devwin.svc.is(dev, &def.name) {
        return;
    }
    if n.devwin.svc.form.is_some() {
        super::svcform::view(n, ui, dev, j);
        return;
    }
    let mut apply = false;
    let mut cancel = false;
    let says = text_says(n, &d, j, &def, &n.devwin.svc.doc.text());
    let past = n.doc.sim.past().is_some();
    let ed = &mut n.devwin.svc;
    ui.next_size(-1, 0);
    // (a kind without a form always restarts; for the others the line below says what happens)
    let head = if super::svcform::form_of(&def.kind).is_some() { tf("Die Einstellungen von {} als JSON. Übernehmen prüft sie.", &[&def.name]) } else { tf("Die Einstellungen von {} als JSON. Übernehmen prüft sie und startet {} mit ihnen neu.", &[&def.name, &d.name]) };
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &head);
    ui.next_size(-1, 240);
    ui.code_editor(ID_SVC_EDIT, &mut ed.doc, &Highlighter::json(), 0);
    if !says.is_empty() && super::svcform::form_of(&def.kind).is_some() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &says);
    }
    if !ed.err.is_empty() {
        let e = ed.err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(ID_SVC_EDIT_CANCEL, t("Abbrechen")) & EV_ACT != 0 {
        cancel = true;
    }
    if past {
        ui.next_flags(ST_DISABLED);
    }
    if ui.accent_button(ID_SVC_EDIT_APPLY, t("Übernehmen")) & EV_ACT != 0 {
        apply = true;
    }
    ui.box_end();
    if cancel {
        n.devwin.svc.close();
        ui.repass();
    }
    if apply {
        take(n, &d, j);
        ui.repass();
    }
}

// Übernehmen: the text as JSON, the simulator's trial, then one undo step
fn take(n: &mut Net, d: &Device, j: usize) {
    let ed = &mut n.devwin.svc;
    let text = ed.doc.text();
    let v = match json::parse_with_limits(text.as_bytes(), file_limits()) {
        Ok(v) => v,
        Err(e) => {
            // the place of the error: its line and column (characters, from 1)
            let mut at = e.offset.min(text.len());
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            let before = &text[..at];
            let line = before.matches('\n').count() + 1;
            let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
            ed.err = tf("Zeile {}, Spalte {}: kein gültiges JSON ({}).", &[&line.to_string(), &col.to_string(), e.kind]);
            ed.doc.set_diagnostics(&[Diag::new(line, col, col + 1, DIAG_ERROR, &ed.err)]);
            return;
        }
    };
    if v.as_object().is_none() {
        ed.err = String::from(t("Die Einstellungen sind ein JSON-Objekt: { … }."));
        return;
    }
    let def = ServiceDef { config: v.clone(), ..d.services[j].clone() };
    if let Err(m) = trial(d, &def) {
        ed.err = tr::msg(&m);
        ed.doc.clear_diagnostics();
        return;
    }
    ed.close();
    ed.err.clear();
    super::svcform::apply_config(n, d.id, j, v);
}

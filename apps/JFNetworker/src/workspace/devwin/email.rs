// email -- the E-Mail program on the Desktop of a host's device window: an account, the inbox and
// the messages sent, reading, writing, answering, sending and fetching through the simulator
//
//   MailUi::new()                  nothing chosen, the list
//   program(n, ui, k, id, dev)     the program of device window k (ids id .. id + 0x3FF)
//
// The account is the file /root/Mail/account of the device (jfn_sim::mailc::Account): Konto …
// shows it as a form (name, address, SMTP server and port, POP3 server and port, user,
// password, whether fetched messages stay on the server); Speichern writes the file as one undo
// step of the project (the simulator takes it without a restart, as every file).  The list
// shows the folder Posteingang (/root/Mail/inbox) or Gesendet (/root/Mail/sent) of the moment
// the window shows, newest first, the chosen message below it.  Abrufen and Senden go to the
// simulator as Action::MailFetch / MailSend (so rewinding replays them); the program in the
// engine (mailc.rs: MailClient) does the POP3 and SMTP sessions and puts the messages into the
// folders, which come back as steps of the project.  The two lines at the bottom say what the
// last send and the last fetch did.  Antworten fills a new message from the chosen one (to its
// sender, "Re: ", the text quoted); Löschen removes the chosen message from the folder (a step).
//
// Ids (id = ID_MAIL0 + 0x400 k): + 1 Abrufen, + 2 Verfassen, + 3 Antworten, + 4 Löschen, + 5 Konto,
// + 6 the folders (+ 7, + 8 their segments); + 0x10 .. + 0x1B the account's fields, Speichern and
// Abbrechen; + 0x20 .. + 0x25 the new message's fields, Senden and Verwerfen; + 0x30 the text
// shown; + 0x40 / + 0x41 the scroll views of the forms; + 0x100 the list (rows + 0x101 + i).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::files::Store;
use jfn_sim::mailc::{self, Account, MailDraft, MailEntry};
use jfn_sim::{Action, Cmd};
use netpkt::imf::{self, Message};
use ui::*;

use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::doc::edits;

/// what the program shows
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    List,
    Compose,
    Account,
}

/// the program's state in a device window
pub(crate) struct MailUi {
    pub view: View,
    /// 0 Posteingang, 1 Gesendet
    pub folder: i32,
    pub grid: GridState,
    // the new message
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub text: String,
    in_reply_to: String,
    references: String,
    // the account's form and what it started from
    f: [String; 8],
    keep: i32,
    src: Option<Account>,
    pub err: String,
    /// a button of a form, handled after the widgets
    pending: Option<u32>,
}

impl MailUi {
    pub(crate) fn new() -> MailUi {
        MailUi {
            view: View::List,
            folder: 0,
            grid: GridState::default(),
            to: String::new(),
            cc: String::new(),
            subject: String::new(),
            text: String::new(),
            in_reply_to: String::new(),
            references: String::new(),
            f: Default::default(),
            keep: 0,
            src: None,
            err: String::new(),
            pending: None,
        }
    }
    /// the form from an account
    fn fill(&mut self, a: &Account) {
        self.f = [a.name.clone(), a.address.clone(), a.smtp.clone(), a.smtp_port.to_string(), a.pop3.clone(), a.pop3_port.to_string(), a.user.clone(), a.password.clone()];
        self.keep = a.keep as i32;
        self.src = Some(a.clone());
    }
}

/// a key for a row of the list
fn key_of(p: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in p.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
    }
    h
}

/// "2026-01-01 00:05" of a message's date
fn when(t: Option<i64>) -> String {
    match t {
        Some(t) => {
            let c = datetime::Civil::from_unix(t);
            format!("{:04}-{:02}-{:02} {:02}:{:02}", c.year, c.month, c.day, c.hour, c.minute)
        }
        None => String::from("–"),
    }
}

/// the device's files as the window shows them (the moment of the view)
fn store_shown(n: &Net, dev: u32) -> Option<Store> {
    n.doc.sim.state().file_store(dev).cloned()
}

/// the device's files as the project has them (what a change starts from)
fn store_project(n: &Net, dev: u32) -> Option<Store> {
    let d = n.doc.proj.device(dev)?;
    let m = n.doc.proj.model_of(dev)?;
    let mut s = Store::for_model(m);
    s.load(&d.files);
    Some(s)
}

// i18n
const FOLDERS: [&str; 2] = ["Posteingang", "Gesendet"];

/// the E-Mail program of device window k
pub(crate) fn program(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let st = store_shown(n, dev);
    let acct = st.as_ref().and_then(Account::of);
    let state = n.doc.sim.state().mail_client(dev);
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.mail;
    // without an account the form comes first
    if acct.is_none() && s.view == View::List && s.src.is_none() {
        s.fill(&Account::parse(""));
        s.view = View::Account;
    }
    let dir = if s.folder == 1 { mailc::SENT } else { mailc::INBOX };
    let mut list: Vec<MailEntry> = st.as_ref().map(|x| mailc::folder(x, dir)).unwrap_or_default();
    list.reverse();
    let keys: Vec<u64> = list.iter().map(|m| key_of(&m.path)).collect();
    s.grid.retain(&keys);
    let cur = s.grid.cur.and_then(|c| list.iter().find(|m| key_of(&m.path) == c)).cloned();
    let busy = state.as_ref().and_then(|v| v.busy);
    let mut act: Option<u32> = None;
    // the tool row
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
    ui.next_icon(IC_REFRESH);
    ui.next_flags(if acct.is_some() { 0 } else { ST_DISABLED });
    if ui.button(id + 1, t("Abrufen")) & EV_ACT != 0 {
        act = Some(1);
    }
    ui.tooltip(t("Neue Nachrichten per POP3 vom Server holen"));
    ui.next_icon(IC_ADD);
    ui.next_flags(if acct.is_some() { 0 } else { ST_DISABLED });
    if ui.button(id + 2, t("Verfassen")) & EV_ACT != 0 {
        act = Some(2);
    }
    let can = cur.is_some() && s.view == View::List;
    ui.next_icon(IC_RETURN);
    ui.next_flags(if can && s.folder == 0 && acct.is_some() { 0 } else { ST_DISABLED });
    if ui.button(id + 3, t("Antworten")) & EV_ACT != 0 {
        act = Some(3);
    }
    ui.next_icon(IC_DELETE);
    ui.next_flags(if can { 0 } else { ST_DISABLED });
    if ui.button(id + 4, t("Löschen")) & EV_ACT != 0 {
        act = Some(4);
    }
    ui.next_icon(IC_PERSON);
    ui.next_flags(ST_SUBTLE);
    if ui.button(id + 5, t("Konto …")) & EV_ACT != 0 {
        act = Some(5);
    }
    ui.box_end();
    match s.view {
        View::List => {
            let mut sel = s.folder;
            let names: Vec<String> = FOLDERS.iter().enumerate().map(|(i, f)| {
                let c = if i as i32 == s.folder { list.len() } else { st.as_ref().map_or(0, |x| mailc::folder(x, if i == 1 { mailc::SENT } else { mailc::INBOX }).len()) };
                format!("{} ({})", t(f), c)
            }).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            if ui.segmented(id + 6, &mut sel, &refs) & EV_ACT != 0 {
                s.folder = sel;
                s.grid.select_only(None);
                ui.repass();
            }
            if list.is_empty() {
                ui.next_size(-1, 120);
                ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
                let (title, text) = if s.folder == 0 { (t("Keine Nachrichten"), t("Abrufen holt die Nachrichten des Kontos vom POP3-Server.")) } else { (t("Nichts gesendet"), t("Verfassen schreibt eine neue Nachricht.")) };
                ui.empty_state(IC_MAIL, title, text);
                ui.box_end();
            } else {
                let cols = [
                    GridCol::new(if s.folder == 0 { t("Von") } else { t("An") }, 160).flags(GC_NOSORT),
                    GridCol::new(t("Betreff"), 220).flags(GC_NOSORT | GC_FILL),
                    GridCol::new(t("Datum"), 130).flags(GC_NOSORT),
                ];
                ui.next_size(-1, 150);
                let (first, end) = ui.grid_begin(id + 0x100, &mut s.grid, &cols, &keys);
                for i in first..end {
                    let m = &list[i as usize];
                    ui.grid_row(&mut s.grid, i);
                    ui.grid_text(&s.grid, 0, if s.folder == 0 { &m.from } else { &m.to });
                    ui.grid_text(&s.grid, 1, if m.subject.is_empty() { t("(ohne Betreff)") } else { &m.subject });
                    ui.grid_text(&s.grid, 2, &when(m.date));
                }
                ui.grid_end(&mut s.grid);
            }
            // the chosen message
            if let Some(m) = cur.as_ref().and_then(|c| st.as_ref().and_then(|x| mailc::read_mail(x, &c.path))) {
                // the header's lines, then the text
                let boxes = |v: Vec<imf::Mailbox>| v.iter().map(|x| imf::decode_words(&x.to_header())).collect::<Vec<_>>().join(", ");
                let rows = [
                    (t("Von"), m.from().map(|x| imf::decode_words(&x.to_header())).unwrap_or_default()),
                    (t("An"), boxes(m.to())),
                    (t("Cc"), boxes(m.cc())),
                    (t("Datum"), when(m.date())),
                    (t("Betreff"), m.subject()),
                ];
                ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 2);
                for (k, v) in rows.iter().filter(|r| !r.1.is_empty()) {
                    ui.box_begin(0, BOX_H, 8);
                    ui.next_size(70, 0);
                    ui.text(TS_CAPTION | TX_SECONDARY, k);
                    ui.next_size(-1, 0);
                    ui.text(TS_BODY | TX_ELLIPSIS, v);
                    ui.box_end();
                }
                ui.box_end();
                let mut text = m.text();
                ui.next_size(-1, -1);
                ui.textbox(id + 0x30, None, &mut text, 1_000_000 | TB_MULTI | TB_WRAP | TB_READONLY);
            } else if !list.is_empty() {
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_SECONDARY, t("Eine Nachricht wählen, um sie zu lesen."));
            }
        }
        View::Compose => compose(ui, id, s),
        View::Account => account(ui, id, s, acct.is_some()),
    }
    // what the last send and fetch did
    let line = |ui: &mut Ui, what: &str, o: &Option<jfn_sim::MailOutcome>, running: bool| {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        match (running, o) {
            (true, _) => {
                ui.status_dot(BADGE_INFO);
                ui.text(TS_CAPTION | TX_SECONDARY, &tf("{}: läuft …", &[what]));
            }
            (false, Some(o)) => {
                ui.status_dot(if o.ok { BADGE_OK } else { BADGE_CRIT });
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &format!("{}: {}", what, tr::msg(&o.msg)));
            }
            (false, None) => {
                ui.status_dot(BADGE_NEUTRAL);
                ui.text(TS_CAPTION | TX_TERTIARY, &tf("{}: noch nicht", &[what]));
            }
        }
        ui.box_end();
    };
    let (ls, lf) = (state.as_ref().and_then(|v| v.last_send.clone()), state.as_ref().and_then(|v| v.last_fetch.clone()));
    line(ui, t("Senden"), &ls, busy == Some("send"));
    line(ui, t("Abrufen"), &lf, busy == Some("fetch"));
    let err = n.devwin.get(k).map(|w| w.desk.mail.err.clone()).unwrap_or_default();
    if !err.is_empty() {
        crate::workspace::error_text(ui, &err, 0);
    }
    // what the buttons and forms asked for, after the widgets
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.mail;
    let todo = act.or(s.pending.take());
    match todo {
        Some(1) => {
            s.err.clear();
            if let Err(e) = n.doc.sim.send(Cmd::Apply(Action::MailFetch { dev })) {
                if let Some(w) = n.devwin.get(k) {
                    w.desk.mail.err = tr::msg(&e.msg());
                }
            }
            ui.repass();
        }
        Some(2) => {
            *s = MailUi { view: View::Compose, folder: s.folder, src: s.src.take(), f: core::mem::take(&mut s.f), keep: s.keep, ..MailUi::new() };
            ui.focus(id + 0x20);
            ui.repass();
        }
        Some(3) => {
            if let Some(m) = cur.as_ref().and_then(|c| st.as_ref().and_then(|x| mailc::read_mail(x, &c.path))) {
                answer(s, &m);
                ui.focus(id + 0x23);
            }
            ui.repass();
        }
        Some(4) => {
            if let Some(c) = cur {
                remove(n, dev, &c.path);
            }
            ui.repass();
        }
        Some(5) => {
            s.err.clear();
            if s.view == View::Account {
                s.view = View::List;
            } else {
                s.fill(&acct.clone().unwrap_or_else(|| Account::parse("")));
                s.view = View::Account;
            }
            ui.repass();
        }
        Some(0x24) => {
            send(n, k, dev);
            ui.repass();
        }
        Some(0x25) => {
            s.view = View::List;
            s.err.clear();
            ui.repass();
        }
        Some(0x1A) => {
            save(n, k, dev);
            ui.repass();
        }
        Some(0x1B) => {
            s.view = View::List;
            s.err.clear();
            ui.repass();
        }
        _ => {}
    }
}

/// a "label  field" row (the field follows, then box_end)
fn field(ui: &mut Ui, label: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(150, 0);
    ui.text(TS_BODY | TX_SECONDARY, label);
}

/// the new message: An, Cc, Betreff, the text, Senden and Verwerfen
fn compose(ui: &mut Ui, id: u32, s: &mut MailUi) {
    ui.next_size(-1, -1);
    ui.box_begin(id + 0x41, BOX_V | pad(4), 8);
    field(ui, t("An"));
    ui.next_size(-1, 0);
    ui.textbox(id + 0x20, Some(t("bob@b.lab, Name <adresse@domäne>")), &mut s.to, 2000);
    ui.box_end();
    field(ui, t("Cc"));
    ui.next_size(-1, 0);
    ui.textbox(id + 0x21, None, &mut s.cc, 2000);
    ui.box_end();
    field(ui, t("Betreff"));
    ui.next_size(-1, 0);
    ui.textbox(id + 0x22, None, &mut s.subject, 500);
    ui.box_end();
    ui.next_size(-1, -1);
    ui.textbox(id + 0x23, Some(t("Der Text der Nachricht")), &mut s.text, 64_000 | TB_MULTI | TB_WRAP);
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(id + 0x25, t("Verwerfen")) & EV_ACT != 0 {
        s.pending = Some(0x25);
    }
    ui.next_icon(IC_MAIL);
    let ready = !s.to.trim().is_empty() || !s.cc.trim().is_empty();
    ui.next_flags(if ready { 0 } else { ST_DISABLED });
    if ui.accent_button(id + 0x24, t("Senden")) & EV_ACT != 0 {
        s.pending = Some(0x24);
    }
    ui.box_end();
    ui.box_end();
}

/// the account's form
fn account(ui: &mut Ui, id: u32, s: &mut MailUi, have: bool) {
    // i18n
    const LABELS: [&str; 8] = ["Name", "E-Mail-Adresse", "SMTP-Server", "SMTP-Port", "POP3-Server", "POP3-Port", "Benutzer", "Passwort"];
    // examples in the fields (names and addresses: no words to translate)
    const HINTS: [&str; 8] = ["Anna Muster", "anna@a.lab", "mail.a.lab", "", "mail.a.lab", "110", "anna", ""];
    ui.next_size(-1, -1);
    ui.scroll_begin(id + 0x40, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 8);
    if !have {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Noch kein Konto: die Angaben des Mailservers eintragen, dann Speichern."));
    }
    for (i, l) in LABELS.iter().enumerate() {
        field(ui, t(l));
        ui.next_size(240, 0);
        let flags = match i {
            3 | 5 => 6 | TB_DIGITS,
            7 => 200 | TB_PASSWORD,
            _ => 200,
        };
        let hint = if i == 3 { Some(t("25 (587 mit Anmeldung)")) } else if HINTS[i].is_empty() { None } else { Some(HINTS[i]) };
        ui.textbox(id + 0x10 + i as u32, hint, &mut s.f[i], flags);
        ui.box_end();
    }
    field(ui, "");
    ui.checkbox(id + 0x19, t("Nachrichten auf dem Server lassen"), &mut s.keep);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(id + 0x1B, t("Abbrechen")) & EV_ACT != 0 {
        s.pending = Some(0x1B);
    }
    ui.next_icon(IC_SAVE);
    if ui.accent_button(id + 0x1A, t("Speichern")) & EV_ACT != 0 {
        s.pending = Some(0x1A);
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Das Konto liegt als /root/Mail/account auf dem Gerät; empfangene Nachrichten in /root/Mail/inbox, gesendete in /root/Mail/sent."));
    ui.box_end();
    ui.scroll_end();
}

/// a reply to message m: to its sender, "Re: ", the text quoted below
fn answer(s: &mut MailUi, m: &Message) {
    let to = m.get("Reply-To").and_then(|v| imf::parse_list(v).into_iter().next()).or_else(|| m.from());
    let subject = m.subject();
    let low = subject.to_ascii_lowercase();
    let subject = if low.starts_with("re:") || low.starts_with("aw:") { subject } else { format!("Re: {}", subject) };
    let who = m.from().map(|f| String::from(f.shown())).unwrap_or_default();
    let quoted: Vec<String> = m.text().lines().map(|l| format!("> {}", l)).collect();
    let id = String::from(m.get("Message-ID").unwrap_or(""));
    let refs = format!("{} {}", m.get("References").unwrap_or(""), id);
    *s = MailUi {
        view: View::Compose,
        folder: s.folder,
        to: to.map(|x| x.to_header()).unwrap_or_default(),
        subject,
        text: format!("\n\n{}\n{}\n", tf("Am {} schrieb {}:", &[&when(m.date()), &who]), quoted.join("\n")),
        in_reply_to: id,
        references: String::from(refs.trim()),
        src: s.src.take(),
        f: core::mem::take(&mut s.f),
        keep: s.keep,
        ..MailUi::new()
    };
}

/// Senden: the message to the simulator's E-Mail program
fn send(n: &mut Net, k: usize, dev: u32) {
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.mail;
    let draft = MailDraft { to: s.to.clone(), cc: s.cc.clone(), subject: s.subject.clone(), text: s.text.clone(), in_reply_to: s.in_reply_to.clone(), references: s.references.clone() };
    let r = n.doc.sim.send(Cmd::Apply(Action::MailSend { dev, draft }));
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.mail;
    match r {
        Ok(_) => {
            *s = MailUi { view: View::List, folder: 1, src: s.src.take(), f: core::mem::take(&mut s.f), keep: s.keep, ..MailUi::new() };
        }
        Err(e) => s.err = tr::msg(&e.msg()),
    }
}

/// Speichern: the account file, one step of the project
fn save(n: &mut Net, k: usize, dev: u32) {
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.mail;
    let f = s.f.clone();
    let keep = s.keep != 0;
    let port = |v: &str, def: u16| -> Option<u16> {
        if v.trim().is_empty() {
            return Some(def);
        }
        v.trim().parse::<u16>().ok().filter(|&p| p > 0)
    };
    let err = if !imf::addr_ok(f[1].trim()) {
        Some(tf("„{}“ ist keine E-Mail-Adresse.", &[f[1].trim()]))
    } else if f[2].trim().is_empty() || f[4].trim().is_empty() {
        Some(String::from(t("Das Konto braucht einen SMTP- und einen POP3-Server.")))
    } else if port(&f[3], 25).is_none() || port(&f[5], 110).is_none() {
        Some(String::from(t("Ein Port ist eine Zahl von 1 bis 65535.")))
    } else {
        None
    };
    if let Some(e) = err {
        s.err = e;
        return;
    }
    let a = Account {
        name: String::from(f[0].trim()),
        address: String::from(f[1].trim()),
        smtp: String::from(f[2].trim()),
        smtp_port: port(&f[3], 25).unwrap_or(25),
        pop3: String::from(f[4].trim()),
        pop3_port: port(&f[5], 110).unwrap_or(110),
        user: String::from(f[6].trim()),
        password: f[7].clone(),
        keep,
    };
    s.err.clear();
    s.view = View::List;
    s.src = Some(a.clone());
    let Some(mut st) = store_project(n, dev) else { return };
    let t0 = n.doc.sim.engine().clock(dev).map_or(0, |c| c.div_euclid(1_000_000_000));
    let dir = st.parent(mailc::ACCOUNT);
    if !st.is_dir(&dir) && st.mkdir(&dir, t0, true).is_err() {
        return;
    }
    if st.write(mailc::ACCOUNT, a.to_text().as_bytes(), t0).is_err() {
        if let Some(w) = n.devwin.get(k) {
            w.desk.mail.err = String::from(t("Das Konto passt nicht mehr auf das Gerät."));
        }
        return;
    }
    let records = st.records();
    let name = n.doc.proj.device(dev).map(|d| d.name.clone()).unwrap_or_default();
    let label = format!("E-Mail-Konto auf {} speichern", name);
    if let Some(e) = edits::device_edit(&n.doc.proj, dev, &label, |x| x.files = records) {
        n.doc.apply(e);
    }
}

/// Löschen: the message goes from the folder (one step)
fn remove(n: &mut Net, dev: u32, path: &str) {
    let Some(name) = n.doc.proj.device(dev).map(|d| d.name.clone()) else { return };
    let label = format!("Nachricht {} auf {} löschen", path, name);
    if let Some(e) = edits::device_edit(&n.doc.proj, dev, &label, |x| x.files.retain(|f| f.path != path)) {
        n.doc.apply(e);
    }
}

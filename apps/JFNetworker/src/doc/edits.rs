// edits -- the undoable changes of a project (ui::Edit over jfn_model::Project)
//
//   add_device(p, model, x, y) -> (id, edit)   a new device (free name: PC1, SW2, ...), applied
//   add_note(p, x, y) -> (id, edit)            a new empty note, applied
//   moves(p, &[(id, x, y)], to) -> edit        devices and notes to new positions (a drag)
//   delete(p, ids) -> edit                     devices with their links, links, notes; undo
//                                              puts everything back where it was in the lists
//   rename(p, id, name) -> Result<edit>        jfn_model's name rules (valid_name, unique),
//                                              applied when it is allowed
//   note_text(p, id, text) / title(p, text)    the text of a note, the project's title
//   connect(p, a, b, cable) -> Result<(id, edit)>   a new link (jfn_model's cable rules: None
//                                              takes the right cable), applied
//   link_edit(p, id, label, f) -> edit         any change of one link: kind, speed, delay,
//                                              loss, plug, route, waypoints (not applied yet)
//   link_since(p, before, label) -> edit       a link changed live (a waypoint drag): the step
//                                              from `before` to what it is now
//   device_edit(p, id, label, f) -> edit       any change of one device: power, configuration,
//                                              services (not applied yet)
//
// Ids are the project's: devices, links and notes share one counter, so an id names exactly
// one thing.  Positions are world units (DIP at 100 %), a device's x, y its symbol's top left.
// The simulator follows every step (doc/sim.rs): these edits only change the project.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::{CableKind, Device, Link, Note, PortRef, ProjErr, Project};
use ui::Edit;

/// something on the canvas
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Device(u32),
    Note(u32),
    Link(u32),
}

/// what the id names: a device, a note, a link or nothing
pub(crate) fn item(p: &Project, id: u32) -> Option<Item> {
    if p.device(id).is_some() {
        Some(Item::Device(id))
    } else if p.note(id).is_some() {
        Some(Item::Note(id))
    } else if p.link(id).is_some() {
        Some(Item::Link(id))
    } else {
        None
    }
}

/// the position of a device or note
pub(crate) fn pos(p: &Project, id: u32) -> Option<[i32; 2]> {
    match item(p, id)? {
        Item::Device(_) => p.device(id).map(|d| [d.x, d.y]),
        Item::Note(_) => p.note(id).map(|n| [n.x, n.y]),
        Item::Link(_) => None,
    }
}

/// put a device or note at x, y (no undo step: live moves, edits)
pub(crate) fn set_pos(p: &mut Project, id: u32, at: [i32; 2]) {
    if !p.move_device(id, at[0], at[1]) {
        if let Some(n) = p.notes.iter_mut().find(|n| n.id == id) {
            n.x = at[0];
            n.y = at[1];
        }
    }
}

// ---------------------------------------------------------------- add

struct AddDevice {
    dev: Device,
    label: String,
}

impl Edit<Project> for AddDevice {
    fn apply(&mut self, p: &mut Project) {
        let _ = p.insert_device(self.dev.clone());
    }
    fn revert(&mut self, p: &mut Project) {
        p.remove_device(self.dev.id);
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// a new device of a catalog model at x, y (applied) -> its id and the undo step
pub(crate) fn add_device(p: &mut Project, model: &str, x: i32, y: i32) -> Result<(u32, Box<dyn Edit<Project>>), ProjErr> {
    let id = p.add_device(model, x, y)?;
    let dev = p.device(id).cloned().unwrap_or_default();
    let label = format!("{} hinzufügen", dev.name);
    Ok((id, Box::new(AddDevice { dev, label })))
}

struct AddNote {
    note: Note,
}

impl Edit<Project> for AddNote {
    fn apply(&mut self, p: &mut Project) {
        if p.note(self.note.id).is_none() {
            p.notes.push(self.note.clone());
            p.next_id = p.next_id.max(self.note.id + 1);
        }
    }
    fn revert(&mut self, p: &mut Project) {
        p.remove_note(self.note.id);
    }
    fn label(&self) -> &str {
        "Notiz hinzufügen"
    }
}

/// a new empty note with its top left corner at x, y (applied)
pub(crate) fn add_note(p: &mut Project, x: i32, y: i32) -> (u32, Box<dyn Edit<Project>>) {
    let id = p.add_note(x, y, "");
    let note = p.note(id).cloned().unwrap_or_default();
    (id, Box::new(AddNote { note }))
}

// ---------------------------------------------------------------- move

struct Moves {
    /// (id, from, to)
    moves: Vec<(u32, [i32; 2], [i32; 2])>,
    label: String,
}

impl Edit<Project> for Moves {
    fn apply(&mut self, p: &mut Project) {
        for &(id, _, to) in &self.moves {
            set_pos(p, id, to);
        }
    }
    fn revert(&mut self, p: &mut Project) {
        for &(id, from, _) in &self.moves {
            set_pos(p, id, from);
        }
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// devices and notes from their positions `from` (id, x, y) to where they are now in p (the
/// move is applied already); None when nothing moved
pub(crate) fn moves(p: &Project, from: &[(u32, [i32; 2])]) -> Option<Box<dyn Edit<Project>>> {
    let moves: Vec<(u32, [i32; 2], [i32; 2])> = from
        .iter()
        .filter_map(|&(id, a)| pos(p, id).filter(|&b| b != a).map(|b| (id, a, b)))
        .collect();
    if moves.is_empty() {
        return None;
    }
    let label = if moves.len() == 1 {
        let name = p.device(moves[0].0).map_or_else(|| String::from("Notiz"), |d| d.name.clone());
        format!("{} verschieben", name)
    } else {
        format!("{} Objekte verschieben", moves.len())
    };
    Some(Box::new(Moves { moves, label }))
}

// ---------------------------------------------------------------- delete

struct Delete {
    /// what was removed with its index in the project's lists (ascending)
    devs: Vec<(usize, Device)>,
    links: Vec<(usize, Link)>,
    notes: Vec<(usize, Note)>,
    label: String,
}

impl Edit<Project> for Delete {
    fn apply(&mut self, p: &mut Project) {
        let (d, l, n) = (&self.devs, &self.links, &self.notes);
        p.devices.retain(|x| !d.iter().any(|(_, y)| y.id == x.id));
        p.links.retain(|x| !l.iter().any(|(_, y)| y.id == x.id));
        p.notes.retain(|x| !n.iter().any(|(_, y)| y.id == x.id));
    }
    fn revert(&mut self, p: &mut Project) {
        // ascending indices: each one is where the item was among the ones before it
        for (i, d) in &self.devs {
            if p.device(d.id).is_none() {
                p.devices.insert((*i).min(p.devices.len()), d.clone());
            }
        }
        for (i, l) in &self.links {
            if p.link(l.id).is_none() {
                p.links.insert((*i).min(p.links.len()), l.clone());
            }
        }
        for (i, n) in &self.notes {
            if p.note(n.id).is_none() {
                p.notes.insert((*i).min(p.notes.len()), n.clone());
            }
        }
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// removes the devices (with every link on them), links and notes of `ids` -> the undo step
/// (not applied yet), None when no id names one
pub(crate) fn delete(p: &Project, ids: &[u32]) -> Option<Box<dyn Edit<Project>>> {
    let devs: Vec<(usize, Device)> = p.devices.iter().enumerate().filter(|(_, d)| ids.contains(&d.id)).map(|(i, d)| (i, d.clone())).collect();
    let gone = |dev: u32| devs.iter().any(|(_, d)| d.id == dev);
    let links: Vec<(usize, Link)> = p
        .links
        .iter()
        .enumerate()
        .filter(|(_, l)| ids.contains(&l.id) || gone(l.a.dev) || gone(l.b.dev))
        .map(|(i, l)| (i, l.clone()))
        .collect();
    let notes: Vec<(usize, Note)> = p.notes.iter().enumerate().filter(|(_, n)| ids.contains(&n.id)).map(|(i, n)| (i, n.clone())).collect();
    if devs.is_empty() && links.is_empty() && notes.is_empty() {
        return None;
    }
    let label = match (devs.len(), notes.len()) {
        (1, 0) => format!("{} löschen", devs[0].1.name),
        (0, 1) if links.is_empty() => String::from("Notiz löschen"),
        (0, 0) if links.len() == 1 => String::from("Kabel trennen"),
        (0, 0) => format!("{} Kabel trennen", links.len()),
        (d, n) => format!("{} Objekte löschen", d + n + links.iter().filter(|(_, l)| ids.contains(&l.id)).count()),
    };
    Some(Box::new(Delete { devs, links, notes, label }))
}

// ---------------------------------------------------------------- rename, texts

struct Rename {
    id: u32,
    from: String,
    to: String,
    label: String,
}

impl Edit<Project> for Rename {
    fn apply(&mut self, p: &mut Project) {
        if let Some(d) = p.device_mut(self.id) {
            d.name = self.to.clone();
        }
    }
    fn revert(&mut self, p: &mut Project) {
        if let Some(d) = p.device_mut(self.id) {
            d.name = self.from.clone();
        }
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// gives a device a new host name (jfn_model's rules) -> the undo step, None when the name is
/// the same; applied when it is Ok
pub(crate) fn rename(p: &mut Project, id: u32, name: &str) -> Result<Option<Box<dyn Edit<Project>>>, ProjErr> {
    let from = p.device(id).map(|d| d.name.clone()).ok_or(ProjErr::UnknownDevice(id))?;
    p.rename(id, name)?;
    let to = p.device(id).map(|d| d.name.clone()).unwrap_or_default();
    if to == from {
        return Ok(None);
    }
    let label = format!("{} umbenennen", from);
    Ok(Some(Box::new(Rename { id, from, to, label })))
}

struct NoteText {
    id: u32,
    from: String,
    to: String,
}

impl Edit<Project> for NoteText {
    fn apply(&mut self, p: &mut Project) {
        if let Some(n) = p.notes.iter_mut().find(|n| n.id == self.id) {
            n.text = self.to.clone();
        }
    }
    fn revert(&mut self, p: &mut Project) {
        if let Some(n) = p.notes.iter_mut().find(|n| n.id == self.id) {
            n.text = self.from.clone();
        }
    }
    fn label(&self) -> &str {
        "Notiz bearbeiten"
    }
}

/// the new text of a note -> the undo step (not applied yet), None when it is the same
pub(crate) fn note_text(p: &Project, id: u32, text: &str) -> Option<Box<dyn Edit<Project>>> {
    let from = p.note(id)?.text.clone();
    if from == text {
        return None;
    }
    Some(Box::new(NoteText { id, from, to: String::from(text) }))
}

struct Title {
    from: String,
    to: String,
}

impl Edit<Project> for Title {
    fn apply(&mut self, p: &mut Project) {
        p.title = self.to.clone();
    }
    fn revert(&mut self, p: &mut Project) {
        p.title = self.from.clone();
    }
    fn label(&self) -> &str {
        "Projekttitel ändern"
    }
}

/// the project's new title -> the undo step (not applied yet), None when it is the same
pub(crate) fn title(p: &Project, text: &str) -> Option<Box<dyn Edit<Project>>> {
    let to = String::from(text.trim());
    if p.title == to {
        return None;
    }
    Some(Box::new(Title { from: p.title.clone(), to }))
}

// ---------------------------------------------------------------- links

struct AddLink {
    link: Link,
    label: String,
}

impl Edit<Project> for AddLink {
    fn apply(&mut self, p: &mut Project) {
        let _ = p.insert_link(self.link.clone());
    }
    fn revert(&mut self, p: &mut Project) {
        p.disconnect(self.link.id);
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// a link between two ports (cable None: the right one for them, "Automatisch") -> its id and
/// the undo step; applied when jfn_model allows it
pub(crate) fn connect(p: &mut Project, a: &PortRef, b: &PortRef, cable: Option<CableKind>) -> Result<(u32, Box<dyn Edit<Project>>), ProjErr> {
    let id = p.connect(a, b, cable)?;
    let link = p.link(id).cloned().unwrap_or_default();
    let name = |d: u32| p.device(d).map_or_else(String::new, |d| d.name.clone());
    let label = format!("{} mit {} verbinden", name(link.a.dev), name(link.b.dev));
    Ok((id, Box::new(AddLink { link, label })))
}

/// one link before and after a change (kind, quality, plug, route, waypoints)
struct LinkEdit {
    from: Link,
    to: Link,
    label: String,
}

fn put_link(p: &mut Project, l: &Link) {
    if let Some(x) = p.link_mut(l.id) {
        *x = l.clone();
    }
}

impl Edit<Project> for LinkEdit {
    fn apply(&mut self, p: &mut Project) {
        put_link(p, &self.to);
    }
    fn revert(&mut self, p: &mut Project) {
        put_link(p, &self.from);
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// a change of link `id` made by f -> the undo step (not applied yet), None when f changes
/// nothing
pub(crate) fn link_edit(p: &Project, id: u32, label: &str, f: impl FnOnce(&mut Link)) -> Option<Box<dyn Edit<Project>>> {
    let from = p.link(id)?.clone();
    let mut to = from.clone();
    f(&mut to);
    if to == from {
        return None;
    }
    Some(Box::new(LinkEdit { from, to, label: String::from(label) }))
}

/// a link that was changed live (a drag of a waypoint): the step from `before` to what it is
/// now (applied already), None when it is the same
pub(crate) fn link_since(p: &Project, before: &Link, label: &str) -> Option<Box<dyn Edit<Project>>> {
    let to = p.link(before.id)?.clone();
    if &to == before {
        return None;
    }
    Some(Box::new(LinkEdit { from: before.clone(), to, label: String::from(label) }))
}

// ---------------------------------------------------------------- devices

/// one device before and after a change (power, configuration, services)
struct DeviceEdit {
    from: Device,
    to: Device,
    label: String,
}

fn put_device(p: &mut Project, d: &Device) {
    if let Some(x) = p.device_mut(d.id) {
        *x = d.clone();
    }
}

impl Edit<Project> for DeviceEdit {
    fn apply(&mut self, p: &mut Project) {
        put_device(p, &self.to);
    }
    fn revert(&mut self, p: &mut Project) {
        put_device(p, &self.from);
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// a change of device `id` made by f (not its name or position: rename, moves) -> the undo
/// step (not applied yet), None when f changes nothing
pub(crate) fn device_edit(p: &Project, id: u32, label: &str, f: impl FnOnce(&mut Device)) -> Option<Box<dyn Edit<Project>>> {
    let from = p.device(id)?.clone();
    let mut to = from.clone();
    f(&mut to);
    if to == from {
        return None;
    }
    Some(Box::new(DeviceEdit { from, to, label: String::from(label) }))
}

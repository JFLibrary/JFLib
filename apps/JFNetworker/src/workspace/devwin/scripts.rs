// scripts -- what is left of the Skripte tab around the IDE (workspace/ide), which lives in the
// desktop's IDE window (devwin/desktop.rs ide_window, started maximised)
//
//   has_tab(project, dev)         a host with a Linux shell has the IDE (and a desktop)
//   states(n, ui) / command(n, ui, id) -> handled   CMD_SCRIPTS: the selected host's window on
//                                 its desktop with the IDE in front (the context menu of a
//                                 device, "IDE …")
//   check_due(n, ui)              TIMER_SCRIPT_CHECK: the IDE's check of the changed drafts
//   map_line(run, now, line) / word_at(text, col)   where a line of the program that runs is
//                                 in the edited text, the word of a column (the IDE's marks of
//                                 runtime errors)

use alloc::string::String;

use jfn_model::catalog::{CliStyle, F_HOST};
use jfn_model::Project;
use ui::*;

use crate::app::constants::*;
use crate::app::Net;
use crate::workspace::devwin;

/// device `dev` has the IDE: a host with a Linux shell (the devices whose OS runs scripts and
/// that have a desktop)
pub(crate) fn has_tab(p: &Project, dev: u32) -> bool {
    p.model_of(dev).is_some_and(|m| m.has(F_HOST) && m.cli == CliStyle::Linux)
}

/// TIMER_SCRIPT_CHECK: the IDE's check while typing
pub(crate) fn check_due(n: &mut Net, ui: &mut Ui) {
    crate::workspace::ide::check_due(n, ui);
}

/// where line `line` (from 1) of the running source is in the edited text: lines before the
/// first difference stay, those after the last one move with it, a changed one is gone
pub(crate) fn map_line(run: &[&str], now: &[String], line: usize) -> Option<usize> {
    let pre = run.iter().zip(now).take_while(|(a, b)| **a == b.as_str()).count();
    let post = run[pre..].iter().rev().zip(now[pre..].iter().rev()).take_while(|(a, b)| **a == b.as_str()).count();
    let i = line.checked_sub(1)?;
    if i < pre {
        Some(line)
    } else if i >= run.len() - post && i < run.len() {
        Some((i as isize + now.len() as isize - run.len() as isize) as usize + 1)
    } else {
        None
    }
}

/// the columns (from 1, characters; the end exclusive) of the word that starts at column `col`
/// of `text` (one character when it is none)
pub(crate) fn word_at(text: &str, col: usize) -> (usize, usize) {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let n = text.chars().skip(col.saturating_sub(1)).take_while(|&c| word(c)).count();
    (col.max(1), col.max(1) + n.max(1))
}

// ---------------------------------------------------------------- the command

// the selected device when it is one host
fn selected_host(n: &Net) -> Option<u32> {
    match crate::workspace::canvas::single_item(n)? {
        crate::doc::edits::Item::Device(d) if has_tab(&n.doc.proj, d) => Some(d),
        _ => None,
    }
}

/// CMD_SCRIPTS: hidden unless one host is selected
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    ui.cmd_state(CMD_SCRIPTS, if selected_host(n).is_some() { 0 } else { CF_DISABLED | CF_HIDDEN });
}

/// CMD_SCRIPTS -> handled
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32) -> bool {
    if id != CMD_SCRIPTS {
        return false;
    }
    if let Some(d) = selected_host(n) {
        devwin::desktop::open_ide(n, ui, d);
    }
    ui.repass();
    true
}

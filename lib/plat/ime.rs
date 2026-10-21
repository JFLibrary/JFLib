//! Bounded preedit snapshots. Composition is separate from committed E_TEXT events.
use alloc::{string::String, vec::Vec};
use rt::sync::SpinLock;
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Preedit { pub text: String, pub cursor: usize }
static PREEDIT: SpinLock<Vec<(u8,Preedit)>> = SpinLock::new(Vec::new());
static COMMITS: SpinLock<(u32,Vec<(u32,String)>)> = SpinLock::new((0,Vec::new()));
/// One composition commit is one event, avoiding truncation by the character ring.
pub fn ime_commit_set(win:u8,text:&str) -> bool {
    if win as usize>=crate::PW_MAX || text.len()>4096 { return false; }
    if text.is_empty() { return true; }
    let mut q=COMMITS.lock();
    // Keep already queued input intact when the bounded payload queue is full.
    if q.1.len()>=16 { return false; }
    q.0=q.0.wrapping_add(1); let token=q.0;
    q.1.push((token,String::from(text)));
    if crate::evq_push(crate::Event {typ:crate::E_IME_COMMIT,win,data:token,..Default::default()}) { true }
    else { q.1.pop(); false }
}
pub fn ime_commit_take(token:u32) -> Option<String> {
    let mut q=COMMITS.lock(); let i=q.1.iter().position(|v|v.0==token)?; Some(q.1.remove(i).1)
}
pub fn ime_preedit(win: u8) -> Preedit { PREEDIT.lock().iter().find(|v| v.0==win).map_or_else(Default::default,|v| v.1.clone()) }
/// Publish valid UTF-8 and a byte caret; invalid/oversized updates clear composition.
pub fn ime_preedit_set(win: u8, text: &str, cursor: usize) {
    if win as usize >= crate::PW_MAX { return; }
    let value=if text.len()<=4096 && text.is_char_boundary(cursor.min(text.len())) { Preedit { text:String::from(text),cursor:cursor.min(text.len()) } } else { Preedit::default() };
    let mut slots=PREEDIT.lock();
    if let Some(slot)=slots.iter_mut().find(|v| v.0==win) { slot.1=value; } else { slots.push((win,value)); }
    drop(slots);
    crate::evq_push(crate::Event { typ:crate::E_PREEDIT,win,..Default::default() });
}

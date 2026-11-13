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
/// Committed text of any length: commits of at most 4096 bytes, split between characters ->
/// the bytes queued (fewer: the commit queue or the event ring is full, the rest can follow
/// once the app took these).
pub fn ime_commit_text(win:u8,text:&str) -> usize {
    let mut at=0;
    while at<text.len() {
        let mut end=(at+4096).min(text.len());
        while !text.is_char_boundary(end) { end-=1; }
        if !ime_commit_set(win,&text[at..end]) { break; }
        at=end;
    }
    at
}
// ---- (E4) what the text being typed is (ime_purpose): the input method and an on-screen
// keyboard choose their mode by it
pub const IP_TEXT: u32 = 0;
pub const IP_URL: u32 = 1;
pub const IP_EMAIL: u32 = 2;
pub const IP_NUMBER: u32 = 3;
pub const IP_PHONE: u32 = 4;
/// no input method (Windows: the window's input context is taken away; X11: no IBus; Wayland:
/// the password purpose); ui's TB_PASSWORD boxes set it themselves
pub const IP_PASSWORD: u32 = 5;
pub const IP_SEARCH: u32 = 6;

static INLINE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static PURPOSE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(IP_TEXT);

/// (E4) the app draws the composition (ime_preedit) at its caret itself: Windows hides the
/// input method's own composition window (its candidates stay; WM_IME_SETCONTEXT without
/// ISC_SHOWUICOMPOSITIONWINDOW, sent again to the window with the keyboard on a change).  IBus
/// and Wayland's text input have no window of their own for it: nothing changes there
pub fn ime_inline(on: bool) {
    if INLINE.swap(on, core::sync::atomic::Ordering::Relaxed) != on {
        #[cfg(windows)]
        crate::ime_context_refresh();
    }
}

/// (E4) ime_inline's state
pub fn ime_inline_on() -> bool {
    INLINE.load(core::sync::atomic::Ordering::Relaxed)
}

/// (E4) the purpose of the text being typed (IP_*): Windows takes the input method away for
/// IP_PASSWORD and gives it back for the others (also when a window gets the keyboard); X11 keeps
/// the keys of an IP_PASSWORD field from IBus; Wayland's text input gets it as its content
/// purpose (again when it changes while enabled); elsewhere it is only kept (ime_purpose_of).
/// ui's TB_PASSWORD boxes set IP_PASSWORD themselves while they have the keyboard
pub fn ime_purpose(p: u32) {
    if PURPOSE.swap(p, core::sync::atomic::Ordering::Relaxed) != p {
        #[cfg(windows)]
        crate::ime_context_refresh();
    }
}

/// (E4) ime_purpose's value
pub fn ime_purpose_of() -> u32 {
    PURPOSE.load(core::sync::atomic::Ordering::Relaxed)
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

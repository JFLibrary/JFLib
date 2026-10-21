// Android editor snapshots and edits. UTF-8 byte offsets cross JNI; Java owns UTF-16.
// This adapter shares the host's single UI thread and never invokes Android under a lock.
use alloc::vec::Vec;
use rt::sync::SpinLock;

struct Editor {
    id: u32, flags: u32, anchor: usize, caret: usize, text: Vec<u8>,
}
static EDITOR: SpinLock<Editor> = SpinLock::new(Editor {
    id: 0, flags: 0, anchor: 0, caret: 0, text: Vec::new(),
});
static EDIT: SpinLock<Option<(u32, Vec<u8>, usize, usize)>> = SpinLock::new(None);
static SNAPSHOT: SpinLock<Vec<u8>> = SpinLock::new(Vec::new());

/// Publish the focused editor's actual text, flags and UTF-8 selection to Android's IME.
pub fn android_ime_publish(id: u32, text: &[u8], anchor: usize, caret: usize, flags: u32) {
    let mut state = EDITOR.lock();
    state.id = id;
    state.flags = flags;
    state.anchor = anchor;
    state.caret = caret;
    if state.text != text { state.text.clear(); state.text.extend_from_slice(text); }
}
/// Close the Android input session when library focus leaves an editor.
pub fn android_ime_clear() {
    android_ime_publish(0, &[], 0, 0, 0);
    *EDIT.lock() = None;
}
/// Consume one host edit for the focused widget, including the requested UTF-8 selection.
pub fn android_ime_take(id: u32) -> Option<(Vec<u8>, usize, usize)> {
    let edit = EDIT.lock().take()?;
    (edit.0 == id).then_some((edit.1, edit.2, edit.3))
}
/// Build a JNI snapshot: id, flags, anchor, caret (little-endian u32), then UTF-8 text.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_editor_snapshot() -> u32 {
    let editor = EDITOR.lock();
    let mut out = SNAPSHOT.lock();
    out.clear();
    for v in [editor.id, editor.flags, editor.anchor as u32, editor.caret as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&editor.text);
    out.len() as u32
}
/// Snapshot address, valid until the next snapshot call on this same host thread.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_editor_data() -> usize { SNAPSHOT.lock().as_ptr() as usize }
/// Queue a bounded, valid editor replacement from the shared JNI input buffer.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_editor_replace(id: u32, len: u32, anchor: u32, caret: u32) -> u32 {
    if id == 0 || EDITOR.lock().id != id || len > 262144 { return 0; }
    let Some(bytes) = super::input(len) else { return 0; };
    let Ok(text) = core::str::from_utf8(&bytes) else { return 0; };
    if !text.is_char_boundary(anchor as usize) || !text.is_char_boundary(caret as usize) { return 0; }
    *EDIT.lock() = Some((id, bytes, anchor as usize, caret as usize));
    super::jfgui_event(crate::E_WAKE as u32, 0, 0, 0, 0, 0, 0)
}

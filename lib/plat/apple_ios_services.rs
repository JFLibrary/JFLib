// apple_ios_services -- iOS's system services of AP5b (docs/APPLE.md): the clipboard (UIPasteboard),
// the system's file picker (UIDocumentPickerViewController), notifications (UNUserNotificationCenter).
//
//   clip_set / clip_set_many / clip_get / clip_has   as on macOS (apple_mac.rs): one pasteboard item
//                                with a representation per format (text as a string); CLIP_IMAGE gets a
//                                PNG, else any image of the pasteboard (JPEG, HEIC, TIFF) made a PNG
//                                through UIImage
//   file_dialog(flags, path, exts) -> (FD_PENDING, 0)   the picker over the scene's window, as
//                                Android's: FD_OPEN opens a document in place (its types from the
//                                extensions), FD_FOLDER a folder, FD_SAVE exports an empty file of the
//                                suggested name to where the user puts it (the app writes it then);
//                                one at a time ((0, 0) while one is open), (-1, 0) without a window
//   file_dialog_poll()           None while it is open, then once (1, 0, path) / (0, 0, "") -- the
//                                path a security-scoped URL's, its access started for the session
//   notes::*                     (tray.rs) notifications: no symbol of a notification area on iOS;
//                                notify asks the user once (alert, sound) and hands the note to the
//                                centre when allowed -- refused, it goes away at once (NoteClosed); one
//                                arriving while the app is in front shows all the same; a tap opens the
//                                app: NoteClicked.  Only in an app (a bundle identifier).
//
// Reading what another app copied makes iOS ask the user once ("Allow Paste"); asking which types the
// pasteboard has does not.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{CStr, c_void};
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use rt::sync::SpinLock;

use super::super::objc::{Id, NIL, Sel, class, msg, nsstring, objc_autoreleasePoolPop, objc_autoreleasePoolPush, release, string_of,
                         subclass};
use crate::{CLIP_DOCX, CLIP_HTML, CLIP_IMAGE, CLIP_PNG, CLIP_RTF, CLIP_TEXT, FD_FOLDER, FD_PENDING, FD_SAVE};

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    fn UIImagePNGRepresentation(image: Id) -> Id;
}
// UTType's classes
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}
// UNUserNotificationCenter
#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

const UTI_TEXT: &CStr = c"public.utf8-plain-text";
const UTI_PNG: &CStr = c"public.png";

/// the pasteboard type of a CLIP_* format
fn clip_type(fmt: u32) -> Option<&'static CStr> {
    Some(match fmt {
        CLIP_TEXT => UTI_TEXT,
        CLIP_PNG | CLIP_IMAGE => UTI_PNG,
        CLIP_HTML => c"public.html",
        CLIP_RTF => c"public.rtf",
        CLIP_DOCX => c"org.jflib.docx-fragment",
        _ => return None,
    })
}

fn pasteboard() -> Id {
    // SAFETY: the general pasteboard (a shared object).
    unsafe { msg!(class(c"UIPasteboard"), c"generalPasteboard"; Id) }
}

/// an autorelease pool around f (the pasteboard's objects are autoreleased)
fn pooled<R>(f: impl FnOnce() -> R) -> R {
    // SAFETY: a pool of ours, popped on this thread after f.
    let pool = unsafe { objc_autoreleasePoolPush() };
    let r = f();
    // SAFETY: the pool pushed above.
    unsafe { objc_autoreleasePoolPop(pool) };
    r
}

fn bytes_of(data: Id) -> Option<Vec<u8>> {
    if data.is_null() {
        return None;
    }
    // SAFETY: bytes / length of a live NSData; copied at once.
    unsafe {
        let p: *const u8 = msg!(data, c"bytes"; *const u8);
        let n: usize = msg!(data, c"length"; usize);
        Some(if p.is_null() || n == 0 { Vec::new() } else { core::slice::from_raw_parts(p, n).to_vec() })
    }
}

/// image file bytes (any format ImageIO reads) -> PNG
fn png_of(bytes: &[u8]) -> Option<Vec<u8>> {
    pooled(|| {
        // SAFETY: NSData / UIImage made here, autoreleased in the pool.
        unsafe {
            let d: Id = msg!(class(c"NSData"), c"dataWithBytes:length:", bytes.as_ptr() => *const u8, bytes.len() => usize; Id);
            let img: Id = msg!(class(c"UIImage"), c"imageWithData:", d => Id; Id);
            if img.is_null() { None } else { bytes_of(UIImagePNGRepresentation(img)) }
        }
    })
}

/// CLIP_TEXT / CLIP_HTML / CLIP_RTF / CLIP_DOCX as clip_set_many; CLIP_PNG: the PNG file;
/// CLIP_IMAGE: a CLIPIMG record (clipimg_pack): its PNG, else a PNG made of its BMP
pub fn clip_set(fmt: u32, data: &[u8]) -> bool {
    match fmt {
        CLIP_PNG => !data.is_empty() && clip_put(&[(UTI_PNG, data)]),
        CLIP_IMAGE => {
            let Some((png, bmp)) = crate::clipimg_split(data) else { return false };
            let made = if png.is_empty() && !bmp.is_empty() { png_of(bmp) } else { None };
            let png = made.as_deref().unwrap_or(png);
            !png.is_empty() && clip_put(&[(UTI_PNG, png)])
        }
        _ => clip_set_many(&[(fmt, data)]),
    }
}

/// several representations of one selection (text formats, cliprich.rs)
pub fn clip_set_many(entries: &[(u32, &[u8])]) -> bool {
    if !crate::clip_many_valid(entries) || entries.iter().any(|(f, _)| clip_type(*f).is_none()) {
        return false;
    }
    let v: Vec<(&CStr, &[u8])> = entries.iter().map(|(f, d)| (clip_type(*f).unwrap_or(c""), *d)).collect();
    clip_put(&v)
}

/// the pasteboard's items replaced by one item with each (type, bytes) (text as a string)
fn clip_put(entries: &[(&CStr, &[u8])]) -> bool {
    let pb = pasteboard();
    if pb.is_null() {
        return false;
    }
    pooled(|| {
        // SAFETY: NSMutableDictionary / NSString / NSData made here (autoreleased or released).
        unsafe {
            let item: Id = msg!(class(c"NSMutableDictionary"), c"dictionary"; Id);
            for (t, d) in entries {
                let key = nsstring(t.to_str().unwrap_or(""));
                let value: Id = if *t == UTI_TEXT {
                    let s = nsstring(core::str::from_utf8(d).unwrap_or(""));
                    msg!(s, c"autorelease"; Id)
                } else {
                    msg!(class(c"NSData"), c"dataWithBytes:length:", d.as_ptr() => *const u8, d.len() => usize; Id)
                };
                msg!(item, c"setObject:forKey:", value => Id, key => Id; ());
                release(key);
            }
            let items: Id = msg!(class(c"NSArray"), c"arrayWithObject:", item => Id; Id);
            msg!(pb, c"setItems:", items => Id; ());
        }
        true
    })
}

/// the bytes of one pasteboard type (text: its UTF-8)
fn clip_data(t: &CStr) -> Option<Vec<u8>> {
    let pb = pasteboard();
    if pb.is_null() {
        return None;
    }
    pooled(|| {
        let ty = nsstring(t.to_str().unwrap_or(""));
        // SAFETY: the pasteboard's getters; the bytes are copied before the pool ends.
        let out = unsafe {
            if t == UTI_TEXT {
                let s: Id = msg!(pb, c"string"; Id);
                (!s.is_null()).then(|| string_of(s).into_bytes())
            } else {
                bytes_of(msg!(pb, c"dataForPasteboardType:", ty => Id; Id))
            }
        };
        release(ty);
        out
    })
}

/// CLIP_IMAGE: the PNG, else any image of the pasteboard as a PNG
pub fn clip_get(fmt: u32) -> Option<Vec<u8>> {
    if fmt == CLIP_IMAGE {
        return clip_data(UTI_PNG).filter(|d| !d.is_empty()).or_else(|| {
            let pb = pasteboard();
            pooled(|| {
                // SAFETY: the pasteboard's first image, made a PNG; autoreleased in the pool.
                unsafe {
                    let img: Id = if pb.is_null() { NIL } else { msg!(pb, c"image"; Id) };
                    if img.is_null() { None } else { bytes_of(UIImagePNGRepresentation(img)) }
                }
            })
        });
    }
    let data = clip_data(clip_type(fmt)?)?;
    if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) && data.len() > crate::CLIP_RICH_MAX {
        return None;
    }
    if fmt == CLIP_HTML && (data.contains(&0) || core::str::from_utf8(&data).is_err()) {
        return None;
    }
    Some(data)
}

/// the pasteboard offers the format (nothing is read: no question to the user)
pub fn clip_has(fmt: u32) -> bool {
    let pb = pasteboard();
    if pb.is_null() {
        return false;
    }
    // SAFETY: hasStrings / hasImages / containsPasteboardTypes: with an array of ours.
    pooled(|| unsafe {
        match fmt {
            CLIP_TEXT => msg!(pb, c"hasStrings"; bool),
            CLIP_IMAGE => msg!(pb, c"hasImages"; bool),
            _ => {
                let Some(t) = clip_type(fmt) else { return false };
                let ty = nsstring(t.to_str().unwrap_or(""));
                let arr: Id = msg!(class(c"NSArray"), c"arrayWithObject:", ty => Id; Id);
                let has = msg!(pb, c"containsPasteboardTypes:", arr => Id; bool);
                release(ty);
                has
            }
        }
    })
}

// ---------------------------------------------------------------- the file picker

/// a picker is showing
static PICKING: AtomicBool = AtomicBool::new(false);
/// its answer: Some(Some(path)) chosen, Some(None) cancelled
static PICKED: SpinLock<Option<Option<String>>> = SpinLock::new(None);
/// the pickers' delegate (one for the process)
static DELEGATE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

fn delegate() -> Id {
    let d = DELEGATE.load(Ordering::Acquire);
    if !d.is_null() {
        return d;
    }
    type Imp = *const c_void;
    let (cls, _) = subclass(c"NSObject", c"JFPickerDelegate", &[
        (c"documentPicker:didPickDocumentsAtURLs:", picked as Imp, c"v@:@@"),
        (c"documentPickerWasCancelled:", cancelled as Imp, c"v@:@"),
    ], &[c"UIDocumentPickerDelegate"]);
    // SAFETY: an instance of our class, kept for the process.
    let d: Id = unsafe { msg!(msg!(cls, c"alloc"; Id), c"init"; Id) };
    DELEGATE.store(d, Ordering::Release);
    d
}

extern "C" fn picked(_this: Id, _sel: Sel, _picker: Id, urls: Id) {
    // SAFETY: the first URL of the array UIKit hands over; its security scope opened for the session.
    let path = unsafe {
        let n: usize = if urls.is_null() { 0 } else { msg!(urls, c"count"; usize) };
        let url: Id = if n == 0 { NIL } else { msg!(urls, c"objectAtIndex:", 0usize => usize; Id) };
        if url.is_null() {
            None
        } else {
            msg!(url, c"startAccessingSecurityScopedResource"; bool);
            Some(string_of(msg!(url, c"path"; Id))).filter(|p| !p.is_empty())
        }
    };
    answer(path);
}

extern "C" fn cancelled(_this: Id, _sel: Sel, _picker: Id) {
    answer(None);
}

fn answer(path: Option<String>) {
    *PICKED.lock() = Some(path);
    PICKING.store(false, Ordering::Release);
    super::wake();
}

/// the extensions of the entries ("png;jpg=Bilder"), the selected one's first, each once
fn ext_list<'a>(flags: u32, extensions: &[&'a str]) -> Vec<&'a str> {
    let exts: Vec<&str> = extensions.iter().take_while(|e| !e.is_empty()).copied().collect();
    let sel = ((flags >> crate::FD_EXT_SHIFT) & 0xFF) as usize;
    let order = exts.get(sel).into_iter().chain(exts.iter().enumerate().filter(|(k, _)| *k != sel).map(|(_, e)| e));
    let mut list: Vec<&str> = Vec::new();
    for e in order {
        for x in e.split('=').next().unwrap_or("").split(';').map(str::trim).filter(|x| !x.is_empty() && *x != "*") {
            if !list.contains(&x) {
                list.push(x);
            }
        }
    }
    list
}

/// the UTType of an identifier ("public.folder") into the array
///
/// SAFETY: types is a live NSMutableArray; an autorelease pool is in place.
unsafe fn add_type(types: Id, how: &CStr, what: &str) {
    let s = nsstring(what);
    // SAFETY: UTType's class methods with a string of ours (released after).
    unsafe {
        let t: Id = msg!(class(c"UTType"), how, s => Id; Id);
        if !t.is_null() {
            msg!(types, c"addObject:", t => Id; ());
        }
    }
    release(s);
}

/// the system's picker (see the module header) -> (FD_PENDING, 0), (0, 0) while one is open,
/// (-1, 0) without a window
pub fn file_dialog(flags: u32, path: &mut String, extensions: &[&str]) -> (i32, u32) {
    let win = super::host().window.get();
    if win.is_null() || !super::on_main() {
        return (-1, 0);
    }
    if PICKING.load(Ordering::Acquire) {
        return (0, 0);
    }
    let (save, folder) = (flags & FD_SAVE != 0, flags & FD_FOLDER != 0);
    let start = path.replace('\\', "/");
    let last = start.rsplit('/').next().unwrap_or("").trim();
    // an export needs the file: an empty one of the suggested name, moved where the user puts it
    let export = if save {
        let name = if last.is_empty() { "Unbenannt" } else { last };
        let Some(dir) = crate::temp_dir().map(|t| alloc::format!("{}/jfgui-export", t.trim_end_matches('/'))) else { return (-1, 0) };
        let _ = crate::fs::mkdir_private(&dir);
        let file = alloc::format!("{dir}/{name}");
        if !crate::fs::write_file_private(&file, &[]) {
            return (-1, 0);
        }
        Some(file)
    } else {
        None
    };
    // where it starts: the folder of the path given (a path of the file system)
    let dir = if folder || start.ends_with('/') { start.trim_end_matches('/') } else { start.rsplit_once('/').map_or("", |(d, _)| d) };
    let ok = pooled(|| {
        // SAFETY: UIKit / UTType objects made here (autoreleased in the pool; the picker released after
        // presenting, the presenting controller keeps it); the main thread.
        unsafe {
            let picker: Id = msg!(class(c"UIDocumentPickerViewController"), c"alloc"; Id);
            let picker: Id = if let Some(file) = &export {
                let p = nsstring(file);
                let url: Id = msg!(class(c"NSURL"), c"fileURLWithPath:", p => Id; Id);
                release(p);
                let urls: Id = msg!(class(c"NSArray"), c"arrayWithObject:", url => Id; Id);
                msg!(picker, c"initForExportingURLs:asCopy:", urls => Id, false => bool; Id)
            } else {
                let types: Id = msg!(class(c"NSMutableArray"), c"array"; Id);
                if folder {
                    add_type(types, c"typeWithIdentifier:", "public.folder");
                } else {
                    for x in ext_list(flags, extensions) {
                        add_type(types, c"typeWithFilenameExtension:", x);
                    }
                }
                if msg!(types, c"count"; usize) == 0 {
                    add_type(types, c"typeWithIdentifier:", "public.item");
                }
                msg!(picker, c"initForOpeningContentTypes:asCopy:", types => Id, false => bool; Id)
            };
            if picker.is_null() {
                return false;
            }
            msg!(picker, c"setDelegate:", delegate() => Id; ());
            msg!(picker, c"setAllowsMultipleSelection:", false => bool; ());
            if dir.starts_with('/') {
                let d = nsstring(dir);
                let url: Id = msg!(class(c"NSURL"), c"fileURLWithPath:", d => Id; Id);
                release(d);
                msg!(picker, c"setDirectoryURL:", url => Id; ());
            }
            // the topmost controller presents it
            let mut vc: Id = msg!(win, c"rootViewController"; Id);
            while !vc.is_null() {
                let next: Id = msg!(vc, c"presentedViewController"; Id);
                if next.is_null() {
                    break;
                }
                vc = next;
            }
            if vc.is_null() {
                release(picker);
                return false;
            }
            *PICKED.lock() = None;
            PICKING.store(true, Ordering::Release);
            msg!(vc, c"presentViewController:animated:completion:", picker => Id, true => bool, NIL => Id; ());
            release(picker);
            true
        }
    });
    if ok { (FD_PENDING, 0) } else { (-1, 0) }
}

/// the picker's answer (see the module header)
pub fn file_dialog_poll() -> Option<(i32, u32, String)> {
    let r = PICKED.lock().take()?;
    Some(match r {
        Some(path) => (1, 0, path),
        None => (0, 0, String::new()),
    })
}

// ---------------------------------------------------------------- notifications

pub(crate) mod notes {
    use super::*;
    use crate::tray::{Note, Tray, TrayEvent};

    /// requests waiting for the user's answer: (id, the UNNotificationRequest, +1)
    static WAITING: SpinLock<Vec<(u32, usize)>> = SpinLock::new(Vec::new());
    static DELEGATE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
    static ASKED: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

    pub fn tray_set(_t: &Tray) -> bool {
        false
    }
    pub fn tray_remove() {}
    pub fn supported() -> bool {
        false
    }

    /// the centre (nil outside an app: iOS has none for a program without a bundle identifier)
    fn center() -> Id {
        // SAFETY: the main bundle's identifier, then the shared centre.
        unsafe {
            let b: Id = msg!(class(c"NSBundle"), c"mainBundle"; Id);
            let id: Id = if b.is_null() { NIL } else { msg!(b, c"bundleIdentifier"; Id) };
            if id.is_null() { NIL } else { msg!(class(c"UNUserNotificationCenter"), c"currentNotificationCenter"; Id) }
        }
    }

    fn key(id: u32) -> String {
        alloc::format!("jflib-note-{id}")
    }

    fn delegate() -> Id {
        let d = DELEGATE.load(Ordering::Acquire);
        if !d.is_null() {
            return d;
        }
        type Imp = *const c_void;
        let (cls, _) = subclass(c"NSObject", c"JFNoteDelegate", &[
            (c"userNotificationCenter:willPresentNotification:withCompletionHandler:", present as Imp, c"v@:@@@?"),
            (c"userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:", response as Imp, c"v@:@@@?"),
        ], &[c"UNUserNotificationCenterDelegate"]);
        // SAFETY: an instance of our class, kept for the process (the centre holds it weakly).
        let d: Id = unsafe { msg!(msg!(cls, c"alloc"; Id), c"init"; Id) };
        DELEGATE.store(d, Ordering::Release);
        d
    }

    pub fn notify(id: u32, n: &Note) -> bool {
        let c = center();
        if c.is_null() {
            return false;
        }
        pooled(|| {
            // SAFETY: the centre's calls with objects made here (the request kept +1 until it is handed
            // over); the answer's block is global.
            unsafe {
                msg!(c, c"setDelegate:", delegate() => Id; ());
                let content: Id = msg!(class(c"UNMutableNotificationContent"), c"new"; Id);
                let (t, b) = (nsstring(&n.title), nsstring(&n.body));
                msg!(content, c"setTitle:", t => Id; ());
                msg!(content, c"setBody:", b => Id; ());
                release(t);
                release(b);
                let sound: Id = msg!(class(c"UNNotificationSound"), c"defaultSound"; Id);
                msg!(content, c"setSound:", sound => Id; ());
                let k = nsstring(&key(id));
                let req: Id = msg!(class(c"UNNotificationRequest"), c"requestWithIdentifier:content:trigger:", k => Id, content => Id, NIL => Id; Id);
                release(k);
                release(content);
                if req.is_null() {
                    return false;
                }
                msg!(req, c"retain"; Id);
                WAITING.lock().push((id, req as usize));
                // alert | sound: iOS asks the user the first time only, later it answers at once
                let asked = crate::apple::block::global(&ASKED, answered as *const c_void);
                msg!(c, c"requestAuthorizationWithOptions:completionHandler:", 6usize => usize, asked => *const c_void; ());
                true
            }
        })
    }

    /// the user's answer (any thread): on to the main queue
    extern "C" fn answered(_block: *const c_void, granted: bool, _error: Id) {
        // SAFETY: a plain function on the main queue; its context says granted.
        unsafe { super::super::dispatch_async_f(super::super::main_queue(), granted as usize as *mut c_void, hand_over) };
    }

    /// the waiting requests to the centre, or (refused) NoteClosed for each
    extern "C" fn hand_over(granted: *mut c_void) {
        let waiting = core::mem::take(&mut *WAITING.lock());
        let c = center();
        for (id, req) in waiting {
            if !granted.is_null() && !c.is_null() {
                // SAFETY: the centre takes the request (no completion handler).
                unsafe { msg!(c, c"addNotificationRequest:withCompletionHandler:", req as Id => Id, core::ptr::null::<c_void>() => *const c_void; ()) };
            } else {
                crate::tray::push(TrayEvent::NoteClosed(id));
            }
            release(req as Id);
        }
        super::super::wake();
    }

    pub fn notify_close(id: u32) {
        WAITING.lock().retain(|(i, req)| {
            if *i == id {
                release(*req as Id);
            }
            *i != id
        });
        let c = center();
        if c.is_null() {
            return;
        }
        pooled(|| {
            // SAFETY: the centre's removals with an array of one identifier.
            unsafe {
                let k = nsstring(&key(id));
                let arr: Id = msg!(class(c"NSArray"), c"arrayWithObject:", k => Id; Id);
                msg!(c, c"removeDeliveredNotificationsWithIdentifiers:", arr => Id; ());
                msg!(c, c"removePendingNotificationRequestsWithIdentifiers:", arr => Id; ());
                release(k);
            }
        });
    }

    /// one arriving while the app is in front: banner, list, sound all the same
    extern "C" fn present(_this: Id, _sel: Sel, _center: Id, _note: Id, handler: *const c_void) {
        // SAFETY: the centre's completion handler, called once (UNNotificationPresentationOptions).
        unsafe { crate::apple::block::call(handler, Some(1 << 4 | 1 << 3 | 1 << 1)) };
    }

    /// a tap on one of ours (the app comes to the front by itself): NoteClicked
    extern "C" fn response(_this: Id, _sel: Sel, _center: Id, resp: Id, handler: *const c_void) {
        // SAFETY: the response's notification, its request's identifier, the action's identifier.
        let (ident, action) = unsafe {
            let n: Id = msg!(resp, c"notification"; Id);
            let r: Id = if n.is_null() { NIL } else { msg!(n, c"request"; Id) };
            let i = if r.is_null() { String::new() } else { string_of(msg!(r, c"identifier"; Id)) };
            (i, string_of(msg!(resp, c"actionIdentifier"; Id)))
        };
        if let Some(id) = ident.strip_prefix("jflib-note-").and_then(|s| s.parse::<u32>().ok()) {
            let closed = action.ends_with("DismissActionIdentifier");
            crate::tray::push(if closed { TrayEvent::NoteClosed(id) } else { TrayEvent::NoteClicked(id) });
            super::super::wake();
        }
        // SAFETY: the centre's completion handler, called once.
        unsafe { crate::apple::block::call(handler, None) };
    }
}

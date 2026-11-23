// web_share -- (share.rs) the share sheet of the browser and of Android, through the host
//
//   share(&Share) -> SHARE_PENDING / SHARE_NONE   REQUEST_SHARE with the items; the host answers
//                                through jfgui_complete (2 done, -4 cancelled, anything else failed:
//                                web.rs hands that to share::answer)
//   available() -> bool          Android: always; the browser: the host granted CAP_SHARE
//                                (jfgui_capabilities: navigator.share exists)
//
// Payload: the title, the text, the link (each u32 length + UTF-8), u32 count, then per file its name
// (u32 length + UTF-8) and its bytes (u32 length + bytes) -- the files are read here (the browser's
// virtual files, the app's own on Android) and become File objects (navigator.share) / content://
// URIs of the host's provider GalleryFiles (Intent.ACTION_SEND(_MULTIPLE) in a chooser, readable by
// the target only).  At most SHARE_BYTES of files in all.
//
// The browser asks for the user's gesture: navigator.share works only shortly after an input event,
// so an app calls share while handling a click or a key.

use alloc::vec::Vec;

use crate::share::{SHARE_NONE, SHARE_PENDING, Share};
use crate::web::{REQUEST_SHARE, RequestStatus, forget_request, host_caps, request, request_status};

/// the files of one share together, at most
const SHARE_BYTES: usize = 64 << 20;

pub fn available() -> bool {
    cfg!(target_os = "android") || host_caps() & crate::CAP_SHARE != 0
}

fn put(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

pub fn share(s: &Share) -> i32 {
    if !available() {
        return SHARE_NONE;
    }
    let mut bytes = Vec::new();
    for t in [&s.title, &s.text, &s.url] {
        put(&mut bytes, t.as_bytes());
    }
    bytes.extend_from_slice(&(s.files.len() as u32).to_le_bytes());
    let mut total = 0usize;
    for f in &s.files {
        let Some(data) = crate::read_file(f) else { return SHARE_NONE };
        total += data.len();
        if total > SHARE_BYTES {
            return SHARE_NONE;
        }
        put(&mut bytes, f.rsplit('/').next().unwrap_or("").as_bytes());
        put(&mut bytes, &data);
    }
    let Some(id) = request(REQUEST_SHARE, &bytes, None) else { return SHARE_NONE };
    if request_status(id) == Some(RequestStatus::Unsupported) {
        forget_request(id);
        return SHARE_NONE;
    }
    SHARE_PENDING
}

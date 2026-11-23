// web_share -- (share.rs) the browser's / Android's share sheet through the host (stand-in until the
// host part is done)

use crate::share::{SHARE_NONE, Share};

pub fn share(_s: &Share) -> i32 {
    SHARE_NONE
}

pub fn available() -> bool {
    false
}

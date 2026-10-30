// background -- the picture under the logical canvas (G1): a floor plan, a map, the photo of a
// rack row.  View data: the simulator never sees it.
//
//   Background                    the record: the image file as it was (its format, name and
//                                 bytes), its size in pixels, its top left corner in world
//                                 units, world units per 1000 pixels, opacity, locked
//   Background::new(name, data, w, h) -> Result   a picture within the limits (MAX_BYTES,
//                                 MAX_SIDE) in one of the formats sniff knows: at (0, 0), one
//                                 world unit per pixel, OPACITY_NEW, not locked
//   sniff(bytes) -> Option<&str>  "png", "jpeg", "bmp" or "gif" from the first bytes
//   rect() / scale() / opacity_pct()   the world box x, y, w, h; world units per pixel; percent
//   BgErr::msg                    why a picture was refused
//
// In the .jfnet file (null without one):
//
//   "background": {"format": "png", "name": "plan.png", "w": 1200, "h": 800, "x": -600,
//                  "y": -400, "scale_pm": 1000, "opacity": 60, "locked": true,
//                  "base64": "iVBORw0KGgo..."}
//
// The file keeps the picture as it was (its own format, as base64, RFC 4648), so a .jfnet stays
// one file that autosave, recovery and undo carry whole (undo steps share the bytes).  Limits:
// at most MAX_BYTES (8 MiB, about 11 MB of base64), MAX_SIDE (8192) pixels per side and
// MAX_PIXELS (32 megapixels); larger pictures are refused with their size, nothing is scaled
// down behind the user's back.  The app
// decodes the picture (PNG, JPEG, BMP and GIF: the codecs every window has) and draws a copy of
// at most 4096 pixels per side.  scale_pm 0 reads as 1000, opacity 0 as 100 %.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use agent_core::jmap::{Field, Json, Value};

use crate::msg;
use crate::msg::Msg;

/// the largest picture file a project takes (bytes)
pub const MAX_BYTES: usize = 8 << 20;
/// the most pixels per side, in all (32 megapixels)
pub const MAX_SIDE: u32 = 8192;
pub const MAX_PIXELS: u64 = 32 << 20;
/// the opacity of a new picture (percent)
pub const OPACITY_NEW: u32 = 60;
/// scale_pm and opacity at their limits
pub const SCALE_MIN: u32 = 10;
pub const SCALE_MAX: u32 = 100_000;
pub const OPACITY_MIN: u32 = 5;

/// the picture under the canvas
#[derive(Clone, PartialEq, Eq)]
pub struct Background {
    /// "png", "jpeg", "bmp", "gif"
    pub format: String,
    /// the name of the file it came from
    pub name: String,
    /// the file's bytes
    pub data: Arc<[u8]>,
    /// its size in pixels
    pub w: u32,
    pub h: u32,
    /// its top left corner in world units
    pub x: i32,
    pub y: i32,
    /// world units per 1000 pixels (0: 1000)
    pub scale_pm: u32,
    /// percent (0: 100)
    pub opacity: u32,
    /// it cannot be selected or dragged
    pub locked: bool,
}

impl Default for Background {
    fn default() -> Background {
        Background { format: String::new(), name: String::new(), data: Arc::from(Vec::new()), w: 0, h: 0, x: 0, y: 0, scale_pm: 0, opacity: 0, locked: false }
    }
}

impl fmt::Debug for Background {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Background({} {}, {} bytes, {}x{} at {},{} scale {} opacity {}{})", self.format, self.name, self.data.len(), self.w, self.h, self.x, self.y, self.scale_pm, self.opacity, if self.locked { " locked" } else { "" })
    }
}

/// why a picture was refused
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BgErr {
    /// none of the formats sniff knows
    Format,
    /// the file is larger than MAX_BYTES (its size)
    TooLarge(usize),
    /// more than MAX_SIDE pixels on a side (its size)
    TooWide(u32, u32),
    /// the picture could not be read (the app's decoder said no)
    Broken,
}

impl BgErr {
    /// the reason as a message
    pub fn msg(&self) -> Msg {
        match self {
            BgErr::Format => msg!("Das ist kein Bild in PNG, JPEG, BMP oder GIF."),
            BgErr::TooLarge(n) => msg!("Das Bild ist zu groß: {} MB; ins Projekt passen höchstens 8 MB.", n.div_ceil(1 << 20)),
            BgErr::TooWide(w, h) => msg!("Das Bild ist zu groß: {} × {} Pixel; höchstens {} Pixel je Seite und 32 Megapixel.", w, h, MAX_SIDE),
            BgErr::Broken => msg!("Das Bild lässt sich nicht lesen."),
        }
    }
}

/// the format of an image file from its first bytes
pub fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpeg")
    } else if b.starts_with(b"BM") && b.len() > 26 {
        Some("bmp")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("gif")
    } else {
        None
    }
}

impl Background {
    /// a picture of w x h pixels from a file named `name`: at (0, 0), one unit per pixel
    pub fn new(name: &str, data: Vec<u8>, w: u32, h: u32) -> Result<Background, BgErr> {
        if data.len() > MAX_BYTES {
            return Err(BgErr::TooLarge(data.len()));
        }
        let format = sniff(&data).ok_or(BgErr::Format)?;
        if w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
            return Err(BgErr::TooWide(w, h));
        }
        if w == 0 || h == 0 {
            return Err(BgErr::Broken);
        }
        Ok(Background { format: String::from(format), name: String::from(name), data: Arc::from(data), w, h, scale_pm: 1000, opacity: OPACITY_NEW, ..Background::default() })
    }
    /// world units per pixel
    pub fn scale(&self) -> f32 {
        (if self.scale_pm == 0 { 1000 } else { self.scale_pm.clamp(SCALE_MIN, SCALE_MAX) }) as f32 / 1000.0
    }
    /// the opacity in percent
    pub fn opacity_pct(&self) -> u32 {
        if self.opacity == 0 { 100 } else { self.opacity.clamp(OPACITY_MIN, 100) }
    }
    /// the world box x, y, w, h
    pub fn rect(&self) -> [f32; 4] {
        let s = self.scale();
        [self.x as f32, self.y as f32, self.w as f32 * s, self.h as f32 * s]
    }
    /// the settings that move with it (position, scale, opacity, locked), without the picture
    pub fn place(&self) -> [i64; 5] {
        [self.x as i64, self.y as i64, self.scale_pm as i64, self.opacity as i64, self.locked as i64]
    }
    /// the settings of place() back
    pub fn set_place(&mut self, v: [i64; 5]) {
        self.x = v[0] as i32;
        self.y = v[1] as i32;
        self.scale_pm = v[2] as u32;
        self.opacity = v[3] as u32;
        self.locked = v[4] != 0;
    }
}

impl Json for Background {
    fn to_json(&self) -> Value {
        let mut o = agent_core::jmap::empty_object();
        o.insert("format", Value::string(&self.format));
        o.insert("name", Value::string(&self.name));
        o.insert("w", Value::u64(self.w as u64));
        o.insert("h", Value::u64(self.h as u64));
        o.insert("x", Value::i64(self.x as i64));
        o.insert("y", Value::i64(self.y as i64));
        o.insert("scale_pm", Value::u64(self.scale_pm as u64));
        o.insert("opacity", Value::u64(self.opacity as u64));
        o.insert("locked", Value::Bool(self.locked));
        o.insert("base64", Value::string(&crypto::b64::encode(&self.data)));
        o
    }
    fn from_json(v: &Value) -> Option<Background> {
        v.as_object()?;
        let data = v.get("base64").and_then(Value::as_str).and_then(|b| crypto::b64::decode(b.trim())).unwrap_or_default();
        Some(Background {
            format: String::take(v.get("format")),
            name: String::take(v.get("name")),
            data: Arc::from(data),
            w: u32::take(v.get("w")),
            h: u32::take(v.get("h")),
            x: i32::take(v.get("x")),
            y: i32::take(v.get("y")),
            scale_pm: u32::take(v.get("scale_pm")),
            opacity: u32::take(v.get("opacity")),
            locked: bool::take(v.get("locked")),
        })
    }
}

impl Field for Background {
    fn put(&self) -> Value {
        self.to_json()
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Background::from_json).unwrap_or_default()
    }
}

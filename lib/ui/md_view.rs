// md_view -- ui.markdown_view / ui.html_view: a read-only document view over an MdDoc
// (md_doc.rs) for help pages, README-like content, release notes and chat output
//
//   ui.markdown_view(id, &mut MdViewState, text, MV_*) -> MdEv   Markdown; parsed again only
//        when the text changes (the state keeps a hash of it)
//   ui.html_view(id, &mut MdViewState, html, MV_*) -> MdEv        HTML (the rich-text reader)
//   ui.doc_view(id, &mut MdViewState, MV_*) -> MdEv                the document set before
//        (st.set_markdown / set_html / set_doc)
//   MdEv { ev, link, hover, field, submit }   ev: EV_FOCUS, EV_HOVER, EV_ACT (a link was
//        activated); link: the activated link's target (a click on it, Enter on the focused
//        one); hover: the target of the link under the pointer; field: a form field the user
//        changed (typed, toggled, chose) or a button without a submit / reset action he pressed;
//        submit: a form was sent (MdSubmit: the form, its action and method, the name / value
//        pairs, the button).  The view fills the place it gets (natural 480 x 320 DIP;
//        next_size(-1, -1) fills).
//   MdViewState::new() / base (where relative image paths start, e.g. "app://help/") / label
//        (the name for screen readers; "" = "Dokument") / doc() / selection() /
//        set_selection(sel) / select_all() / selected_text() / selected_html() /
//        scroll_to_block(i) / scroll_to_end() (the end, also while the document grows: chat
//        output; scrolling to the end of a long document does the same until the user scrolls
//        away) / first_visible() -> (block, px above the view) / focused_link()
//   Changes of the document (HTML: its elements, md_dom.rs -- page scripts): st.doc_mut() ->
//        &mut MdDoc; the view takes the changes every frame (st.update(): now): the blocks
//        derived again (MdDoc::update), the layouts and heights of the blocks that stay kept,
//        the view on the block it showed, the selection and the focused link with their blocks,
//        the fields' values by their elements (what the user or the app gave a field stays;
//        the others follow the document's values), a focused field's widget moved with it
//   Images from the app: st.images = MI_LOCAL (default: VFS names and paths, other addresses
//        show their alt text), MI_ASK (as MI_LOCAL; http:, https: and data: addresses are asked
//        from the app) or MI_APP (every image is asked: nothing is read by the view -- pages
//        from the network).  st.image_requests() -> the addresses the view waits for (each
//        once; relative ones after base, the rest as written); st.set_image(addr, Some(bytes) |
//        None) answers (decoded at once by the image decoders: SVG or img_core; None or bytes
//        it cannot decode: the alt text) -- in a frame before the view, or the app asks for one
//        (invalidate) when the bytes come outside.  Until then a frame with the alt text stands
//        in (at the width / height the HTML gives, else the frame's size); the image's block is
//        laid out again when its bytes come, the view stays where it is.
//   Forms (HTML: md_doc.rs "Forms"): the fields are the library's widgets in the text flow --
//        text boxes (password fields: dots, nothing to copy or read out), text areas, check
//        boxes, radios, drop-down lists, buttons -- with the style's look, high contrast, text
//        scale and mirroring.  Their values live in the state: field_text(i) / field_checked(i)
//        / field_selected(i), set_field_text / set_field_checked / set_field_selected,
//        form_data(form, button) (the pairs a browser sends), reset_form(form),
//        focused_field(); a new document starts with its first values (a changed one keeps
//        them, above).  Keys: Tab / Shift+Tab
//        go through links and fields in reading order (a group of radios is one stop: the
//        checked one; Up / Down and Left / Right -- as the radios lie in their paragraph --
//        choose inside it), Space toggles, Enter in a text field, check
//        box or radio sends its form (with its first submit button, none when that is
//        disabled), Enter / Space on a button presses it, Esc in a text field returns to the
//        view, PgUp / PgDn that a field does not take scroll the view.  The fields follow the
//        window's mirroring (as every widget), not the direction of their paragraph.  A
//        document without fields reads as before.
//   Flags: MV_NO_FRAME (no card, a smaller padding), MV_OPEN_LINKS (an activated link is also
//        opened with open_url: its scheme rules and the app's policy apply; never without the
//        user's click or Enter), MV_RTL / MV_LTR (the document's direction; default: that of
//        its first strong character, else the window's)
//
// Layout: the blocks in a column; block quotes indent with a bar, lists with their markers in
// front (bullets by depth: disc, ring, square; numbers; task boxes), headings 2 / 1.5 / 1.25 /
// 1 / 0.875 / 0.85 times the body text in bold (h1 and h2 underlined), code blocks in the
// monospace font on a tinted box with the code editor's highlighting (Highlighter::for_name of
// the fence's language, ui.code_theme colours), tables with column widths from their cells
// (wrapped to fit the column), images at their size (DIP; wider than the column: scaled down;
// SVGs through img_svg, sharp at every size), rules.  Text wraps at text::segment's line
// breaks (clusters inside a word too long).  A paragraph's direction comes from its first
// strong character: Hebrew and Arabic run right to left (their runs shaped and reordered by
// text_shape); indents and markers follow the document's direction.  Only the blocks in view
// are laid out (cached per width, dpi, text scale and fonts) and drawn; scrolling is anchored at
// the first visible block (vlist.rs Rows), so blocks laid out for the first time never move
// what is shown.
// Mouse: a drag selects (the view scrolls at its edges), Shift+click extends, a double click
// selects a word; a click on a link activates it.  Keys (focused): Up / Down scroll by a line,
// PgUp / PgDn and Space / Shift+Space by a view, Home / End to the ends, Tab / Shift+Tab
// through the links and fields (the focused link is framed, a field gets the focus; scrolled
// into view; past the last one the focus moves on), Enter activates the focused link, Esc
// drops it, Ctrl+A selects all,
// Ctrl+C / Ctrl+Insert copy the selection as text (list markers, cells by tabs) and HTML.
// Images: names of the VFS ("app://", "data://" ...) and paths (relative ones after base) are
// read on first view and kept in the state; http(s) addresses are never fetched by the view
// (MI_LOCAL: the alt text shows in a frame instead; MI_ASK / MI_APP: the app fetches them).
// Records: K_BOX id (label: the label, value: the scroll position in DIP); per visible block
// K_HEADING (value: the level), K_TEXT (paragraphs, code, table cells), K_LISTITEM (the first
// block of a list item, value: its number), K_CHECKBOX (a task's box, checked), K_LINK per
// link, K_IMAGE (label: the alt text) per image; per visible field the widget's record
// (K_TEXTBOX, K_CHECKBOX, K_RADIO, K_COMBO, K_BUTTON: label the field's name -- its <label>,
// aria-label, title, placeholder -- or a button's caption; text boxes and lists take it from a
// K_TEXT record without area along their top edge).  Ids: id (focus, keys), part_id(id, 1)
// the scroll bar, md_field_id(id, i) the fields.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Surface;
use img_svg::Svg;
use text::segment;
use text::{FS_SYN_BOLD, FS_SYN_ITALIC};

use crate::cmd::{Draw, DrawRef, Gfx};
use crate::consts::*;
use crate::ed_core::{word_range, TextPos, TextSel};
use crate::highlight::{Highlighter, Token};
use crate::i18n::tr;
use crate::imgview::SvgDraw;
use crate::keys::*;
use crate::list::{anim_decay, anim_steps};
use crate::md_doc::*;
use crate::md_dom::MdNode;
use crate::theme::color_mix;
use crate::value::part_id;
use crate::vlist::Rows;
use crate::{Sp, Ui};

/// markdown_view flags: no card around the document
pub const MV_NO_FRAME: u32 = 1;
/// an activated link is also opened (open_url)
pub const MV_OPEN_LINKS: u32 = 2;
/// the document runs right to left (indents and markers on the right)
pub const MV_RTL: u32 = 4;
/// the document runs left to right (also in a mirrored window)
pub const MV_LTR: u32 = 8;

/// MdViewState::images: VFS names and paths are read, other addresses show their alt text
pub const MI_LOCAL: u8 = 0;
/// as MI_LOCAL, but http:, https: and data: addresses are asked from the app
pub const MI_ASK: u8 = 1;
/// every image is asked from the app (the view reads no file)
pub const MI_APP: u8 = 2;

const NAT_W: i32 = 480;
const NAT_H: i32 = 320;
const PAD: i32 = 16;
const PAD_BARE: i32 = 4;
const PARA_GAP: i32 = 12;
const TIGHT_GAP: i32 = 4;
const LIST_W: i32 = 28;
const QUOTE_W: i32 = 18;
const CODE_PX: i32 = 12;
const CODE_PY: i32 = 10;
const CODE_PAD: i32 = 3;
const CELL_PX: i32 = 10;
const CELL_PY: i32 = 6;
const RULE_H: i32 = 20;
const LINE_DIP: i32 = 40;
const WHEEL_DIP: i32 = 48;
const IMG_BYTES: usize = 32 << 20;
const HEAD_SCALE: [u32; 6] = [200, 150, 125, 100, 88, 85];
const TABLE_SAMPLE: usize = 200;
const LAYS_KEEP: usize = 400;
const LABEL_MAX: usize = 4000;
const FIELD_CHARS: u32 = 20; // a text field's width in characters (HTML's default size / cols)
const FIELD_ROWS: u32 = 2; // a text area's lines (rows)
const FIELD_GAP: i32 = 2; // room above and below a field in its row (DIP)
const FIELD_CAP: u32 = 1 << 20; // bytes a field holds without maxlength

/// what a document view reports (see the module header)
#[derive(Clone, Debug, Default)]
pub struct MdEv {
    /// EV_FOCUS, EV_HOVER, EV_ACT (a link was activated)
    pub ev: u32,
    /// the target of the link activated this frame
    pub link: Option<String>,
    /// the target of the link under the pointer
    pub hover: Option<String>,
    /// a field the user changed this frame, or a button (MF_BUTTON) he pressed (index of
    /// doc().fields())
    pub field: Option<usize>,
    /// a form sent this frame (Enter in a field, a submit button)
    pub submit: Option<MdSubmit>,
}

/// a form being sent: what a browser would send (MdViewState::form_data)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdSubmit {
    /// the form (index of doc().forms()), its action (as written) and method ("get" / "post")
    pub form: usize,
    pub action: String,
    pub method: String,
    /// the name / value pairs in document order
    pub fields: Vec<(String, String)>,
    /// the submit button that sent it (None: Enter in a form without one)
    pub button: Option<usize>,
}

// an image of the document as loaded (Pending: asked from the app, its address)
#[derive(Clone)]
enum Img {
    Unread,
    Failed,
    Pending(Rc<str>),
    Bitmap(Rc<Surface>),
    Vector(Rc<Svg>),
}

// an address the app supplies: asked, or its picture
enum Req {
    Asked,
    Got(Img),
}

// the current value of a field: text (text fields, hidden ones), checked, the chosen option;
// the field's kind and element, whether the user or the app changed it (dirty: changes of the
// document keep it; else it follows the field's first value)
#[derive(Clone, Default)]
struct Val {
    text: String,
    on: bool,
    sel: i32,
    kind: u8,
    node: MdNode,
    dirty: bool,
}

// a stop of Tab: a link spot, a field
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Link(usize),
    Field(usize),
}

/// the widget id of field i of the view `id` (the drop-down lists' rows take ids of their own)
pub fn md_field_id(id: u32, i: usize) -> u32 {
    (id.wrapping_mul(0x9E37_79B1) & 0x0FFF_0000) | 0xE000_0000 | (i as u32 + 1) & 0xFFFF
}

// the field of view `id` whose widget id is fid (n fields)
fn field_of(id: u32, fid: u32, n: usize) -> Option<usize> {
    let k = (fid & 0xFFFF) as usize;
    (fid & 0xFFFF_0000 == md_field_id(id, 0) & 0xFFFF_0000 && k >= 1 && k <= n).then(|| k - 1)
}

// a field's name for screen readers: its label, else its placeholder, else its name
fn field_name(f: &MdField) -> &str {
    if !f.label.is_empty() {
        &f.label
    } else if !f.placeholder.is_empty() {
        &f.placeholder
    } else {
        &f.name
    }
}

// a scheme at the start of an address ("http:", "data:"; not a drive letter)
fn has_scheme(s: &str) -> bool {
    match s.find(':') {
        Some(i) if i >= 2 => s.as_bytes()[0].is_ascii_alphabetic() && s[..i].bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')),
        _ => false,
    }
}

// where an image comes from: (a file to read, else an address the app supplies); (None, None):
// nowhere (the alt text)
fn img_at(src: &str, base: &str, mode: u8) -> (Option<String>, Option<String>) {
    let src = src.trim();
    let low = src.to_ascii_lowercase();
    if src.is_empty() || low.starts_with("javascript:") {
        return (None, None);
    }
    let remote = low.starts_with("http:") || low.starts_with("https:") || low.starts_with("data:");
    let path = if src.contains("://") || src.starts_with('/') || base.is_empty() || (mode == MI_APP && has_scheme(src)) {
        String::from(src)
    } else {
        let s = src.strip_prefix("./").unwrap_or(src);
        if base.ends_with('/') { format!("{}{}", base, s) } else { format!("{}/{}", base, s) }
    };
    match mode {
        MI_APP => (None, Some(path)),
        MI_ASK if remote => (None, Some(String::from(src))),
        _ if remote => (None, None),
        _ => (Some(path), None),
    }
}

// a picture from its bytes (an SVG by its name or its first character, else what img_core
// decodes)
fn img_decode(d: &[u8], name: &str) -> Img {
    if d.len() > IMG_BYTES {
        return Img::Failed;
    }
    let low = name.to_ascii_lowercase();
    let head = core::str::from_utf8(&d[..d.len().min(256)]).unwrap_or("");
    let svg = low.ends_with(".svg") || low.ends_with(".svgz") || head.trim_start_matches('\u{FEFF}').trim_start().starts_with('<');
    if svg {
        Svg::parse(d).map(|s| Img::Vector(Rc::new(s))).unwrap_or(Img::Failed)
    } else {
        img_core::io::decode(d).map(|s| Img::Bitmap(Rc::new(s))).unwrap_or(Img::Failed)
    }
}

// a piece of a row: bytes a .. b of the block's text in one font and colour, an image or a
// field (asc: its height above the baseline)
#[derive(Clone)]
struct Piece {
    a: u32,
    b: u32,
    x: i32,
    w: i32,
    font: u32,
    size: i32,
    asc: i32,
    style: u16,
    color: u32,
    bg: u32,
    link: u32,
    image: u32,
    ih: i32,
    syn: u8,
    shaped: bool,
    field: u32,
}

// a visual row: its bytes, top, height, baseline, pieces and caret stops (byte, x)
struct LRow {
    a: u32,
    b: u32,
    y: i32,
    h: i32,
    base: i32,
    pieces: Vec<Piece>,
    stops: Vec<(u32, i32)>,
}

impl LRow {
    fn hit(&self, x: i32) -> u32 {
        self.stops.iter().min_by_key(|s| (s.1 - x).abs()).map_or(self.a, |s| s.0)
    }
}

// laid out text: rows, their height, the widest row
#[derive(Default)]
struct Flow {
    rows: Vec<LRow>,
    h: i32,
    w: i32,
}

// a text area of a block: x (from the column's start), y (from the content's top), width
struct FBox {
    x: i32,
    y: i32,
    w: i32,
    flow: Flow,
}

// the layout of a block
struct BLay {
    // the whole height (gap included), the gap above, the content's height
    h: i32,
    gap: i32,
    ch: i32,
    // the content column: x from the column's start, width
    x: i32,
    w: i32,
    // the text areas (one; table rows: one per cell)
    boxes: Vec<FBox>,
    // quote bars: x from the column's start
    bars: Vec<i32>,
}

// the faces: [monospace][bold | italic << 1], the body text size (24.8)
#[derive(Clone, Copy, Default, PartialEq)]
struct Faces {
    f: [[u32; 4]; 2],
    size: i32,
}

impl Faces {
    fn get(&self, mono: bool, bold: bool, italic: bool) -> u32 {
        self.f[mono as usize][bold as usize | (italic as usize) << 1]
    }
}

// a run of text with its look (layout input)
#[derive(Clone)]
struct Seg {
    a: usize,
    b: usize,
    font: u32,
    size: i32,
    asc: i32,
    desc: i32,
    lh: i32,
    style: u16,
    color: u32,
    bg: u32,
    link: u32,
    image: u32,
    iw: i32,
    ih: i32,
    syn: u8,
    shaped: bool,
    // room at both ends (inline code's box)
    pad: i32,
    // a field (index + 1): iw x ih, asc above the baseline, desc below it (its row's room
    // included)
    field: u32,
}

// the layout cache of a view
#[derive(Default)]
struct Cache {
    key: (i32, i32, u32, bool, Faces),
    lays: BTreeMap<usize, Rc<BLay>>,
    cols: BTreeMap<u32, Rc<Vec<i32>>>,
    rows_of: BTreeMap<u32, Vec<usize>>,
    imgs: Vec<Img>,
    scaled: BTreeMap<(usize, i32, i32), Rc<Surface>>,
    svgs: BTreeMap<(usize, u32), Rc<SvgDraw>>,
    // where the images come from (MI_*), the addresses the app supplies, the ones it has not
    // taken yet (image_requests)
    mode: u8,
    req: BTreeMap<String, Req>,
    queue: Vec<String>,
}

/// the state of a document view (see the module header)
pub struct MdViewState {
    doc: MdDoc,
    // the source: kind (1 Markdown, 2 HTML, 3 set_doc), hash, length
    src: (u8, u64, usize),
    /// where relative image paths start ("app://help/"; "" = as given)
    pub base: String,
    /// the name for screen readers ("" = "Dokument")
    pub label: String,
    /// where the images come from: MI_LOCAL (default), MI_ASK, MI_APP (see the module header)
    pub images: u8,
    // the block of every image
    img_block: Vec<usize>,
    // the fields' values, the Tab stops (links and fields in reading order), the first radio of
    // every radio's group (+ 1; 0: no group), the field with the focus (last pass)
    vals: Vec<Val>,
    stops: Vec<Stop>,
    group: Vec<usize>,
    ffocus: Option<usize>,
    rows: Rows,
    sel: TextSel,
    link: Option<usize>,
    lc: Cache,
    // the rows were sized for this document
    sized: bool,
    // the document's direction by its first strong character (None: it has none)
    auto_rtl: Option<bool>,
    // the view stays at the end (End, scroll_to_end, scrolled to the end of a long document):
    // a growing document (chat output) keeps showing its end
    follow: bool,
    drag: u8,
    dragged: bool,
    word: (TextPos, TextPos),
    press_link: Option<usize>,
    pending: i32,
    t0: i32,
    // a block range to show: block, y0, y1 (px from the block's top)
    reveal: Option<(usize, i32, i32)>,
    // the pointer's y and the bar's position when the scroll bar last moved the view
    sb_y: i32,
    sb_pos: i32,
    // after changes of the document: the focused field's old and new index (the widget's id
    // moves with it), blocks without a height estimate
    refocus: Option<(usize, Option<usize>)>,
    reguess: bool,
}

impl Default for MdViewState {
    fn default() -> MdViewState {
        MdViewState::new()
    }
}

// a quick hash of a text (no cryptography: tells a changed source apart)
fn text_hash(s: &str) -> u64 {
    let b = s.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut it = b.chunks_exact(8);
    for c in &mut it {
        let v = u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
        h = (h ^ v).wrapping_mul(0x0000_0100_0000_01B3).rotate_left(29);
    }
    for &x in it.remainder() {
        h = (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h ^ b.len() as u64
}

// a right-to-left character (Hebrew, Arabic, Syriac, Thaana, N'Ko, their presentation forms)
fn rtl_char(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

// the direction of a text by its first strong character (None: none)
fn text_rtl(s: &str) -> Option<bool> {
    s.chars().find(|c| c.is_alphabetic()).map(rtl_char)
}

fn opaque(c: u32) -> u32 {
    if c >> 24 == 0 { c | 0xFF00_0000 } else { c }
}

impl MdViewState {
    /// an empty document
    pub fn new() -> MdViewState {
        MdViewState {
            doc: MdDoc::default(),
            src: (0, 0, 0),
            base: String::new(),
            label: String::new(),
            images: MI_LOCAL,
            img_block: Vec::new(),
            vals: Vec::new(),
            stops: Vec::new(),
            group: Vec::new(),
            ffocus: None,
            rows: Rows::default(),
            sel: TextSel::default(),
            link: None,
            lc: Cache::default(),
            sized: false,
            auto_rtl: None,
            follow: false,
            drag: 0,
            dragged: false,
            word: (TextPos::default(), TextPos::default()),
            press_link: None,
            pending: 0,
            t0: 0,
            reveal: None,
            sb_y: i32::MIN,
            sb_pos: 0,
            refocus: None,
            reguess: false,
        }
    }

    /// the document shown
    pub fn doc(&self) -> &MdDoc {
        &self.doc
    }

    /// the document to change (an HTML document's elements, md_dom.rs): the view takes the
    /// changes at its next frame (or update()) and keeps its place, selection, focus and the
    /// values the user gave the fields that stay
    pub fn doc_mut(&mut self) -> &mut MdDoc {
        &mut self.doc
    }

    /// take the document's changes now (MdDoc::update; the view does it every frame) -> there
    /// were changes
    pub fn update(&mut self) -> bool {
        self.sync()
    }

    /// show Markdown (parsed now unless it is the text shown already)
    pub fn set_markdown(&mut self, s: &str) {
        let key = (1, text_hash(s), s.len());
        if key != self.src {
            self.src = key;
            self.set_doc_inner(MdDoc::markdown(s));
        }
    }

    /// show HTML (parsed now unless it is the text shown already)
    pub fn set_html(&mut self, s: &str) {
        let key = (2, text_hash(s), s.len());
        if key != self.src {
            self.src = key;
            self.set_doc_inner(MdDoc::html(s));
        }
    }

    /// show a document
    pub fn set_doc(&mut self, doc: MdDoc) {
        self.src = (3, 0, 0);
        self.set_doc_inner(doc);
    }

    // a new document: the place in it is kept (at the end it stays at the end, as for chat
    // output), the selection and the layouts go
    fn set_doc_inner(&mut self, doc: MdDoc) {
        self.doc = doc;
        self.doc.update();
        self.doc.take_change();
        self.refocus = None;
        self.sel = TextSel::default();
        self.link = None;
        self.press_link = None;
        self.drag = 0;
        self.reveal = None;
        self.lc.lays.clear();
        self.lc.cols.clear();
        self.lc.scaled.clear();
        self.lc.svgs.clear();
        self.lc.imgs = alloc::vec![Img::Unread; self.doc.images().len()];
        self.blocks_index();
        // the app's images: the ones this document uses stay (asked or supplied)
        if !self.lc.req.is_empty() {
            let used: BTreeSet<String> = self.doc.images().iter().filter_map(|m| img_at(&m.src, &self.base, self.lc.mode).1).collect();
            self.lc.req.retain(|a, _| used.contains(a));
            self.lc.queue.retain(|a| used.contains(a));
        }
        self.fields_init(&[]);
        self.ffocus = None;
        self.sized = false;
        self.auto_rtl = self.doc.blocks().iter().filter(|b| b.kind != MB_CODE).find_map(|b| text_rtl(&b.text));
    }

    // the rows of every table, the block of every image
    fn blocks_index(&mut self) {
        self.lc.rows_of.clear();
        self.img_block = alloc::vec![0; self.doc.images().len()];
        for (k, b) in self.doc.blocks().iter().enumerate() {
            if b.kind == MB_ROW {
                self.lc.rows_of.entry(b.table).or_default().push(k);
            }
            for s in b.spans.iter().filter(|s| s.image != 0) {
                if let Some(x) = self.img_block.get_mut(s.image as usize - 1) {
                    *x = k;
                }
            }
        }
    }

    // the fields' values (first values; a field of `old` with the same element and kind that the
    // user or the app changed keeps its value), the radios' groups (form and name), the Tab stops
    fn fields_init(&mut self, old: &[Val]) {
        let by: BTreeMap<MdNode, &Val> = old.iter().filter(|v| !v.node.is_none()).map(|v| (v.node, v)).collect();
        self.vals = self
            .doc
            .fields()
            .iter()
            .map(|f| match by.get(&f.node) {
                Some(v) if v.dirty && v.kind == f.kind => {
                    let mut v = (*v).clone();
                    if v.sel >= f.options.len() as i32 {
                        v.sel = f.selected;
                    }
                    v
                }
                _ => Val {
                    text: if f.kind == MF_CHECKBOX || f.kind == MF_RADIO { String::new() } else { f.value.clone() },
                    on: f.checked,
                    sel: f.selected,
                    kind: f.kind,
                    node: f.node,
                    dirty: false,
                },
            })
            .collect();
        let mut first: BTreeMap<(u32, &str), usize> = BTreeMap::new();
        self.group = alloc::vec![0; self.doc.fields().len()];
        for (i, f) in self.doc.fields().iter().enumerate() {
            if f.kind == MF_RADIO && !f.name.is_empty() {
                self.group[i] = *first.entry((f.form, f.name.as_str())).or_insert(i) + 1;
            }
        }
        // (one checked radio per group: one the user chose, else the last one, as browsers
        // parse it)
        let mut checked: BTreeMap<usize, usize> = BTreeMap::new();
        for i in 0..self.vals.len() {
            if self.vals[i].on && self.group[i] != 0 {
                match checked.get(&self.group[i]).copied() {
                    Some(p) if self.vals[p].dirty && !self.vals[i].dirty => self.vals[i].on = false,
                    Some(p) => {
                        self.vals[p].on = false;
                        checked.insert(self.group[i], i);
                    }
                    None => {
                        checked.insert(self.group[i], i);
                    }
                }
            }
        }
        let (links, fields) = (self.doc.link_spots(), self.doc.field_spots());
        self.stops = Vec::with_capacity(links.len() + fields.len());
        let (mut a, mut b) = (0, 0);
        while a < links.len() || b < fields.len() {
            let take_link = b >= fields.len() || (a < links.len() && (links[a].block, links[a].a) <= (fields[b].block, fields[b].a));
            if take_link {
                self.stops.push(Stop::Link(a));
                a += 1;
            } else {
                self.stops.push(Stop::Field(fields[b].field as usize - 1));
                b += 1;
            }
        }
    }

    // the document's changes (MdDoc::update): the blocks that stay keep their layouts and
    // heights, the view stays on the block it shows (scrolling is anchored there), the selection
    // and the focused link move with their blocks, the fields keep the values the user gave them
    // (by their elements) and the focus -> there were changes
    fn sync(&mut self) -> bool {
        self.doc.update();
        let Some(ch) = self.doc.take_change() else { return false };
        let n = self.doc.blocks().len();
        // an old block -> the new one (inside the changed range: clamped into it)
        let range = ch.range;
        let map = |k: usize| -> (usize, bool) {
            match range {
                Some((a, o, nn)) if k >= a + o => (k - o + nn, false),
                Some((a, o, nn)) if k >= a => (a + (k - a).min(nn.saturating_sub(1)), o != nn),
                _ => (k, false),
            }
        };
        if let Some((a, o, nn)) = range {
            // heights: the new blocks get estimates (reguess), the anchor stays on its block
            let (top, off) = (self.rows.top, self.rows.off);
            if o != nn {
                self.rows.remove(a, o);
                self.rows.insert(a, nn);
            } else {
                for k in a..a + o {
                    self.rows.forget(k);
                }
            }
            self.rows.top = map(top).0.min(n.saturating_sub(1));
            self.rows.off = off;
            let lays = core::mem::take(&mut self.lc.lays);
            for (k, l) in lays {
                if k < a {
                    self.lc.lays.insert(k, l);
                } else if k >= a + o {
                    self.lc.lays.insert(k - o + nn, l);
                }
            }
            // (the blocks around the range: the boxes of their elements may end elsewhere now)
            self.lc.lays.remove(&a.wrapping_sub(1));
            self.lc.lays.remove(&(a + nn));
            self.reguess = true;
        }
        for k in &ch.redo {
            self.lc.lays.remove(k);
        }
        if ch.full {
            // the pictures by address, the tables' columns again, the fields by element
            let mut pics: BTreeMap<&str, Img> = BTreeMap::new();
            for (m, img) in ch.images.iter().zip(self.lc.imgs.iter()) {
                pics.entry(m.src.as_str()).or_insert_with(|| img.clone());
            }
            self.lc.imgs = self.doc.images().iter().map(|m| pics.get(m.src.as_str()).cloned().unwrap_or(Img::Unread)).collect();
            self.lc.scaled.clear();
            self.lc.svgs.clear();
            self.lc.cols.clear();
            let rows: Vec<usize> = self.lc.lays.keys().copied().filter(|&k| self.doc.blocks().get(k).is_some_and(|b| b.kind == MB_ROW)).collect();
            for k in rows {
                self.lc.lays.remove(&k);
            }
            self.blocks_index();
            let old = core::mem::take(&mut self.vals);
            let focus = self.ffocus.and_then(|i| old.get(i)).map(|v| v.node);
            self.fields_init(&old);
            if let Some(i) = self.ffocus {
                let j = focus.and_then(|nd| self.doc.field_of(nd));
                if j != Some(i) {
                    self.refocus = Some((i, j));
                    self.ffocus = j;
                }
            }
            self.link = self.link.and_then(|k| {
                let s = ch.spots.get(k)?;
                let (b, moved) = map(s.block);
                if moved {
                    return None;
                }
                self.doc.link_spots().iter().position(|t| t.block == b && t.a == s.a && t.link != 0)
            });
            self.press_link = None;
        }
        // the selection, a word being dragged, a range to show
        let mp = |p: TextPos| {
            let (line, moved) = map(p.line);
            TextPos::new(line, if moved { 0 } else { p.byte })
        };
        self.sel = TextSel::new(self.clamp(mp(self.sel.anchor)), self.clamp(mp(self.sel.caret)));
        self.word = (self.clamp(mp(self.word.0)), self.clamp(mp(self.word.1)));
        self.reveal = self.reveal.map(|(b, y0, y1)| (map(b).0, y0, y1));
        self.auto_rtl = self.doc.blocks().iter().filter(|b| b.kind != MB_CODE).find_map(|b| text_rtl(&b.text));
        true
    }

    /// the selection (TextPos: block, byte of its text)
    pub fn selection(&self) -> TextSel {
        self.sel
    }

    /// select a range (clamped to the document)
    pub fn set_selection(&mut self, sel: TextSel) {
        self.sel = TextSel::new(self.clamp(sel.anchor), self.clamp(sel.caret));
    }

    /// select the whole document
    pub fn select_all(&mut self) {
        self.sel = TextSel::new(TextPos::default(), self.doc.end());
    }

    /// the selection as text (list markers, cells by tabs, images as their alt texts)
    pub fn selected_text(&self) -> String {
        if self.sel.is_empty() {
            return String::new();
        }
        self.doc.plain_range(self.sel.start(), self.sel.end())
    }

    /// the selection as HTML
    pub fn selected_html(&self) -> String {
        if self.sel.is_empty() {
            return String::new();
        }
        self.doc.html_range(self.sel.start(), self.sel.end())
    }

    /// show block i at the top
    pub fn scroll_to_block(&mut self, i: usize) {
        self.rows.top = i.min(self.doc.blocks().len().saturating_sub(1));
        self.rows.off = 0;
        self.pending = 0;
        self.reveal = None;
        self.follow = false;
    }

    /// show the end, also while the document grows (until the user scrolls away)
    pub fn scroll_to_end(&mut self) {
        self.pending = 0;
        self.reveal = None;
        self.follow = true;
    }

    /// the first visible block and how many px of it are above the view
    pub fn first_visible(&self) -> (usize, i32) {
        (self.rows.top, self.rows.off)
    }

    /// the link focused with Tab (an index of doc().link_spots())
    pub fn focused_link(&self) -> Option<usize> {
        self.link
    }

    // ------------------------------------------------------------------ images from the app

    /// the image addresses the view waits for since the last call (MI_ASK / MI_APP; each one
    /// once): the app fetches them and answers with set_image
    pub fn image_requests(&mut self) -> Vec<String> {
        core::mem::take(&mut self.lc.queue)
    }

    /// the bytes of an image address (None: it could not be had): decoded now, shown from the
    /// next frame on (the alt text when they are no picture) -> an image of the document waited
    /// for it.  An address not asked yet is kept for the images that need it later.
    pub fn set_image(&mut self, addr: &str, data: Option<&[u8]>) -> bool {
        let img = data.map_or(Img::Failed, |d| img_decode(d, addr));
        self.lc.req.insert(String::from(addr), Req::Got(img.clone()));
        self.lc.queue.retain(|a| a != addr);
        let mut hit = false;
        for i in 0..self.lc.imgs.len() {
            if matches!(&self.lc.imgs[i], Img::Pending(a) if &**a == addr) {
                self.lc.imgs[i] = img.clone();
                self.lc.scaled.retain(|k, _| k.0 != i);
                self.lc.svgs.retain(|k, _| k.0 != i);
                // its block is laid out again (a table row: the whole table, its columns may
                // change)
                let k = self.img_block.get(i).copied().unwrap_or(0);
                match self.doc.blocks().get(k) {
                    Some(b) if b.kind == MB_ROW => {
                        self.lc.cols.remove(&b.table);
                        for r in self.lc.rows_of.get(&b.table).into_iter().flatten() {
                            self.lc.lays.remove(r);
                        }
                    }
                    _ => {
                        self.lc.lays.remove(&k);
                    }
                }
                hit = true;
            }
        }
        hit
    }

    // ------------------------------------------------------------------ forms

    /// field i's text (text fields, text areas, hidden fields; "" for the other kinds)
    pub fn field_text(&self, i: usize) -> &str {
        self.vals.get(i).map_or("", |v| v.text.as_str())
    }

    /// field i is checked (check boxes, radios)
    pub fn field_checked(&self, i: usize) -> bool {
        self.vals.get(i).is_some_and(|v| v.on)
    }

    /// the option chosen in field i (drop-down lists; -1: none)
    pub fn field_selected(&self, i: usize) -> i32 {
        self.vals.get(i).map_or(-1, |v| v.sel)
    }

    /// set field i's text (text fields, text areas, hidden fields; changes of the document keep
    /// it, as the user's input)
    pub fn set_field_text(&mut self, i: usize, s: &str) {
        if let Some(v) = self.vals.get_mut(i) {
            v.text = String::from(s);
            v.dirty = true;
        }
    }

    /// check field i or not (a radio: the others of its group go off)
    pub fn set_field_checked(&mut self, i: usize, on: bool) {
        if i >= self.vals.len() {
            return;
        }
        let g = self.group[i];
        if on && g != 0 {
            for (v, &h) in self.vals.iter_mut().zip(self.group.iter()) {
                if h == g {
                    v.on = false;
                    v.dirty = true;
                }
            }
        }
        self.vals[i].on = on;
        self.vals[i].dirty = true;
    }

    /// choose option k of field i (a drop-down list; -1: none)
    pub fn set_field_selected(&mut self, i: usize, k: i32) {
        let n = self.doc.fields().get(i).map_or(0, |f| f.options.len()) as i32;
        if let Some(v) = self.vals.get_mut(i) {
            v.sel = if k >= 0 && k < n { k } else { -1 };
            v.dirty = true;
        }
    }

    /// the name / value pairs form `form` sends with the submit button `button` (as browsers
    /// build them: fields with a name that are not disabled; check boxes and radios when
    /// checked; a list's chosen value; of the buttons only `button`)
    pub fn form_data(&self, form: usize, button: Option<usize>) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (i, f) in self.doc.fields().iter().enumerate() {
            if f.form as usize != form + 1 || f.disabled || f.name.is_empty() {
                continue;
            }
            let v = &self.vals[i];
            let value = match f.kind {
                MF_CHECKBOX | MF_RADIO if v.on => f.value.clone(),
                MF_SELECT if v.sel >= 0 => f.options.get(v.sel as usize).map_or(String::new(), |o| o.value.clone()),
                MF_SUBMIT if button == Some(i) => f.value.clone(),
                MF_TEXT | MF_PASSWORD | MF_TEXTAREA | MF_HIDDEN => v.text.clone(),
                _ => continue,
            };
            out.push((f.name.clone(), value));
        }
        out
    }

    /// the fields of form `form` get their first values back
    pub fn reset_form(&mut self, form: usize) {
        for (i, f) in self.doc.fields().iter().enumerate() {
            if f.form as usize == form + 1 {
                let v = &mut self.vals[i];
                if f.kind != MF_CHECKBOX && f.kind != MF_RADIO {
                    v.text = f.value.clone();
                }
                v.on = f.checked;
                v.sel = f.selected;
                v.dirty = false;
            }
        }
    }

    /// the field with the keyboard focus (an index of doc().fields())
    pub fn focused_field(&self) -> Option<usize> {
        self.ffocus
    }

    // the form field i sends with Enter: its form with the first submit button (none when
    // that is disabled; a field outside a form sends nothing)
    fn implicit(&self, i: usize) -> Option<MdSubmit> {
        let f = self.doc.fields().get(i)?;
        let form = (f.form as usize).checked_sub(1)?;
        let button = self.doc.fields().iter().position(|b| b.kind == MF_SUBMIT && b.form == f.form);
        if button.is_some_and(|b| self.doc.fields()[b].disabled) {
            return None;
        }
        Some(self.submit(form, button))
    }

    // form `form` sent with `button`
    fn submit(&self, form: usize, button: Option<usize>) -> MdSubmit {
        let fm = self.doc.forms().get(form);
        MdSubmit {
            form,
            action: fm.map_or(String::new(), |m| m.action.clone()),
            method: fm.map_or(String::from("get"), |m| m.method.clone()),
            fields: self.form_data(form, button),
            button,
        }
    }

    // the next / previous Tab stop after stop `cur` (None: before the first) -> None: past the
    // ends.  A field is one when it is not disabled; of a group of radios only the checked one
    // (else its first).
    fn step(&self, cur: Option<usize>, back: bool) -> Option<Stop> {
        let n = self.stops.len();
        let checked: BTreeMap<usize, usize> = (0..self.vals.len()).filter(|&j| self.group[j] != 0 && self.vals[j].on).map(|j| (self.group[j], j)).collect();
        let stop = |i: usize| {
            let Some(f) = self.doc.fields().get(i) else { return false };
            let g = self.group[i];
            !f.disabled && (g == 0 || checked.get(&g).copied().unwrap_or(g - 1) == i)
        };
        let mut k = cur;
        loop {
            k = match (k, back) {
                (None, false) => (n > 0).then_some(0),
                (None, true) => None,
                (Some(i), false) => (i + 1 < n).then_some(i + 1),
                (Some(i), true) => i.checked_sub(1),
            };
            let s = self.stops[k?];
            match s {
                Stop::Field(f) if !stop(f) => continue,
                _ => return Some(s),
            }
        }
    }

    // the radio before / after radio i in its group (wrapping)
    fn radio_step(&self, i: usize, back: bool) -> Option<usize> {
        let g = *self.group.get(i)?;
        if g == 0 {
            return None;
        }
        let members: Vec<usize> = (0..self.group.len()).filter(|&j| self.group[j] == g && !self.doc.fields()[j].disabled).collect();
        let p = members.iter().position(|&j| j == i)?;
        let n = members.len();
        Some(members[if back { (p + n - 1) % n } else { (p + 1) % n }])
    }

    fn clamp(&self, p: TextPos) -> TextPos {
        let bl = self.doc.blocks();
        if bl.is_empty() {
            return TextPos::default();
        }
        let line = p.line.min(bl.len() - 1);
        let t = &bl[line].text;
        let mut b = p.byte.min(t.len());
        while !t.is_char_boundary(b) {
            b -= 1;
        }
        TextPos::new(line, b)
    }
}

// the first link spot of block k that matches (the spots are in reading order)
fn spot_of(spots: &[MdLinkSpot], k: usize, f: impl Fn(&MdLinkSpot) -> bool) -> Option<usize> {
    let i0 = spots.partition_point(|s| s.block < k);
    spots[i0..].iter().take_while(|s| s.block == k).position(f).map(|i| i0 + i)
}

// a text area's x on the screen: the column mirrored for a right-to-left document
fn box_x(cx: i32, cw: i32, x: i32, w: i32, rtl: bool) -> i32 {
    if rtl { cx + cw - x - w } else { cx + x }
}

impl Ui {
    /// ui_markdown_view(id, &mut st, text, MV_*) -> MdEv (see the module header of md_view.rs)
    pub fn markdown_view(&mut self, id: u32, st: &mut MdViewState, text: &str, flags: u32) -> MdEv {
        st.set_markdown(text);
        self.doc_view(id, st, flags)
    }

    /// ui_html_view(id, &mut st, html, MV_*) -> MdEv: the HTML counterpart of markdown_view
    pub fn html_view(&mut self, id: u32, st: &mut MdViewState, html: &str, flags: u32) -> MdEv {
        st.set_html(html);
        self.doc_view(id, st, flags)
    }

    // ------------------------------------------------------------------ layout

    fn md_faces(&mut self) -> Faces {
        self.ts_font(TS_BODY);
        let (_, size) = self.text.get_font();
        let mut f = Faces { size, ..Faces::default() };
        for mono in 0..2 {
            for k in 0..4 {
                let fam = if mono == 1 { "monospace" } else { "" };
                f.f[mono][k] = self.rt_face(fam, k & 1 != 0, k & 2 != 0);
            }
        }
        f
    }

    // metrics of a font: ascent, descent, line height
    fn md_metrics(&mut self, font: u32, size: i32) -> (i32, i32, i32) {
        self.text.set_font(font, size);
        let (asc, desc) = (self.text.ascent(), self.text.descent());
        (asc, desc, self.text.line_h().max(asc + desc))
    }

    // an image: loaded on first use (VFS names, paths after base; the app's addresses asked
    // once: Pending until they come)
    fn md_img(&mut self, lc: &mut Cache, doc: &MdDoc, base: &str, i: usize) -> Img {
        if let Some(Img::Unread) = lc.imgs.get(i) {
            let src = doc.images().get(i).map_or("", |m| m.src.as_str()).trim();
            let img = match img_at(src, base, lc.mode) {
                (Some(path), _) => match plat::read_file(&path) {
                    Some(d) => img_decode(&d, src),
                    None => Img::Failed,
                },
                (None, Some(addr)) => match lc.req.get(&addr) {
                    Some(Req::Got(img)) => img.clone(),
                    Some(Req::Asked) => Img::Pending(Rc::from(addr.as_str())),
                    None => {
                        lc.queue.push(addr.clone());
                        let p = Img::Pending(Rc::from(addr.as_str()));
                        lc.req.insert(addr, Req::Asked);
                        p
                    }
                },
                _ => Img::Failed,
            };
            lc.imgs[i] = img;
        }
        lc.imgs.get(i).cloned().unwrap_or(Img::Failed)
    }

    // an image's size in px at most `avail` wide (a frame with the alt text when it failed;
    // while it is asked: the size the HTML gives, else that frame)
    fn md_img_size(&mut self, lc: &mut Cache, doc: &MdDoc, base: &str, i: usize, avail: i32, fc: &Faces) -> (i32, i32) {
        let m = doc.images().get(i);
        let (aw, ah) = m.map_or((0, 0), |m| (m.w as i32, m.h as i32));
        let nat = match self.md_img(lc, doc, base, i) {
            Img::Bitmap(sf) => Some((sf.w as f64, sf.h as f64)),
            Img::Vector(s) => Some(s.size()),
            Img::Pending(_) if aw > 0 && ah > 0 => Some((aw as f64, ah as f64)),
            _ => None,
        };
        let (w, h) = match nat {
            Some((w, h)) if w > 0.0 && h > 0.0 => {
                let (w, h) = match (aw > 0, ah > 0) {
                    (true, true) => (aw as f64, ah as f64),
                    (true, false) => (aw as f64, aw as f64 * h / w),
                    (false, true) => (ah as f64 * w / h, ah as f64),
                    _ => (w, h),
                };
                (self.dp((w + 0.5).min(100_000.0) as i32).max(1), self.dp((h + 0.5).min(100_000.0) as i32).max(1))
            }
            _ => {
                // the alt text in a frame
                let alt = m.map_or("", |m| m.alt.as_str());
                let alt = if alt.is_empty() { tr("Bild") } else { alt };
                let (_, _, lh) = self.md_metrics(fc.get(false, false, false), fc.size);
                let tw = self.text.width(alt);
                (tw + self.dp(16), lh + self.dp(8))
            }
        };
        if w > avail && avail > 0 {
            (avail, ((h as i64 * avail as i64) / w as i64).max(1) as i32)
        } else {
            (w, h)
        }
    }

    // a field's box in px (at most `avail` wide; the style's sizes, text fields as wide as their
    // characters) and its room above / below the baseline: its middle on the middle of the
    // body text, a gap above and below
    fn md_field_size(&mut self, doc: &MdDoc, i: usize, avail: i32, fc: &Faces) -> (i32, i32, i32, i32) {
        let Some(f) = doc.fields().get(i) else { return (1, 1, 1, 0) };
        let st = self.style;
        self.ts_font(TS_BODY);
        let cw = self.text.width("0").max(1);
        let (w, h) = match f.kind {
            MF_TEXT | MF_PASSWORD | MF_TEXTAREA => {
                let multi = f.kind == MF_TEXTAREA;
                let lh = self.ts_line(TS_BODY);
                let mut sp = Sp { value2: lh, flags: multi as u32, ..Sp::default() };
                let (_, h) = st.textbox_metrics(self, &mut sp);
                let n = if f.size > 0 { f.size.min(1000) } else { FIELD_CHARS } as i32;
                let w = sp.s[0] + sp.s[1] + cw * n;
                if multi {
                    let rows = if f.rows > 0 { f.rows.min(200) } else { FIELD_ROWS } as i32;
                    (w, sp.s[2] * 2 + lh * rows)
                } else {
                    self.ctrl_size(w, h, 0, false)
                }
            }
            MF_CHECKBOX | MF_RADIO => {
                let mut sp = Sp::default();
                if f.kind == MF_CHECKBOX {
                    st.checkbox_size(self, &mut sp);
                } else {
                    st.radio_size(self, &mut sp);
                }
                (sp.s[0], sp.s[0])
            }
            MF_SELECT => {
                self.ts_font(TS_BODY);
                let widest = f.options.iter().map(|o| self.text.width(&o.label)).max().unwrap_or(0);
                let mut sp = Sp { value: widest, ..Sp::default() };
                let (w, h) = st.combo_size(self, &mut sp);
                self.ctrl_size(w, h, 0, false)
            }
            MF_HIDDEN => (1, 1),
            _ => {
                let mut sp = Sp { label: Some(f.caption()), ..Sp::default() };
                let (w, h) = st.button_size(self, &mut sp);
                self.ctrl_size(w, h, 0, false)
            }
        };
        let (w, h) = (w.min(avail.max(1)).max(1), h.max(1));
        let (asc, desc, _) = self.md_metrics(fc.get(false, false, false), fc.size);
        let up = ((h + asc - desc) / 2).clamp(0, h);
        let gap = self.dp(FIELD_GAP);
        (w, h, up + gap, h - up + gap)
    }

    // the runs of bytes a .. b of a block's text from its spans
    #[allow(clippy::too_many_arguments)]
    fn md_segs(&mut self, lc: &mut Cache, doc: &MdDoc, base: &str, k: usize, a: usize, b: usize, scale: u32, bold: bool, avail: i32, fc: &Faces) -> Vec<Seg> {
        let bl = &doc.blocks()[k];
        let text = bl.text.as_str();
        let size = (fc.size as i64 * scale as i64 / 100) as i32;
        let mut out: Vec<Seg> = Vec::new();
        let mut metr: Vec<((u32, i32), (i32, i32, i32))> = Vec::new();
        let mut add = |ui: &mut Ui, lc: &mut Cache, s0: usize, s1: usize, sp: Option<&MdSpan>| {
            if s0 >= s1 {
                return;
            }
            let style = sp.map_or(0, |s| s.style);
            let code = style & MS_CODE != 0;
            let (bo, it) = (bold || style & MS_BOLD != 0, style & MS_ITALIC != 0);
            let shaped = text[s0..s1].chars().any(rtl_char);
            let mut font = fc.get(code, bo, it);
            if shaped {
                font = ui.md_rtl_font(font, &text[s0..s1], bo, it);
            }
            let sz = if code { size * 9 / 10 } else { size };
            let m = match metr.iter().find(|m| m.0 == (font, sz)) {
                Some(m) => m.1,
                None => {
                    let m = ui.md_metrics(font, sz);
                    metr.push(((font, sz), m));
                    m
                }
            };
            let image = sp.map_or(0, |s| s.image);
            let (iw, ih) = if image != 0 { ui.md_img_size(lc, doc, base, image as usize - 1, avail, fc) } else { (0, 0) };
            // a field: its box and its room above / below the baseline
            let field = sp.map_or(0, |s| s.field);
            let (iw, ih, m) = if field != 0 {
                let (w, h, up, down) = ui.md_field_size(doc, field as usize - 1, avail, fc);
                (w, h, (up, down, up + down))
            } else {
                (iw, ih, m)
            };
            out.push(Seg {
                a: s0,
                b: s1,
                font,
                size: sz,
                asc: m.0,
                desc: m.1,
                lh: m.2,
                style,
                color: sp.map_or(0, |s| s.color),
                bg: sp.map_or(0, |s| s.bg),
                link: sp.map_or(0, |s| s.link),
                image,
                iw,
                ih,
                syn: 0,
                shaped,
                pad: if code { ui.dp_text(CODE_PAD) } else { 0 },
                field,
            });
        };
        let mut i = a;
        for s in &bl.spans {
            let (sa, sb) = ((s.a as usize).max(a), (s.b as usize).min(b));
            if sb <= sa {
                continue;
            }
            if sa > i {
                add(self, lc, i, sa, None);
            }
            add(self, lc, sa.max(i), sb, Some(s));
            i = i.max(sb);
        }
        if i < b {
            add(self, lc, i, b, None);
        }
        out
    }

    // the font of a right-to-left run: the run's own when it has the script, else the font of
    // the fallback chain that has it (shaping takes the script's tables from one font), styled
    // alike
    fn md_rtl_font(&mut self, font: u32, s: &str, bold: bool, italic: bool) -> u32 {
        let Some(c) = s.chars().find(|&c| rtl_char(c) && c.is_alphabetic()) else { return font };
        if self.text.has(font, c as u32) {
            return font;
        }
        let Some(f) = self.text.fallback_for(c as u32) else { return font };
        let flags = if bold { FS_SYN_BOLD } else { 0 } | if italic { FS_SYN_ITALIC } else { 0 };
        if flags == 0 {
            return f;
        }
        let r = self.text.style(f, flags);
        if r >= 0 { r as u32 } else { f }
    }

    // the runs of a code block: the highlighter's tokens of every line
    fn md_code_segs(&mut self, text: &str, lang: &str, fc: &Faces) -> Vec<Seg> {
        let h = Highlighter::for_name(lang);
        let font = fc.get(true, false, false);
        let size = fc.size * 9 / 10;
        let (asc, desc, lh) = self.md_metrics(font, size);
        let fb = fc.get(true, true, false);
        let (basc, bdesc, blh) = self.md_metrics(fb, size);
        let theme = self.code_theme();
        let mut out: Vec<Seg> = Vec::new();
        let mut push = |a: usize, b: usize, kind: u8| {
            if a >= b {
                return;
            }
            let bold = theme.bold & (1 << kind) != 0;
            if let Some(l) = out.last_mut() {
                if l.b == a && l.syn == kind + 1 {
                    l.b = b;
                    return;
                }
            }
            let (f, m) = if bold { (fb, (basc, bdesc, blh)) } else { (font, (asc, desc, lh)) };
            out.push(Seg {
                a,
                b,
                font: f,
                size,
                asc: m.0,
                desc: m.1,
                lh: m.2,
                style: if theme.italic & (1 << kind) != 0 { MS_ITALIC } else { 0 },
                color: 0,
                bg: 0,
                link: 0,
                image: 0,
                iw: 0,
                ih: 0,
                syn: kind + 1,
                shaped: false,
                pad: 0,
                field: 0,
            });
        };
        let mut state = 0;
        let mut toks: Vec<Token> = Vec::new();
        let mut o = 0;
        for line in text.split('\n') {
            state = h.line(line, state, &mut toks);
            let mut i = 0;
            for t in &toks {
                let (ta, tb) = (t.start.min(line.len()), t.end.min(line.len()));
                if ta > i {
                    push(o + i, o + ta, 0);
                }
                push(o + ta.max(i), o + tb, t.kind);
                i = i.max(tb);
            }
            push(o + i, o + line.len(), 0);
            if o + line.len() < text.len() {
                push(o + line.len(), o + line.len() + 1, 0);
            }
            o += line.len() + 1;
        }
        out
    }

    // lay out bytes a .. b of a text in rows at most `avail` px wide (align: 0 start, 1 centre,
    // 2 end; rtl: the rows run right to left; stops: the caret stops are needed).  Linear in
    // the text: the runs are found by binary search, an atom wider than a row is split by
    // its clusters' widths.
    #[allow(clippy::too_many_arguments)]
    fn md_flow(&mut self, text: &str, a: usize, b: usize, segs: &[Seg], avail: i32, align: u8, rtl: bool, empty: (i32, i32, i32), stops_too: bool) -> Flow {
        let tb = text.as_bytes();
        let shaping0 = self.text.set_shaping(false);
        let avail = avail.max(1);
        let lim = (avail as i64) << 8;
        // the width (24.8) of x .. y
        let width = |ui: &mut Ui, x: usize, y: usize| -> i64 {
            let mut pen = 0i64;
            let k0 = segs.partition_point(|s| s.b <= x);
            for s in &segs[k0..] {
                if s.a >= y {
                    break;
                }
                let (s0, s1) = (s.a.max(x), s.b.min(y));
                if s0 >= s1 {
                    continue;
                }
                if s.image != 0 || s.field != 0 {
                    pen += (s.iw as i64) << 8;
                    continue;
                }
                ui.text.set_font(s.font, s.size);
                ui.text.set_shaping(s.shaped);
                pen += ui.text.measure_n(&tb[s0..], s1 - s0) as i64;
                if s.pad > 0 {
                    pen += (((s0 == s.a) as i64) + ((s1 == s.b) as i64)) * ((s.pad as i64) << 8);
                }
            }
            pen
        };
        // the end of x .. e without its trailing blanks and line end
        let vis = |e: usize, s: usize| {
            let mut v = e;
            while v > s && matches!(tb[v - 1], b' ' | b'\t' | b'\n') {
                v -= 1;
            }
            v
        };
        let sub = &text[a..b];
        let mut rows_ab: Vec<(usize, usize)> = Vec::new();
        if !sub.is_empty() {
            let ops: Vec<(usize, bool)> = segment::line_breaks(sub).into_iter().map(|o| (o.0 + a, o.1)).collect();
            let mut start = a;
            let mut x = 0i64;
            let mut prev = a;
            for &(o, mand) in &ops {
                if o <= prev {
                    continue;
                }
                let atom = width(self, prev, vis(o, prev));
                if x + atom > lim && prev > start {
                    rows_ab.push((start, prev));
                    start = prev;
                    x = 0;
                }
                if x == 0 && atom > lim {
                    // an atom wider than a row: rows of the clusters that fit
                    let (mut s, mut acc, mut e) = (start, 0i64, start);
                    let ve = vis(o, start);
                    while e < ve {
                        let n = segment::grapheme_next(text, e).min(ve);
                        let gw = width(self, e, n);
                        if acc + gw > lim && e > s {
                            rows_ab.push((s, e));
                            s = e;
                            acc = 0;
                        }
                        acc += gw;
                        e = n;
                    }
                    start = s;
                }
                x += width(self, prev.max(start), o);
                prev = o;
                if mand && o < b {
                    rows_ab.push((start, o));
                    start = o;
                    x = 0;
                }
            }
            if start < b || rows_ab.is_empty() {
                rows_ab.push((start, b));
            }
        } else {
            rows_ab.push((a, a));
        }
        let mut flow = Flow::default();
        let mut y = 0;
        for &(ra, rb) in &rows_ab {
            let mut pieces: Vec<Piece> = Vec::new();
            let (mut asc, mut desc, mut lh) = (0, 0, 0);
            let mut pen = 0i64;
            let k0 = segs.partition_point(|s| s.b <= ra);
            for s in &segs[k0..] {
                if s.a >= rb {
                    break;
                }
                let (s0, s1) = (s.a.max(ra), s.b.min(rb));
                if s0 >= s1 {
                    continue;
                }
                if s.pad > 0 && s0 == s.a {
                    pen += (s.pad as i64) << 8;
                }
                let base = Piece {
                    a: s0 as u32,
                    b: s1 as u32,
                    x: ((pen + 128) >> 8) as i32,
                    w: 0,
                    font: s.font,
                    size: s.size,
                    asc: s.asc,
                    style: s.style,
                    color: s.color,
                    bg: s.bg,
                    link: s.link,
                    image: s.image,
                    ih: s.ih,
                    syn: s.syn,
                    shaped: s.shaped,
                    field: s.field,
                };
                if s.field != 0 {
                    asc = asc.max(s.asc);
                    desc = desc.max(s.desc);
                    lh = lh.max(s.lh);
                    pieces.push(Piece { w: s.iw, ..base });
                    pen += (s.iw as i64) << 8;
                    continue;
                }
                if s.image != 0 {
                    asc = asc.max(s.ih);
                    lh = lh.max(s.ih);
                    pieces.push(Piece { w: s.iw, ..base });
                    pen += (s.iw as i64) << 8;
                    continue;
                }
                asc = asc.max(s.asc);
                desc = desc.max(s.desc);
                lh = lh.max(s.lh);
                self.text.set_font(s.font, s.size);
                self.text.set_shaping(s.shaped);
                let w = self.text.measure_n(&tb[s0..], s1 - s0) as i64;
                pieces.push(Piece { w: ((w + 255) >> 8) as i32, ..base });
                pen += w;
                if s.pad > 0 && s1 == s.b {
                    pen += (s.pad as i64) << 8;
                }
            }
            if pieces.is_empty() {
                (asc, desc, lh) = empty;
            }
            let h = lh.max(asc + desc).max(1);
            let base = asc + (h - asc - desc) / 2;
            let ve = vis(rb, ra);
            let vis_w = ((width(self, ra, ve) + 255) >> 8) as i32;
            flow.w = flow.w.max(vis_w);
            if rtl {
                for p in pieces.iter_mut() {
                    p.x = vis_w - p.x - p.w;
                }
            }
            let spare = (avail - vis_w).max(0);
            let off = match (align, rtl) {
                (1, _) => spare / 2,
                (0, true) | (2, false) => spare,
                _ => 0,
            };
            // the caret stops: the grapheme boundaries of every piece (measured from the
            // piece's start, kerning included; very long pieces cluster by cluster)
            let mut stops: Vec<(u32, i32)> = Vec::new();
            for p in pieces.iter_mut() {
                p.x += off;
                if !stops_too {
                    continue;
                }
                if p.image != 0 || p.field != 0 {
                    let (l, r) = if rtl { (p.x + p.w, p.x) } else { (p.x, p.x + p.w) };
                    stops.push((p.a, l));
                    stops.push((p.b, r));
                    continue;
                }
                let s = &tb[p.a as usize..p.b as usize];
                self.text.set_font(p.font, p.size);
                self.text.set_shaping(p.shaped);
                let long = s.len() > 512 && !p.shaped;
                let (mut g, mut acc) = (p.a as usize, 0i64);
                loop {
                    let k = g - p.a as usize;
                    let rel = if p.shaped {
                        self.text.caret_x(s, k)
                    } else if long {
                        ((acc + 128) >> 8) as i32
                    } else {
                        (self.text.measure_n(s, k) + 128) >> 8
                    };
                    stops.push((g as u32, p.x + rel));
                    if g >= p.b as usize {
                        break;
                    }
                    let n = segment::grapheme_next(text, g).min(p.b as usize);
                    if long {
                        acc += self.text.measure_n(&tb[g..], n - g) as i64;
                    }
                    g = n;
                }
            }
            if stops.is_empty() {
                stops.push((ra as u32, if rtl { avail - off.min(avail) } else { off }));
            }
            stops.dedup_by_key(|s| s.0);
            flow.rows.push(LRow { a: ra as u32, b: rb as u32, y, h, base, pieces, stops });
            y += h;
        }
        flow.h = y;
        self.text.set_shaping(shaping0);
        flow
    }

    // the column widths of table t in a column `cw` px wide (from the cells' natural widths)
    fn md_cols(&mut self, lc: &mut Cache, doc: &MdDoc, base: &str, t: u32, cw: i32, fc: &Faces) -> Rc<Vec<i32>> {
        if let Some(c) = lc.cols.get(&t) {
            return c.clone();
        }
        let rows: Vec<usize> = lc.rows_of.get(&t).map(|v| v.iter().copied().take(TABLE_SAMPLE).collect()).unwrap_or_default();
        let ncols = doc.tables().get(t as usize).map_or(0, |x| x.aligns.len()).max(rows.iter().map(|&k| doc.blocks()[k].cells.len()).max().unwrap_or(0)).max(1);
        let pad = self.dp_text(CELL_PX) * 2;
        let mut nat = alloc::vec![self.dp(24); ncols];
        let empty = self.md_metrics(fc.get(false, false, false), fc.size);
        for &k in &rows {
            let bl = &doc.blocks()[k];
            for (c, &(ca, cb)) in bl.cells.iter().enumerate().take(ncols) {
                let segs = self.md_segs(lc, doc, base, k, ca as usize, cb as usize, 100, bl.head, cw, fc);
                let f = self.md_flow(&bl.text, ca as usize, cb as usize, &segs, 1 << 22, 0, false, empty, false);
                nat[c] = nat[c].max(f.w + pad);
            }
        }
        let total: i64 = nat.iter().map(|&w| w as i64).sum();
        let mut wd = nat.clone();
        if total > cw as i64 {
            // narrow columns keep their width, the others share the rest
            let mut fixed = alloc::vec![false; ncols];
            loop {
                let free: i64 = cw as i64 - (0..ncols).filter(|&c| fixed[c]).map(|c| nat[c] as i64).sum::<i64>();
                let n = fixed.iter().filter(|f| !**f).count().max(1) as i64;
                let share = (free / n).max(self.dp(48) as i64);
                let mut changed = false;
                for c in 0..ncols {
                    if !fixed[c] && (nat[c] as i64) <= share {
                        fixed[c] = true;
                        changed = true;
                    }
                }
                if !changed {
                    for c in 0..ncols {
                        if !fixed[c] {
                            wd[c] = share as i32;
                        }
                    }
                    break;
                }
            }
        }
        let rc = Rc::new(wd);
        lc.cols.insert(t, rc.clone());
        rc
    }

    // the layout of block k in a column cw px wide
    #[allow(clippy::too_many_arguments)]
    fn md_block(&mut self, lc: &mut Cache, doc: &MdDoc, base: &str, k: usize, cw: i32, drtl: bool, fc: &Faces) -> BLay {
        let bl = &doc.blocks()[k];
        let depth = (bl.quote as usize + bl.list as usize).min(32);
        let mut x = 0;
        let mut bars = Vec::new();
        for lv in 0..depth {
            if bl.path >> lv & 1 != 0 {
                bars.push(x + self.dp(2));
                x += self.dp_text(QUOTE_W);
            } else {
                x += self.dp_text(LIST_W);
            }
        }
        let w = (cw - x).max(self.dp(48));
        let mut gap = match bl.space {
            0 => 0,
            1 => self.dp_text(TIGHT_GAP),
            _ => self.dp_text(PARA_GAP),
        };
        let rtl = text_rtl(&bl.text).unwrap_or(drtl);
        let empty = self.md_metrics(fc.get(false, false, false), fc.size);
        let align = match (bl.align, rtl) {
            (1, _) => 1,
            (2, false) | (0, true) => 2,
            _ => 0,
        };
        // align 2 for a right-to-left block means its start (the flow takes the direction)
        let falign = if rtl { [0, 1, 0][align as usize] } else { align };
        let falign = if bl.align == 2 && rtl { 2 } else { falign };
        let mut boxes = Vec::new();
        let ch;
        match bl.kind {
            MB_HEADING => {
                let lv = bl.level.clamp(1, 6) as usize;
                if bl.space > 0 {
                    gap = gap.max(self.dp_text(if lv <= 2 { 22 } else { 16 }));
                }
                let scale = HEAD_SCALE[lv - 1];
                let segs = self.md_segs(lc, doc, base, k, 0, bl.text.len(), scale, true, w, fc);
                let hm = self.md_metrics(fc.get(false, true, false), (fc.size as i64 * scale as i64 / 100) as i32);
                let flow = self.md_flow(&bl.text, 0, bl.text.len(), &segs, w, falign, rtl, hm, true);
                ch = flow.h + if lv <= 2 { self.dp(10) } else { 0 };
                boxes.push(FBox { x, y: 0, w, flow });
            }
            MB_CODE => {
                let (px, py) = (self.dp_text(CODE_PX), self.dp_text(CODE_PY));
                let segs = self.md_code_segs(&bl.text, &bl.lang, fc);
                let mono = self.md_metrics(fc.get(true, false, false), fc.size * 9 / 10);
                let flow = self.md_flow(&bl.text, 0, bl.text.len(), &segs, w - px * 2, 0, false, mono, true);
                ch = flow.h + py * 2;
                boxes.push(FBox { x: x + px, y: py, w: w - px * 2, flow });
            }
            MB_RULE => {
                ch = self.dp_text(RULE_H);
            }
            MB_ROW => {
                let cols = self.md_cols(lc, doc, base, bl.table, w, fc);
                let aligns = doc.tables().get(bl.table as usize).map(|t| t.aligns.clone()).unwrap_or_default();
                let (px, py) = (self.dp_text(CELL_PX), self.dp_text(CELL_PY));
                let mut cx = x;
                let mut hmax = 0;
                for (c, &cwd) in cols.iter().enumerate() {
                    let (ca, cb) = bl.cells.get(c).map_or((bl.text.len(), bl.text.len()), |&(a, b)| (a as usize, b as usize));
                    let segs = self.md_segs(lc, doc, base, k, ca, cb, 100, bl.head, cwd - px * 2, fc);
                    // a column's alignment is physical: right stays right in a right-to-left table
                    let al = match aligns.get(c).copied().unwrap_or(MA_NONE) {
                        MA_CENTER => 1,
                        MA_RIGHT => if rtl { 0 } else { 2 },
                        MA_LEFT => if rtl { 2 } else { 0 },
                        _ => 0,
                    };
                    let flow = self.md_flow(&bl.text, ca, cb, &segs, cwd - px * 2, al, rtl, empty, true);
                    hmax = hmax.max(flow.h);
                    boxes.push(FBox { x: cx + px, y: py, w: cwd - px * 2, flow });
                    cx += cwd;
                }
                ch = hmax + py * 2;
                // the cells of a right-to-left row from the right: mirror inside the table
                if rtl != drtl {
                    let tw = cx - x;
                    for b in boxes.iter_mut() {
                        b.x = x + tw - (b.x - x) - b.w;
                    }
                }
            }
            _ => {
                let segs = self.md_segs(lc, doc, base, k, 0, bl.text.len(), 100, false, w, fc);
                let flow = self.md_flow(&bl.text, 0, bl.text.len(), &segs, w, falign, rtl, empty, true);
                ch = flow.h;
                boxes.push(FBox { x, y: 0, w, flow });
            }
        }
        BLay { h: gap + ch, gap, ch, x, w, boxes, bars }
    }

    // a block's layout from the cache (laid out now if needed; its height goes to the rows)
    fn md_lay(&mut self, st: &mut MdViewState, k: usize, cw: i32, drtl: bool, fc: &Faces) -> Rc<BLay> {
        if let Some(l) = st.lc.lays.get(&k) {
            return l.clone();
        }
        let l = Rc::new(self.md_block(&mut st.lc, &st.doc, &st.base, k, cw, drtl, fc));
        st.rows.set(k, l.h);
        st.lc.lays.insert(k, l.clone());
        l
    }

    // the heights of the blocks not laid out yet (cheap guesses for the scroll bar)
    fn md_guess(&mut self, st: &mut MdViewState, cw: i32, fc: &Faces) {
        let px = (fc.size >> 8).max(1);
        let lh = px * 14 / 10;
        let cpl = ((cw / ((px / 2).max(1))).max(8)) as usize;
        let (gp, gt) = (self.dp_text(PARA_GAP), self.dp_text(TIGHT_GAP));
        let (cpy, cell) = (self.dp_text(CODE_PY) * 2, self.dp_text(CELL_PY) * 2);
        let rule = self.dp_text(RULE_H);
        let n = st.doc.blocks().len();
        st.rows.resize(n);
        for k in 0..n {
            let b = &st.doc.blocks()[k];
            let gap = match b.space {
                0 => 0,
                1 => gt,
                _ => gp,
            };
            let h = match b.kind {
                MB_CODE => (b.text.bytes().filter(|&c| c == b'\n').count() as i32 + 1) * lh + cpy,
                MB_RULE => rule,
                MB_ROW => lh + cell,
                MB_HEADING => {
                    let s = HEAD_SCALE[b.level.clamp(1, 6) as usize - 1] as i32;
                    lh * s / 100 * (b.text.len() / cpl.max(1) + 1) as i32 + gp
                }
                _ => lh * ((b.text.len() / cpl.max(1)) as i32 + 1),
            };
            st.rows.guess(k, gap + h.max(1));
        }
    }

    // ------------------------------------------------------------------ the widget

    /// ui_doc_view(id, &mut st, MV_*) -> MdEv: the document set in st (see the module header)
    pub fn doc_view(&mut self, id: u32, st: &mut MdViewState, flags: u32) -> MdEv {
        let mut out = MdEv::default();
        let stflags = self.take_flags();
        let r = self.place(self.dp(NAT_W), self.dp(NAT_H));
        let frame = flags & MV_NO_FRAME == 0;
        let pad = self.dp(if frame { PAD } else { PAD_BARE });
        let sbw = self.style.scrollbar_thickness(self);
        let hair = self.hair().max(1);
        let drtl = if flags & MV_RTL != 0 {
            true
        } else if flags & MV_LTR != 0 {
            false
        } else {
            st.auto_rtl.unwrap_or(self.rtl)
        };
        // the column: the view inside its padding, the scroll bar's room at the end
        let (vx, vy, vw, vh) = (r[0] + hair, r[1] + hair, (r[2] - hair * 2).max(1), (r[3] - hair * 2).max(1));
        let cw = (vw - pad * 2 - sbw).max(self.dp(80));
        // (island coordinates: in a mirrored window the scroll bar is at the left)
        let cx = if self.rtl { vx + pad + sbw } else { vx + pad };
        let view_h = vh;
        // the images come from elsewhere now: read again
        if st.lc.mode != st.images {
            st.lc.mode = st.images;
            st.lc.imgs.iter_mut().for_each(|m| *m = Img::Unread);
            st.lc.lays.clear();
            st.lc.cols.clear();
            st.lc.scaled.clear();
            st.lc.svgs.clear();
        }
        // the document's changes (its elements, md_dom.rs): a focused field's widget moves with
        // its field
        st.sync();
        if let Some((old, new)) = st.refocus.take() {
            let fid = md_field_id(id, old);
            if self.wp.ctx.focus == fid {
                let nid = new.map_or(id, |j| md_field_id(id, j));
                self.wp.ctx.focus = nid;
                self.tb_rekey(fid, nid);
            }
        }
        // the layouts belong to a width, dpi, text scale, direction and fonts
        let fc = self.md_faces();
        let key = (self.wp.rt.dpi, cw, self.layout_scale(), drtl, fc);
        let n = st.doc.blocks().len();
        if key != st.lc.key || !st.sized || st.rows.len() != n {
            if key != st.lc.key {
                st.lc.lays.clear();
                st.lc.cols.clear();
                st.lc.scaled.clear();
            }
            st.lc.key = key;
            st.rows.forget_all();
            self.md_guess(st, cw, &fc);
            st.sized = true;
            st.rows.clamp(view_h);
        } else if st.reguess {
            // (blocks a change brought: estimates, the anchor where it is)
            self.md_guess(st, cw, &fc);
            st.rows.clamp(view_h);
        }
        st.reguess = false;
        // behave: one box for the pointer and the focus
        self.wp.ctx.last_rect = r;
        // (a pass on every pointer move: the link under the pointer, MdEv::hover)
        let mut hb = HB_FOCUS | HB_OWNKEYS | HB_MOVES | CUR_IBEAM << HB_CURSOR_SHIFT;
        if stflags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        let ev = self.behave(id, hb);
        let disabled = self.wp.ctx.last_state == SS_DISABLED as u32;
        let focused = !disabled && self.wp.ctx.focus == id;
        out.ev = ev & (EV_HOVER | EV_FOCUS);
        if !focused {
            st.link = None;
        }
        let sel0 = st.sel;
        let mut activate: Option<usize> = None;
        // ---- keys
        if focused && n > 0 {
            activate = self.md_keys(st, id, view_h, cw, drtl, &fc);
        }
        // ---- smooth scrolling (the wheel)
        if st.pending != 0 {
            let p0 = st.pending;
            if self.anim_off {
                st.pending = 0;
            } else {
                let steps = anim_steps(self.now_ms, &mut st.t0);
                st.pending = anim_decay(p0, 0, steps);
            }
            let p = st.rows.pos() + (p0 - st.pending) as i64;
            st.rows.set_pos(p);
            st.follow = false;
            if st.pending != 0 {
                self.animating = true;
            }
        }
        // ---- the visible blocks (laid out; the anchor settles at the end, a range to reveal)
        let mut vis = self.md_walk(st, view_h, cw, drtl, &fc);
        // ---- the pointer
        let (mx, my) = (self.rtl_map(self.wp.ctx.mx, vx, vw), self.wp.ctx.my);
        let c = &self.wp.ctx;
        let press = c.pressed & (1 << BTN_LEFT) != 0 && ev & EV_HOVER != 0 && c.active == id;
        let held = ev & EV_HELD != 0;
        if press && !disabled {
            let p = self.md_hit(st, &vis, mx - cx, my - vy, cw, drtl);
            st.press_link = self.md_link_at(st, &vis, mx, my, cx, vy, cw, drtl);
            st.dragged = false;
            if let Some(p) = p {
                if ev & EV_DOUBLE != 0 && st.press_link.is_none() {
                    let t = &st.doc.blocks()[p.line].text;
                    let (a, b) = word_range(t, p.byte);
                    st.word = (TextPos::new(p.line, a), TextPos::new(p.line, b));
                    st.sel = TextSel::new(st.word.0, st.word.1);
                    st.drag = 2;
                } else if self.mods & MOD_SHIFT != 0 {
                    st.sel = TextSel::new(st.sel.anchor, p);
                    st.drag = 1;
                } else {
                    st.sel = TextSel::at(p);
                    st.drag = 1;
                }
            }
        } else if held && st.drag != 0 {
            if ev & EV_DRAG != 0 {
                st.dragged = true;
            }
            // beyond the edges: scroll
            let edge = if my < vy {
                my - vy
            } else if my >= vy + view_h {
                my - (vy + view_h) + 1
            } else {
                0
            };
            if edge != 0 && st.dragged {
                let d = edge.signum() * (edge.abs() / 2).clamp(self.dp(4), view_h / 4);
                let p = st.rows.pos() + d as i64;
                st.rows.set_pos(p.max(0));
                st.rows.clamp(view_h);
                st.follow = false;
                self.animating = true;
                vis = self.md_walk(st, view_h, cw, drtl, &fc);
            }
            let yy = my.clamp(vy, vy + view_h - 1);
            if let Some(p) = self.md_hit(st, &vis, mx - cx, yy - vy, cw, drtl) {
                if st.drag == 2 {
                    let t = &st.doc.blocks()[p.line].text;
                    let (a, b) = word_range(t, p.byte);
                    let (wa, wb) = (TextPos::new(p.line, a), TextPos::new(p.line, b));
                    st.sel = if wa < st.word.0 { TextSel::new(st.word.1, wa) } else { TextSel::new(st.word.0, wb.max(st.word.1)) };
                } else {
                    st.sel = TextSel::new(st.sel.anchor, p);
                }
            }
        } else if !held {
            if ev & EV_ACT != 0 && !st.dragged {
                if let Some(k) = st.press_link {
                    if self.md_link_at(st, &vis, mx, my, cx, vy, cw, drtl) == Some(k) {
                        activate = Some(k);
                        st.sel = TextSel::at(st.sel.caret);
                    }
                }
            }
            st.drag = 0;
            st.press_link = None;
        }
        if let Some(k) = activate {
            if let Some(sp) = st.doc.link_spots().get(k).copied() {
                let url = String::from(st.doc.link(sp.link));
                if flags & MV_OPEN_LINKS != 0 {
                    self.open_url(&url);
                }
                out.link = Some(url);
                out.ev |= EV_ACT;
                self.repass();
            }
        }
        if st.sel != sel0 {
            self.repass();
        }
        // ---- draw
        let st_ = self.style;
        if frame {
            let mut sp = Sp { flags: BOX_CARD, ..Sp::default() };
            sp.place(r);
            st_.box_deco(self, &sp);
        }
        self.clip_push(vx, vy, vw, vh);
        self.ltr_begin(vx, vw);
        // the blocks move with the position (scrollcopy.rs)
        let copy = st.rows.total() > view_h as i64;
        if copy {
            self.scroll_copy_begin(id, vx as i64, vy as i64 - st.rows.pos());
        }
        let bgc = self.md_bg(frame);
        let mut links: Vec<Link> = Vec::new();
        let mut fields: Vec<(usize, [i32; 4])> = Vec::new();
        let mut hover: Option<usize> = None;
        for &(k, y) in &vis {
            let lay = st.lc.lays.get(&k).cloned();
            if let Some(lay) = lay {
                self.md_draw_block(&st.doc, &mut st.lc, st.sel, k, &lay, cx, vy + y, cw, drtl, focused, bgc, (vy, vy + view_h), &mut links, &mut fields);
            }
        }
        // the focused link: a frame around its pieces
        if let (Some(f), true) = (st.link, focused && self.wp.ctx.focus_visible) {
            for l in &links {
                if l.spot == f {
                    let st_ = self.style;
                    let lr = l.r;
                    st_.focus_ring(self, [lr[0] - self.dp(1), lr[1], lr[2] + self.dp(2), lr[3]], 2);
                }
            }
        }
        if copy {
            self.scroll_copy_end();
        }
        self.ltr_end();
        // links: a hand over them, a record each
        for l in &links {
            let lr = l.logical;
            self.wp.ctx.last_rect = lr;
            let before = self.wp.ctx.hits[self.wp.ctx.hit_cur].len();
            self.hit(id, hb & ((1 << HB_CURSOR_SHIFT) - 1) | CUR_HAND << HB_CURSOR_SHIFT);
            if let Some(h) = self.wp.ctx.hits[self.wp.ctx.hit_cur].get_mut(before) {
                h.continuation = true;
            }
            let (pmx, pmy) = (self.wp.ctx.mx, self.wp.ctx.my);
            if ev & EV_HOVER != 0 && pmx >= lr[0] && pmx < lr[0] + lr[2] && pmy >= lr[1] && pmy < lr[1] + lr[3] {
                hover = Some(l.spot);
            }
            self.wp.ctx.last_state = if st.link == Some(l.spot) && focused { SS_FOCUSED as u32 } else { 0 };
            self.sem(K_LINK, 0, Some(&l.label), 0);
        }
        // the fields in view: widgets over the text
        self.md_fields(st, id, &fields, disabled, view_h, cw, drtl, &fc, &mut out);
        self.clip_pop();
        if let Some(k) = hover {
            if let Some(sp) = st.doc.link_spots().get(k) {
                out.hover = Some(String::from(st.doc.link(sp.link)));
            }
        }
        // ---- the scroll bar
        let total = st.rows.total().min(i32::MAX as i64) as i32;
        if total > view_h {
            // a dragged thumb moves the view when the pointer moves, not when blocks laid out in
            // this pass change the total (that would chase the thumb pass after pass): while
            // the pointer stays, the bar compares with the position it reported last
            let sid = part_id(id, 1);
            let c = &self.wp.ctx;
            let (py, press) = (c.my, c.pressed & (1 << BTN_LEFT) != 0);
            let held = c.active == sid && c.mbtn & (1 << BTN_LEFT) != 0;
            let mut pos = if held && !press && py == st.sb_y { st.sb_pos } else { st.rows.pos().clamp(0, i32::MAX as i64) as i32 };
            self.next_rect_px(vx + vw - sbw, vy, sbw, vh);
            let sev = self.scrollbar(sid, &mut pos, total, view_h, true);
            if sev & EV_ACT != 0 && (press || py != st.sb_y) {
                st.rows.set_pos(pos as i64);
                st.rows.clamp(view_h);
                st.pending = 0;
                st.reveal = None;
                st.follow = false;
                self.repass();
            }
            st.sb_y = py;
            st.sb_pos = pos;
        }
        // ---- the wheel (over the text or one of its fields)
        let wy = self.wp.ctx.wheel_y;
        let over = ev & EV_HOVER != 0 || field_of(id, self.wp.ctx.hot, st.doc.fields().len()).is_some();
        if over && wy != 0 {
            let d = wy.wrapping_neg().wrapping_mul(self.dp(WHEEL_DIP)) / 120;
            let max = (st.rows.total() - view_h as i64).max(0);
            let p = st.rows.pos();
            if (d < 0 && p > 0) || (d > 0 && p < max) {
                if st.pending == 0 {
                    st.t0 = self.now_ms as i32;
                }
                st.pending += d;
                st.reveal = None;
                self.wp.ctx.wheel_y = 0;
                if self.anim_off {
                    self.repass();
                } else {
                    self.animating = true;
                }
            }
        }
        // ---- the focus visual and the record
        self.wp.ctx.last_rect = r;
        if self.focus_ring(ev) && st.link.is_none() {
            self.focus_ring_draw(ev);
        }
        let mut s = 0;
        if ev & EV_HOVER != 0 {
            s |= SS_HOVER as u32;
        }
        if focused {
            s |= SS_FOCUSED as u32;
        }
        if disabled {
            s |= SS_DISABLED as u32;
        }
        self.wp.ctx.last_state = s;
        let label = if st.label.is_empty() { String::from(tr("Dokument")) } else { st.label.clone() };
        let pos = self.px2dip(st.rows.pos().min(i32::MAX as i64) as i32);
        self.sem(K_BOX, id, Some(&label), pos);
        // layouts far from the view go
        if st.lc.lays.len() > LAYS_KEEP {
            let top = st.rows.top;
            st.lc.lays.retain(|&k, _| k + LAYS_KEEP / 2 >= top && k <= top + LAYS_KEEP / 2);
        }
        self.wp.ctx.last_rect = r;
        out
    }

    // the view's background colour (opaque)
    fn md_bg(&self, frame: bool) -> u32 {
        let bg = self.color(T_BG) | 0xFF00_0000;
        if !frame {
            return bg;
        }
        let c = self.color(T_CARD);
        let a = (c >> 24) as i32;
        color_mix(bg, c | 0xFF00_0000, a + (a >> 7))
    }

    // the blocks in view, laid out: (block, top px from the view's top); the anchor is clamped
    // to the end and a range to reveal is brought into view (until the layouts settle)
    fn md_walk(&mut self, st: &mut MdViewState, view_h: i32, cw: i32, drtl: bool, fc: &Faces) -> Vec<(usize, i32)> {
        let n = st.doc.blocks().len();
        let mut vis = Vec::new();
        if n == 0 {
            return vis;
        }
        if st.reveal.is_some() {
            st.follow = false;
        }
        for _ in 0..6 {
            if st.follow {
                st.rows.set_pos(i64::MAX / 4);
            }
            if let Some((b, y0, y1)) = st.reveal {
                let b = b.min(n - 1);
                // (laid out: its height is known)
                self.md_lay(st, b, cw, drtl, fc);
                let m = self.dp(8);
                let top = st.rows.prefix(b) - st.rows.pos();
                if top + (y0 as i64) < 0 || (y1 - y0) >= view_h {
                    st.rows.set_pos(st.rows.prefix(b) + (y0 - m).max(0) as i64);
                } else if top + y1 as i64 > view_h as i64 {
                    st.rows.set_pos(st.rows.prefix(b) + (y1 + m - view_h) as i64);
                }
            }
            st.rows.clamp(view_h);
            vis.clear();
            let (t0, o0) = (st.rows.top, st.rows.off);
            let mut k = st.rows.top;
            let mut y = -st.rows.off;
            while k < n && y < view_h {
                let lay = self.md_lay(st, k, cw, drtl, fc);
                vis.push((k, y));
                y += lay.h;
                k += 1;
            }
            st.rows.clamp(view_h);
            let shown = match st.reveal {
                Some((b, y0, y1)) => vis.iter().find(|v| v.0 == b).is_some_and(|v| v.1 + y0 >= 0 && v.1 + y1 <= view_h),
                None => true,
            };
            if shown {
                st.reveal = None;
            }
            if (st.rows.top, st.rows.off) == (t0, o0) && shown {
                break;
            }
        }
        st.reveal = None;
        // at the end of a document longer than the view: stay there while it grows
        let total = st.rows.total();
        if total > view_h as i64 {
            st.follow = st.rows.pos() + view_h as i64 >= total - 1;
        }
        vis
    }

    // the text position at (x, y): x from the column's start, y from the view's top
    fn md_hit(&self, st: &MdViewState, vis: &[(usize, i32)], x: i32, y: i32, cw: i32, drtl: bool) -> Option<TextPos> {
        let &(k, top) = vis.iter().rev().find(|v| v.1 <= y).or(vis.first())?;
        let lay = st.lc.lays.get(&k)?;
        let ly = y - top - lay.gap;
        if lay.boxes.is_empty() {
            return Some(TextPos::new(k, 0));
        }
        // the text area under x (else the nearest)
        let bx = |b: &FBox| box_x(0, cw, b.x, b.w, drtl);
        let fb = lay
            .boxes
            .iter()
            .min_by_key(|b| {
                let l = bx(b);
                if x < l { l - x } else if x >= l + b.w { x - (l + b.w) + 1 } else { 0 }
            })?;
        let fy = ly - fb.y;
        let row = fb.flow.rows.iter().rposition(|r| r.y <= fy).unwrap_or(0);
        let r = fb.flow.rows.get(row)?;
        if fy >= fb.flow.h && k + 1 < st.doc.blocks().len() && ly >= lay.ch {
            return Some(TextPos::new(k, r.b as usize));
        }
        Some(TextPos::new(k, r.hit(x - bx(fb)) as usize))
    }

    // the link spot under the pointer (window px)
    #[allow(clippy::too_many_arguments)]
    fn md_link_at(&self, st: &MdViewState, vis: &[(usize, i32)], mx: i32, my: i32, cx: i32, vy: i32, cw: i32, drtl: bool) -> Option<usize> {
        let y = my - vy;
        let &(k, top) = vis.iter().rev().find(|v| v.1 <= y)?;
        let lay = st.lc.lays.get(&k)?;
        let ly = y - top - lay.gap;
        for fb in &lay.boxes {
            let x0 = box_x(cx, cw, fb.x, fb.w, drtl);
            for r in &fb.flow.rows {
                let ry = fb.y + r.y;
                if ly < ry || ly >= ry + r.h {
                    continue;
                }
                for p in &r.pieces {
                    if p.link != 0 && mx >= x0 + p.x && mx < x0 + p.x + p.w {
                        return spot_of(st.doc.link_spots(), k, |s| s.link == p.link && s.a <= p.a && p.b <= s.b);
                    }
                }
            }
        }
        None
    }

    // the keys of a focused view -> a link to activate
    #[allow(clippy::too_many_arguments)]
    fn md_keys(&mut self, st: &mut MdViewState, id: u32, view_h: i32, cw: i32, drtl: bool, fc: &Faces) -> Option<usize> {
        let find = |ui: &Ui, vk: u32, need: u32, mask: u32| -> Option<usize> {
            ui.wp.ctx.keyq.iter().position(|&k| (k as i32) >= 0 && k & 0xFFFF == vk && (k >> 16) & mask == need)
        };
        let take = |ui: &mut Ui, i: usize| {
            if let Some(k) = ui.wp.ctx.keyq.get_mut(i) {
                *k |= 0x8000_0000;
            }
        };
        const M: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT;
        let line = self.dp(LINE_DIP);
        let page = (view_h - line).max(line);
        let mut d: i64 = 0;
        let mut act = None;
        if let Some(i) = find(self, VK_UP, 0, M) {
            take(self, i);
            d -= line as i64;
        }
        if let Some(i) = find(self, VK_DOWN, 0, M) {
            take(self, i);
            d += line as i64;
        }
        if let Some(i) = find(self, VK_PRIOR, 0, M & !MOD_SHIFT) {
            take(self, i);
            d -= page as i64;
        }
        if let Some(i) = find(self, VK_NEXT, 0, M & !MOD_SHIFT) {
            take(self, i);
            d += page as i64;
        }
        if let Some(i) = find(self, VK_SPACE, 0, MOD_CTRL | MOD_ALT) {
            let shift = (self.wp.ctx.keyq[i] >> 16) & MOD_SHIFT != 0;
            take(self, i);
            d += if shift { -(page as i64) } else { page as i64 };
        }
        let mut home = false;
        let mut end = false;
        if let Some(i) = find(self, VK_HOME, 0, MOD_SHIFT | MOD_ALT) {
            take(self, i);
            home = true;
        }
        if let Some(i) = find(self, VK_END, 0, MOD_SHIFT | MOD_ALT) {
            take(self, i);
            end = true;
        }
        if find(self, b'A' as u32, MOD_CTRL, M).map(|i| take(self, i)).is_some() {
            st.select_all();
        }
        if find(self, b'C' as u32, MOD_CTRL, M).or_else(|| find(self, VK_INSERT, MOD_CTRL, M)).map(|i| take(self, i)).is_some() && !st.sel.is_empty() {
            let (t, h) = (st.selected_text(), st.selected_html());
            self.clip_set_many(&[(CLIP_TEXT, t.as_bytes()), (CLIP_HTML, h.as_bytes())]);
        }
        // Tab / Shift+Tab: the next / previous link or field (past the ends the focus moves on)
        if let Some(i) = find(self, VK_TAB, 0, MOD_CTRL | MOD_ALT) {
            let back = (self.wp.ctx.keyq[i] >> 16) & MOD_SHIFT != 0;
            let cur = st.link.and_then(|k| st.stops.iter().position(|&s| s == Stop::Link(k)));
            match st.step(cur, back) {
                Some(Stop::Link(k)) => {
                    take(self, i);
                    st.link = Some(k);
                    self.wp.ctx.focus_visible = true;
                    self.md_reveal_link(st, k, cw, drtl, fc);
                    self.repass();
                }
                Some(Stop::Field(f)) => {
                    take(self, i);
                    st.link = None;
                    self.md_focus_field(st, id, f, cw, drtl, fc);
                }
                None if st.link.is_some() && back => {
                    take(self, i);
                    st.link = None;
                    self.repass();
                }
                None => st.link = None,
            }
        }
        if let Some(k) = st.link {
            if let Some(i) = find(self, VK_RETURN, 0, M) {
                take(self, i);
                act = Some(k);
            }
            if let Some(i) = find(self, VK_ESCAPE, 0, M) {
                take(self, i);
                st.link = None;
                self.repass();
            }
        }
        if home {
            st.rows.set_pos(0);
            st.pending = 0;
            st.follow = false;
        }
        if end {
            st.pending = 0;
            st.follow = true;
        }
        if d != 0 {
            st.rows.set_pos((st.rows.pos() + d).max(0));
            st.pending = 0;
            st.follow = false;
        }
        if home || end || d != 0 {
            st.reveal = None;
            self.wp.ctx.focus_visible = true;
            self.repass();
        }
        act
    }

    // give field f the keyboard focus (its ring shown) and scroll it into view
    fn md_focus_field(&mut self, st: &mut MdViewState, id: u32, f: usize, cw: i32, drtl: bool, fc: &Faces) {
        self.wp.ctx.focus = md_field_id(id, f);
        self.wp.ctx.focus_visible = true;
        st.ffocus = Some(f);
        self.repass();
        let Some(sp) = st.doc.field_spots().iter().find(|s| s.field as usize == f + 1).copied() else { return };
        let lay = self.md_lay(st, sp.block, cw, drtl, fc);
        let (mut y0, mut y1) = (i32::MAX, 0);
        for fb in &lay.boxes {
            for r in &fb.flow.rows {
                if r.pieces.iter().any(|p| p.field == sp.field) {
                    y0 = y0.min(lay.gap + fb.y + r.y);
                    y1 = y1.max(lay.gap + fb.y + r.y + r.h);
                }
            }
        }
        if y0 <= y1 {
            st.reveal = Some((sp.block, y0, y1));
        }
    }

    // an unconsumed key vk of this pass with none of the modifiers in `mask` -> its place in
    // the queue
    fn md_key_at(&self, vk: u32, mask: u32) -> Option<usize> {
        self.wp.ctx.keyq.iter().position(|&k| (k as i32) >= 0 && k & 0xFFFF == vk && (k >> 16) & mask == 0)
    }

    // consume the key at place i of the queue
    fn md_key_take(&mut self, i: usize) {
        if let Some(k) = self.wp.ctx.keyq.get_mut(i) {
            *k |= 0x8000_0000;
        }
    }

    // a check box, radio or button of a form at r (logical px): behave, the style's picture (a
    // box alone: its text is the document's), the focus ring, the record (label: the field's
    // name, a button's caption)
    fn md_ctl(&mut self, fid: u32, kind: u8, r: [i32; 4], label: &str, on: bool, disabled: bool) -> u32 {
        self.wp.ctx.last_rect = r;
        let button = !matches!(kind, MF_CHECKBOX | MF_RADIO);
        let mut hb = HB_FOCUS | if button { 0 } else { HB_SPACE };
        if disabled {
            hb |= HB_DISABLED;
        }
        let ev = self.behave(fid, hb);
        let st = self.style;
        let mut sp = Sp { flags: if disabled { ST_DISABLED } else { 0 }, label: button.then_some(label), ..Sp::default() };
        match kind {
            MF_CHECKBOX => st.checkbox_size(self, &mut sp),
            MF_RADIO => st.radio_size(self, &mut sp),
            _ => st.button_size(self, &mut sp),
        };
        self.sp_placed(&mut sp, ev);
        sp.value = on as i32;
        match kind {
            MF_CHECKBOX => st.checkbox(self, &sp),
            MF_RADIO => st.radio(self, &sp),
            _ => st.button(self, &sp),
        }
        self.focus_ring_draw(ev);
        if on {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        let k = match kind {
            MF_CHECKBOX => K_CHECKBOX,
            MF_RADIO => K_RADIO,
            _ => K_BUTTON,
        };
        self.sem(k, fid, Some(label), on as i32);
        ev
    }

    // the fields in view: the library's widgets at the rectangles of the layout (logical px;
    // off: the view is disabled, so are they), their values in the state; Enter sends the
    // form, the arrows choose a radio, Tab leaves a field for the next / previous link or field;
    // what the user did goes to `out`
    #[allow(clippy::too_many_arguments)]
    fn md_fields(&mut self, st: &mut MdViewState, id: u32, drawn: &[(usize, [i32; 4])], off: bool, view_h: i32, cw: i32, drtl: bool, fc: &Faces, out: &mut MdEv) {
        let n = st.doc.fields().len();
        if n == 0 {
            return;
        }
        const M: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT;
        let mut changed: Option<usize> = None;
        let mut submit: Option<MdSubmit> = None;
        let mut reset: Option<usize> = None;
        let mut moved: Option<usize> = None;
        for &(f, r) in drawn {
            let fid = md_field_id(id, f);
            let focused = self.wp.ctx.focus == fid;
            let h0 = self.wp.ctx.hits[self.wp.ctx.hit_cur].len();
            let fd = &st.doc.fields()[f];
            let (kind, form, disabled, maxlength) = (fd.kind, fd.form as usize, fd.disabled || off, fd.maxlength as usize);
            match kind {
                MF_TEXT | MF_PASSWORD | MF_TEXTAREA | MF_SELECT => {
                    // the name: a record without area along the top edge (text boxes and lists
                    // take the text record right above them as their name)
                    let name = field_name(fd);
                    if !name.is_empty() {
                        self.wp.ctx.last_rect = [r[0], r[1], r[2], 0];
                        self.wp.ctx.last_state = 0;
                        self.sem(K_TEXT, 0, Some(name), 0);
                    }
                    let enter = if focused && kind != MF_TEXTAREA && kind != MF_SELECT { self.md_key_at(VK_RETURN, 0) } else { None };
                    let esc = focused && kind != MF_SELECT && self.md_key_at(VK_ESCAPE, M).is_some();
                    if disabled {
                        self.next_flags(ST_DISABLED);
                    }
                    self.next_rect_px(r[0], r[1], r[2], r[3]);
                    let ev = if kind == MF_SELECT {
                        let opts: Vec<&str> = fd.options.iter().map(|o| if o.label.is_empty() { " " } else { o.label.as_str() }).collect();
                        self.combobox(fid, &mut st.vals[f].sel, &opts)
                    } else {
                        let mut flags = match kind {
                            MF_PASSWORD => TB_PASSWORD,
                            MF_TEXTAREA => TB_MULTI | TB_WRAP,
                            _ => 0,
                        };
                        if fd.readonly {
                            flags |= TB_READONLY;
                        }
                        let cap = if maxlength > 0 { (maxlength as u32 * 4 + 1).min(FIELD_CAP) } else { FIELD_CAP };
                        let ph = (!fd.placeholder.is_empty()).then_some(fd.placeholder.as_str());
                        self.textbox(fid, ph, &mut st.vals[f].text, cap | flags)
                    };
                    if ev & EV_ACT != 0 {
                        st.vals[f].dirty = true;
                        // (maxlength counts characters)
                        let t = &mut st.vals[f].text;
                        if maxlength > 0 && t.chars().count() > maxlength {
                            let cut = t.char_indices().nth(maxlength).map_or(t.len(), |c| c.0);
                            t.truncate(cut);
                        }
                        changed = Some(f);
                    }
                    if enter.is_some() && ev & EV_COMMIT != 0 {
                        submit = st.implicit(f);
                    }
                    if esc && self.wp.ctx.focus == 0 {
                        // Esc in a text field: back to the view
                        self.wp.ctx.focus = id;
                        self.wp.ctx.focus_visible = true;
                    }
                }
                _ => {
                    let label = if matches!(kind, MF_CHECKBOX | MF_RADIO) { field_name(fd) } else { fd.caption() };
                    let on = st.vals[f].on;
                    let ev = self.md_ctl(fid, kind, r, label, on, disabled);
                    match kind {
                        MF_CHECKBOX | MF_RADIO => {
                            if focused {
                                if let Some(i) = self.md_key_at(VK_RETURN, M) {
                                    self.md_key_take(i);
                                    submit = st.implicit(f);
                                }
                            }
                            if focused && kind == MF_RADIO {
                                // Up / Down: the previous / next radio; Left / Right as they lie
                                // in the paragraph (its direction; the window's mirroring undone:
                                // the text keeps its order)
                                let rtl = st.doc.field_spots().iter().find(|s| s.field as usize == f + 1).and_then(|s| st.doc.blocks().get(s.block)).and_then(|b| text_rtl(&b.text)).unwrap_or(drtl);
                                for vk in [VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN] {
                                    if let Some(i) = self.md_key_at(vk, M) {
                                        let k = self.rtl_text_key(self.wp.ctx.keyq[i]) & 0xFFFF;
                                        self.md_key_take(i);
                                        let back = k == VK_UP || (k != VK_DOWN && ((k == VK_LEFT) != rtl));
                                        moved = st.radio_step(f, back);
                                        break;
                                    }
                                }
                            }
                            if ev & EV_ACT != 0 && (kind == MF_CHECKBOX || !on) {
                                st.set_field_checked(f, !on);
                                changed = Some(f);
                            }
                        }
                        MF_SUBMIT if ev & EV_ACT != 0 => match form.checked_sub(1) {
                            Some(fm) => submit = Some(st.submit(fm, Some(f))),
                            None => changed = Some(f),
                        },
                        MF_RESET if ev & EV_ACT != 0 => {
                            reset = form.checked_sub(1);
                            changed = Some(f);
                        }
                        _ if ev & EV_ACT != 0 => changed = Some(f),
                        _ => {}
                    }
                }
            }
            // (no Tab stop of the window: the view steps through its links and fields)
            let c = &mut self.wp.ctx;
            for h in c.hits[c.hit_cur][h0..].iter_mut() {
                if h.id == fid {
                    h.continuation = true;
                }
            }
        }
        if let Some(fm) = reset {
            st.reset_form(fm);
            self.repass();
        }
        if let Some(g) = moved {
            st.set_field_checked(g, true);
            self.md_focus_field(st, id, g, cw, drtl, fc);
            changed = Some(g);
        }
        // Tab / Shift+Tab from a field: the next / previous link or field; past the last one the
        // focus moves on from the view, before the first one it stays on the view
        if let Some(f) = field_of(id, self.wp.ctx.focus, n) {
            if let Some(i) = self.md_key_at(VK_TAB, MOD_CTRL | MOD_ALT) {
                let back = (self.wp.ctx.keyq[i] >> 16) & MOD_SHIFT != 0;
                let cur = st.stops.iter().position(|&s| s == Stop::Field(f));
                match st.step(cur, back) {
                    Some(Stop::Field(g)) => {
                        self.md_key_take(i);
                        self.md_focus_field(st, id, g, cw, drtl, fc);
                    }
                    s => {
                        st.link = None;
                        if let Some(Stop::Link(k)) = s {
                            st.link = Some(k);
                            self.md_reveal_link(st, k, cw, drtl, fc);
                        }
                        if s.is_some() || back {
                            self.md_key_take(i);
                        }
                        self.wp.ctx.focus = id;
                        self.wp.ctx.focus_visible = true;
                        self.repass();
                    }
                }
            }
        }
        // PgUp / PgDn on a field the field does not take: the view scrolls by a page (the field
        // keeps the focus)
        if field_of(id, self.wp.ctx.focus, n).is_some() {
            for (vk, dir) in [(VK_PRIOR, -1i64), (VK_NEXT, 1)] {
                if let Some(i) = self.md_key_at(vk, MOD_CTRL | MOD_ALT) {
                    self.md_key_take(i);
                    let line = self.dp(LINE_DIP);
                    st.rows.set_pos((st.rows.pos() + dir * (view_h - line).max(line) as i64).max(0));
                    st.pending = 0;
                    st.reveal = None;
                    st.follow = false;
                    self.repass();
                }
            }
        }
        st.ffocus = field_of(id, self.wp.ctx.focus, n);
        if changed.is_some() || submit.is_some() {
            self.repass();
        }
        out.field = changed;
        out.submit = submit;
    }

    // scroll link spot k into view
    fn md_reveal_link(&mut self, st: &mut MdViewState, k: usize, cw: i32, drtl: bool, fc: &Faces) {
        let Some(sp) = st.doc.link_spots().get(k).copied() else { return };
        let lay = self.md_lay(st, sp.block, cw, drtl, fc);
        let (mut y0, mut y1) = (i32::MAX, 0);
        for fb in &lay.boxes {
            for r in &fb.flow.rows {
                if r.pieces.iter().any(|p| p.link == sp.link && p.a < sp.b && p.b > sp.a) {
                    y0 = y0.min(lay.gap + fb.y + r.y);
                    y1 = y1.max(lay.gap + fb.y + r.y + r.h);
                }
            }
        }
        if y0 <= y1 {
            st.reveal = Some((sp.block, y0, y1));
        }
    }

    // ------------------------------------------------------------------ drawing

    #[allow(clippy::too_many_arguments)]
    // (rows outside vis: y0 .. y1 are skipped -- a block may be far taller than the view; the
    // fields go to `fields` with their logical rectangles: md_fields builds their widgets)
    fn md_draw_block(&mut self, doc: &MdDoc, lc: &mut Cache, sel: TextSel, k: usize, lay: &BLay, cx: i32, top: i32, cw: i32, drtl: bool, focused: bool, bgc: u32, vis: (i32, i32), links: &mut Vec<Link>, fields: &mut Vec<(usize, [i32; 4])>) {
        let hc = self.high_contrast;
        let bl = &doc.blocks()[k];
        let ctop = top + lay.gap;
        let tcol = self.color(T_TEXT) | 0xFF00_0000;
        let t2 = self.color(T_TEXT2) | 0xFF00_0000;
        let div = if hc { tcol } else { color_mix(bgc, tcol, 48) };
        let tint = color_mix(bgc, tcol, if self.theme_dark { 22 } else { 14 });
        let hair = self.hair().max(1);
        // quote bars (through the gap when the block before is quoted as deep)
        let prev_q = if k > 0 { doc.blocks()[k - 1].quote } else { 0 };
        let mut qn = 0;
        for &bx in &lay.bars {
            qn += 1;
            let bw = self.dp(3);
            let y0 = if prev_q >= qn && k > 0 { top } else { ctop };
            let x = box_x(cx, cw, bx, bw, drtl);
            self.draw_color(if hc { tcol } else { color_mix(bgc, tcol, 70) });
            self.draw_rect(x, y0, bw, ctop + lay.ch - y0);
        }
        let colx = box_x(cx, cw, lay.x, lay.w, drtl);
        match bl.kind {
            MB_CODE => {
                if hc {
                    self.draw_color(tcol);
                    self.draw_frame(colx, ctop, lay.w, lay.ch, self.dp(4), hair);
                } else {
                    let th = self.code_theme();
                    let fill = if th.bg != 0 { color_mix(bgc, tcol, if self.theme_dark { 18 } else { 10 }) } else { tint };
                    self.cmd_rrect(colx, ctop, lay.w, lay.ch, self.dp(6), 0, fill, 0, 0);
                }
            }
            MB_RULE => {
                let t = self.dp(2).max(1);
                self.draw_color(div);
                self.draw_rect(colx, ctop + lay.ch / 2 - t / 2, lay.w, t);
            }
            MB_ROW => {
                // cell frames, the header's fill
                let px = self.dp_text(CELL_PX);
                for fb in &lay.boxes {
                    let x = box_x(cx, cw, fb.x - px, fb.w + px * 2, drtl);
                    if bl.head && !hc {
                        self.draw_color(tint);
                        self.draw_rect(x, ctop, fb.w + px * 2, lay.ch);
                    }
                    self.draw_color(div);
                    self.draw_frame(x, ctop, fb.w + px * 2 + hair, lay.ch + hair, 0, hair);
                }
            }
            MB_HEADING if bl.level <= 2 => {
                let fh = lay.boxes.first().map_or(0, |b| b.flow.h);
                self.draw_color(div);
                self.draw_rect(colx, ctop + fh + self.dp(6), lay.w, hair);
            }
            _ => {}
        }
        // the selection under the text
        let (sa, sb) = sel.range();
        let selc = if focused { (self.color(T_ACCENT) & 0x00FF_FFFF) | 0x60 << 24 } else { (self.color(T_TEXT3) & 0x00FF_FFFF) | 0x38 << 24 };
        if sa < sb && sa.line <= k && sb.line >= k {
            let lo = if sa.line == k { sa.byte as u32 } else { 0 };
            let hi = if sb.line == k { sb.byte as u32 } else { u32::MAX };
            if bl.kind == MB_RULE {
                self.draw_color(selc);
                self.draw_rect(colx, ctop, lay.w, lay.ch);
            }
            for fb in &lay.boxes {
                let x0 = box_x(cx, cw, fb.x, fb.w, drtl);
                for r in &fb.flow.rows {
                    let ry = ctop + fb.y + r.y;
                    if ry + r.h <= vis.0 || ry >= vis.1 {
                        continue;
                    }
                    for p in &r.pieces {
                        let (a, b) = (p.a.max(lo), p.b.min(hi));
                        if a >= b {
                            continue;
                        }
                        let (mut xa, mut xb) = (i32::MAX, i32::MIN);
                        for s in &r.stops {
                            if s.0 >= a && s.0 <= b {
                                xa = xa.min(s.1);
                                xb = xb.max(s.1);
                            }
                        }
                        if xa < xb {
                            self.draw_color(selc);
                            self.draw_rect(x0 + xa, ctop + fb.y + r.y, xb - xa, r.h);
                        }
                    }
                }
            }
        }
        // the text
        let quote = bl.quote > 0;
        let theme = if bl.kind == MB_CODE { Some(self.code_theme()) } else { None };
        let lcol = self.color(T_ACCENT_TEXT) | 0xFF00_0000;
        // the first row: the list marker's baseline and font
        let first_base: Option<(i32, i32, u32, i32, i32)> = lay.boxes.first().and_then(|fb| {
            let r = fb.flow.rows.first()?;
            let (f, s, a) = r.pieces.iter().find(|p| p.image == 0 && p.field == 0).map_or((0, 0, 0), |p| (p.font, p.size, p.asc));
            Some((ctop + fb.y + r.y + r.base, box_x(cx, cw, fb.x, fb.w, drtl), f, s, a))
        });
        for fb in &lay.boxes {
            let x0 = box_x(cx, cw, fb.x, fb.w, drtl);
            for r in &fb.flow.rows {
                let ry = ctop + fb.y + r.y;
                if ry + r.h <= vis.0 || ry >= vis.1 {
                    continue;
                }
                let base = ry + r.base;
                let mut prev_link: Option<([i32; 4], u32)> = None;
                for p in &r.pieces {
                    let x = x0 + p.x;
                    if p.image != 0 {
                        self.md_draw_image(lc, doc, p.image as usize - 1, x, base - p.ih, p.w, p.ih, tcol, bgc);
                        let alt = doc.images().get(p.image as usize - 1).map_or("", |m| m.alt.as_str());
                        self.wp.ctx.last_rect = self.ltr_rect([x, base - p.ih, p.w, p.ih]);
                        self.wp.ctx.last_state = 0;
                        let label = if alt.is_empty() { tr("Bild") } else { alt };
                        self.sem(K_IMAGE, 0, Some(label), 0);
                    } else if p.field != 0 {
                        // a field: its widget after the blocks, here (its row's gap above it)
                        let y = base - p.asc + self.dp(FIELD_GAP);
                        fields.push((p.field as usize - 1, self.ltr_rect([x, y, p.w, p.ih])));
                    } else {
                        let link = p.link != 0;
                        let col = if let Some(t) = &theme {
                            t.tokens.get(p.syn.saturating_sub(1) as usize).copied().unwrap_or(tcol)
                        } else if link {
                            lcol
                        } else if p.color != 0 && !hc {
                            opaque(p.color)
                        } else if quote || (bl.kind == MB_HEADING && bl.level >= 6) {
                            t2
                        } else {
                            tcol
                        };
                        if p.bg != 0 && !hc {
                            self.draw_color(opaque(p.bg));
                            self.draw_rect(x, ry, p.w, r.h);
                        }
                        if p.style & MS_CODE != 0 && bl.kind != MB_CODE {
                            let (h, yy, cp) = (p.asc + self.dp(6), base - p.asc - self.dp(2), self.dp_text(CODE_PAD));
                            if hc {
                                self.draw_color(tcol);
                                self.draw_frame(x - cp, yy, p.w + cp * 2, h, self.dp(3), hair);
                            } else {
                                self.cmd_rrect(x - cp, yy, p.w + cp * 2, h, self.dp(3), 0, tint, 0, 0);
                            }
                        }
                        let s = &bl.text.as_bytes()[p.a as usize..p.b as usize];
                        let s = if s.last() == Some(&b'\n') { &s[..s.len() - 1] } else { s };
                        if p.shaped {
                            let d: DrawRef = Rc::new(Shaped { s: s.to_vec(), font: p.font, size: p.size, color: col, dx: self.dp(2), base: base - ry });
                            let ver = (text_hash(core::str::from_utf8(s).unwrap_or("")) as u32) ^ col ^ (p.size as u32).rotate_left(7) ^ p.font;
                            self.cmd_custom(x - self.dp(2), ry, p.w + self.dp(4), r.h, Some(d), 0, ver);
                        } else {
                            self.cmd_text(x, base, s, p.font, p.size, col, 0);
                        }
                        let px = p.size >> 8;
                        let tl = (px / 14).max(1);
                        if link || p.style & MS_UNDER != 0 {
                            self.draw_color(col);
                            self.draw_rect(x, base + (px / 9).max(1), p.w, tl);
                        }
                        if p.style & MS_STRIKE != 0 {
                            self.draw_color(col);
                            self.draw_rect(x, base - p.asc * 3 / 10, p.w, tl);
                        }
                    }
                    // links: one rectangle per row and link
                    if p.link != 0 {
                        let pr = [x, ry, p.w, r.h];
                        match &mut prev_link {
                            Some((lr, l)) if *l == p.link && (lr[0] + lr[2] == x || x + p.w == lr[0]) => {
                                let x0 = lr[0].min(x);
                                lr[2] = (lr[0] + lr[2]).max(x + p.w) - x0;
                                lr[0] = x0;
                            }
                            _ => {
                                if let Some((lr, l)) = prev_link.take() {
                                    self.md_link_rec(doc, k, l, lr, r, links);
                                }
                                prev_link = Some((pr, p.link));
                            }
                        }
                    } else if let Some((lr, l)) = prev_link.take() {
                        self.md_link_rec(doc, k, l, lr, r, links);
                    }
                }
                if let Some((lr, l)) = prev_link.take() {
                    self.md_link_rec(doc, k, l, lr, r, links);
                }
            }
        }
        // the list marker in front of the first row
        if bl.marker != MK_NONE {
            let (base, _, mf, ms, masc) = first_base.unwrap_or((ctop + self.dp(14), 0, 0, 0, self.dp(12)));
            let (mf, ms, masc) = if mf == 0 {
                let f = self.md_faces();
                let fnt = f.get(false, false, false);
                let (a, _, _) = self.md_metrics(fnt, f.size);
                (fnt, f.size, a)
            } else {
                (mf, ms, masc)
            };
            let gut = self.dp_text(LIST_W);
            let gx = box_x(cx, cw, lay.x - gut, gut, drtl);
            let mcol = if quote { t2 } else { tcol };
            let label = bl.text.chars().take(LABEL_MAX).collect::<String>();
            if bl.task != 0 {
                let s = ((masc as f32 * 0.9) as i32).max(self.dp(10));
                let bx = if drtl { gx + self.dp_text(6) } else { gx + gut - s - self.dp_text(8) };
                let by = base - s + self.dp(1);
                let done = bl.task == 2;
                if done {
                    self.cmd_rrect(bx, by, s, s, self.dp(3), 0, self.color(T_ACCENT) | 0xFF00_0000, 0, 0);
                    let c = self.color(T_ON_ACCENT) | 0xFF00_0000;
                    self.draw_color(c);
                    let (fx, fy) = ((bx << 8) as i64, (by << 8) as i64);
                    let sz = (s << 8) as i64;
                    let w = (self.dp(2).max(1) << 8) * 3 / 4;
                    let p = |a: i64, b: i64| ((fx + sz * a / 100) as i32, (fy + sz * b / 100) as i32);
                    let (a0, a1) = p(24, 52);
                    let (b0, b1) = p(43, 70);
                    let (c0, c1) = p(76, 32);
                    self.draw_line(a0, a1, b0, b1, w);
                    self.draw_line(b0, b1, c0, c1, w);
                } else {
                    self.cmd_rrect(bx, by, s, s, self.dp(3), hair, 0, self.color(T_STRONG_STROKE) | 0xFF00_0000, self.color(T_STRONG_STROKE) | 0xFF00_0000);
                }
                self.wp.ctx.last_rect = self.ltr_rect([bx, by, s, s]);
                self.wp.ctx.last_state = if done { SS_CHECKED as u32 } else { 0 };
                self.sem(K_CHECKBOX, 0, Some(&label), done as i32);
            } else if bl.marker == MK_BULLET {
                let d = ((masc as f32 * 0.36) as i32).max(self.dp(4)) & !1;
                let mx = if drtl { gx + self.dp_text(12) } else { gx + gut - self.dp_text(12) };
                let my = base - (masc as f32 * 0.34) as i32;
                self.draw_color(mcol);
                match (bl.list.max(1) - 1) % 3 {
                    0 => self.draw_circle(mx << 8, my << 8, (d << 8) / 2, 0),
                    1 => self.draw_circle(mx << 8, my << 8, (d << 8) / 2 - 128, self.hair().max(1) << 8),
                    _ => self.draw_rect(mx - d / 2, my - d / 2, d, d),
                }
            } else {
                // (right to left the delimiter reads after the number: it stands on its left)
                let d = if bl.delim == b')' { '(' } else { '.' };
                let m = if drtl { format!("{}{}", d, bl.number) } else { format!("{}{}", bl.number, if bl.delim == b')' { ')' } else { '.' }) };
                self.text.set_font(mf, ms);
                let mw = self.text.width(&m);
                let x = if drtl { gx + self.dp_text(6) } else { gx + gut - self.dp_text(6) - mw };
                self.cmd_text(x, base, m.as_bytes(), mf, ms, mcol, 0);
            }
            // the list item's record
            self.wp.ctx.last_rect = self.ltr_rect([box_x(cx, cw, lay.x - gut, lay.w + gut, drtl), ctop, lay.w + gut, lay.ch]);
            self.wp.ctx.last_state = 0;
            self.sem(K_LISTITEM, 0, Some(&label), bl.number as i32);
        }
        // the block's record
        self.wp.ctx.last_rect = self.ltr_rect([colx, ctop, lay.w, lay.ch]);
        self.wp.ctx.last_state = 0;
        match bl.kind {
            MB_HEADING => {
                let label = doc.plain_range(TextPos::new(k, 0), TextPos::new(k, bl.text.len()));
                self.sem(K_HEADING, 0, Some(&label), bl.level as i32);
            }
            MB_ROW => {
                for (c, fb) in lay.boxes.iter().enumerate() {
                    let Some(&(a, b)) = bl.cells.get(c) else { continue };
                    let t: String = doc.plain_range(TextPos::new(k, a as usize), TextPos::new(k, b as usize));
                    let px = self.dp_text(CELL_PX);
                    self.wp.ctx.last_rect = self.ltr_rect([box_x(cx, cw, fb.x - px, fb.w + px * 2, drtl), ctop, fb.w + px * 2, lay.ch]);
                    self.sem(K_TEXT, 0, Some(&t), 0);
                }
            }
            MB_RULE => {}
            _ if bl.marker == MK_NONE || bl.kind == MB_CODE => {
                let t: String = if bl.kind == MB_CODE {
                    bl.text.chars().take(LABEL_MAX).collect()
                } else {
                    doc.plain_range(TextPos::new(k, 0), TextPos::new(k, bl.text.len())).chars().take(LABEL_MAX).collect()
                };
                if !t.is_empty() {
                    self.sem(K_TEXT, 0, Some(&t), 0);
                }
            }
            _ => {}
        }
    }

    // a link's rectangle in a row: merged with the link's other pieces of the row
    fn md_link_rec(&self, doc: &MdDoc, k: usize, link: u32, lr: [i32; 4], r: &LRow, links: &mut Vec<Link>) {
        let Some(spot) = spot_of(doc.link_spots(), k, |s| s.link == link && r.pieces.iter().any(|p| p.link == link && p.a >= s.a && p.b <= s.b)) else {
            return;
        };
        let sp = doc.link_spots()[spot];
        let t = doc.plain_range(TextPos::new(k, sp.a as usize), TextPos::new(k, sp.b as usize));
        links.push(Link { r: lr, logical: self.ltr_rect(lr), label: t, spot }); // (visible rows only)
    }

    // an image at x, y (w x h px): the picture, else a frame with its alt text
    #[allow(clippy::too_many_arguments)]
    fn md_draw_image(&mut self, lc: &mut Cache, doc: &MdDoc, i: usize, x: i32, y: i32, w: i32, h: i32, tcol: u32, bgc: u32) {
        let img = lc.imgs.get(i).cloned().unwrap_or(Img::Failed);
        match img {
            Img::Bitmap(sf) => {
                let key = (i, w, h);
                let s = if sf.w == w && sf.h == h {
                    sf
                } else if let Some(s) = lc.scaled.get(&key) {
                    s.clone()
                } else {
                    let s = img_core::ops::scale(&sf, w, h, true).map(Rc::new).unwrap_or(sf);
                    lc.scaled.insert(key, s.clone());
                    s
                };
                self.draw_image(&s, i as u32, x, y, w, h);
            }
            Img::Vector(svg) => {
                let color = tcol;
                let fonts = self.svg_fonts();
                let d = lc.svgs.entry((i, color)).or_insert_with(|| Rc::new(SvgDraw { doc: svg.clone(), color, fonts })).clone();
                let obj: DrawRef = d;
                self.cmd_custom(x, y, w, h, Some(obj), 0, (i as u32).wrapping_mul(0x9E37_79B1) ^ color);
            }
            _ => {
                let hair = self.hair().max(1);
                self.draw_color(color_mix(bgc, tcol, 90));
                self.draw_frame(x, y, w, h, self.dp(3), hair);
                let alt = doc.images().get(i).map_or("", |m| m.alt.as_str());
                let alt = if alt.is_empty() { tr("Bild") } else { alt };
                let f = self.md_faces();
                let font = f.get(false, false, true);
                let (asc, desc, _) = self.md_metrics(font, f.size);
                let by = y + (h - asc - desc) / 2 + asc;
                self.draw_clip(x, y, w, h);
                self.cmd_text(x + self.dp(8), by, alt.as_bytes(), font, f.size, self.color(T_TEXT2) | 0xFF00_0000, 0);
                self.draw_unclip();
            }
        }
    }
}

// a link's rectangle in a row: drawn (island coordinates) and logical (records, hit boxes)
struct Link {
    r: [i32; 4],
    logical: [i32; 4],
    label: String,
    spot: usize,
}

// a run drawn with OpenType shaping and bidirectional reordering (text_shape): Hebrew and
// Arabic in their visual order, joined
struct Shaped {
    s: Vec<u8>,
    font: u32,
    size: i32,
    color: u32,
    // the pen and the baseline from the rectangle's corner
    dx: i32,
    base: i32,
}

impl Draw for Shaped {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let old = g.text.set_shaping(true);
        let prev = g.text.get_font();
        g.dc.set_color(self.color);
        g.text.set_font(self.font, self.size);
        g.text.draw_n(g.dc, g.sf, r[0] + self.dx, r[1] + self.base, &self.s, self.s.len());
        g.text.set_font(prev.0, prev.1);
        g.text.set_shaping(old);
    }
}

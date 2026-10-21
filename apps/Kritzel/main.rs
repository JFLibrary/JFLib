// Kritzel -- a drawing program on JFLib  [apps/paint/src/main.s]
//
//   python build_rs.py app Kritzel
//   build/win/Kritzel.exe --shot p.png --size 1280x800
//   build/win/Kritzel.exe --script apps/Kritzel/tests/draw.uis
//
// Layout (top to bottom): menu row, ribbon (Auswahl, Bild, Tools, Pinsel, Formen, Farbe,
// Ebenen), canvas with the size/opacity sliders, status bar.
//
// Modules (one per assembly file; the asm's g_* globals are the fields of one state struct
// per module, gathered in Kritzel).  The [apps/paint/...] tags name the files of the assembly
// version in legacy/, which was called paint:
//   consts      paint.inc
//   doc         layers, composite, pick                      Doc       (paint-core)
//   view        zoom, scroll, coordinate transforms, draw    View      (paint-core)
//   history     undo / redo                                  Hist      (paint-core)
//   tools       tool set, freehand strokes, flood fill       Tools     (paint-core)
//   brushes     the 9 brush variants                         Brushes   (paint-core)
//   fileio      open / save, the prompt, title, error bar    FileIo    (paint-core)
//   imageops    flip / rotate / scale / canvas / crop / skew, layer ops   (paint-core)
//   dialogs     resize, properties, colour dialog            Dialogs   (paint-core)
//   layers_ui   the "Ebenen" panel                           LayersUi  (paint-core)
//   select      selection, floating pixels, clipboard        Sel       (paint-tools)
//   shapes      shape tool                                   Shape     (paint-tools)
//   shapes_geom shape geometry (Sg: the emitter transform, local)       (paint-tools)
//   textool     text tool                                    TextTool  (paint-tools)
//
// Conventions: a module's .globl functions keep their asm names and parameter order.  They
// take `&mut Kritzel` (plus `&mut Ui` when they reach the UI); pure accessors of one module's
// state take only that state (`doc_w(&p.doc)`, `view_to_doc(&p.view, x, y)`,
// `tool_set(&mut p.tools, t)`) so they can be called while other parts of Kritzel are
// borrowed.  Out-parameters become return values; eax 1 / 0 results become bool.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod brushes;
mod agent;
mod consts;
mod dialogs;
mod doc;
mod fileio;
mod history;
mod imageops;
mod layers_ui;
mod select;
mod shapes;
mod shapes_geom;
mod textool;
mod tools;
mod view;

use alloc::format;
use alloc::string::String;

use ui::*;

use consts::*;

const SLIDER_CARD_DIP: i32 = 56; // space of the floating size / opacity card

const DESC: AppDesc = AppDesc {
    title: "Unbenannt - Kritzel",
    flags: AF_RESIZABLE | AF_CUSTOM_TITLEBAR,
    w: 1280,
    h: 800,
    min_w: 720,
    min_h: 480,
    accent: 0,
    cmds: &PT_CMDS,
    menubar: &PT_MENUBAR,
};

/// pt_cmds: the command table of the app record
static PT_CMDS: [Command; 27] = [
    Command::new(CMD_NEW, "&Neu", IC_PAGE, MOD_CTRL, b'N' as u32, 0),
    Command::new(CMD_OPEN, "Ö&ffnen…", IC_OPEN_FILE, MOD_CTRL, b'O' as u32, 0),
    Command::new(CMD_SAVE, "&Speichern", IC_SAVE, MOD_CTRL, b'S' as u32, 0),
    Command::new(CMD_SAVE_AS, "Speichern &unter…", IC_SAVE_AS, MOD_CTRL | MOD_SHIFT, b'S' as u32, 0),
    Command::new(CMD_EXIT, "&Beenden", 0, 0, 0, 0),
    Command::new(CMD_UNDO, "&Rückgängig", IC_UNDO, MOD_CTRL, b'Z' as u32, 0),
    Command::new(CMD_REDO, "&Wiederholen", IC_REDO, MOD_CTRL, b'Y' as u32, 0),
    Command::new(CMD_CUT, "&Ausschneiden", IC_CUT, MOD_CTRL, b'X' as u32, 0),
    Command::new(CMD_COPY, "&Kopieren", IC_COPY, MOD_CTRL, b'C' as u32, 0),
    Command::new(CMD_PASTE, "E&infügen", IC_PASTE, MOD_CTRL, b'V' as u32, 0),
    Command::new(CMD_SELECT_ALL, "Alles aus&wählen", IC_SELECT_ALL, MOD_CTRL, b'A' as u32, 0),
    Command::new(CMD_DELETE, "&Löschen", IC_DELETE, 0, VK_DELETE, 0),
    Command::new(CMD_ZOOM_IN, "Ver&größern", IC_ZOOM_IN, MOD_CTRL, VK_OEM_PLUS, 0),
    Command::new(CMD_ZOOM_OUT, "Ver&kleinern", IC_ZOOM_OUT, MOD_CTRL, VK_OEM_MINUS, 0),
    Command::new(CMD_ZOOM_100, "&100 %", 0, MOD_CTRL, b'0' as u32, 0),
    Command::new(CMD_ZOOM_FIT, "An &Fenster anpassen", IC_FIT_PAGE, 0, 0, 0),
    Command::new(CMD_TOGGLE_LAYERS, "&Ebenen", IC_LAYERS, 0, 0, 0),
    Command::new(CMD_THEME, "&Design wechseln", IC_BRIGHTNESS, 0, 0, 0),
    Command::new(CMD_CROP, "&Zuschneiden", IC_CROP, MOD_CTRL | MOD_SHIFT, b'X' as u32, 0),
    Command::new(CMD_RESIZE, "&Größe ändern und schräg stellen…", IC_RESIZE, MOD_CTRL, b'W' as u32, 0),
    Command::new(CMD_PROPERTIES, "Bild&eigenschaften…", IC_INFO, MOD_CTRL, b'E' as u32, 0),
    Command::new(CMD_EDIT_COLOR, "&Farben bearbeiten…", IC_COLOR, 0, 0, 0),
    Command::new(CMD_ROT_R, "Nach &rechts drehen", IC_ROTATE, 0, 0, 0),
    Command::new(CMD_ROT_L, "Nach &links drehen", 0, 0, 0, 0),
    Command::new(CMD_ROT_180, "Um &180° drehen", 0, 0, 0, 0),
    Command::new(CMD_FLIP_H, "&Horizontal spiegeln", 0, 0, 0, 0),
    Command::new(CMD_FLIP_V, "&Vertikal spiegeln", 0, 0, 0, 0),
];

/// pt_menubar
static PT_MENUBAR: [Mi; 3] = [Mi::Menu("&Datei", &M_FILE), Mi::Menu("&Bearbeiten", &M_EDIT), Mi::Menu("&Ansicht", &M_VIEW)];
static M_FILE: [Mi; 8] = [
    Mi::Cmd(CMD_NEW),
    Mi::Cmd(CMD_OPEN),
    Mi::Cmd(CMD_SAVE),
    Mi::Sub("Speichern &unter", &M_SAVEAS),
    Mi::Sep,
    Mi::Cmd(CMD_PROPERTIES),
    Mi::Sep,
    Mi::Cmd(CMD_EXIT),
];
static M_SAVEAS: [Mi; 6] = [
    Mi::Item(CMD_SAVE_PNG, "&PNG-Bild", IC_PHOTO),
    Mi::Item(CMD_SAVE_JPEG, "&JPEG-Bild", IC_PHOTO),
    Mi::Item(CMD_SAVE_BMP, "&BMP-Bild", IC_PHOTO),
    Mi::Item(CMD_SAVE_GIF, "&GIF-Bild", IC_PHOTO),
    Mi::Sep,
    Mi::Item(CMD_SAVE_AS, "&Anderes Format…", 0),
];
static M_EDIT: [Mi; 13] = [
    Mi::Cmd(CMD_UNDO),
    Mi::Cmd(CMD_REDO),
    Mi::Sep,
    Mi::Cmd(CMD_CUT),
    Mi::Cmd(CMD_COPY),
    Mi::Cmd(CMD_PASTE),
    Mi::Sep,
    Mi::Cmd(CMD_SELECT_ALL),
    Mi::Cmd(CMD_DELETE),
    Mi::Sep,
    Mi::Cmd(CMD_CROP),
    Mi::Cmd(CMD_RESIZE),
    Mi::Sub("&Drehen und spiegeln", &M_ROTATE),
];
static M_ROTATE: [Mi; 6] =
    [Mi::Cmd(CMD_ROT_R), Mi::Cmd(CMD_ROT_L), Mi::Cmd(CMD_ROT_180), Mi::Sep, Mi::Cmd(CMD_FLIP_H), Mi::Cmd(CMD_FLIP_V)];
static M_VIEW: [Mi; 7] = [
    Mi::Cmd(CMD_ZOOM_IN),
    Mi::Cmd(CMD_ZOOM_OUT),
    Mi::Cmd(CMD_ZOOM_100),
    Mi::Cmd(CMD_ZOOM_FIT),
    Mi::Sep,
    Mi::Cmd(CMD_TOGGLE_LAYERS),
    Mi::Cmd(CMD_THEME),
];

const RANGE_SIZE: Range = Range::new(1, 100, 1, RG_VERT);
const RANGE_OPACITY: Range = Range::new(1, 100, 1, RG_VERT);
const RANGE_ZOOM: Range = Range::new(13, 800, 1, 0);
/// handle positions: (0/1/2 thirds of width, 0/1/2 of height) as halves
const HANDLE_POS: [[u8; 2]; 8] = [[0, 0], [1, 0], [2, 0], [0, 1], [2, 1], [0, 2], [1, 2], [2, 2]];
/// pointer shape per handle (0 = not interactive): right, bottom, corner resize
const HANDLE_CURSOR: [u32; 8] = [0, 0, 0, 0, CUR_SIZE_WE, 0, CUR_SIZE_NS, CUR_SIZE_NWSE];
const PALETTE: [u32; 30] = [
    0xFF000000, 0xFF7F7F7F, 0xFF880015, 0xFFED1C24, 0xFFFF7F27, 0xFFFFF200, 0xFF22B14C, 0xFF00A2E8, 0xFF3F48CC, 0xFFA349A4,
    0xFFFFFFFF, 0xFFC3C3C3, 0xFFB97A57, 0xFFFFAEC9, 0xFFFFC90E, 0xFFEFE4B0, 0xFFB5E61D, 0xFF99D9EA, 0xFF7092BE, 0xFFC8BFE7,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// fmt_buf(buf, cap, ...): the text cut to cap - 1 bytes (never inside a UTF-8 sequence)
pub(crate) fn fmt_cap(s: String, cap: usize) -> String {
    if s.len() < cap {
        return s;
    }
    let mut n = cap - 1;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    String::from(&s[..n])
}

/// the app: every module's state (the asm's globals) and the .data / .bss of main.s
pub(crate) struct Kritzel {
    pub(crate) doc: doc::Doc,
    pub(crate) view: view::View,
    pub(crate) hist: history::Hist,
    pub(crate) tools: tools::Tools,
    pub(crate) br: brushes::Brushes,
    pub(crate) file: fileio::FileIo,
    pub(crate) dlg: dialogs::Dialogs,
    pub(crate) lay: layers_ui::LayersUi,
    pub(crate) sel: select::Sel,
    pub(crate) shape: shapes::Shape,
    pub(crate) txt: textool::TextTool,
    // main.s
    /// 1 or 2: the colour well the palette sets (g_active_col)
    pub(crate) active_col: i32,
    /// 30 palette colours, the last row custom (g_palette)
    pub(crate) palette: [u32; 30],
    resize_handle: i32,
    pub(crate) new_w: i32,
    pub(crate) new_h: i32,
    zoom_pct: i32,
}

impl Default for Kritzel {
    fn default() -> Kritzel {
        Kritzel {
            doc: Default::default(),
            view: Default::default(),
            hist: Default::default(),
            tools: Default::default(),
            br: Default::default(),
            file: Default::default(),
            dlg: Default::default(),
            lay: Default::default(),
            sel: Default::default(),
            shape: Default::default(),
            txt: Default::default(),
            active_col: 1,
            palette: PALETTE,
            resize_handle: -1,
            new_w: 0,
            new_h: 0,
            zoom_pct: 0,
        }
    }
}

impl App for Kritzel {
    fn actions(&self) -> &'static [agent_core::ActionSpec] { agent::ACTIONS }
    fn action_revision(&self) -> Option<u64> { Some(doc::doc_version(&self.doc) as u64) }
    fn action_available(&self, spec: &agent_core::ActionSpec) -> Result<(), agent_core::ActionError> { self.agent_available(spec) }
    fn action(&mut self, ui: &mut Ui, spec: &agent_core::ActionSpec, input: &agent_core::Value, _context: &agent_core::ActionContext) -> Result<agent_core::ActionResult, agent_core::ActionError> {
        self.agent_action(ui, spec, input)
    }
    fn action_inspect(&self, query: &agent_core::Value) -> Result<agent_core::Value, agent_core::ActionError> { Ok(self.agent_state(query)) }
    // pt_init() -> 0
    fn init(&mut self, ui: &mut Ui) -> bool {
        let p = self;
        view::view_reclaim(p);
        doc::doc_new(p, DEF_W, DEF_H);
        // a file to open: --open FILE, or a first argument that is not an option
        let mut f = plat::arg_value("--open");
        if f.is_none() {
            let a = plat::args();
            if a.len() >= 2 && !a[1].starts_with('-') {
                f = Some(a[1].clone());
            }
        }
        if let Some(path) = f {
            fileio::open_file(p, ui, &path);
        }
        true
    }

    // pt_frame() -- the whole window
    fn frame(&mut self, ui: &mut Ui) {
        let p = self;
        view::view_reclaim(p); // the layers lent to the canvas draw object
        cmd_states(p, ui);
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V, 0);
        let t = String::from(ui.title());
        ui.titlebar_begin(&t, IC_PENCIL);
        ui.titlebar_end();
        menu_row(p, ui);
        ribbon(p, ui);
        fileio::file_infobar(p, ui);
        canvas_area(p, ui);
        status_bar(p, ui);
        fileio::file_frame(p, ui);
        dialogs::dialogs_frame(p, ui);
        ui.box_end();
        // the canvas composites at render time (view_draw): lend it the layers
        view::view_frame_end(p, ui);
    }

    // pt_cmd(id, arg) -> handled
    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        view::view_reclaim(self);
        let r = pt_cmd(self, ui, id, arg);
        // a render may follow without another pass
        view::view_frame_end(self, ui);
        r
    }
}

// ------------------------------------------------------------------ frame

/// cmd_states() -- enable / check the commands for menus, buttons and accelerators
fn cmd_states(p: &mut Kritzel, ui: &mut Ui) {
    ui.cmd_state(CMD_UNDO, if history::hist_can_undo(&p.hist) { 0 } else { CF_DISABLED });
    ui.cmd_state(CMD_REDO, if history::hist_can_redo(&p.hist) { 0 } else { CF_DISABLED });
    ui.cmd_state(CMD_TOGGLE_LAYERS, if layers_ui::layers_visible(&p.lay) { CF_CHECKED } else { 0 });
}

/// menu_row() -- menu bar, quick actions, settings
fn menu_row(_p: &mut Kritzel, ui: &mut Ui) {
    ui.next_size(-1, 40);
    ui.box_begin(0, BOX_H | BOX_XCENTER | PAD_4, 2);
    ui.menubar(None); // the menu bar of the app record
    ui.separator();
    ui.cmd_button(CMD_SAVE, ST_ICON_ONLY | ST_SUBTLE);
    ui.cmd_button(CMD_UNDO, ST_ICON_ONLY | ST_SUBTLE);
    ui.cmd_button(CMD_REDO, ST_ICON_ONLY | ST_SUBTLE);
    ui.spacer(-1);
    if ui.icon_button(2010, IC_SETTINGS, "Design wechseln") & EV_ACT != 0 {
        ui.cmd_run(CMD_THEME, 0);
    }
    ui.box_end();
}

/// group_begin() -- a ribbon group: content row + caption
fn group_begin(ui: &mut Ui) {
    ui.box_begin(0, BOX_V | BOX_XCENTER, 2);
    ui.next_size(0, 64);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 2);
}

/// group_end(caption)
fn group_end(ui: &mut Ui, caption: &str) {
    ui.box_end();
    ui.text(TS_CAPTION | TX_SECONDARY, caption);
    ui.box_end();
    ui.separator();
}

/// col_begin() -- a vertical stack of small buttons inside a group
fn col_begin(ui: &mut Ui) {
    ui.box_begin(0, BOX_V | BOX_CENTER, 2);
}

/// tool(i, icon, label) -- a tool button (selects tools.tool = i)
fn tool(p: &mut Kritzel, ui: &mut Ui, i: i32, icon: u32, label: &str) {
    ui.tool_button(ID_TOOL0 + i as u32, icon, label, &mut p.tools.tool, i);
}

/// ribbon() -- the tool bar
fn ribbon(p: &mut Kritzel, ui: &mut Ui) {
    ui.next_size(-1, 92);
    ui.box_begin(0, BOX_H | BOX_XSTART | PAD_8, 10);
    // ---- Auswahl
    group_begin(ui);
    ui.next_flags(ST_LARGE);
    let ic = select::select_icon(p); // (select agent) rectangle / lasso icon
    tool(p, ui, TOOL_SELECT, ic, "");
    select::select_ribbon(p, ui);
    group_end(ui, "Auswahl");
    // ---- Bild
    group_begin(ui);
    col_begin(ui);
    ui.cmd_button(CMD_CROP, ST_ICON_ONLY | ST_SUBTLE);
    ui.cmd_button(CMD_RESIZE, ST_ICON_ONLY | ST_SUBTLE);
    ui.box_end();
    col_begin(ui);
    // rotate / flip drop-down
    if ui.icon_button(ID_ROTATE, IC_ROTATE, "Drehen und spiegeln") & EV_ACT != 0 {
        ui.popup_open(ID_ROTATE);
    }
    if ui.flyout_begin(ID_ROTATE, FL_BELOW) {
        ui.menu_cmd(CMD_ROT_R);
        ui.menu_cmd(CMD_ROT_L);
        ui.menu_cmd(CMD_ROT_180);
        ui.menu_sep();
        ui.menu_cmd(CMD_FLIP_V);
        ui.menu_cmd(CMD_FLIP_H);
    }
    ui.flyout_end();
    ui.box_end();
    group_end(ui, "Bild");
    // ---- Tools (2 rows x 3)
    group_begin(ui);
    col_begin(ui);
    tool(p, ui, TOOL_PENCIL, IC_PENCIL, "Stift");
    tool(p, ui, TOOL_ERASER, IC_ERASER, "Radierer");
    ui.box_end();
    col_begin(ui);
    tool(p, ui, TOOL_FILL, IC_DROP, "Füllen");
    tool(p, ui, TOOL_PICKER, IC_EYEDROPPER, "Farbauswahl");
    ui.box_end();
    col_begin(ui);
    tool(p, ui, TOOL_TEXT, IC_FONT, "Text");
    tool(p, ui, TOOL_ZOOM, IC_ZOOM, "Lupe");
    ui.box_end();
    group_end(ui, "Tools");
    // ---- Pinsel
    group_begin(ui);
    ui.next_flags(ST_LARGE);
    tool(p, ui, TOOL_BRUSH, IC_BRUSH, "");
    brushes::brush_ribbon(p, ui);
    group_end(ui, "Pinsel");
    // ---- Formen
    group_begin(ui);
    shapes::shape_ribbon(p, ui);
    group_end(ui, "Formen");
    // ---- Farbe
    group_begin(ui);
    colors_group(p, ui);
    group_end(ui, "Farbe");
    // ---- Ebenen
    group_begin(ui);
    ui.next_flags(ST_LARGE);
    if layers_ui::layers_visible(&p.lay) {
        ui.next_flags(ST_CHECKED);
    }
    if ui.icon_button(ID_LAYERS, IC_LAYERS, "") & EV_ACT != 0 {
        layers_ui::layers_toggle(&mut p.lay);
    }
    group_end(ui, "Ebenen");
    textool::text_ribbon(p, ui); // contextual text options (text tool)
    ui.box_end();
}

/// colors_group() -- colour 1 / 2, the palette and "edit colours"
fn colors_group(p: &mut Kritzel, ui: &mut Ui) {
    // colour 1 and 2 (bigger wells), the active one is ringed
    ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 4);
    ui.next_size(30, 30);
    if ui.swatch(ID_COLOR1, p.tools.col1, p.active_col == 1) & EV_ACT != 0 {
        p.active_col = 1;
    }
    ui.next_size(30, 30);
    if ui.swatch(ID_COLOR2, p.tools.col2, p.active_col == 2) & EV_ACT != 0 {
        p.active_col = 2;
    }
    ui.box_end();
    // palette: 3 rows x 10
    ui.box_begin(0, BOX_V | BOX_CENTER, 2);
    for row in 0..3 {
        ui.box_begin(0, BOX_H, 2);
        for col in 0..10 {
            let i = row * 10 + col;
            ui.next_size(20, 20);
            if ui.swatch(ID_PAL0 + i as u32, p.palette[i], false) & EV_ACT != 0 {
                let c = p.palette[i];
                if c & 0xFF00_0000 != 0 {
                    if p.active_col == 2 {
                        p.tools.col2 = c;
                    } else {
                        p.tools.col1 = c;
                    }
                }
            }
        }
        ui.box_end();
    }
    ui.box_end();
    ui.cmd_button(CMD_EDIT_COLOR, ST_ICON_ONLY | ST_SUBTLE);
}

/// canvas_area() -- the image with the side sliders
fn canvas_area(p: &mut Kritzel, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H | BOX_CLIP, 0);
    // background of the canvas area
    let c = ui.color(T_BG_ALT);
    ui.draw_color(c);
    let r = ui.box_rect();
    ui.draw_rect(r[0], r[1], r[2], r[3]);
    // the canvas widget fills the area
    ui.next_size(-1, -1);
    let f = canvas_cursor(p, ui);
    ui.next_flags(f);
    let ver = view::view_version(p);
    let draw = view::view_draw_ref(p);
    let inp = ui.canvas(ID_CANVAS, Some(draw), 0, ver).clone();
    // the image is laid out right of the floating slider card
    let m = ui.dp(SLIDER_CARD_DIP);
    let rc = inp.rect;
    view::view_layout(p, ui, rc[0] + m, rc[1], rc[2] - m, rc[3]);
    tools::tool_input(p, ui, &inp);
    // the canvas command was recorded with the old version: when the tool changed the
    // image, the zoom or the scroll position, build the frame once more
    if view::view_version(p) != ver {
        ui.repass();
    }
    // overlays stay inside the canvas widget (not over the side panels)
    ui.clip_push(rc[0], rc[1], rc[2], rc[3]);
    draw_handles(p, ui);
    shapes::shape_overlay(p, ui);
    select::select_overlay(p, ui);
    textool::text_overlay(p, ui);
    brushes::brush_overlay(p, ui);
    ui.clip_pop();
    canvas_scrollbars(p, ui, &inp);
    // floating slider card at the left
    ui.next_rect(12, 40, 44, 320);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_XCENTER | BOX_BLOCK | PAD_8, 6);
    ui.icon(IC_SLIDERS, 16);
    ui.next_size(0, -1);
    ui.slider(ID_SIZE, &mut p.tools.size, &RANGE_SIZE);
    ui.icon(IC_DROP, 16);
    ui.next_size(0, -1);
    ui.slider(ID_OPACITY, &mut p.tools.opacity, &RANGE_OPACITY);
    ui.box_end();
    layers_ui::layers_panel(p, ui);
    ui.box_end();
}

/// canvas_cursor() -> next_flags value selecting the pointer shape for the tool
fn canvas_cursor(p: &mut Kritzel, ui: &mut Ui) -> u32 {
    let t = p.tools.tool;
    if t == TOOL_SHAPE {
        return shapes::shape_cursor(p, ui); // (shapes agent) handles / move cursor
    }
    if t == TOOL_SELECT {
        return select::select_cursor(p, ui); // (select agent) handles / move cursor
    }
    let mut c = CUR_CROSS;
    if t == TOOL_ZOOM {
        c = CUR_HAND;
    }
    if t == TOOL_TEXT {
        c = CUR_IBEAM;
    }
    c << HB_CURSOR_SHIFT
}

/// draw_handles() -- the eight squares around the image; right, bottom and the bottom-right
/// corner resize the canvas (applied on release, undoable)
fn draw_handles(p: &mut Kritzel, ui: &mut Ui) {
    let hs = ui.dp(7);
    let cr = view::view_canvas_rect(&p.view);
    for i in 0..8usize {
        let (hx, hy) = (HANDLE_POS[i][0] as i32, HANDLE_POS[i][1] as i32);
        let half = (hs as u32 >> 1) as i32;
        let x = (cr[2].wrapping_mul(hx) >> 1).wrapping_add(cr[0]).wrapping_sub(half);
        let y = (cr[3].wrapping_mul(hy) >> 1).wrapping_add(cr[1]).wrapping_sub(half);
        // interactive handles: 4 right, 6 bottom, 7 corner
        let cur = HANDLE_CURSOR[i];
        if cur != 0 {
            let id = ID_HANDLE0 + i as u32;
            let ev = ui.hit_rect(id, x - 3, y - 3, hs + 6, hs + 6, cur << HB_CURSOR_SHIFT);
            ui.sem(K_BUTTON, id, None, 0);
            if ev & EV_HELD != 0 {
                p.resize_handle = i as i32;
                let (mx, my) = ui.mouse();
                let (dx, dy) = view::view_to_doc(&p.view, mx, my);
                let mut w = doc::doc_w(&p.doc);
                let mut h = doc::doc_h(&p.doc);
                if i != 6 {
                    w = dx;
                }
                if i != 4 {
                    h = dy;
                }
                w = w.clamp(1, DOC_MAX);
                h = h.clamp(1, DOC_MAX);
                p.new_w = w;
                p.new_h = h;
            } else if ev & EV_COMMIT != 0 && p.resize_handle == i as i32 {
                p.resize_handle = -1;
                ui.cmd_run(CMD_CANVAS, 0);
            }
        }
        ui.draw_color(0xFF8A8A8A);
        ui.draw_rect(x, y, hs, hs);
        ui.draw_color(0xFFFFFFFF);
        ui.draw_rect(x + 1, y + 1, hs - 2, hs - 2);
    }
    // while dragging: dashed-looking outline of the new canvas size
    if p.resize_handle >= 0 {
        let (ex, ey) = view::doc_to_view(&p.view, p.new_w, p.new_h);
        ui.draw_color(0xFF0078D4);
        ui.draw_frame(cr[0], cr[1], ex - cr[0], ey - cr[1], 0, 1);
    }
}

/// status_bar() -- pointer position, image size, zoom
fn status_bar(p: &mut Kritzel, ui: &mut Ui) {
    ui.next_size(-1, 32);
    ui.box_begin(0, BOX_H | BOX_XCENTER | PAD_8, 8);
    ui.icon(IC_POINTER, 16);
    // pointer position (only while over the image)
    let mut s1 = String::new();
    let cd = p.tools.cursor_doc;
    if p.tools.canvas_ev & EV_HOVER != 0
        && (cd[0] as u32) < doc::doc_w(&p.doc) as u32
        && (cd[1] as u32) < doc::doc_h(&p.doc) as u32
    {
        s1 = format!("{}, {}px", cd[0], cd[1]);
    }
    ui.next_size(110, 0);
    ui.text(TS_CAPTION, &s1);
    ui.icon(IC_SELECT_RECT, 16);
    ui.next_size(110, 0);
    let ss = select::select_status(p); // (select agent) selection size
    ui.text(TS_CAPTION, &ss);
    ui.icon(IC_PAGE, 16);
    let s2 = format!("{} × {}px", doc::doc_w(&p.doc), doc::doc_h(&p.doc));
    ui.text(TS_CAPTION, &s2);
    ui.spacer(-1);
    if ui.icon_button(ID_ZOOM_FIT, IC_FIT_PAGE, "An Fenster anpassen") & EV_ACT != 0 {
        ui.cmd_run(CMD_ZOOM_FIT, 0);
    }
    let s3 = format!("{} %", p.view.zoom as u32 / 10);
    ui.next_size(48, 0);
    ui.text(TS_CAPTION | TX_RIGHT, &s3);
    if ui.icon_button(ID_ZOOM_OUT, IC_ZOOM_OUT, "Verkleinern") & EV_ACT != 0 {
        ui.cmd_run(CMD_ZOOM_OUT, 0);
    }
    // zoom slider works on the zoom in percent
    p.zoom_pct = (p.view.zoom as u32 / 10) as i32;
    ui.next_size(140, 0);
    if ui.slider(ID_ZOOM_SLIDER, &mut p.zoom_pct, &RANGE_ZOOM) & EV_ACT != 0 {
        let cr = view::view_canvas_rect(&p.view);
        let ax = cr[0] + (cr[2] as u32 >> 1) as i32;
        let ay = cr[1] + (cr[3] as u32 >> 1) as i32;
        view::view_set_zoom(p, ui, p.zoom_pct.wrapping_mul(10), ax, ay);
    }
    if ui.icon_button(ID_ZOOM_IN, IC_ZOOM_IN, "Vergrößern") & EV_ACT != 0 {
        ui.cmd_run(CMD_ZOOM_IN, 0);
    }
    ui.box_end();
}

// ------------------------------------------------------------------ commands

/// pt_cmd(id, arg) -> handled
fn pt_cmd(p: &mut Kritzel, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if select::select_pre_cmd(p, ui, id, arg) {
        return true; // (select agent) floating pixels first
    }
    if shapes::shape_pre_cmd(p, ui, id) {
        return true; // (shapes agent) commit the edited shape
    }
    // Neu / Öffnen / Speichern / close request
    let r = fileio::file_cmd(p, ui, id, arg);
    if id == UIC_CLOSE {
        return r;
    }
    if r {
        return true;
    }
    if dialogs::dialogs_cmd(p, ui, id, arg) {
        return true;
    }
    match id {
        CMD_UNDO => {
            history::hist_undo(p);
        }
        CMD_REDO => {
            history::hist_redo(p);
        }
        CMD_ZOOM_IN => zoom_center(p, ui, 1),
        CMD_ZOOM_OUT => zoom_center(p, ui, -1),
        CMD_ZOOM_100 => zoom_100(p, ui),
        CMD_ZOOM_FIT => view::view_fit(p, ui),
        CMD_TOGGLE_LAYERS => layers_ui::layers_toggle(&mut p.lay),
        CMD_THEME => {
            let d = ui.theme_dark();
            ui.theme_set(!d as u32);
        }
        CMD_ROT_R => imageops::op_rotate90(p, 1),
        CMD_ROT_L => imageops::op_rotate90(p, -1),
        CMD_ROT_180 => imageops::op_rotate180(p),
        CMD_FLIP_H => imageops::op_flip_h(p),
        CMD_FLIP_V => imageops::op_flip_v(p),
        CMD_CANVAS => {
            if doc::doc_w(&p.doc) != p.new_w || doc::doc_h(&p.doc) != p.new_h {
                imageops::op_canvas(p, p.new_w, p.new_h);
            }
        }
        CMD_LAYER_ADD => {
            imageops::layer_add(p);
        }
        CMD_LAYER_DEL => imageops::layer_delete(p),
        CMD_LAYER_DUP => {
            imageops::layer_duplicate(p);
        }
        CMD_LAYER_MERGE => imageops::layer_merge_down(p),
        CMD_LAYER_UP => imageops::layer_move(p, 1),
        CMD_LAYER_DOWN => imageops::layer_move(p, -1),
        _ => {
            // tool modules
            return select::select_cmd(p, ui, id, arg)
                || shapes::shape_cmd(p, ui, id, arg)
                || textool::text_cmd(p, ui, id, arg)
                || brushes::brush_cmd(p, ui, id, arg);
        }
    }
    true
}

/// zoom_center(dir) -- zoom step around the view centre
fn zoom_center(p: &mut Kritzel, ui: &mut Ui, dir: i32) {
    let cr = view::view_canvas_rect(&p.view);
    let ax = cr[0] + (cr[2] as u32 >> 1) as i32;
    let ay = cr[1] + (cr[3] as u32 >> 1) as i32;
    view::view_zoom_step(p, ui, dir, ax, ay);
}

/// canvas_scrollbars(&UIINPUT) -- scroll bars along the right and bottom edge of the canvas
/// when the image (plus margins) is larger than the view
fn canvas_scrollbars(p: &mut Kritzel, ui: &mut Ui, inp: &UiInput) {
    let (cw, ch, vw, vh) = view::view_extent(p, ui);
    let t = ui.dp(12);
    let hbar = cw > vw;
    let vbar = ch > vh;
    let r = inp.rect;
    if hbar {
        // horizontal: bottom edge, leaving the corner to the vertical bar
        let w = if vbar { r[2] - t } else { r[2] };
        ui.next_rect_px(r[0], r[1] + r[3] - t, w, t);
        if ui.scrollbar(ID_HSCROLL, &mut p.view.sx, cw, vw, false) & EV_ACT != 0 {
            ui.repass();
        }
    }
    if vbar {
        let h = if hbar { r[3] - t } else { r[3] };
        ui.next_rect_px(r[0] + r[2] - t, r[1], t, h);
        if ui.scrollbar(ID_VSCROLL, &mut p.view.sy, ch, vh, true) & EV_ACT != 0 {
            ui.repass();
        }
    }
}

/// zoom_100() -- 100 % around the view centre
fn zoom_100(p: &mut Kritzel, ui: &mut Ui) {
    let cr = view::view_canvas_rect(&p.view);
    let ax = cr[0] + (cr[2] as u32 >> 1) as i32;
    let ay = cr[1] + (cr[3] as u32 >> 1) as i32;
    view::view_set_zoom(p, ui, 1000, ax, ay);
}

ui::main!(ui::launch(&DESC, Kritzel::default(), &style::STYLE));

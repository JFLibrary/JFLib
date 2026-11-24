// ribbon -- the registers of the ribbon: Start, Einfügen, Entwurf, (JD18) Daten, Ansicht with
// every P1 command of FEATURES.md 3.3-3.9, the context registers Container-Tools › Format (RK-01)
// and Textwerkzeuge › Text (RK-11), and (JD10) Funktionsübergreifendes Flussdiagramm (RK-03)
// while the page holds one
//
//   tabs(app) -> Vec<Reg>         the registers shown now: File, the four, the context ones
//                                 the selection asks for (app.selection_kind)
//   draw(app, ui, reg)            the ribbon of a register
//
// A register is a list of groups; a group a caption, an optional dialog launcher (a command)
// and columns: a large button (plain, menu, split), a column of up to three small controls
// (buttons, menus, split buttons, check boxes, a row of icon toggles), a gallery placeholder
// (rendered previews come with JD11, FEATURES 7: galleries get no drawn icons), or the font
// boxes.  Every control is a command of commands.rs: it is enabled when the command has a
// handler, checked when its toggle is on, and its QuickInfo names the shortcut of the current
// language.  Labels come through tc() where they are drawn; the style breaks a long label of a
// large button into two lines.
// Ids: a control has its command's id (a register shows one ribbon at a time), a split
// button's menu id | SPLIT, a launcher LAUNCH + register * 32 + group, gallery tiles TILE + n.

use alloc::string::String;
use alloc::vec::Vec;
use ui::i18n::t;
use ui::*;

use crate::commands::{self, *};
use crate::{lang, tc, Diagrammer};

/// the kinds of selection that bring context registers (until JD7 the shell preview,
/// --shell-preview, stands in for a real selection)
pub(crate) const SEL_NONE: u8 = 0;
pub(crate) const SEL_SHAPE: u8 = 1;
pub(crate) const SEL_CONTAINER: u8 = 2;
pub(crate) const SEL_TEXT: u8 = 3;

/// a register of the row
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Reg {
    File,
    Start,
    Insert,
    Design,
    /// (JD18) Daten
    Data,
    View,
    Container,
    Text,
    /// (JD10) the cross-functional flowchart's register
    Cff,
}

// the registers' labels (German, translated where drawn)
// i18n
const L_FILE: &str = "Datei";
const L_START: &str = "Start";
const L_INSERT: &str = "Einfügen";
const L_DESIGN: &str = "Entwurf";
const L_VIEW: &str = "Ansicht";
const L_DATA: &str = "Daten";
const L_CONTAINER_SET: &str = "Container-Tools";
const L_CONTAINER: &str = "Format|Register";
const L_TEXT_SET: &str = "Textwerkzeuge";
const L_TEXT: &str = "Text|Register";
const L_CFF: &str = "Funktionsübergreifendes Flussdiagramm";

/// the registers shown now (the File register first)
pub(crate) fn tabs(app: &Diagrammer) -> Vec<Reg> {
    let mut v = alloc::vec![Reg::File, Reg::Start, Reg::Insert, Reg::Design, Reg::Data, Reg::View];
    // (JD13, VL-04) also when the drawing's template brought the register
    if crate::structures::has_cff(app) || crate::templates::has_register(app, "cff") {
        v.push(Reg::Cff);
    }
    match app.selection_kind() {
        SEL_CONTAINER => v.push(Reg::Container),
        SEL_TEXT => v.push(Reg::Text),
        _ => {}
    }
    v
}

/// the row's description of the registers, in the current language
pub(crate) fn tab_descs(ui: &Ui, regs: &[Reg]) -> Vec<RibbonTab<'static>> {
    regs.iter()
        .map(|r| match r {
            Reg::File => RibbonTab::file(t(L_FILE)),
            Reg::Start => RibbonTab::new(t(L_START)),
            Reg::Insert => RibbonTab::new(t(L_INSERT)),
            Reg::Design => RibbonTab::new(t(L_DESIGN)),
            Reg::Data => RibbonTab::new(t(L_DATA)),
            Reg::View => RibbonTab::new(t(L_VIEW)),
            Reg::Container => RibbonTab::context(tc(L_CONTAINER), t(L_CONTAINER_SET), ui.color(T_OK)),
            Reg::Text => RibbonTab::context(tc(L_TEXT), t(L_TEXT_SET), ui.color(T_WARN)),
            Reg::Cff => RibbonTab::new(t(L_CFF)),
        })
        .collect()
}

// ------------------------------------------------------------------ the registers as data

// an entry of a menu
enum It {
    C(u32),
    Sep,
    Sub(&'static str, &'static [It]),
}

// a small control
enum Sm {
    Btn(u32),
    Menu(u32, &'static [It]),
    Split(u32, &'static [It]),
    Check(u32),
    Row(&'static [u32]),
}

// a column of a group
enum Col {
    Big(u32),
    BigMenu(u32, &'static [It]),
    BigSplit(u32, &'static [It]),
    Small(&'static [Sm]),
    Gallery(u32, u32, u32),
    Fonts,
}

// a group: caption, launcher command (0 = none), columns
struct Group(&'static str, u32, &'static [Col]);

use It::{Sep, Sub, C};

static PASTE_MENU: [It; 4] = [C(PASTE), C(PASTE_SPECIAL), C(PASTE_LINK), C(DUPLICATE)];
static DRAW_MENU: [It; 6] = [C(RECTANGLE), C(ELLIPSE), C(LINE_TOOL), C(FREEFORM), C(ARC), C(PENCIL)];
static FILL_MENU: [It; 4] = [C(FILL_NONE), Sep, C(MORE_COLORS), C(FILL_OPTIONS)];
static LINE_MENU: [It; 8] = [C(LINE_NONE), C(MORE_COLORS), Sep, C(LINE_WEIGHT), C(LINE_DASH), C(LINE_ARROWS), Sep, C(LINE_OPTIONS)];
static COLOR_MENU: [It; 1] = [C(MORE_COLORS)];
static ALIGN_MENU: [It; 9] = [C(AUTO_ALIGN), Sep, C(ALIGN_LEFT), C(ALIGN_CENTER), C(ALIGN_RIGHT), Sep, C(ALIGN_TOP), C(ALIGN_MIDDLE), C(ALIGN_BOTTOM)];
static ROTATE_MENU: [It; 4] = [C(ROTATE_LEFT), C(ROTATE_RIGHT), C(FLIP_H), C(FLIP_V)];
// i18n
// (JD21 layout) Visio's Positionieren: spacing, distribution, page breaks, turning
static POSITION_MENU: [It; 13] = [
    C(AUTO_SPACE),
    C(AUTO_ALIGN_SPACE),
    C(SPACING_OPTIONS),
    Sep,
    C(DISTRIBUTE_H),
    C(DISTRIBUTE_V),
    C(DISTRIBUTE_MORE),
    Sep,
    C(MOVE_OFF_BREAKS),
    C(AVOID_BREAKS),
    Sep,
    Sub("Shapes drehen", &ROTATE_MENU),
    Sub("Diagramm drehen", &DIAGRAM_MENU), // i18n
];
static DIAGRAM_MENU: [It; 4] = [C(DIAGRAM_ROTATE_LEFT), C(DIAGRAM_ROTATE_RIGHT), C(DIAGRAM_FLIP_V), C(DIAGRAM_FLIP_H)];
static FRONT_MENU: [It; 2] = [C(BRING_FRONT), C(BRING_FORWARD)];
static BACK_MENU: [It; 2] = [C(SEND_BACK), C(SEND_BACKWARD)];
// (JD21 ops) the shape operations (Entwicklertools › Shape-Design › Vorgänge in Visio; JD23 brings
// that register)
// i18n
static GROUP_MENU_ITEMS: [It; 4] = [C(GROUP), C(UNGROUP), Sep, Sub("Vorgänge", &OPERATIONS_MENU)];
static OPERATIONS_MENU: [It; 7] = [C(OP_UNION), C(OP_COMBINE), C(OP_FRAGMENT), C(OP_INTERSECT), C(OP_SUBTRACT), Sep, C(UPDATE_ALIGN_BOX)];
static FIND_MENU_ITEMS: [It; 2] = [C(FIND), C(REPLACE)];
static LAYER_MENU: [It; 2] = [C(LAYER_ASSIGN), C(LAYER_PROPERTIES)];
static SELECT_MENU_ITEMS: [It; 4] = [C(SELECT_ALL), C(SELECT_BY_TYPE), C(SELECT_AREA), C(LASSO)];
static PAGE_MENU: [It; 3] = [C(PAGE_BLANK), C(PAGE_BACKGROUND), C(PAGE_DUPLICATE)];
static PICTURE_MENU: [It; 1] = [C(PICTURE_FILE)];
static CONTAINER_MENU: [It; 6] = [C(CONTAINER_1), C(CONTAINER_2), C(CONTAINER_3), C(CONTAINER_4), C(CONTAINER_5), C(CONTAINER_6)];
static CALLOUT_MENU: [It; 6] = [C(CALLOUT_1), C(CALLOUT_2), C(CALLOUT_3), C(CALLOUT_4), C(CALLOUT_5), C(CALLOUT_6)];
static CT_STYLE_MENU: [It; 6] = [C(CT_STYLE_1), C(CT_STYLE_2), C(CT_STYLE_3), C(CT_STYLE_4), C(CT_STYLE_5), C(CT_STYLE_6)];
static CT_HEADING_MENU: [It; 4] = [C(CT_HEADING_1), C(CT_HEADING_2), C(CT_HEADING_3), C(CT_HEADING_4)];
static CT_AUTOSIZE_MENU: [It; 3] = [C(CT_RESIZE_0), C(CT_RESIZE_1), C(CT_RESIZE_2)];
static CFF_ORIENT_MENU: [It; 2] = [C(CFF_HORIZONTAL), C(CFF_VERTICAL)];
static TEXTBOX_MENU: [It; 2] = [C(TEXT_BOX_H), C(TEXT_BOX_V)];
static ORIENT_MENU: [It; 2] = [C(PORTRAIT), C(LANDSCAPE)];
// (JD8 rest: pages) the paper sizes with their measures (page_size.rs)
static SIZE_MENU: [It; 14] = [
    C(SIZE_LETTER), C(SIZE_LEGAL), C(SIZE_TABLOID), C(SIZE_EXECUTIVE), C(SIZE_A5), C(SIZE_A4), C(SIZE_A3), C(SIZE_B5), C(SIZE_B4),
    // (JD15, RW-02) envelopes and the US government sizes below captions
    Sub("Umschläge", &ENVELOPE_MENU), // i18n
    Sub("Behördenformate", &GOV_MENU), // i18n
    Sep, C(FIT_DRAWING), C(MORE_SIZES),
];
static ENVELOPE_MENU: [It; 6] = [C(SIZE_ENV_10), C(SIZE_ENV_DL), C(SIZE_ENV_C4), C(SIZE_ENV_C5), C(SIZE_ENV_C6), C(SIZE_ENV_MONARCH)];
static GOV_MENU: [It; 2] = [C(SIZE_GOV_LETTER), C(SIZE_GOV_LEGAL)];
// (JD11) the variants' own menus (TH-05: new colours, effects, connectors, fonts, embellishment) are P2
static THEME_MENU: [It; 0] = [];
static CONN_MENU: [It; 8] = [C(CONN_RIGHT_ANGLE), C(CONN_STRAIGHT), C(CONN_CURVED), Sep, C(LINE_JUMPS), C(REROUTE), Sep, C(CONNECT_SHAPES)];
static PANES_MENU: [It; 5] = [C(PANE_SHAPES), C(PANE_DATA), C(PANE_SIZEPOS), C(PANE_PANZOOM), C(PANE_FORMAT)];
static MARGIN_MENU: [It; 4] = [C(CT_MARGIN_0), C(CT_MARGIN_1), C(CT_MARGIN_2), C(CT_MARGIN_3)];

// i18n
static START: [Group; 7] = [
    Group("Zwischenablage", 0, &[
        Col::BigSplit(PASTE, &PASTE_MENU),
        Col::Small(&[Sm::Btn(CUT), Sm::Btn(COPY), Sm::Btn(FORMAT_PAINTER)]),
    ]),
    Group("Schriftart", TEXT_DIALOG, &[Col::Fonts]),
    Group("Absatz", TEXT_DIALOG, &[Col::Small(&[
        Sm::Row(&[VALIGN_TOP, VALIGN_MIDDLE, VALIGN_BOTTOM]),
        Sm::Row(&[TEXT_LEFT, TEXT_CENTER, TEXT_RIGHT, TEXT_JUSTIFY]),
    ])]),
    Group("Tools", 0, &[Col::Small(&[Sm::Btn(POINTER), Sm::Btn(CONNECTOR_TOOL), Sm::Btn(TEXT_TOOL)]), Col::Small(&[Sm::Split(DRAW_TOOLS, &DRAW_MENU)])]),
    Group("Formenarten", FORMAT_PANE, &[Col::Small(&[Sm::Menu(FILL, &FILL_MENU), Sm::Menu(LINE_FORMAT, &LINE_MENU)])]),
    Group("Anordnen", 0, &[
        Col::BigMenu(ALIGN, &ALIGN_MENU),
        Col::BigMenu(POSITION, &POSITION_MENU),
        Col::Small(&[Sm::Split(BRING_FRONT, &FRONT_MENU), Sm::Split(SEND_BACK, &BACK_MENU), Sm::Menu(GROUP_MENU, &GROUP_MENU_ITEMS)]),
    ]),
    Group("Bearbeiten", 0, &[Col::Small(&[Sm::Menu(FIND_MENU, &FIND_MENU_ITEMS), Sm::Menu(LAYERS, &LAYER_MENU), Sm::Menu(SELECT_MENU, &SELECT_MENU_ITEMS)])]),
];

// i18n
static INSERT: [Group; 5] = [
    Group("Seiten", 0, &[Col::BigSplit(NEW_PAGE, &PAGE_MENU)]),
    Group("Illustrationen", 0, &[Col::BigMenu(PICTURES, &PICTURE_MENU)]),
    Group("Diagrammteile", 0, &[Col::BigMenu(CONTAINER, &CONTAINER_MENU), Col::BigMenu(CALLOUT, &CALLOUT_MENU), Col::Big(INSERT_CONNECTOR)]),
    Group("Links", 0, &[Col::Big(LINK)]),
    Group("Text", 0, &[Col::BigSplit(TEXT_BOX, &TEXTBOX_MENU), Col::Big(FIELD)]),
];

// i18n
static DESIGN: [Group; 4] = [
    Group("Seite einrichten", PAGE_SETUP, &[Col::BigMenu(ORIENTATION, &ORIENT_MENU), Col::BigMenu(PAGE_SIZE, &SIZE_MENU), Col::Big(AUTO_SIZE)]),
    Group("Designs", 0, &[Col::Gallery(THEMES, 6, 0xFF1F5FA8)]),
    Group("Varianten", 0, &[
        Col::Gallery(VAR_COLORS, 4, 0xFF4A7A3A),
        Col::Small(&[Sm::Menu(VAR_COLORS, &THEME_MENU), Sm::Menu(VAR_EFFECTS, &THEME_MENU), Sm::Menu(VAR_CONNECTORS, &THEME_MENU)]),
        Col::Small(&[Sm::Menu(VAR_FONTS, &THEME_MENU), Sm::Menu(VAR_EMBELLISHMENT, &THEME_MENU)]),
    ]),
    Group("Layout", LAYOUT_ROUTING, &[Col::BigMenu(RELAYOUT, &RELAYOUT_MENU), Col::BigMenu(CONNECTORS, &CONN_MENU)]),
];

// (JD21 layout) Seitenlayout neu anordnen: Visio's gallery as rows (a live preview while one is
// pointed at, layout_cmds::row_event), then the dialog
static RELAYOUT_MENU: [It; 19] = [
    C(LAYOUT_FLOW_DOWN),
    C(LAYOUT_FLOW_UP),
    C(LAYOUT_FLOW_RIGHT),
    C(LAYOUT_FLOW_LEFT),
    Sep,
    C(LAYOUT_HIER_DOWN),
    C(LAYOUT_HIER_UP),
    C(LAYOUT_HIER_RIGHT),
    C(LAYOUT_HIER_LEFT),
    Sep,
    C(LAYOUT_TREE_DOWN_RIGHT),
    C(LAYOUT_TREE_RIGHT_DOWN),
    C(LAYOUT_TREE_DOWN_LEFT),
    C(LAYOUT_TREE_LEFT_DOWN),
    Sep,
    C(LAYOUT_RADIAL),
    C(LAYOUT_CIRCULAR),
    Sep,
    C(LAYOUT_MORE),
];

// (JD18) Daten, as Visio's register (FEATURES 3.7); the data graphics wait for JD19
static REFRESH_MENU: [It; 4] = [C(DATA_REFRESH_ALL), C(DATA_REFRESH), Sep, C(DATA_AUTO_LINK_REFRESH)];
// i18n
static DATA: [Group; 5] = [
    Group("Externe Daten", 0, &[Col::Big(DATA_QUICK_IMPORT), Col::Big(DATA_CUSTOM_IMPORT), Col::BigSplit(DATA_REFRESH_ALL, &REFRESH_MENU), Col::Big(DATA_TEXT_DRAWING)]),
    Group("Datengrafiken", 0, &[Col::Big(DATA_GRAPHICS), Col::Small(&[Sm::Btn(DG_POSITION), Sm::Btn(DG_CONFIGURE), Sm::Btn(DG_AUTO_SPACE)])]),
    Group("Daten anzeigen", 0, &[Col::Big(DATA_LEGEND)]),
    Group("Einblenden/Ausblenden", 0, &[Col::Small(&[Sm::Check(PANE_SHAPE_DATA_WINDOW), Sm::Check(PANE_EXTERNAL_DATA), Sm::Check(PANE_DG_FIELDS)])]),
    Group("Erweiterte Datenverknüpfung", 0, &[Col::Small(&[Sm::Btn(DATA_LINK), Sm::Btn(DATA_ADVANCED_DG)])]),
];

// i18n
static VIEW: [Group; 3] = [
    Group("Anzeigen", 0, &[
        Col::Small(&[Sm::Check(RULER), Sm::Check(PAGE_BREAKS)]),
        Col::Small(&[Sm::Check(GRID), Sm::Check(GUIDES)]),
        Col::BigMenu(TASK_PANES, &PANES_MENU),
    ]),
    Group("Zoom", 0, &[Col::Big(ZOOM), Col::Small(&[Sm::Btn(FIT_WINDOW), Sm::Btn(PAGE_WIDTH)])]),
    Group("Visuelle Unterstützung", SNAP_GLUE, &[Col::Small(&[Sm::Check(DYN_GRID), Sm::Check(CONNECT_ARROWS), Sm::Check(CONN_POINTS)])]),
];

// i18n
static CONTAINER_FORMAT: [Group; 3] = [
    Group("Größe", 0, &[Col::BigMenu(CT_MARGINS, &MARGIN_MENU), Col::Big(CT_FIT), Col::BigMenu(CT_AUTOSIZE, &CT_AUTOSIZE_MENU)]),
    Group("Containerformatvorlagen", 0, &[Col::BigMenu(CT_STYLES, &CT_STYLE_MENU), Col::BigMenu(CT_HEADING, &CT_HEADING_MENU)]),
    Group("Mitgliedschaft", 0, &[Col::Big(CT_LOCK), Col::Big(CT_SELECT), Col::Big(CT_DISBAND)]),
];

// i18n
static TEXT_TOOLS: [Group; 3] = [
    Group("Schriftart", TEXT_DIALOG, &[Col::Fonts]),
    Group("Absatz", TEXT_DIALOG, &[Col::Small(&[
        Sm::Row(&[VALIGN_TOP, VALIGN_MIDDLE, VALIGN_BOTTOM]),
        Sm::Row(&[TEXT_LEFT, TEXT_CENTER, TEXT_RIGHT, TEXT_JUSTIFY]),
    ])]),
    Group("Einfügen", 0, &[Col::Big(SYMBOL), Col::Big(FIELD), Col::Big(LINK), Col::Big(SCREEN_TIP)]),
];


const SPLIT: u32 = 0x8000;
const LAUNCH: u32 = 0x2_0000;
const TILE: u32 = 0x3_0000;
const ID_RIBBON: u32 = 200;
const ID_GROUP: u32 = 1000;
const CONTENT_H: i32 = 66; // the netlab ribbon's control height (three 22 DIP rows)
const SMALL_H: i32 = 22;
const ROW_W: i32 = 24; // an icon toggle of a row
const FONT_W: i32 = 132; // the font box
const FONT_W_COMPACT: i32 = 96;
const SIZE_W: i32 = 56; // the size box
const ICON_W: i32 = 28; // a small button showing its icon only
const TILE_W: i32 = 56;

// i18n
static CFF: [Group; 3] = [
    Group("Einfügen", 0, &[Col::Big(CFF_LANE), Col::Big(CFF_SEPARATOR)]),
    Group("Anordnen", 0, &[Col::Small(&[Sm::Menu(CFF_ORIENTATION, &CFF_ORIENT_MENU)])]),
    Group("Entwurf", 0, &[Col::Small(&[Sm::Check(CFF_TITLE), Sm::Check(CFF_SEPARATORS), Sm::Check(CFF_ROTATE)])]),
];

fn groups(reg: Reg) -> &'static [Group] {
    match reg {
        Reg::Cff => &CFF,
        Reg::Insert => &INSERT,
        Reg::Design => &DESIGN,
        Reg::Data => &DATA,
        Reg::View => &VIEW,
        Reg::Container => &CONTAINER_FORMAT,
        Reg::Text => &TEXT_TOOLS,
        _ => &START,
    }
}

// (JD8 rest) the registers with their own ribbon, in the row's order
const RIBBONS: [Reg; 8] = [Reg::Start, Reg::Insert, Reg::Design, Reg::Data, Reg::View, Reg::Cff, Reg::Container, Reg::Text];

// the menu of the button whose command is `id`: its register, its popup id, its rows
fn menu_of(id: u32) -> Option<(Reg, u32, &'static [It])> {
    for reg in RIBBONS {
        for g in groups(reg) {
            for col in g.2 {
                match col {
                    Col::BigMenu(c, items) if *c == id => return Some((reg, id, items)),
                    Col::BigSplit(c, items) if *c == id => return Some((reg, id | SPLIT, items)),
                    Col::Small(sms) => {
                        for s in *sms {
                            match s {
                                Sm::Menu(c, items) if *c == id => return Some((reg, id, items)),
                                Sm::Split(c, items) if *c == id => return Some((reg, id | SPLIT, items)),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// (JD8 rest) where the menu of the button whose command is `id` opens: its register and the
/// popup's id (Designs: the gallery's list of all designs)
pub(crate) fn menu_place(id: u32) -> Option<(Reg, u32)> {
    if id == THEMES {
        return Some((Reg::Design, crate::design_gallery::ID_ALL));
    }
    menu_of(id).map(|(reg, pid, _)| (reg, pid))
}

/// (JD8 rest) a menu button's own command: usable while one of its rows is (None: `id` opens
/// no menu of the ribbon)
pub(crate) fn menu_usable(app: &Diagrammer, id: u32) -> Option<bool> {
    let (_, _, items) = menu_of(id)?;
    // a split button's action is a command of its own (Neue Seite, Zeichentools)
    // (JD18) Alle aktualisieren is a row of its own menu: asking the rows would ask it again
    if matches!(id, NEW_PAGE | DRAW_TOOLS | PASTE | TEXT_BOX | BRING_FRONT | SEND_BACK | FONT_COLOR | DATA_REFRESH_ALL) {
        return None;
    }
    Some(any_enabled(app, items))
}

// the flags of a command's control: disabled without a handler, checked when on
fn cflags(app: &Diagrammer, id: u32) -> u32 {
    (if commands::usable(app, id) { 0 } else { ST_DISABLED }) | if commands::checked(app, id) { ST_CHECKED } else { 0 }
}

// a menu's rows lead somewhere: one of them (or of a submenu) has a handler
fn any_enabled(app: &Diagrammer, items: &[It]) -> bool {
    items.iter().any(|i| match i {
        C(id) => commands::usable(app, *id),
        Sub(_, sub) => any_enabled(app, sub),
        Sep => false,
    })
}

// the flags of a menu or split button: enabled when its command or one of its rows is
fn mflags(app: &Diagrammer, id: u32, items: &[It]) -> u32 {
    let f = cflags(app, id);
    if any_enabled(app, items) { f & !ST_DISABLED } else { f }
}

// the icon of a command in the current language (the text formats' letters)
fn icon(id: u32) -> u32 {
    lang::letter_icon(id, ui::i18n::language() as usize)
}

fn large_icon(id: u32) -> u32 {
    commands::def(id).map_or(0, |d| if d.large != 0 { d.large } else { d.icon })
}

impl Col {
    // the column's width in DIP in the current language; compact: small buttons show their
    // icons only (a narrow window, before groups fold into a menu)
    fn width(&self, ui: &mut Ui, compact: bool) -> i32 {
        match self {
            Col::Big(id) => {
                let l = commands::label(*id);
                ui.ribbon_button_width(l, large_icon(*id), ST_LARGE).max(44)
            }
            Col::BigMenu(id, _) => {
                let l = commands::label(*id);
                ui.ribbon_button_width(l, large_icon(*id), ST_LARGE | ST_MENU).max(44)
            }
            Col::BigSplit(id, _) => {
                let l = commands::label(*id);
                ui.ribbon_button_width(l, large_icon(*id), ST_LARGE | ST_MENU).max(44)
            }
            Col::Small(items) if compact && items.iter().all(|s| matches!(s, Sm::Btn(_) | Sm::Menu(..) | Sm::Split(..))) => {
                items.iter().fold(0, |w, s| w.max(if matches!(s, Sm::Btn(_)) { ICON_W } else { ICON_W + 16 }))
            }
            Col::Small(items) => items.iter().fold(0, |w, s| {
                w.max(match s {
                    Sm::Btn(id) => ui.ribbon_button_width(commands::label(*id), icon(*id), 0),
                    Sm::Menu(id, _) | Sm::Split(id, _) => ui.ribbon_button_width(commands::label(*id), icon(*id), ST_MENU) + if matches!(s, Sm::Split(..)) { 4 } else { 0 },
                    Sm::Check(id) => {
                        ui.ts_font(TS_BODY);
                        let w = ui.text.width(commands::label(*id));
                        ui.px2dip(w) + 30
                    }
                    Sm::Row(ids) => ids.len() as i32 * ROW_W,
                })
            }),
            Col::Gallery(_, n, _) => *n as i32 * (TILE_W + 2) + 18,
            Col::Fonts if compact => FONT_W_COMPACT + 2 + SIZE_W,
            Col::Fonts => FONT_W + 2 + SIZE_W,
        }
    }
}

/// the ribbon of register `reg`
pub(crate) fn draw(app: &mut Diagrammer, ui: &mut Ui, reg: Reg) {
    let groups = groups(reg);
    let rno = reg as u32;
    let widths: Vec<Vec<i32>> = groups.iter().map(|g| g.2.iter().map(|c| c.width(ui, false)).collect()).collect();
    let narrow: Vec<Vec<i32>> = groups.iter().map(|g| g.2.iter().map(|c| c.width(ui, true)).collect()).collect();
    let sum = |w: &Vec<i32>| w.iter().sum::<i32>() + 2 * w.len().saturating_sub(1) as i32;
    let desc: Vec<RibbonGroup> = groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let label = t(g.0);
            let launcher = if g.1 != 0 { 20 } else { 0 };
            let min = ui.ribbon_button_width(label, 0, 0) + launcher;
            let width = sum(&widths[i]).max(min);
            RibbonGroup::new(ID_GROUP + rno * 100 + i as u32 * 10, label, width, sum(&narrow[i]).max(min))
        })
        .collect();
    let layout = ui.ribbon_begin(ID_RIBBON, &desc);
    for (i, d) in desc.iter().enumerate() {
        let Some(mode) = ui.ribbon_group_begin(&layout, i, d) else { continue };
        let compact = mode.width() < d.width;
        ui.next_size(-1, CONTENT_H);
        ui.box_begin(0, BOX_H | BOX_FIXED, 2);
        let ws = if compact { &narrow[i] } else { &widths[i] };
        for (col, &width) in groups[i].2.iter().zip(ws) {
            column(app, ui, col, width, rno * 64 + i as u32 * 8, compact);
        }
        ui.box_end();
        let g = &groups[i];
        if g.1 != 0 {
            let name = alloc::format!("{}: {}", commands::label(g.1), d.label);
            let f = if commands::usable(app, g.1) { 0 } else { ST_DISABLED };
            if ui.ribbon_group_end_launcher(mode, d.label, LAUNCH + rno * 32 + i as u32, &name, f) & EV_ACT != 0 {
                ui.cmd_run(g.1, 0);
            }
            ui.tooltip(&commands::tip(ui, g.1));
        } else {
            ui.ribbon_group_end(mode, d.label);
        }
    }
    ui.ribbon_end();
}

// one column of a group
fn column(app: &mut Diagrammer, ui: &mut Ui, col: &Col, width: i32, tiles: u32, compact: bool) {
    match col {
        Col::Big(id) => {
            let l = commands::label(*id);
            ui.next_size(width, CONTENT_H);
            if ui.ribbon_button(*id, l, large_icon(*id), ST_LARGE | cflags(app, *id)) & EV_ACT != 0 {
                ui.cmd_run(*id, 0);
            }
            ui.tooltip(&commands::tip(ui, *id));
        }
        Col::BigMenu(id, items) => {
            let l = commands::label(*id);
            ui.next_size(width, CONTENT_H);
            ui.ribbon_menu_button(*id, l, large_icon(*id), ST_LARGE | mflags(app, *id, items));
            ui.tooltip(&commands::tip(ui, *id));
            menu(app, ui, *id, items);
        }
        Col::BigSplit(id, items) => {
            let l = commands::label(*id);
            ui.next_size(width, CONTENT_H);
            let label = commands::label(*id);
            let more = alloc::format!("{}: {}", label, t("Weitere Optionen"));
            let ev = ui.ribbon_stacked_split_button(*id, *id | SPLIT, l, &more, large_icon(*id), mflags(app, *id, items));
            if ev & EV_ACT != 0 && commands::usable(app, *id) {
                ui.cmd_run(*id, 0);
            }
            ui.tooltip(&commands::tip(ui, *id));
            menu(app, ui, *id | SPLIT, items);
        }
        Col::Small(items) => {
            ui.next_size(width, CONTENT_H);
            ui.box_begin(0, BOX_V | BOX_FIXED, 0);
            for s in *items {
                small(app, ui, s, compact);
            }
            ui.box_end();
        }
        // (JD11) the design galleries
        Col::Gallery(THEMES, ..) => {
            ui.next_size(width, CONTENT_H);
            crate::design_gallery::designs(app, ui, CONTENT_H);
        }
        Col::Gallery(VAR_COLORS, ..) => {
            ui.next_size(width, CONTENT_H);
            crate::design_gallery::variants(app, ui, CONTENT_H);
        }
        Col::Gallery(id, n, color) => {
            ui.next_size(width, CONTENT_H);
            ui.box_begin(0, BOX_H | BOX_FIXED, 2);
            for k in 0..*n {
                ui.next_size(TILE_W, CONTENT_H);
                ui.next_flags(ST_DISABLED);
                let name = alloc::format!("{} {}", commands::label(*id), k + 1);
                ui.ribbon_theme_tile(TILE + tiles + k, &name, *color, k);
                ui.tooltip(&alloc::format!("{} · {}", name, t("Vorschauen folgen mit den Designs")));
            }
            ui.box_end();
        }
        Col::Fonts => {
            ui.next_size(width, CONTENT_H);
            ui.box_begin(0, BOX_V | BOX_FIXED, 0);
            ui.next_size(-1, SMALL_H + 2);
            ui.box_begin(0, BOX_H | BOX_FIXED, 2);
            // (JD11) the fonts of the design first, the size
            crate::format_menus::font_boxes(app, ui, if compact { FONT_W_COMPACT } else { FONT_W }, SIZE_W, SMALL_H);
            ui.box_end();
            small(app, ui, &Sm::Row(&[BOLD, ITALIC, UNDERLINE, STRIKE]), false);
            small(app, ui, &Sm::Split(FONT_COLOR, &COLOR_MENU), compact);
            ui.box_end();
        }
    }
}

// a small control of a column
fn small(app: &mut Diagrammer, ui: &mut Ui, s: &Sm, compact: bool) {
    let only = if compact { ST_ICON_ONLY } else { ST_ALIGN_START };
    match s {
        Sm::Btn(id) => {
            ui.next_size(-1, SMALL_H);
            let ev = ui.ribbon_button(*id, commands::label(*id), icon(*id), only | cflags(app, *id));
            // (JD11) a double click keeps Format übertragen on
            if ev & EV_DOUBLE != 0 && *id == FORMAT_PAINTER {
                app.format.double = true;
            }
            if ev & EV_ACT != 0 {
                ui.cmd_run(*id, 0);
            }
            ui.tooltip(&commands::tip(ui, *id));
        }
        Sm::Menu(id, items) => {
            ui.next_size(-1, SMALL_H);
            ui.ribbon_menu_button(*id, commands::label(*id), icon(*id), only | mflags(app, *id, items));
            ui.tooltip(&commands::tip(ui, *id));
            menu(app, ui, *id, items);
        }
        Sm::Split(id, items) => {
            // a small split button: the action and an arrow beside it
            ui.next_size(-1, SMALL_H);
            ui.box_begin(0, BOX_H | BOX_FIXED, 0);
            let label = commands::label(*id);
            ui.next_size(-1, SMALL_H);
            // (JD8 rest) Zeichentools shows the drawing tool it repeats (cmds_draw.rs)
            let ic = if *id == DRAW_TOOLS { crate::cmds_draw::split_icon(app) } else { icon(*id) };
            if ui.ribbon_button(*id, label, ic, only | cflags(app, *id)) & EV_ACT != 0 {
                ui.cmd_run(*id, 0);
            }
            let tip = if *id == DRAW_TOOLS { crate::cmds_draw::split_tip(app, ui) } else { commands::tip(ui, *id) };
            ui.tooltip(&tip);
            ui.next_size(16, SMALL_H);
            let more = alloc::format!("{}: {}", label, t("Weitere Optionen"));
            ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY | (mflags(app, *id, items) & ST_DISABLED));
            ui.dropdown_button(*id | SPLIT, &more);
            ui.keytip(2, &more);
            ui.box_end();
            menu(app, ui, *id | SPLIT, items);
        }
        Sm::Check(id) => {
            let mut v = commands::checked(app, *id) as i32;
            ui.next_size(-1, SMALL_H);
            ui.next_flags(if commands::usable(app, *id) { 0 } else { ST_DISABLED });
            if ui.checkbox(*id, commands::label(*id), &mut v) & EV_ACT != 0 {
                ui.cmd_run(*id, 0);
            }
            ui.tooltip(&commands::tip(ui, *id));
            ui.keytip(2, commands::label(*id));
        }
        Sm::Row(ids) => {
            ui.next_size(-1, SMALL_H);
            ui.box_begin(0, BOX_H | BOX_FIXED, 0);
            for id in *ids {
                ui.next_size(ROW_W, SMALL_H);
                if ui.ribbon_button(*id, commands::label(*id), icon(*id), ST_ICON_ONLY | cflags(app, *id)) & EV_ACT != 0 {
                    ui.cmd_run(*id, 0);
                }
                ui.tooltip(&commands::tip(ui, *id));
            }
            ui.box_end();
        }
    }
}

// the rows of a menu button's flyout `pid`
fn menu(app: &mut Diagrammer, ui: &mut Ui, pid: u32, items: &[It]) {
    // (JD11) Füllung, Linie and Schriftfarbe bring their colour menus
    if crate::format_menus::flyout(app, ui, pid) {
        return;
    }
    if ui.flyout_begin(pid, 0) {
        rows(app, ui, items);
    }
    ui.flyout_end();
}

fn rows(app: &mut Diagrammer, ui: &mut Ui, items: &[It]) {
    for it in items {
        match it {
            C(id) => {
                let ic = icon(*id);
                if ic != 0 {
                    ui.next_icon(ic);
                }
                ui.next_flags(cflags(app, *id));
                // (JD8 rest: pages) a paper size shows its measures where shortcuts stand
                let keys = crate::page_size::size_note(*id).unwrap_or_else(|| shortcut(ui, *id));
                let ev = ui.menu_item_keys(*id, commands::label(*id), &keys);
                // (JD21 layout) a layout's live preview while its row is pointed at
                crate::layout_cmds::row_event(app, ui, *id, ev);
                if ev & EV_ACT != 0 {
                    ui.cmd_run(*id, 0);
                }
            }
            Sep => {
                ui.menu_sep();
            }
            Sub(label, sub) => {
                // a submenu: its rows below a caption (flyouts nest one level here)
                ui.menu_sep();
                ui.text(TS_CAPTION | TX_SECONDARY, t(label));
                rows(app, ui, sub);
            }
        }
    }
}

// the shortcut of a command in the current language ("" = none)
fn shortcut(ui: &Ui, id: u32) -> String {
    ui.desc
        .cmds
        .iter()
        .take_while(|c| c.id != 0)
        .find(|c| c.id == id && c.key != 0)
        .map_or(String::new(), |c| ui.key_text(c.key as u32, c.mods as u32))
}

// commands -- every command of the window: its id, label and icons, its handler (once the
// function behind it exists), its state, and the library's command table in the current
// language with that language's shortcuts (lang/shortcuts.tsv)
//
//   DEFS                      the commands (id, label, 16 px icon, 32 px icon); the constant's
//                             name (BOLD, NEW_PAGE ...) is the command's key in shortcuts.tsv
//                             and in the [keys] section of the settings (own shortcuts)
//   HANDLERS                  the commands that do something today; a later stage adds a line
//                             (id, fn) here and the ribbon, the menus, the shortcuts and the
//                             quick access toolbar enable it everywhere
//   CHECKED                   toggles: whether the command shows as checked
//   run(app, ui, id)          App::command: the handler, if any
//   states(app, ui)           every pass: CF_DISABLED for commands without a handler,
//                             CF_CHECKED for toggles that are on
//   table(lang, ini)          the library's command table for language `lang` (a catalogue
//                             column): labels in that language, the Office shortcuts of the
//                             language, own shortcuts from the settings on top; made once per
//                             language and set of own shortcuts
//   tip(ui, id)               the QuickInfo: label, shortcut, "not available yet"
//   label(id)                 the label in the current language
//
// Commands without a handler are shown everywhere (ribbon, menus, shortcuts), disabled, with
// the QuickInfo "Noch nicht verfügbar"; the handler table is the only place that changes when
// a function arrives (PLAN.md JD8: "Befehle werden angeschlossen, sobald die Funktionen dahinter
// fertig sind").  Labels with '|' carry a context for German words with two meanings
// ("Linie|Format" is the line format, "Linie|Zeichentool" the drawing tool); the screen shows
// the part before it (tc).

use alloc::string::String;
use alloc::vec::Vec;

use base::Ini;
use ui::i18n;
use ui::*;

use crate::icons::*;
use crate::{tc, Diagrammer};

/// a command of the window
pub(crate) struct Def {
    pub id: u32,
    /// the constant's name: the key of shortcuts.tsv and of own shortcuts
    pub name: &'static str,
    /// the German label (catalogue key, may carry a context after '|')
    pub label: &'static str,
    /// 16 px icon (0 = none)
    pub icon: u32,
    /// 32 px icon for large buttons (0 = none)
    pub large: u32,
}

macro_rules! commands {
    ($($c:ident = $id:literal, $label:literal, $icon:expr, $large:expr;)*) => {
        // some are reached only by name (shortcuts.tsv) until a later stage draws them
        $(#[allow(dead_code)] pub(crate) const $c: u32 = $id;)*
        pub(crate) static DEFS: &[Def] = &[$(Def { id: $id, name: stringify!($c), label: $label, icon: $icon, large: $large },)*];
    };
}

// i18n
commands! {
    // the File register, the quick access toolbar
    NEW = 0x1001, "Neu", JD_NEW, JD_NEW_L;
    OPEN = 0x1002, "Öffnen", JD_OPEN, JD_OPEN_L;
    SAVE = 0x1003, "Speichern", JD_SAVE, JD_SAVE_L;
    SAVE_AS = 0x1004, "Speichern unter", 0, JD_SAVE_AS_L;
    PRINT = 0x1005, "Drucken", JD_PRINT, JD_PRINT_L;
    EXPORT = 0x1006, "Exportieren", 0, JD_EXPORT_L;
    CLOSE = 0x1007, "Schließen", 0, JD_CLOSE_L;
    OPTIONS = 0x1008, "Optionen", 0, JD_OPTIONS_L;
    INFO = 0x1009, "Info", 0, JD_INFO_L;
    UNDO = 0x1010, "Rückgängig", JD_UNDO, 0;
    REDO = 0x1011, "Wiederholen", JD_REDO, 0;
    QAT_CUSTOMIZE = 0x1012, "Symbolleiste für den Schnellzugriff anpassen", JD_QAT_CUSTOMIZE, 0;
    // Start: Zwischenablage
    PASTE = 0x1100, "Einfügen|Zwischenablage", JD_PASTE, JD_PASTE_L;
    PASTE_SPECIAL = 0x1101, "Inhalte einfügen", JD_PASTE_SPECIAL, 0;
    PASTE_LINK = 0x1102, "Als Link einfügen", JD_PASTE_LINK, 0;
    DUPLICATE = 0x1103, "Duplizieren", JD_DUPLICATE, 0;
    CUT = 0x1104, "Ausschneiden", JD_CUT, 0;
    COPY = 0x1105, "Kopieren", JD_COPY, 0;
    FORMAT_PAINTER = 0x1106, "Format übertragen", JD_FORMAT_PAINTER, JD_FORMAT_PAINTER_L;
    // Start: Schriftart, Absatz
    FONT = 0x1110, "Schriftart", 0, 0;
    FONT_SIZE = 0x1111, "Schriftgrad", 0, 0;
    BOLD = 0x1112, "Fett", JD_BOLD_F, 0;
    ITALIC = 0x1113, "Kursiv", JD_ITALIC_K, 0;
    UNDERLINE = 0x1114, "Unterstrichen", JD_UNDERLINE_U, 0;
    STRIKE = 0x1115, "Durchgestrichen", JD_STRIKETHROUGH, 0;
    FONT_COLOR = 0x1116, "Schriftfarbe", JD_FONT_COLOR, 0;
    TEXT_DIALOG = 0x1117, "Text|Dialogfeld", 0, 0;
    VALIGN_TOP = 0x1120, "Oben ausrichten|Text", JD_VALIGN_TOP, 0;
    VALIGN_MIDDLE = 0x1121, "Mittig ausrichten|Text", JD_VALIGN_MIDDLE, 0;
    VALIGN_BOTTOM = 0x1122, "Unten ausrichten|Text", JD_VALIGN_BOTTOM, 0;
    TEXT_LEFT = 0x1123, "Linksbündig", JD_TEXT_LEFT, 0;
    TEXT_CENTER = 0x1124, "Zentriert", JD_TEXT_CENTER, 0;
    TEXT_RIGHT = 0x1125, "Rechtsbündig", JD_TEXT_RIGHT, 0;
    TEXT_JUSTIFY = 0x1126, "Blocksatz", JD_TEXT_JUSTIFY, 0;
    // Start: Tools
    POINTER = 0x1130, "Zeigertool", JD_POINTER, JD_POINTER_L;
    CONNECTOR_TOOL = 0x1131, "Verbinder|Tool", JD_CONNECTOR, JD_CONNECTOR_L;
    TEXT_TOOL = 0x1132, "Text|Tool", JD_TEXT, JD_TEXT_L;
    DRAW_TOOLS = 0x1133, "Zeichentools", JD_RECTANGLE, 0;
    RECTANGLE = 0x1134, "Rechteck", JD_RECTANGLE, 0;
    ELLIPSE = 0x1135, "Ellipse", JD_ELLIPSE, 0;
    LINE_TOOL = 0x1136, "Linie|Zeichentool", JD_LINE, 0;
    FREEFORM = 0x1137, "Freihandform", JD_FREEFORM, 0;
    ARC = 0x1138, "Bogen", JD_ARC, 0;
    PENCIL = 0x1139, "Bleistift", JD_PENCIL, 0;
    // Start: Formenarten
    FILL = 0x1140, "Füllung", JD_FILL, 0;
    FILL_NONE = 0x1141, "Keine Füllung", 0, 0;
    FILL_OPTIONS = 0x1142, "Füllungsoptionen", 0, 0;
    LINE_FORMAT = 0x1143, "Linie|Format", JD_LINE_COLOR, 0;
    LINE_NONE = 0x1144, "Keine Linie", 0, 0;
    LINE_WEIGHT = 0x1145, "Stärke", JD_LINE_WEIGHT, 0;
    LINE_DASH = 0x1146, "Striche", JD_LINE_DASH, 0;
    LINE_ARROWS = 0x1147, "Pfeile", JD_LINE_ARROWS, 0;
    LINE_OPTIONS = 0x1148, "Linienoptionen", JD_LINE_OPTIONS, 0;
    MORE_COLORS = 0x1149, "Weitere Farben", JD_MORE_COLORS, 0;
    FORMAT_PANE = 0x114A, "Shape formatieren", 0, 0;
    // (JD11) the quick styles of the design (TH-06)
    QUICK_STYLES = 0x114B, "Schnellformatvorlagen", 0, JD_QUICK_STYLES_L;
    // Start: Anordnen
    ALIGN = 0x1150, "Ausrichten", JD_ALIGN, JD_ALIGN_L;
    AUTO_ALIGN = 0x1151, "Automatisch ausrichten", 0, 0;
    ALIGN_LEFT = 0x1152, "Links ausrichten", JD_ALIGN_LEFT, 0;
    ALIGN_CENTER = 0x1153, "Zentriert ausrichten", JD_ALIGN_CENTER, 0;
    ALIGN_RIGHT = 0x1154, "Rechts ausrichten", JD_ALIGN_RIGHT, 0;
    ALIGN_TOP = 0x1155, "Oben ausrichten|Shapes", JD_ALIGN_TOP, 0;
    ALIGN_MIDDLE = 0x1156, "Mittig ausrichten|Shapes", JD_ALIGN_MIDDLE, 0;
    ALIGN_BOTTOM = 0x1157, "Unten ausrichten|Shapes", JD_ALIGN_BOTTOM, 0;
    POSITION = 0x1158, "Positionieren", JD_POSITION, JD_POSITION_L;
    DISTRIBUTE_H = 0x1159, "Horizontal verteilen", JD_DISTRIBUTE_H, 0;
    DISTRIBUTE_V = 0x115A, "Vertikal verteilen", JD_DISTRIBUTE_V, 0;
    ROTATE_LEFT = 0x115B, "Um 90° nach links drehen", JD_ROTATE_LEFT, 0;
    ROTATE_RIGHT = 0x115C, "Um 90° nach rechts drehen", JD_ROTATE_RIGHT, 0;
    FLIP_H = 0x115D, "Horizontal spiegeln", JD_FLIP_H, 0;
    FLIP_V = 0x115E, "Vertikal spiegeln", JD_FLIP_V, 0;
    BRING_FRONT = 0x1160, "In den Vordergrund", JD_BRING_FRONT, 0;
    BRING_FORWARD = 0x1161, "Eine Ebene nach vorne", JD_BRING_FORWARD, 0;
    SEND_BACK = 0x1162, "In den Hintergrund", JD_SEND_BACK, 0;
    SEND_BACKWARD = 0x1163, "Eine Ebene nach hinten", JD_SEND_BACKWARD, 0;
    GROUP_MENU = 0x1164, "Gruppieren|Menü", JD_GROUP, 0;
    GROUP = 0x1165, "Gruppieren", JD_GROUP, 0;
    UNGROUP = 0x1166, "Gruppierung aufheben", JD_UNGROUP, 0;
    // Start: Bearbeiten
    FIND_MENU = 0x1170, "Suchen|Menü", JD_SEARCH, 0;
    FIND = 0x1171, "Suchen", JD_SEARCH, 0;
    REPLACE = 0x1172, "Ersetzen", JD_REPLACE, 0;
    LAYERS = 0x1173, "Ebenen", JD_LAYER_ASSIGN, 0;
    LAYER_ASSIGN = 0x1174, "Ebene zuweisen", JD_LAYER_ASSIGN, 0;
    LAYER_PROPERTIES = 0x1175, "Ebeneneigenschaften", JD_LAYER_PROPERTIES, 0;
    SELECT_MENU = 0x1176, "Markieren", JD_SELECT, 0;
    SELECT_ALL = 0x1177, "Alles markieren", JD_SELECT_ALL, 0;
    SELECT_BY_TYPE = 0x1178, "Nach Typ auswählen", JD_SELECT_BY_TYPE, 0;
    SELECT_AREA = 0x1179, "Bereich auswählen", JD_SELECT_AREA, 0;
    LASSO = 0x117A, "Lassoauswahl", JD_LASSO, 0;
    // Einfügen
    NEW_PAGE = 0x1200, "Neue Seite", JD_NEW_PAGE, JD_NEW_PAGE_L;
    PAGE_BLANK = 0x1201, "Leere Seite", JD_PAGE_BLANK, 0;
    PAGE_BACKGROUND = 0x1202, "Hintergrundseite", JD_PAGE_BACKGROUND, 0;
    PAGE_DUPLICATE = 0x1203, "Diese Seite duplizieren", JD_PAGE_DUPLICATE, 0;
    PICTURES = 0x1204, "Bilder", JD_PICTURES, JD_PICTURES_L;
    PICTURE_FILE = 0x1205, "Bild aus Datei", JD_PICTURES, 0;
    CONTAINER = 0x1206, "Container", JD_CONTAINER, JD_CONTAINER_L;
    CALLOUT = 0x1207, "Beschriftung", JD_CALLOUT, JD_CALLOUT_L;
    INSERT_CONNECTOR = 0x1208, "Verbinder|Einfügen", JD_CONNECTOR, JD_INSERT_CONNECTOR_L;
    LINK = 0x1209, "Link", JD_LINK, JD_LINK_L;
    TEXT_BOX = 0x120A, "Textfeld", JD_TEXT_BOX_H, JD_TEXT_BOX_L;
    TEXT_BOX_H = 0x120B, "Horizontales Textfeld", JD_TEXT_BOX_H, 0;
    TEXT_BOX_V = 0x120C, "Vertikales Textfeld", JD_TEXT_BOX_V, 0;
    FIELD = 0x120D, "Feld", 0, JD_FIELD_L;
    // Entwurf
    ORIENTATION = 0x1300, "Ausrichtung|Seite", 0, JD_ORIENTATION_L;
    PORTRAIT = 0x1301, "Hochformat", JD_PORTRAIT, 0;
    LANDSCAPE = 0x1302, "Querformat", JD_LANDSCAPE, 0;
    PAGE_SIZE = 0x1303, "Größe|Seite", 0, JD_PAGE_SIZE_L;
    FIT_DRAWING = 0x1304, "Größe an Zeichnung anpassen", JD_FIT_DRAWING, 0;
    MORE_SIZES = 0x1305, "Weitere Seitenformate", JD_MORE_SIZES, 0;
    AUTO_SIZE = 0x1306, "Automatisch anpassen", 0, JD_AUTO_SIZE_L;
    PAGE_SETUP = 0x1307, "Seite einrichten", 0, 0;
    THEMES = 0x1308, "Designs", 0, 0;
    THEME_NEW_SHAPES = 0x1309, "Design auf neue Shapes anwenden", 0, 0;
    VAR_COLORS = 0x130A, "Farben", JD_VAR_COLORS, 0;
    VAR_EFFECTS = 0x130B, "Effekte|Varianten", JD_VAR_EFFECTS, 0;
    VAR_CONNECTORS = 0x130C, "Verbinder|Varianten", JD_VAR_CONNECTORS, 0;
    VAR_FONTS = 0x130D, "Schriftarten", JD_VAR_FONTS, 0;
    VAR_EMBELLISHMENT = 0x130E, "Verzierung", JD_VAR_EMBELLISHMENT, 0;
    CONNECTORS = 0x1310, "Verbinder|Layout", JD_CONNECTOR, JD_CONNECTOR_L;
    CONN_RIGHT_ANGLE = 0x1311, "Rechtwinklig", JD_CONNECTOR_RIGHT_ANGLE, 0;
    CONN_STRAIGHT = 0x1312, "Gerade", JD_CONNECTOR_STRAIGHT, 0;
    CONN_CURVED = 0x1313, "Gebogen", JD_CONNECTOR_CURVED, 0;
    LINE_JUMPS = 0x1314, "Linienübergänge anzeigen", JD_LINE_JUMPS, 0;
    REROUTE = 0x1315, "Verbinder neu routen", 0, 0;
    LAYOUT_ROUTING = 0x1316, "Layout und Routing", 0, 0;
    // Ansicht
    RULER = 0x1400, "Lineal", JD_RULER, 0;
    PAGE_BREAKS = 0x1401, "Seitenumbrüche", 0, 0;
    GRID = 0x1402, "Gitternetz", JD_GRID, 0;
    GUIDES = 0x1403, "Führungslinien", JD_GUIDES, 0;
    TASK_PANES = 0x1404, "Aufgabenbereiche", JD_TASK_PANES, JD_TASK_PANES_L;
    ZOOM = 0x1405, "Zoom", JD_ZOOM, JD_ZOOM_L;
    FIT_WINDOW = 0x1406, "An Fenster anpassen", JD_FIT_WINDOW, 0;
    PAGE_WIDTH = 0x1407, "Seitenbreite", JD_PAGE_WIDTH, 0;
    DYN_GRID = 0x1408, "Dynamisches Gitter", 0, 0;
    CONNECT_ARROWS = 0x1409, "Verbindungspfeile", 0, 0;
    CONN_POINTS = 0x140A, "Verbindungspunkte", JD_CONNECTION_POINT, 0;
    SNAP_GLUE = 0x140B, "Ausrichten und Kleben", JD_SNAP_GLUE, 0;
    ZOOM_IN = 0x140C, "Vergrößern", JD_ZOOM_IN, 0;
    ZOOM_OUT = 0x140D, "Verkleinern", JD_ZOOM_OUT, 0;
    // the task panes (Ansicht › Aufgabenbereiche)
    PANE_SHAPES = 0x1410, "Shapes", JD_STENCIL_MENU, 0;
    PANE_FORMAT = 0x1411, "Shape formatieren|Bereich", 0, 0;
    PANE_SIZEPOS = 0x1412, "Größe und Position", 0, 0;
    PANE_DATA = 0x1413, "Shape-Daten", JD_SHAPE_DATA, 0;
    PANE_PANZOOM = 0x1414, "Verschieben und Zoomen", 0, 0;
    // pages
    PAGE_NEXT = 0x1420, "Nächste Seite", 0, 0;
    PAGE_PREV = 0x1421, "Vorherige Seite", 0, 0;
    PAGES_ALL = 0x1422, "Alle Seiten", JD_PAGES_ALL, 0;
    PAGE_INSERT = 0x1423, "Seite einfügen", JD_PAGE_ADD, 0;
    PAGE_DELETE = 0x1424, "Seite löschen", 0, 0;
    PAGE_RENAME = 0x1425, "Seite umbenennen", 0, 0;
    PAGE_REORDER = 0x1426, "Seiten neu anordnen", 0, 0;
    DRAWING_SCALE = 0x1427, "Zeichnungsmaßstab", 0, 0;
    // text
    EDIT_TEXT = 0x1430, "Text bearbeiten", 0, 0;
    SYMBOL = 0x1431, "Symbol", 0, JD_SYMBOL_L;
    SCREEN_TIP = 0x1432, "QuickInfo", 0, JD_SCREEN_TIP_L;
    // context register Container-Tools › Format
    CT_MARGINS = 0x1500, "Ränder", JD_CT_MARGINS, JD_CT_MARGINS_L;
    CT_FIT = 0x1501, "An Inhalt anpassen", JD_CT_FIT_CONTENTS, JD_CT_FIT_CONTENTS_L;
    CT_AUTOSIZE = 0x1502, "Automatische Größe", JD_CT_AUTO_SIZE, JD_CT_AUTO_SIZE_L;
    CT_STYLES = 0x1503, "Containerformatvorlagen", JD_CONTAINER, JD_CONTAINER_L;
    CT_HEADING = 0x1504, "Kopfzeilenstil", JD_CT_HEADING, JD_CT_HEADING_L;
    CT_LOCK = 0x1505, "Container sperren", JD_CT_LOCK, JD_CT_LOCK_L;
    CT_SELECT = 0x1506, "Inhalt auswählen", JD_CT_SELECT_CONTENTS, JD_CT_SELECT_CONTENTS_L;
    CT_DISBAND = 0x1507, "Container auflösen", JD_CT_DISBAND, JD_CT_DISBAND_L;
    CT_ADD = 0x1508, "Zu Container hinzufügen", 0, 0;
    CT_REMOVE = 0x1509, "Aus Container entfernen", 0, 0;
    // (JD10) the container's styles, headings, margins and automatic size (structures_cmds.rs)
    CT_STYLE_1 = 0x1510, "Klar|Container", JD_CONTAINER, 0;
    CT_STYLE_2 = 0x1511, "Akzent|Container", JD_CONTAINER, 0;
    CT_STYLE_3 = 0x1512, "Gestrichelt|Container", JD_CONTAINER, 0;
    CT_STYLE_4 = 0x1513, "Fläche|Container", JD_CONTAINER, 0;
    CT_STYLE_5 = 0x1514, "Eckwinkel|Container", JD_CONTAINER, 0;
    CT_STYLE_6 = 0x1515, "Abgerundet|Container", JD_CONTAINER, 0;
    CT_HEADING_1 = 0x1516, "Kopfband oben", 0, 0;
    CT_HEADING_2 = 0x1517, "Titel oben", 0, 0;
    CT_HEADING_3 = 0x1518, "Kopfband links", 0, 0;
    CT_HEADING_4 = 0x1519, "Ohne Kopfzeile", 0, 0;
    CT_MARGIN_0 = 0x151A, "Kein Rand", 0, 0;
    CT_MARGIN_1 = 0x151B, "Schmal|Rand", 0, 0;
    CT_MARGIN_2 = 0x151C, "Mittel|Rand", 0, 0;
    CT_MARGIN_3 = 0x151D, "Breit|Rand", 0, 0;
    CT_RESIZE_0 = 0x151E, "Keine automatische Größe", 0, 0;
    CT_RESIZE_1 = 0x151F, "Nach Bedarf erweitern", 0, 0;
    CT_RESIZE_2 = 0x1520, "Immer an Inhalt anpassen", 0, 0;
    // (JD10) Einfügen › Container, Einfügen › Beschriftung
    CONTAINER_1 = 0x1521, "Klar|Container", JD_CONTAINER, 0;
    CONTAINER_2 = 0x1522, "Akzent|Container", JD_CONTAINER, 0;
    CONTAINER_3 = 0x1523, "Gestrichelt|Container", JD_CONTAINER, 0;
    CONTAINER_4 = 0x1524, "Fläche|Container", JD_CONTAINER, 0;
    CONTAINER_5 = 0x1525, "Eckwinkel|Container", JD_CONTAINER, 0;
    CONTAINER_6 = 0x1526, "Abgerundet|Container", JD_CONTAINER, 0;
    CALLOUT_1 = 0x1527, "Rechteck|Beschriftung", JD_CALLOUT, 0;
    CALLOUT_2 = 0x1528, "Abgerundet|Beschriftung", JD_CALLOUT, 0;
    CALLOUT_3 = 0x1529, "Sprechblase", JD_CALLOUT, 0;
    CALLOUT_4 = 0x152A, "Linie|Beschriftung", JD_CALLOUT, 0;
    CALLOUT_5 = 0x152B, "Klammer", JD_CALLOUT, 0;
    CALLOUT_6 = 0x152C, "Kreis|Beschriftung", JD_CALLOUT, 0;
    // (JD10) Shapes verbinden (CN-16), Layout-Verhalten (CN-17)
    CONNECT_SHAPES = 0x152D, "Shapes verbinden", JD_CONNECTOR, 0;
    LAYOUT_BEHAVIOR = 0x152E, "Layout-Verhalten", 0, 0;
    // (JD10) the register Funktionsübergreifendes Flussdiagramm (RK-03, structures_cff.rs)
    CFF_LANE = 0x1530, "Bahn", JD_CFF_SWIMLANE, JD_CFF_SWIMLANE_L;
    CFF_SEPARATOR = 0x1531, "Trennlinie", JD_CFF_SEPARATOR, JD_CFF_SEPARATOR_L;
    CFF_ORIENTATION = 0x1532, "Ausrichtung|Flussdiagramm", JD_CFF_ORIENTATION, 0;
    CFF_HORIZONTAL = 0x1533, "Horizontal", 0, 0;
    CFF_VERTICAL = 0x1534, "Vertikal", 0, 0;
    CFF_TITLE = 0x1535, "Titelleiste anzeigen", JD_CFF_TITLE_BAR, 0;
    CFF_SEPARATORS = 0x1536, "Trennlinien anzeigen", JD_CFF_SEPARATORS, 0;
    CFF_ROTATE = 0x1537, "Bahnbeschriftungen waagerecht", JD_CFF_ROTATE_LABELS, 0;
    // the shape's context menu
    SHAPE_DATA_DEFINE = 0x1600, "Shape-Daten definieren", 0, 0;
    HYPERLINK = 0x1601, "Hyperlink", 0, 0;
    FORMAT_TEXT = 0x1602, "Text formatieren", 0, 0;
    FORMAT_LINE = 0x1603, "Linie formatieren", 0, 0;
    FORMAT_FILL = 0x1604, "Füllung formatieren", 0, 0;
    FORMAT_SHADOW = 0x1605, "Schatten formatieren", 0, 0;
    CONN_RESET = 0x1606, "Verbinder zurücksetzen", 0, 0;
    ADD_TO_MY_SHAPES = 0x1607, "Zu meinen Shapes hinzufügen", 0, 0;
    COMMENT_NEW = 0x1608, "Neuer Kommentar", 0, 0;
    // the stencil's menu (Shapes sidebar)
    STENCIL_CLOSE = 0x1700, "Schablone schließen", 0, 0;
    STENCIL_SAVE = 0x1701, "Schablone speichern", 0, 0;
    STENCIL_EDIT = 0x1702, "Schablone bearbeiten", 0, 0;
    STENCIL_VIEW_ICONS = 0x1703, "Symbole und Namen", 0, 0;
    STENCIL_VIEW_ICONS_ONLY = 0x1704, "Nur Symbole", 0, 0;
    STENCIL_VIEW_NAMES = 0x1705, "Nur Namen", 0, 0;
    STENCIL_NEW_MASTER = 0x1706, "Neues Master-Shape", 0, 0;
    STENCIL_ORDER = 0x1707, "Reihenfolge der Schablonen", 0, 0;
    MORE_SHAPES = 0x1708, "Weitere Shapes", JD_MORE_SHAPES, 0;
    // (JD9) the Shapes panel: view option, "Weitere Shapes" commands
    STENCIL_VIEW_DETAILS = 0x1709, "Symbole und Details", 0, 0;
    STENCIL_OPEN = 0x170A, "Schablone öffnen...", 0, 0;
    STENCIL_NEW_METRIC = 0x170B, "Neue Schablone (metrisch)", 0, 0;
    STENCIL_NEW_US = 0x170C, "Neue Schablone (US-Einheiten)", 0, 0;
    // (JD8 rest: pages) Entwurf › Größe: the paper sizes (page_size.rs)
    SIZE_LETTER = 0x1800, "Letter", 0, 0;
    SIZE_LEGAL = 0x1801, "Legal", 0, 0;
    SIZE_TABLOID = 0x1802, "Tabloid", 0, 0;
    SIZE_EXECUTIVE = 0x1803, "Executive", 0, 0;
    SIZE_A5 = 0x1804, "A5", 0, 0;
    SIZE_A4 = 0x1805, "A4", 0, 0;
    SIZE_A3 = 0x1806, "A3", 0, 0;
    SIZE_B5 = 0x1807, "B5 (JIS)", 0, 0;
    SIZE_B4 = 0x1808, "B4 (JIS)", 0, 0;
}

/// a command's handler: what it does (the function behind it exists)
pub(crate) type Handler = fn(&mut Diagrammer, &mut Ui);

// the commands that work today; later stages add their line here (and to CHECKED)
static HANDLERS: &[(u32, Handler)] = &[
    (NEW, |a, ui| a.open_backstage(ui, crate::backstage::PAGE_NEW)),
    (OPEN, |a, ui| a.open_backstage(ui, crate::backstage::PAGE_OPEN)),
    (PRINT, |a, ui| a.open_backstage(ui, crate::backstage::PAGE_PRINT)),
    (EXPORT, |a, ui| a.open_backstage(ui, crate::backstage::PAGE_EXPORT)),
    (OPTIONS, |a, ui| a.open_backstage(ui, crate::backstage::PAGE_OPTIONS)),
    (INFO, |a, ui| a.open_backstage(ui, crate::backstage::PAGE_INFO)),
    (POINTER, |a, _| a.tool = POINTER),
    (GRID, |a, _| a.view_grid = !a.view_grid),
    (FIT_WINDOW, |a, _| a.canvas.request_fit()),
    (PAGE_WIDTH, |a, _| crate::viewer::page_width(a)),
    (PAGE_NEXT, |a, ui| a.page_step(ui, 1)),
    (PAGE_PREV, |a, ui| a.page_step(ui, -1)),
    (PAGES_ALL, |a, ui| {
        a.pages_menu = true;
        ui.repass();
    }),
    (CLOSE, |a, ui| crate::viewer::close(a, ui)),
    (PAGE_SETUP, |a, ui| crate::print_ui::open_setup(a, ui, 0)),
    (LAYOUT_ROUTING, |a, ui| crate::print_ui::open_setup(a, ui, 4)),
    (ZOOM_IN, |a, _| a.zoom_step(1)),
    (ZOOM_OUT, |a, _| a.zoom_step(-1)),
    (FORMAT_PANE, |a, _| a.panes.show(crate::panes::P_FORMAT)),
    (PANE_SHAPES, |a, _| a.panes.toggle(crate::panes::P_SHAPES)),
    (PANE_FORMAT, |a, _| a.panes.toggle(crate::panes::P_FORMAT)),
    (PANE_SIZEPOS, |a, _| a.panes.toggle(crate::panes::P_SIZEPOS)),
    (PANE_DATA, |a, _| a.panes.toggle(crate::panes::P_DATA)),
    (PANE_PANZOOM, |a, _| a.panes.toggle(crate::panes::P_PANZOOM)),
    // (JD7) editing: files, undo, the clipboard, tools, arrange, group, text, view toggles
    (SAVE, |a, ui| crate::edit_doc::save(a, ui, false)),
    (SAVE_AS, |a, ui| crate::edit_doc::save(a, ui, true)),
    (UNDO, crate::edit_cmds::undo),
    (REDO, crate::edit_cmds::redo),
    (CUT, crate::edit_cmds::cut),
    (COPY, crate::edit_cmds::copy),
    (PASTE, crate::edit_cmds::paste),
    (DUPLICATE, crate::edit_cmds::duplicate),
    (SELECT_ALL, crate::edit_cmds::select_all),
    (CONNECTOR_TOOL, |a, _| crate::edit_cmds::tool(a, CONNECTOR_TOOL)),
    (INSERT_CONNECTOR, |a, _| crate::edit_cmds::tool(a, CONNECTOR_TOOL)),
    (TEXT_TOOL, |a, _| crate::edit_cmds::tool(a, TEXT_TOOL)),
    (TEXT_BOX, |a, _| crate::edit_cmds::tool(a, TEXT_TOOL)),
    (TEXT_BOX_H, |a, _| crate::edit_cmds::tool(a, TEXT_TOOL)),
    (RECTANGLE, |a, _| crate::edit_cmds::tool(a, RECTANGLE)),
    (ELLIPSE, |a, _| crate::edit_cmds::tool(a, ELLIPSE)),
    (LINE_TOOL, |a, _| crate::edit_cmds::tool(a, LINE_TOOL)),
    (PENCIL, |a, _| crate::edit_cmds::tool(a, PENCIL)),
    (BRING_FRONT, |a, ui| crate::edit_cmds::arrange(a, ui, diagram_core::ops::Z::Front)),
    (BRING_FORWARD, |a, ui| crate::edit_cmds::arrange(a, ui, diagram_core::ops::Z::Forward)),
    (SEND_BACK, |a, ui| crate::edit_cmds::arrange(a, ui, diagram_core::ops::Z::Back)),
    (SEND_BACKWARD, |a, ui| crate::edit_cmds::arrange(a, ui, diagram_core::ops::Z::Backward)),
    (GROUP, crate::edit_cmds::group),
    (UNGROUP, crate::edit_cmds::ungroup),
    (ROTATE_LEFT, |a, ui| crate::edit_cmds::rotate90(a, ui, 1.0)),
    (ROTATE_RIGHT, |a, ui| crate::edit_cmds::rotate90(a, ui, -1.0)),
    (FLIP_H, |a, ui| crate::edit_cmds::flip(a, ui, true)),
    (FLIP_V, |a, ui| crate::edit_cmds::flip(a, ui, false)),
    (ALIGN_LEFT, |a, ui| crate::edit_cmds::align(a, ui, 0)),
    (ALIGN_CENTER, |a, ui| crate::edit_cmds::align(a, ui, 1)),
    (ALIGN_RIGHT, |a, ui| crate::edit_cmds::align(a, ui, 2)),
    (ALIGN_TOP, |a, ui| crate::edit_cmds::align(a, ui, 3)),
    (ALIGN_MIDDLE, |a, ui| crate::edit_cmds::align(a, ui, 4)),
    (ALIGN_BOTTOM, |a, ui| crate::edit_cmds::align(a, ui, 5)),
    (DISTRIBUTE_H, |a, ui| crate::edit_cmds::distribute(a, ui, true)),
    (DISTRIBUTE_V, |a, ui| crate::edit_cmds::distribute(a, ui, false)),
    (EDIT_TEXT, crate::edit_cmds::edit_text),
    (RULER, |a, _| a.view_rulers = !a.view_rulers),
    (PAGE_BREAKS, |a, _| a.view_breaks = !a.view_breaks),
    (GUIDES, |a, _| a.view_guides = !a.view_guides),
    (DYN_GRID, |a, _| a.se.dyn_grid = !a.se.dyn_grid),
    (CONN_POINTS, |a, _| a.se.show_points = !a.se.show_points),
    // (JD15, RA-07) the dialog, no silent toggle
    (SNAP_GLUE, crate::dlg_snap::open),
    (CONN_RIGHT_ANGLE, |a, ui| crate::edit_cmds::conn_style(a, ui, diagram_core::glue::Style::RightAngle)),
    (CONN_STRAIGHT, |a, ui| crate::edit_cmds::conn_style(a, ui, diagram_core::glue::Style::Straight)),
    (CONN_CURVED, |a, ui| crate::edit_cmds::conn_style(a, ui, diagram_core::glue::Style::Curved)),
    (CONN_RESET, crate::edit_cmds::conn_reset),
    (LASSO, |a, _| {
        a.se.lasso = true;
        a.tool = POINTER;
    }),
    (SELECT_AREA, |a, _| {
        a.se.lasso = false;
        a.tool = POINTER;
    }),
    (REROUTE, crate::edit_cmds::conn_reset),
    // (JD9) the Shapes panel (stencils.rs, stencils_panel.rs)
    (MORE_SHAPES, |a, _| a.stencils.more_menu = true),
    (STENCIL_OPEN, |a, _| a.stencils.open_dlg.set(1)),
    (STENCIL_NEW_METRIC, |a, ui| crate::stencils::open_new(a, ui, diagram_io::vsdx::Measure::Metric)),
    (STENCIL_NEW_US, |a, ui| crate::stencils::open_new(a, ui, diagram_io::vsdx::Measure::Us)),
    (STENCIL_CLOSE, |a, ui| {
        if let Some(k) = crate::stencils_panel::target_stencil(a) {
            crate::stencils::close(a, ui, k);
        }
    }),
    (STENCIL_SAVE, |a, ui| {
        if let Some(k) = crate::stencils_panel::target_stencil(a) {
            crate::stencils::save(a, ui, k);
        }
    }),
    (STENCIL_VIEW_ICONS, |a, _| a.stencils.panel.view = SV_ICONS_NAMES),
    (STENCIL_VIEW_ICONS_ONLY, |a, _| a.stencils.panel.view = SV_ICONS),
    (STENCIL_VIEW_NAMES, |a, _| a.stencils.panel.view = SV_NAMES),
    (STENCIL_VIEW_DETAILS, |a, _| a.stencils.panel.view = SV_DETAILS),
    // (JD11) formats (format_menus.rs, format_pane.rs)
    (FILL_NONE, crate::format_menus::fill_none),
    (LINE_NONE, crate::format_menus::line_none),
    (MORE_COLORS, crate::format_menus::more_colors),
    (FILL_OPTIONS, |a, _| crate::format_menus::pane(a, 0)),
    (LINE_OPTIONS, |a, _| crate::format_menus::pane(a, 0)),
    (FORMAT_SHADOW, |a, _| crate::format_menus::pane(a, 1)),
    (FONT_COLOR, crate::format_menus::font_color),
    (BOLD, crate::format_menus::bold),
    (ITALIC, crate::format_menus::italic),
    (UNDERLINE, crate::format_menus::underline),
    (STRIKE, crate::format_menus::strike),
    (VALIGN_TOP, |a, ui| crate::format_menus::valign(a, ui, 0)),
    (VALIGN_MIDDLE, |a, ui| crate::format_menus::valign(a, ui, 1)),
    (VALIGN_BOTTOM, |a, ui| crate::format_menus::valign(a, ui, 2)),
    (TEXT_LEFT, |a, ui| crate::format_menus::halign(a, ui, 0)),
    (TEXT_CENTER, |a, ui| crate::format_menus::halign(a, ui, 1)),
    (TEXT_RIGHT, |a, ui| crate::format_menus::halign(a, ui, 2)),
    (TEXT_JUSTIFY, |a, ui| crate::format_menus::halign(a, ui, 3)),
    (FORMAT_PAINTER, crate::format_menus::painter),
    (THEME_NEW_SHAPES, crate::design_gallery::toggle_new_shapes),
    // (JD8 rest: shell) a menu button's own command opens its menu; Schrift, Stärke/Striche/Pfeile,
    // Schnellformatvorlagen, Linie/Füllung formatieren, Zoom (cmds_shell.rs)
    (FILL, |a, ui| crate::cmds_shell::open_menu(a, ui, FILL)),
    (LINE_FORMAT, |a, ui| crate::cmds_shell::open_menu(a, ui, LINE_FORMAT)),
    (ALIGN, |a, ui| crate::cmds_shell::open_menu(a, ui, ALIGN)),
    (POSITION, |a, ui| crate::cmds_shell::open_menu(a, ui, POSITION)),
    (GROUP_MENU, |a, ui| crate::cmds_shell::open_menu(a, ui, GROUP_MENU)),
    (FIND_MENU, |a, ui| crate::cmds_shell::open_menu(a, ui, FIND_MENU)),
    (LAYERS, |a, ui| crate::cmds_shell::open_menu(a, ui, LAYERS)),
    (SELECT_MENU, |a, ui| crate::cmds_shell::open_menu(a, ui, SELECT_MENU)),
    (PICTURES, |a, ui| crate::cmds_shell::open_menu(a, ui, PICTURES)),
    (ORIENTATION, |a, ui| crate::cmds_shell::open_menu(a, ui, ORIENTATION)),
    (PAGE_SIZE, |a, ui| crate::cmds_shell::open_menu(a, ui, PAGE_SIZE)),
    (TASK_PANES, |a, ui| crate::cmds_shell::open_menu(a, ui, TASK_PANES)),
    (CONNECTORS, |a, ui| crate::cmds_shell::open_menu(a, ui, CONNECTORS)),
    (CT_MARGINS, |a, ui| crate::cmds_shell::open_menu(a, ui, CT_MARGINS)),
    (CT_AUTOSIZE, |a, ui| crate::cmds_shell::open_menu(a, ui, CT_AUTOSIZE)),
    (CT_STYLES, |a, ui| crate::cmds_shell::open_menu(a, ui, CT_STYLES)),
    (CT_HEADING, |a, ui| crate::cmds_shell::open_menu(a, ui, CT_HEADING)),
    (THEMES, |a, ui| crate::cmds_shell::open_menu(a, ui, THEMES)),
    (FONT, |a, ui| crate::cmds_shell::font_focus(a, ui, FONT)),
    (FONT_SIZE, |a, ui| crate::cmds_shell::font_focus(a, ui, FONT_SIZE)),
    (LINE_WEIGHT, |a, ui| crate::cmds_shell::line_choice(a, ui, LINE_WEIGHT)),
    (LINE_DASH, |a, ui| crate::cmds_shell::line_choice(a, ui, LINE_DASH)),
    (LINE_ARROWS, |a, ui| crate::cmds_shell::line_choice(a, ui, LINE_ARROWS)),
    (QUICK_STYLES, crate::cmds_shell::quick_styles),
    (FORMAT_LINE, |a, _| crate::format_menus::pane(a, 0)),
    (FORMAT_FILL, |a, _| crate::format_menus::pane(a, 0)),
    (ZOOM, crate::cmds_shell::zoom_open),
    (AUTO_ALIGN, crate::cmds_arrange::auto_align),
    (SHAPE_DATA_DEFINE, crate::pane_data::define_open),
    (QAT_CUSTOMIZE, crate::qat::open_menu),
    (ADD_TO_MY_SHAPES, crate::cmds_myshapes::add),
    // (JD8 rest: pages) pages and Seite einrichten (pages.rs, page_size.rs)
    (NEW_PAGE, |a, ui| crate::pages::new_page(a, ui, NEW_PAGE)),
    (PAGE_BLANK, |a, ui| crate::pages::new_page(a, ui, PAGE_BLANK)),
    (PAGE_INSERT, |a, ui| crate::pages::new_page(a, ui, PAGE_INSERT)),
    (PAGE_BACKGROUND, crate::pages::background_page),
    (PAGE_DUPLICATE, crate::pages::duplicate),
    (PAGE_DELETE, crate::pages::delete),
    (PAGE_RENAME, crate::pages::open_rename),
    (PAGE_REORDER, crate::pages::open_reorder),
    (PORTRAIT, |a, ui| crate::page_size::orientation(a, ui, false)),
    (LANDSCAPE, |a, ui| crate::page_size::orientation(a, ui, true)),
    (FIT_DRAWING, crate::page_size::fit_drawing),
    (MORE_SIZES, |a, ui| crate::print_ui::open_setup(a, ui, 1)),
    (AUTO_SIZE, crate::page_size::auto_size),
    (DRAWING_SCALE, |a, ui| crate::print_ui::open_setup(a, ui, 2)),
    (SIZE_LETTER, |a, ui| crate::page_size::size(a, ui, SIZE_LETTER)),
    (SIZE_LEGAL, |a, ui| crate::page_size::size(a, ui, SIZE_LEGAL)),
    (SIZE_TABLOID, |a, ui| crate::page_size::size(a, ui, SIZE_TABLOID)),
    (SIZE_EXECUTIVE, |a, ui| crate::page_size::size(a, ui, SIZE_EXECUTIVE)),
    (SIZE_A5, |a, ui| crate::page_size::size(a, ui, SIZE_A5)),
    (SIZE_A4, |a, ui| crate::page_size::size(a, ui, SIZE_A4)),
    (SIZE_A3, |a, ui| crate::page_size::size(a, ui, SIZE_A3)),
    (SIZE_B5, |a, ui| crate::page_size::size(a, ui, SIZE_B5)),
    (SIZE_B4, |a, ui| crate::page_size::size(a, ui, SIZE_B4)),
    // (JD8 rest: Zeichentools, Bilder, Inhalte einfügen, Ebenen, Nach Typ auswählen)
    (DRAW_TOOLS, crate::cmds_draw::draw_tools),
    (FREEFORM, |a, _| crate::edit_cmds::tool(a, FREEFORM)),
    (ARC, |a, _| crate::edit_cmds::tool(a, ARC)),
    (PICTURE_FILE, crate::cmds_picture::open),
    (PASTE_SPECIAL, crate::cmds_paste::special),
    (PASTE_LINK, crate::cmds_paste::link),
    (LAYER_ASSIGN, crate::layers::open_assign),
    (LAYER_PROPERTIES, crate::layers::open_props),
    (SELECT_BY_TYPE, crate::select_type::open),
    // (JD8 rest: Text, Felder, Links, Suchen -- cmds_text.rs, dlg_*.rs)
    (TEXT_DIALOG, crate::dlg_text::open),
    (FORMAT_TEXT, crate::dlg_text::open),
    (TEXT_BOX_V, |a, _| crate::edit_cmds::tool(a, TEXT_BOX_V)),
    (FIELD, crate::dlg_field::open),
    (LINK, crate::dlg_link::open),
    (HYPERLINK, crate::dlg_link::open),
    (FIND, |a, ui| crate::dlg_find::open(a, ui, false)),
    (REPLACE, |a, ui| crate::dlg_find::open(a, ui, true)),
];

// toggles: whether the command is on
static CHECKED: &[(u32, fn(&Diagrammer) -> bool)] = &[
    (POINTER, |a| a.tool == POINTER),
    (CONNECTOR_TOOL, |a| a.tool == CONNECTOR_TOOL),
    (TEXT_TOOL, |a| a.tool == TEXT_TOOL),
    (RECTANGLE, |a| a.tool == RECTANGLE),
    (ELLIPSE, |a| a.tool == ELLIPSE),
    (LINE_TOOL, |a| a.tool == LINE_TOOL),
    (PENCIL, |a| a.tool == PENCIL),
    (RULER, |a| a.view_rulers),
    (PAGE_BREAKS, |a| a.view_breaks),
    (GUIDES, |a| a.view_guides),
    (DYN_GRID, |a| a.se.dyn_grid),
    (CONN_POINTS, |a| a.se.show_points),
    (LASSO, |a| a.se.lasso),
    (SELECT_AREA, |a| !a.se.lasso),
    (GRID, |a| a.view_grid),
    (PANE_SHAPES, |a| a.panes.visible(crate::panes::P_SHAPES)),
    (PANE_FORMAT, |a| a.panes.visible(crate::panes::P_FORMAT)),
    (PANE_SIZEPOS, |a| a.panes.visible(crate::panes::P_SIZEPOS)),
    (PANE_DATA, |a| a.panes.visible(crate::panes::P_DATA)),
    (PANE_PANZOOM, |a| a.panes.visible(crate::panes::P_PANZOOM)),
    (STENCIL_VIEW_ICONS, |a| a.stencils.panel.view == SV_ICONS_NAMES),
    (STENCIL_VIEW_ICONS_ONLY, |a| a.stencils.panel.view == SV_ICONS),
    (STENCIL_VIEW_NAMES, |a| a.stencils.panel.view == SV_NAMES),
    (STENCIL_VIEW_DETAILS, |a| a.stencils.panel.view == SV_DETAILS),
    // (JD11) the format toggles of the first selected shape
    (BOLD, |a| crate::format_menus::style_on(a, 1)),
    (ITALIC, |a| crate::format_menus::style_on(a, 2)),
    (UNDERLINE, |a| crate::format_menus::style_on(a, 4)),
    (VALIGN_TOP, |a| a.format.valign_now == Some(0)),
    (VALIGN_MIDDLE, |a| a.format.valign_now == Some(1)),
    (VALIGN_BOTTOM, |a| a.format.valign_now == Some(2)),
    (TEXT_LEFT, |a| a.format.halign_now == Some(0)),
    (TEXT_CENTER, |a| a.format.halign_now == Some(1)),
    (TEXT_RIGHT, |a| a.format.halign_now == Some(2)),
    (TEXT_JUSTIFY, |a| a.format.halign_now == Some(3)),
    (FORMAT_PAINTER, |a| a.format.painter.is_some()),
    (THEME_NEW_SHAPES, crate::design_gallery::new_shapes),
    // (JD8 rest: pages) the orientation, the size and the automatic size of the page shown
    (PORTRAIT, |a| crate::page_size::checked(a, PORTRAIT)),
    (LANDSCAPE, |a| crate::page_size::checked(a, LANDSCAPE)),
    (AUTO_SIZE, |a| crate::page_size::checked(a, AUTO_SIZE)),
    (SIZE_LETTER, |a| crate::page_size::checked(a, SIZE_LETTER)),
    (SIZE_LEGAL, |a| crate::page_size::checked(a, SIZE_LEGAL)),
    (SIZE_TABLOID, |a| crate::page_size::checked(a, SIZE_TABLOID)),
    (SIZE_EXECUTIVE, |a| crate::page_size::checked(a, SIZE_EXECUTIVE)),
    (SIZE_A5, |a| crate::page_size::checked(a, SIZE_A5)),
    (SIZE_A4, |a| crate::page_size::checked(a, SIZE_A4)),
    (SIZE_A3, |a| crate::page_size::checked(a, SIZE_A3)),
    (SIZE_B5, |a| crate::page_size::checked(a, SIZE_B5)),
    (SIZE_B4, |a| crate::page_size::checked(a, SIZE_B4)),
    // (JD8 rest: Zeichentools)
    (DRAW_TOOLS, crate::cmds_draw::any_tool),
    (FREEFORM, |a| a.tool == FREEFORM),
    (ARC, |a| a.tool == ARC),
    // (JD8 rest: Text, Felder, Links, Suchen)
    (TEXT_BOX_V, |a| a.tool == TEXT_BOX_V),
];

/// the handler of a command (None: not available yet)
pub(crate) fn handler(id: u32) -> Option<Handler> {
    HANDLERS.iter().find(|h| h.0 == id).map(|h| h.1)
}

/// the command exists and works
pub(crate) fn enabled(id: u32) -> bool {
    handler(id).is_some() || crate::structures_cmds::owns(id) || crate::layout_ui::owns(id)
}

/// (JD7) the command exists and can run now (a selection, something to undo ...)
pub(crate) fn usable(app: &Diagrammer, id: u32) -> bool {
    enabled(id)
        && crate::edit_cmds::available(app, id)
        && crate::format_menus::available(app, id)
        // (JD8 rest) menu buttons, panes, pages, pasting, layers, text
        && crate::cmds_shell::available(app, id)
        && crate::pages::available(app, id)
        && crate::page_size::available(app, id)
        && crate::cmds_paste::available(app, id)
        && crate::layers::available(app, id)
        && crate::cmds_text::available(app, id)
}

/// the toggle is on
pub(crate) fn checked(app: &Diagrammer, id: u32) -> bool {
    if crate::structures_cmds::owns(id) {
        return crate::structures_cmds::checked(app, id);
    }
    CHECKED.iter().find(|c| c.0 == id).is_some_and(|c| (c.1)(app))
}

/// App::command: run the handler -> handled
pub(crate) fn run(app: &mut Diagrammer, ui: &mut Ui, id: u32) -> bool {
    // (JD10) structures and layout run their own commands
    if crate::structures_cmds::owns(id) {
        return crate::structures_cmds::run(app, ui, id);
    }
    if crate::layout_ui::owns(id) {
        return crate::layout_ui::run(app, ui, id);
    }
    match handler(id) {
        Some(h) => {
            h(app, ui);
            ui.repass();
            true
        }
        None => DEFS.iter().any(|d| d.id == id),
    }
}

/// every pass: the commands' states for the menus and the shortcuts
pub(crate) fn states(app: &Diagrammer, ui: &mut Ui) {
    for d in DEFS {
        let on = usable(app, d.id);
        let f = if on { 0 } else { CF_DISABLED } | if checked(app, d.id) { CF_CHECKED } else { 0 };
        ui.cmd_state(d.id, f);
    }
}

/// the definition of a command
pub(crate) fn def(id: u32) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.id == id)
}

/// a command's label in the current language
pub(crate) fn label(id: u32) -> &'static str {
    def(id).map_or("", |d| tc(d.label))
}

/// the QuickInfo of a command: its label, its shortcut, and that it is not there yet
pub(crate) fn tip(ui: &Ui, id: u32) -> String {
    let mut s = String::from(label(id));
    if let Some(c) = ui.desc.cmds.iter().take_while(|c| c.id != 0).find(|c| c.id == id && c.key != 0) {
        s.push_str(" (");
        s.push_str(&ui.key_text(c.key as u32, c.mods as u32));
        s.push(')');
    }
    if !enabled(id) {
        s.push_str(" · ");
        s.push_str(i18n::t("Noch nicht verfügbar"));
    }
    s
}

// ------------------------------------------------------------------ shortcuts

static SHORTCUTS: &str = include_str!("../lang/shortcuts.tsv");

/// the library's command table in language column `lang` (de en es it fr), with own
/// shortcuts from the settings' [keys] section (NAME=Ctrl+Shift+F, "" = none) on top
pub(crate) fn table(lang: usize, own: &Ini) -> &'static [Command] {
    // made once per language and set of own shortcuts
    static CACHE: rt::sync::SpinLock<Vec<(usize, String, &'static [Command])>> = rt::sync::SpinLock::new(Vec::new());
    let key = own_keys(own);
    if let Some(t) = CACHE.lock().iter().find(|c| c.0 == lang && c.1 == key).map(|c| c.2) {
        return t;
    }
    let keys = shortcuts(lang, own);
    let mut v: Vec<Command> = Vec::with_capacity(DEFS.len() + 8);
    for d in DEFS {
        let label = tc(d.label);
        let mine: Vec<&(u32, u32, u32)> = keys.iter().filter(|k| k.0 == d.id).collect();
        let icon = crate::lang::letter_icon(d.id, lang);
        if mine.is_empty() {
            v.push(Command::new(d.id, label, icon, 0, 0, 0));
        }
        for k in mine {
            v.push(Command::new(d.id, label, icon, k.2, k.1, 0));
        }
    }
    v.push(Command::new(0, "", 0, 0, 0, 0));
    let t: &'static [Command] = Vec::leak(v);
    CACHE.lock().push((lang, key, t));
    t
}

// the own shortcuts as one text (the cache key)
fn own_keys(own: &Ini) -> String {
    let mut s = String::new();
    for d in DEFS {
        if let Some(v) = own.get("keys", d.name) {
            s.push_str(d.name);
            s.push('=');
            s.push_str(v);
            s.push('\n');
        }
    }
    s
}

/// (id, vk, mods) of every shortcut in language column `lang`, own ones replacing the table's
pub(crate) fn shortcuts(lang: usize, own: &Ini) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    for line in SHORTCUTS.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        let Some(d) = DEFS.iter().find(|d| d.name == cells[0]) else { continue };
        let cell = match own.get("keys", d.name) {
            Some(v) => v,
            None => cells.get(1 + lang).copied().unwrap_or(""),
        };
        for chord in cell.split('|') {
            if let Some((vk, mods)) = parse_chord(chord) {
                out.push((d.id, vk, mods));
            }
        }
    }
    // own shortcuts of commands the table has no line for
    for d in DEFS {
        if !out.iter().any(|k| k.0 == d.id) {
            if let Some(v) = own.get("keys", d.name) {
                for chord in v.split('|') {
                    if let Some((vk, mods)) = parse_chord(chord) {
                        out.push((d.id, vk, mods));
                    }
                }
            }
        }
    }
    out
}

/// "Ctrl+Shift+F", "Alt+F9", "F12", "Ctrl+PgDn" -> (VK, MOD_*) (English key names, as in
/// shortcuts.tsv; None for "" and anything unknown)
pub(crate) fn parse_chord(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut mods = 0;
    let mut vk = 0;
    for part in s.split('+') {
        match part.trim() {
            "Ctrl" => mods |= MOD_CTRL,
            "Shift" => mods |= MOD_SHIFT,
            "Alt" => mods |= MOD_ALT,
            "PgUp" => vk = VK_PRIOR,
            "PgDn" => vk = VK_NEXT,
            "Del" => vk = VK_DELETE,
            "Ins" => vk = VK_INSERT,
            "Home" => vk = VK_HOME,
            "End" => vk = VK_END,
            "Enter" => vk = VK_RETURN,
            "Left" => vk = VK_LEFT,
            "Right" => vk = VK_RIGHT,
            "Up" => vk = VK_UP,
            "Down" => vk = VK_DOWN,
            "=" => vk = VK_OEM_PLUS,
            "<" => vk = VK_OEM_102,
            ">" => {
                vk = VK_OEM_102;
                mods |= MOD_SHIFT;
            }
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => vk = k.as_bytes()[0].to_ascii_uppercase() as u32,
            k if k.len() > 1 && k.starts_with('F') => {
                let n: u32 = k[1..].parse().ok()?;
                if !(1..=24).contains(&n) {
                    return None;
                }
                vk = VK_F1 + n - 1;
            }
            _ => return None,
        }
    }
    (vk != 0).then_some((vk, mods))
}

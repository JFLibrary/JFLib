// keys -- VK_* key codes (Win32 numbering on every OS) and the IC_* icon code points of the
// style's icon font                       [ASMLIB/inc/keys.inc, ASMLIB/inc/icons.inc (generated)]
#![allow(dead_code)]

pub const VK_BACK: u32 = 0x08;
pub const VK_TAB: u32 = 0x09;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12;
pub const VK_PAUSE: u32 = 0x13;
pub const VK_CAPITAL: u32 = 0x14;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_PRIOR: u32 = 0x21;
pub const VK_NEXT: u32 = 0x22;
pub const VK_END: u32 = 0x23;
pub const VK_HOME: u32 = 0x24;
pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;
pub const VK_SNAPSHOT: u32 = 0x2C;
pub const VK_INSERT: u32 = 0x2D;
pub const VK_DELETE: u32 = 0x2E;
pub const VK_0: u32 = 0x30;
pub const VK_1: u32 = 0x31;
pub const VK_2: u32 = 0x32;
pub const VK_3: u32 = 0x33;
pub const VK_4: u32 = 0x34;
pub const VK_5: u32 = 0x35;
pub const VK_6: u32 = 0x36;
pub const VK_7: u32 = 0x37;
pub const VK_8: u32 = 0x38;
pub const VK_9: u32 = 0x39;
pub const VK_A: u32 = 0x41;
pub const VK_B: u32 = 0x42;
pub const VK_C: u32 = 0x43;
pub const VK_D: u32 = 0x44;
pub const VK_E: u32 = 0x45;
pub const VK_F: u32 = 0x46;
pub const VK_G: u32 = 0x47;
pub const VK_H: u32 = 0x48;
pub const VK_I: u32 = 0x49;
pub const VK_J: u32 = 0x4A;
pub const VK_K: u32 = 0x4B;
pub const VK_L: u32 = 0x4C;
pub const VK_M: u32 = 0x4D;
pub const VK_N: u32 = 0x4E;
pub const VK_O: u32 = 0x4F;
pub const VK_P: u32 = 0x50;
pub const VK_Q: u32 = 0x51;
pub const VK_R: u32 = 0x52;
pub const VK_S: u32 = 0x53;
pub const VK_T: u32 = 0x54;
pub const VK_U: u32 = 0x55;
pub const VK_V: u32 = 0x56;
pub const VK_W: u32 = 0x57;
pub const VK_X: u32 = 0x58;
pub const VK_Y: u32 = 0x59;
pub const VK_Z: u32 = 0x5A;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;
pub const VK_APPS: u32 = 0x5D;
pub const VK_NUMPAD0: u32 = 0x60;
pub const VK_NUMPAD9: u32 = 0x69;
pub const VK_MULTIPLY: u32 = 0x6A;
pub const VK_ADD: u32 = 0x6B;
pub const VK_SEPARATOR: u32 = 0x6C;
pub const VK_SUBTRACT: u32 = 0x6D;
pub const VK_DECIMAL: u32 = 0x6E;
pub const VK_DIVIDE: u32 = 0x6F;
pub const VK_F1: u32 = 0x70;
pub const VK_F2: u32 = 0x71;
pub const VK_F3: u32 = 0x72;
pub const VK_F4: u32 = 0x73;
pub const VK_F5: u32 = 0x74;
pub const VK_F6: u32 = 0x75;
pub const VK_F7: u32 = 0x76;
pub const VK_F8: u32 = 0x77;
pub const VK_F9: u32 = 0x78;
pub const VK_F10: u32 = 0x79;
pub const VK_F11: u32 = 0x7A;
pub const VK_F12: u32 = 0x7B;
pub const VK_NUMLOCK: u32 = 0x90;
pub const VK_SCROLL: u32 = 0x91;
pub const VK_LSHIFT: u32 = 0xA0;
pub const VK_RSHIFT: u32 = 0xA1;
pub const VK_LCONTROL: u32 = 0xA2;
pub const VK_RCONTROL: u32 = 0xA3;
pub const VK_LMENU: u32 = 0xA4;
pub const VK_RMENU: u32 = 0xA5;
pub const VK_OEM_1: u32 = 0xBA;
pub const VK_OEM_PLUS: u32 = 0xBB;
pub const VK_OEM_COMMA: u32 = 0xBC;
pub const VK_OEM_MINUS: u32 = 0xBD;
pub const VK_OEM_PERIOD: u32 = 0xBE;
pub const VK_OEM_2: u32 = 0xBF;
pub const VK_OEM_3: u32 = 0xC0;
pub const VK_OEM_4: u32 = 0xDB;
pub const VK_OEM_5: u32 = 0xDC;
pub const VK_OEM_6: u32 = 0xDD;
pub const VK_OEM_7: u32 = 0xDE;
pub const VK_OEM_102: u32 = 0xE2;

pub const IC_MENU: u32 = 0xE700;
pub const IC_ADD: u32 = 0xE710;
pub const IC_CANCEL: u32 = 0xE711;
pub const IC_MORE: u32 = 0xE712;
pub const IC_SETTINGS: u32 = 0xE713;
pub const IC_CHEV_DOWN: u32 = 0xE70D;
pub const IC_CHEV_UP: u32 = 0xE70E;
pub const IC_CHEV_LEFT: u32 = 0xE76B;
pub const IC_CHEV_RIGHT: u32 = 0xE76C;
pub const IC_FORWARD: u32 = 0xE72A;
pub const IC_BACK: u32 = 0xE72B;
pub const IC_UP: u32 = 0xE74A;
pub const IC_DOWN: u32 = 0xE74B;
pub const IC_EDIT: u32 = 0xE70F;
pub const IC_SEARCH: u32 = 0xE721;
pub const IC_REFRESH: u32 = 0xE72C;
pub const IC_SHARE: u32 = 0xE72D;
pub const IC_LOCK: u32 = 0xE72E;
pub const IC_UNLOCK: u32 = 0xE785;
pub const IC_REMOVE: u32 = 0xE738;
pub const IC_STAR: u32 = 0xE734;
pub const IC_STAR_FILL: u32 = 0xE735;
pub const IC_CHECKBOX: u32 = 0xE739;
pub const IC_CHECKBOX_COMPOSITE: u32 = 0xE73A;
pub const IC_CHECKBOX_FILL: u32 = 0xE73B;
pub const IC_CHECKBOX_INDET: u32 = 0xE73C;
pub const IC_CHECK_MARK: u32 = 0xE73E;
pub const IC_FULL_SCREEN: u32 = 0xE740;
pub const IC_BACK_TO_WINDOW: u32 = 0xE73F;
pub const IC_DELETE: u32 = 0xE74D;
pub const IC_SAVE: u32 = 0xE74E;
pub const IC_PLAY: u32 = 0xE768;
pub const IC_PAUSE: u32 = 0xE769;
pub const IC_STOP: u32 = 0xE71A;
pub const IC_UNDO: u32 = 0xE7A7;
pub const IC_REDO: u32 = 0xE7A6;
pub const IC_WARNING: u32 = 0xE7BA;
pub const IC_ERROR: u32 = 0xE783;
pub const IC_INFO: u32 = 0xE946;
pub const IC_HELP: u32 = 0xE897;
pub const IC_CHROME_CLOSE: u32 = 0xE8BB;
pub const IC_CHROME_MIN: u32 = 0xE921;
pub const IC_CHROME_MAX: u32 = 0xE922;
pub const IC_CHROME_RESTORE: u32 = 0xE923;
pub const IC_COPY: u32 = 0xE8C8;
pub const IC_CUT: u32 = 0xE8C6;
pub const IC_PASTE: u32 = 0xE77F;
pub const IC_OPEN_FILE: u32 = 0xE8E5;
pub const IC_DOCUMENT: u32 = 0xE8A5;
pub const IC_FOLDER: u32 = 0xE8B7;
pub const IC_FOLDER_OPEN: u32 = 0xE838;
pub const IC_NEW_FOLDER: u32 = 0xE8F4;
pub const IC_DOWNLOAD: u32 = 0xE896;
pub const IC_UPLOAD: u32 = 0xE898;
pub const IC_FONT: u32 = 0xE8D2;
pub const IC_BOLD: u32 = 0xE8DD;
pub const IC_ITALIC: u32 = 0xE8DB;
pub const IC_UNDERLINE: u32 = 0xE8DC;
pub const IC_ALIGN_LEFT: u32 = 0xE8E4;
pub const IC_ALIGN_CENTER: u32 = 0xE8E3;
pub const IC_ALIGN_RIGHT: u32 = 0xE8E2;
pub const IC_FONT_SIZE: u32 = 0xE8E9;
pub const IC_FONT_INCREASE: u32 = 0xE8E8;
pub const IC_FONT_DECREASE: u32 = 0xE8E7;
pub const IC_COLOR: u32 = 0xE790;
pub const IC_BRUSH: u32 = 0xE771;
pub const IC_ZOOM_IN: u32 = 0xE8A3;
pub const IC_ZOOM_OUT: u32 = 0xE71F;
pub const IC_SELECT_ALL: u32 = 0xE8B3;
pub const IC_SORT: u32 = 0xE8CB;
pub const IC_CROP: u32 = 0xE7A8;
pub const IC_ROTATE: u32 = 0xE7AD;
pub const IC_ERASER: u32 = 0xE75C;
pub const IC_INKING_TOOL: u32 = 0xE76D;
pub const IC_HIGHLIGHT: u32 = 0xE7E6;
pub const IC_PHOTO: u32 = 0xE91B;
pub const IC_POINTER: u32 = 0xE8B0;
pub const IC_HOME: u32 = 0xE80F;
pub const IC_HISTORY: u32 = 0xE81C;
pub const IC_RECENT: u32 = 0xE823;
pub const IC_CALENDAR: u32 = 0xE787;
pub const IC_CALCULATOR: u32 = 0xE8EF;
pub const IC_CLEAR: u32 = 0xE894;
pub const IC_ACCEPT: u32 = 0xE8FB;
pub const IC_UNKNOWN: u32 = 0xE9CE;
pub const IC_FILTER: u32 = 0xE71C;
pub const IC_BULLETED_LIST: u32 = 0xE8FD;
pub const IC_REPAIR: u32 = 0xE90F;
pub const IC_VIEW: u32 = 0xE890;
pub const IC_POWER: u32 = 0xE7E8;
pub const IC_GLOBE: u32 = 0xE774;
pub const IC_OPEN_IN_NEW_WINDOW: u32 = 0xE8A7;
pub const IC_HEART: u32 = 0xEB51;
pub const IC_HEART_FILL: u32 = 0xEB52;
pub const IC_RECORD: u32 = 0xE7C8;
pub const IC_SWITCH: u32 = 0xE8AB;
pub const IC_VIEW_ALL: u32 = 0xE8A9;
pub const IC_CIRCLE_RING: u32 = 0xEA3A;
pub const IC_CIRCLE_FILL: u32 = 0xEA3B;
pub const IC_SHUFFLE: u32 = 0xE8B1;
pub const IC_EYEDROPPER: u32 = 0xEF3C;
pub const IC_CALLIGRAPHY_PEN: u32 = 0xEDFB;
pub const IC_ZOOM: u32 = 0xE71E;
pub const IC_PENCIL: u32 = 0xEB7E;
pub const IC_CRAYON: u32 = 0xED63;
pub const IC_DROP: u32 = 0xEB42;
pub const IC_TEXT_BOX: u32 = 0xF683;
pub const IC_FONT_COLOR: u32 = 0xE8D3;
pub const IC_STRIKETHROUGH: u32 = 0xEDE0;
pub const IC_SELECT_RECT: u32 = 0xE65C;
pub const IC_LASSO: u32 = 0xF1BE;
pub const IC_SHAPES: u32 = 0xEB91;
pub const IC_DIAMOND: u32 = 0xE18A;
pub const IC_CURVE: u32 = 0xF128;
pub const IC_LINE: u32 = 0xF462;
pub const IC_ROTATE_CW: u32 = 0xE149;
pub const IC_ROTATE_CCW: u32 = 0xED21;
pub const IC_RESIZE: u32 = 0xE93A;
pub const IC_SIZE_GRIP: u32 = 0xE788;
pub const IC_LAYERS: u32 = 0xE81E;
pub const IC_FIT_PAGE: u32 = 0xE9A6;
pub const IC_GRID: u32 = 0xF232;
pub const IC_IMAGE: u32 = 0xEB9F;
pub const IC_PICTURES: u32 = 0xE8B9;
pub const IC_PAGE: u32 = 0xE7C3;
pub const IC_SAVE_AS: u32 = 0xE792;
pub const IC_MOVE: u32 = 0xE7C2;
pub const IC_CONTRAST: u32 = 0xE7A1;
pub const IC_BRIGHTNESS: u32 = 0xE706;
pub const IC_SLIDERS: u32 = 0xE9E9;
pub const IC_HIDE: u32 = 0xED1A;
pub const IC_PRINT: u32 = 0xE749;
pub const IC_RADIO_OFF: u32 = 0xECCA;
pub const IC_RADIO_ON: u32 = 0xECCB;
pub const IC_RADIO_DOT: u32 = 0xECCC;
pub const IC_CHECKBOX_CHECKED_FILL: u32 = 0xE73D;
pub const IC_CARET_LEFT: u32 = 0xEDD9;
pub const IC_CARET_RIGHT: u32 = 0xEDDA;
pub const IC_CARET_UP: u32 = 0xEDDB;
pub const IC_CARET_DOWN: u32 = 0xEDDC;
pub const IC_CHEV_UP_SMALL: u32 = 0xE96D;
pub const IC_CHEV_DOWN_SMALL: u32 = 0xE96E;
pub const IC_CHEV_LEFT_SMALL: u32 = 0xE96F;
pub const IC_CHEV_RIGHT_SMALL: u32 = 0xE970;
pub const IC_CHEV_UP_DOWN: u32 = 0xEC8F;
pub const IC_TOGGLE_OFF: u32 = 0xF19E;
pub const IC_TOGGLE_ON: u32 = 0xF19F;
pub const IC_CANCEL_CIRCLE: u32 = 0xEA39;
pub const IC_ERROR_FILL: u32 = 0xEB90;
pub const IC_CHECK_CIRCLE_FILL: u32 = 0xEC61;
pub const IC_ADD_CIRCLE: u32 = 0xECC8;
pub const IC_REMOVE_CIRCLE: u32 = 0xECC9;
pub const IC_LINK: u32 = 0xE71B;
pub const IC_MAIL: u32 = 0xE715;
pub const IC_PIN: u32 = 0xE718;
pub const IC_VOLUME: u32 = 0xE767;
pub const IC_PERSON: u32 = 0xE77B;
pub const IC_LIGHTBULB: u32 = 0xE82F;
pub const IC_FLAG: u32 = 0xE7C1;
pub const IC_BELL: u32 = 0xEA8F;
pub const IC_BACKSPACE: u32 = 0xE750;
pub const IC_RETURN: u32 = 0xE751;
pub const IC_CLEAR_SELECTION: u32 = 0xE8E6;
pub const IC_SCAN: u32 = 0xE8FE;
pub const IC_RULER: u32 = 0xED5E;
pub const IC_DEV_TOOLS: u32 = 0xEC7A;
pub const IC_SAVE_COPY: u32 = 0xEA35;
pub const IC_MAGIC_SELECT: u32 = 0xEB1D;
pub const IC_TOUCH_POINTER: u32 = 0xF271;
pub const IC_PREVIOUS: u32 = 0xE892;
pub const IC_NEXT: u32 = 0xE893;

// Office semantic icons (generated by styles/jfoffice/mkicons.py).
/// Office paste symbol; style-independent semantic identifier.
pub const IC_OFFICE_PASTE: u32 = 0xF800;
/// Office cut symbol; style-independent semantic identifier.
pub const IC_OFFICE_CUT: u32 = 0xF801;
/// Office copy symbol; style-independent semantic identifier.
pub const IC_OFFICE_COPY: u32 = 0xF802;
/// Office painter symbol; style-independent semantic identifier.
pub const IC_OFFICE_PAINTER: u32 = 0xF803;
/// Office bold symbol; style-independent semantic identifier.
pub const IC_OFFICE_BOLD: u32 = 0xF804;
/// Office italic symbol; style-independent semantic identifier.
pub const IC_OFFICE_ITALIC: u32 = 0xF805;
/// Office underline symbol; style-independent semantic identifier.
pub const IC_OFFICE_UNDERLINE: u32 = 0xF806;
/// Office strike symbol; style-independent semantic identifier.
pub const IC_OFFICE_STRIKE: u32 = 0xF807;
/// Office charborder symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHARBORDER: u32 = 0xF808;
/// Office subscript symbol; style-independent semantic identifier.
pub const IC_OFFICE_SUBSCRIPT: u32 = 0xF809;
/// Office superscript symbol; style-independent semantic identifier.
pub const IC_OFFICE_SUPERSCRIPT: u32 = 0xF80A;
/// Office grow symbol; style-independent semantic identifier.
pub const IC_OFFICE_GROW: u32 = 0xF80B;
/// Office shrink symbol; style-independent semantic identifier.
pub const IC_OFFICE_SHRINK: u32 = 0xF80C;
/// Office case symbol; style-independent semantic identifier.
pub const IC_OFFICE_CASE: u32 = 0xF80D;
/// Office clear symbol; style-independent semantic identifier.
pub const IC_OFFICE_CLEAR: u32 = 0xF80E;
/// Office highlight symbol; style-independent semantic identifier.
pub const IC_OFFICE_HIGHLIGHT: u32 = 0xF80F;
/// Office fontcolor symbol; style-independent semantic identifier.
pub const IC_OFFICE_FONTCOLOR: u32 = 0xF810;
/// Office effects symbol; style-independent semantic identifier.
pub const IC_OFFICE_EFFECTS: u32 = 0xF811;
/// Office fontdialog symbol; style-independent semantic identifier.
pub const IC_OFFICE_FONTDIALOG: u32 = 0xF812;
/// Office bullets symbol; style-independent semantic identifier.
pub const IC_OFFICE_BULLETS: u32 = 0xF813;
/// Office numbering symbol; style-independent semantic identifier.
pub const IC_OFFICE_NUMBERING: u32 = 0xF814;
/// Office multilevel symbol; style-independent semantic identifier.
pub const IC_OFFICE_MULTILEVEL: u32 = 0xF815;
/// Office outdent symbol; style-independent semantic identifier.
pub const IC_OFFICE_OUTDENT: u32 = 0xF816;
/// Office indent symbol; style-independent semantic identifier.
pub const IC_OFFICE_INDENT: u32 = 0xF817;
/// Office sort symbol; style-independent semantic identifier.
pub const IC_OFFICE_SORT: u32 = 0xF818;
/// Office pilcrow symbol; style-independent semantic identifier.
pub const IC_OFFICE_PILCROW: u32 = 0xF819;
/// Office alignLeft symbol; style-independent semantic identifier.
pub const IC_OFFICE_ALIGN_LEFT: u32 = 0xF81A;
/// Office alignCenter symbol; style-independent semantic identifier.
pub const IC_OFFICE_ALIGN_CENTER: u32 = 0xF81B;
/// Office alignRight symbol; style-independent semantic identifier.
pub const IC_OFFICE_ALIGN_RIGHT: u32 = 0xF81C;
/// Office justify symbol; style-independent semantic identifier.
pub const IC_OFFICE_JUSTIFY: u32 = 0xF81D;
/// Office textLtr symbol; style-independent semantic identifier.
pub const IC_OFFICE_TEXT_LTR: u32 = 0xF81E;
/// Office textRtl symbol; style-independent semantic identifier.
pub const IC_OFFICE_TEXT_RTL: u32 = 0xF81F;
/// Office lineSpacing symbol; style-independent semantic identifier.
pub const IC_OFFICE_LINE_SPACING: u32 = 0xF820;
/// Office shading symbol; style-independent semantic identifier.
pub const IC_OFFICE_SHADING: u32 = 0xF821;
/// Office borders symbol; style-independent semantic identifier.
pub const IC_OFFICE_BORDERS: u32 = 0xF822;
/// Office find symbol; style-independent semantic identifier.
pub const IC_OFFICE_FIND: u32 = 0xF823;
/// Office replace symbol; style-independent semantic identifier.
pub const IC_OFFICE_REPLACE: u32 = 0xF824;
/// Office select symbol; style-independent semantic identifier.
pub const IC_OFFICE_SELECT: u32 = 0xF825;
/// Office dictate symbol; style-independent semantic identifier.
pub const IC_OFFICE_DICTATE: u32 = 0xF826;
/// Office editor symbol; style-independent semantic identifier.
pub const IC_OFFICE_EDITOR: u32 = 0xF827;
/// Office spelling symbol; style-independent semantic identifier.
pub const IC_OFFICE_SPELLING: u32 = 0xF828;
/// Office styles symbol; style-independent semantic identifier.
pub const IC_OFFICE_STYLES: u32 = 0xF829;
/// Office stylesPane symbol; style-independent semantic identifier.
pub const IC_OFFICE_STYLES_PANE: u32 = 0xF82A;
/// Office coverPage symbol; style-independent semantic identifier.
pub const IC_OFFICE_COVER_PAGE: u32 = 0xF82B;
/// Office blankPage symbol; style-independent semantic identifier.
pub const IC_OFFICE_BLANK_PAGE: u32 = 0xF82C;
/// Office pageBreak symbol; style-independent semantic identifier.
pub const IC_OFFICE_PAGE_BREAK: u32 = 0xF82D;
/// Office drawTable symbol; style-independent semantic identifier.
pub const IC_OFFICE_DRAW_TABLE: u32 = 0xF82E;
/// Office table symbol; style-independent semantic identifier.
pub const IC_OFFICE_TABLE: u32 = 0xF82F;
/// Office picture symbol; style-independent semantic identifier.
pub const IC_OFFICE_PICTURE: u32 = 0xF830;
/// Office onlinePicture symbol; style-independent semantic identifier.
pub const IC_OFFICE_ONLINE_PICTURE: u32 = 0xF831;
/// Office shapes symbol; style-independent semantic identifier.
pub const IC_OFFICE_SHAPES: u32 = 0xF832;
/// Office shapeEffects symbol; style-independent semantic identifier.
pub const IC_OFFICE_SHAPE_EFFECTS: u32 = 0xF833;
/// Office icons symbol; style-independent semantic identifier.
pub const IC_OFFICE_ICONS: u32 = 0xF834;
/// Office models3d symbol; style-independent semantic identifier.
pub const IC_OFFICE_MODELS3D: u32 = 0xF835;
/// Office smartArt symbol; style-independent semantic identifier.
pub const IC_OFFICE_SMART_ART: u32 = 0xF836;
/// Office chart symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHART: u32 = 0xF837;
/// Office screenshot symbol; style-independent semantic identifier.
pub const IC_OFFICE_SCREENSHOT: u32 = 0xF838;
/// Office video symbol; style-independent semantic identifier.
pub const IC_OFFICE_VIDEO: u32 = 0xF839;
/// Office link symbol; style-independent semantic identifier.
pub const IC_OFFICE_LINK: u32 = 0xF83A;
/// Office bookmark symbol; style-independent semantic identifier.
pub const IC_OFFICE_BOOKMARK: u32 = 0xF83B;
/// Office crossRef symbol; style-independent semantic identifier.
pub const IC_OFFICE_CROSS_REF: u32 = 0xF83C;
/// Office comment symbol; style-independent semantic identifier.
pub const IC_OFFICE_COMMENT: u32 = 0xF83D;
/// Office newComment symbol; style-independent semantic identifier.
pub const IC_OFFICE_NEW_COMMENT: u32 = 0xF83E;
/// Office header symbol; style-independent semantic identifier.
pub const IC_OFFICE_HEADER: u32 = 0xF83F;
/// Office footer symbol; style-independent semantic identifier.
pub const IC_OFFICE_FOOTER: u32 = 0xF840;
/// Office pageNumber symbol; style-independent semantic identifier.
pub const IC_OFFICE_PAGE_NUMBER: u32 = 0xF841;
/// Office textBox symbol; style-independent semantic identifier.
pub const IC_OFFICE_TEXT_BOX: u32 = 0xF842;
/// Office quickParts symbol; style-independent semantic identifier.
pub const IC_OFFICE_QUICK_PARTS: u32 = 0xF843;
/// Office wordArt symbol; style-independent semantic identifier.
pub const IC_OFFICE_WORD_ART: u32 = 0xF844;
/// Office dropCap symbol; style-independent semantic identifier.
pub const IC_OFFICE_DROP_CAP: u32 = 0xF845;
/// Office signature symbol; style-independent semantic identifier.
pub const IC_OFFICE_SIGNATURE: u32 = 0xF846;
/// Office dateTime symbol; style-independent semantic identifier.
pub const IC_OFFICE_DATE_TIME: u32 = 0xF847;
/// Office object symbol; style-independent semantic identifier.
pub const IC_OFFICE_OBJECT: u32 = 0xF848;
/// Office equation symbol; style-independent semantic identifier.
pub const IC_OFFICE_EQUATION: u32 = 0xF849;
/// Office symbol symbol; style-independent semantic identifier.
pub const IC_OFFICE_SYMBOL: u32 = 0xF84A;
/// Office pen symbol; style-independent semantic identifier.
pub const IC_OFFICE_PEN: u32 = 0xF84B;
/// Office pencil symbol; style-independent semantic identifier.
pub const IC_OFFICE_PENCIL: u32 = 0xF84C;
/// Office eraser symbol; style-independent semantic identifier.
pub const IC_OFFICE_ERASER: u32 = 0xF84D;
/// Office thickness symbol; style-independent semantic identifier.
pub const IC_OFFICE_THICKNESS: u32 = 0xF84E;
/// Office lasso symbol; style-independent semantic identifier.
pub const IC_OFFICE_LASSO: u32 = 0xF84F;
/// Office inkToShape symbol; style-independent semantic identifier.
pub const IC_OFFICE_INK_TO_SHAPE: u32 = 0xF850;
/// Office inkToMath symbol; style-independent semantic identifier.
pub const IC_OFFICE_INK_TO_MATH: u32 = 0xF851;
/// Office canvas symbol; style-independent semantic identifier.
pub const IC_OFFICE_CANVAS: u32 = 0xF852;
/// Office replay symbol; style-independent semantic identifier.
pub const IC_OFFICE_REPLAY: u32 = 0xF853;
/// Office themes symbol; style-independent semantic identifier.
pub const IC_OFFICE_THEMES: u32 = 0xF854;
/// Office colors symbol; style-independent semantic identifier.
pub const IC_OFFICE_COLORS: u32 = 0xF855;
/// Office fonts symbol; style-independent semantic identifier.
pub const IC_OFFICE_FONTS: u32 = 0xF856;
/// Office paraSpacing symbol; style-independent semantic identifier.
pub const IC_OFFICE_PARA_SPACING: u32 = 0xF857;
/// Office effectsDesign symbol; style-independent semantic identifier.
pub const IC_OFFICE_EFFECTS_DESIGN: u32 = 0xF858;
/// Office setDefault symbol; style-independent semantic identifier.
pub const IC_OFFICE_SET_DEFAULT: u32 = 0xF859;
/// Office watermark symbol; style-independent semantic identifier.
pub const IC_OFFICE_WATERMARK: u32 = 0xF85A;
/// Office pageColor symbol; style-independent semantic identifier.
pub const IC_OFFICE_PAGE_COLOR: u32 = 0xF85B;
/// Office pageBorders symbol; style-independent semantic identifier.
pub const IC_OFFICE_PAGE_BORDERS: u32 = 0xF85C;
/// Office margins symbol; style-independent semantic identifier.
pub const IC_OFFICE_MARGINS: u32 = 0xF85D;
/// Office orientation symbol; style-independent semantic identifier.
pub const IC_OFFICE_ORIENTATION: u32 = 0xF85E;
/// Office size symbol; style-independent semantic identifier.
pub const IC_OFFICE_SIZE: u32 = 0xF85F;
/// Office columns symbol; style-independent semantic identifier.
pub const IC_OFFICE_COLUMNS: u32 = 0xF860;
/// Office breaks symbol; style-independent semantic identifier.
pub const IC_OFFICE_BREAKS: u32 = 0xF861;
/// Office lineNumbers symbol; style-independent semantic identifier.
pub const IC_OFFICE_LINE_NUMBERS: u32 = 0xF862;
/// Office hyphenation symbol; style-independent semantic identifier.
pub const IC_OFFICE_HYPHENATION: u32 = 0xF863;
/// Office position symbol; style-independent semantic identifier.
pub const IC_OFFICE_POSITION: u32 = 0xF864;
/// Office wrapText symbol; style-independent semantic identifier.
pub const IC_OFFICE_WRAP_TEXT: u32 = 0xF865;
/// Office bringForward symbol; style-independent semantic identifier.
pub const IC_OFFICE_BRING_FORWARD: u32 = 0xF866;
/// Office sendBackward symbol; style-independent semantic identifier.
pub const IC_OFFICE_SEND_BACKWARD: u32 = 0xF867;
/// Office selectionPane symbol; style-independent semantic identifier.
pub const IC_OFFICE_SELECTION_PANE: u32 = 0xF868;
/// Office align symbol; style-independent semantic identifier.
pub const IC_OFFICE_ALIGN: u32 = 0xF869;
/// Office group symbol; style-independent semantic identifier.
pub const IC_OFFICE_GROUP: u32 = 0xF86A;
/// Office rotate symbol; style-independent semantic identifier.
pub const IC_OFFICE_ROTATE: u32 = 0xF86B;
/// Office indentLeft symbol; style-independent semantic identifier.
pub const IC_OFFICE_INDENT_LEFT: u32 = 0xF86C;
/// Office indentRight symbol; style-independent semantic identifier.
pub const IC_OFFICE_INDENT_RIGHT: u32 = 0xF86D;
/// Office spaceBefore symbol; style-independent semantic identifier.
pub const IC_OFFICE_SPACE_BEFORE: u32 = 0xF86E;
/// Office spaceAfter symbol; style-independent semantic identifier.
pub const IC_OFFICE_SPACE_AFTER: u32 = 0xF86F;
/// Office toc symbol; style-independent semantic identifier.
pub const IC_OFFICE_TOC: u32 = 0xF870;
/// Office addText symbol; style-independent semantic identifier.
pub const IC_OFFICE_ADD_TEXT: u32 = 0xF871;
/// Office updateTable symbol; style-independent semantic identifier.
pub const IC_OFFICE_UPDATE_TABLE: u32 = 0xF872;
/// Office update symbol; style-independent semantic identifier.
pub const IC_OFFICE_UPDATE: u32 = 0xF873;
/// Office footnote symbol; style-independent semantic identifier.
pub const IC_OFFICE_FOOTNOTE: u32 = 0xF874;
/// Office endnote symbol; style-independent semantic identifier.
pub const IC_OFFICE_ENDNOTE: u32 = 0xF875;
/// Office nextFootnote symbol; style-independent semantic identifier.
pub const IC_OFFICE_NEXT_FOOTNOTE: u32 = 0xF876;
/// Office showNotes symbol; style-independent semantic identifier.
pub const IC_OFFICE_SHOW_NOTES: u32 = 0xF877;
/// Office researcher symbol; style-independent semantic identifier.
pub const IC_OFFICE_RESEARCHER: u32 = 0xF878;
/// Office docPrefs symbol; style-independent semantic identifier.
pub const IC_OFFICE_DOC_PREFS: u32 = 0xF879;
/// Office unlinkCitations symbol; style-independent semantic identifier.
pub const IC_OFFICE_UNLINK_CITATIONS: u32 = 0xF87A;
/// Office addNote symbol; style-independent semantic identifier.
pub const IC_OFFICE_ADD_NOTE: u32 = 0xF87B;
/// Office citation symbol; style-independent semantic identifier.
pub const IC_OFFICE_CITATION: u32 = 0xF87C;
/// Office sources symbol; style-independent semantic identifier.
pub const IC_OFFICE_SOURCES: u32 = 0xF87D;
/// Office bibliography symbol; style-independent semantic identifier.
pub const IC_OFFICE_BIBLIOGRAPHY: u32 = 0xF87E;
/// Office caption symbol; style-independent semantic identifier.
pub const IC_OFFICE_CAPTION: u32 = 0xF87F;
/// Office tableOfFigures symbol; style-independent semantic identifier.
pub const IC_OFFICE_TABLE_OF_FIGURES: u32 = 0xF880;
/// Office markEntry symbol; style-independent semantic identifier.
pub const IC_OFFICE_MARK_ENTRY: u32 = 0xF881;
/// Office index symbol; style-independent semantic identifier.
pub const IC_OFFICE_INDEX: u32 = 0xF882;
/// Office markCitation symbol; style-independent semantic identifier.
pub const IC_OFFICE_MARK_CITATION: u32 = 0xF883;
/// Office tableOfAuthorities symbol; style-independent semantic identifier.
pub const IC_OFFICE_TABLE_OF_AUTHORITIES: u32 = 0xF884;
/// Office envelope symbol; style-independent semantic identifier.
pub const IC_OFFICE_ENVELOPE: u32 = 0xF885;
/// Office labels symbol; style-independent semantic identifier.
pub const IC_OFFICE_LABELS: u32 = 0xF886;
/// Office mailMerge symbol; style-independent semantic identifier.
pub const IC_OFFICE_MAIL_MERGE: u32 = 0xF887;
/// Office recipients symbol; style-independent semantic identifier.
pub const IC_OFFICE_RECIPIENTS: u32 = 0xF888;
/// Office editRecipients symbol; style-independent semantic identifier.
pub const IC_OFFICE_EDIT_RECIPIENTS: u32 = 0xF889;
/// Office mergeField symbol; style-independent semantic identifier.
pub const IC_OFFICE_MERGE_FIELD: u32 = 0xF88A;
/// Office addressBlock symbol; style-independent semantic identifier.
pub const IC_OFFICE_ADDRESS_BLOCK: u32 = 0xF88B;
/// Office greetingLine symbol; style-independent semantic identifier.
pub const IC_OFFICE_GREETING_LINE: u32 = 0xF88C;
/// Office rules symbol; style-independent semantic identifier.
pub const IC_OFFICE_RULES: u32 = 0xF88D;
/// Office matchFields symbol; style-independent semantic identifier.
pub const IC_OFFICE_MATCH_FIELDS: u32 = 0xF88E;
/// Office highlightFields symbol; style-independent semantic identifier.
pub const IC_OFFICE_HIGHLIGHT_FIELDS: u32 = 0xF88F;
/// Office preview symbol; style-independent semantic identifier.
pub const IC_OFFICE_PREVIEW: u32 = 0xF890;
/// Office next symbol; style-independent semantic identifier.
pub const IC_OFFICE_NEXT: u32 = 0xF891;
/// Office previous symbol; style-independent semantic identifier.
pub const IC_OFFICE_PREVIOUS: u32 = 0xF892;
/// Office first symbol; style-independent semantic identifier.
pub const IC_OFFICE_FIRST: u32 = 0xF893;
/// Office last symbol; style-independent semantic identifier.
pub const IC_OFFICE_LAST: u32 = 0xF894;
/// Office finish symbol; style-independent semantic identifier.
pub const IC_OFFICE_FINISH: u32 = 0xF895;
/// Office checkErrors symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHECK_ERRORS: u32 = 0xF896;
/// Office thesaurus symbol; style-independent semantic identifier.
pub const IC_OFFICE_THESAURUS: u32 = 0xF897;
/// Office wordCount symbol; style-independent semantic identifier.
pub const IC_OFFICE_WORD_COUNT: u32 = 0xF898;
/// Office readAloud symbol; style-independent semantic identifier.
pub const IC_OFFICE_READ_ALOUD: u32 = 0xF899;
/// Office accessibility symbol; style-independent semantic identifier.
pub const IC_OFFICE_ACCESSIBILITY: u32 = 0xF89A;
/// Office translate symbol; style-independent semantic identifier.
pub const IC_OFFICE_TRANSLATE: u32 = 0xF89B;
/// Office language symbol; style-independent semantic identifier.
pub const IC_OFFICE_LANGUAGE: u32 = 0xF89C;
/// Office deleteComment symbol; style-independent semantic identifier.
pub const IC_OFFICE_DELETE_COMMENT: u32 = 0xF89D;
/// Office prevComment symbol; style-independent semantic identifier.
pub const IC_OFFICE_PREV_COMMENT: u32 = 0xF89E;
/// Office nextComment symbol; style-independent semantic identifier.
pub const IC_OFFICE_NEXT_COMMENT: u32 = 0xF89F;
/// Office showComments symbol; style-independent semantic identifier.
pub const IC_OFFICE_SHOW_COMMENTS: u32 = 0xF8A0;
/// Office resolve symbol; style-independent semantic identifier.
pub const IC_OFFICE_RESOLVE: u32 = 0xF8A1;
/// Office trackChanges symbol; style-independent semantic identifier.
pub const IC_OFFICE_TRACK_CHANGES: u32 = 0xF8A2;
/// Office markup symbol; style-independent semantic identifier.
pub const IC_OFFICE_MARKUP: u32 = 0xF8A3;
/// Office reviewingPane symbol; style-independent semantic identifier.
pub const IC_OFFICE_REVIEWING_PANE: u32 = 0xF8A4;
/// Office accept symbol; style-independent semantic identifier.
pub const IC_OFFICE_ACCEPT: u32 = 0xF8A5;
/// Office reject symbol; style-independent semantic identifier.
pub const IC_OFFICE_REJECT: u32 = 0xF8A6;
/// Office prevChange symbol; style-independent semantic identifier.
pub const IC_OFFICE_PREV_CHANGE: u32 = 0xF8A7;
/// Office nextChange symbol; style-independent semantic identifier.
pub const IC_OFFICE_NEXT_CHANGE: u32 = 0xF8A8;
/// Office compare symbol; style-independent semantic identifier.
pub const IC_OFFICE_COMPARE: u32 = 0xF8A9;
/// Office combine symbol; style-independent semantic identifier.
pub const IC_OFFICE_COMBINE: u32 = 0xF8AA;
/// Office protect symbol; style-independent semantic identifier.
pub const IC_OFFICE_PROTECT: u32 = 0xF8AB;
/// Office restrict symbol; style-independent semantic identifier.
pub const IC_OFFICE_RESTRICT: u32 = 0xF8AC;
/// Office blockAuthors symbol; style-independent semantic identifier.
pub const IC_OFFICE_BLOCK_AUTHORS: u32 = 0xF8AD;
/// Office hideInk symbol; style-independent semantic identifier.
pub const IC_OFFICE_HIDE_INK: u32 = 0xF8AE;
/// Office readMode symbol; style-independent semantic identifier.
pub const IC_OFFICE_READ_MODE: u32 = 0xF8AF;
/// Office printLayout symbol; style-independent semantic identifier.
pub const IC_OFFICE_PRINT_LAYOUT: u32 = 0xF8B0;
/// Office webLayout symbol; style-independent semantic identifier.
pub const IC_OFFICE_WEB_LAYOUT: u32 = 0xF8B1;
/// Office outline symbol; style-independent semantic identifier.
pub const IC_OFFICE_OUTLINE: u32 = 0xF8B2;
/// Office draft symbol; style-independent semantic identifier.
pub const IC_OFFICE_DRAFT: u32 = 0xF8B3;
/// Office focus symbol; style-independent semantic identifier.
pub const IC_OFFICE_FOCUS: u32 = 0xF8B4;
/// Office immersive symbol; style-independent semantic identifier.
pub const IC_OFFICE_IMMERSIVE: u32 = 0xF8B5;
/// Office vertical symbol; style-independent semantic identifier.
pub const IC_OFFICE_VERTICAL: u32 = 0xF8B6;
/// Office sideToSide symbol; style-independent semantic identifier.
pub const IC_OFFICE_SIDE_TO_SIDE: u32 = 0xF8B7;
/// Office ruler symbol; style-independent semantic identifier.
pub const IC_OFFICE_RULER: u32 = 0xF8B8;
/// Office gridlines symbol; style-independent semantic identifier.
pub const IC_OFFICE_GRIDLINES: u32 = 0xF8B9;
/// Office navPane symbol; style-independent semantic identifier.
pub const IC_OFFICE_NAV_PANE: u32 = 0xF8BA;
/// Office zoom symbol; style-independent semantic identifier.
pub const IC_OFFICE_ZOOM: u32 = 0xF8BB;
/// Office zoomIn symbol; style-independent semantic identifier.
pub const IC_OFFICE_ZOOM_IN: u32 = 0xF8BC;
/// Office zoomOut symbol; style-independent semantic identifier.
pub const IC_OFFICE_ZOOM_OUT: u32 = 0xF8BD;
/// Office zoom100 symbol; style-independent semantic identifier.
pub const IC_OFFICE_ZOOM100: u32 = 0xF8BE;
/// Office onePage symbol; style-independent semantic identifier.
pub const IC_OFFICE_ONE_PAGE: u32 = 0xF8BF;
/// Office multiplePages symbol; style-independent semantic identifier.
pub const IC_OFFICE_MULTIPLE_PAGES: u32 = 0xF8C0;
/// Office pageWidth symbol; style-independent semantic identifier.
pub const IC_OFFICE_PAGE_WIDTH: u32 = 0xF8C1;
/// Office darkMode symbol; style-independent semantic identifier.
pub const IC_OFFICE_DARK_MODE: u32 = 0xF8C2;
/// Office interfaceTheme symbol; style-independent semantic identifier.
pub const IC_OFFICE_INTERFACE_THEME: u32 = 0xF8C3;
/// Office newWindow symbol; style-independent semantic identifier.
pub const IC_OFFICE_NEW_WINDOW: u32 = 0xF8C4;
/// Office arrangeAll symbol; style-independent semantic identifier.
pub const IC_OFFICE_ARRANGE_ALL: u32 = 0xF8C5;
/// Office split symbol; style-independent semantic identifier.
pub const IC_OFFICE_SPLIT: u32 = 0xF8C6;
/// Office sideBySide symbol; style-independent semantic identifier.
pub const IC_OFFICE_SIDE_BY_SIDE: u32 = 0xF8C7;
/// Office syncScroll symbol; style-independent semantic identifier.
pub const IC_OFFICE_SYNC_SCROLL: u32 = 0xF8C8;
/// Office switchWindows symbol; style-independent semantic identifier.
pub const IC_OFFICE_SWITCH_WINDOWS: u32 = 0xF8C9;
/// Office macros symbol; style-independent semantic identifier.
pub const IC_OFFICE_MACROS: u32 = 0xF8CA;
/// Office properties symbol; style-independent semantic identifier.
pub const IC_OFFICE_PROPERTIES: u32 = 0xF8CB;
/// Office info symbol; style-independent semantic identifier.
pub const IC_OFFICE_INFO: u32 = 0xF8CC;
/// Office insertAbove symbol; style-independent semantic identifier.
pub const IC_OFFICE_INSERT_ABOVE: u32 = 0xF8CD;
/// Office insertBelow symbol; style-independent semantic identifier.
pub const IC_OFFICE_INSERT_BELOW: u32 = 0xF8CE;
/// Office insertLeft symbol; style-independent semantic identifier.
pub const IC_OFFICE_INSERT_LEFT: u32 = 0xF8CF;
/// Office insertRight symbol; style-independent semantic identifier.
pub const IC_OFFICE_INSERT_RIGHT: u32 = 0xF8D0;
/// Office deleteTable symbol; style-independent semantic identifier.
pub const IC_OFFICE_DELETE_TABLE: u32 = 0xF8D1;
/// Office merge symbol; style-independent semantic identifier.
pub const IC_OFFICE_MERGE: u32 = 0xF8D2;
/// Office splitCells symbol; style-independent semantic identifier.
pub const IC_OFFICE_SPLIT_CELLS: u32 = 0xF8D3;
/// Office splitTable symbol; style-independent semantic identifier.
pub const IC_OFFICE_SPLIT_TABLE: u32 = 0xF8D4;
/// Office autofit symbol; style-independent semantic identifier.
pub const IC_OFFICE_AUTOFIT: u32 = 0xF8D5;
/// Office distributeRows symbol; style-independent semantic identifier.
pub const IC_OFFICE_DISTRIBUTE_ROWS: u32 = 0xF8D6;
/// Office distributeCols symbol; style-independent semantic identifier.
pub const IC_OFFICE_DISTRIBUTE_COLS: u32 = 0xF8D7;
/// Office rowHeight symbol; style-independent semantic identifier.
pub const IC_OFFICE_ROW_HEIGHT: u32 = 0xF8D8;
/// Office colWidth symbol; style-independent semantic identifier.
pub const IC_OFFICE_COL_WIDTH: u32 = 0xF8D9;
/// Office cellAlign symbol; style-independent semantic identifier.
pub const IC_OFFICE_CELL_ALIGN: u32 = 0xF8DA;
/// Office textDirection symbol; style-independent semantic identifier.
pub const IC_OFFICE_TEXT_DIRECTION: u32 = 0xF8DB;
/// Office cellMargins symbol; style-independent semantic identifier.
pub const IC_OFFICE_CELL_MARGINS: u32 = 0xF8DC;
/// Office repeatHeader symbol; style-independent semantic identifier.
pub const IC_OFFICE_REPEAT_HEADER: u32 = 0xF8DD;
/// Office toText symbol; style-independent semantic identifier.
pub const IC_OFFICE_TO_TEXT: u32 = 0xF8DE;
/// Office formula symbol; style-independent semantic identifier.
pub const IC_OFFICE_FORMULA: u32 = 0xF8DF;
/// Office selectTable symbol; style-independent semantic identifier.
pub const IC_OFFICE_SELECT_TABLE: u32 = 0xF8E0;
/// Office draw symbol; style-independent semantic identifier.
pub const IC_OFFICE_DRAW: u32 = 0xF8E1;
/// Office borderPainter symbol; style-independent semantic identifier.
pub const IC_OFFICE_BORDER_PAINTER: u32 = 0xF8E2;
/// Office save symbol; style-independent semantic identifier.
pub const IC_OFFICE_SAVE: u32 = 0xF8E3;
/// Office undo symbol; style-independent semantic identifier.
pub const IC_OFFICE_UNDO: u32 = 0xF8E4;
/// Office redo symbol; style-independent semantic identifier.
pub const IC_OFFICE_REDO: u32 = 0xF8E5;
/// Office search symbol; style-independent semantic identifier.
pub const IC_OFFICE_SEARCH: u32 = 0xF8E6;
/// Office share symbol; style-independent semantic identifier.
pub const IC_OFFICE_SHARE: u32 = 0xF8E7;
/// Office user symbol; style-independent semantic identifier.
pub const IC_OFFICE_USER: u32 = 0xF8E8;
/// Office close symbol; style-independent semantic identifier.
pub const IC_OFFICE_CLOSE: u32 = 0xF8E9;
/// Office dropdown symbol; style-independent semantic identifier.
pub const IC_OFFICE_DROPDOWN: u32 = 0xF8EA;
/// Office more symbol; style-independent semantic identifier.
pub const IC_OFFICE_MORE: u32 = 0xF8EB;
/// Office launcher symbol; style-independent semantic identifier.
pub const IC_OFFICE_LAUNCHER: u32 = 0xF8EC;
/// Office pin symbol; style-independent semantic identifier.
pub const IC_OFFICE_PIN: u32 = 0xF8ED;
/// Office plus symbol; style-independent semantic identifier.
pub const IC_OFFICE_PLUS: u32 = 0xF8EE;
/// Office minus symbol; style-independent semantic identifier.
pub const IC_OFFICE_MINUS: u32 = 0xF8EF;
/// Office check symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHECK: u32 = 0xF8F0;
/// Office chevronLeft symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHEVRON_LEFT: u32 = 0xF8F1;
/// Office chevronRight symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHEVRON_RIGHT: u32 = 0xF8F2;
/// Office chevronUp symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHEVRON_UP: u32 = 0xF8F3;
/// Office chevronDown symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHEVRON_DOWN: u32 = 0xF8F4;
/// Office mediaPlay symbol; style-independent semantic identifier.
pub const IC_OFFICE_MEDIA_PLAY: u32 = 0xF8F5;
/// Office mediaPause symbol; style-independent semantic identifier.
pub const IC_OFFICE_MEDIA_PAUSE: u32 = 0xF8F6;
/// Office mediaStop symbol; style-independent semantic identifier.
pub const IC_OFFICE_MEDIA_STOP: u32 = 0xF8F7;
/// Office mediaPrev symbol; style-independent semantic identifier.
pub const IC_OFFICE_MEDIA_PREV: u32 = 0xF8F8;
/// Office mediaNext symbol; style-independent semantic identifier.
pub const IC_OFFICE_MEDIA_NEXT: u32 = 0xF8F9;
/// Office pastCitation symbol; style-independent semantic identifier.
pub const IC_OFFICE_PAST_CITATION: u32 = 0xF8FA;
/// Office chevronDoubleRight symbol; style-independent semantic identifier.
pub const IC_OFFICE_CHEVRON_DOUBLE_RIGHT: u32 = 0xF8FB;
/// Office addins symbol; style-independent semantic identifier.
pub const IC_OFFICE_ADDINS: u32 = 0xF8FC;
/// Office help symbol; style-independent semantic identifier.
pub const IC_OFFICE_HELP: u32 = 0xF8FD;
/// Office open symbol; style-independent semantic identifier.
pub const IC_OFFICE_OPEN: u32 = 0xF8FE;
/// Office print symbol; style-independent semantic identifier.
pub const IC_OFFICE_PRINT: u32 = 0xF8FF;
/// Office export symbol; style-independent semantic identifier.
pub const IC_OFFICE_EXPORT: u32 = 0xF900;
/// Office home symbol; style-independent semantic identifier.
pub const IC_OFFICE_HOME: u32 = 0xF901;
/// Office settings symbol; style-independent semantic identifier.
pub const IC_OFFICE_SETTINGS: u32 = 0xF902;
/// Office history symbol; style-independent semantic identifier.
pub const IC_OFFICE_HISTORY: u32 = 0xF903;
/// Office document symbol; style-independent semantic identifier.
pub const IC_OFFICE_DOCUMENT: u32 = 0xF904;
// End generated Office semantic icons.

// constants -- the ids of JFNetworker's commands, widgets, dock panels and windows
//
// Commands reach App::command (menus, buttons, accelerators, `cmd N` in scripts); widget ids
// stay below ui::ID_HASH_BASE and leave room for the ids a widget takes after its own
// (segmented control, dock area, file dialog).

// ---- commands (the table in commands.rs)
pub(crate) const CMD_NEW: u32 = 1;
pub(crate) const CMD_OPEN: u32 = 2;
pub(crate) const CMD_SAVE: u32 = 3;
pub(crate) const CMD_SAVE_AS: u32 = 4;
pub(crate) const CMD_QUIT: u32 = 5;
pub(crate) const CMD_UNDO: u32 = 10;
pub(crate) const CMD_REDO: u32 = 11;
pub(crate) const CMD_DELETE: u32 = 12;
pub(crate) const CMD_SELECT_ALL: u32 = 13;
pub(crate) const CMD_RENAME: u32 = 14;
pub(crate) const CMD_ZOOM_IN: u32 = 20;
pub(crate) const CMD_ZOOM_OUT: u32 = 21;
pub(crate) const CMD_ZOOM_100: u32 = 22;
pub(crate) const CMD_FIT: u32 = 23;
pub(crate) const CMD_GRID: u32 = 24;
pub(crate) const CMD_SNAP: u32 = 25;
pub(crate) const CMD_MINIMAP: u32 = 26;
pub(crate) const CMD_LAYOUT_RESET: u32 = 27;
/// .. + 7: show the dock panel PANELS[k] and give it the keyboard (Ctrl+1 ..)
pub(crate) const CMD_PANEL0: u32 = 30;
pub(crate) const CMD_TOOL_SELECT: u32 = 40;
pub(crate) const CMD_TOOL_HAND: u32 = 41;
pub(crate) const CMD_TOOL_CONNECT: u32 = 42;
pub(crate) const CMD_TOOL_NOTE: u32 = 43;
pub(crate) const CMD_SIM_REALTIME: u32 = 50;
pub(crate) const CMD_SIM_MODE: u32 = 51;
pub(crate) const CMD_SIM_PLAY: u32 = 52;
pub(crate) const CMD_SIM_STEP: u32 = 53;
pub(crate) const CMD_SIM_BACK: u32 = 54;
pub(crate) const CMD_SIM_RESET: u32 = 55;
pub(crate) const CMD_ABOUT: u32 = 60;
/// .. + RECENT_MAX - 1: open a recently used project
pub(crate) const CMD_RECENT0: u32 = 70;
pub(crate) const RECENT_MAX: usize = 8;
/// the file dialog chose a file (arg 1: the save dialog); queued by doc::file::dialogs
pub(crate) const CMD_FILE_DONE: u32 = 80;
/// the "save changes?" question was answered (arg 1: Speichern, 0: Verwerfen)
pub(crate) const CMD_ASK_ANSWER: u32 = 81;
/// the selected cables' route (context menu of a cable)
pub(crate) const CMD_ROUTE_STRAIGHT: u32 = 90;
pub(crate) const CMD_ROUTE_ORTHO: u32 = 91;
pub(crate) const CMD_ROUTE_CURVE: u32 = 92;
/// disconnect the selected cables
pub(crate) const CMD_DISCONNECT: u32 = 93;
/// switch the selected devices on or off
pub(crate) const CMD_POWER: u32 = 94;
/// the kind of the selected cables: + 0 "Automatisch", + 1 + k CableKind::ALL[k]
pub(crate) const CMD_CABLE0: u32 = 100;

// ---- timers (App::command(UIC_TIMER, id))
/// the simulator's clock (doc::sim)
pub(crate) const TIMER_SIM: u32 = 0x100;

// ---- dock panels (DockPanel ids, kept in the saved layout: never change them)
pub(crate) const P_DEVICES: u32 = 1;
pub(crate) const P_PROPS: u32 = 2;
pub(crate) const P_TERMINAL: u32 = 3;
pub(crate) const P_EVENTS: u32 = 4;
pub(crate) const P_PACKET: u32 = 5;
pub(crate) const P_LOG: u32 = 6;
pub(crate) const P_PROBLEMS: u32 = 7;
/// the panels in menu order (CMD_PANEL0 + k)
pub(crate) const PANELS: [u32; 8] = [P_DEVICES, P_PROPS, P_TERMINAL, P_EVENTS, P_PACKET, P_LOG, P_PROBLEMS, P_PDUS];

// ---- widgets
/// the dock area; its floating tool windows are ID_DOCK + 1 .. + 6
pub(crate) const ID_DOCK: u32 = 0x1000;
pub(crate) const ID_TOOLBAR: u32 = 0x2000;
pub(crate) const ID_TB_ZOOM: u32 = 0x2024;
/// the Echtzeit | Simulation control (+ 1 + i: its segments)
pub(crate) const ID_TB_MODE: u32 = 0x2030;
pub(crate) const ID_CANVAS: u32 = 0x3000;
pub(crate) const ID_MINIMAP: u32 = 0x3001;
/// the rename flyout (a popup id) and its widgets
pub(crate) const ID_RENAME: u32 = 0x3010;
pub(crate) const ID_RENAME_BOX: u32 = 0x3011;
pub(crate) const ID_RENAME_OK: u32 = 0x3012;
pub(crate) const ID_RENAME_CANCEL: u32 = 0x3013;
/// the info bar above the canvas (+ 1: its close button)
pub(crate) const ID_INFOBAR: u32 = 0x3020;
/// the port choice when connecting (a popup id) and its widgets; the port lists of both
/// devices take their row ids after them (+ 1 + row)
pub(crate) const ID_CONNECT: u32 = 0x3030;
pub(crate) const ID_CONN_CABLE: u32 = 0x3031;
pub(crate) const ID_CONN_OK: u32 = 0x3032;
pub(crate) const ID_CONN_CANCEL: u32 = 0x3033;
pub(crate) const ID_CONN_A: u32 = 0x3100;
pub(crate) const ID_CONN_B: u32 = 0x3200;
/// the × that disconnects the cable under the pointer
pub(crate) const ID_CABLE_X: u32 = 0x3040;
pub(crate) const ID_PALETTE: u32 = 0x4000;
pub(crate) const ID_SEARCH: u32 = 0x4001;
/// the category expanders: ID_CATEGORY + index in Category::ALL
pub(crate) const ID_CATEGORY: u32 = 0x4010;
/// the tiles: ID_TILE + index in the catalog
pub(crate) const ID_TILE: u32 = 0x4100;
pub(crate) const ID_INSPECTOR: u32 = 0x5000;
pub(crate) const ID_INS_NAME: u32 = 0x5001;
pub(crate) const ID_INS_TITLE: u32 = 0x5002;
pub(crate) const ID_INS_NOTE: u32 = 0x5003;
pub(crate) const ID_INS_NOTE_APPLY: u32 = 0x5004;
pub(crate) const ID_INS_RENAME: u32 = 0x5005;
/// a device: power; its interfaces (a grid, rows + 1 + i) in a sideways scroll view; the
/// fields of the chosen interface
pub(crate) const ID_INS_POWER: u32 = 0x5006;
pub(crate) const ID_INS_IF_SCROLL: u32 = 0x5007;
pub(crate) const ID_INS_IP: u32 = 0x5008;
pub(crate) const ID_INS_IP_APPLY: u32 = 0x5009;
pub(crate) const ID_INS_DHCP: u32 = 0x500A;
pub(crate) const ID_INS_IF_UP: u32 = 0x500B;
pub(crate) const ID_INS_GW: u32 = 0x500C;
pub(crate) const ID_INS_GW_APPLY: u32 = 0x500D;
pub(crate) const ID_INS_IFACES: u32 = 0x5200;
/// a cable: kind, speed, delay, loss, plugged in
pub(crate) const ID_INS_CABLE: u32 = 0x5010;
pub(crate) const ID_INS_KBPS: u32 = 0x5011;
pub(crate) const ID_INS_DELAY: u32 = 0x5012;
pub(crate) const ID_INS_LOSS: u32 = 0x5013;
pub(crate) const ID_INS_PLUG: u32 = 0x5014;
pub(crate) const ID_INS_DISCONNECT: u32 = 0x5016;
pub(crate) const ID_INS_LINK_APPLY: u32 = 0x5017;
/// the route of a cable (+ 1 + i: its segments)
pub(crate) const ID_INS_ROUTE: u32 = 0x5020;
/// several things: switch all on / off
pub(crate) const ID_INS_ALL_ON: u32 = 0x5018;
pub(crate) const ID_INS_ALL_OFF: u32 = 0x5019;
/// the problems list (rows + 1 + i)
pub(crate) const ID_PROBLEM_LIST: u32 = 0x5400;
/// the "save changes?" question and the info dialog
pub(crate) const ID_ASK: u32 = 0x6000;
pub(crate) const ID_ABOUT: u32 = 0x6010;
/// the file dialog: ID_FILEDLG .. + 0x3FFF
pub(crate) const ID_FILEDLG: u32 = 0x0010_0000;

/// drags of palette tiles: the app's own kind, the catalog id as the bytes
pub(crate) const DT_DEVICE: u32 = ui::DT_USER0;

// ---- consoles, device windows, the log (U3: commands 120..159, widgets 0x6100..0x7FFF)
/// the selected device: its console (Terminal panel), its device window
pub(crate) const CMD_CONSOLE: u32 = 120;
pub(crate) const CMD_DEVWIN: u32 = 121;
/// the shown console tab: close it, rename it, into a window of its own; every console in a
/// window, side by side; clear the shown console
pub(crate) const CMD_TERM_CLOSE: u32 = 122;
pub(crate) const CMD_TERM_RENAME: u32 = 123;
pub(crate) const CMD_TERM_FLOAT: u32 = 124;
pub(crate) const CMD_TERM_TILE: u32 = 125;
pub(crate) const CMD_TERM_CLEAR: u32 = 126;
/// forget the lines of the log
pub(crate) const CMD_LOG_CLEAR: u32 = 127;
/// the Terminal panel: the tabs of the open consoles (+ 1 + i: tab i)
pub(crate) const ID_TERM_TABS: u32 = 0x6100;
/// the rename flyout of a console tab (a popup id) and its widgets
pub(crate) const ID_TERM_RENAME: u32 = 0x6140;
pub(crate) const ID_TERM_RENAME_BOX: u32 = 0x6141;
pub(crate) const ID_TERM_RENAME_OK: u32 = 0x6142;
pub(crate) const ID_TERM_RENAME_CANCEL: u32 = 0x6143;
/// "Einschalten" above the console of a device without power
pub(crate) const ID_TERM_POWER: u32 = 0x6150;
/// "Zeigen" where a console is shown in another window
pub(crate) const ID_TERM_SHOW: u32 = 0x6151;
/// the consoles: console slot k takes ID_TERM0 + 8 k .. + 7 (+ 1: its input line)
pub(crate) const ID_TERM0: u32 = 0x6200;
pub(crate) const TERM_SLOTS: u32 = 128;
/// consoles in windows of their own: window k is ID_TERM_WIN0 + 0x10 k (+ 1: "Andocken")
pub(crate) const ID_TERM_WIN0: u32 = 0x6600;
pub(crate) const TERM_WINS: usize = 3;
/// the device windows: window k is ID_DEVWIN0 + 0x200 k, its widgets follow (devwin/)
pub(crate) const ID_DEVWIN0: u32 = 0x6800;
pub(crate) const DEVWINS: usize = 2;
/// the Protokoll panel: device, severity, search, "Neueste zeigen"; its table (rows + 1 + i,
/// at most workspace::logview::LOG_MAX)
pub(crate) const ID_LOG_DEV: u32 = 0x7000;
pub(crate) const ID_LOG_SEV: u32 = 0x7001;
pub(crate) const ID_LOG_FIND: u32 = 0x7002;
pub(crate) const ID_LOG_FOLLOW: u32 = 0x7003;
pub(crate) const ID_LOG_GRID: u32 = 0x7400;
// ---- the simulation mode (workspace/sim: commands 160 .. 199, widgets 0x8000 .. 0x9FFF,
// timers 0x110 .. 0x11F)
/// the speed: + k for sim::SPEEDS[k] (0,25× .. 8×)
pub(crate) const CMD_SIM_TEMPO0: u32 = 160;
/// the view back to the present
pub(crate) const CMD_SIM_LIVE: u32 = 170;
/// steps, the event list and the animation only for the selected devices
pub(crate) const CMD_SIM_ONLY_SEL: u32 = 171;
/// the tool for simple PDUs (P): a source, then a destination
pub(crate) const CMD_TOOL_PDU: u32 = 172;
/// the dialog of a PDU with protocol, ports, size, TTL, count
pub(crate) const CMD_PDU_COMPLEX: u32 = 173;
/// Datei › Exportieren: the recording as a pcapng file (the save dialog)
pub(crate) const CMD_EXPORT_PCAP: u32 = 174;
/// the pcapng dialog chose its file (queued)
pub(crate) const CMD_PCAP_DONE: u32 = 175;
/// the view at a moment of the recording, arg: ns of virtual time (scripts; up to 4,29 s)
pub(crate) const CMD_SIM_SEEK: u32 = 176;
/// the protocols of steps, events and animation: + k shows or hides sim::CATS[k]
pub(crate) const CMD_SIM_CAT0: u32 = 180;
/// the animation of the simulation mode (tweens, fading crosses, playing)
pub(crate) const TIMER_ANIM: u32 = 0x110;
/// the toolbar: play / pause, the speed
pub(crate) const ID_TB_PLAY: u32 = 0x8000;
pub(crate) const ID_TB_TEMPO: u32 = 0x8001;
/// "Zurück zu live" in the hint above the canvas
pub(crate) const ID_SIM_LIVE: u32 = 0x8008;
/// the dock panel of the user's PDUs (right, below Eigenschaften)
pub(crate) const P_PDUS: u32 = 8;
/// Ereignisse: the protocol chips (+ k), "Nur Auswahl", the split, the timeline; the grid
/// last (its rows + 1 + i go past the range, up to the trace's 400 000 events)
pub(crate) const ID_EV_CHIP0: u32 = 0x8010;
pub(crate) const ID_EV_ONLY_SEL: u32 = 0x801A;
pub(crate) const ID_EV_TL_SPLIT: u32 = 0x8021;
pub(crate) const ID_EV_TIMELINE: u32 = 0x8028;
pub(crate) const ID_EV_GRID: u32 = 0x9800;
/// Paket: the splits, the device's step (a scroll view, its PDUs + i), the hex view, the
/// layer tree (rows + 1 + i)
pub(crate) const ID_PK_SPLIT: u32 = 0x8040;
pub(crate) const ID_PK_SPLIT2: u32 = 0x8041;
pub(crate) const ID_PK_STEP: u32 = 0x8048;
pub(crate) const ID_PK_PDU0: u32 = 0x8050;
pub(crate) const ID_PK_HEX: u32 = 0x8078;
pub(crate) const ID_PK_TREE: u32 = 0x8800;
/// the PDUs panel: its buttons, the list (rows + 1 + i), per row resend / stop / delete
/// (+ 4 * i + k)
pub(crate) const ID_PDU_SIMPLE: u32 = 0x8100;
pub(crate) const ID_PDU_NEW: u32 = 0x8101;
pub(crate) const ID_PDU_LIST: u32 = 0x8C00;
pub(crate) const ID_PDU_ACT0: u32 = 0x9000;
/// the dialog of a complex PDU and its fields (4 apart: number boxes take ids after theirs)
pub(crate) const ID_PDU_DLG: u32 = 0x8110;
pub(crate) const ID_PD_SRC: u32 = 0x8120;
pub(crate) const ID_PD_DST: u32 = 0x8124;
pub(crate) const ID_PD_PROTO: u32 = 0x8128;
pub(crate) const ID_PD_SPORT: u32 = 0x812C;
pub(crate) const ID_PD_DPORT: u32 = 0x8130;
pub(crate) const ID_PD_SIZE: u32 = 0x8134;
pub(crate) const ID_PD_TTL: u32 = 0x8138;
pub(crate) const ID_PD_COUNT: u32 = 0x813C;
pub(crate) const ID_PD_INTERVAL: u32 = 0x8140;
pub(crate) const ID_PD_PERIODIC: u32 = 0x8144;
/// the frames on the canvas: + (frame number % 1024) (press: select)
pub(crate) const ID_PKT0: u32 = 0x9400;
/// the pcapng save dialog: ID_PCAP_DLG .. + 0x3FFF (a file dialog takes a block like
/// ID_FILEDLG)
pub(crate) const ID_PCAP_DLG: u32 = 0x0011_0000;
// ---- U5: keyboard and screen readers, settings, autosave, shortcuts (commands 215 .. 259,
// widgets 0xA200 .. 0xAFFF, timers 0x130 .. 0x13F)
/// the topology canvas gets the keyboard (Ctrl+9)
pub(crate) const CMD_FOCUS_CANVAS: u32 = 215;
/// the settings dialog, the overview of the shortcuts
pub(crate) const CMD_SETTINGS: u32 = 216;
pub(crate) const CMD_SHORTCUTS: u32 = 217;
/// the recovery bar: open the autosaved project, throw it away
pub(crate) const CMD_RECOVER: u32 = 218;
pub(crate) const CMD_RECOVER_DROP: u32 = 219;
/// the autosave timer (doc::autosave)
pub(crate) const TIMER_AUTOSAVE: u32 = 0x130;
/// the toolbar as a pane (F6 order) and its parts as focus groups (one Tab stop each)
pub(crate) const ID_TB_PANE: u32 = 0xA200;
pub(crate) const ID_TB_GRP_TOOLS: u32 = 0xA201;
pub(crate) const ID_TB_GRP_EDIT: u32 = 0xA202;
pub(crate) const ID_TB_GRP_ZOOM: u32 = 0xA203;
pub(crate) const ID_TB_GRP_VIEW: u32 = 0xA204;
pub(crate) const ID_TB_GRP_PLAY: u32 = 0xA205;
pub(crate) const ID_TB_GRP_PDU: u32 = 0xA206;
/// the palette: the tiles of category k are one focus group (+ k)
pub(crate) const ID_TILE_GROUP: u32 = 0xA210;
/// the recovery bar (a card) and its buttons
pub(crate) const ID_RECOVER_BAR: u32 = 0xA220;
pub(crate) const ID_RECOVER_OK: u32 = 0xA224;
pub(crate) const ID_RECOVER_DROP: u32 = 0xA225;
pub(crate) const ID_RECOVER_LATER: u32 = 0xA226;
/// the settings dialog, its scroll view and page (a scope of hashed ids)
pub(crate) const ID_SETTINGS_DLG: u32 = 0xA230;
pub(crate) const ID_SETTINGS_SCROLL: u32 = 0xA231;
pub(crate) const ID_SETTINGS_PAGE: u32 = 0xA232;
/// the overview of the shortcuts and its scroll view
pub(crate) const ID_KEYS_DLG: u32 = 0xA240;
pub(crate) const ID_KEYS_SCROLL: u32 = 0xA241;
// ---- performance (U5: commands 200 .. 209, widgets 0xA000 .. 0xA0FF, timers 0x120 .. 0x12F)
/// the frames on the canvas in the simulation mode: a record whose value is their number
pub(crate) const ID_SIM_FRAMES: u32 = 0xA000;

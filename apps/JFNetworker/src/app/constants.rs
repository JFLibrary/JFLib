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
// ---- U6b: tasks and labs (commands 300 .. 339, widgets 0xC000 .. 0xCFFF, timers 0x150 ..
// 0x15F, the dock panel P_TASK)
/// Datei › Labor öffnen …: the list of the labs in app://labs
pub(crate) const CMD_LAB_OPEN: u32 = 300;
/// the Aufgabe panel shows and gets the keyboard
pub(crate) const CMD_TASK_PANEL: u32 = 301;
/// "Prüfen": the goals are judged now and the open ones say why they are not reached
pub(crate) const CMD_TASK_CHECK: u32 = 302;
/// the next hint: of the first open goal, else of the whole task
pub(crate) const CMD_TASK_HINT: u32 = 303;
/// the list of labs chose one (arg: its index in the list; queued by the dialog)
pub(crate) const CMD_LAB_CHOSEN: u32 = 304;
/// the dock panel of the lab's task (right, a tab beside Eigenschaften)
pub(crate) const P_TASK: u32 = 10;
/// the Aufgabe panel: its scroll view, "Prüfen", the progress, the task's hint button, the
/// empty state's "Labor öffnen …", the introduction; per goal its record (+ i) and its hint
/// button (+ i)
pub(crate) const ID_TASK: u32 = 0xC000;
pub(crate) const ID_TASK_CHECK: u32 = 0xC001;
pub(crate) const ID_TASK_PROGRESS: u32 = 0xC002;
pub(crate) const ID_TASK_HINT: u32 = 0xC003;
pub(crate) const ID_TASK_OPEN: u32 = 0xC004;
/// the expander of the introduction
pub(crate) const ID_TASK_INTRO: u32 = 0xC005;
pub(crate) const ID_TASK_GOAL0: u32 = 0xC010;
pub(crate) const ID_TASK_GOAL_HINT0: u32 = 0xC100;
/// goals a lab shows at most
pub(crate) const TASK_GOALS: usize = 200;
/// the list of labs (a dialog) and its list (rows + 1 + i)
pub(crate) const ID_LABS_DLG: u32 = 0xC400;
pub(crate) const ID_LABS_LIST: u32 = 0xC410;
/// the Dienste tab of device window k: "Einstellungen …" of service i is + 0x40 k + i; the
/// settings editor of a service (a code editor: + 0 .. 15), Übernehmen, Abbrechen
pub(crate) const ID_SVC_EDIT_BTN0: u32 = 0xC800;
pub(crate) const ID_SVC_EDIT: u32 = 0xC900;
pub(crate) const ID_SVC_EDIT_APPLY: u32 = 0xC910;
pub(crate) const ID_SVC_EDIT_CANCEL: u32 = 0xC911;
// ---- the script editor (U7: commands 340 .. 379, widgets 0xD000 .. 0xDFFF, timers 0x160 ..
// 0x16F; no dock panel: P_SCRIPT = 11 stays free)
/// the selected host's device window on its Skripte tab (the context menu of a device)
pub(crate) const CMD_SCRIPTS: u32 = 340;
/// the check of the edited scripts while typing (at most every devwin::scripts::CHECK_MS)
pub(crate) const TIMER_SCRIPT_CHECK: u32 = 0x160;
/// the Skripte tab of device window k: ID_SCRIPTS0 + 0x400 k .. + 0x3FF (devwin/scripts.rs:
/// + 0x20 the code editor, + 0x30 the output, + 0x40 the errors (rows + 1 + i), + 0x100 the
/// list of scripts (rows + 1 + i))
pub(crate) const ID_SCRIPTS0: u32 = 0xD000;
// ---- U6a: the lab in the window -- the Metriken panel, the badges and sparklines of the
// canvas, faults in the inspector (commands 260 .. 299, widgets 0xB000 .. 0xBFFF, timers
// 0x140 .. 0x14F)
/// the Metriken panel shows and its service tree gets the keyboard (Ctrl+Shift+M)
pub(crate) const CMD_METRICS: u32 = 260;
/// every active fault heals (one undo step; Simulation › Alle Fehler beheben)
pub(crate) const CMD_HEAL_ALL: u32 = 261;
/// the dock panel of the lab's metrics (bottom, a tab beside Terminal .. Probleme)
pub(crate) const P_METRICS: u32 = 9;
/// the Metriken panel: the split between tree and charts, the scroll view of the charts, the
/// span of the charts (+ 1 + i: its segments), the charts (+ k)
pub(crate) const ID_MET_SPLIT: u32 = 0xB000;
pub(crate) const ID_MET_SCROLL: u32 = 0xB001;
pub(crate) const ID_MET_SPAN: u32 = 0xB008;
pub(crate) const ID_MET_CHART0: u32 = 0xB020;
/// the mini sparklines under the lab's devices on the canvas (+ k, at most 24)
pub(crate) const ID_SPARK0: u32 = 0xB040;
/// faults in the inspector: the duration; a device's power, service (+ its stop button),
/// slower disk (extra ms, factor, button); a cable's unplug, delay, loss, rate, impair; the
/// partition of a selection; "Alle wiederherstellen"; the heal buttons of the active faults
/// (+ k, at most 64)
pub(crate) const ID_FLT_DUR: u32 = 0xB100;
pub(crate) const ID_FLT_POWER: u32 = 0xB101;
pub(crate) const ID_FLT_SVC: u32 = 0xB102;
pub(crate) const ID_FLT_STOP: u32 = 0xB103;
pub(crate) const ID_FLT_DISK_MS: u32 = 0xB104;
pub(crate) const ID_FLT_DISK_X: u32 = 0xB105;
pub(crate) const ID_FLT_DISK: u32 = 0xB106;
pub(crate) const ID_FLT_UNPLUG: u32 = 0xB107;
pub(crate) const ID_FLT_DELAY: u32 = 0xB108;
pub(crate) const ID_FLT_LOSS: u32 = 0xB109;
pub(crate) const ID_FLT_RATE: u32 = 0xB10A;
pub(crate) const ID_FLT_IMPAIR: u32 = 0xB10B;
pub(crate) const ID_FLT_PART: u32 = 0xB10C;
pub(crate) const ID_FLT_HEAL_ALL: u32 = 0xB10D;
pub(crate) const ID_FLT_HEAL0: u32 = 0xB140;
/// the service tree (its rows + 1 + i, at most 2000: up to 0xBFD1)
pub(crate) const ID_MET_TREE: u32 = 0xB800;
// ---- A1: assessments -- an assessment in the Aufgabe panel, the author's panel (commands
// 510 .. 539, widgets 0xF800 .. 0xFBFF, timers 0x188 .. 0x18B, the dock panel P_AUTHOR)
/// Werkzeuge › Aufgabe erstellen …: the author's panel shows (filled from the project's
/// assessment when it has one)
pub(crate) const CMD_AUTHOR: u32 = 510;
/// hand the assessment in (Simulation › Aufgabe abgeben; the question "Jetzt abgeben?" queues it)
pub(crate) const CMD_ASSESS_SUBMIT: u32 = 511;
/// the author's panel: the network as it runs becomes the target; the start network made from
/// the target; the lab with its assessment into the project, then the save dialog
pub(crate) const CMD_AUTHOR_TARGET: u32 = 512;
pub(crate) const CMD_AUTHOR_START: u32 = 513;
pub(crate) const CMD_AUTHOR_SAVE: u32 = 514;
/// the countdown of a time limit (every second while it runs)
pub(crate) const TIMER_ASSESS: u32 = 0x188;
/// the dock panel of the author (right, a tab beside Eigenschaften)
pub(crate) const P_AUTHOR: u32 = 13;
/// the author's panel: its scroll view, the buttons of the target, the start network and
/// saving; the chosen item's points (a number box: + 1 .. 3 are its own), comparison (+ k),
/// other accepted values, texts, "Entfernen" and expectation; a new check's expectation,
/// points and button; the task's texts, topic (+ k), feedback (+ k) and time limit; per check
/// its row (+ i) and its remove button (+ i)
pub(crate) const ID_AU: u32 = 0xF800;
pub(crate) const ID_AU_TARGET: u32 = 0xF801;
pub(crate) const ID_AU_START: u32 = 0xF802;
pub(crate) const ID_AU_SAVE: u32 = 0xF803;
pub(crate) const ID_AU_POINTS: u32 = 0xF804;
pub(crate) const ID_AU_CMP: u32 = 0xF808;
pub(crate) const ID_AU_ALTS: u32 = 0xF80C;
pub(crate) const ID_AU_TEXT_DE: u32 = 0xF80D;
pub(crate) const ID_AU_TEXT_EN: u32 = 0xF80E;
pub(crate) const ID_AU_REMOVE: u32 = 0xF80F;
pub(crate) const ID_AU_CHECK: u32 = 0xF810;
pub(crate) const ID_AU_NEW_CHECK: u32 = 0xF811;
pub(crate) const ID_AU_NEW_POINTS: u32 = 0xF814;
pub(crate) const ID_AU_ADD_CHECK: u32 = 0xF818;
pub(crate) const ID_AU_TITLE_DE: u32 = 0xF820;
pub(crate) const ID_AU_TITLE_EN: u32 = 0xF821;
pub(crate) const ID_AU_SUMMARY_DE: u32 = 0xF822;
pub(crate) const ID_AU_SUMMARY_EN: u32 = 0xF823;
pub(crate) const ID_AU_INTRO_DE: u32 = 0xF824;
pub(crate) const ID_AU_INTRO_EN: u32 = 0xF825;
pub(crate) const ID_AU_TOPIC: u32 = 0xF828;
pub(crate) const ID_AU_FEEDBACK: u32 = 0xF830;
pub(crate) const ID_AU_MINUTES: u32 = 0xF834;
pub(crate) const ID_AU_CHECK0: u32 = 0xF840;
pub(crate) const ID_AU_CHECK_X0: u32 = 0xF8C0;
/// checks the author's panel lists at most
pub(crate) const AUTHOR_CHECKS: usize = 128;
/// the Aufgabe panel: an assessment's card, its progress, "Abgeben" beside the time, the
/// question before handing in; per item its record (+ i)
pub(crate) const ID_AS_CARD: u32 = 0xF940;
pub(crate) const ID_AS_PROGRESS: u32 = 0xF941;
pub(crate) const ID_AS_SUBMIT: u32 = 0xF942;
pub(crate) const ID_AS_ASK: u32 = 0xF943;
pub(crate) const ID_AS_ITEM0: u32 = 0xF950;
/// items an assessment shows at most
pub(crate) const ASSESS_ITEMS: usize = 200;
/// the author's tree of the target's settings, last (its rows + 1 + i, at most AUTHOR_ROWS: up
/// to 0xFBFF)
pub(crate) const ID_AU_TREE: u32 = 0xFA20;
pub(crate) const AUTHOR_ROWS: usize = 478;
// ---- W1: WLAN (commands 480 .. 509, widgets 0xF400 .. 0xF7FF, timers 0x184 .. 0x187)
/// the ranges of every access point on the canvas (Ansicht › WLAN-Reichweiten)
pub(crate) const CMD_WLAN_RANGES: u32 = 480;
/// the inspector's WLAN of an access point: network k's fields at ID_WL_NET0 + 0x10 k + WL_*
/// (at most 16 networks), "Netz hinzufügen"
pub(crate) const ID_WL_NET0: u32 = 0xF400;
pub(crate) const WL_SSID: u32 = 0;
pub(crate) const WL_BAND: u32 = 1;
pub(crate) const WL_CHANNEL: u32 = 2;
pub(crate) const WL_SECURITY: u32 = 3;
pub(crate) const WL_PASS: u32 = 4;
pub(crate) const WL_SHOW: u32 = 5;
pub(crate) const WL_VLAN: u32 = 6;
pub(crate) const WL_GUEST: u32 = 7;
pub(crate) const WL_ISOLATE: u32 = 8;
pub(crate) const WL_APPLY: u32 = 9;
pub(crate) const WL_REMOVE: u32 = 10;
pub(crate) const ID_WL_ADD: u32 = 0xF500;
/// the inspector's WLAN of a station: SSID, passphrase, show it, join, leave
pub(crate) const ID_WL_JOIN_SSID: u32 = 0xF510;
pub(crate) const ID_WL_JOIN_PASS: u32 = 0xF511;
pub(crate) const ID_WL_JOIN_SHOW: u32 = 0xF512;
pub(crate) const ID_WL_JOIN_APPLY: u32 = 0xF513;
pub(crate) const ID_WL_JOIN_LEAVE: u32 = 0xF514;
/// the project's scale (world units per metre) and its button
pub(crate) const ID_WL_SCALE: u32 = 0xF520;
pub(crate) const ID_WL_SCALE_APPLY: u32 = 0xF521;
// ---- X1a: files on devices, TFTP and FTP (commands 550 .. 589, widgets 0xF100 .. 0xF3FF)
/// the answers of the Dateien tab's file dialog (workspace/devwin/files.rs)
pub(crate) const CMD_FILES_IMPORT_DONE: u32 = 550;
pub(crate) const CMD_FILES_EXPORT_DONE: u32 = 551;
/// the Dateien tab of device window k: ID_FILES0 + 0x180 k (+ 1 .. 4 its buttons and the text,
/// + 0x10 the grid with its rows)
pub(crate) const ID_FILES0: u32 = 0xF100;
/// its file dialog: a block of its own like ID_PCAP_DLG (.. + 0x3FFF)
pub(crate) const ID_FILES_DLG: u32 = 0x0016_0000;

// ---- K1: rewinding with keyframes (commands 420 .. 449, widgets 0xEC00 .. 0xEFFF, timers
// 0x174 .. 0x177)
/// "Von hier fortsetzen" (the hint above the canvas, Simulation): its question first
pub(crate) const CMD_SIM_RESUME: u32 = 420;
/// the question's answer: the state shown becomes the present
pub(crate) const CMD_SIM_RESUME_GO: u32 = 421;
/// the view at a moment of the recording, arg: ms of virtual time (scripts; CMD_SIM_SEEK takes
/// ns up to 4,29 s)
pub(crate) const CMD_SIM_SEEK_MS: u32 = 422;
/// the question "Von hier fortsetzen?"
pub(crate) const ID_SIM_RESUME_ASK: u32 = 0xEC00;

// ---- P1: the physical view (commands 380 .. 419, widgets 0xE000 .. 0xEBFF, timers 0x170 ..
// 0x173 unused, no dock panel: P_PHYS = 14 stays free)
/// Ansicht › Logische Ansicht (Ctrl+Shift+L) / Physische Ansicht (Ctrl+Shift+P)
pub(crate) const CMD_VIEW_LOGICAL: u32 = 380;
pub(crate) const CMD_VIEW_PHYS: u32 = 381;
/// the physical view: one level up (Backspace), back (Shift+Backspace), open the selected place, rename
/// it, remove it; add a place of kind workspace::phys::KINDS[k] to the place shown (+ k)
pub(crate) const CMD_PHYS_UP: u32 = 382;
pub(crate) const CMD_PHYS_BACK: u32 = 383;
pub(crate) const CMD_PHYS_OPEN: u32 = 384;
pub(crate) const CMD_PHYS_RENAME: u32 = 385;
pub(crate) const CMD_PHYS_REMOVE: u32 = 386;
pub(crate) const CMD_PHYS_ADD0: u32 = 387;
/// the toolbar's switch Logisch | Physisch (+ 1 + i: its segments)
pub(crate) const ID_TB_VIEW: u32 = 0xE000;
/// the physical view: the plan (ui::diagram), back, up, rename and remove the selected place,
/// the breadcrumbs (+ 1 + i), the buttons that add a place (+ k), the rename flyout (a popup
/// id) and its widgets
pub(crate) const ID_PH_CANVAS: u32 = 0xE010;
pub(crate) const ID_PH_BACK: u32 = 0xE011;
pub(crate) const ID_PH_UP: u32 = 0xE012;
pub(crate) const ID_PH_RENAME: u32 = 0xE013;
pub(crate) const ID_PH_REMOVE: u32 = 0xE014;
pub(crate) const ID_PH_CRUMB: u32 = 0xE020;
pub(crate) const ID_PH_ADD0: u32 = 0xE030;
pub(crate) const ID_PH_FLY: u32 = 0xE040;
pub(crate) const ID_PH_FLY_BOX: u32 = 0xE041;
pub(crate) const ID_PH_FLY_OK: u32 = 0xE042;
pub(crate) const ID_PH_FLY_CANCEL: u32 = 0xE043;
/// the inspector (workspace/phys/props.rs): a place's name, its fields (+ k: east, south,
/// width, depth, rack units, storey), Setzen, Öffnen, Entfernen; a device's place, its rack
/// unit and Setzen, Im Plan zeigen; a cable's length by hand, Länge setzen, Automatisch; the
/// optic of cage i (+ i, at most 64)
pub(crate) const ID_PHY_NAME: u32 = 0xE050;
pub(crate) const ID_PHY_FIELD0: u32 = 0xE058;
pub(crate) const ID_PHY_SET: u32 = 0xE05F;
pub(crate) const ID_PHY_OPEN: u32 = 0xE060;
pub(crate) const ID_PHY_REMOVE: u32 = 0xE061;
pub(crate) const ID_PHY_PLACE: u32 = 0xE068;
pub(crate) const ID_PHY_UNIT: u32 = 0xE069;
pub(crate) const ID_PHY_UNIT_SET: u32 = 0xE06A;
pub(crate) const ID_PHY_SHOW: u32 = 0xE06B;
pub(crate) const ID_PHY_LEN: u32 = 0xE070;
pub(crate) const ID_PHY_LEN_SET: u32 = 0xE071;
pub(crate) const ID_PHY_LEN_AUTO: u32 = 0xE072;
pub(crate) const ID_PHY_OPTIC0: u32 = 0xE080;

// ---- X1c: mail (widgets 0x0001_2000 .. 0x0001_2FFF; commands 640 .. 669 and timers 0x198 ..
// 0x19B are reserved, none used yet)
/// the E-Mail program on the Desktop of device window k: ID_MAIL0 + 0x400 k .. + 0x3FF
/// (devwin/email.rs)
pub(crate) const ID_MAIL0: u32 = 0x0001_2000;

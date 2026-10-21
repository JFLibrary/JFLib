// consts -- constants shared by the PaintCraft modules  [apps/PaintCraft/inc/*.inc]
//
// Generated from the .inc files (values evaluated; the struct offsets DOC_*, LY_*, CO_*,
// DV_*, HD_*, HE_*, HT_*, TD_*, TC_*, TL_*, NB_*, BEP_*, SEL_*, PL_*, RP_*, DR_*, ST_*,
// IP_*, FS_* (pn_fx) are struct fields in Rust).  Ids (CMD_ / ID_ / WINID_ / TMR_) are u32,
// everything else i32.  Module-private constants of a .s file live in its .rs file.
// SM_COUNT (selection modes) is also a ui name (style metrics): write crate::app::constants::SM_COUNT.
#![allow(dead_code)]


// ====================================================================== pn.inc
// ---- cross-module API (main.s)
pub(crate) const ZOOM_PRESETS_N: i32 = 28;
pub(crate) const WND_TOOLS: i32 = 0;
pub(crate) const WND_HISTORY: i32 = 1;
pub(crate) const WND_LAYERS: i32 = 2;
pub(crate) const WND_COLORS: i32 = 3;
pub(crate) const WND_COUNT: i32 = 4;
pub(crate) const DM_AUTO: i32 = 0; // default layout
pub(crate) const DM_ANCHOR: i32 = 1; // moved inside the main window: distances from two edges
pub(crate) const DM_FREE: i32 = 2; // dragged out of the main window

// ====================================================================== pn_ids.inc
// ---- Datei 100-149 -> fio_cmd (fileio.s)
pub(crate) const CMD_NEW: u32 = 100; // Neu...                        Strg+N
pub(crate) const CMD_OPEN: u32 = 101; // Öffnen...                     Strg+O
pub(crate) const CMD_SAVE: u32 = 102; // Speichern                     Strg+S
pub(crate) const CMD_SAVE_AS: u32 = 103; // Speichern unter...            Strg+Umschalt+S
pub(crate) const CMD_SAVE_ALL: u32 = 104; // Alles speichern               Strg+Alt+S
pub(crate) const CMD_CLOSE: u32 = 105; // Schließen                     Strg+W
pub(crate) const CMD_PRINT: u32 = 106; // Drucken...                    Strg+P
pub(crate) const CMD_EXIT: u32 = 107; // Beenden                       Strg+Q
pub(crate) const CMD_ACQUIRE: u32 = 108; // system acquisition: WIA / SANE
pub(crate) const CMD_RECENT_NONE: u32 = 109; // "(Keine)" row of Zuletzt geöffnet (always disabled)
pub(crate) const CMD_RECENT_CLEAR: u32 = 110; // Liste leeren
pub(crate) const CMD_RECENT0: u32 = 111; // 111..120: recent file rows (label -> pn_cmd_label, CF_HIDDEN unused)
pub(crate) const RECENT_MAX: i32 = 10;
pub(crate) const CMD_FILE_FIRST: u32 = 100;
pub(crate) const CMD_FILE_LAST: u32 = 149; // 121..149 free for fileio (drop, save options, ...)
// ---- PR1 (print.s): routed to prt_cmd by fio_cmd
pub(crate) const CMD_PRT_DONE: u32 = 140; // print dialog closed: arg 1 = Drucken…, 0 = Abbrechen
// ---- end PR1
// ---- P1 (plugins.s): 145..149 are routed to plug_cmd before fio_cmd (main.s route_tab)
pub(crate) const CMD_P1_FIRST: u32 = 145;
pub(crate) const CMD_P1_PROBE: u32 = 145; // test hook: arg = plugin record -> #0x82401
pub(crate) const CMD_P1_LAST: u32 = 149;
// ---- end P1
// ---- Bearbeiten 150-219
pub(crate) const CMD_UNDO: u32 = 150; // Rückgängig                    Strg+Z
pub(crate) const CMD_REDO: u32 = 151; // Wiederholen                   Strg+Y
pub(crate) const CMD_HIST_CLEAR: u32 = 152; // Verlauf löschen (spec HIST-017; not in the pdn 5.1 menus)
pub(crate) const CMD_HIST_REWIND: u32 = 153; // Zurück zum Anfang (history window)
pub(crate) const CMD_HIST_FFWD: u32 = 154; // Vor bis zum Ende (history window)
pub(crate) const CMD_HIST_JUMP: u32 = 155; // arg = history index (history window list)
pub(crate) const CMD_HIST_FIRST: u32 = 150;
pub(crate) const CMD_HIST_LAST: u32 = 159;
pub(crate) const CMD_CUT: u32 = 160; // Ausschneiden                  Strg+X
pub(crate) const CMD_COPY: u32 = 161; // Kopieren                      Strg+C
pub(crate) const CMD_COPY_MERGED: u32 = 162; // Sichtbares kopieren      Strg+Umschalt+C
pub(crate) const CMD_PASTE: u32 = 163; // Einfügen                      Strg+V
pub(crate) const CMD_PASTE_LAYER: u32 = 164; // Einfügen als neue Ebene        Strg+Umschalt+V
pub(crate) const CMD_PASTE_IMAGE: u32 = 165; // Einfügen als neues Bild        Strg+Alt+V
pub(crate) const CMD_COPY_SEL: u32 = 166; // Auswahl kopieren              Strg+Alt+Umschalt+C
pub(crate) const CMD_PASTE_SEL: u32 = 167; // Auswahl einfügen > Ersetzen   Strg+Alt+Umschalt+V
pub(crate) const CMD_PASTE_SEL_ADD: u32 = 168; // > Hinzufügen (Union)
pub(crate) const CMD_PASTE_SEL_SUB: u32 = 169; // > Ausschließen
pub(crate) const CMD_PASTE_SEL_AND: u32 = 170; // > Schneiden
pub(crate) const CMD_PASTE_SEL_XOR: u32 = 171; // > Umkehren ("xor")
pub(crate) const CMD_CLIP_FIRST: u32 = 160;
pub(crate) const CMD_CLIP_LAST: u32 = 179;
pub(crate) const CMD_SELECT_ALL: u32 = 180; // Alles auswählen               Strg+A
pub(crate) const CMD_DESELECT: u32 = 181; // Auswahl aufheben              Strg+D
pub(crate) const CMD_ERASE_SEL: u32 = 182; // Auswahl löschen               Entf
pub(crate) const CMD_FILL_SEL: u32 = 183; // Auswahl füllen                Rücktaste
pub(crate) const CMD_INVERT_SEL: u32 = 184; // Auswahl umkehren              Strg+I
pub(crate) const CMD_FEATHER_SEL: u32 = 185;
pub(crate) const CMD_SEL_FIRST: u32 = 180;
pub(crate) const CMD_SEL_LAST: u32 = 199;
pub(crate) const CMD_PAL_FOLDER: u32 = 200; // Ordner mit Paletten öffnen...
pub(crate) const CMD_PAL_SAVE: u32 = 201; // Palette speichern als...
pub(crate) const CMD_PAL_RESET: u32 = 202; // Standardpalette laden
pub(crate) const CMD_PAL_LOAD: u32 = 203; // arg = palette index (colors window palette menu)
pub(crate) const CMD_PAL_OPEN: u32 = 204; // Palette öffnen... (A3)
pub(crate) const CMD_PAL_ADD: u32 = 205; // Farbe hinzufügen (A3; no menu row, Colors window + button)
pub(crate) const CMD_PAL_FIRST: u32 = 200;
pub(crate) const CMD_PAL_LAST: u32 = 219;
// ---- Ansicht 220-269 -> view_cmd (view.s; now canvas_stub.s)
pub(crate) const CMD_ZOOM_IN: u32 = 220; // Vergrößern                    Strg++ (also Strg+Num+)
pub(crate) const CMD_ZOOM_OUT: u32 = 221; // Verkleinern                   Strg+- (also Strg+Num-)
pub(crate) const CMD_ZOOM_WINDOW: u32 = 222; // Ganzes Bild                   Strg+B
pub(crate) const CMD_ZOOM_SEL: u32 = 223; // Auf Auswahl zoomen
pub(crate) const CMD_ZOOM_ACTUAL: u32 = 224; // Originalgröße                 Strg+0 (also Strg+Umschalt+A)
pub(crate) const CMD_GRID: u32 = 225; // Pixel-Raster (toggle)
pub(crate) const CMD_RULERS: u32 = 226; // Lineale (toggle)
pub(crate) const CMD_UNIT_PX: u32 = 227; // Pixel      (radio)
pub(crate) const CMD_UNIT_IN: u32 = 228; // Zoll       (radio)
pub(crate) const CMD_UNIT_CM: u32 = 229; // Zentimeter (radio)
pub(crate) const CMD_ZOOM_SET: u32 = 230; // arg = zoom in 1/100 % (10000 = 100 %), 0 = Ganzes Bild
pub(crate) const CMD_UNIT_SET: u32 = 231; // arg = UNIT_*
pub(crate) const CMD_VIEW_FIRST: u32 = 220;
pub(crate) const CMD_VIEW_LAST: u32 = 269;
// ---- Bild 270-319 -> iop_cmd (imgops.s)
pub(crate) const CMD_CROP: u32 = 270; // Auf Auswahl zuschneiden    Strg+Umschalt+X
pub(crate) const CMD_RESIZE: u32 = 271; // Größe ändern...               Strg+R
pub(crate) const CMD_CANVAS_SIZE: u32 = 272; // Leinwandgröße...              Strg+Umschalt+R
pub(crate) const CMD_FLIP_H: u32 = 273; // Horizontal spiegeln
pub(crate) const CMD_FLIP_V: u32 = 274; // Vertikal spiegeln
pub(crate) const CMD_ROT_CW: u32 = 275; // 90° nach rechts drehen            Strg+H
pub(crate) const CMD_ROT_CCW: u32 = 276; // 90° nach links drehen  Strg+G
pub(crate) const CMD_ROT_180: u32 = 277; // Um 180° drehen                            Strg+J
pub(crate) const CMD_COLOR_PROFILE: u32 = 278; // Farbprofil... (dialog with Zuweisen / Konvertieren)
pub(crate) const CMD_FLATTEN: u32 = 279; // Zusammenfassen                Strg+Umschalt+F
pub(crate) const CMD_PROFILE_ASSIGN: u32 = 280; // (inside the colour profile dialog)
pub(crate) const CMD_PROFILE_CONVERT: u32 = 281;
// ---- I1 (colormgmt.s): arg of 280 / 281 = row of the dialog's profile list
pub(crate) const CMD_CM_IMPORT: u32 = 282; // Farbprofil dialog: Import... (file dialog)
pub(crate) const CMD_CM_EXPORT: u32 = 283; // Farbprofil dialog: Export...
pub(crate) const CMD_CM_TEST0: u32 = 290; // test hooks 290..299 (see colormgmt.s)
pub(crate) const CMD_CM_TEST_LAST: u32 = 299;
pub(crate) const WINID_DLG_PROFILE: u32 = 0x6038; // WINID_DLG0 + 40: Farbprofil dialog window
pub(crate) const ID_FD_ICC: u32 = 0x24000; // colormgmt.s file dialog (Import / Export) 0x24000-0x27FFF
// ---- end I1
pub(crate) const CMD_IMG_FIRST: u32 = 270;
pub(crate) const CMD_IMG_LAST: u32 = 319;
// ---- Ebenen 320-379 -> lay_cmd (layer.s)
pub(crate) const CMD_LAYER_ADD: u32 = 320; // Neue Ebene         Strg+Umschalt+N
pub(crate) const CMD_LAYER_DEL: u32 = 321; // Ebene löschen                 Strg+Umschalt+Entf
pub(crate) const CMD_LAYER_DUP: u32 = 322; // Ebene verdoppeln              Strg+Umschalt+D
pub(crate) const CMD_LAYER_MERGE: u32 = 323; // Nach unten vereinen  Strg+M
pub(crate) const CMD_LAYER_IMPORT: u32 = 324; // Als Ebene importieren...
pub(crate) const CMD_LAYER_FLIP_H: u32 = 325; // Horizontal spiegeln
pub(crate) const CMD_LAYER_FLIP_V: u32 = 326; // Vertikal spiegeln
pub(crate) const CMD_LAYER_ROT_180: u32 = 327; // Um 180° rotieren
pub(crate) const CMD_LAYER_ROTZOOM: u32 = 328; // Rotationszoom...              Strg+Umschalt+Z
pub(crate) const CMD_LAYER_GO_TOP: u32 = 329; // Oberste Ebene wählen       Strg+Alt+Bild auf
pub(crate) const CMD_LAYER_GO_UP: u32 = 330; // Gehe zu Ebene oben            Alt+Bild auf
pub(crate) const CMD_LAYER_GO_DOWN: u32 = 331; // Gehe zu Ebene unten           Alt+Bild ab
pub(crate) const CMD_LAYER_GO_BOTTOM: u32 = 332; // Unterste Ebene wählen      Strg+Alt+Bild ab
pub(crate) const CMD_LAYER_TO_TOP: u32 = 333; // Ebene ganz nach oben
pub(crate) const CMD_LAYER_UP: u32 = 334; // Ebene eine Stufe höher
pub(crate) const CMD_LAYER_DOWN: u32 = 335; // Ebene eine Stufe tiefer
pub(crate) const CMD_LAYER_TO_BOTTOM: u32 = 336; // Ebene ganz nach unten
pub(crate) const CMD_LAYER_TOGGLE_VIS: u32 = 337; // Ebene ein-/ausblenden  Strg+,
pub(crate) const CMD_LAYER_PROPS: u32 = 338; // Ebeneneigenschaften...       F4
pub(crate) const CMD_LAYER_SELECT: u32 = 339; // arg = layer index (layers window)
pub(crate) const CMD_LAYER_SET_VIS: u32 = 340; // arg = layer index (eye button in the layers window)
pub(crate) const CMD_LAYER_REORDER: u32 = 341; // arg = from | to << 16 (drag in the layers window)
pub(crate) const CMD_LAYER_FIRST: u32 = 320;
pub(crate) const CMD_LAYER_LAST: u32 = 379;
// ---- Korrekturen 380-439 -> fx_cmd (fx.s)
pub(crate) const CMD_ADJ_AUTO: u32 = 380; // Automatisch (Auto-Level)      Strg+Umschalt+L
pub(crate) const CMD_ADJ_BW: u32 = 381; // Schwarzweiß                   Strg+Umschalt+G
pub(crate) const CMD_ADJ_BRICON: u32 = 382; // Helligkeit / Kontrast...      Strg+Umschalt+T (unverified)
pub(crate) const CMD_ADJ_CURVES: u32 = 383; // Kurven...                     Strg+Umschalt+M
pub(crate) const CMD_ADJ_EXPOSURE: u32 = 384; // Belichtung...
pub(crate) const CMD_ADJ_HISHADOW: u32 = 385; // Tiefen / Lichter...
pub(crate) const CMD_ADJ_HUESAT: u32 = 386; // Farbton / Sättigung...        Strg+Umschalt+U
pub(crate) const CMD_ADJ_INV_ALPHA: u32 = 387; // Alpha umkehren
pub(crate) const CMD_ADJ_INVERT: u32 = 388; // Farben umkehren               Strg+Umschalt+I
pub(crate) const CMD_ADJ_LEVELS: u32 = 389; // Manuelle Anpassung... (Levels)  Strg+L
pub(crate) const CMD_ADJ_POSTERIZE: u32 = 390; // Tontrennung...                Strg+Umschalt+P
pub(crate) const CMD_ADJ_SEPIA: u32 = 391; // Sepia...                      Strg+Umschalt+E
pub(crate) const CMD_ADJ_TEMPTINT: u32 = 392; // Farbtemperatur / Tönung...
pub(crate) const CMD_ADJ_BALANCE: u32 = 393;
pub(crate) const CMD_ADJ_BW_PLUS: u32 = 394;
pub(crate) const CMD_ADJ_PLUGIN0: u32 = 400; // 400..439: plugin adjustments (fx.s registers them)
pub(crate) const CMD_ADJ_FIRST: u32 = 380;
pub(crate) const CMD_ADJ_LAST: u32 = 439;
// ---- Effekte 440-699 -> fx_cmd (fx.s)
pub(crate) const CMD_FX_REPEAT: u32 = 440; // "<Effekt> wiederholen"        Strg+F
pub(crate) const CMD_FX_PLUGIN_ERRORS: u32 = 441; // Plugin-Fehler...
pub(crate) const CMD_FX_INK_SKETCH: u32 = 450; // Skizze nachzeichnen...
pub(crate) const CMD_FX_OIL: u32 = 451; // Ölgemälde...
pub(crate) const CMD_FX_PENCIL_SKETCH: u32 = 452; // Bleistiftskizze...
pub(crate) const CMD_FX_DREAM: u32 = 453;
pub(crate) const CMD_FX_OIL_PLUS: u32 = 454;
pub(crate) const CMD_FX_PASTEL: u32 = 455;
pub(crate) const CMD_FX_FEATHER: u32 = 551;
pub(crate) const CMD_FX_INNER_SHADOW: u32 = 552;
pub(crate) const CMD_FX_BEVEL: u32 = 553;
pub(crate) const CMD_FX_DODGE_BURN: u32 = 576;
pub(crate) const CMD_FX_CONTENT_FILL: u32 = 577;
pub(crate) const CMD_FX_PORTRAIT: u32 = 578;
pub(crate) const CMD_FX_BOKEH: u32 = 470; // Bokeh...
pub(crate) const CMD_FX_FRAGMENT: u32 = 471; // Verwackelung...
pub(crate) const CMD_FX_GAUSS: u32 = 472; // Gaußscher Weichzeichner...
pub(crate) const CMD_FX_MEDIAN_BLUR: u32 = 473; // Medianunschärfe...
pub(crate) const CMD_FX_MOTION_BLUR: u32 = 474; // Bewegungsunschärfe...
pub(crate) const CMD_FX_RADIAL_BLUR: u32 = 475; // Radialer Weichzeichner...
pub(crate) const CMD_FX_SKETCH_BLUR: u32 = 476; // Skizzenunschärfe...
pub(crate) const CMD_FX_SQUARE_BLUR: u32 = 477; // Box-Weichzeichner...
pub(crate) const CMD_FX_SURFACE_BLUR: u32 = 478; // Kantenerhaltende Unschärfe...
pub(crate) const CMD_FX_UNFOCUS: u32 = 479; // Unschärfe...
pub(crate) const CMD_FX_ZOOM_BLUR: u32 = 480; // Zoom-Weichzeichner...
pub(crate) const CMD_FX_QUANTIZE: u32 = 490; // Quantize...
pub(crate) const CMD_FX_BULGE: u32 = 510; // Wölbung...
pub(crate) const CMD_FX_CRYSTALLIZE: u32 = 511; // Kristallisieren...
pub(crate) const CMD_FX_DENTS: u32 = 512; // Dellen...
pub(crate) const CMD_FX_FROSTED: u32 = 513; // Milchglas...
pub(crate) const CMD_FX_MORPHOLOGY: u32 = 514; // Morphologie...
pub(crate) const CMD_FX_PIXELATE: u32 = 515; // In Pixel auflösen...
pub(crate) const CMD_FX_POLAR_INV: u32 = 516; // Polarinvertierung...
pub(crate) const CMD_FX_TILE_REFL: u32 = 517; // Kachelspiegelung...
pub(crate) const CMD_FX_TWIST: u32 = 518; // Spirale...
pub(crate) const CMD_FX_ADD_NOISE: u32 = 530; // Rauschen hinzufügen...
pub(crate) const CMD_FX_MEDIAN: u32 = 531; // Median...
pub(crate) const CMD_FX_REDUCE_NOISE: u32 = 532; // Rauschen reduzieren...
pub(crate) const CMD_FX_DROP_SHADOW: u32 = 550; // Schlagschatten...
pub(crate) const CMD_FX_GLOW: u32 = 570; // Leuchten...
pub(crate) const CMD_FX_RED_EYE: u32 = 571; // Rote Augen entfernen...
pub(crate) const CMD_FX_SHARPEN: u32 = 572; // Verschärfen...
pub(crate) const CMD_FX_SOFTEN: u32 = 573; // Porträt weichzeichnen...
pub(crate) const CMD_FX_STRAIGHTEN: u32 = 574; // Begradigen...
pub(crate) const CMD_FX_VIGNETTE: u32 = 575; // Vignette...
pub(crate) const CMD_FX_CLOUDS: u32 = 590; // Wolken...
pub(crate) const CMD_FX_JULIA: u32 = 591; // Julia-Fraktal...
pub(crate) const CMD_FX_MANDELBROT: u32 = 592; // Mandelbrot-Fraktal...
pub(crate) const CMD_FX_TURBULENCE: u32 = 593; // Turbulenz...
pub(crate) const CMD_FX_EDGE: u32 = 610; // Kantenerkennung...
pub(crate) const CMD_FX_EMBOSS: u32 = 611; // Einprägen...
pub(crate) const CMD_FX_OUTLINE: u32 = 612; // Umriss...
pub(crate) const CMD_FX_RELIEF: u32 = 613; // Gravur...
pub(crate) const CMD_FX_PLUGIN0: u32 = 630; // 630..699: plugin effects (fx.s registers them)
pub(crate) const CMD_FX_FIRST: u32 = 440;
pub(crate) const CMD_FX_LAST: u32 = 699;
// ---- Fenster / Hilfe 700-749
pub(crate) const CMD_WND_TOOLS: u32 = 700; // Werkzeuge (toggle)            F5
pub(crate) const CMD_WND_HISTORY: u32 = 701; // Verlauf (toggle)              F6
pub(crate) const CMD_WND_LAYERS: u32 = 702; // Ebenen (toggle)               F7
pub(crate) const CMD_WND_COLORS: u32 = 703; // Farben (toggle)               F8
pub(crate) const CMD_WND_RESET: u32 = 704; // Fensteranordnung zurücksetzen
pub(crate) const CMD_WND_TRANSLUCENT: u32 = 705; // Durchscheinend (toggle)
pub(crate) const CMD_HELP: u32 = 711; // opens the Hilfe flyout        Alt+H
pub(crate) const CMD_IMG_NEXT: u32 = 706; // Nächstes Bild                 Strg+Tab
pub(crate) const CMD_IMG_PREV: u32 = 707; // Vorheriges Bild               Strg+Umschalt+Tab
pub(crate) const CMD_IMG_LIST: u32 = 708; // Offene Bilder auflisten
pub(crate) const CMD_SETTINGS: u32 = 710; // Einstellungen                 Alt+X
pub(crate) const CMD_HELP_DOCS: u32 = 720; // Dokumentation                 F1
pub(crate) const CMD_HELP_SEARCH: u32 = 721; // Suchen...
pub(crate) const CMD_HELP_FORUM: u32 = 722; // Forum
pub(crate) const CMD_HELP_TUTORIALS: u32 = 723; // Lernprogramme
pub(crate) const CMD_HELP_PLUGINS: u32 = 724; // Plug-Ins
pub(crate) const CMD_HELP_WEBSITE: u32 = 725; // Paint.NET-Webseite
pub(crate) const CMD_HELP_DONATE: u32 = 726; // Spenden
pub(crate) const CMD_HELP_FEEDBACK: u32 = 727; // Fehler melden oder Feedback geben...
pub(crate) const CMD_HELP_ABOUT: u32 = 728; // Info
pub(crate) const CMD_WND_FIRST: u32 = 700;
pub(crate) const CMD_WND_LAST: u32 = 749;
// ---- Werkzeuge 750-799 -> tool_cmd (tool.s)
pub(crate) const CMD_TOOL0: u32 = 750; // 750..768: select tool TOOL_* (pn_tool.inc)
pub(crate) const CMD_TOOLKEY_S: u32 = 770; // S  Auswahlwerkzeuge durchschalten (Rechteck, Lasso, Ellipse, Zauberstab)
pub(crate) const CMD_TOOLKEY_M: u32 = 771; // M  Pixel verschieben <-> Auswahl verschieben
pub(crate) const CMD_TOOLKEY_Z: u32 = 772; // Z  Zoomen
pub(crate) const CMD_TOOLKEY_H: u32 = 773; // H  Ansicht verschieben
pub(crate) const CMD_TOOLKEY_F: u32 = 774; // F  Farbeimer
pub(crate) const CMD_TOOLKEY_G: u32 = 775; // G  Farbverlauf
pub(crate) const CMD_TOOLKEY_B: u32 = 776; // B  Farbpinsel
pub(crate) const CMD_TOOLKEY_E: u32 = 777; // E  Radierer
pub(crate) const CMD_TOOLKEY_P: u32 = 778; // P  Zeichenstift
pub(crate) const CMD_TOOLKEY_K: u32 = 779; // K  Farbauswahl
pub(crate) const CMD_TOOLKEY_L: u32 = 780; // L  Klonen (Stempel)
pub(crate) const CMD_TOOLKEY_R: u32 = 781; // R  Neu färben
pub(crate) const CMD_TOOLKEY_T: u32 = 782; // T  Text
pub(crate) const CMD_TOOLKEY_O: u32 = 783; // O  Linie / Kurve <-> Formen
pub(crate) const CMD_SWAP_COLORS: u32 = 784; // X  Farben vertauschen
pub(crate) const CMD_RESET_COLORS: u32 = 785; // Schwarz / Weiß (colors window button)
pub(crate) const CMD_BRUSH_DEC: u32 = 786; // [  Pinsel verkleinern  (VK_OEM_4)
pub(crate) const CMD_BRUSH_INC: u32 = 787; // ]  Pinsel vergrößern     (VK_OEM_6)
pub(crate) const CMD_BRUSH_DEC5: u32 = 788; // Strg+[  um 5 verringern
pub(crate) const CMD_BRUSH_INC5: u32 = 789; // Strg+]  um 5 erhöhen
pub(crate) const CMD_TOOL_COMMIT: u32 = 790; // Fertigstellen (Eingabetaste; options bar button)
pub(crate) const CMD_TOOL_CANCEL: u32 = 791; // live tool abbrechen
pub(crate) const CMD_TOOL_FIRST: u32 = 750;
pub(crate) const CMD_TOOL_LAST: u32 = 799;
pub(crate) const CMD_TGRAD_FIRST: u32 = 792; // t_gradient.s 792-794 -> tgrad_cmd (internal / test hooks)
pub(crate) const CMD_TGRAD_LAST: u32 = 794;
pub(crate) const CMD_TSHP_FIRST: u32 = 795; // t_shapes.s   795-799 -> tshp_cmd (internal / test hooks)
pub(crate) const CMD_TSHP_LAST: u32 = 799;
// ---- Dokumente / Tabs 800-849 -> tabs_cmd (tabs.s)
pub(crate) const CMD_TAB_SELECT: u32 = 800; // arg = document index
pub(crate) const CMD_TAB_CLOSE: u32 = 801; // arg = document index
pub(crate) const CMD_TAB_COPY_PATH: u32 = 802; // Pfad kopieren          (tab context menu)
pub(crate) const CMD_TAB_OPEN_FOLDER: u32 = 803; // Speicherort öffnen  (tab context menu)
pub(crate) const CMD_TAB_MOVE: u32 = 804; // arg = from | to << 16 (drag reorder)
pub(crate) const CMD_TAB_FIRST: u32 = 800;
pub(crate) const CMD_TAB_LAST: u32 = 839;
pub(crate) const CMD_TTEXT_FIRST: u32 = 840; // t_text.s    840-844 -> ttext_cmd (internal / test hooks)
pub(crate) const CMD_TTEXT_LAST: u32 = 844;
pub(crate) const CMD_TCLONE_FIRST: u32 = 845; // t_clone.s   845-849 -> tclone_cmd (Klonen + Neu färben)
pub(crate) const CMD_TCLONE_LAST: u32 = 849;
// ---- Einstellungen 850-899 -> set_cmd (dlg_settings.s)
pub(crate) const CMD_SET_FIRST: u32 = 850;
pub(crate) const CMD_SET_LAST: u32 = 899;
// ---- Test hooks 900-999 -> diag_cmd (diag.s)
pub(crate) const CMD_TEST_FIRST: u32 = 900;
pub(crate) const CMD_TEST_LAST: u32 = 999;
pub(crate) const ID_APP_ICON: u32 = 1000;
pub(crate) const ID_TITLE_TOOLS: u32 = 1001; // (title row toggles use the command ids 700..703, 710, 711)
pub(crate) const ID_HELP_FLYOUT: u32 = 1002; // anchor / popup id of the Hilfe flyout
pub(crate) const ID_TOOL_CHOOSER: u32 = 1010; // "Werkzeug:" drop-down button (popup id of the tool menu)
pub(crate) const ID_SB_HINT: u32 = 1020; // status bar: tool hint text
pub(crate) const ID_SB_SIZE: u32 = 1021; // image size text
pub(crate) const ID_SB_CURSOR: u32 = 1022; // cursor position text
pub(crate) const ID_SB_SELSIZE: u32 = 1023; // selection size text
pub(crate) const ID_SB_UNITS: u32 = 1024; // units combo box (rows: library sub-ids)
pub(crate) const ID_SB_ZOOM: u32 = 1025; // zoom combo box
pub(crate) const ID_SB_ZOOM_SLIDER: u32 = 1026; // zoom slider
pub(crate) const ID_TOOLROW_SCROLL: u32 = 1030; // tool row: horizontal scroll view of the tool options (U3)
pub(crate) const ID_TOOLROW_BACK: u32 = 1031; // tool row: page back button
pub(crate) const ID_TOOLROW_FWD: u32 = 1032; // tool row: page forward button
pub(crate) const ID_CHROME_LAST: u32 = 1199;
pub(crate) const ID_TAB0: u32 = 1200; // + document index (up to DOC_MAX = 64)
pub(crate) const ID_TAB_CLOSE0: u32 = 1300; // + document index
pub(crate) const ID_TABS_LAST: u32 = 1399;
pub(crate) const ID_CANVAS: u32 = 1400;
pub(crate) const ID_HSCROLL: u32 = 1401;
pub(crate) const ID_VSCROLL: u32 = 1402;
pub(crate) const ID_RULER_H: u32 = 1403;
pub(crate) const ID_RULER_V: u32 = 1404;
pub(crate) const ID_VIEW_LAST: u32 = 1599;
pub(crate) const ID_FIO_FIRST: u32 = 1600;
pub(crate) const ID_FIO_LAST: u32 = 1799;
pub(crate) const ID_TOOLOPT_FIRST: u32 = 2000;
pub(crate) const ID_TOOLOPT_LAST: u32 = 2199;
pub(crate) const TOOLOPT_BASE: i32 = 2200;
pub(crate) const TOOLOPT_STRIDE: i32 = 50;
pub(crate) const ID_WTOOLS_FIRST: u32 = 4000; // wnd_tools.s   4000-4099 (tool i = 4000 + i)
pub(crate) const ID_WHIST_FIRST: u32 = 0x1004; // wnd_history.s 4100-4199 (list: ID_HIST_LIST)
pub(crate) const ID_WLAY_FIRST: u32 = 0x1068; // wnd_layers.s  4200-4399 (list: ID_LAYER_LIST)
pub(crate) const ID_WCOL_FIRST: u32 = 0x1130; // wnd_colors.s  4400-4699
pub(crate) const ID_PAL_FIRST: u32 = 0x125C; // palette.s     4700-4799 (96 swatches 4700 + i)
pub(crate) const ID_WND_LAST: u32 = 0x1387;
pub(crate) const ID_DLG_NEW: u32 = 0x1388; // dlg_new.s         5000-5099
pub(crate) const ID_DLG_RESIZE: u32 = 0x13EC; // dlg_image.s       5100-5199
pub(crate) const ID_DLG_CANVAS: u32 = 0x1450; // dlg_image.s       5200-5299
pub(crate) const ID_DLG_ROTZOOM: u32 = 0x14B4; // dlg_image.s       5300-5399
pub(crate) const ID_DLG_LAYERPROPS: u32 = 0x1518; // dlg_layerprops.s  5400-5499
pub(crate) const ID_DLG_SETTINGS: u32 = 0x157C; // dlg_settings.s    5500-5999
pub(crate) const ID_DLG_SAVEOPT: u32 = 0x1770; // dlg_saveopt.s     6000-6199
pub(crate) const ID_DLG_PRINT: u32 = 0x1838; // print.s           6200-6299
pub(crate) const ID_DLG_PROFILE: u32 = 0x189C; // colormgmt.s       6300-6399
pub(crate) const ID_DLG_MISC: u32 = 0x1900; // dlg_misc.s        6400-6799 (Info, Fehler, Plugin-Fehler, ...)
pub(crate) const ID_CRASH_FIRST: u32 = 0x1A2C; // crash.s           6700-6749 (the crash report window)
pub(crate) const ID_DLG_FX: u32 = 0x1B58; // fxui.s            7000-7999 (generic effect property dialogs)
pub(crate) const ID_DLG_CURVES: u32 = 0x1F40; // adj_curves.s      8000-8199
pub(crate) const ID_DLG_LEVELS: u32 = 0x2008; // adj_levels.s      8200-8399
pub(crate) const ID_DLG_ADJ: u32 = 0x20D0; // adj_*.s           8400-8999
pub(crate) const ID_TEST_FIRST: u32 = 0x26AC;
pub(crate) const ID_TEST_LAST: u32 = 0x270F;
pub(crate) const ID_FD_OPEN: u32 = 0x10000; // fileio.s: Öffnen
pub(crate) const ID_FD_SAVE: u32 = 0x14000; // fileio.s: Speichern unter
pub(crate) const ID_FD_IMPORT: u32 = 0x18000; // layer.s:  Als Ebene importieren
pub(crate) const ID_FD_PALETTE: u32 = 0x1C000; // palette.s
pub(crate) const ID_FD_MISC: u32 = 0x20000; // settings (temp folder, plugins ...)
pub(crate) const ID_HIST_LIST: u32 = 0x30000; // wnd_history.s list (rows 0x30001..0x3FFFF)
pub(crate) const ID_LAYER_LIST: u32 = 0x40000; // wnd_layers.s list
pub(crate) const ID_FONT_LIST: u32 = 0x50000; // t_text.s font list
pub(crate) const ID_SHAPE_LIST: u32 = 0x60000; // t_shapes.s shape gallery
pub(crate) const ID_FX_LIST: u32 = 0x70000; // fxui.s lists
pub(crate) const ID_TGRAD_FIRST: u32 = 0x80000; // t_gradient.s 0x80000-0x800FF (probe records, extra widgets)
pub(crate) const ID_TSHP_FIRST: u32 = 0x80100; // t_shapes.s + shapes_lib.s 0x80100-0x81FFF
pub(crate) const ID_TTEXT_FIRST: u32 = 0x82000; // t_text.s 0x82000-0x820FF
pub(crate) const ID_TCLONE_FIRST: u32 = 0x82100; // t_clone.s 0x82100-0x821FF
pub(crate) const ID_CLIP_FIRST: u32 = 0x82200; // clip.s (K1) 0x82200-0x822FF probes; commands 172-176 test hooks,
pub(crate) const ID_ICC_FIRST: u32 = 0x82300; // colormgmt.s (I1) 0x82300-0x823FF
pub(crate) const ID_PLUG_FIRST: u32 = 0x82400; // plugins.s (P1) 0x82400-0x824FF
pub(crate) const ID_FREE_FIRST: u32 = 0x82500; // 0x82500..0xFFFFFF: unassigned (ask the integrator)
pub(crate) const TMR_CHROME: u32 = 0x5000; // chrome.s      0x5000-0x500F
pub(crate) const TMR_VIEW: u32 = 0x5010; // view.s        0x5010-0x501F
pub(crate) const TMR_SEL: u32 = 0x5020; // sel.s         0x5020-0x502F (marching ants)
pub(crate) const TMR_TOOL: u32 = 0x5030; // tool.s, t_*.s 0x5030-0x506F
pub(crate) const TMR_TGRAD: u32 = 0x5050; // t_gradient.s  0x5050-0x5057
pub(crate) const TMR_TSHP: u32 = 0x5058; // t_shapes.s    0x5058-0x505F
pub(crate) const TMR_TTEXT: u32 = 0x5060; // t_text.s      0x5060-0x5067 (caret blink)
pub(crate) const TMR_TCLONE: u32 = 0x5068; // t_clone.s     0x5068-0x506F
pub(crate) const TMR_FX: u32 = 0x5070; // fx.s          0x5070-0x507F (preview debounce)
pub(crate) const TMR_FIO: u32 = 0x5080; // fileio.s      0x5080-0x508F
pub(crate) const TMR_HIST: u32 = 0x5090; // hist*.s       0x5090-0x509F
pub(crate) const TMR_TABS: u32 = 0x50A0; // tabs.s        0x50A0-0x50AF (thumbnail refresh)
pub(crate) const TMR_WND: u32 = 0x50B0; // wnd_*.s       0x50B0-0x50EF
pub(crate) const TMR_SET: u32 = 0x50F0; // settings      0x50F0-0x50FF
pub(crate) const TMR_CRASH: u32 = 0x5100; // crash.s       0x5100 (report window placement)
pub(crate) const WINID_TOOLS: u32 = 0x6001; // Werkzeuge
pub(crate) const WINID_HISTORY: u32 = 0x6002; // Verlauf
pub(crate) const WINID_LAYERS: u32 = 0x6003; // Ebenen
pub(crate) const WINID_COLORS: u32 = 0x6004; // Farben
pub(crate) const WINID_DLG0: u32 = 0x6010; // 0x6010-0x60FF: dialogs hosted in their own windows

// ====================================================================== pn_tool.inc
pub(crate) const TOOL_RECT_SELECT: i32 = 0; // Rechteckige Auswahl        S
pub(crate) const TOOL_MOVE_PIXELS: i32 = 1; // Pixel verschieben  M
pub(crate) const TOOL_LASSO: i32 = 2; // Lasso-Auswahl              S S
pub(crate) const TOOL_MOVE_SEL: i32 = 3; // Auswahl verschieben        M M
pub(crate) const TOOL_ELLIPSE_SELECT: i32 = 4; // Elliptische Auswahl        S S S
pub(crate) const TOOL_ZOOM: i32 = 5; // Zoomen                     Z
pub(crate) const TOOL_MAGIC_WAND: i32 = 6; // Zauberstab                 S S S S
pub(crate) const TOOL_PAN: i32 = 7; // Ansicht verschieben     H
pub(crate) const TOOL_BUCKET: i32 = 8; // Farbeimer                  F
pub(crate) const TOOL_GRADIENT: i32 = 9; // Farbverlauf                G
pub(crate) const TOOL_BRUSH: i32 = 10; // Farbpinsel                 B
pub(crate) const TOOL_ERASER: i32 = 11; // Radierer                   E
pub(crate) const TOOL_PENCIL: i32 = 12; // Zeichenstift               P
pub(crate) const TOOL_PICKER: i32 = 13; // Farbauswahl                K
pub(crate) const TOOL_CLONE: i32 = 14; // Klonen (Stempel)           L
pub(crate) const TOOL_RECOLOR: i32 = 15; // Neu färben                 R
pub(crate) const TOOL_TEXT: i32 = 16; // Text                       T
pub(crate) const TOOL_LINE: i32 = 17; // Linie / Kurve              O
pub(crate) const TOOL_SHAPES: i32 = 18; // Formen                     O O
pub(crate) const TOOL_COUNT: i32 = 19;
pub(crate) const TOOL_DEFAULT: i32 = 10; // TOOL_BRUSH
pub(crate) const UNIT_PX: i32 = 0; // Pixel
pub(crate) const UNIT_IN: i32 = 1; // Zoll
pub(crate) const UNIT_CM: i32 = 2; // Zentimeter
pub(crate) const ZOOM_MIN: i32 = 100;
pub(crate) const ZOOM_MAX: i32 = 640000;
pub(crate) const ZOOM_100: i32 = 10000;
pub(crate) const DOC_MAX: i32 = 64; // open images (tabs)
pub(crate) const DEF_W: i32 = 800; // Neu: default size (paint.net: 800 x 600 at 96 dpi)
pub(crate) const DEF_H: i32 = 600;

// ====================================================================== pn_doc.inc
// ---- DOC (640 bytes)
pub(crate) const DF_MODIFIED: i32 = 1; // unsaved changes (title '*')
pub(crate) const DF_UNTITLED: i32 = 2; // never saved
pub(crate) const DF_BUSY: i32 = 4; // a background job works on the document
pub(crate) const DF_LIVE: i32 = 8; // a live tool edit is pending (not committed)
pub(crate) const DF_OPTOK: i32 = 16; // DOC_SAVEOPT was confirmed in the save dialog (Speichern asks no more)
pub(crate) const DNF_EMPTY: i32 = 1; // no "Hintergrund" layer (loaders add their layers)
pub(crate) const DNF_NOACTIVATE: i32 = 2; // do not make it the active document
// ---- DOC_VIEW block (view.s)
// ---- DOC_COMP block (compose.s)
// ---- LAYER (96 bytes)
pub(crate) const LYF_VISIBLE: i32 = 1;
pub(crate) const LYF_BG: i32 = 2; // background layer (paint.net isBackground)
pub(crate) const TILE_SH: i32 = 6; // 64 x 64 px tiles everywhere (dirty, history, masks, effects)
pub(crate) const TILE: i32 = 64;
// ---- ruler parameters (view.s -> rulers.s)
// ---- functions (doc.s, layer.s, compose.s)

// ====================================================================== pn_hist.inc
// ---- RESOURCE OWNERSHIP (doc.s / layer.s / imgops.s / compose.s MUST follow this)
// ---- SELECTION HOOKS (sel.s registers them in sel_init; defaults: no selection history)
// ---- more (hist.s / hist_mem.s / wnd_*.s)
// ---- Test hooks (diag.s block A2 -> hist_diag_cmd, ids CMD_HIST_TEST0..; see hist.s)
pub(crate) const HIST_MAX_DEFAULT: i32 = 1000; // entries per document before the oldest are dropped
pub(crate) const CMD_HIST_TEST0: u32 = 950;
pub(crate) const CMD_HIST_TEST1: u32 = 979;
pub(crate) const ID_HIST_TEST0: u32 = 0x26DE; // hidden probe texts of hist_diag_frame (9950..9979)
pub(crate) const HDF_CAPTURE: i32 = 1; // a pixel capture is open
pub(crate) const HK_BASE: i32 = 0; // first entry ("Neues Bild"): no data
pub(crate) const HK_PX: i32 = 1; // pixel tiles of one layer
pub(crate) const HK_PROPS: i32 = 2; // layer properties
pub(crate) const HK_LAYERS: i32 = 3; // LAYER array + document size / resolution
pub(crate) const HK_SEL: i32 = 4; // selection blob (sel.s hooks)
pub(crate) const HK_COMP: i32 = 5; // compound: child entries
pub(crate) const HEF_SIZE: i32 = 1; // HK_LAYERS: stores a different document size
pub(crate) const TST_RAW: i32 = 0; // 64 x 64 x 4 bytes in RAM (row pitch 256)
pub(crate) const TST_SOLID: i32 = 1; // every valid pixel = HT_PTR (low 32 bits)
pub(crate) const TST_Z: i32 = 2; // zlib stream in the zpool
pub(crate) const TST_DISK: i32 = 3; // in the spill file (zlib, or raw with HTF_STORED)
pub(crate) const HTF_STORED: i32 = 1; // disk payload is the raw tile
pub(crate) const HTF_NOZ: i32 = 2; // deflate did not help: keep raw (spill raw)
pub(crate) const TILE_BYTES: i32 = 16384;
pub(crate) const TILE_PITCH: i32 = 256;

// ====================================================================== pn_tooli.inc
// ---- framework API (tool.s; UI thread)
// ---- tool layer (the single write path, plan "Tool-Layer")
// ---- selection clip (sel.s, selection agent)
// ---- nubs (nubs.s: handles of live tools, reusable for move / shape / text tools)
// ---- brush engine (brush_engine.s; stroke coverage, reusable for clone / recolor)
// ---- TOOLDESC
pub(crate) const PNC_BASE: i32 = 0x100; // TD_CURSOR result: PaintCraft cursor `kind` (cursors.s pn_cursor:
pub(crate) const PNC_ROTATE: i32 = 24; // 0..TOOL_COUNT-1 a tool's cursor, PNC_ROTATE rotate arrows)
pub(crate) const TF_LIVE: i32 = 0x1; // live edit with nubs until Fertigstellen / Enter (button shown)
pub(crate) const TF_CLIP_SEL: i32 = 0x2; // paints through the selection clip (informational)
pub(crate) const TF_USES_WIDTH: i32 = 0x4; // brush width tool ([ ] change TC_WIDTH)
pub(crate) const TF_AA: i32 = 0x8; // has the antialiasing option
pub(crate) const TF_BLEND: i32 = 0x10; // has the blend mode option
pub(crate) const TF_FILL: i32 = 0x20; // has the fill style option
pub(crate) const TF_HARD: i32 = 0x40; // has hardness (and spacing)
pub(crate) const TF_TOL: i32 = 0x80; // tolerance + flood mode
pub(crate) const TF_SAMPLE: i32 = 0x100; // sampling Ebene / Bild
pub(crate) const TF_SELMODE: i32 = 0x200; // selection mode
pub(crate) const TF_SELQUAL: i32 = 0x400; // selection quality
pub(crate) const TF_OUTLINE: i32 = 0x800; // the framework draws the brush outline (TC_WIDTH) at the pointer
pub(crate) const TF_TEXTKEYS: i32 = 0x1000; // (T5) Enter is not "Fertigstellen" but goes to TD_KEY (text tool:
// ---- TOOLCFG (tool_cfg())
pub(crate) const TC_SIZE: i32 = 64;
pub(crate) const WIDTH_MIN: i32 = 1;
pub(crate) const WIDTH_MAX: i32 = 2000;
pub(crate) const SMP_LAYER: i32 = 0; // Ebene
pub(crate) const SMP_IMAGE: i32 = 1; // Bild (all visible layers, the composite)
pub(crate) const SM_REPLACE: i32 = 0; // Ersetzen
pub(crate) const SM_UNION: i32 = 1; // Vereinigen
pub(crate) const SM_EXCLUDE: i32 = 2; // Subtrahieren
pub(crate) const SM_INTERSECT: i32 = 3; // Schneiden
pub(crate) const SM_XOR: i32 = 4; // Exklusiv (XOR)
pub(crate) const SM_COUNT: i32 = 5;
pub(crate) const SQ_AA: i32 = 0; // Auswahl mit Kantenglättung
pub(crate) const SQ_ALIASED: i32 = 1; // Auswahl ohne Kantenglättung
pub(crate) const FS_SOLID: i32 = 0; // Volltonfarbe
pub(crate) const FS_HATCH0: i32 = 1; // FS_HATCH0 + HatchStyle (Horizontal 0 .. SolidDiamond 52)
pub(crate) const FS_COUNT: i32 = 54;
// ---- TL (tool layer)
pub(crate) const TL_SIZE: i32 = 128;
pub(crate) const TLK_NONE: i32 = 0;
pub(crate) const TLK_SOLID: i32 = 1; // colour / pattern blended with TL_MODE through the coverage
pub(crate) const TLK_ERASE: i32 = 2; // alpha *= 1 - coverage * strength (k_erase_row)
pub(crate) const TLK_SOURCE: i32 = 3; // TL_SRC blended with TL_MODE through the coverage (moved pixels,
pub(crate) const TLF_NOSEL: i32 = 1; // do not clip to the selection
// ---- sel_clip_row results
pub(crate) const SCR_NONE: i32 = 0;
pub(crate) const SCR_PART: i32 = 1;
pub(crate) const SCR_ALL: i32 = 2;
// ---- NUB (nubs.s)
pub(crate) const NK_CIRCLE: i32 = 0; // round handle (move / origin)
pub(crate) const NK_SQUARE: i32 = 1; // square handle (resize)
pub(crate) const NK_ROTATE: i32 = 2; // small round handle with a ring (rotation)
pub(crate) const NBF_HIDDEN: i32 = 1;
pub(crate) const NBF_HOVER: i32 = 2;
pub(crate) const NBF_DRAG: i32 = 4;
pub(crate) const NBF_DRAWN: i32 = 8; // internal: NB_SX / NB_SY valid
pub(crate) const NF_TL: i32 = 0;
pub(crate) const NF_T: i32 = 1;
pub(crate) const NF_TR: i32 = 2;
pub(crate) const NF_R: i32 = 3;
pub(crate) const NF_BR: i32 = 4;
pub(crate) const NF_B: i32 = 5;
pub(crate) const NF_BL: i32 = 6;
pub(crate) const NF_L: i32 = 7;
pub(crate) const NF_COUNT: i32 = 8;
// ---- brush engine parameters (BEP)
pub(crate) const BEF_AA: i32 = 1; // antialiased dabs (else hard pixel dabs)
pub(crate) const BEF_PENCIL: i32 = 2; // 1 px aliased lines (Bresenham) instead of dabs
pub(crate) const BEF_PRESSURE: i32 = 4; // pressure scales the radius
pub(crate) const BEF_SMOOTH: i32 = 8; // the path is smoothed (be_finish draws the last piece)
pub(crate) const BE_PRESSURE_ONE: i32 = 1024;
// ---- option menu flags
pub(crate) const OMF_ICONONLY: i32 = 1; // icon + chevron, the entry's label is the tooltip
pub(crate) const OMF_NOICON: i32 = 2; // text only
pub(crate) const OMF_ICON1: i32 = 4; // icons = ONE u32 icon for the button, rows without icons
// ---- widget ids (2000-2199)
pub(crate) const ID_OPT_WIDTH: u32 = 2000; // Pinselgröße number box (value = width, 0 decimals)
pub(crate) const ID_OPT_WDEC: u32 = 2001; // Pinsel verkleinern
pub(crate) const ID_OPT_WINC: u32 = 2002; // Pinsel vergrößern
pub(crate) const ID_OPT_HARD: u32 = 2010; // Härte slider
pub(crate) const ID_OPT_SPACING: u32 = 2012; // Abstand slider
pub(crate) const ID_OPT_AA: u32 = 2020; // Antialiasing drop-down (rows 2021..2022)
pub(crate) const ID_OPT_BLEND: u32 = 2030; // Füllmethode drop-down (rows 2031..2045)
pub(crate) const ID_OPT_FILL: u32 = 2050; // Fülleffekt drop-down (swatches 2051..2104)
pub(crate) const ID_OPT_TOL: u32 = 2110; // Toleranz slider
pub(crate) const ID_OPT_FLOOD: u32 = 2115; // Füllungsmodus drop-down (rows 2116..2117)
pub(crate) const ID_OPT_SAMPLE: u32 = 2120; // Probennahme Ebene / Bild (rows 2121..2122)
pub(crate) const ID_OPT_SELMODE: u32 = 2130; // Auswahlmodus (rows 2131..2135)
pub(crate) const ID_OPT_SELQUAL: u32 = 2140; // Auswahl-Qualität (rows 2141..2142)
pub(crate) const ID_OPT_SMOOTH: u32 = 2150; // Pfadglättung (rows 2151..2152)
pub(crate) const ID_OPT_PRESSURE: u32 = 2155; // (PN1) Druckempfindlichkeit (rows 2156..2157)
pub(crate) const ID_OPT_GRAD0: u32 = 2160; // t_gradient.s option widgets 2160-2169 (M5)
pub(crate) const ID_OPT_SHP0: u32 = 2170; // t_shapes.s option widgets 2170-2199 (M5)
// ---- test hooks (diag.s block T1)
pub(crate) const CMD_T1_TEST0: u32 = 930; // 930..949
pub(crate) const ID_T1_TEST0: u32 = 0x26CA; // hidden records 9930..9949

// ====================================================================== pn_sel.inc
// ---- reading (any module; sel_clip_row also on worker threads)
// ---- geometry (UI thread)
// ---- changing the selection (UI thread; record history BEFORE with hist_push_sel)
// ---- pixels / document (UI thread; each one history step, eax 1 = step)
// ---- clipboard (Auswahl kopieren / einfügen, SEL-011/012)
// ---- move tools (t_move.s)
// ---- DOC_SEL block (128 bytes)
pub(crate) const SELF_XF: i32 = 1; // a pending transform is applied (move tools)
pub(crate) const SELF_PREVIEW: i32 = 2; // a preview op is shown
// ---- POLY block (sel.s)
// ---- ids (T2)
pub(crate) const TOOLOPT_T2: i32 = 2200; // per-tool option widgets: TOOLOPT_BASE + TOOL_* * 50
pub(crate) const TMR_ANTS: u32 = 0x5020; // marching ants (sel.s)
pub(crate) const TMR_TMOVE: u32 = 0x5021; // move tools (t_move.s)
pub(crate) const CMD_T2_TEST0: u32 = 980; // test hooks 980..984, 995..999 (diag.s block T2)
pub(crate) const ID_T2_TEST0: u32 = 0x26FC; // hidden records 9980..9984

// ====================================================================== pn_fx.inc
// ---- fx.s (UI thread unless noted)
// ---- dialog modules (called by fx.s)
// ---- the running effect (g_fx)
pub(crate) const FSM_NONE: i32 = 0; // no effect
pub(crate) const FSM_DIALOG: i32 = 1; // dialog open, live preview
pub(crate) const FSM_APPLY: i32 = 2; // OK / no dialog: commit as soon as the render is complete
pub(crate) const FSP_IDLE: i32 = 0;
pub(crate) const FSP_PREPARE: i32 = 1;
pub(crate) const FSP_RENDER: i32 = 2;
pub(crate) const FSP_DONE: i32 = 3; // every tile rendered with FS_RPARAM
pub(crate) const FSP_FAILED: i32 = 4;
pub(crate) const FSK_GENERIC: i32 = 0; // fxui.s
pub(crate) const FSK_CURVES: i32 = 1; // adj_curves.s
pub(crate) const FSK_LEVELS: i32 = 2; // adj_levels.s
pub(crate) const FSK_NONE: i32 = 3; // applied at once (FXF_NO_DIALOG, Strg+F)
// ---- ids (A6)
pub(crate) const CMD_FX_OK: u32 = 442; // internal: dialog OK
pub(crate) const CMD_FX_CANCEL: u32 = 443; // internal: dialog / progress Abbrechen
pub(crate) const WINID_FX: u32 = 0x6070; // generic effect dialog
pub(crate) const WINID_CURVES: u32 = 0x6071; // Kurven
pub(crate) const WINID_LEVELS: u32 = 0x6072; // Manuelle Anpassung
pub(crate) const WINID_FXPROG: u32 = 0x6073; // "Rendern..." (applying)
pub(crate) const TMR_FX_DEBOUNCE: u32 = 0x5070; // 50 ms restart after a parameter change
pub(crate) const ID_FX_PROG: u32 = 0x1B59; // progress bar (dialog and progress window)
pub(crate) const ID_FX_PROGTXT: u32 = 0x1B5A; // ID_DLG_FX + 2
pub(crate) const ID_FX_PROP0: u32 = 0x1B68; // property i: ID_FX_PROP0 + i * 16 + part
pub(crate) const CMD_A6_TEST0: u32 = 995; // test hooks 995..999 (diag.s block A6)
pub(crate) const ID_A6_TEST0: u32 = 0x2706; // hidden records 9990..9999

// ====================================================================== pn_lay.inc
// ---- internal commands
pub(crate) const CMD_LAY_PROPS_DONE: u32 = 350; // Ebeneneigenschaften closed: arg 1 = OK, 0 = cancel
pub(crate) const CMD_LAY_RZ_DONE: u32 = 351; // Rotationszoom closed: arg 1 = OK, 0 = cancel
pub(crate) const CMD_LAY_RZ_UPDATE: u32 = 352; // recompute the Rotationszoom preview
pub(crate) const CMD_LAY_IMP_FILE: u32 = 353; // the import file dialog chose g_imp_path
pub(crate) const CMD_LAY_IMP_ANSWER: u32 = 354; // canvas question: 0 cancel, 1 enlarge, 2 keep the size
pub(crate) const CMD_LAY_FLAT_ANSWER: u32 = 355; // "Ausgeblendete Ebenen löschen?": 1 OK (flatten), 0 cancel
pub(crate) const CMD_LAY_TEST0: u32 = 370; // 370..379 test hooks (layer.s, see a4_test)
pub(crate) const CMD_LAY_TEST_LAST: u32 = 379;
// ---- windows (A4 block)
pub(crate) const WINID_LPROPS: u32 = 0x6020; // 0x6020 Ebeneneigenschaften (dlg_layerprops.s)
pub(crate) const WINID_ROTZOOM: u32 = 0x6021; // 0x6021 Rotationszoom (dlg_rotzoom.s)
pub(crate) const WINID_IMPASK: u32 = 0x6022; // 0x6022 import: image larger than the canvas
pub(crate) const WINID_FLATASK: u32 = 0x6023; // 0x6023 Zusammenfassen: hidden layers
// ---- widget ids
pub(crate) const ID_WL_ADD: u32 = 0x1069; // ID_WLAY_FIRST + 1
pub(crate) const ID_WL_DEL: u32 = 0x106A; // ID_WLAY_FIRST + 2
pub(crate) const ID_WL_DUP: u32 = 0x106B; // ID_WLAY_FIRST + 3
pub(crate) const ID_WL_MERGE: u32 = 0x106C; // ID_WLAY_FIRST + 4
pub(crate) const ID_WL_UP: u32 = 0x106D; // ID_WLAY_FIRST + 5
pub(crate) const ID_WL_DOWN: u32 = 0x106E; // ID_WLAY_FIRST + 6
pub(crate) const ID_WL_PROPS: u32 = 0x106F; // ID_WLAY_FIRST + 7
pub(crate) const ID_A4_PROBE: u32 = 0x1126; // 4390..4399 hidden probe records (layer.s)
pub(crate) const ID_LP_NAME: u32 = 0x1519; // ID_DLG_LAYERPROPS + 1
pub(crate) const ID_LP_VIS: u32 = 0x151A; // ID_DLG_LAYERPROPS + 2
pub(crate) const ID_LP_BLEND: u32 = 0x151B; // ID_DLG_LAYERPROPS + 3
pub(crate) const ID_LP_OPAC: u32 = 0x151C; // parts: number box ui_part_id(5404, 1)
pub(crate) const ID_IMP_ENLARGE: u32 = 0x154A; // ID_DLG_LAYERPROPS + 50
pub(crate) const ID_IMP_KEEP: u32 = 0x154B; // ID_DLG_LAYERPROPS + 51
pub(crate) const ID_FLAT_TEXT: u32 = 0x1554; // ID_DLG_LAYERPROPS + 60
pub(crate) const ID_RZ_ANGLE: u32 = 0x14B5; // dial
pub(crate) const ID_RZ_ANGLE_NUM: u32 = 0x14B6; // number box (degrees)
pub(crate) const ID_RZ_PAN: u32 = 0x14B7; // ID_DLG_ROTZOOM + 3
pub(crate) const ID_RZ_ZOOM: u32 = 0x14B8; // slider (log2 of the factor)
pub(crate) const ID_RZ_TILE: u32 = 0x14B9; // ID_DLG_ROTZOOM + 5
pub(crate) const ID_RZ_KEEP: u32 = 0x14BA; // ID_DLG_ROTZOOM + 6
pub(crate) const ID_RZ_SAMPLE: u32 = 0x14BB; // ID_DLG_ROTZOOM + 7
pub(crate) const ID_RZ_ZOOM_NUM: u32 = 0x14BC; // number box (the factor)
// ---- timers
pub(crate) const TMR_WLAY: u32 = 0x50D0; // layers window: thumbnail refresh
// ---- misc
pub(crate) const LNAME_CAP: i32 = 512; // layer names (bytes incl. the NUL)
pub(crate) const LAY_PATH_CAP: i32 = 1040;

// ====================================================================== pn_iop.inc
// ---- internal commands
pub(crate) const CMD_IOP_RESIZE_DO: u32 = 300; // Größe ändern: OK (parameters in g_iop_par)
pub(crate) const CMD_IOP_CANVAS_DO: u32 = 301; // Leinwandgröße: OK (parameters in g_iop_par)
pub(crate) const CMD_IOP_TEST0: u32 = 310; // 310..319 test hooks (imgops.s iop_test)
pub(crate) const CMD_IOP_TEST_LAST: u32 = 319;
// ---- windows (A5 block)
pub(crate) const WINID_RESIZE: u32 = 0x6028; // 0x6028 Größe ändern
pub(crate) const WINID_CANVAS: u32 = 0x6029; // 0x6029 Leinwandgröße
// ---- transforms (iop_xform)
pub(crate) const XK_FLIPH: i32 = 0; // Horizontal spiegeln           (CMD_FLIP_H + kind = command)
pub(crate) const XK_FLIPV: i32 = 1; // Vertikal spiegeln
pub(crate) const XK_ROTCW: i32 = 2; // 90° nach rechts drehen
pub(crate) const XK_ROTCCW: i32 = 3; // 90° nach links drehen
pub(crate) const XK_ROT180: i32 = 4; // Um 180° drehen
pub(crate) const XK_COUNT: i32 = 5;
// ---- parameter record (IOPPAR)
pub(crate) const AN_TL: i32 = 0;
pub(crate) const AN_T: i32 = 1;
pub(crate) const AN_TR: i32 = 2;
pub(crate) const AN_L: i32 = 3;
pub(crate) const AN_M: i32 = 4;
pub(crate) const AN_R: i32 = 5;
pub(crate) const AN_BL: i32 = 6;
pub(crate) const AN_B: i32 = 7;
pub(crate) const AN_BR: i32 = 8;
pub(crate) const FILLC_SECONDARY: i32 = 0; // Sekundärfarbe (default)
pub(crate) const FILLC_PRIMARY: i32 = 1; // Grundfarbe
pub(crate) const FILLC_TRANSPARENT: i32 = 2; // Transparent
pub(crate) const FILLC_BLACK: i32 = 3; // Schwarz
pub(crate) const FILLC_WHITE: i32 = 4; // Weiß
// ---- widget ids
pub(crate) const DI_PCT_RB: i32 = 1; // radio "Prozentual:"
pub(crate) const DI_PCT: i32 = 2; // number box percent
pub(crate) const DI_ABS_RB: i32 = 3; // radio "Absolut:"
pub(crate) const DI_KEEP: i32 = 4; // check box "Seitenverhältnis beibehalten"
pub(crate) const DI_W: i32 = 5; // number box width (Pixel)
pub(crate) const DI_H: i32 = 6; // number box height (Pixel)
pub(crate) const DI_RES: i32 = 7; // number box (f64, 2 decimals) Pixeldichte
pub(crate) const DI_RUNIT: i32 = 8; // combo box Pixel/in | Pixel/cm (linked with DI_PUNIT)
pub(crate) const DI_PW: i32 = 9; // number box (f64) print width
pub(crate) const DI_PUNIT: i32 = 10; // combo box Zoll | Zentimeter
pub(crate) const DI_PH: i32 = 11; // number box (f64) print height
pub(crate) const ID_RS_FILTER: u32 = 0x13F8; // combo box Interpolation
pub(crate) const ID_RS_GAMMA: u32 = 0x13F9; // check box Gammakorrektur anwenden
pub(crate) const ID_CS_ANCHOR: u32 = 0x145C; // combo box Anker
pub(crate) const ID_CS_FILL: u32 = 0x145D; // combo box Fülleffekt
pub(crate) const ID_CS_CELL0: u32 = 0x1464; // anchor grid cells 5220 .. 5228 (AN_* order)
pub(crate) const ID_IOP_PROBE: u32 = 0x1446; // 5190..5199 hidden probe records (imgops.s)

// ====================================================================== pn_set.inc
// ---- settings.s (persistence)
pub(crate) const SET_VERSION: i32 = 1; // [PaintCraft] version = 1 (the INI layout)
// ---- commands 850..899 (CMD_SET_FIRST..LAST)
pub(crate) const CMD_SET_SAVE: u32 = 850; // 850 save the settings file now (test hook)
pub(crate) const CMD_SET_RELOAD: u32 = 851; // 851 factory state + load the file (round trip)
pub(crate) const CMD_SET_FACTORY: u32 = 852; // 852 factory state of everything (test hook)
pub(crate) const CMD_SET_PAGE: u32 = 853; // 853 arg = SP_*: open the dialog on that page
pub(crate) const CMD_SET_FROMBAR: u32 = 854; // 854 Werkzeuge: Aktuelle Werkzeugleiste übernehmen
pub(crate) const CMD_SET_TRESET: u32 = 855; // 855 Werkzeuge: Zurücksetzen
pub(crate) const CMD_SET_DIAGCOPY: u32 = 856; // 856 Diagnose: In die Zwischenablage kopieren
pub(crate) const CMD_SET_SCREENS: u32 = 857; // 857 test hook: st_check_screens now
// ---- windows / timers
pub(crate) const WINID_SETTINGS: u32 = 0x6030; // 0x6030 = 24624 Einstellungen
pub(crate) const TMR_SET_SCREENS: u32 = 0x50F0; // windows outside every monitor (after start-up)
pub(crate) const TMR_SET_MAX: u32 = 0x50F1; // maximize the main window after its normal rect
pub(crate) const TMR_SET_SAVE: u32 = 0x50F2; // test hook --settings-save-at MS (real windows)
pub(crate) const TMR_SET_PLACE: u32 = 0x50F3; // place the main window (after the first frame)
pub(crate) const TMR_SET_FIX: u32 = 0x50F4; // (unused since U3: the library resizes the framebuffer)
// ---- pages (list order)
pub(crate) const SP_UI: i32 = 0; // Benutzeroberfläche
pub(crate) const SP_TOOLS: i32 = 1; // Werkzeuge
pub(crate) const SP_HIST: i32 = 2; // Verlauf (own page: HIST-012..017)
pub(crate) const SP_GFX: i32 = 3; // Grafik (CPU: threads, SIMD)
pub(crate) const SP_COLOR: i32 = 4; // Farbmanagement (status only)
pub(crate) const SP_UPDATES: i32 = 5; // HTTPS release check through Python 3
pub(crate) const SP_DIAG: i32 = 6; // Diagnose
pub(crate) const SP_PLUGINS: i32 = 7; // Plugin-Fehler (placeholder until M8)
pub(crate) const SP_PEN: i32 = 8; // (PN1) Stift & Tablett
pub(crate) const SP_COUNT: i32 = 9;
// ---- widget ids 5500..5999 (ID_DLG_SETTINGS)
pub(crate) const ID_SET_NAV: u32 = 0x157C; // 5500 page list (rows 5501..5508)
pub(crate) const ID_SET_PAGEBOX: u32 = 0x1590; // 5520 scroll view of the page
pub(crate) const ID_SET_THEME: u32 = 0x159A; // 5530 Farbschema (Standard / Hell / Dunkel)
pub(crate) const ID_SET_ACCENT: u32 = 0x159B; // 5531 Akzentfarbe (hex text box)
pub(crate) const ID_SET_LANG: u32 = 0x159C; // 5532 Sprache (disabled)
pub(crate) const ID_SET_ANIM: u32 = 0x159D; // 5533 Animationen
pub(crate) const ID_SET_DEFTOOL: u32 = 0x15A4; // 5540 Standardtool
pub(crate) const ID_SET_FROMBAR: u32 = 0x15A5; // 5541 Aktuelle Werkzeugleiste übernehmen
pub(crate) const ID_SET_TRESET: u32 = 0x15A6; // 5542 Zurücksetzen
pub(crate) const ID_SET_TSUM: u32 = 0x15A7; // 5543..5547 summary rows of the default options
pub(crate) const ID_SET_BUDGET: u32 = 0x15AE; // 5550 Verlauf: memory budget (MB, 0 = automatic)
pub(crate) const ID_SET_TEMPDIR: u32 = 0x15AF; // 5551 Verlauf: folder of the spill file
pub(crate) const ID_SET_HUSE: u32 = 0x15B0; // 5552 Verlauf: memory in use (text)
pub(crate) const ID_SET_THREADS: u32 = 0x15B8; // 5560 Grafik: worker threads (0 = automatic)
pub(crate) const ID_SET_UPD_AUTO: u32 = 0x15C2; // 5570 Aktualisierung (disabled)
pub(crate) const ID_SET_UPD_BETA: u32 = 0x15C3; // 5571
pub(crate) const ID_SET_UPD_NOW: u32 = 0x15C4; // 5572
pub(crate) const ID_SET_DIAG: u32 = 0x15CC; // 5580 Diagnose text (read-only, multi-line)
pub(crate) const ID_SET_DIAGCOPY: u32 = 0x15CD; // 5581 In die Zwischenablage kopieren
pub(crate) const ID_SET_PEN: u32 = 0x15D6; // 5590 (PN1) Stift & Tablett: Windows-Tinte
pub(crate) const ID_SET_PENDEV: u32 = 0x15D7; // 5591 (PN1) devices text, value = count (-1 unknown)
pub(crate) const ID_SET_PROBE: u32 = 0x170C; // 5900..5919, see docs/SETTINGS.md §6
// ---- g_st (settings.s) -- values edited by the dialog
pub(crate) const ST_PATH_CAP: i32 = 1040;
pub(crate) const THEME_LIGHT: i32 = 0;
pub(crate) const THEME_DARK: i32 = 1;
pub(crate) const THEME_SYSTEM: i32 = 2;
pub(crate) const BUDGET_MB_MAX: i32 = 262144; // 256 GB
pub(crate) const THREADS_MAX: i32 = 64; // THR_MAX

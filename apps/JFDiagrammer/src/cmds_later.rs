// cmds_later -- (JD8 rest) the commands that stay without a function in phase 2, each with its
// reason (FEATURES.md rates them P2 or P3), and the check that every other command works
//
//   LATER                  (command, FEATURES id and why)
//   check() -> Report      the commands without a handler that are not on LATER, and the ones on
//                          LATER that have a handler by now (the list must follow)
//   --check-commands       test aid: the report as the notice under the ribbon at start
//                          (tests/commands_check.uis expects it to be empty)
//
// Phase 2 ends with every P1 command of FEATURES working (PLAN.md, M3).  A command shown in the
// ribbon or a menu but listed here stays disabled with the QuickInfo "Noch nicht verfügbar".

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::commands::*;

/// the commands that wait for phase 2b or 3, with the reason
pub(crate) static LATER: &[(u32, &str)] = &[
    (COMMENT_NEW, "HL-04, RU-05: Kommentare sind P2"),
    (SYMBOL, "RE-14: Symbol (Sonderzeichen) ist P2"),
    (SCREEN_TIP, "RE-11, HL-03: QuickInfo am Shape ist P2"),
    (STENCIL_EDIT, "SL-08, RK-08: Schablone bearbeiten ist P2"),
    (STENCIL_NEW_MASTER, "SL-08, RK-08: Neues Master-Shape ist P2"),
    (STENCIL_ORDER, "SL-08: Reihenfolge der Schablonen ist P2"),
    (VAR_COLORS, "RW-06, TH-05: eigene Farbsätze sind P2 (die Variantengalerie selbst geht)"),
    (VAR_EFFECTS, "RW-06, TH-05: Effektsätze sind P2"),
    (VAR_CONNECTORS, "RW-06, TH-05: Verbindersätze sind P2"),
    (VAR_FONTS, "RW-06, TH-05: Schriftsätze sind P2"),
    (VAR_EMBELLISHMENT, "RW-06, TH-05: Verzierungsgrad ist P2"),
    // (JD18) Register Daten: was P3 ist (die Datengrafiken kamen mit JD19)
    (DATA_QUICK_IMPORT, "RD-01, DA-06: Schnellimport ist P3 (Benutzerdefinierter Import geht)"),
    (DG_AUTO_SPACE, "RD-06: Automatischer Abstand ist P3"),
];

/// what check() found
pub(crate) struct Report {
    /// commands without a handler that are not on LATER
    pub missing: Vec<&'static str>,
    /// commands on LATER that work by now
    pub stale: Vec<&'static str>,
}

/// every command works or is on LATER
pub(crate) fn check() -> Report {
    let later = |id: u32| LATER.iter().any(|l| l.0 == id);
    Report {
        missing: DEFS.iter().filter(|d| !enabled(d.id) && !later(d.id)).map(|d| d.name).collect(),
        stale: DEFS.iter().filter(|d| enabled(d.id) && later(d.id)).map(|d| d.name).collect(),
    }
}

/// the report as one line (a test aid: not translated): "Alle Befehle angeschlossen, n
/// zurückgestellt" when nothing is missing and the list is up to date
pub(crate) fn report_text() -> String {
    let r = check();
    if r.missing.is_empty() && r.stale.is_empty() {
        return format!("Alle Befehle angeschlossen, {} zurückgestellt", LATER.len());
    }
    let working = DEFS.iter().filter(|d| enabled(d.id)).count();
    let mut s = format!("{working} von {} Befehlen angeschlossen, {} zurückgestellt", DEFS.len(), LATER.len());
    if !r.missing.is_empty() {
        s.push_str(&format!("; ohne Funktion: {}", r.missing.join(", ")));
    }
    if !r.stale.is_empty() {
        s.push_str(&format!("; angeschlossen, aber zurückgestellt: {}", r.stale.join(", ")));
    }
    s
}

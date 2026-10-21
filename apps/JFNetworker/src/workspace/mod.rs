// workspace -- the main window: shell (menus, toolbar, dock, status bar), the device palette,
// the topology canvas, the inspector and the pictures they use
//
//   frame(n, ui)               the whole window (shell.rs)
//   error_text(ui, s, w)       a refusal in the error colour below a field
//
// shell.rs builds the frame and the empty panels of the simulator, palette.rs the Geräte panel,
// inspector.rs the Eigenschaften panel, canvas/ the topology, sim/ the simulation mode (the
// controls, the frames on the canvas, the Ereignisse, Paket and PDUs panels, the pcapng
// export), icons/ the device symbols and the UI icons; terminal.rs the Terminal panel and the
// windows of consoles, logview.rs the Protokoll panel, devwin/ the device windows, settings.rs
// the dialogs of the settings and the shortcuts.

pub(crate) mod canvas;
pub(crate) mod devwin;
pub(crate) mod icons;
pub(crate) mod inspector;
pub(crate) mod logview;
pub(crate) mod palette;
pub(crate) mod settings;
pub(crate) mod shell;
pub(crate) mod terminal;
pub(crate) mod sim;

use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::Net;

/// the main window, then the windows of consoles and devices
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    // what the simulator reported: console output, saved configurations, names, the log
    crate::doc::console::pump(n);
    shell::frame(n, ui);
    terminal::windows(n, ui);
    devwin::frame(n, ui);
}

/// s in the error colour (caption size), wrapped at w DIP (0: the box's width); scripts find
/// it as text:"s"
pub(crate) fn error_text(ui: &mut Ui, s: &str, w_dip: i32) {
    ui.ts_font(TS_CAPTION);
    let bw = ui.box_rect()[2];
    let w = if w_dip > 0 { ui.dp(w_dip) } else if bw > 0 { bw } else { ui.dp(240) };
    let mut lines: Vec<String> = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let cand = if line.is_empty() { String::from(word) } else { alloc::format!("{} {}", line, word) };
            if !line.is_empty() && ui.text.width(&cand) > w {
                lines.push(core::mem::take(&mut line));
                line = String::from(word);
            } else {
                line = cand;
            }
        }
        lines.push(line);
    }
    let lh = ui.ts_line(TS_CAPTION);
    let r = ui.place(w, lh * lines.len() as i32);
    let c = ui.color(T_CRIT);
    ui.ts_font(TS_CAPTION);
    let asc = ui.text.ascent();
    let lead = (lh - ui.text.ascent() - ui.text.descent()) / 2;
    for (k, l) in lines.iter().enumerate() {
        ui.emit_text(r[0], r[1] + lead + asc + lh * k as i32, l, c);
    }
    ui.sem(K_TEXT, 0, Some(s), 0);
}

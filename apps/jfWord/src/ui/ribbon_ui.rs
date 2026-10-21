// App command composition only. Sizing, icons, menus and galleries belong to ui/style.
use crate::*;

pub(super) fn action(ui: &mut Ui, id: u32, command: u32, label: &str, icon: u32, enabled: bool) {
    ui.next_size(-1, 22);
    if ui.ribbon_button(
        id,
        label,
        icon,
        ST_ALIGN_START | if enabled { 0 } else { ST_DISABLED },
    ) & EV_ACT
        != 0
    {
        ui.popup_close();
        ui.cmd_run(command, 0);
    }
}
pub(super) fn unavailable(ui: &mut Ui, id: u32, label: &str, icon: u32, large: bool, menu: bool) {
    let flags = ST_DISABLED
        | if large { ST_LARGE } else { ST_ALIGN_START }
        | if menu { ST_MENU } else { 0 };
    if !large {
        ui.next_size(-1, 22);
    }
    ui.ribbon_button(id, label, icon, flags);
    ui.tooltip("Diese Funktion ist noch nicht implementiert.");
}
pub(super) fn large(ui: &mut Ui, id: u32, label: &str, icon: u32, flags: u32) -> bool {
    ui.ribbon_large_button(id, label, icon, flags) & EV_ACT != 0
}
pub(super) fn row(ui: &mut Ui) {
    ui.next_size(-1, 66);
    ui.box_begin(0, BOX_H | BOX_FIXED, 2);
}
pub(super) fn stack(ui: &mut Ui, width: i32) {
    ui.next_size(width, 66);
    ui.box_begin(0, BOX_V | BOX_FIXED, 0);
}
pub(super) fn icon(ui: &mut Ui, id: u32, label: &str, glyph: u32, flags: u32) -> bool {
    ui.next_size(if flags & ST_MENU != 0 { 32 } else { 24 }, 22);
    ui.ribbon_button(id, label, glyph, flags | ST_ICON_ONLY) & EV_ACT != 0
}
impl Word {
    pub(crate) fn ribbon(&mut self, ui: &mut Ui) {
        match self.tab {
            1 => self.start_ribbon(ui),
            2 => self.references_ribbon(ui),
            3 => self.page_ribbon(ui),
            4 => self.view_ribbon(ui),
            5 => self.insert_ribbon(ui),
            6 => self.draw_ribbon(ui),
            7 => self.design_ribbon(ui),
            8 => self.mailings_ribbon(ui),
            9 => self.review_ribbon(ui),
            10 => self.help_ribbon(ui),
            _ => {} // File opens the full backstage surface.
        }
    }
}

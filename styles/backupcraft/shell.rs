// shell -- BackupCraft app shell parts (ui::StyleShell): navigation items with a rounded shade
// and an accent pill, wizard steps and toasts from the stock painters on the BackupCraft
// tokens; the command bar sits on the window background

use ui::consts::*;
use ui::{Sp, Stock, StyleShell, Ui};

use crate::common::fill_rrect;

pub(crate) struct Shell;
pub(crate) static SHELL: Shell = Shell;

impl StyleShell for Shell {
    fn nav_pane(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let c = ui.color(T_LAYER);
        fill_rrect(ui, r[0], r[1], r[2], r[3], 0, c);
        let h = ui.hair();
        let d = ui.color(T_DIVIDER);
        ui.draw_color(d);
        ui.draw_rect(r[0] + r[2] - h, r[1], h, r[3]);
    }
    fn nav_item(&self, ui: &mut Ui, sp: &Sp) {
        Stock.nav_item(ui, sp)
    }
}

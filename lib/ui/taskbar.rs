// taskbar -- the app's taskbar / dock button (plat::taskbar): progress, a badge, jump list tasks
//
//   taskbar_progress(TaskbarProgress) -> bool   TaskbarProgress::Normal(0.4), Paused(..),
//                                    Error(..), Indeterminate, None
//   taskbar_badge(n) -> bool         n > 0: a red dot with the number on the button (Windows:
//                                    the overlay icon drawn here at the window's dpi, "99+"
//                                    above 99; Linux: the dock's own counter); 0 removes it
//   taskbar_tasks(&[JumpTask]) -> bool   the jump list's tasks (Windows): each starts the
//                                    program with its arguments (AF_SINGLE_INSTANCE: they
//                                    arrive as UIC_OPEN); empty removes them
// Headless runs (scripts, --shot) only print what they would show (plat::tray's dry run).
// Call from the UI thread (frame / command), as everything of Ui.

use alloc::format;
use alloc::string::String;

use gfx::{Surface, FMT_PARGB};
use plat::taskbar::{self, JumpTask, TaskbarProgress};
use plat::tray::{self, TrayIcon};

use super::cmd::Gfx;
use super::consts::FONT_BOLD;
use super::Ui;

/// the badge's colours: a red dot, white digits
const BADGE_BG: u32 = 0xFFC4_2B1C;
const BADGE_FG: u32 = 0xFFFF_FFFF;

impl Ui {
    /// show progress on the app's taskbar button -> false: not shown here
    pub fn taskbar_progress(&mut self, p: TaskbarProgress) -> bool {
        tray::set_dry_run(self.headless);
        taskbar::taskbar_progress(p)
    }

    /// a badge with n on the app's taskbar button (0: none) -> false: not shown here
    pub fn taskbar_badge(&mut self, n: u32) -> bool {
        tray::set_dry_run(self.headless);
        if n != 0 && taskbar::taskbar_badge_shown(n) {
            return true; // drawn and shown already (calls every frame stay cheap)
        }
        let icon = if n == 0 { None } else { self.badge_icon(n) };
        taskbar::taskbar_badge(n, icon.as_ref())
    }

    /// the tasks of the button's jump list (empty: none) -> false: no jump list here
    pub fn taskbar_tasks(&mut self, tasks: &[JumpTask]) -> bool {
        tray::set_dry_run(self.headless);
        taskbar::taskbar_tasks(tasks)
    }

    /// the badge for n: a red circle with the number, 16 x 16 at 100 % (at most 64 px)
    pub(crate) fn badge_icon(&mut self, n: u32) -> Option<TrayIcon> {
        let side = ((16 * self.wp.rt.dpi.max(96) + 48) / 96).clamp(16, 64);
        let label = if n > 99 { String::from("99+") } else { format!("{n}") };
        let mut sf = Surface::new(side, side, FMT_PARGB);
        if sf.px.is_empty() {
            return None;
        }
        let saved = self.text.get_font();
        self.dc.begin(&sf);
        {
            let mut g = Gfx { dc: &mut self.dc, sf: &mut sf, text: &mut self.text };
            g.dc.set_color(BADGE_BG);
            g.shapes().fill_circle_fx(side << 7, side << 7, side << 7);
            // the largest bold size whose text fits inside the circle
            let mut px = side * 3 / 4;
            loop {
                g.text.set_font(FONT_BOLD, px << 8);
                if px <= 6 || g.text.width(&label) <= side - side / 4 {
                    break;
                }
                px -= 1;
            }
            let w = g.text.width(&label);
            let (asc, desc) = (g.text.ascent(), g.text.descent());
            let base = (side + asc - desc + 1) / 2;
            g.dc.set_color(BADGE_FG);
            g.text.draw(g.dc, g.sf, (side - w + 1) / 2, base, label.as_bytes());
        }
        self.dc.end();
        self.text.set_font(saved.0, saved.1);
        super::tray::icon_of(&sf)
    }
}

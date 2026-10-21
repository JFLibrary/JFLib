// Office icon atlas: shared renderer acceptance at several sizes, themes and DPIs.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;
mod typography;
mod rendering_checks;
use ui::*;
use ui::keys::{IC_OFFICE_PASTE, IC_OFFICE_DOCUMENT};

const DESC: AppDesc = AppDesc {
    title: "Office icon atlas", w: 1280, h: 1560,
    cmds: &[Command::new(3, "Typography comparison", 0, 0, 0, 0)],
    flags: AF_RESIZABLE, ..AppDesc::NONE
};
#[derive(Default)]
struct Atlas {
    probe_open: Shared<i32>,
    typography: bool,
    type_fonts: [u32; 6],
}
impl App for Atlas {
    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            1 => { ui.accent_set(arg); true },
            2 => { self.probe_open.set(arg as i32); true },
            3 => {
                self.typography = arg != 0;
                if self.typography && self.type_fonts[0] == 0 {
                    for (i, bytes) in typography::FONTS.iter().enumerate() {
                        self.type_fonts[i] = if bytes.starts_with(b"SGF1") { ui.text.register(bytes) } else { ui.text.load_mem(bytes, 0) } as u32;
                    }
                }
                true
            },
            _ => false,
        }
    }
    fn frame(&mut self, ui: &mut Ui) {
        if self.typography {
            ui.next_size(-1, -1);
            ui.canvas(500, Some(alloc::rc::Rc::new(typography::Probe { dpi: ui.dpi() as u32, fonts: self.type_fonts, dark: ui.theme_dark() })), 0, ui.dpi() as u32 | ((ui.theme_dark() as u32) << 16));
            return;
        }
        let foreground=ui.color(T_TEXT);
        let disabled=ui.color(T_TEXT_DISABLED);
        ui.draw_color(foreground);
        ui.cmd_text(ui.dp(12),ui.dp(22),b"Office icons: 16 / 20 / 32 DIP + disabled",FONT_REG,ui.dp(16)<<8,foreground,0);
        ui.draw_color(ui.color(T_ACCENT));
        ui.draw_rect(ui.dp(400),ui.dp(4),ui.dp(80),ui.dp(24));
        ui.draw_color(0xffffffff);
        ui.draw_icon(ui::keys::IC_BACK,ui.dp(408),ui.dp(8),ui.dp(16)<<8);
        ui.draw_icon(ui::keys::IC_FORWARD,ui.dp(440),ui.dp(8),ui.dp(16)<<8);
        for (index,cp) in (IC_OFFICE_PASTE..=IC_OFFICE_DOCUMENT).enumerate() {
            let x=12+(index%9) as i32*140;
            let y=44+(index/9) as i32*52;
            for (offset,size) in [(0,16),(25,20),(56,32)] {
                ui.draw_color(foreground);
                ui.draw_icon(cp,ui.dp(x+offset),ui.dp(y+16-size/2),ui.dp(size)<<8);
            }
            ui.draw_color(disabled);
            ui.draw_icon(cp,ui.dp(x+98),ui.dp(y+6),ui.dp(20)<<8);
            let label=alloc::format!("{:04X}",cp);
            ui.cmd_text(ui.dp(x+2),ui.dp(y+43),label.as_bytes(),FONT_REG,ui.dp(10)<<8,foreground,0);
        }
        if ui.window_begin(100,"Palette probe",Some(&self.probe_open),WF_TOOL,None) {
            // Its drawing commands remain identical across accent changes. The
            // separate saved WinParts must still be invalidated by theme_build.
            ui.draw_color(foreground);
            ui.draw_icon(IC_OFFICE_PASTE,ui.dp(24),ui.dp(44),ui.dp(32)<<8);
            ui.draw_icon(ui::keys::IC_OFFICE_SOURCES,ui.dp(72),ui.dp(44),ui.dp(32)<<8);
        }
        ui.window_end();
    }
}
ui::main!({
    if plat::args().iter().any(|a| a == "--render-checks") { rendering_checks::run() }
    else { ui::launch(&DESC,Atlas::default(),&style::STYLE) }
});

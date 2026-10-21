// icons -- BackupCraft's domain icons (servers, agents, repositories, tape, shield, ...): code
// points in Supplementary Private Use Area-B, drawn by the icon hook from the vector art of
// icons_data.rs (mkicons.py), so they look the same in every style

use alloc::rc::Rc;

#[path = "icons_data.rs"]
mod icons_data;

pub const BC_SERVER: u32 = 0x10_F000;
pub const BC_AGENT: u32 = 0x10_F001;
pub const BC_REPOSITORY: u32 = 0x10_F002;
pub const BC_DISK: u32 = 0x10_F003;
pub const BC_TAPE: u32 = 0x10_F004;
pub const BC_VM: u32 = 0x10_F005;
pub const BC_SHIELD: u32 = 0x10_F006;
pub const BC_SHIELD_CHECK: u32 = 0x10_F007;
pub const BC_SHIELD_FILL: u32 = 0x10_F008;
pub const BC_CLOUD: u32 = 0x10_F009;
pub const BC_NETWORK: u32 = 0x10_F00A;
pub const BC_KEY: u32 = 0x10_F00B;
pub const BC_CERT: u32 = 0x10_F00C;
pub const BC_ACTIVITY: u32 = 0x10_F00D;
pub const BC_DASHBOARD: u32 = 0x10_F00E;
pub const BC_JOB: u32 = 0x10_F00F;
pub const BC_RESTORE: u32 = 0x10_F010;
pub const BC_REPLICATE: u32 = 0x10_F011;
pub const BC_USERS: u32 = 0x10_F012;
pub const BC_LOG: u32 = 0x10_F013;
pub const BC_TERMINAL: u32 = 0x10_F014;
pub const BC_PAIR: u32 = 0x10_F015;
pub const BC_SNAPSHOT: u32 = 0x10_F016;
pub const BC_BUG: u32 = 0x10_F017;
pub const BC_ARCHIVE: u32 = 0x10_F018;
pub const BC_CLOCK: u32 = 0x10_F019;
pub const BC_COPY_JOB: u32 = 0x10_F01A;
pub const BC_PLUG: u32 = 0x10_F01B;
pub const BC_GAUGE: u32 = 0x10_F01C;
pub const BC_EXIT: u32 = 0x10_F01D;
pub const BC_MOON: u32 = 0x10_F01E;
pub const BC_SUN: u32 = 0x10_F01F;
pub const BC_LANGUAGE: u32 = 0x10_F020;

/// install the hook that draws the BC_* icons
pub fn install(ui: &mut ui::Ui) {
    ui.icon_hook(Some(Rc::new(|g: &mut ui::Gfx, cp: u32, x: i32, y: i32, size: i32, argb: u32| {
        match icons_data::art(cp) {
            Some((art, stroke)) => ui::paint_svg_icon(g, art, x, y, size, argb, stroke),
            None => false,
        }
    })));
}

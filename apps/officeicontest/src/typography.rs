// English strings present in the supplied PNGs, isolated for raster comparison.
use ui::{Draw, Gfx};
pub(super) const FONTS: [&[u8]; 6] = [
    include_bytes!("../../../styles/jfoffice/assets/font_reg.bin"),
    include_bytes!("../../../styles/jfoffice/assets/font_medium.bin"),
    include_bytes!("../../../styles/jfoffice/assets/font_bold.bin"),
    include_bytes!("../../../styles/jfoffice/assets/Inter-Regular.ttf"),
    include_bytes!("../../../styles/jfoffice/assets/Inter-Medium.ttf"),
    include_bytes!("../../../styles/jfoffice/assets/Inter-SemiBold.ttf"),
];
pub(super) struct Probe { pub dpi: u32, pub fonts: [u32; 6], pub dark: bool }
impl Draw for Probe {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let samples = [
            ("Navigation", 2, 15 * 256),
            ("Layout", 1, 12 * 256 + 128),
            ("Design", 1, 12 * 256 + 128),
            ("AutoSave", 0, 11 * 256 + 128),
            ("Clipboard", 0, 11 * 256),
            ("Good morning", 2, 26 * 256),
            ("Blank document", 0, 12 * 256 + 128),
            ("AaBbCcDd", 0, 14 * 256),
        ];
        let dp = |x: i32| (x * self.dpi as i32 + 48) / 96;
        g.dc.set_color(if self.dark { 0xff1f1f1f } else { 0xfffafafa });
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        for (col, raster) in [text::FontRaster::Legacy, text::FontRaster::Natural].iter().enumerate() {
            for (row, (s, font, size)) in samples.iter().enumerate() {
                let face = self.fonts[*font as usize + if col == 1 { 3 } else { 0 }];
                let font = &face;
                let saved = g.text.font_raster(*font);
                g.text.set_font_raster(*font, *raster);
                g.text.set_font(*font, size * self.dpi as i32 / 96);
                g.dc.set_color(if self.dark { 0xffeeeeee } else { 0xff242424 });
                g.text.draw(g.dc, g.sf, r[0] + dp(16 + col as i32 * 260), r[1] + dp(40 + row as i32 * 50), s.as_bytes());
                g.text.set_font_raster(*font, saved);
            }
        }
    }
}

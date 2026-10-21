// Shared page paint adapter and bounded thumbnails. Colors belong to the document.
// Extracted unchanged in operation order from jfWord's original preview renderer.

use alloc::rc::Rc;
use doc_layout::Layout;
use gfx::{Dc, FMT_PARGB, Surface};
use text::Text;
use ui::{Draw, Gfx};

use crate::{Fonts, PageTransform, page_height};

/// Paint the subtle page drop shadow before a page canvas, keeping hit geometry unchanged.
pub fn page_backdrop(ui: &mut ui::Ui, rect: [i32; 4]) {
    let grow = (ui.dp_fx(384) / 256).max(1);
    ui.cmd_rrect(rect[0] - grow, rect[1] + ui.dp(2) - grow,
        rect[2] + grow * 2, rect[3] + grow * 2, ui.dp(1), 0, 0x1f000000, 0, 0);
}

/// A page index, geometry or thumbnail size cannot be rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageError {
    /// The requested page is not in the layout.
    Page,
    /// The page has invalid physical dimensions.
    Geometry,
    /// A thumbnail exceeds the bounded allocation budget.
    Limit,
    /// The document faces are not registered in the supplied text engine.
    Fonts,
}

/// A retained page drawing usable in `Ui::canvas`, thumbnails and agent render resources.
pub struct PageDraw {
    layout: Rc<Layout>,
    index: usize,
    fonts: Fonts,
    frame: bool,
}

impl PageDraw {
    /// Retain one page and the document fonts used during pagination.
    pub fn new(layout: Rc<Layout>, index: usize, fonts: Fonts) -> Result<Self, PageError> {
        if index >= layout.pages.len() {
            return Err(PageError::Page);
        }
        if layout.settings.width == 0 || layout.settings.height == 0 {
            return Err(PageError::Geometry);
        }
        Ok(Self {
            layout,
            index,
            fonts,
            frame: true,
        })
    }

    /// Omit the page outline when a surrounding gallery card paints its own frame.
    /// Document text, colors, measurement and pagination are unchanged.
    pub fn without_frame(mut self) -> Self {
        self.frame = false;
        self
    }

    /// Render a thumbnail through exactly the page renderer used by the editor canvas.
    /// Width is limited to 2048 pixels and the resulting height to 4096 pixels.
    pub fn thumbnail(&self, text: &mut Text, width: i32) -> Result<Surface, PageError> {
        if !(1..=2048).contains(&width) {
            return Err(PageError::Limit);
        }
        let height = page_height(
            self.layout.settings.width,
            self.layout.settings.height,
            width,
        );
        if !(1..=4096).contains(&height) {
            return Err(PageError::Limit);
        }
        if !self.fonts.valid(text) {
            return Err(PageError::Fonts);
        }
        let mut surface = Surface::new(width, height, FMT_PARGB);
        let mut dc = Dc::new(width, height);
        self.draw(
            &mut Gfx {
                dc: &mut dc,
                sf: &mut surface,
                text,
            },
            [0, 0, width, height],
        );
        Ok(surface)
    }
}

impl Draw for PageDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let Some(transform) = PageTransform::new(self.layout.settings.width, r) else {
            return;
        };
        let scale = |twips: i32| transform.scale(twips);
        g.dc.set_color(0xffffffff);
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        if self.frame {
            g.dc.set_color(0xffc6c6c6);
            g.dc.fill(g.sf, r[0], r[1], r[2], 1);
            g.dc.fill(g.sf, r[0], r[1], 1, r[3]);
            g.dc.fill(g.sf, r[0] + r[2] - 1, r[1], 1, r[3]);
            g.dc.fill(g.sf, r[0], r[1] + r[3] - 1, r[2], 1);
        }
        g.dc.set_color(0xff555555);
        for separator in &self.layout.pages[self.index].separators {
            g.dc.fill(
                g.sf,
                r[0] + scale(separator.x),
                r[1] + scale(separator.y),
                scale(separator.width),
                scale(12).max(1),
            );
        }
        for cell in &self.layout.pages[self.index].cells {
            let [x,y,w,h]=cell.rect; let (left,top,right,bottom)=(r[0]+scale(x),r[1]+scale(y),r[0]+scale(x+w),r[1]+scale(y+h));
            if let Some(fill)=cell.fill {g.dc.set_color(0xff000000|fill);g.dc.fill(g.sf,left,top,right-left,bottom-top);}
        }
        // Paint fills before shared edges so a neighbour cannot erase half of a stroke.
        for cell in &self.layout.pages[self.index].cells {crate::cell_borders::draw(g,cell,transform,r);}
        let previous = g.text.get_font();
        for line in &self.layout.pages[self.index].lines {
            let y = r[1] + scale(line.top);
            if y + scale(line.height) < g.dc.clip[1] || y > g.dc.clip[3] {
                continue;
            }
            let cell = line.cell.and_then(|id| self.layout.pages[self.index].cells.iter().find(|c| c.cell == id));
            if let Some(cell) = cell {
                let [x, y, w, h] = cell.rect;
                g.dc.clip_push(r[0] + scale(x), r[1] + scale(y), scale(x+w)-scale(x), scale(y+h)-scale(y));
            }
            // Backgrounds first: a highlighted neighbour must not cover italic overhangs.
            for fragment in &line.fragments {
                let x = r[0] + scale(fragment.x);
                let y = r[1] + scale(fragment.baseline);
                let size = transform.font_size(fragment.format.display_size_half_points());
                g.text.set_font(self.fonts.get(fragment.format), size);
                if let Some(rgb) = fragment.format.highlight.rgb() {
                    g.dc.set_color(0xff000000 | rgb);
                    // Adjacent fragments share edges after zoom rounding, including spaces.
                    let right = r[0] + scale(fragment.x + fragment.width);
                    g.dc.fill(g.sf, x, y - g.text.ascent(), (right - x).max(1),
                        (g.text.ascent() + g.text.descent()).max(1));
                }
            }
            g.dc.set_color(0xff222222);
            for &bar in &line.bar_tabs {
                g.dc.fill(g.sf, r[0] + scale(bar), y, scale(10).max(1), scale(line.height).max(1));
            }
            for fragment in &line.fragments {
                let x = r[0] + scale(fragment.x);
                let y = r[1] + scale(fragment.baseline);
                let size = transform.font_size(fragment.format.display_size_half_points());
                g.text.set_font(self.fonts.get(fragment.format), size);
                g.dc.set_color(fragment.format.color);
                if fragment.leader != doc_core::TabLeader::None && fragment.width > 0 {
                    use doc_core::TabLeader;
                    let right = r[0] + scale(fragment.x + fragment.width);
                    let symbol = match fragment.leader { TabLeader::Dot => ".", TabLeader::Hyphen => "-", TabLeader::MiddleDot => "·", _ => "_" };
                    let step = g.text.width(symbol).max(1);
                    g.dc.clip_push(x, r[1] + scale(line.top), (right - x).max(0), scale(line.height).max(1));
                    if matches!(fragment.leader, TabLeader::Underscore | TabLeader::Heavy) {
                        g.dc.fill(g.sf, x, y + scale(15), (right - x).max(0), scale(if fragment.leader == TabLeader::Heavy { 30 } else { 12 }).max(1));
                    } else {
                        let mut at = x + step - x.rem_euclid(step);
                        while at + step <= right { g.text.draw(g.dc, g.sf, at, y, symbol.as_bytes()); at += step; }
                    }
                    g.dc.clip_pop();
                }
                g.text.draw(g.dc, g.sf, x, y, fragment.text.as_bytes());
                if fragment.format.underline {
                    g.dc.fill(
                        g.sf,
                        x,
                        y + (size / 256 / 12).max(1),
                        scale(fragment.width),
                        (size / 256 / 16).max(1),
                    );
                }
                if fragment.format.strike {
                    g.dc.fill(
                        g.sf,
                        x,
                        y - (g.text.ascent() / 3).max(1),
                        scale(fragment.width),
                        (size / 256 / 16).max(1),
                    );
                }
            }
            if cell.is_some() { g.dc.clip_pop(); }
        }
        g.text.set_font(previous.0, previous.1);
    }
}

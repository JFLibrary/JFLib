// dxftext -- TEXT, MTEXT and HATCH of a DXF into a scene (G6e2; the rest of the mapping in dxf.rs).  Text styles become
// render2d text styles in a substitute font of the repository (::dxf::font_substitute), widened or narrowed so that a
// sample runs as wide as in the original (decision 31); TEXT's %% decorations become lines along the laid-out glyphs;
// an MTEXT of plain text is render2d's MText, one with format codes is laid out here into single-line texts (runs of a
// font, height, width, slant and colour), lines for under-, over- and strike-through and fraction bars, paragraphs with
// their alignment and indents, tabs, stacked fractions, column breaks and static columns.  Hatches: their boundary
// paths as rings, their pattern's line families as render2d's (more than HATCH_MAX: several fills of the same rings).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::f64::consts::TAU;

use ::dxf as d;
use ::dxf::mtext::{Align, Item, StackKind};

use crate::dxf::{atan2, ellipse_of, through, Map, Ocs, Own, Props, Tgt, P};
use crate::math::sincos;
use crate::scene::{HatchLine, Shape, Style, StyleId, BY_BLOCK_COLOR, HATCH_DASH_MAX, HATCH_MAX};
use crate::text::{layout_text, Attach, HAlign, Layout, MText, Text, TextStyle, TextStyleId, VAlign, LINE_SPACING};
use crate::dxf::Family;

/// a text style of the file as drawn: the substitute's family, its width correction, the style's width, slant (radians),
/// generation flags and fixed height
#[derive(Clone, Copy)]
pub(crate) struct Sty {
    pub family: Family,
    pub corr: f64,
    pub width: f64,
    pub oblique: f64,
    pub generation: u32,
    pub height: f64,
}

fn rad(deg: f64) -> f64 {
    deg * (core::f64::consts::PI / 180.0)
}

/// a slant within render2d's 80 degrees
fn slant(r: f64) -> f64 {
    if r.is_finite() { r.clamp(-1.39, 1.39) } else { 0.0 }
}

impl Map<'_, '_> {
    /// the font of a family (added once; a family without bytes: Sans; none: None)
    fn font(&mut self, f: Family) -> Option<crate::text::FontId> {
        if let Some(v) = self.fonts.get(&f) {
            return *v;
        }
        let bytes = self.o.fonts.and_then(|src| src(f).or_else(|| src(Family::Sans)));
        let id = bytes.and_then(|b| self.s.add_font(b, 0).ok());
        if id.is_none() {
            self.note(String::from("no font for text: texts are left out"));
        }
        self.fonts.insert(f, id);
        id
    }

    /// render2d's text style of a family, a width factor and a slant (fallbacks: the other families)
    fn tstyle(&mut self, f: Family, width: f64, oblique: f64) -> Option<(TextStyleId, TextStyle)> {
        let key = (f, width.to_bits(), oblique.to_bits());
        let font = self.font(f)?;
        let mut fb = Vec::new();
        for other in [Family::Sans, Family::Serif, Family::Mono] {
            if other != f {
                if let Some(id) = self.font(other) {
                    if id != font && !fb.contains(&id) {
                        fb.push(id);
                    }
                }
            }
        }
        let mut ts = TextStyle::new(font).with_fallbacks(&fb);
        ts.width = if width > 0.0 && width.is_finite() { width } else { 1.0 };
        ts.oblique = slant(oblique);
        if let Some(&id) = self.tstyles.get(&key) {
            return Some((id, ts));
        }
        let id = self.s.add_text_style(&ts).ok()?;
        self.tstyles.insert(key, id);
        Some((id, ts))
    }

    /// how much wider than the substitute the original runs (its sample over the substitute's)
    fn correction(&mut self, sub: d::Substitute) -> f64 {
        if sub.sample_advance <= 0.0 {
            return 1.0;
        }
        let key = sub.family;
        let measured = match self.corr.get(&key) {
            Some(&m) => m,
            None => {
                let m = self.measure(key, d::SAMPLE, 1.0, 1.0, 0.0).map_or(0.0, |(w, _)| w);
                self.corr.insert(key, m);
                m
            }
        };
        if measured > 0.0 { (sub.sample_advance / measured).clamp(0.5, 2.0) } else { 1.0 }
    }

    /// a text style of the file (None: STANDARD's, else txt)
    pub(crate) fn sty(&mut self, n: Option<d::Name>) -> Sty {
        // (a text without 7: STANDARD)
        let def = n.or(self.standard).and_then(|n| self.t.style(n).cloned());
        let (font, family, width, oblique, generation, height) = match &def {
            Some(s) => (s.font.clone(), s.family.clone(), s.width, s.oblique, s.generation, s.height),
            None => (String::from("txt"), None, 1.0, 0.0, 0, 0.0),
        };
        let font = if font.trim().is_empty() && family.is_none() { String::from("txt") } else { font };
        let sub = d::font_substitute(&font, family.as_deref());
        let corr = self.correction(sub);
        if sub.shx {
            self.approx = true;
            self.note(alloc::format!("SHX fonts are drawn in a substitute of the repository ({font}: {:?}, width x {corr:.3})", sub.family));
        }
        Sty { family: sub.family, corr, width: if width > 0.0 { width } else { 1.0 }, oblique: rad(oblique), generation, height }
    }

    /// a string's advance in drawing units (height h, width factor w, slant) and its layout (left, on the baseline at the
    /// origin, unturned)
    fn measure(&mut self, f: Family, s: &str, h: f64, w: f64, obl: f64) -> Option<(f64, Layout)> {
        let (_, ts) = self.tstyle(f, 1.0, 0.0)?;
        let mut t = Text::new(s, TextStyleId(0), [0.0, 0.0]);
        t.height = Some(h);
        t.width = Some(w);
        t.oblique = Some(slant(obl));
        let lay = layout_text(self.s.fonts(), &[ts], &t).ok()?;
        let width = lay.lines.first().map_or(0.0, |l| l.width) * lay.m[0];
        Some((width, lay))
    }

    /// the cap height and descender of a family in em
    fn metrics(&mut self, f: Family) -> (f64, f64) {
        match self.font(f).and_then(|id| self.s.fonts().metrics(id)) {
            Some(m) => (m.cap_height, m.descender),
            None => (0.7, -0.2),
        }
    }

    fn color_props(&self, p: &Props, c: Option<d::Color>) -> Props {
        let mut q = p.clone();
        match c {
            Some(d::Color::ByBlock) => {
                q.by_block |= BY_BLOCK_COLOR;
                q.color = self.fg;
            }
            Some(c @ (d::Color::Aci(_) | d::Color::Rgb(_))) => {
                q.by_block &= !BY_BLOCK_COLOR;
                q.color = self.rgb(c);
            }
            _ => {}
        }
        q.dash = None;
        q.lw_px = 0.0;
        q
    }

    /// a text's style: colour, transparency, by block (no width, no pattern)
    fn text_paint(&mut self, p: &Props) -> Option<StyleId> {
        let mut q = p.clone();
        q.dash = None;
        q.lw_px = 0.0;
        q.by_block &= BY_BLOCK_COLOR;
        self.stroke(&q)
    }
}

/// a line along a layout's em coordinates (underline, overline, strike-through)
fn deco(m: &mut Map, tgt: Tgt, st: StyleId, lay: &Layout, x0: f64, x1: f64, y: f64) -> bool {
    let a = lay.to_drawing([x0, y]);
    let b = lay.to_drawing([x1, y]);
    m.put_own(tgt, st, &Own::Line(a, b))
}

/// TEXT, ATTRIB, ATTDEF (`tag`: an ATTDEF outside a block shows its tag)
pub(crate) fn text(m: &mut Map, tgt: Tgt, p: &Props, o: &Ocs, t: &d::Text, tag: bool) -> Option<bool> {
    if let (Some(mt), false) = (&t.mtext, tag) {
        return mtext(m, tgt, p, &Ocs::WCS, mt);
    }
    let raw = if tag { t.tag.clone().unwrap_or_default() } else { t.text.clone() };
    let runs = d::specials(&raw);
    let full: String = runs.iter().map(|r| r.text.as_str()).collect();
    if full.trim().is_empty() {
        return None;
    }
    let sty = m.sty(t.style);
    let h = if t.height > 0.0 { t.height } else if sty.height > 0.0 { sty.height } else { m.h.textsize };
    if !(h > 0.0) || !h.is_finite() {
        return Some(false);
    }
    let wf = t.width.filter(|w| *w > 0.0).unwrap_or(sty.width) * sty.corr;
    let obl = slant(t.oblique.map_or(sty.oblique, rad));
    let generation = t.generation.unwrap_or(sty.generation);
    let (ts, tsd) = m.tstyle(sty.family, 1.0, 0.0)?;
    let p10 = o.p(t.at);
    let p11 = t.at2.map(|q| o.p(q));
    let (mut ha, va) = (t.halign, t.valign);
    if (ha == 3 || ha == 5) && p11.is_none() {
        ha = 0;
    }
    let at = if (ha == 0 && va == 0) || ha == 3 || ha == 5 { p10 } else { p11.unwrap_or(p10) };
    let r = rad(t.rotation);
    let (sn, cs) = sincos(r);
    let dir = o.v(cs, sn);
    let mirrored = o.det() < 0.0;
    let mut x = Text::new(&full, ts, at);
    x.at2 = p11.unwrap_or(at);
    x.height = Some(h);
    x.rotation = atan2(dir[1], dir[0]);
    x.width = Some(wf);
    x.oblique = Some(obl);
    x.halign = match ha {
        1 => HAlign::Center,
        2 => HAlign::Right,
        3 => HAlign::Aligned,
        4 => HAlign::Middle,
        5 => HAlign::Fit,
        _ => HAlign::Left,
    };
    x.valign = match va {
        1 => VAlign::Bottom,
        2 => VAlign::Middle,
        3 => VAlign::Top,
        _ => VAlign::Baseline,
    };
    x.backward = generation & 2 != 0;
    x.upside_down = (generation & 4 != 0) ^ mirrored;
    let Some(st) = m.text_paint(p) else { return Some(false) };
    if !m.put(tgt, st, &Shape::Text(x)) {
        return Some(false);
    }
    // its box and its decorations from the same layout
    let mut y = x;
    y.style = TextStyleId(0);
    if let Ok(lay) = layout_text(m.s.fonts(), &[tsd], &y) {
        let b = lay.bounds();
        m.grow_by(b);
        if runs.iter().any(|r| r.underline || r.overline || r.strike) {
            let (cap, _) = m.metrics(sty.family);
            let mut at_byte = 0usize;
            let deco_st = m.stroke(&m.color_props(p, None));
            for r in &runs {
                let range = at_byte..at_byte + r.text.len();
                at_byte = range.end;
                if !(r.underline || r.overline || r.strike) || r.text.is_empty() {
                    continue;
                }
                let gs: Vec<&crate::text::Placed> = lay.glyphs.iter().filter(|g| range.contains(&(g.cluster as usize))).collect();
                let (Some(x0), Some(x1)) = (gs.iter().map(|g| g.pen[0]).reduce(f64::min), gs.iter().map(|g| g.pen[0] + g.advance).reduce(f64::max)) else { continue };
                let Some(ds) = deco_st else { continue };
                for (on, yy) in [(r.underline, -0.2 * cap), (r.overline, 1.25 * cap), (r.strike, 0.5 * cap)] {
                    if on {
                        deco(m, tgt, ds, &lay, x0, x1, yy);
                    }
                }
            }
        }
    }
    Some(true)
}

/// a piece of a laid-out MTEXT: in the MTEXT's frame (x right, y up, the first line's top at 0)
struct Piece {
    x: f64,
    y: f64,
    text: String,
    h: f64,
    w: f64,
    obl: f64,
    family: Family,
    color: Option<d::Color>,
}

struct Rule {
    a: P,
    b: P,
    color: Option<d::Color>,
}

/// MTEXT (and the MTEXT of a multi-line ATTRIB)
pub(crate) fn mtext(m: &mut Map, tgt: Tgt, p: &Props, o: &Ocs, t: &d::MText) -> Option<bool> {
    let sty = m.sty(t.style);
    let h = if t.height > 0.0 { t.height } else if sty.height > 0.0 { sty.height } else { m.h.textsize };
    if !(h > 0.0) || !h.is_finite() {
        return Some(false);
    }
    let doc = d::mtext::parse(&t.text, h);
    if doc.text().trim().is_empty() {
        return None;
    }
    let at = [t.at[0], t.at[1]];
    // the text's frame in the drawing: x along 11 (WCS), else the rotation 50 from the WCS x axis (MTEXT is no OCS entity;
    // as ezdxf reads it), y the normal times x: a normal pointing down (z < 0) mirrors, the lines run the other way round
    let ex = match t.x_dir {
        Some(x) if x[0] != 0.0 || x[1] != 0.0 => {
            let l = crate::math::sqrt(x[0] * x[0] + x[1] * x[1]);
            [x[0] / l, x[1] / l]
        }
        _ => {
            let (sn, cs) = sincos(t.rotation);
            [cs, sn]
        }
    };
    let ey = if o.det() < 0.0 { [ex[1], -ex[0]] } else { [-ex[1], ex[0]] };
    let rot = atan2(ex[1], ex[0]);
    let mirrored = ex[0] * ey[1] - ex[1] * ey[0] < 0.0;
    let attach = t.attach.clamp(1, 9);
    let wf = sty.width * sty.corr;
    let Some(paint) = m.text_paint(p) else { return Some(false) };
    let plain = if t.column_type == 0 && !mirrored { doc.plain(h) } else { None };
    if let Some(s) = plain {
        let (ts, tsd) = m.tstyle(sty.family, wf, sty.oblique)?;
        let mut x = MText::new(&s, ts, at);
        x.height = Some(h);
        x.width = if t.width > 0.0 { t.width } else { 0.0 };
        x.rotation = rot;
        x.attach = [Attach::TopLeft, Attach::TopCenter, Attach::TopRight, Attach::MiddleLeft, Attach::MiddleCenter, Attach::MiddleRight, Attach::BottomLeft, Attach::BottomCenter,
                    Attach::BottomRight][(attach - 1) as usize];
        x.line_spacing = if t.spacing > 0.0 { t.spacing } else { 1.0 };
        let mut y = x;
        y.style = TextStyleId(0);
        let lay = crate::text::layout_mtext(m.s.fonts(), &[tsd], &y).ok();
        if let (Some(lay), true) = (&lay, t.bg_flags & 1 != 0) {
            background(m, tgt, p, t, lay.quad(), h);
        }
        if !m.put(tgt, paint, &Shape::MText(x)) {
            return Some(false);
        }
        if let Some(lay) = lay {
            m.grow_by(lay.bounds());
        }
        return Some(true);
    }
    m.approx = m.approx || doc.paragraphs.iter().any(|pa| pa.align == Align::Justify || pa.align == Align::Distribute);
    // ---- the layout of runs
    let (cap_em, desc_em) = m.metrics(sty.family);
    let wrap = if t.width > 0.0 { t.width } else { f64::INFINITY };
    let col_w = if t.column_width > 0.0 { t.column_width } else if t.width > 0.0 { t.width } else { 0.0 };
    let col_h = |k: usize| -> f64 {
        if t.column_type == 0 {
            return f64::INFINITY;
        }
        t.column_heights.get(k).copied().filter(|v| *v > 0.0).or_else(|| (t.rect_height > 0.0).then_some(t.rect_height)).unwrap_or(f64::INFINITY)
    };
    let mut pieces: Vec<Piece> = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    // per line: (first piece, first rule, width, max height, paragraph's alignment, column)
    struct L {
        p0: usize,
        r0: usize,
        width: f64,
        hmax: f64,
        align: Align,
        col: usize,
        y: f64,
    }
    let mut lines: Vec<L> = Vec::new();
    let mut col = 0usize;
    let mut y = 0.0f64;
    let mut first_in_col = true;
    let max_pieces = 1usize << 16;
    for para in &doc.paragraphs {
        if para.column_break && !lines.is_empty() {
            col += 1;
            y = 0.0;
            first_in_col = true;
        }
        // the paragraph as tokens: words with the spaces after them, stacks, tabs; a word may be several runs
        enum Tok {
            Run(String, d::mtext::Format, bool),
            Stack(String, String, StackKind, d::mtext::Format),
            Tab(d::mtext::Format),
        }
        let mut toks: Vec<Tok> = Vec::new();
        for it in &para.items {
            match it {
                Item::Text(s, f) => {
                    let mut cur = String::new();
                    let mut sp = false;
                    for c in s.chars() {
                        if c == ' ' {
                            cur.push(c);
                            sp = true;
                        } else {
                            if sp {
                                toks.push(Tok::Run(core::mem::take(&mut cur), f.clone(), true));
                                sp = false;
                            }
                            cur.push(c);
                        }
                    }
                    if !cur.is_empty() {
                        toks.push(Tok::Run(cur, f.clone(), sp));
                    }
                }
                Item::Stack { top, bottom, kind, format } => toks.push(Tok::Stack(top.clone(), bottom.clone(), *kind, format.clone())),
                Item::Tab(f) => toks.push(Tok::Tab(f.clone())),
                _ => {}
            }
        }
        let indent0 = para.indent_left + para.indent_first;
        let right = wrap - para.indent_right;
        let mut x = indent0;
        let mut line = L { p0: pieces.len(), r0: rules.len(), width: 0.0, hmax: 0.0, align: para.align, col, y: 0.0 };
        let mut line_end = 0.0f64;
        let mut i = 0;
        // a word group: tokens up to one that ends in a space
        while i < toks.len() {
            let mut j = i;
            while j < toks.len() {
                let ends = match &toks[j] {
                    Tok::Run(_, _, sp) => *sp,
                    _ => true,
                };
                j += 1;
                if ends {
                    break;
                }
            }
            // the group's width (without its trailing spaces)
            let mut gw = 0.0;
            let mut widths: Vec<(f64, f64)> = Vec::new();
            for tk in &toks[i..j] {
                let (w, w_trim) = match tk {
                    Tok::Run(s, f, _) => {
                        let fam = run_family(sty.family, f);
                        let fw = f.width.unwrap_or(1.0) * wf;
                        let w = m.measure(fam, s, f.height, fw, 0.0).map_or(0.0, |r| r.0) * f.tracking;
                        let wt = if s.ends_with(' ') { m.measure(fam, s.trim_end(), f.height, fw, 0.0).map_or(0.0, |r| r.0) * f.tracking } else { w };
                        (w, wt)
                    }
                    Tok::Stack(a, b, k, f) => {
                        let hs = 0.7 * f.height;
                        let fw = f.width.unwrap_or(1.0) * wf;
                        let wa = m.measure(sty.family, a, hs, fw, 0.0).map_or(0.0, |r| r.0);
                        let wb = m.measure(sty.family, b, hs, fw, 0.0).map_or(0.0, |r| r.0);
                        let w = if *k == StackKind::Diagonal { wa + wb + 0.3 * f.height } else { wa.max(wb) } + 0.1 * f.height;
                        (w, w)
                    }
                    Tok::Tab(f) => {
                        // the next tab stop after x (the paragraph's, else every 4 heights)
                        let step = 4.0 * f.height;
                        let at = x + gw;
                        let next = para.tabs.iter().copied().find(|&s| s > at + 1e-9).unwrap_or((crate::dxf::floor(at / step) + 1.0) * step);
                        ((next - at).max(0.0), (next - at).max(0.0))
                    }
                };
                widths.push((w, w_trim));
                gw += w;
            }
            let g_trim = gw - widths.last().map_or(0.0, |w| w.0 - w.1);
            if x > para.indent_left + 1e-9 && x + g_trim > right && pieces.len() > line.p0 {
                // a new line (wrapped)
                line.width = line_end;
                lines.push(line);
                x = para.indent_left;
                line = L { p0: pieces.len(), r0: rules.len(), width: 0.0, hmax: 0.0, align: para.align, col, y: 0.0 };
            }
            for (tk, (w, w_trim)) in toks[i..j].iter().zip(widths) {
                if pieces.len() >= max_pieces {
                    break;
                }
                match tk {
                    Tok::Run(s, f, _) => {
                        let fam = run_family(sty.family, f);
                        let dy = match f.valign {
                            1 => 0.5 * (h - f.height).max(0.0),
                            2 => (h - f.height).max(0.0),
                            _ => 0.0,
                        };
                        // (italic: there is no italic font here, a slant of 12 degrees stands in)
                        let obl = f.oblique.map_or(if f.italic { rad(12.0) } else { sty.oblique }, rad);
                        pieces.push(Piece { x, y: dy, text: s.clone(), h: f.height, w: f.width.unwrap_or(1.0) * wf * f.tracking, obl, family: fam, color: f.color });
                        let x1 = x + w_trim;
                        for (on, yy) in [(f.underline, -0.2 * f.height), (f.overline, 1.25 * f.height), (f.strike, 0.5 * f.height)] {
                            if on {
                                rules.push(Rule { a: [x, dy + yy], b: [x1, dy + yy], color: f.color });
                            }
                        }
                        line.hmax = line.hmax.max(f.height);
                        line_end = x + w_trim;
                    }
                    Tok::Stack(a, b, k, f) => {
                        let hs = 0.7 * f.height;
                        let fw = f.width.unwrap_or(1.0) * wf;
                        let wa = m.measure(sty.family, a, hs, fw, 0.0).map_or(0.0, |r| r.0);
                        let wb = m.measure(sty.family, b, hs, fw, 0.0).map_or(0.0, |r| r.0);
                        let x0 = x + 0.05 * f.height;
                        let mk = |s: &str, px: f64, py: f64| Piece { x: px, y: py, text: String::from(s), h: hs, w: fw, obl: sty.oblique, family: sty.family, color: f.color };
                        match k {
                            StackKind::Diagonal => {
                                pieces.push(mk(a, x0, 0.45 * f.height));
                                let sx = x0 + wa + 0.05 * f.height;
                                rules.push(Rule { a: [sx, -0.15 * f.height], b: [sx + 0.2 * f.height, 1.15 * f.height], color: f.color });
                                pieces.push(mk(b, sx + 0.25 * f.height, -0.15 * f.height));
                            }
                            _ => {
                                let wmax = wa.max(wb);
                                let bar = 0.5 * f.height;
                                let gap = 0.15 * f.height;
                                let (ax, bx) = if *k == StackKind::Tolerance { (x0, x0) } else { (x0 + (wmax - wa) / 2.0, x0 + (wmax - wb) / 2.0) };
                                pieces.push(mk(a, ax, bar + gap));
                                pieces.push(mk(b, bx, bar - gap - hs));
                                if *k == StackKind::Bar {
                                    rules.push(Rule { a: [x0, bar], b: [x0 + wmax, bar], color: f.color });
                                }
                            }
                        }
                        line.hmax = line.hmax.max(f.height * 1.2);
                        line_end = x + w;
                    }
                    Tok::Tab(_) => {}
                }
                x += w;
            }
            i = j;
        }
        line.width = line_end;
        if line.hmax == 0.0 {
            line.hmax = h;
        }
        lines.push(line);
        // the lines of this paragraph get their baselines; a static column that is full: the next column
        let first = lines.iter().rposition(|l| l.col != col || l.y != 0.0).map_or(0, |k| k + 1);
        for k in first..lines.len() {
            let lh = lines[k].hmax.max(1e-12);
            let step = LINE_SPACING * lh * if t.spacing > 0.0 { t.spacing } else { 1.0 };
            let mut base = if first_in_col { -lh } else { y - step };
            if -base + 0.4 * lh > col_h(col) && !first_in_col {
                col += 1;
                base = -lh;
            }
            lines[k].col = col;
            lines[k].y = base;
            y = base;
            first_in_col = false;
        }
    }
    // the block's width, the attachment point's offsets
    let block_w = if t.width > 0.0 { t.width } else { lines.iter().map(|l| l.width).fold(0.0, f64::max) };
    let bottom = lines.iter().map(|l| l.y + desc_em / cap_em.max(1e-9) * l.hmax).fold(0.0, f64::min);
    let dx = match (attach - 1) % 3 {
        0 => 0.0,
        1 => -0.5 * block_w,
        _ => -block_w,
    };
    let dy = match (attach - 1) / 3 {
        0 => 0.0,
        1 => -0.5 * bottom,
        _ => -bottom,
    };
    let to = |q: P| {
        let (x, y) = (q[0] + dx, q[1] + dy);
        [at[0] + ex[0] * x + ey[0] * y, at[1] + ex[1] * x + ey[1] * y]
    };
    if t.bg_flags & 1 != 0 {
        let cols = lines.iter().map(|l| l.col).max().unwrap_or(0) as f64;
        let w_all = block_w + cols * (col_w + t.column_gutter);
        let q = [to([0.0, 0.0]), to([w_all, 0.0]), to([w_all, bottom]), to([0.0, bottom])];
        background(m, tgt, p, t, q, h);
    }
    let mut ok = false;
    let n_lines = lines.len();
    for k in 0..n_lines {
        let (p0, p1) = (lines[k].p0, if k + 1 < n_lines { lines[k + 1].p0 } else { pieces.len() });
        let (r0, r1) = (lines[k].r0, if k + 1 < n_lines { lines[k + 1].r0 } else { rules.len() });
        let align = match lines[k].align {
            Align::Default => match (attach - 1) % 3 {
                0 => Align::Left,
                1 => Align::Center,
                _ => Align::Right,
            },
            a => a,
        };
        let ox = match align {
            Align::Center => 0.5 * (block_w - lines[k].width),
            Align::Right => block_w - lines[k].width,
            _ => 0.0,
        } + lines[k].col as f64 * (col_w + t.column_gutter);
        let base = lines[k].y;
        for pc in &pieces[p0..p1] {
            if pc.text.trim().is_empty() {
                continue;
            }
            let Some((ts, tsd)) = m.tstyle(pc.family, 1.0, 0.0) else { continue };
            let q = to([pc.x + ox, base + pc.y]);
            let mut x = Text::new(&pc.text, ts, q);
            x.height = Some(pc.h);
            x.rotation = rot;
            x.width = Some(pc.w);
            x.oblique = Some(slant(pc.obl));
            x.upside_down = mirrored;
            let pp = m.color_props(p, pc.color);
            let Some(st) = m.text_paint(&pp) else { continue };
            if m.put(tgt, st, &Shape::Text(x)) {
                ok = true;
                let mut y2 = x;
                y2.style = TextStyleId(0);
                if let Ok(lay) = layout_text(m.s.fonts(), &[tsd], &y2) {
                    m.grow_by(lay.bounds());
                }
            }
        }
        for r in &rules[r0..r1] {
            let pp = m.color_props(p, r.color);
            let Some(st) = m.stroke(&pp) else { continue };
            ok |= m.put_own(tgt, st, &Own::Line(to([r.a[0] + ox, base + r.a[1]]), to([r.b[0] + ox, base + r.b[1]])));
        }
    }
    Some(ok)
}

/// a run's family: the substitute of its font (MTEXT's f and F codes; else the style's), bold where f says so
fn run_family(style: Family, f: &d::mtext::Format) -> Family {
    let fam = f.font.as_deref().map_or(style, |n| d::font_substitute(n, if f.shx { None } else { Some(n) }).family);
    if !f.bold {
        return fam;
    }
    match fam {
        Family::Sans | Family::Ui => Family::SansBold,
        Family::Serif => Family::SerifBold,
        Family::Mono => Family::MonoBold,
        b => b,
    }
}

/// MTEXT's background fill (90 bit 1; bit 2: the view's background), the text's box grown by (45 - 1) / 2 heights
fn background(m: &mut Map, tgt: Tgt, p: &Props, t: &d::MText, q: [P; 4], h: f64) {
    let color = if t.bg_flags & 2 != 0 { m.o.background | 0xFF00_0000 } else { m.rgb(t.bg_color) };
    let grow = ((if t.bg_scale > 0.0 { t.bg_scale } else { 1.5 }) - 1.0).max(0.0) * h / 2.0;
    let c = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0];
    let out: Vec<P> = q.iter().map(|v| {
        let dv = [v[0] - c[0], v[1] - c[1]];
        let l = (dv[0] * dv[0] + dv[1] * dv[1]).sqrt_approx().max(1e-300);
        [v[0] + dv[0] / l * grow * core::f64::consts::SQRT_2, v[1] + dv[1] / l * grow * core::f64::consts::SQRT_2]
    }).collect();
    let mut pp = p.clone();
    pp.color = color;
    pp.by_block = 0;
    if let Some(st) = m.fill_style(&pp) {
        m.put_fill(tgt, st, &[vec![Own::Poly(out, true)]]);
    }
}

trait SqrtApprox {
    fn sqrt_approx(self) -> f64;
}

impl SqrtApprox for f64 {
    fn sqrt_approx(self) -> f64 {
        crate::math::sqrt(self)
    }
}

// ---------------------------------------------------------------------------------------------------- hatches

/// an edge path's or polyline path's pieces in the drawing
fn path_ring(m: &mut Map, o: &Ocs, z: f64, path: &d::Path) -> Vec<Own> {
    let mut ring: Vec<Own> = Vec::new();
    let pt = |q: [f64; 2]| o.p([q[0], q[1], z]);
    for e in &path.edges {
        match e {
            d::Edge::Line { a, b } => ring.push(Own::Line(pt(*a), pt(*b))),
            d::Edge::Arc { center, radius, start, end, ccw } => {
                let span = crate::dxf::rem(end - start, 360.0);
                let span = if span < 1e-9 { 360.0 } else { span };
                // a clockwise edge stores its angles mirrored: it runs from -start clockwise
                let (s, sw) = if *ccw { (rad(*start), rad(span)) } else { (-rad(*start), -rad(span)) };
                if let Some(a) = crate::dxf::arc_shape(o, [center[0], center[1], z], *radius, s, sw) {
                    ring.push(a);
                }
            }
            d::Edge::Ellipse { center, major, ratio, start, end, ccw } => {
                let span = crate::dxf::rem(end - start, 360.0);
                let span = if span < 1e-9 { 360.0 } else { span };
                let (s, sw) = if *ccw { (rad(*start), rad(span)) } else { (-rad(*start), -rad(span)) };
                let minor = [-major[1] * ratio, major[0] * ratio];
                let c = pt(*center);
                if let Some(el) = ellipse_of(c, o.v(major[0], major[1]), o.v(minor[0], minor[1]), s, sw) {
                    ring.push(el);
                }
            }
            d::Edge::Spline { degree, rational, knots, control, weights, fit, .. } => {
                let ctrl: Vec<P> = control.iter().map(|c| pt(*c)).collect();
                let n = ctrl.len();
                let deg = *degree as usize;
                let knots_ok = knots.len() == n + deg + 1 && knots.windows(2).all(|w| w[0] <= w[1]) && n > deg && knots[deg] < knots[n];
                if (1..=3).contains(&deg) && n > deg {
                    let w = if *rational && weights.len() == n && weights.iter().all(|w| *w > 0.0) { weights.clone() } else { Vec::new() };
                    ring.push(Own::Spline(deg as u32, ctrl, w, if knots_ok { knots.clone() } else { Vec::new() }, false));
                } else if fit.len() >= 2 {
                    m.approx = true;
                    let f: Vec<P> = fit.iter().map(|c| pt(*c)).collect();
                    ring.push(Own::Cubic(through(&f, None, None, false)));
                } else if n >= 2 {
                    m.approx = true;
                    ring.push(Own::Poly(ctrl, false));
                }
            }
            d::Edge::Poly { vertices, closed } => {
                let n = vertices.len();
                let segs = if *closed { n } else { n.saturating_sub(1) };
                let mut run: Vec<P> = Vec::new();
                for i in 0..segs {
                    let (a, b) = (vertices[i], vertices[(i + 1) % n]);
                    let (pa, pb) = ([a[0], a[1]], [b[0], b[1]]);
                    match crate::dxf::bulge_of(pa, pb, a[2]) {
                        Some((c, r, s, th)) => {
                            if run.len() >= 2 {
                                ring.push(Own::Poly(core::mem::take(&mut run), false));
                            }
                            run.clear();
                            if let Some(arc) = crate::dxf::arc_shape(o, [c[0], c[1], z], r, s, th) {
                                ring.push(arc);
                            }
                        }
                        None => {
                            if run.is_empty() {
                                run.push(pt(pa));
                            }
                            run.push(pt(pb));
                        }
                    }
                }
                if run.len() >= 2 {
                    ring.push(Own::Poly(run, false));
                } else if n == 1 {
                    ring.push(Own::Poly(vec![pt([vertices[0][0], vertices[0][1]])], false));
                }
            }
            _ => {}
        }
    }
    ring
}

pub(crate) fn hatch(m: &mut Map, tgt: Tgt, p: &Props, o: &Ocs, h: &d::Hatch) -> Option<bool> {
    let z = h.elevation;
    // which paths: all (normal, odd parity), the outer ones (1: external and outermost), the external one (2: ignore)
    let keep = |f: u32| match h.style {
        1 => f & (1 | 16) != 0,
        2 => f & 1 != 0,
        _ => true,
    };
    let flagged = h.paths.iter().any(|pa| keep(pa.flags));
    let mut rings: Vec<Vec<Own>> = Vec::new();
    for pa in &h.paths {
        if flagged && !keep(pa.flags) {
            continue;
        }
        let r = path_ring(m, o, z, pa);
        if !r.is_empty() {
            rings.push(r);
        }
    }
    if rings.is_empty() {
        return Some(false);
    }
    if h.solid || h.gradient || h.lines.is_empty() {
        let mut pp = p.clone();
        if h.gradient {
            m.approx = true;
            m.note(String::from("gradient hatches: filled in their first colour"));
            if let Some(c) = h.gradient_colors.first() {
                pp.color = m.rgb(*c);
                pp.by_block &= !BY_BLOCK_COLOR;
            }
        }
        if !h.solid && !h.gradient && h.lines.is_empty() {
            // a pattern hatch without its line families (written without them): solid
            m.approx = true;
            m.note(String::from("pattern hatches without their pattern lines: filled solid"));
        }
        let st = m.fill_style(&pp)?;
        return Some(m.put_fill(tgt, st, &rings));
    }
    // the line families through the OCS
    let mut fams: Vec<HatchLine> = Vec::new();
    for l in &h.lines {
        let (sn, cs) = sincos(rad(l.angle));
        let dv = o.v(cs, sn);
        let dl = (dv[0] * dv[0] + dv[1] * dv[1]).sqrt_approx();
        if !(dl > 0.0) {
            continue;
        }
        let dir = [dv[0] / dl, dv[1] / dl];
        let nrm = [-dir[1], dir[0]];
        let off = o.v(l.offset[0], l.offset[1]);
        let (mut spacing, mut shift) = (off[0] * nrm[0] + off[1] * nrm[1], off[0] * dir[0] + off[1] * dir[1]);
        if spacing < 0.0 {
            spacing = -spacing;
            shift = -shift;
        }
        if !(spacing > 1e-12) || !spacing.is_finite() {
            m.note(String::from("a hatch line family without spacing: left out"));
            continue;
        }
        let mut base = o.p([l.base[0], l.base[1], z]);
        let mut hl = HatchLine::new(atan2(dir[1], dir[0]), spacing);
        hl.shift = shift;
        if !l.dashes.is_empty() {
            match Map::pattern(&l.dashes, dl, HATCH_DASH_MAX) {
                Some((pat, phase)) => {
                    // the rotated pattern starts `phase` before the base point
                    base = [base[0] - dir[0] * phase, base[1] - dir[1] * phase];
                    hl = hl.dashed(&pat);
                }
                None => {
                    if l.dashes.iter().all(|v| *v < 0.0) {
                        continue;
                    }
                }
            }
        }
        hl.base = base;
        fams.push(hl);
    }
    if fams.is_empty() {
        let st = m.fill_style(p)?;
        m.approx = true;
        return Some(m.put_fill(tgt, st, &rings));
    }
    if fams.len() > HATCH_MAX {
        m.approx = true;
        m.note(alloc::format!("hatch patterns of more than {HATCH_MAX} line families: several fills of the same boundary, {HATCH_MAX} families each"));
    }
    let mut ok = false;
    for chunk in fams.chunks(HATCH_MAX) {
        let mut st = Style::new(p.color).hatched(chunk, p.lw_px);
        st.by_block = p.by_block & BY_BLOCK_COLOR;
        st.transparency = p.transparency;
        let Ok(id) = m.s.add_style(&st) else { continue };
        ok |= m.put_fill(tgt, id, &rings);
    }
    let _ = TAU;
    Some(ok)
}

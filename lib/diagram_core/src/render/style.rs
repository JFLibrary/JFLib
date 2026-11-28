// style -- the tables behind lines and fills (JD4): Visio's 24 colours, line patterns, fill
// patterns, and line ends drawn by JFDiagrammer itself
//
//   PALETTE[i]                      the colours of index 0 .. 23 (the document's Colors table
//                                   comes first)
//   dash(pattern) -> &[f64]         LinePattern 2 .. 23 in multiples of the line's weight
//   fill_pattern(p) -> FillKind     FillPattern 2 .. 24 as Visio draws them (hatches, dot
//                                   patterns as a tint), 25 .. 40 as the old two-colour gradients
//   arrow(kind, size, weight)       BeginArrow / EndArrow 1 .. 45 with the size 0 (very small)
//                                   .. 6 (colossal): JFDiagrammer's own line ends, pointing to
//                                   +x with the tip at the origin, and how far the line stops
//                                   short of the tip
//
// The tables are JFDiagrammer's own approximations of what the numbers mean (FEATURES.md
// FM-01 .. FM-03); no artwork is taken from Visio.  Arrow kinds group into families (open,
// filled and hollow triangles, notched, circles, diamonds, squares, bars, crow's feet); where
// a number's look is unknown it gets the nearest family.  One place to tune them once the
// reference exports (PLAN.md, "Testen") show the differences.

use alloc::vec::Vec;

use crate::geometry::{arc_cubics, Path, Seg, Sub};

/// Visio's default colours 0 .. 23
pub const PALETTE: [u32; 24] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0,
    0xE6E6E6, 0xCDCDCD, 0xB3B3B3, 0x9A9A9A, 0x808080, 0x666666, 0x4D4D4D, 0x333333, 0x1A1A1A,
];

/// a line pattern as lengths on and off in multiples of the line's weight (at least 1 pt);
/// None: solid (1, unknown numbers, the patterns of masters above 23)
pub fn dash(pattern: u32) -> Option<&'static [f64]> {
    Some(match pattern {
        2 => &[6.0, 3.0],
        3 => &[1.0, 2.0],
        4 => &[6.0, 3.0, 1.0, 3.0],
        5 => &[6.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        6 => &[6.0, 3.0, 6.0, 3.0, 1.0, 3.0],
        7 => &[14.0, 3.0, 6.0, 3.0],
        8 => &[14.0, 3.0, 6.0, 3.0, 6.0, 3.0],
        9 => &[14.0, 6.0],
        10 => &[3.0, 1.5],
        11 => &[1.0, 1.0],
        12 => &[3.0, 1.5, 1.0, 1.5],
        13 => &[3.0, 1.5, 1.0, 1.5, 1.0, 1.5],
        14 => &[3.0, 1.5, 3.0, 1.5, 1.0, 1.5],
        15 => &[7.0, 1.5, 3.0, 1.5],
        16 => &[7.0, 1.5, 3.0, 1.5, 3.0, 1.5],
        17 => &[12.0, 6.0],
        18 => &[2.0, 6.0],
        19 => &[12.0, 6.0, 2.0, 6.0],
        20 => &[12.0, 6.0, 2.0, 6.0, 2.0, 6.0],
        21 => &[12.0, 6.0, 12.0, 6.0, 2.0, 6.0],
        22 => &[28.0, 6.0, 12.0, 6.0],
        23 => &[28.0, 6.0, 12.0, 6.0, 12.0, 6.0],
        _ => return None,
    })
}

/// what a FillPattern number draws
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FillKind {
    None,
    Solid,
    /// lines in the foreground colour over the background colour: up to two families (angle in
    /// degrees, spacing in inches)
    Hatch(&'static [(f64, f64)]),
    /// a dot pattern too fine to draw: the foreground's share (0 .. 1) mixed over the background
    Tint(f64),
    /// the old gradients from the foreground to the background colour
    Legacy(Legacy),
}

/// the old two-colour gradients (FillPattern 25 .. 40)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Legacy {
    /// along an angle (degrees, 0: left to right, 90: bottom to top), mirrored: background at
    /// both ends
    Linear(f64, bool),
    /// from a point of the box (fractions, 0..1) outwards
    Radial(f64, f64),
}

/// FillPattern 2 .. 24 as Visio draws them (JD26 Visio check: its SVG export holds each as an
/// 8 x 8 bitmap tiled every 6 pt, a pixel 1/96 inch): the line patterns as hatches with Visio's
/// directions and spacings (hatch lines are a pixel wide), the dot patterns as their share of the
/// foreground over the background; 25 .. 40 the old gradients (36 .. 39 from the corners)
pub fn fill_pattern(p: u32) -> FillKind {
    // spacings in inches: along the axes every 8, 4 or 2 pixels, across the diagonals the same
    // divided by the square root of 2
    const P8: f64 = 8.0 / 96.0;
    const P4: f64 = 4.0 / 96.0;
    const P2: f64 = 2.0 / 96.0;
    const D8: f64 = P8 * core::f64::consts::FRAC_1_SQRT_2;
    const D4: f64 = P4 * core::f64::consts::FRAC_1_SQRT_2;
    // two diagonal families covering half the area: 1 - (1 - w/s)^2 = 0.5
    const HALF: f64 = 1.0 / 96.0 / (1.0 - core::f64::consts::FRAC_1_SQRT_2);
    match p {
        0 => FillKind::None,
        1 => FillKind::Solid,
        2 => FillKind::Hatch(&[(45.0, D8)]),
        3 => FillKind::Hatch(&[(0.0, P8), (90.0, P8)]),
        4 => FillKind::Hatch(&[(45.0, D8), (135.0, D8)]),
        5 => FillKind::Hatch(&[(135.0, D8)]),
        6 => FillKind::Hatch(&[(0.0, P8)]),
        7 => FillKind::Hatch(&[(90.0, P8)]),
        8 => FillKind::Tint(0.75),
        9 => FillKind::Tint(0.5),
        10 => FillKind::Tint(0.25),
        11 => FillKind::Tint(0.125),
        12 => FillKind::Tint(0.0625),
        13 => FillKind::Hatch(&[(0.0, P2)]),
        14 => FillKind::Hatch(&[(90.0, P2)]),
        15 => FillKind::Hatch(&[(135.0, P2)]),
        16 => FillKind::Hatch(&[(45.0, P2)]),
        17 => FillKind::Hatch(&[(45.0, HALF), (135.0, HALF)]),
        18 => FillKind::Hatch(&[(45.0, P2), (135.0, P2)]),
        19 => FillKind::Hatch(&[(0.0, P4)]),
        20 => FillKind::Hatch(&[(90.0, P4)]),
        21 => FillKind::Hatch(&[(135.0, D4)]),
        22 => FillKind::Hatch(&[(45.0, D4)]),
        23 => FillKind::Hatch(&[(0.0, P4), (90.0, P4)]),
        24 => FillKind::Hatch(&[(45.0, D4), (135.0, D4)]),
        25 => FillKind::Legacy(Legacy::Linear(0.0, false)),
        26 => FillKind::Legacy(Legacy::Linear(0.0, true)),
        27 => FillKind::Legacy(Legacy::Linear(180.0, false)),
        28 => FillKind::Legacy(Legacy::Linear(270.0, false)),
        29 => FillKind::Legacy(Legacy::Linear(90.0, true)),
        30 => FillKind::Legacy(Legacy::Linear(90.0, false)),
        31 | 36 => FillKind::Legacy(Legacy::Radial(0.0, 1.0)),
        32 | 37 => FillKind::Legacy(Legacy::Radial(1.0, 1.0)),
        33 | 38 => FillKind::Legacy(Legacy::Radial(0.0, 0.0)),
        34 | 39 => FillKind::Legacy(Legacy::Radial(1.0, 0.0)),
        35 | 40 => FillKind::Legacy(Legacy::Radial(0.5, 0.5)),
        _ => FillKind::Solid,
    }
}

/// a line end in its own space: the tip at the origin, the line coming in from -x
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Arrow {
    /// filled in the line's colour
    pub fill: Path,
    /// stroked with the line's style
    pub stroke: Path,
    /// how far the line stops short of the tip
    pub setback: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum Fam {
    /// two barbs, half angle in degrees
    Open(f64),
    /// a triangle, half angle, filled
    Tri(f64, bool),
    /// a notched triangle, filled
    Notch(bool),
    /// one barb
    Half,
    Circle(bool),
    Diamond(bool),
    Square(bool),
    /// a bar across the line
    Bar,
    /// a slash across the line
    Slash,
    /// two open barbs one behind the other
    DoubleOpen,
    /// two triangles one behind the other
    DoubleTri,
    /// crow's feet: one, many, one or many, zero or many, zero or one, only one
    CrowOne,
    CrowMany,
    CrowOneMany,
    CrowZeroMany,
    CrowZeroOne,
    CrowOnlyOne,
}

fn family(kind: u32) -> Option<Fam> {
    Some(match kind {
        0 => return None,
        1 => Fam::Open(28.0),
        2 => Fam::Notch(true),
        3 => Fam::Open(40.0),
        4 => Fam::Tri(28.0, true),
        5 => Fam::Tri(18.0, true),
        6 => Fam::Tri(40.0, true),
        7 => Fam::Open(18.0),
        8 => Fam::Half,
        9 => Fam::Tri(28.0, false),
        10 => Fam::Circle(true),
        11 => Fam::Tri(40.0, false),
        12 => Fam::Diamond(true),
        13 => Fam::Tri(22.0, true),
        14 => Fam::Tri(22.0, false),
        15 => Fam::Notch(false),
        16 => Fam::Circle(false),
        17 => Fam::Square(true),
        18 => Fam::Square(false),
        19 => Fam::Diamond(true),
        20 => Fam::Circle(true),
        21 => Fam::Diamond(false),
        22 => Fam::Tri(45.0, false),
        23 => Fam::Open(55.0),
        24 => Fam::Bar,
        25 => Fam::Slash,
        26 => Fam::DoubleOpen,
        27 => Fam::DoubleTri,
        28 => Fam::Circle(false),
        29 | 35 | 41 => Fam::CrowOne,
        30 | 36 | 42 => Fam::CrowMany,
        31 | 37 | 43 => Fam::CrowOneMany,
        32 | 38 | 44 => Fam::CrowZeroMany,
        33 | 39 | 45 => Fam::CrowZeroOne,
        34 | 40 => Fam::CrowOnlyOne,
        _ => Fam::Tri(22.0, true),
    })
}

/// how long a line end of `size` (0 .. 6) is on a line of `weight` inches
pub fn arrow_length(size: u32, weight: f64) -> f64 {
    const K: [f64; 7] = [0.45, 0.7, 1.0, 1.35, 1.8, 2.6, 4.0];
    K[(size as usize).min(6)] * (0.075 + 3.0 * weight.max(0.0))
}

/// the line end `kind` (1 .. 45) of `size` on a line of `weight` inches
pub fn arrow(kind: u32, size: u32, weight: f64) -> Option<Arrow> {
    let fam = family(kind)?;
    let l = arrow_length(size, weight);
    let mut a = Arrow::default();
    let tri = |half: f64, len: f64, at: f64| {
        let w = len * tan_deg(half);
        poly(&[[at, 0.0], [at - len, w], [at - len, -w]], true)
    };
    match fam {
        Fam::Open(h) => {
            let w = l * tan_deg(h);
            a.stroke.subs.push(poly(&[[-l, w], [0.0, 0.0], [-l, -w]], false));
        }
        Fam::Tri(h, filled) => {
            let p = tri(h, l, 0.0);
            if filled {
                a.fill.subs.push(p);
            } else {
                a.stroke.subs.push(p);
            }
            a.setback = l;
        }
        Fam::Notch(filled) => {
            let w = l * tan_deg(26.0);
            let p = poly(&[[0.0, 0.0], [-l, w], [-0.7 * l, 0.0], [-l, -w]], true);
            if filled {
                a.fill.subs.push(p);
            } else {
                a.stroke.subs.push(p);
            }
            a.setback = 0.7 * l;
        }
        Fam::Half => {
            let w = l * tan_deg(28.0);
            a.stroke.subs.push(poly(&[[-l, w], [0.0, 0.0]], false));
        }
        Fam::Circle(filled) => {
            let r = l * 0.4;
            let c = circle([-r, 0.0], r);
            if filled {
                a.fill.subs.push(c);
            } else {
                a.stroke.subs.push(c);
            }
            a.setback = 2.0 * r;
        }
        Fam::Diamond(filled) => {
            let w = l * 0.35;
            let p = poly(&[[0.0, 0.0], [-l * 0.5, w], [-l, 0.0], [-l * 0.5, -w]], true);
            if filled {
                a.fill.subs.push(p);
            } else {
                a.stroke.subs.push(p);
            }
            a.setback = l;
        }
        Fam::Square(filled) => {
            let s = l * 0.35;
            let p = poly(&[[0.0, s], [-2.0 * s, s], [-2.0 * s, -s], [0.0, -s]], true);
            if filled {
                a.fill.subs.push(p);
            } else {
                a.stroke.subs.push(p);
            }
            a.setback = 2.0 * s;
        }
        Fam::Bar => {
            let w = l * 0.4;
            a.stroke.subs.push(poly(&[[0.0, w], [0.0, -w]], false));
        }
        Fam::Slash => {
            let w = l * 0.4;
            a.stroke.subs.push(poly(&[[w * 0.5, w], [-w * 0.5, -w]], false));
        }
        Fam::DoubleOpen => {
            let w = l * tan_deg(28.0);
            a.stroke.subs.push(poly(&[[-l, w], [0.0, 0.0], [-l, -w]], false));
            a.stroke.subs.push(poly(&[[-1.6 * l, w], [-0.6 * l, 0.0], [-1.6 * l, -w]], false));
        }
        Fam::DoubleTri => {
            a.fill.subs.push(tri(22.0, l, 0.0));
            a.fill.subs.push(tri(22.0, l, -l));
            a.setback = 2.0 * l;
        }
        Fam::CrowOne | Fam::CrowOnlyOne | Fam::CrowZeroOne => {
            let w = l * 0.45;
            a.stroke.subs.push(poly(&[[-0.5 * l, w], [-0.5 * l, -w]], false));
            if fam == Fam::CrowOnlyOne {
                a.stroke.subs.push(poly(&[[-0.8 * l, w], [-0.8 * l, -w]], false));
            }
            if fam == Fam::CrowZeroOne {
                a.stroke.subs.push(circle([-1.1 * l, 0.0], 0.25 * l));
            }
        }
        Fam::CrowMany | Fam::CrowOneMany | Fam::CrowZeroMany => {
            let w = l * 0.45;
            a.stroke.subs.push(poly(&[[0.0, w], [-l, 0.0], [0.0, -w]], false));
            if fam == Fam::CrowOneMany {
                a.stroke.subs.push(poly(&[[-1.15 * l, w], [-1.15 * l, -w]], false));
            }
            if fam == Fam::CrowZeroMany {
                a.stroke.subs.push(circle([-1.4 * l, 0.0], 0.25 * l));
            }
        }
    }
    Some(a)
}

fn tan_deg(d: f64) -> f64 {
    crate::geometry::tan(d * core::f64::consts::PI / 180.0)
}

fn poly(pts: &[[f64; 2]], closed: bool) -> Sub {
    let mut segs: Vec<Seg> = pts[1..].iter().map(|p| Seg::Line(*p)).collect();
    if closed {
        segs.push(Seg::Line(pts[0]));
    }
    Sub { start: pts[0], segs, closed }
}

fn circle(c: [f64; 2], r: f64) -> Sub {
    let mut segs = Vec::new();
    arc_cubics(c, r, 0.0, core::f64::consts::TAU, &mut segs);
    Sub { start: [c[0] + r, c[1]], segs, closed: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_arrow_has_a_shape() {
        for k in 1..=45 {
            let a = arrow(k, 2, 0.01).unwrap();
            assert!(!(a.fill.is_empty() && a.stroke.is_empty()), "{k}");
            assert!(a.setback >= 0.0 && a.setback < 1.0, "{k}");
        }
        assert!(arrow(0, 2, 0.01).is_none());
        assert!(arrow_length(6, 0.01) > arrow_length(0, 0.01) * 5.0);
    }

    #[test]
    fn patterns() {
        assert!(dash(1).is_none() && dash(2).is_some() && dash(24).is_none());
        assert_eq!(fill_pattern(0), FillKind::None);
        assert!(matches!(fill_pattern(29), FillKind::Legacy(Legacy::Linear(_, true))));
        // Visio's bitmaps (JD26 Visio check): 6 horizontal, 7 vertical lines every 8 pixels, 4 the
        // diagonal cross, 10 a quarter of the pixels, 36 dark from the top left corner
        assert_eq!(fill_pattern(6), FillKind::Hatch(&[(0.0, 8.0 / 96.0)]));
        assert_eq!(fill_pattern(7), FillKind::Hatch(&[(90.0, 8.0 / 96.0)]));
        assert!(matches!(fill_pattern(4), FillKind::Hatch(f) if f.len() == 2 && f[0].0 == 45.0 && f[1].0 == 135.0));
        assert_eq!(fill_pattern(10), FillKind::Tint(0.25));
        assert_eq!(fill_pattern(36), FillKind::Legacy(Legacy::Radial(0.0, 1.0)));
        for p in 2..=24 {
            assert!(matches!(fill_pattern(p), FillKind::Hatch(_) | FillKind::Tint(_)), "{p}");
        }
    }
}

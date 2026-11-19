// tests of the design model: the theme parts of the own designs read back through the
// engine's and the viewer's theme readers

use super::catalog;
use crate::formula::Theme;
use crate::render::ThemeInfo;

#[test]
fn parts_read_back() {
    for d in catalog::all() {
        let xml = d.theme_xml();
        let info = ThemeInfo::parse(&xml).expect("a theme");
        assert_eq!(info.name, d.name);
        assert_eq!(info.scheme_ids, [Some(d.id), Some(d.id), Some(d.id), Some(d.id)], "{}", d.name);
        assert_eq!(info.colors[4], Some(d.colors[4]));
        assert_eq!(info.variations.len(), 4);
        for (k, v) in d.variants.iter().enumerate() {
            assert_eq!(info.variations[k][0], Some(v.colors[0]));
        }
        assert_eq!(info.background, Some(d.background));
        assert_eq!(info.minor_font, d.minor_font);
        let t = Theme::parse(&xml);
        assert!(!t.tree.elems.is_empty());
        let text = core::str::from_utf8(&xml).unwrap();
        assert!(text.contains("http://schemas.microsoft.com/office/visio/2012/theme"));
        assert_eq!(text.matches("<vt:lineStyle>").count(), d.shapes.lines.len() + d.connectors.lines.len() + 2);
    }
}

#[test]
fn own_ids_are_distinct_and_high() {
    let all = catalog::all();
    for (i, d) in all.iter().enumerate() {
        assert!(d.id >= super::FIRST_ID && d.id < 65534);
        assert!(all[i + 1..].iter().all(|e| e.id != d.id));
        assert_eq!(catalog::by_id(d.id).as_ref(), Some(d));
        // the conventions of the module comment
        assert_eq!(d.shapes.fonts[0].color, super::Clr::slot(super::Slot::Dk1));
        assert!(d.connectors.lines[0].end != 0);
        assert_eq!(d.shapes.fills.len(), 6);
        assert_eq!(d.shapes.lines.len(), 6);
        assert_eq!(d.shapes.effects.len(), 6);
        assert_eq!(d.shapes.fonts.len(), 6);
        assert_eq!(d.connectors.lines.len(), 3);
        // what Visio reads (the user's Visio, 2026-11-19): variant styles name 1 .. 5 only, fill
        // colours are the placeholder with transforms
        for v in &d.variants {
            for st in &v.styles {
                assert!([st.fill, st.line, st.effect, st.font].iter().all(|k| (1..=5).contains(k)), "{} {st:?}", d.name);
            }
        }
        for f in d.shapes.fills.iter().chain(&d.shapes.backgrounds).chain(&d.connectors.fills) {
            assert!(matches!(f.color, super::Clr::Scheme(super::Slot::Ph, _)), "{} {f:?}", d.name);
        }
    }
    assert_eq!(all[0].name, "NetLab");
}

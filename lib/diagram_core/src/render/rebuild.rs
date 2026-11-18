// rebuild -- the items of some top-level shapes made anew in their place (JD7: a shape that
// moves, grows or changes its look while it is dragged), the rest of the drawing kept
//
//   render::rebuild(&doc, &mut drawing, &mut measure, &opts, &shapes) -> bool
//
// A top-level shape's items lie together in painter's order (build draws a top-level shape
// with everything below it before the next one), so making one anew replaces a range of the
// items: the items after it move when the count changes, the grid follows (Grid::replace),
// the notes and the font families are added up again.  `shapes` may name subshapes: their
// top-level shape is made anew; shapes of pages the drawing does not show are passed over.
// The result is the list build would make of the document now, as long as only those shapes
// changed.  false (nothing changed in the drawing): a shape came or went, the top-level order
// changed, the page's size or scale changed, a named shape of the page is not in the drawing
// -- the caller builds anew.  Changes the shapes do not show (the page's layers, styles,
// masters, themes) are the caller's to know: they need build.

use alloc::vec::Vec;

use super::{chain_of, fonts_of, merge_notes, page_frame, Builder, Drawing, Item, Measure, Note, Options};
use crate::document::{Document, Owner, ShapeRef};
use crate::geometry::Xform;

/// the items of the top-level shapes of `shapes` made anew in `d` -> false: build anew
pub fn rebuild(doc: &Document, d: &mut Drawing, measure: &mut dyn Measure, opts: &Options, shapes: &[ShapeRef]) -> bool {
    let Some(page) = d.page else { return false };
    // the same pages, top-level shapes and order as built
    let chain = chain_of(doc, page, opts);
    let mut k = 0usize;
    for &p in chain.iter().rev() {
        for r in doc.top_shapes(Owner::Page(p)) {
            if d.tops.get(k).map(|t| t.shape) != Some(r) {
                return false;
            }
            k += 1;
        }
    }
    if k != d.tops.len() {
        return false;
    }
    let (w, h, scale) = page_frame(doc, page);
    if w != d.width || h != d.height || scale != d.scale {
        return false;
    }
    // the top-level shapes to make anew (indices of d.tops, ascending)
    let mut want: Vec<usize> = Vec::new();
    for &s in shapes {
        let Owner::Page(p) = s.owner else { continue };
        if !chain.contains(&p) {
            continue;
        }
        let Some(top) = top_of(doc, s) else { return false };
        let Some(i) = d.tops.iter().position(|t| t.shape == top) else { return false };
        want.push(i);
    }
    want.sort_unstable();
    want.dedup();
    if want.is_empty() {
        return true;
    }
    // build them, each in its page's context
    let themes = core::mem::take(&mut d.themes);
    let mut b = Builder::with_themes(doc, measure, *opts, themes);
    let mut made: Vec<(usize, Vec<Item>, Vec<(Note, u32)>)> = Vec::with_capacity(want.len());
    let mut cur = None;
    for &i in &want {
        let r = d.tops[i].shape;
        let Owner::Page(p) = r.owner else { continue };
        if cur != Some(p) {
            b.context(p);
            cur = Some(p);
        }
        let m = Xform::scale(b.scale, b.scale);
        b.top_shape(r, &m);
        let items = core::mem::take(&mut b.items);
        let notes = b.tops.pop().map(|t| t.notes).unwrap_or_default();
        made.push((i, items, notes));
    }
    d.themes = core::mem::take(&mut b.themes);
    drop(b);
    // into the list
    let mut changes: Vec<(usize, usize, usize)> = Vec::with_capacity(made.len());
    if made.iter().all(|(i, items, _)| d.top_range(*i).len() == items.len()) {
        for (i, items, notes) in made {
            let r = d.top_range(i);
            changes.push((r.start, r.len(), items.len()));
            for (slot, it) in d.items[r].iter_mut().zip(items) {
                *slot = it;
            }
            d.tops[i].notes = notes;
        }
    } else {
        let old = core::mem::take(&mut d.items);
        let old_len = old.len();
        let grown: usize = made.iter().map(|m| m.1.len()).sum();
        let mut out: Vec<Item> = Vec::with_capacity(old_len + grown);
        let mut olds = old.into_iter();
        let mut made = made.into_iter().peekable();
        let n = d.tops.len();
        for k in 0..n {
            // (the next top's start is still the old one here)
            let s = d.tops[k].start as usize;
            let e = if k + 1 < n { d.tops[k + 1].start as usize } else { old_len };
            let len = e.saturating_sub(s);
            let start = out.len() as u32;
            match made.next_if(|m| m.0 == k) {
                Some((_, items, notes)) => {
                    changes.push((s, len, items.len()));
                    olds.by_ref().take(len).for_each(drop);
                    out.extend(items);
                    d.tops[k].notes = notes;
                }
                None => out.extend(olds.by_ref().take(len)),
            }
            d.tops[k].start = start;
        }
        d.items = out;
    }
    d.notes = merge_notes(&d.tops);
    d.fonts = fonts_of(&d.items);
    d.grid.replace(&changes, &d.items);
    true
}

/// the top-level shape a shape lies in (itself at the top); None: no such shape
pub(crate) fn top_of(doc: &Document, s: ShapeRef) -> Option<ShapeRef> {
    let mut cur = s;
    for _ in 0..1024 {
        match doc.shape(cur)?.parent() {
            Some(p) => cur = p,
            None => return Some(cur),
        }
    }
    None
}

#[cfg(test)]
#[path = "rebuild_tests.rs"]
mod tests;

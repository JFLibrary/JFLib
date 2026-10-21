use super::*;
use doc_core::{ParagraphFormat, TabAlignment, TabLeader};

/// Measure the following tab field across formatting runs. Generated fields and note
/// references participate in alignment without changing their original source ranges.
pub(super) fn advance(current: &Pending, following: &[Atom], pf: ParagraphFormat, measure: &mut impl Measure) -> Result<(i32, TabLeader), Error> {
    let relative = current.left + current.indent + current.width;
    let position = i32::from(pf.left) + relative;
    let stop = pf.tabs.as_slice().iter().find(|s| s.position > position && !matches!(s.alignment, TabAlignment::Bar | TabAlignment::Clear));
    // Hanging indents always supply an implicit stop at the regular left edge.
    if relative < 0 && stop.is_none_or(|s| s.position >= i32::from(pf.left)) { return Ok((-relative, TabLeader::None)); }
    let Some(stop) = stop else {
        // Keep the established paragraph-relative raster for documents without custom
        // stops; an explicit stop list uses positions relative to the text margin.
        let grid = if pf.tabs.is_empty() { relative } else { position };
        return Ok((720 - grid.rem_euclid(720), TabLeader::None));
    };
    let mut width = 0i64;
    let mut decimal = None;
    if matches!(stop.alignment, TabAlignment::Center | TabAlignment::Right | TabAlignment::Decimal) {
        for a in following.iter().take_while(|a| !a.tab && !a.newline) {
            if decimal.is_none() {
                if let Some(i) = a.text.find(['.', ',']) {
                    let prefix = measure.width(&a.text[..i], a.format);
                    if !(0..=100_000_000).contains(&prefix) { return Err(Error::Metrics); }
                    decimal = Some(width + i64::from(prefix));
                }
            }
            width += i64::from(a.width);
        }
    }
    let offset = match stop.alignment { TabAlignment::Center => width / 2, TabAlignment::Right => width, TabAlignment::Decimal => decimal.unwrap_or(width), _ => 0 };
    // Oversized fields start at the cursor and wrap normally, never overlap prior text.
    Ok(((i64::from(stop.position - position) - offset).max(0) as i32, stop.leader))
}
pub(super) fn bars(pf: ParagraphFormat, page: PageSettings) -> Vec<i32> {
    pf.tabs.as_slice().iter().filter(|s| s.alignment == TabAlignment::Bar)
        .map(|s| page.left as i32 + s.position).filter(|x| *x >= 0 && *x <= page.width as i32).collect()
}

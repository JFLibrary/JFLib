// fonts -- which font of the repository stands in for a text style's font (decision 31: SHX fonts belong to Autodesk,
// and there are no Microsoft fonts in the repository).  A substitute names a family (the caller has the bytes) and how
// wide the original runs: the advance of the sample "a..z A..Z 0..9" over its cap height, from the original's metrics
// (Helvetica / Arial, Times, Courier: their published advance widths; SHX fonts: estimates from their glyph grids,
// docs/RENDER2D.md "DXF").  render2d::dxf turns that into a width factor against the substitute's own advance.

/// a family of the fonts in the repository
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Family {
    /// Inter (styles/netlab/assets): sans-serif, for Arial, Helvetica, Calibri, Verdana, Tahoma and every SHX font
    Sans,
    /// Inter SemiBold: bold sans-serif (Arial Black, "... Bold")
    SansBold,
    /// DejaVu Serif (lib/doc_view/assets): Times, Georgia, Cambria, Garamond, Book Antiqua
    Serif,
    SerifBold,
    /// DejaVu Sans Mono: Courier, Consolas, Lucida Console, Source Code Pro, monotxt.shx
    Mono,
    MonoBold,
    /// Selawik (styles/winui/assets): Segoe UI
    Ui,
}

/// a font's stand-in
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Substitute {
    pub family: Family,
    /// the original's advance of the sample "a..z A..Z 0..9" (62 characters) in cap heights; 0: keep the substitute's width
    pub sample_advance: f64,
    /// an SHX font (single strokes) or a shape file
    pub shx: bool,
}

/// the sample whose advance sets the width factor of a substitute
pub const SAMPLE: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

// the originals' advances of SAMPLE in cap heights, from the advance widths of the Adobe font metrics (Arial and Times
// New Roman have Helvetica's and Times' widths, Courier New Courier's):
//   Helvetica: lower case 12.726 em, upper case 17.613, digits 5.560 -> 35.899 em / cap height 0.718 = 50.00
//   Times:     lower case 11.943 em, upper case 17.332, digits 5.000 -> 34.275 em / cap height 0.662 = 51.78
//   Courier:   62 x 0.600 = 37.2 em / cap height 0.571 (Courier New) = 65.15
// SHX fonts (estimates, the fonts are not here): txt.shx draws on a grid of 6 units of cap height with an advance of 6
// for most characters (4 for the narrow ones) -> about 0.95 cap heights each, 58.9; simplex / romans (Hershey's simplex
// Roman: cap height 21, upper case about 20, lower case 18, digits 20) -> 0.92 each, 57.0; isocp / iso* (ISO 3098
// type B lettering) -> 0.8 each, 49.6; monotxt -> 1.0 each, 62.0
const HELVETICA: f64 = 50.0;
const TIMES: f64 = 51.78;
const COURIER: f64 = 65.15;
const SHX_TXT: f64 = 58.9;
const SHX_SIMPLEX: f64 = 57.0;
const SHX_ISO: f64 = 49.6;
const SHX_MONO: f64 = 62.0;

/// the stand-in of a style's font file (3) or TrueType family (its XDATA), compared without case and path: TrueType
/// names by their family, everything else (".shx", no extension, unknown) as an SHX font
pub fn font_substitute(file: &str, family: Option<&str>) -> Substitute {
    // the file's name without folders and without its extension
    let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let lower: alloc::string::String = base.chars().map(|c| c.to_ascii_lowercase()).collect();
    let (stem, ext) = match lower.rfind('.') {
        Some(i) => (&lower[..i], &lower[i + 1..]),
        None => (lower.as_str(), ""),
    };
    let fam: alloc::string::String = family.unwrap_or("").chars().map(|c| c.to_ascii_lowercase()).collect();
    let ttf = matches!(ext, "ttf" | "otf" | "ttc") || (!fam.is_empty() && ext != "shx");
    if !ttf {
        let (w, family) = match stem {
            "txt" | "txt_" => (SHX_TXT, Family::Sans),
            "monotxt" => (SHX_MONO, Family::Mono),
            s if s.starts_with("iso") => (SHX_ISO, Family::Sans),
            _ => (SHX_SIMPLEX, Family::Sans),
        };
        return Substitute { family, sample_advance: w, shx: true };
    }
    let name = if fam.is_empty() { stem } else { fam.as_str() };
    let has = |k: &str| name.contains(k);
    // (Windows' file names of bold cuts: arialbd, timesbd, courbd, verdanab, calibrib, consolab, ariblk)
    let bold = has("bold") || has("black") || has("heavy") || ["bd", "bi", "ab", "ib", "blk"].iter().any(|e| stem.ends_with(e));
    let mono = ["cour", "consol", "lucon", "lucida console", "mono", "code", "menlo", "inconsolata"].iter().any(|k| has(k));
    let serif = ["times", "georgia", "cambria", "garamond", "book", "roman", "palatino", "century", "bodoni", "bell"].iter().any(|k| has(k)) || (has("serif") && !has("sans"));
    let (family, w) = if mono {
        (if bold { Family::MonoBold } else { Family::Mono }, COURIER)
    } else if serif {
        (if bold { Family::SerifBold } else { Family::Serif }, TIMES)
    } else if has("segoe") || stem.starts_with("seg") {
        (Family::Ui, 0.0)
    } else {
        (if bold { Family::SansBold } else { Family::Sans }, HELVETICA)
    };
    // narrow and wide cuts of the sans fonts
    let w = if has("narrow") || stem == "arialn" || stem.starts_with("arialn") { w * 0.82 } else { w };
    Substitute { family, sample_advance: w, shx: false }
}

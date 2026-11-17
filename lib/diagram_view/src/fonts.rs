// fonts -- the fonts a diagram's texts are drawn with, and the substitution of missing ones
//
//   FontSet::new()                          empty: the app adds what it has
//   set.add(Class::Sans, bold, italic, bytes)   a font of the substitution set (TrueType or
//                                            OpenType bytes); the app's own (its style's) fonts
//   set.add_family("Calibri", bold, italic, bytes, face)   a font under its own name (an
//                                            installed font the app found)
//   set.set_source(Box<dyn FontSource>)     asked once per family the set does not know (the
//                                            app looks at the installed fonts there)
//   class_of(family) -> Class               serif, monospaced or sans by the family's name
//   SAMPLE_WIDTHS                            how wide common fonts set a sample line (em)
//
// TX-09: a family the set does not have is drawn with the set's font of its class (sans when
// the class is missing too), stretched by the width factor that makes the sample line as wide
// as in the missing font (Calibri set in Inter: about 0.85), so lines wrap about where Visio
// wraps them.  An italic without its own face leans 0.2 radians (synthetic), a bold without its
// own face stays regular.  Only fonts the app hands over are used; the crate embeds none.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

/// what kind of face a family is (the substitution goes by it)
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Sans,
    Serif,
    Mono,
}

/// installed fonts the app knows how to find
pub trait FontSource {
    /// the bytes and face index of `family` in this style, if installed
    fn find(&mut self, family: &str, bold: bool, italic: bool) -> Option<(Vec<u8>, u32)>;
}

#[derive(Clone)]
pub(crate) struct Face {
    pub family: String,
    pub class: Option<Class>,
    pub bold: bool,
    pub italic: bool,
    pub data: Rc<[u8]>,
    pub index: u32,
}

/// the fonts a page view may use
#[derive(Default)]
pub struct FontSet {
    pub(crate) faces: Vec<Face>,
    pub(crate) source: Option<Box<dyn FontSource>>,
    /// families the source was asked for (found or not)
    pub(crate) asked: Vec<String>,
}

impl FontSet {
    pub fn new() -> FontSet {
        FontSet::default()
    }

    /// a face of the substitution set
    pub fn add(&mut self, class: Class, bold: bool, italic: bool, data: &[u8]) {
        self.faces.push(Face { family: String::new(), class: Some(class), bold, italic, data: Rc::from(data), index: 0 });
    }

    /// a face under its family name
    pub fn add_family(&mut self, family: &str, bold: bool, italic: bool, data: &[u8], index: u32) {
        self.faces.push(Face { family: String::from(family), class: None, bold, italic, data: Rc::from(data), index });
    }

    /// where faces of unknown families are looked for
    pub fn set_source(&mut self, source: Box<dyn FontSource>) {
        self.source = Some(source);
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// the face for a family and style: (face, synthetic italic, substituted); asks the source
    /// once per family and style the set lacks
    pub(crate) fn pick(&mut self, family: &str, bold: bool, italic: bool) -> Option<(Face, bool, bool)> {
        let same = |f: &Face| f.class.is_none() && f.family.eq_ignore_ascii_case(family);
        if !self.faces.iter().any(|f| same(f) && f.bold == bold && f.italic == italic) {
            let key = alloc::format!("{family}\u{1}{bold}\u{1}{italic}");
            if !self.asked.contains(&key) && self.asked.len() < 4096 {
                self.asked.push(key);
                if let Some(src) = self.source.as_mut() {
                    if let Some((data, index)) = src.find(family, bold, italic) {
                        self.add_family(family, bold, italic, &data, index);
                    }
                }
            }
        }
        if let Some(f) = best(self.faces.iter().filter(|f| same(f)), bold, italic) {
            return Some((f.clone(), italic && !f.italic, false));
        }
        let class = class_of(family);
        let in_class = best(self.faces.iter().filter(|f| f.class == Some(class)), bold, italic);
        let f = in_class.or_else(|| best(self.faces.iter().filter(|f| f.class == Some(Class::Sans)), bold, italic)).or_else(|| best(self.faces.iter(), bold, italic))?;
        Some((f.clone(), italic && !f.italic, true))
    }
}

// the face nearest to the style: same bold and italic, else same bold, else any
fn best<'a>(it: impl Iterator<Item = &'a Face> + Clone, bold: bool, italic: bool) -> Option<&'a Face> {
    it.clone().find(|f| f.bold == bold && f.italic == italic).or_else(|| it.clone().find(|f| f.bold == bold && !f.italic)).or_else(|| it.clone().find(|f| !f.bold && !f.italic)).or_else(|| it.clone().next())
}

/// serif, monospaced or sans by the family's name
pub fn class_of(family: &str) -> Class {
    let f = family.to_ascii_lowercase();
    const MONO: &[&str] = &["consolas", "courier", "lucida console", "lucida sans typewriter", "menlo", "monaco", "mono", "fixedsys", "terminal", "ocr"];
    const SERIF: &[&str] = &["times", "cambria", "georgia", "garamond", "book antiqua", "palatino", "constantia", "baskerville", "bodoni", "century", "serif", "minion", "bookman", "rockwell", "perpetua", "goudy", "sylfaen"];
    if MONO.iter().any(|m| f.contains(m)) {
        Class::Mono
    } else if SERIF.iter().any(|s| f.contains(s)) && !f.contains("sans") {
        Class::Serif
    } else {
        Class::Sans
    }
}

/// the sample line the widths are measured on
pub const SAMPLE: &str = "The quick brown fox jumps over the lazy dog 0123456789";

/// how wide common fonts set SAMPLE (em, regular and bold): advance widths from the fonts'
/// metrics, the numbers only
pub const SAMPLE_WIDTHS: &[(&str, f64, f64)] = &[
    ("Calibri", 23.4209, 23.8364),
    ("Calibri Light", 23.0, 23.0),
    ("Segoe UI", 25.4883, 27.2686),
    ("Arial", 25.6270, 27.2310),
    ("Helvetica", 25.6270, 27.2310),
    ("Verdana", 29.4502, 31.5),
    ("Tahoma", 25.7188, 27.0),
    ("Trebuchet MS", 25.7339, 27.0),
    ("Franklin Gothic Medium", 25.0068, 25.0068),
    ("Franklin Gothic Book", 24.5, 25.0),
    ("Candara", 24.0923, 24.6),
    ("Corbel", 23.4199, 24.0),
    ("Times New Roman", 23.5234, 24.6),
    ("Georgia", 25.3955, 27.5),
    ("Cambria", 24.8828, 25.8),
    ("Consolas", 29.6895, 29.6895),
    ("Courier New", 32.4053, 32.4053),
    ("Lucida Console", 32.5371, 32.5371),
];

/// the sample width of a family (regular or bold), if the table knows it
pub fn sample_width(family: &str, bold: bool) -> Option<f64> {
    SAMPLE_WIDTHS.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(family)).map(|&(_, r, b)| if bold { b } else { r })
}

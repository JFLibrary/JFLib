// template_src -- the text sources of a template (template.txt, *.starter, rules.txt) as
// entries (JD13)
//
//   parse(file, text, sections) -> TSource { header, sections }   header lines, sections
//                                       ([Name] or [Name argument]), rows with continuation
//                                       lines, ';'-separated cells
//
// The common rules are the stencils' (stencil_src.rs, apps/JFDiagrammer/stencils/README.md):
// comments, `key: value` header lines, the suffixes .de .en .es .it .fr and .metric / .us,
// errors with file and line; resolving a variant, classifying a constant and unquoting a text
// are stencil_src's.  What differs (apps/JFDiagrammer/templates/README.md):
// * the sections are the ones the file kind names; a section may take an argument
//   ([Style Normal], [Style Text Only]);
// * a row of a named section is headed by a name up to its colon that may hold '-', spaces and
//   brackets (`Page-2:`, `Background-1:`, `start:`), a row of an indexed section by `Row`;
// * a cell name may start with Char. or Para. (row 0 of Character and Paragraph in a style):
//   `Char.Size = 8 pt`, `Para.HorzAlign.us = 0`.
// Nothing here panics, nothing recurses.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::stencil::SourceError;
use crate::stencil_src::{self, err, Entry};

/// the most lines of one file, items of one list, sections of one file (trusted repo files,
/// bounded all the same)
const MAX_LINES: usize = 200_000;
const MAX_ITEMS: usize = 100_000;
const MAX_SECTIONS: usize = 1_000;

/// how a section's lines are read
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum TKind {
    /// single cells only ([PageSheet], [Style ...])
    Cells,
    /// rows headed by the word Row ([Layer])
    Indexed,
    /// rows headed by their name and a colon ([Pages], [Shapes], [Rules])
    Named,
}

/// a section a file kind may have: its name, how it is read, whether it takes an argument
pub(crate) type SecSpec = (&'static str, TKind, bool);

/// a row: its head and its cells
#[derive(Clone, Debug)]
pub(crate) struct TRow {
    pub head: String,
    pub line: u32,
    pub cells: Vec<Entry>,
}

/// a section as written
#[derive(Clone, Debug)]
pub(crate) struct TSection {
    /// the name ("PageSheet", "Style" ...)
    pub name: &'static str,
    /// the argument after the name ("Normal" in [Style Normal]; "" without)
    pub arg: String,
    pub kind: TKind,
    pub line: u32,
    /// the single cells ([PageSheet], [Style ...])
    pub cells: Vec<Entry>,
    pub rows: Vec<TRow>,
}

/// a source file: the header lines and the sections
#[derive(Clone, Debug, Default)]
pub(crate) struct TSource {
    pub header: Vec<Entry>,
    pub sections: Vec<TSection>,
}

impl TSource {
    /// the sections named `name`
    pub fn sections<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s TSection> + 's {
        self.sections.iter().filter(move |s| s.name == name)
    }
}

// a row head: a name without the characters the format uses
fn head_ok(h: &str) -> bool {
    !h.is_empty() && !h.chars().any(|c| c.is_control() || matches!(c, ':' | ';' | '=' | '"' | '#' | '[' | ']' | '|'))
}

// the prefixes of a style's row 0 of Character and Paragraph
const ROW_PREFIXES: [&str; 2] = ["Char.", "Para."];

/// `a = 1; Char.Size.us = 8 pt` -> entries
fn parse_cells(file: &str, line: u32, s: &str, out: &mut Vec<Entry>) -> Result<(), SourceError> {
    let parts = stencil_src::split_cells(s).ok_or_else(|| err(file, line, "Anführungszeichen nicht geschlossen"))?;
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        let Some((name, value)) = p.split_once('=') else {
            return Err(err(file, line, format!("„{p}“: erwartet Zelle = Wert")));
        };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            return Err(err(file, line, format!("„{name}“ ist kein Zellname")));
        }
        if value.is_empty() {
            return Err(err(file, line, format!("{name}: Wert fehlt")));
        }
        let (prefix, rest) = match ROW_PREFIXES.iter().find(|p| name.starts_with(**p)) {
            Some(p) => (*p, &name[p.len()..]),
            None => ("", name),
        };
        let (n, lang, variant) = stencil_src::suffixes(file, line, rest)?;
        if out.len() >= MAX_ITEMS {
            return Err(err(file, line, "zu viele Zellen"));
        }
        out.push(Entry { name: format!("{prefix}{n}"), lang, variant, value: String::from(value), line });
    }
    Ok(())
}

/// parse a source file; `sections`: the sections its kind may have (none: header lines only)
pub(crate) fn parse(file: &str, text: &str, sections: &[SecSpec]) -> Result<TSource, SourceError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut src = TSource::default();
    // the row names of the current section
    let mut names: Vec<String> = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        if i >= MAX_LINES {
            return Err(err(file, i as u32 + 1, "zu viele Zeilen"));
        }
        let line_no = i as u32 + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        if t.starts_with('[') && !indented {
            if sections.is_empty() {
                return Err(err(file, line_no, format!("{file} hat keine Abschnitte ({t})")));
            }
            let Some(inner) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) else {
                return Err(err(file, line_no, format!("Abschnittskopf {t}: erwartet [Name]")));
            };
            let inner = inner.trim();
            let (name, arg) = match inner.split_once(char::is_whitespace) {
                Some((n, a)) => (n, a.trim()),
                None => (inner, ""),
            };
            let Some(&(spec, kind, takes)) = sections.iter().find(|s| s.0 == name) else {
                let known: Vec<String> = sections.iter().map(|s| format!("[{}{}]", s.0, if s.2 { " …" } else { "" })).collect();
                return Err(err(file, line_no, format!("unbekannter Abschnitt [{inner}] (bekannt: {})", known.join(", "))));
            };
            if takes && arg.is_empty() {
                return Err(err(file, line_no, format!("[{name}] braucht einen Namen ([{name} Normal])")));
            }
            if !takes && !arg.is_empty() {
                return Err(err(file, line_no, format!("[{name}] hat keinen Zusatz ({arg})")));
            }
            if src.sections.iter().any(|s| s.name == spec && s.arg == arg) {
                return Err(err(file, line_no, format!("Abschnitt [{inner}] doppelt")));
            }
            if src.sections.len() >= MAX_SECTIONS {
                return Err(err(file, line_no, "zu viele Abschnitte"));
            }
            names.clear();
            src.sections.push(TSection { name: spec, arg: String::from(arg), kind, line: line_no, cells: Vec::new(), rows: Vec::new() });
            continue;
        }
        let Some(sec) = src.sections.last_mut() else {
            // a header line: key: value
            let Some((key, value)) = t.split_once(':') else {
                return Err(err(file, line_no, format!("Kopfzeile ohne „:“ ({t})")));
            };
            let key = key.trim();
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
                return Err(err(file, line_no, format!("„{key}“ ist kein Schlüssel")));
            }
            let (name, lang, variant) = stencil_src::suffixes(file, line_no, key)?;
            if src.header.len() >= MAX_ITEMS {
                return Err(err(file, line_no, "zu viele Kopfzeilen"));
            }
            src.header.push(Entry { name, lang, variant, value: String::from(value.trim()), line: line_no });
            continue;
        };
        if sec.kind == TKind::Cells {
            parse_cells(file, line_no, t, &mut sec.cells)?;
            continue;
        }
        if indented {
            let Some(row) = sec.rows.last_mut() else {
                return Err(err(file, line_no, "eingerückte Folgezeile ohne Zeile davor"));
            };
            parse_cells(file, line_no, t, &mut row.cells)?;
            continue;
        }
        let (head, cells) = match sec.kind {
            TKind::Indexed => {
                let rest = t.strip_prefix("Row").filter(|r| r.is_empty() || r.starts_with(char::is_whitespace));
                let Some(rest) = rest else {
                    return Err(err(file, line_no, format!("Zeilen in [{}] beginnen mit Row ({t})", sec.name)));
                };
                (String::from("Row"), rest)
            }
            _ => {
                // the head up to the first colon outside a string, before any '='
                let colon = t.find(':');
                let eq = t.find(['=', '"']);
                let Some(c) = colon.filter(|&c| eq.is_none_or(|e| c < e)) else {
                    return Err(err(file, line_no, format!("Zeile „{t}“: erwartet Name: Zellen")));
                };
                let head = t[..c].trim();
                if !head_ok(head) {
                    return Err(err(file, line_no, format!("„{head}“ ist kein Zeilenname")));
                }
                if names.iter().any(|n| n == head) {
                    return Err(err(file, line_no, format!("Zeile {head} doppelt in [{}]", sec.name)));
                }
                if names.len() >= MAX_ITEMS {
                    return Err(err(file, line_no, "zu viele Zeilen im Abschnitt"));
                }
                names.push(String::from(head));
                (String::from(head), &t[c + 1..])
            }
        };
        if sec.rows.len() >= MAX_ITEMS {
            return Err(err(file, line_no, "zu viele Zeilen im Abschnitt"));
        }
        let mut row = TRow { head, line: line_no, cells: Vec::new() };
        parse_cells(file, line_no, cells, &mut row.cells)?;
        sec.rows.push(row);
    }
    Ok(src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vsdx::Measure;

    const SECS: &[SecSpec] = &[("PageSheet", TKind::Cells, false), ("Style", TKind::Cells, true), ("Layer", TKind::Indexed, false), ("Pages", TKind::Named, false)];

    #[test]
    fn sections_rows_and_prefixes() {
        let text = "template: T\nname.de: Ä\n# comment\n[PageSheet]\nPageWidth = 297 mm; PageWidth.us = 11 in\n[Style Text Only]\nChar.Size = 8 pt\nPara.HorzAlign.us = 0\n[Layer]\nRow Name.de = \"Ebene\"; Visible = TRUE\n    Print = FALSE\n[Pages]\nPage-2: Name.de = \"Seite-2\"\nBackground 1:  Background = TRUE\n";
        let src = parse("template.txt", text, SECS).unwrap();
        assert_eq!(src.header.len(), 2);
        assert_eq!(src.header[1].lang, Some(0));
        assert_eq!(src.sections.len(), 4);
        let st = &src.sections[1];
        assert_eq!((st.name, st.arg.as_str()), ("Style", "Text Only"));
        assert_eq!(st.cells[0].name, "Char.Size");
        assert_eq!((st.cells[1].name.as_str(), st.cells[1].variant), ("Para.HorzAlign", Some(Measure::Us)));
        let layer = &src.sections[2];
        assert_eq!(layer.rows.len(), 1);
        assert_eq!(layer.rows[0].cells.len(), 3, "the continuation line belongs to the row");
        let pages = &src.sections[3];
        assert_eq!(pages.rows[0].head, "Page-2");
        assert_eq!(pages.rows[1].head, "Background 1");
        let r = stencil_src::resolve("template.txt", &src.sections[0].cells, Measure::Us).unwrap();
        assert_eq!(r[0].value, "11 in");
    }

    #[test]
    fn faults_name_file_and_line() {
        let e = parse("x.txt", "a: 1\n[Nope]\n", SECS).unwrap_err();
        assert_eq!((e.file.as_str(), e.line), ("x.txt", 2));
        let e = parse("x.txt", "a: 1\n[Style]\n", SECS).unwrap_err();
        assert_eq!(e.line, 2);
        let e = parse("x.txt", "a: 1\n[Pages]\nP: A = 1\nP: B = 2\n", SECS).unwrap_err();
        assert_eq!(e.line, 4, "{e}");
        let e = parse("x.txt", "a: 1\n[Layer]\nZeile A = 1\n", SECS).unwrap_err();
        assert_eq!(e.line, 3);
        let e = parse("x.txt", "a: 1\n[PageSheet]\nPageWidth.xx = 1 mm\n", SECS).unwrap_err();
        assert_eq!(e.line, 3);
        let e = parse("x.txt", "a: 1\n[Pages]\n    A = 1\n", SECS).unwrap_err();
        assert_eq!(e.line, 3);
        let e = parse("x.txt", "a: 1\n[PageSheet]\n", &[]).unwrap_err();
        assert_eq!(e.line, 2);
    }
}

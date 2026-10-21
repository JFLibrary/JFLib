// Bounded RIS bibliography interchange. New native codec, no legacy counterpart.
// Tag semantics: Clarivate RIS / primary Zotero RIS translator mapping; see public API scope.
use crate::Error;
use alloc::{collections::BTreeSet, format, string::String, vec::Vec};
use base::encoding;
use doc_core::{Person, Source, SourceKind};

const INPUT_LIMIT: usize = 4 << 20;
const MODEL_LIMIT: usize = 2 << 20;
const FIELD_LIMIT: usize = 4096;

struct Field {
    tag: String,
    value: String,
}
fn fail(record: usize, tag: &str, reason: &str) -> Error {
    Error::Unsupported(format!("RIS Datensatz {record}, {tag}: {reason}"))
}
fn tagged(line: &str) -> Option<(&str, &str)> {
    let b = line.as_bytes();
    if b.len() >= 5
        && b[0].is_ascii_uppercase()
        && (b[1].is_ascii_uppercase() || b[1].is_ascii_digit())
        && &b[2..5] == b"  -"
    {
        if b.len() == 5 {
            return Some((&line[..2], ""));
        }
        if b[5] == b' ' {
            return Some((&line[..2], &line[6..]));
        }
    }
    None
}
fn scalar<'a>(fields: &'a [Field], tags: &[&str], record: usize) -> Result<&'a str, Error> {
    let mut value = None;
    for f in fields.iter().filter(|f| tags.contains(&f.tag.as_str())) {
        if value.is_some() {
            return Err(fail(record, &f.tag, "mehrfaches oder mehrdeutiges Feld"));
        }
        value = Some(f.value.as_str());
    }
    Ok(value.unwrap_or(""))
}
fn single_line(value: &str, record: usize, tag: &str) -> Result<(), Error> {
    if value.contains(['\r', '\n']) || value.trim() != value {
        return Err(fail(
            record,
            tag,
            "einzeiliger Wert ohne äußere Leerzeichen erforderlich",
        ));
    }
    Ok(())
}
fn year(value: &str, record: usize, tag: &str) -> Result<String, Error> {
    let (y, tail) = value.split_once('/').unwrap_or((value, ""));
    if y.len() != 4 || !y.bytes().all(|c| c.is_ascii_digit()) || tail.chars().any(|c| c != '/') {
        return Err(fail(record, tag, "nur vierstellige Jahreszahl darstellbar"));
    }
    Ok(String::from(y))
}
fn access(value: &str, record: usize) -> Result<[String; 3], Error> {
    let value = value.trim_end_matches('/');
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(fail(record, "Y2", "Zugriffsdatum muss JJJJ[/MM[/TT]] sein"));
    }
    let y = year(parts[0], record, "Y2")?;
    let mut out = [y, String::new(), String::new()];
    for i in 1..parts.len() {
        if parts[i].len() > 2 || !parts[i].bytes().all(|c| c.is_ascii_digit()) {
            return Err(fail(record, "Y2", "ungültiges Zugriffsdatum"));
        }
        let n = parts[i].parse::<u16>().unwrap_or(0);
        if n == 0 || n > if i == 1 { 12 } else { 31 } {
            return Err(fail(record, "Y2", "ungültiges Zugriffsdatum"));
        }
        out[i] = String::from(parts[i]);
    }
    if parts.len() == 3 {
        let y = out[0].parse::<u32>().unwrap_or(0);
        let month = out[1].parse::<u32>().unwrap_or(0);
        let days = match month {
            2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        if out[2].parse::<u32>().unwrap_or(0) > days {
            return Err(fail(record, "Y2", "ungültiger Kalendertag"));
        }
    }
    Ok(out)
}
fn doi(value: &str, record: usize) -> Result<String, Error> {
    let id = value.strip_prefix("https://doi.org/").unwrap_or(value);
    let Some((prefix, suffix)) = id.split_once('/') else {
        return Err(fail(
            record,
            "DO",
            "DOI ist nicht als eindeutige URL darstellbar",
        ));
    };
    let digits = prefix.strip_prefix("10.").unwrap_or("");
    if !(4..=9).contains(&digits.len())
        || !digits.bytes().all(|c| c.is_ascii_digit())
        || suffix.is_empty()
        || !id.is_ascii()
        || id
            .bytes()
            .any(|c| !c.is_ascii_alphanumeric() && !b"-._~:/!$&'()*+,;=@".contains(&c))
    {
        return Err(fail(
            record,
            "DO",
            "DOI ist nicht als eindeutige URL darstellbar",
        ));
    }
    Ok(format!("https://doi.org/{id}"))
}
fn source(fields: &[Field], record: usize) -> Result<Source, Error> {
    let kind = match scalar(fields, &["TY"], record)? {
        "BOOK" => SourceKind::Book,
        "JOUR" => SourceKind::JournalArticle,
        "ELEC" => SourceKind::Website,
        _ => return Err(fail(record, "TY", "nur BOOK, JOUR und ELEC unterstützt")),
    };
    let mut s = Source {
        id: record as u64,
        kind,
        ..Default::default()
    };
    let allowed = [
        "TY", "ID", "AU", "A1", "TI", "T1", "BT", "PY", "Y1", "PB", "CY", "ET", "JF", "JO", "T2",
        "VL", "IS", "SP", "EP", "UR", "DO", "SN", "ST", "N1", "Y2", "LA",
    ];
    for f in fields {
        if !allowed.contains(&f.tag.as_str()) {
            return Err(fail(
                record,
                &f.tag,
                "Feld kann nicht verlustfrei dargestellt werden",
            ));
        }
        if f.value.is_empty() {
            return Err(fail(record, &f.tag, "leerer Feldwert"));
        }
        if f.value.len() > FIELD_LIMIT {
            return Err(Error::Limit);
        }
        if ["AU", "A1"].contains(&f.tag.as_str()) {
            single_line(&f.value, record, &f.tag)?;
            let parts: Vec<_> = f.value.split(',').collect();
            // No suffix storage, no inference of corporations or given/family name order.
            if parts.len() > 2 || parts.iter().any(|p| p.trim().is_empty()) {
                return Err(fail(
                    record,
                    &f.tag,
                    "Person als Nachname[, Vorname]; Suffix/Körperschaft nicht unterstützt",
                ));
            }
            if s.authors.len() == 64 {
                return Err(Error::Limit);
            }
            s.authors.push(Person {
                family: String::from(parts[0].trim()),
                given: String::from(parts.get(1).copied().unwrap_or("").trim()),
                middle: String::new(),
            });
        }
    }
    // The external RIS reference number is deliberately not a persistent source GUID.
    let external = scalar(fields, &["ID"], record)?;
    if !external.is_empty() {
        single_line(external, record, "ID")?;
        if external
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(fail(record, "ID", "ungültige Referenzkennung"));
        }
    }
    s.title = String::from(scalar(
        fields,
        if kind == SourceKind::Book {
            &["TI", "T1", "BT"]
        } else {
            &["TI", "T1"]
        },
        record,
    )?);
    if kind != SourceKind::Book && fields.iter().any(|f| f.tag == "BT") {
        return Err(fail(
            record,
            "BT",
            "Buchtitel für diesen Quellentyp nicht darstellbar",
        ));
    }
    let value = scalar(fields, &["PY", "Y1"], record)?;
    if !value.is_empty() {
        s.year = year(value, record, "PY/Y1")?;
    }
    macro_rules! read {
        ($field:ident, $tag:literal) => {
            s.$field = String::from(scalar(fields, &[$tag], record)?);
        };
    }
    read!(publisher, "PB");
    read!(city, "CY");
    read!(edition, "ET");
    read!(volume, "VL");
    read!(issue, "IS");
    read!(pages, "SP");
    read!(url, "UR");
    read!(standard_number, "SN");
    read!(short_title, "ST");
    read!(comments, "N1");
    read!(language, "LA");
    match kind {
        SourceKind::JournalArticle => {
            s.journal = String::from(scalar(fields, &["JF", "JO", "T2"], record)?)
        }
        SourceKind::Website => {
            s.website = String::from(scalar(fields, &["T2"], record)?);
            for tag in ["JF", "JO", "VL", "IS", "ET", "SP", "EP"] {
                if fields.iter().any(|f| f.tag == tag) {
                    return Err(fail(
                        record,
                        tag,
                        "abweichende ELEC-Feldbedeutung nicht unterstützt",
                    ));
                }
            }
        }
        SourceKind::Book => {
            for tag in ["JF", "JO", "T2"] {
                if fields.iter().any(|f| f.tag == tag) {
                    return Err(fail(
                        record,
                        tag,
                        "Zeitschriften-/Reihentitel nicht darstellbar",
                    ));
                }
            }
        }
    }
    let end = scalar(fields, &["EP"], record)?;
    if !end.is_empty() {
        if s.pages.is_empty()
            || s.pages.contains(['-', '–', '\n'])
            || end.contains(['-', '–', '\n'])
        {
            return Err(fail(record, "SP/EP", "Seitenbereich ist mehrdeutig"));
        }
        s.pages.push('-');
        s.pages.push_str(end);
    }
    let d = scalar(fields, &["DO"], record)?;
    if !d.is_empty() {
        let url = doi(d, record)?;
        if !s.url.is_empty() && s.url != url {
            return Err(fail(
                record,
                "DO/UR",
                "DOI und abweichende URL benötigen getrennte Felder",
            ));
        }
        s.url = url;
    }
    let value = scalar(fields, &["Y2"], record)?;
    if !value.is_empty() {
        [s.year_accessed, s.month_accessed, s.day_accessed] = access(value, record)?;
    }
    s.validate()
        .map_err(|_| fail(record, "Quelle", "ungültige Quelle oder Feldgröße"))?;
    Ok(s)
}

/// Decode a complete bounded RIS list: BOOK/JOUR/ELEC, people in AU/A1 order,
/// TI/T1 (BOOK: BT), PY/Y1 year only, PB/CY/ET, JOUR JF/JO/T2, ELEC T2,
/// VL/IS/SP/EP (not ELEC), UR/DO, SN/ST/N1/LA and Y2 access date.
/// UTF-8/BOM and BOM UTF-16, LF/CRLF and multiline values are accepted. Continuations
/// strip one six-space indent; other continuation text is preserved with LF.
/// ID is validated then remapped sequentially; use Edit::ImportSources for document IDs.
/// No corporate/suffix inference, role conversion, abstract/keyword/attachment dropping,
/// or publication-date truncation. Unknown or duplicate scalar tags abort the whole list.
/// DOI occupies the URL only when a safe doi.org URL and any UR agree. LA is kept verbatim.
/// Limits: 4 MiB encoded/decoded input, 4096 tags, 1024 records, 4096 bytes per field,
/// 64 authors per record and 2 MiB retained model. No input causes filesystem/network access.
pub fn decode_ris(bytes: &[u8]) -> Result<Vec<Source>, Error> {
    let (text, _) = encoding::decode(bytes, INPUT_LIMIT).map_err(|e| match e {
        encoding::Error::Limit => Error::Limit,
        _ => fail(
            1,
            "Kodierung",
            "gültiges UTF-8 oder UTF-16 mit BOM erforderlich",
        ),
    })?;
    let mut out = Vec::new();
    let mut fields: Vec<Field> = Vec::new();
    let mut ids = BTreeSet::new();
    let mut tags = 0usize;
    let mut total = 0usize;
    for line in text.split_terminator('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let record = out.len() + 1;
        if let Some((tag, value)) = tagged(line) {
            tags += 1;
            if tags > FIELD_LIMIT {
                return Err(Error::Limit);
            }
            if tag == "TY" {
                if !fields.is_empty() {
                    return Err(fail(record, "TY", "vorheriges ER fehlt"));
                }
                if out.len() == 1024 {
                    return Err(Error::Limit);
                }
            } else if fields.is_empty() {
                return Err(fail(record, tag, "TY als erstes Feld erforderlich"));
            }
            if tag == "ER" {
                if !value.is_empty() {
                    return Err(fail(record, tag, "ER darf keinen Wert enthalten"));
                }
                let s = source(&fields, record)?;
                let id = scalar(&fields, &["ID"], record)?;
                if !id.is_empty() && !ids.insert(String::from(id)) {
                    return Err(fail(record, "ID", "doppelte Referenzkennung"));
                }
                total += s.byte_size();
                if total > MODEL_LIMIT {
                    return Err(Error::Limit);
                }
                out.push(s);
                fields.clear();
            } else {
                if value.len() > FIELD_LIMIT {
                    return Err(Error::Limit);
                }
                fields.push(Field {
                    tag: String::from(tag),
                    value: String::from(value),
                });
            }
        } else if let Some(last) = fields.last_mut() {
            let continuation = line.strip_prefix("      ").unwrap_or(line);
            if last.value.len() + 1 + continuation.len() > FIELD_LIMIT {
                return Err(Error::Limit);
            }
            last.value.push('\n');
            last.value.push_str(continuation);
        } else if !line.trim().is_empty() {
            return Err(fail(
                record,
                "Zeile",
                "Text außerhalb eines TY/ER-Datensatzes",
            ));
        }
    }
    if !fields.is_empty() {
        return Err(fail(out.len() + 1, "ER", "Datensatz nicht abgeschlossen"));
    }
    Ok(out)
}
fn write(out: &mut String, tag: &str, value: &str, record: usize) -> Result<(), Error> {
    if value.is_empty() {
        return Ok(());
    }
    // CR has no lossless representation in a line-oriented interchange format.
    if value.contains('\r') {
        return Err(fail(
            record,
            tag,
            "CR innerhalb eines Feldes nicht darstellbar",
        ));
    }
    out.push_str(tag);
    out.push_str("  - ");
    let mut first = true;
    for line in value.split('\n') {
        if !first {
            out.push_str("      ");
        }
        first = false;
        out.push_str(line);
        out.push_str("\r\n");
    }
    if out.len() > INPUT_LIMIT {
        return Err(Error::Limit);
    }
    Ok(())
}
/// Export deterministic UTF-8 RIS with CRLF and indented continuation lines.
/// Only fields represented by decode_ris are accepted; source IDs are local and omitted.
/// Corporate authors, separately stored middle names, external GUIDs, incompatible
/// type-specific fields and non-year publication dates produce record/tag diagnostics.
/// People with commas or outer whitespace in name components cannot be emitted faithfully.
/// Validation includes decoding the complete result and model equality (except source IDs),
/// so no partial output or silently omitted metadata is ever returned.
pub fn encode_ris(sources: &[Source]) -> Result<Vec<u8>, Error> {
    if sources.len() > 1024 || sources.iter().map(Source::byte_size).sum::<usize>() > MODEL_LIMIT {
        return Err(Error::Limit);
    }
    let mut out = String::new();
    for (index, s) in sources.iter().enumerate() {
        let record = index + 1;
        s.validate()?;
        for (tag, value) in [("AU/Körperschaft", &s.corporate), ("GUID", &s.external_id)] {
            if !value.is_empty() {
                return Err(fail(
                    record,
                    tag,
                    "kein eindeutiges RIS-Feld im unterstützten Profil",
                ));
            }
        }
        let ty = match s.kind {
            SourceKind::Book => "BOOK",
            SourceKind::JournalArticle => "JOUR",
            SourceKind::Website => "ELEC",
        };
        write(&mut out, "TY", ty, record)?;
        for p in &s.authors {
            if !p.middle.is_empty()
                || p.family.is_empty()
                || p.family.contains(',')
                || p.given.contains(',')
            {
                return Err(fail(
                    record,
                    "AU",
                    "getrennte Mittelnamen, Kommas oder fehlender Nachname nicht darstellbar",
                ));
            }
            single_line(&p.family, record, "AU")?;
            single_line(&p.given, record, "AU")?;
            let person = if p.given.is_empty() {
                p.family.clone()
            } else {
                format!("{}, {}", p.family, p.given)
            };
            write(&mut out, "AU", &person, record)?;
        }
        macro_rules! put {
            ($tag:literal, $field:ident) => {
                write(&mut out, $tag, &s.$field, record)?;
            };
        }
        put!("TI", title);
        put!("PY", year);
        put!("PB", publisher);
        put!("CY", city);
        put!("ET", edition);
        if s.kind != SourceKind::JournalArticle && !s.journal.is_empty() {
            return Err(fail(record, "JF", "Zeitschrift nur bei JOUR darstellbar"));
        }
        if s.kind != SourceKind::Website && !s.website.is_empty() {
            return Err(fail(record, "T2", "Websitetitel nur bei ELEC darstellbar"));
        }
        put!("JF", journal);
        put!("T2", website);
        put!("VL", volume);
        put!("IS", issue);
        put!("SP", pages);
        put!("UR", url);
        put!("SN", standard_number);
        put!("ST", short_title);
        put!("N1", comments);
        put!("LA", language);
        if !s.year_accessed.is_empty() || !s.month_accessed.is_empty() || !s.day_accessed.is_empty()
        {
            let mut date = s.year_accessed.clone();
            if !s.month_accessed.is_empty() || !s.day_accessed.is_empty() {
                date.push('/');
                date.push_str(&s.month_accessed);
            }
            if !s.day_accessed.is_empty() {
                date.push('/');
                date.push_str(&s.day_accessed);
            }
            write(&mut out, "Y2", &date, record)?;
        }
        out.push_str("ER  -\r\n");
    }
    let readback = decode_ris(out.as_bytes())?;
    for (index, (old, new)) in sources.iter().zip(&readback).enumerate() {
        let mut expected = old.clone();
        expected.id = index as u64 + 1;
        if expected != *new {
            return Err(fail(
                index + 1,
                "Quelle",
                "Feld würde beim RIS-Rundlauf verändert",
            ));
        }
    }
    Ok(out.into_bytes())
}

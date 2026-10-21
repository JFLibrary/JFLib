// Bounded, explicit Unicode BibTeX interchange subset. New native library code.
// Syntax reference: Oren Patashnik, BibTeXing, https://texdoc.org/serve/btxdoc/0.
// This is a metadata codec, not a TeX interpreter or a BibLaTeX style processor.
use crate::Error;
use alloc::{collections::BTreeSet, format, string::String, vec::Vec};
use doc_core::{Person, Source, SourceKind};

const INPUT_LIMIT: usize = 4 << 20;
const MODEL_LIMIT: usize = 2 << 20;
const AUTHOR_LIMIT: usize = 64 * (4 * 4096 + 16);

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    entry: String,
    field: String,
}
impl<'a> Parser<'a> {
    fn error(&self, why: &str) -> Error {
        issue(&self.entry, &self.field, why)
    }
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }
    fn space(&mut self) {
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                self.pos += 1;
            }
            if self.peek() != Some(b'%') {
                break;
            }
            while self.peek().is_some_and(|c| c != b'\n' && c != b'\r') {
                self.pos += 1;
            }
        }
    }
    fn take(&mut self, byte: u8) -> Result<(), Error> {
        self.space();
        if self.peek() != Some(byte) {
            return Err(self.error("Unerwartetes oder fehlendes Trennzeichen"));
        }
        self.pos += 1;
        Ok(())
    }
    fn word(&mut self) -> Result<String, Error> {
        self.space();
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        if self.pos == start || self.pos - start > 64 {
            return Err(self.error("Eintragstyp oder Feldname aus ASCII-Buchstaben erwartet"));
        }
        Ok(self.text[start..self.pos].to_ascii_lowercase())
    }
    // Return the original inner text. Grouping survives lexing so decoding cannot
    // silently discard case protection, TeX commands or corporate author grouping.
    fn value(&mut self) -> Result<&'a str, Error> {
        self.space();
        let limit = if self.field == "author" {
            AUTHOR_LIMIT
        } else {
            2 * 4096 + 16
        };
        let first = self.peek().ok_or_else(|| self.error("Feldwert fehlt"))?;
        if first.is_ascii_digit() {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos - start > limit {
                return Err(self.error("Feldwert überschreitet die unterstützte Größe"));
            }
            return Ok(&self.text[start..self.pos]);
        }
        if first != b'{' && first != b'"' {
            return Err(self.error("Textmakros werden nicht unterstützt; Wert in Klammern oder Anführungszeichen angeben"));
        }
        self.pos += 1;
        let start = self.pos;
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            if self.pos - start > limit {
                return Err(self.error("Feldwert überschreitet die unterstützte Größe"));
            }
            if c == b'\\' {
                self.pos += 1;
                if self.peek().is_none() {
                    return Err(self.error("Unvollständige Escape-Sequenz"));
                }
                // The value decoder rejects brace escapes and TeX commands. Skipping
                // here merely finds the proper error location without losing bytes.
                self.pos += self.text[self.pos..].chars().next().unwrap().len_utf8();
                continue;
            }
            if depth == 0 && ((first == b'{' && c == b'}') || (first == b'"' && c == b'"')) {
                let end = self.pos;
                self.pos += 1;
                return Ok(&self.text[start..end]);
            }
            match c {
                b'{' => {
                    depth += 1;
                    if depth > 48 {
                        return Err(self.error("Klammerung überschreitet 48 Ebenen"));
                    }
                }
                b'}' => {
                    if depth == 0 {
                        return Err(self.error("Unausgeglichene Klammern im zitierten Wert"));
                    }
                    depth -= 1;
                }
                _ => (),
            }
            self.pos += 1;
        }
        Err(self.error("Nicht abgeschlossener Wert in Klammern oder Anführungszeichen"))
    }
    fn comment(&mut self, open: u8) -> Result<(), Error> {
        let close = if open == b'{' { b'}' } else { b')' };
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == open {
                depth += 1;
                if depth > 48 {
                    return Err(self.error("Kommentarverschachtelung überschreitet 48 Ebenen"));
                }
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
        Err(self.error("Nicht abgeschlossener Kommentar"))
    }
}
fn issue(entry: &str, field: &str, why: &str) -> Error {
    Error::Unsupported(format!("BibTeX [{entry}] {field}: {why}"))
}
fn normalized(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}
fn scalar(raw: &str, url: bool, entry: &str, field: &str) -> Result<String, Error> {
    let mut out = String::new();
    let mut chars = raw.chars();
    let mut space = false;
    while let Some(c) = chars.next() {
        let c = match c {
            '{' | '}' => {
                return Err(issue(
                    entry,
                    field,
                    "Geschützte Großschreibung und wörtliche Klammern können nicht abgebildet werden",
                ));
            }
            '\\' => match chars.next() {
                Some(c @ ('&' | '%' | '_' | '#' | '$')) if !url => c,
                _ => {
                    return Err(issue(
                        entry,
                        field,
                        "Nicht unterstützter TeX-Befehl oder Escape-Sequenz; Unicode-Text verwenden",
                    ));
                }
            },
            '&' | '%' | '_' | '#' | '$' | '^' | '~' if !url => {
                return Err(issue(entry, field, "Nicht maskiertes TeX-Sonderzeichen"));
            }
            c if c.is_control() && !c.is_ascii_whitespace() => {
                return Err(issue(entry, field, "Steuerzeichen im Feld"));
            }
            _ => c,
        };
        if c.is_ascii_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
        if out.len() > 4096 {
            return Err(issue(entry, field, "Feld überschreitet 4096 Bytes"));
        }
    }
    Ok(out)
}
fn author_parts(raw: &str) -> Result<Vec<&str>, ()> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let b = raw.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 1;
            if i < b.len() {
                i += raw[i..].chars().next().ok_or(())?.len_utf8();
            }
            continue;
        }
        if b[i] == b'{' {
            depth += 1;
        } else if b[i] == b'}' {
            depth = depth.checked_sub(1).ok_or(())?;
        } else if depth == 0
            && i > 0
            && b[i - 1].is_ascii_whitespace()
            && b.get(i..i + 3)
                .is_some_and(|s| s.eq_ignore_ascii_case(b"and"))
            && b.get(i + 3).is_some_and(|c| c.is_ascii_whitespace())
        {
            parts.push(raw[start..i].trim());
            if parts.len() >= 64 {
                return Err(());
            }
            i += 3;
            start = i;
            continue;
        }
        i += 1;
    }
    if depth != 0 {
        return Err(());
    }
    parts.push(raw[start..].trim());
    Ok(parts)
}
fn read_authors(raw: &str, s: &mut Source, entry: &str) -> Result<(), Error> {
    if raw.trim().is_empty() {
        return Ok(());
    }
    let parts = author_parts(raw).map_err(|_| {
        issue(
            entry,
            "author",
            "Unausgeglichene Gruppierung oder mehr als 64 Autoren",
        )
    })?;
    for part in &parts {
        if part.starts_with('{') && part.ends_with('}') {
            if parts.len() != 1 {
                return Err(issue(
                    entry,
                    "author",
                    "Gemischte Personen und Organisationen als Autoren können nicht abgebildet werden",
                ));
            }
            s.corporate = scalar(&part[1..part.len() - 1], false, entry, "author")?;
            if s.corporate.is_empty() {
                return Err(issue(entry, "author", "Leere Organisation als Autor"));
            }
        } else {
            let mut pieces = part.split(',');
            let family = scalar(pieces.next().unwrap_or(""), false, entry, "author.family")?;
            let given = pieces
                .next()
                .map(|s| scalar(s, false, entry, "author.given"))
                .transpose()?;
            if pieces.next().is_some()
                || family.is_empty()
                || (given.is_none() && family.chars().any(char::is_whitespace))
                || family
                    .split_ascii_whitespace()
                    .chain(given.as_deref().unwrap_or("").split_ascii_whitespace())
                    .any(|w| w.eq_ignore_ascii_case("and"))
            {
                return Err(issue(
                    entry,
                    "author",
                    "Nachname, Vorname verwenden; mehrdeutige Namen und Namenszusätze werden nicht unterstützt",
                ));
            }
            if family.eq_ignore_ascii_case("others") && given.is_none() {
                return Err(issue(
                    entry,
                    "author",
                    "Der besondere Autoreneintrag 'others' kann nicht abgebildet werden",
                ));
            }
            s.authors.push(Person {
                family,
                given: given.unwrap_or_default(),
                middle: String::new(),
            });
        }
    }
    Ok(())
}
fn read_date(text: &str, s: &mut Source, entry: &str) -> Result<(), Error> {
    let fields = text.split('-').collect::<Vec<_>>();
    let valid = fields.len() <= 3
        && fields[0].len() == 4
        && fields.iter().all(|f| f.bytes().all(|c| c.is_ascii_digit()))
        && (fields.len() < 2
            || (fields[1].len() == 2 && matches!(fields[1].parse::<u8>(), Ok(1..=12))))
        && (fields.len() < 3
            || (fields[2].len() == 2 && matches!(fields[2].parse::<u8>(), Ok(1..=31))));
    if !valid {
        return Err(issue(
            entry,
            "urldate",
            "JJJJ, JJJJ-MM oder JJJJ-MM-TT erwartet",
        ));
    }
    if fields.len() == 3 {
        let year = fields[0].parse::<u32>().unwrap();
        let month = fields[1].parse::<u8>().unwrap();
        let days = match month {
            2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        if fields[2].parse::<u8>().unwrap() > days {
            return Err(issue(entry, "urldate", "Ungültiger Tag für diesen Monat"));
        }
    }
    s.year_accessed = String::from(fields[0]);
    s.month_accessed = String::from(fields.get(1).copied().unwrap_or(""));
    s.day_accessed = String::from(fields.get(2).copied().unwrap_or(""));
    Ok(())
}
fn read_field(s: &mut Source, key: &str, field: &str, raw: &str) -> Result<(), Error> {
    if field == "author" {
        return read_authors(raw, s, key);
    }
    let value = scalar(raw, field == "url", key, field)?;
    match field {
        "title" => s.title = value,
        "year" => s.year = value,
        "publisher" => s.publisher = value,
        "address" => s.city = value,
        "edition" => s.edition = value,
        "journal" => s.journal = value,
        "volume" => s.volume = value,
        "number" => s.issue = value,
        "pages" => s.pages = value,
        "url" => s.url = value,
        "note" => s.comments = value,
        "isbn" if s.kind == SourceKind::Book => s.standard_number = value,
        "issn" if s.kind == SourceKind::JournalArticle => s.standard_number = value,
        "howpublished" if s.kind == SourceKind::Website => s.website = value,
        "urldate" => read_date(&value, s, key)?,
        _ => {
            return Err(issue(
                key,
                field,
                "Nicht unterstütztes Feld oder unpassende Kombination von Feld und Quellentyp",
            ));
        }
    }
    Ok(())
}

/// Decode a bounded Unicode BibTeX subset (@book, @article, @online and web @misc).
/// Values may be braced, quoted or numeric. Names use Family, Given, single-token
/// family names, or one braced corporation. UTF-8 and BOM-marked UTF-16 are accepted.
/// Entry keys are validated and remapped to sequential IDs, not retained as metadata.
/// Unsupported fields, TeX commands, case protection, macros and suffixes abort the
/// entire import. Limits: 4 MiB input, 1024 sources, 64 authors, 4096 bytes per scalar,
/// 2 MiB retained model. ASCII field whitespace follows BibTeX normalization.
pub fn decode_bibtex(bytes: &[u8]) -> Result<Vec<Source>, Error> {
    let (text, _) = base::encoding::decode(bytes, INPUT_LIMIT).map_err(|e| match e {
        base::encoding::Error::Limit => Error::Limit,
        _ => issue(
            "Eingabe",
            "Zeichenkodierung",
            "Gültiges UTF-8 oder UTF-16 mit BOM erwartet",
        ),
    })?;
    let mut p = Parser {
        text: &text,
        pos: 0,
        entry: String::from("Eingabe"),
        field: String::new(),
    };
    let mut out = Vec::new();
    let mut keys = BTreeSet::new();
    let mut retained = 0usize;
    loop {
        p.space();
        if p.peek().is_none() {
            break;
        }
        p.field.clear();
        p.entry = format!("Datensatz {}", out.len() + 1);
        p.take(b'@')?;
        let kind = p.word()?;
        p.space();
        let open = p.peek().ok_or_else(|| p.error("Eintragsklammer fehlt"))?;
        if !matches!(open, b'{' | b'(') {
            return Err(p.error("Geschweifte oder runde Eintragsklammer erwartet"));
        }
        p.pos += 1;
        if kind == "comment" {
            p.comment(open)?;
            continue;
        }
        let source_kind = match kind.as_str() {
            "book" => SourceKind::Book,
            "article" => SourceKind::JournalArticle,
            "online" | "misc" => SourceKind::Website,
            _ => return Err(p.error("Nicht unterstützter Eintragstyp, Makro oder Präambel")),
        };
        if out.len() >= 1024 {
            return Err(Error::Limit);
        }
        p.space();
        let start = p.pos;
        while p
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || b"_:-.+/".contains(&c))
        {
            p.pos += 1;
        }
        if p.pos == start || p.pos - start > 256 {
            return Err(
                p.error("Eintragsschlüssel muss 1 bis 256 unterstützte ASCII-Zeichen enthalten")
            );
        }
        p.entry = String::from(&text[start..p.pos]);
        if !keys.insert(p.entry.clone()) {
            return Err(p.error("Doppelter Eintragsschlüssel"));
        }
        p.take(b',')?;
        let mut s = Source {
            id: out.len() as u64 + 1,
            kind: source_kind,
            ..Default::default()
        };
        let mut fields = BTreeSet::new();
        let close = if open == b'{' { b'}' } else { b')' };
        loop {
            p.space();
            if p.peek() == Some(close) {
                p.pos += 1;
                break;
            }
            p.field = p.word()?;
            if !fields.insert(p.field.clone()) {
                return Err(p.error("Doppeltes Feld"));
            }
            p.take(b'=')?;
            let raw = p.value()?;
            read_field(&mut s, &p.entry, &p.field, raw)?;
            p.space();
            if p.peek() == Some(b'#') {
                return Err(p.error("Verkettung von Feldwerten wird nicht unterstützt"));
            }
            if p.peek() == Some(close) {
                p.pos += 1;
                break;
            }
            p.take(b',')?;
        }
        if kind == "misc" && s.url.is_empty() {
            return Err(issue(
                &p.entry,
                "url",
                "@misc wird nur für Websites mit URL unterstützt",
            ));
        }
        s.validate().map_err(|_| {
            issue(
                &p.entry,
                "Datensatz",
                "Titel fehlt oder Quellentext ist ungültig",
            )
        })?;
        retained += s.byte_size();
        if retained > MODEL_LIMIT {
            return Err(Error::Limit);
        }
        out.push(s);
    }
    Ok(out)
}

fn escaped(text: &str, url: bool, entry: &str, field: &str) -> Result<String, Error> {
    if normalized(text) != text {
        return Err(issue(
            entry,
            field,
            "Leerraum würde sich in BibTeX ändern; einzelne Leerzeichen verwenden",
        ));
    }
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '{' | '}' | '\\' => {
                return Err(issue(
                    entry,
                    field,
                    "Wörtliche Klammern und Backslashes können nicht exportiert werden",
                ));
            }
            '^' | '~' if !url => {
                return Err(issue(
                    entry,
                    field,
                    "Dieses wörtliche TeX-Sonderzeichen kann nicht exportiert werden",
                ));
            }
            c @ ('&' | '%' | '_' | '#' | '$') if !url => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    Ok(out)
}
fn write_field(out: &mut String, key: &str, field: &str, value: &str) -> Result<(), Error> {
    if !value.is_empty() {
        let value = escaped(value, field == "url", key, field)?;
        out.push_str(&format!("  {field} = {{{value}}},\n"));
    }
    Ok(())
}
fn write_authors(s: &Source, key: &str) -> Result<String, Error> {
    if !s.corporate.is_empty() {
        return Ok(format!(
            "{{{}}}",
            escaped(&s.corporate, false, key, "author")?
        ));
    }
    let mut names = Vec::new();
    for p in &s.authors {
        if !p.middle.is_empty() {
            return Err(issue(
                key,
                "author.middle",
                "BibTeX hat kein getrenntes Feld für weitere Vornamen",
            ));
        }
        if p.family.is_empty()
            || p.family.contains(',')
            || p.given.contains(',')
            || [&p.family, &p.given].iter().any(|s| {
                s.split_ascii_whitespace()
                    .any(|w| w.eq_ignore_ascii_case("and"))
            })
            || (p.family.eq_ignore_ascii_case("others") && p.given.is_empty())
        {
            return Err(issue(
                key,
                "author",
                "Name kann nicht ohne Bedeutungsänderung exportiert werden",
            ));
        }
        let family = escaped(&p.family, false, key, "author.family")?;
        let given = escaped(&p.given, false, key, "author.given")?;
        // A comma makes a compound family name explicit even without given names.
        names.push(
            if !given.is_empty() || family.chars().any(char::is_whitespace) {
                format!("{family}, {given}")
            } else {
                family
            },
        );
    }
    Ok(names.join(" and "))
}

/// Encode the representable Source subset as deterministic UTF-8 BibTeX.
/// Keys are jf<ID>; IDs must be nonzero, unique and below 0x7fff_ffff. Separate middle
/// names, external IDs, language, short titles and other unmapped metadata are errors.
/// Export validates the entire list and refuses values whose meaning/spacing would
/// change. Unicode is emitted directly; no TeX commands, resource loading or execution.
pub fn encode_bibtex(sources: &[Source]) -> Result<Vec<u8>, Error> {
    if sources.len() > 1024 || sources.iter().map(Source::byte_size).sum::<usize>() > MODEL_LIMIT {
        return Err(Error::Limit);
    }
    let mut ids = BTreeSet::new();
    let mut out = String::new();
    for s in sources {
        s.validate()?;
        let key = format!("jf{}", s.id);
        if s.id == 0 || s.id >= 0x7fff_ffff || !ids.insert(s.id) {
            return Err(issue(
                &key,
                "id",
                "Quellen-IDs müssen eindeutig und gültig sein",
            ));
        }
        for (field, value) in [
            ("external_id", &s.external_id),
            ("language", &s.language),
            ("short_title", &s.short_title),
        ] {
            if !value.is_empty() {
                return Err(issue(
                    &key,
                    field,
                    "Quellenfeld wird durch diesen BibTeX-Umfang nicht abgebildet",
                ));
            }
        }
        if !s.website.is_empty() && s.kind != SourceKind::Website {
            return Err(issue(
                &key,
                "website",
                "Website-Titel erfordert eine Website als Quellentyp",
            ));
        }
        if !s.standard_number.is_empty() && s.kind == SourceKind::Website {
            return Err(issue(
                &key,
                "standard_number",
                "Standardnummer einer Website hat keine unterstützte Abbildung",
            ));
        }
        let kind = match s.kind {
            SourceKind::Book => "book",
            SourceKind::JournalArticle => "article",
            SourceKind::Website => "online",
        };
        out.push_str(&format!("@{kind}{{{key},\n"));
        let authors = write_authors(s, &key)?;
        if !authors.is_empty() {
            out.push_str(&format!("  author = {{{authors}}},\n"));
        }
        for (field, value) in [
            ("title", &s.title),
            ("year", &s.year),
            ("publisher", &s.publisher),
            ("address", &s.city),
            ("edition", &s.edition),
            ("journal", &s.journal),
            ("volume", &s.volume),
            ("number", &s.issue),
            ("pages", &s.pages),
            ("url", &s.url),
            ("note", &s.comments),
            ("howpublished", &s.website),
            (
                if s.kind == SourceKind::Book {
                    "isbn"
                } else {
                    "issn"
                },
                &s.standard_number,
            ),
        ] {
            write_field(&mut out, &key, field, value)?;
        }
        if !s.year_accessed.is_empty() || !s.month_accessed.is_empty() || !s.day_accessed.is_empty()
        {
            let mut date = s.year_accessed.clone();
            if !s.month_accessed.is_empty() {
                date.push('-');
                date.push_str(&s.month_accessed);
            }
            if !s.day_accessed.is_empty() {
                date.push('-');
                date.push_str(&s.day_accessed);
            }
            let mut parsed = Source::default();
            read_date(&date, &mut parsed, &key)?;
            if parsed.year_accessed != s.year_accessed
                || parsed.month_accessed != s.month_accessed
                || parsed.day_accessed != s.day_accessed
            {
                return Err(issue(
                    &key,
                    "urldate",
                    "Bestandteile des Zugriffsdatums können nicht exakt abgebildet werden",
                ));
            }
            write_field(&mut out, &key, "urldate", &date)?;
        }
        out.push_str("}\n\n");
        if out.len() > INPUT_LIMIT {
            return Err(Error::Limit);
        }
    }
    // Names and escaped scalar bounds must also satisfy the decoder. This is a
    // bounded all-or-nothing check; no output is exposed until it has succeeded.
    let decoded = decode_bibtex(out.as_bytes())?;
    for (source, decoded) in sources.iter().zip(decoded) {
        let mut expected = source.clone();
        expected.id = decoded.id;
        if expected != decoded {
            return Err(issue(
                &format!("jf{}", source.id),
                "Datensatz",
                "Quelle kann in diesem BibTeX-Umfang nicht verlustfrei gespeichert und wieder eingelesen werden",
            ));
        }
    }
    Ok(out.into_bytes())
}

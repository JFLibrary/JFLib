// Bounded Word CITATION field instructions. New native code; ECMA-376 Part 1
// 17.16.5.8 and MS-OI29500 2.1.464 define switches and their per-source scope.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{CitationItem, Inline};

use crate::{Error, source_id, unsupported};

pub(super) fn write(items: &[CitationItem]) -> String {
    let mut out = String::from(" CITATION");
    for (index, item) in items.iter().enumerate() {
        if index != 0 {
            out.push_str(" \\m");
        }
        out.push_str(&format!(" jf{}", item.source));
        for (switch, value) in [
            ("p", item.locator.as_str()),
            ("f", item.prefix.as_str()),
            ("s", item.suffix.as_str()),
        ] {
            if !value.is_empty() {
                out.push_str(&format!(" \\{switch} \"{value}\""));
            }
        }
        for (switch, enabled) in [
            ("n", item.suppress_author),
            ("y", item.suppress_year),
            ("t", item.suppress_title),
        ] {
            if enabled {
                out.push_str(&format!(" \\{switch}"));
            }
        }
    }
    out.push(' ');
    out
}

struct Token<'a> {
    value: &'a str,
    quoted: bool,
}

// A quoted argument is deliberately not a general escape language. The model
// rejects quotes/backslashes inside values, so accepting those would lose data.
fn next<'a>(tail: &mut &'a str) -> Result<Option<Token<'a>>, Error> {
    *tail = tail.trim_start_matches(|c: char| c.is_ascii_whitespace());
    if tail.is_empty() {
        return Ok(None);
    }
    if let Some(quoted) = tail.strip_prefix('"') {
        let end = quoted.find('"').ok_or(Error::Invalid)?;
        let value = &quoted[..end];
        if value.contains('\\') {
            return Err(Error::Invalid);
        }
        let rest = &quoted[end + 1..];
        if rest
            .as_bytes()
            .first()
            .is_some_and(|b| !b.is_ascii_whitespace())
        {
            return Err(Error::Invalid);
        }
        *tail = rest;
        Ok(Some(Token {
            value,
            quoted: true,
        }))
    } else {
        let end = tail
            .as_bytes()
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(tail.len());
        let value = &tail[..end];
        if value.contains('"') {
            return Err(Error::Invalid);
        }
        *tail = &tail[end..];
        Ok(Some(Token {
            value,
            quoted: false,
        }))
    }
}

fn argument<'a>(tail: &mut &'a str) -> Result<&'a str, Error> {
    let token = next(tail)?.ok_or(Error::Invalid)?;
    if token.value.contains('\\') {
        return Err(Error::Invalid);
    }
    Ok(token.value)
}

pub(super) fn read(instr: &str) -> Result<Inline, Error> {
    if instr.len() > 16 << 10 {
        return Err(Error::Limit);
    }
    let mut tail = instr;
    let field = next(&mut tail)?.ok_or(Error::Invalid)?;
    if field.quoted || field.value != "CITATION" {
        return Err(unsupported("Feldtyp"));
    }
    let first = source_id(argument(&mut tail)?)?;
    let mut items: Vec<CitationItem> = vec![CitationItem {
        source: first,
        ..Default::default()
    }];
    let mut seen = 0u8;
    while let Some(token) = next(&mut tail)? {
        if token.quoted {
            return Err(Error::Invalid);
        }
        if token.value == "\\m" {
            if items.len() == 32 {
                return Err(Error::Limit);
            }
            let source = source_id(argument(&mut tail)?)?;
            items.push(CitationItem {
                source,
                ..Default::default()
            });
            seen = 0;
            continue;
        }
        let bit = match token.value {
            "\\p" => 1,
            "\\f" => 2,
            "\\s" => 4,
            "\\n" => 8,
            "\\y" => 16,
            "\\t" => 32,
            _ => return Err(unsupported("Zitierfeld-Schalter")),
        };
        if seen & bit != 0 {
            return Err(Error::Invalid);
        }
        seen |= bit;
        let item = items.last_mut().ok_or(Error::Invalid)?;
        match token.value {
            "\\p" => item.locator = String::from(argument(&mut tail)?),
            "\\f" => item.prefix = String::from(argument(&mut tail)?),
            "\\s" => item.suffix = String::from(argument(&mut tail)?),
            "\\n" => item.suppress_author = true,
            "\\y" => item.suppress_year = true,
            "\\t" => item.suppress_title = true,
            _ => return Err(Error::Invalid),
        }
    }
    Ok(Inline::citation(items)?)
}

// JSON as Cargo writes it (cargo metadata, --message-format=json): enough to read, nothing to write.

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

const NOTHING: &[Value] = &[];

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// the elements of an array, none for anything else
    pub fn arr(&self) -> &[Value] {
        match self {
            Value::Arr(items) => items,
            _ => NOTHING,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// the field `key` as a string, "" when it is none
    pub fn s(&self, key: &str) -> &str {
        self.get(key).and_then(Value::str).unwrap_or("")
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON: {what} at byte {}", self.i))
    }

    fn eat(&mut self, word: &str) -> bool {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 128 {
            return self.err("nested too deep");
        }
        self.ws();
        let Some(&c) = self.b.get(self.i) else { return self.err("unexpected end") };
        match c {
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.eat("}") {
                    return Ok(Value::Obj(fields));
                }
                loop {
                    self.ws();
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return self.err("':' expected");
                    }
                    fields.push((key, self.value(depth + 1)?));
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        return Ok(Value::Obj(fields));
                    }
                    return self.err("',' or '}' expected");
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Ok(Value::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Value::Arr(items));
                    }
                    return self.err("',' or ']' expected");
                }
            }
            b'"' => Ok(Value::Str(self.string()?)),
            b't' if self.eat("true") => Ok(Value::Bool(true)),
            b'f' if self.eat("false") => Ok(Value::Bool(false)),
            b'n' if self.eat("null") => Ok(Value::Null),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.b[start..self.i]).unwrap_or("");
                text.parse::<f64>().map(Value::Num).or_else(|_| self.err("bad number"))
            }
            _ => self.err("unexpected character"),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.b.get(self.i..self.i + 4).and_then(|d| std::str::from_utf8(d).ok());
        let Some(n) = digits.and_then(|d| u32::from_str_radix(d, 16).ok()) else { return self.err("bad \\u escape") };
        self.i += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, String> {
        if !self.eat("\"") {
            return self.err("'\"' expected");
        }
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "JSON: a string is not UTF-8".to_string())?);
            match self.b.get(self.i) {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => {}
            }
            self.i += 1; // the backslash
            let Some(&e) = self.b.get(self.i) else { return self.err("unterminated escape") };
            self.i += 1;
            match e {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    let mut c = self.hex4()?;
                    if (0xD800..0xDC00).contains(&c) && self.eat("\\u") {
                        let low = self.hex4()?;
                        c = 0x10000 + ((c - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                    }
                    out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                }
                _ => return self.err("bad escape"),
            }
        }
    }
}

/// one JSON document
pub(crate) fn parse(text: &str) -> Result<Value, String> {
    let mut r = Reader { b: text.as_bytes(), i: 0 };
    let v = r.value(0)?;
    r.ws();
    if r.i != r.b.len() {
        return r.err("text after the value");
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_messages() {
        let v = parse(r#"{"reason":"compiler-artifact","target":{"kind":["cdylib"],"name":"hallo"},"filenames":["C:\\x\\hallo.wasm"],"fresh":false,"n":-1.5e2,"e":null,"u":"\u00e4\ud83d\ude00"}"#).unwrap();
        assert_eq!(v.s("reason"), "compiler-artifact");
        assert_eq!(v.get("target").unwrap().get("kind").unwrap().arr()[0].str(), Some("cdylib"));
        assert_eq!(v.get("filenames").unwrap().arr()[0].str(), Some("C:\\x\\hallo.wasm"));
        assert_eq!(v.get("fresh"), Some(&Value::Bool(false)));
        assert_eq!(v.get("n"), Some(&Value::Num(-150.0)));
        assert!(v.get("e").unwrap().is_null());
        assert_eq!(v.s("u"), "ä😀");
        assert!(parse("{\"a\":1,}").is_err());
        assert!(parse("[1 2]").is_err());
        assert_eq!(parse(" [] ").unwrap(), Value::Arr(vec![]));
    }
}

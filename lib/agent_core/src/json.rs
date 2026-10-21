// Strict, bounded UTF-8 JSON for local and browser control channels. New shared code.

use alloc::{
    collections::BTreeSet,
    format,
    string::{String, ToString},
    vec::Vec,
};

/// Objects with more members than this check duplicate names with an ordered set instead of
/// comparing every pair (same results, no quadratic time on large objects).
const SMALL_OBJECT: usize = 16;

/// Exact JSON number token; construction validates the JSON number grammar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Number(String);

impl Number {
    /// Parse one complete JSON number without rounding it through floating point.
    pub fn parse(text: &str) -> Option<Self> {
        let mut p = Parser::new(text, Limits::default());
        let number = p.number().ok()?;
        (p.pos == text.len()).then_some(number)
    }
    /// Original validated JSON representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Read an exactly integral JSON number without precision loss, including exponents.
    pub fn as_i64(&self) -> Option<i64> {
        let (negative, magnitude) = self.integer()?;
        if negative {
            if magnitude == 1u64 << 63 {
                Some(i64::MIN)
            } else {
                i64::try_from(magnitude).ok().map(|v| -v)
            }
        } else {
            i64::try_from(magnitude).ok()
        }
    }
    /// Read an exactly integral, nonnegative JSON number without precision loss.
    pub fn as_u64(&self) -> Option<u64> {
        let (negative, magnitude) = self.integer()?;
        (!negative || magnitude == 0).then_some(magnitude)
    }
    fn integer(&self) -> Option<(bool, u64)> {
        let negative = self.0.starts_with('-');
        let text = self.0.strip_prefix('-').unwrap_or(&self.0);
        let (mantissa, exponent) = if let Some(index) = text.find(['e', 'E']) {
            let exponent = &text[index + 1..];
            let exponent_negative = exponent.starts_with('-');
            let mut magnitude = 0i32;
            for byte in exponent.bytes().filter(|b| b.is_ascii_digit()) {
                magnitude = (magnitude.saturating_mul(10) + (byte - b'0') as i32).min(1_000_000);
            }
            (
                &text[..index],
                if exponent_negative {
                    -magnitude
                } else {
                    magnitude
                },
            )
        } else {
            (text, 0)
        };
        let fraction = mantissa
            .find('.')
            .map(|i| mantissa.len() - i - 1)
            .unwrap_or(0) as i32;
        let mut digits: Vec<u8> = mantissa
            .bytes()
            .filter(|b| *b != b'.')
            .skip_while(|b| *b == b'0')
            .collect();
        if digits.is_empty() {
            return Some((false, 0));
        }
        let mut scale = exponent - fraction;
        while scale < 0 && digits.last() == Some(&b'0') {
            digits.pop();
            scale += 1;
        }
        if scale < 0 || scale > 20 || digits.len() + scale as usize > 20 {
            return None;
        }
        let mut magnitude = 0u64;
        for digit in digits {
            magnitude = magnitude
                .checked_mul(10)?
                .checked_add((digit - b'0') as u64)?;
        }
        for _ in 0..scale {
            magnitude = magnitude.checked_mul(10)?;
        }
        Some((negative, magnitude))
    }
}

/// Owned JSON data. Objects retain member order and parsed objects have unique keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// JSON null.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// Exact JSON number.
    Number(Number),
    /// Unicode string.
    String(String),
    /// Ordered values.
    Array(Vec<Value>),
    /// Ordered name/value members.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Make a JSON string.
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }
    /// Make an unsigned number without converting to floating point.
    pub fn u64(value: u64) -> Self {
        Self::Number(Number(value.to_string()))
    }
    /// Make a signed number without converting to floating point.
    pub fn i64(value: i64) -> Self {
        Self::Number(Number(value.to_string()))
    }
    /// Make an object, with later duplicate names replacing earlier values.
    pub fn object<const N: usize>(fields: [(&str, Value); N]) -> Self {
        let mut result = Self::Object(Vec::with_capacity(N));
        for (key, value) in fields {
            result.insert(key, value);
        }
        result
    }
    /// Insert or replace an object member; false when this value is not an object.
    pub fn insert(&mut self, key: &str, value: Value) -> bool {
        let Self::Object(fields) = self else {
            return false;
        };
        if let Some((_, old)) = fields.iter_mut().find(|(name, _)| name == key) {
            *old = value;
        } else {
            fields.push((key.into(), value));
        }
        true
    }
    /// Look up an object member.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }
    /// Borrow an object member list.
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }
    /// Borrow an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(v) => Some(v),
            _ => None,
        }
    }
    /// Borrow a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(v) => Some(v),
            _ => None,
        }
    }
    /// Read a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }
    /// Read an unsigned integer token.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(v) => v.as_u64(),
            _ => None,
        }
    }
    /// Read a signed integer token.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(v) => v.as_i64(),
            _ => None,
        }
    }
}

/// Bounds applied before and during decoding or encoding.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum encoded byte count.
    pub bytes: usize,
    /// Maximum nested arrays/objects; the root value starts at depth zero.
    pub depth: usize,
    /// Maximum total values, including the root.
    pub values: usize,
    /// Maximum decoded bytes in any string, including member names.
    pub string_bytes: usize,
    /// Maximum members/elements in any container.
    pub members: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 1_048_576,
            depth: 32,
            values: 16_384,
            string_bytes: 262_144,
            members: 4096,
        }
    }
}

/// Parse or serialization failure with the byte offset where it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Zero-based byte offset (zero for serialization limits).
    pub offset: usize,
    /// Stable machine-readable failure category.
    pub kind: &'static str,
}

/// Parse a complete UTF-8 JSON value using default limits.
pub fn parse(bytes: &[u8]) -> Result<Value, Error> {
    parse_with_limits(bytes, Limits::default())
}

/// Parse a complete UTF-8 JSON value. Duplicate object keys and lone surrogates are errors.
pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Value, Error> {
    if bytes.len() > limits.bytes {
        return Err(Error {
            offset: limits.bytes,
            kind: "byte_limit",
        });
    }
    let text = core::str::from_utf8(bytes).map_err(|e| Error {
        offset: e.valid_up_to(),
        kind: "invalid_utf8",
    })?;
    let mut parser = Parser::new(text, limits);
    let value = parser.value(0)?;
    parser.space();
    if parser.pos != text.len() {
        return parser.fail("trailing_data");
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    values: usize,
    limits: Limits,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str, limits: Limits) -> Self {
        Self {
            text,
            pos: 0,
            values: 0,
            limits,
        }
    }
    fn fail<T>(&self, kind: &'static str) -> Result<T, Error> {
        Err(Error {
            offset: self.pos,
            kind,
        })
    }
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\r' | b'\n' | b'\t')) {
            self.pos += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        self.space();
        if depth > self.limits.depth {
            return self.fail("depth_limit");
        }
        self.values += 1;
        if self.values > self.limits.values {
            return self.fail("value_limit");
        }
        match self.peek() {
            Some(b'n') => {
                self.literal("null")?;
                Ok(Value::Null)
            }
            Some(b't') => {
                self.literal("true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.literal("false")?;
                Ok(Value::Bool(false))
            }
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'-' | b'0'..=b'9') => Ok(Value::Number(self.number()?)),
            Some(b'[') => {
                self.pos += 1;
                self.space();
                let mut values = Vec::new();
                if self.take(b']') {
                    return Ok(Value::Array(values));
                }
                loop {
                    if values.len() >= self.limits.members {
                        return self.fail("member_limit");
                    }
                    values.push(self.value(depth + 1)?);
                    self.space();
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return self.fail("expected_comma");
                    }
                }
                Ok(Value::Array(values))
            }
            Some(b'{') => {
                self.pos += 1;
                self.space();
                let mut fields: Vec<(String, Value)> = Vec::new();
                let mut names: Option<BTreeSet<String>> = None;
                if self.take(b'}') {
                    return Ok(Value::Object(fields));
                }
                loop {
                    if fields.len() >= self.limits.members {
                        return self.fail("member_limit");
                    }
                    self.space();
                    let key = self.string()?;
                    self.space();
                    let duplicate = match &mut names {
                        Some(set) => !set.insert(key.clone()),
                        None => fields.iter().any(|(name, _)| name == &key),
                    };
                    if duplicate {
                        return self.fail("duplicate_key");
                    }
                    if names.is_none() && fields.len() >= SMALL_OBJECT {
                        names = Some(fields.iter().map(|(name, _)| name.clone()).chain([key.clone()]).collect());
                    }
                    if !self.take(b':') {
                        return self.fail("expected_colon");
                    }
                    fields.push((key, self.value(depth + 1)?));
                    self.space();
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return self.fail("expected_comma");
                    }
                }
                Ok(Value::Object(fields))
            }
            _ => self.fail("expected_value"),
        }
    }
    fn literal(&mut self, text: &str) -> Result<(), Error> {
        if self.text[self.pos..].starts_with(text) {
            self.pos += text.len();
            Ok(())
        } else {
            self.fail("invalid_literal")
        }
    }
    fn number(&mut self) -> Result<Number, Error> {
        let start = self.pos;
        self.take(b'-');
        if !self.take(b'0') {
            if !matches!(self.peek(), Some(b'1'..=b'9')) {
                return self.fail("invalid_number");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if self.take(b'.') {
            let begin = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if begin == self.pos {
                return self.fail("invalid_number");
            }
        }
        if self.take(b'e') || self.take(b'E') {
            if !self.take(b'+') {
                self.take(b'-');
            }
            let begin = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if begin == self.pos {
                return self.fail("invalid_number");
            }
        }
        if self.pos - start > 256 {
            return self.fail("number_limit");
        }
        Ok(Number(self.text[start..self.pos].into()))
    }
    fn hex4(&mut self) -> Result<u32, Error> {
        let mut result = 0;
        for _ in 0..4 {
            let digit = match self.peek() {
                Some(b'0'..=b'9') => self.peek().unwrap() - b'0',
                Some(b'a'..=b'f') => self.peek().unwrap() - b'a' + 10,
                Some(b'A'..=b'F') => self.peek().unwrap() - b'A' + 10,
                _ => return self.fail("invalid_escape"),
            };
            self.pos += 1;
            result = (result << 4) | digit as u32;
        }
        Ok(result)
    }
    fn string(&mut self) -> Result<String, Error> {
        if !self.take(b'"') {
            return self.fail("expected_string");
        }
        let mut result = String::new();
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(result);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let escaped = match self.peek() {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.pos += 1;
                            let first = self.hex4()?;
                            let scalar = if (0xD800..=0xDBFF).contains(&first) {
                                if !self.take(b'\\') || !self.take(b'u') {
                                    return self.fail("unpaired_surrogate");
                                }
                                let low = self.hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return self.fail("unpaired_surrogate");
                                }
                                0x10000 + ((first - 0xD800) << 10) + (low - 0xDC00)
                            } else {
                                first
                            };
                            let Some(ch) = char::from_u32(scalar) else {
                                return self.fail("unpaired_surrogate");
                            };
                            result.push(ch);
                            if result.len() > self.limits.string_bytes {
                                return self.fail("string_limit");
                            }
                            continue;
                        }
                        _ => return self.fail("invalid_escape"),
                    };
                    self.pos += 1;
                    result.push(escaped);
                }
                Some(0..=31) => return self.fail("unescaped_control"),
                Some(_) => {
                    let Some(ch) = self.text[self.pos..].chars().next() else {
                        return self.fail("unterminated_string");
                    };
                    self.pos += ch.len_utf8();
                    result.push(ch);
                }
                None => return self.fail("unterminated_string"),
            }
            if result.len() > self.limits.string_bytes {
                return self.fail("string_limit");
            }
        }
    }
}

/// Encode JSON without literal newlines, enforcing the default bounds.
pub fn stringify(value: &Value) -> Result<String, Error> {
    stringify_with_limits(value, Limits::default())
}

/// Encode bounded JSON. Invalid duplicate keys in manually built objects are rejected.
pub fn stringify_with_limits(value: &Value, limits: Limits) -> Result<String, Error> {
    let mut writer = Writer {
        out: String::new(),
        limits,
        values: 0,
        indent: 0,
    };
    writer.value(value, 0)?;
    Ok(writer.out)
}

/// Encode bounded JSON for people to read: every array element and object member on its own
/// line, indented by `indent` spaces per level, "name": value; empty containers stay "[]" / "{}".
/// The output ends without a newline.
pub fn stringify_pretty(value: &Value, limits: Limits, indent: usize) -> Result<String, Error> {
    let mut writer = Writer {
        out: String::new(),
        limits,
        values: 0,
        indent: indent.clamp(1, 8),
    };
    writer.value(value, 0)?;
    Ok(writer.out)
}

struct Writer {
    out: String,
    limits: Limits,
    values: usize,
    /// spaces per level; 0 = compact
    indent: usize,
}

impl Writer {
    fn fail<T>(&self, kind: &'static str) -> Result<T, Error> {
        Err(Error {
            offset: self.out.len(),
            kind,
        })
    }
    fn push(&mut self, text: &str) -> Result<(), Error> {
        if text.len() > self.limits.bytes.saturating_sub(self.out.len()) {
            return self.fail("byte_limit");
        }
        self.out.push_str(text);
        Ok(())
    }
    /// a line break and the indentation of `depth` (pretty output only)
    fn newline(&mut self, depth: usize) -> Result<(), Error> {
        if self.indent == 0 {
            return Ok(());
        }
        let n = 1 + depth * self.indent;
        if n > self.limits.bytes.saturating_sub(self.out.len()) {
            return self.fail("byte_limit");
        }
        self.out.push('\n');
        for _ in 1..n {
            self.out.push(' ');
        }
        Ok(())
    }
    fn string(&mut self, text: &str) -> Result<(), Error> {
        if text.len() > self.limits.string_bytes {
            return self.fail("string_limit");
        }
        self.push("\"")?;
        for ch in text.chars() {
            match ch {
                '"' => self.push("\\\"")?,
                '\\' => self.push("\\\\")?,
                '\n' => self.push("\\n")?,
                '\r' => self.push("\\r")?,
                '\t' => self.push("\\t")?,
                ch if (ch as u32) < 32 => self.push(&format!("\\u{:04x}", ch as u32))?,
                ch => {
                    let mut bytes = [0; 4];
                    self.push(ch.encode_utf8(&mut bytes))?;
                }
            }
        }
        self.push("\"")
    }
    fn value(&mut self, value: &Value, depth: usize) -> Result<(), Error> {
        if depth > self.limits.depth {
            return self.fail("depth_limit");
        }
        self.values += 1;
        if self.values > self.limits.values {
            return self.fail("value_limit");
        }
        match value {
            Value::Null => self.push("null"),
            Value::Bool(v) => self.push(if *v { "true" } else { "false" }),
            Value::Number(v) => self.push(v.as_str()),
            Value::String(v) => self.string(v),
            Value::Array(values) => {
                if values.len() > self.limits.members {
                    return self.fail("member_limit");
                }
                self.push("[")?;
                for (i, item) in values.iter().enumerate() {
                    if i != 0 {
                        self.push(",")?;
                    }
                    self.newline(depth + 1)?;
                    self.value(item, depth + 1)?;
                }
                if !values.is_empty() {
                    self.newline(depth)?;
                }
                self.push("]")
            }
            Value::Object(fields) => {
                if fields.len() > self.limits.members {
                    return self.fail("member_limit");
                }
                self.push("{")?;
                let mut names: BTreeSet<&str> = BTreeSet::new();
                for (i, (name, item)) in fields.iter().enumerate() {
                    let duplicate = if fields.len() > SMALL_OBJECT {
                        !names.insert(name)
                    } else {
                        fields[..i].iter().any(|(key, _)| key == name)
                    };
                    if duplicate {
                        return self.fail("duplicate_key");
                    }
                    if i != 0 {
                        self.push(",")?;
                    }
                    self.newline(depth + 1)?;
                    self.string(name)?;
                    self.push(if self.indent > 0 { ": " } else { ":" })?;
                    self.value(item, depth + 1)?;
                }
                if !fields.is_empty() {
                    self.newline(depth)?;
                }
                self.push("}")
            }
        }
    }
}

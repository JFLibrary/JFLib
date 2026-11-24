// The objects of a PDF file as the reader gives them (ISO 32000-1, 7.3): null, booleans,
// integers, reals, strings (bytes; text strings are decoded by text_string), names, arrays,
// dictionaries, streams and references to indirect objects.  A stream holds its dictionary and
// where its data lies in the file; Document::decode gives the data with the filters applied.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::borrow::Borrow;

/// a reference to an indirect object: its number and generation
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ref {
    pub num: u32,
    pub generation: u16,
}

/// as PDF writes it: 12 0 R
impl core::fmt::Debug for Ref {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} {} R", self.num, self.generation)
    }
}

/// a name (/Type): its bytes with #xx escapes resolved
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Name(pub Vec<u8>);

/// as PDF writes it: /Type, with #xx for bytes outside the printable ASCII range and delimiters
impl core::fmt::Debug for Name {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("/")?;
        for &c in &self.0 {
            if c.is_ascii_graphic() && !super::lex::is_delim(c) && c != b'#' {
                write!(f, "{}", c as char)?;
            } else {
                write!(f, "#{c:02X}")?;
            }
        }
        Ok(())
    }
}

impl Name {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    /// the name as text (bytes that are not UTF-8 become U+FFFD)
    pub fn to_text(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

impl Borrow<[u8]> for Name {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, o: &str) -> bool {
        self.0 == o.as_bytes()
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, o: &&str) -> bool {
        self.0 == o.as_bytes()
    }
}

/// a dictionary; a key given twice keeps its last value
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Dict(pub(crate) BTreeMap<Name, Object>);

impl Dict {
    pub fn new() -> Dict {
        Dict(BTreeMap::new())
    }
    /// the value of a key (without the slash: "Type")
    pub fn get(&self, key: &str) -> Option<&Object> {
        self.0.get(key.as_bytes())
    }
    pub fn get_bytes(&self, key: &[u8]) -> Option<&Object> {
        self.0.get(key)
    }
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key.as_bytes())
    }
    pub fn insert(&mut self, key: &str, value: Object) {
        self.0.insert(Name(key.as_bytes().to_vec()), value);
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&Name, &Object)> {
        self.0.iter()
    }
    /// the name under a key (Type, Subtype, ...), if it is one
    pub fn name(&self, key: &str) -> Option<&[u8]> {
        self.get(key).and_then(Object::as_name)
    }
    /// a key whose value is the name `v`
    pub fn is(&self, key: &str, v: &str) -> bool {
        self.name(key) == Some(v.as_bytes())
    }
    pub fn int(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Object::as_int)
    }
    pub fn num(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Object::as_f64)
    }
}

/// a stream: its dictionary, and where its (still encoded) data lies in the file
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// the object it is (its key when the file is encrypted)
    pub(crate) owner: Option<Ref>,
}

impl Stream {
    /// the number of encoded bytes
    pub fn raw_len(&self) -> usize {
        self.end - self.start
    }
}

/// one object of a PDF file
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Object {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// a string's bytes (decrypted); text strings decode with text_string
    String(Vec<u8>),
    Name(Name),
    Array(Vec<Object>),
    Dict(Dict),
    /// boxed: keeps every object at 32 bytes
    Stream(alloc::boxed::Box<Stream>),
    Ref(Ref),
}

impl Object {
    pub fn name(s: &str) -> Object {
        Object::Name(Name(s.as_bytes().to_vec()))
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Object::Null)
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Object::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// an integer, or a real that is one (some writers put 1.0 where an integer belongs)
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Object::Int(i) => Some(i),
            Object::Real(r) if r.is_finite() && r == (r as i64) as f64 => Some(r as i64),
            _ => None,
        }
    }
    /// a number
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Object::Int(i) => Some(i as f64),
            Object::Real(r) if r.is_finite() => Some(r),
            _ => None,
        }
    }
    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(&n.0),
            _ => None,
        }
    }
    /// a string's bytes
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Object::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }
    /// a dictionary, or a stream's dictionary
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }
    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_ref(&self) -> Option<Ref> {
        match self {
            Object::Ref(r) => Some(*r),
            _ => None,
        }
    }
    /// a rectangle [x0 y0 x1 y1], normalized so that x0 <= x1 and y0 <= y1
    pub fn as_rect(&self) -> Option<[f64; 4]> {
        let a = self.as_array()?;
        if a.len() < 4 {
            return None;
        }
        let v = [a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?, a[3].as_f64()?];
        Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
    }
}

/// a text string (7.9.2.2) as text: UTF-16BE after FE FF, UTF-16LE after FF FE (seen in the
/// wild), UTF-8 after EF BB BF (PDF 2.0), else PDFDocEncoding
pub fn text_string(b: &[u8]) -> String {
    if b.len() >= 2 && (b[0] == 0xFE && b[1] == 0xFF || b[0] == 0xFF && b[1] == 0xFE) {
        let be = b[0] == 0xFE;
        let units = b[2..].chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) });
        let mut s: String = char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect();
        // a language escape (ESC code ESC) inside UTF-16 is dropped
        while let Some(i) = s.find('\u{1B}') {
            match s[i + 1..].find('\u{1B}') {
                Some(j) => s.replace_range(i..i + 1 + j + 1, ""),
                None => break,
            }
        }
        return s;
    }
    if b.len() >= 3 && b[..3] == [0xEF, 0xBB, 0xBF] {
        return String::from_utf8_lossy(&b[3..]).into_owned();
    }
    b.iter().filter_map(|&c| char::from_u32(super::tables::PDFDOC[c as usize] as u32).filter(|&ch| ch != '\0' || c == 0)).collect()
}

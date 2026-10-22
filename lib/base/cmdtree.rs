// cmdtree -- command trees for IOS-like command lines: unique prefix abbreviations ("sh ip int
// br"), "?" help with descriptions, Tab completion, modes with prompts, the "no" form, typed
// parameters and the position of an error for the IOS-like messages ("% Invalid input detected at
// '^' marker.", "% Incomplete command.", "% Ambiguous command:")
//
//   Tree::new()                          an empty tree of commands of type C (an app's enum)
//   mode(prompt, title, parent, no_help) a mode: its prompt after the host name ("(config)#"),
//                                        the title of its "?" list, the mode its commands fall
//                                        back to, the help of its "no" (None: no "no")
//   add(mode, path, help, cmd, flags)    a command path; shared prefixes merge, the last word
//                                        gets the help text, `cmd` makes the path complete
//   also(mode, path, target)             the words after `target` may follow `path` too
//                                        (options in any order)
//   parse(mode, line, &dyn Names)        -> Result<Parsed<C>, Error>
//   help(mode, line, &dyn Names)         -> Help: the answer to "?" typed after `line`
//   complete(mode, line, &dyn Names)     -> Completion (Tab)
//   Error::render / Help::render         the text an IOS-like console shows
//   find_mode(prompt)                    a mode by its prompt (modes that parts of an app add
//                                        need no fixed number)
//
// Paths are words separated by spaces.  A keyword stands for itself ("shutdown"; "access=mode"
// stores it under the key "mode").  A parameter is TYPE or TYPE=key (a later value of a key
// hides an earlier one, so "mode access=mode" reads as "access"):
//
//   <lo-hi>      a decimal number in the range          Val::Num
//   A.B.C.D      an IPv4 address                         Val::Ipv4
//   MASK         a contiguous dotted netmask              Val::Mask (its length)
//   A.B.C.D/nn   an address with a prefix length          Val::Prefix
//   X:X:X:X::X   an IPv6 address (every RFC 4291 form)    Val::Ipv6
//   X:X:X:X::X/<0-128>   an IPv6 address with a prefix length   Val::Prefix6
//   H.H.H        a MAC address (H.H.H or with colons)     Val::Mac
//   WORD         one word                                 Val::Word
//   LINE         the rest of the line, spaces included    Val::Line
//   NAME, NAME1 .. NAME9   one of the caller's names (lists 0 .. 9, interfaces): a type and a
//                number ("gi0/0/0", "Gi 0/0/0", "lo0"), the type abbreviated like a keyword,
//                also as two words                       Val::Name (the full name)
//
// Matching is IOS-like: keywords ignore case and may be shortened to any prefix that no other
// word of that place shares; a word that is a whole keyword wins; numbers, addresses and names
// must be valid; WORD and LINE take what no keyword takes.  The error position is the first
// character no candidate accepts (the "^" marker).  "no" is a keyword of every mode that has
// it; it leads to the commands added with F_NO, and a no-form may end early where add_no says.
// Help lists keywords and parameters in ASCII order (parameters and capitalised interface
// types first) with "<cr>" when the command may end there.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// the index of a node of a tree
pub type NodeId = u32;
/// the index of a mode
pub type ModeId = u16;

/// a command may be negated ("no shutdown"); set on its first word
pub const F_NO: u8 = 1;
/// not listed by help and completion (aliases, commands of the configuration file only)
pub const F_HIDDEN: u8 = 2;

/// a typed parameter
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    Num(i64, i64),
    Ipv4,
    Mask,
    Prefix,
    Mac,
    Word,
    Line,
    Name(u8),
    Ipv6,
    Prefix6,
}

impl Param {
    /// what help shows for it ("<1-4094>", "A.B.C.D", "WORD")
    pub fn label(&self) -> String {
        match self {
            Param::Num(a, b) => format!("<{}-{}>", a, b),
            Param::Ipv4 | Param::Mask => String::from("A.B.C.D"),
            Param::Prefix => String::from("A.B.C.D/nn"),
            Param::Mac => String::from("H.H.H"),
            Param::Word => String::from("WORD"),
            Param::Line => String::from("LINE"),
            Param::Name(_) => String::from("NAME"),
            Param::Ipv6 => String::from("X:X:X:X::X"),
            Param::Prefix6 => String::from("X:X:X:X::X/<0-128>"),
        }
    }
    fn of(s: &str) -> Option<Param> {
        Some(match s {
            "A.B.C.D" => Param::Ipv4,
            "MASK" => Param::Mask,
            "A.B.C.D/nn" => Param::Prefix,
            "H.H.H" => Param::Mac,
            "WORD" => Param::Word,
            "LINE" => Param::Line,
            "NAME" => Param::Name(0),
            "X:X:X:X::X" => Param::Ipv6,
            "X:X:X:X::X/<0-128>" => Param::Prefix6,
            _ => {
                if let Some(d) = s.strip_prefix("NAME")
                    && d.len() == 1
                    && let Some(n) = d.chars().next().and_then(|c| c.to_digit(10))
                {
                    return Some(Param::Name(n as u8));
                }
                let r = s.strip_prefix('<')?.strip_suffix('>')?;
                // "<-5-5>" is allowed: the dash after the first digit separates
                let cut = r.char_indices().skip(1).find(|&(_, c)| c == '-').map(|(i, _)| i)?;
                Param::Num(r[..cut].parse().ok()?, r[cut + 1..].parse().ok()?)
            }
        })
    }
}

/// a value of a parsed command
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Val {
    /// a keyword, written out in full
    Key(String),
    Num(i64),
    Ipv4([u8; 4]),
    Mask(u8),
    Prefix([u8; 4], u8),
    Mac([u8; 6]),
    Word(String),
    Line(String),
    Name(String),
    Ipv6([u8; 16]),
    Prefix6([u8; 16], u8),
}

/// the names a NAME parameter takes (interfaces of a device); "*" in a name stands for a number
/// ("Loopback*", "GigabitEthernet0/0/0.*")
pub trait Names {
    fn names(&self, list: u8) -> Vec<String>;
    /// the help text of a type of names ("GigabitEthernet" -> "GigabitEthernet IEEE 802.3z")
    fn type_help(&self, _list: u8, ty: &str) -> String {
        format!("{} interface", ty)
    }
}

/// no names at all
pub struct NoNames;

impl Names for NoNames {
    fn names(&self, _list: u8) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Clone, Debug)]
struct Node<C> {
    /// the keyword ("" for a parameter)
    word: String,
    param: Option<Param>,
    key: String,
    help: String,
    next: Vec<NodeId>,
    /// the words after this node may also follow it
    also: Option<NodeId>,
    cmd: Option<C>,
    /// the command when the no-form ends here
    no_cmd: Option<C>,
    flags: u8,
}

/// a mode of the command line
#[derive(Clone, Debug)]
pub struct Mode {
    /// what follows the host name in the prompt ("#", "(config-if)#")
    pub prompt: String,
    /// the first line of the "?" list ("Commands at the user level:")
    pub title: String,
    /// where a command that is not one of this mode goes (configuration submodes: the global
    /// configuration); the caller decides whether it tries there
    pub parent: Option<ModeId>,
    root: NodeId,
    /// the "no" keyword of this mode
    no: Option<NodeId>,
}

/// a parsed command line
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed<C> {
    pub cmd: C,
    /// the line started with "no"
    pub no: bool,
    /// the keywords and parameters in order, by key
    pub vals: Vec<(String, Val)>,
}

impl<C> Parsed<C> {
    /// the value of a key; a later one hides an earlier one ("name WORD=name" gives the word,
    /// "ping X repeat 5 repeat 7" the 7)
    pub fn get(&self, key: &str) -> Option<&Val> {
        self.vals.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    /// the keyword or parameter is there
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    pub fn num(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            Val::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn ip(&self, key: &str) -> Option<[u8; 4]> {
        match self.get(key)? {
            Val::Ipv4(a) => Some(*a),
            _ => None,
        }
    }
    /// a netmask's length
    pub fn mask(&self, key: &str) -> Option<u8> {
        match self.get(key)? {
            Val::Mask(m) => Some(*m),
            _ => None,
        }
    }
    pub fn prefix(&self, key: &str) -> Option<([u8; 4], u8)> {
        match self.get(key)? {
            Val::Prefix(a, l) => Some((*a, *l)),
            _ => None,
        }
    }
    pub fn mac(&self, key: &str) -> Option<[u8; 6]> {
        match self.get(key)? {
            Val::Mac(m) => Some(*m),
            _ => None,
        }
    }
    pub fn ip6(&self, key: &str) -> Option<[u8; 16]> {
        match self.get(key)? {
            Val::Ipv6(a) => Some(*a),
            _ => None,
        }
    }
    /// an IPv6 address with its prefix length
    pub fn prefix6(&self, key: &str) -> Option<([u8; 16], u8)> {
        match self.get(key)? {
            Val::Prefix6(a, l) => Some((*a, *l)),
            _ => None,
        }
    }
    /// a word, line, name or keyword as text
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Val::Key(s) | Val::Word(s) | Val::Line(s) | Val::Name(s) => Some(s),
            _ => None,
        }
    }
}

/// why a line does not parse
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// nothing was typed
    Empty,
    /// no command goes on at this byte of the line (the "^" marker)
    Invalid(usize),
    /// the line ends before the command does
    Incomplete,
    /// the word at this byte fits several commands
    Ambiguous(usize),
}

impl Error {
    /// the console text; `col` is where the line starts on the screen (the prompt's
    /// length), so the marker stands under the bad character
    pub fn render(&self, col: usize, line: &str) -> String {
        match self {
            Error::Empty => String::new(),
            Error::Invalid(at) => {
                let chars = line.char_indices().take_while(|&(i, _)| i < *at).count();
                let mut s: String = core::iter::repeat_n(' ', col + chars).collect();
                s.push_str("^\n% Invalid input detected at '^' marker.\n\n");
                s
            }
            Error::Incomplete => String::from("% Incomplete command.\n\n"),
            Error::Ambiguous(_) => format!("% Ambiguous command:  \"{}\"\n", line.trim()),
        }
    }
}

/// the answer to "?"
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Help {
    /// what may follow: (label, description) in display order; `cr` when the command may end
    /// here; the mode's title for an empty line
    List { title: Option<String>, items: Vec<(String, String)>, cr: bool },
    /// the keywords a partly typed word may become
    Words(Vec<String>),
    /// nothing starts like the partly typed word
    Unrecognized,
    Error(Error),
}

impl Help {
    /// the console text (`col`: the prompt's length, for an error marker)
    pub fn render(&self, col: usize, line: &str) -> String {
        match self {
            Help::List { title, items, cr } => {
                let mut s = String::new();
                if let Some(t) = title {
                    s.push_str(t);
                    s.push('\n');
                }
                let w = items.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0) + 2;
                for (n, d) in items {
                    if d.is_empty() {
                        s.push_str(&format!("  {}\n", n));
                    } else {
                        s.push_str(&format!("  {:<w$}{}\n", n, d, w = w));
                    }
                }
                if *cr {
                    s.push_str("  <cr>\n");
                }
                s.push('\n');
                s
            }
            Help::Words(ws) => {
                let mut s = String::new();
                let mut col = 0;
                for w in ws {
                    if col > 0 && col + w.len() + 2 > 79 {
                        s.push('\n');
                        col = 0;
                    }
                    s.push_str(w);
                    s.push_str("  ");
                    col += w.len() + 2;
                }
                s.push_str("\n\n");
                s
            }
            Help::Unrecognized => String::from("% Unrecognized command\n"),
            Help::Error(e) => e.render(col, line),
        }
    }
}

/// what Tab does
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    /// the line with the last word completed (None: nothing to complete, the console rings the bell)
    pub line: Option<String>,
    /// what the last word may become
    pub choices: Vec<String>,
}

/// a word of a line and where it starts
#[derive(Clone, Copy, Debug)]
struct Tok<'a> {
    s: &'a str,
    at: usize,
}

fn tokens(line: &str) -> Vec<Tok<'_>> {
    let mut v = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i > start {
            v.push(Tok { s: &line[start..i], at: start });
        }
    }
    v
}

/// a common prefix in characters, ignoring ASCII case
fn common(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x.eq_ignore_ascii_case(y)).count()
}

fn starts_ci(word: &str, part: &str) -> bool {
    word.len() >= part.len() && word.as_bytes()[..part.len()].eq_ignore_ascii_case(part.as_bytes())
}

/// "10.1.2.3" -> [10, 1, 2, 3] (four decimal parts of at most 255, no signs)
pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0;
    for part in s.split('.') {
        if n == 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let v: u32 = part.parse().ok()?;
        if v > 255 {
            return None;
        }
        out[n] = v as u8;
        n += 1;
    }
    if n == 4 { Some(out) } else { None }
}

/// the length of a contiguous dotted netmask ("255.255.255.0" -> 24)
pub fn parse_mask(s: &str) -> Option<u8> {
    let a = parse_ipv4(s)?;
    let v = u32::from_be_bytes(a);
    let len = v.leading_ones();
    if v.checked_shl(len).unwrap_or(0) == 0 { Some(len as u8) } else { None }
}

/// "aabb.ccdd.eeff", "aa:bb:cc:dd:ee:ff" or "aa-bb-cc-dd-ee-ff"
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let hex: String = match s.len() {
        14 if s.as_bytes()[4] == b'.' && s.as_bytes()[9] == b'.' => s.split('.').collect(),
        17 => {
            let sep = s.as_bytes()[2];
            if (sep != b':' && sep != b'-') || s.bytes().enumerate().any(|(i, c)| (i % 3 == 2) != (c == sep)) {
                return None;
            }
            s.split(sep as char).collect()
        }
        _ => return None,
    };
    if hex.len() != 12 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut m = [0u8; 6];
    for (i, o) in m.iter_mut().enumerate() {
        *o = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(m)
}

/// an IPv6 address in any RFC 4291 text form: eight groups of up to four hex digits, "::" once
/// for a run of zero groups, the last 32 bits as dotted decimal ("::ffff:10.0.0.1"); hex digits
/// in either case
pub fn parse_ipv6(s: &str) -> Option<[u8; 16]> {
    if s.is_empty() || s.len() > 45 {
        return None;
    }
    // the groups before and after "::" (one side only without it)
    let (head, tail, gap) = match s.find("::") {
        Some(p) if s[p + 2..].contains("::") => return None,
        Some(p) => (&s[..p], &s[p + 2..], true),
        None => (s, "", false),
    };
    let groups = |part: &str, last: bool| -> Option<Vec<u16>> {
        let mut v: Vec<u16> = Vec::new();
        if part.is_empty() {
            return Some(v);
        }
        let pieces: Vec<&str> = part.split(':').collect();
        for (i, p) in pieces.iter().enumerate() {
            if last && i + 1 == pieces.len() && p.contains('.') {
                let a = parse_ipv4(p)?;
                v.push(u16::from_be_bytes([a[0], a[1]]));
                v.push(u16::from_be_bytes([a[2], a[3]]));
            } else if p.is_empty() || p.len() > 4 || !p.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            } else {
                v.push(u16::from_str_radix(p, 16).ok()?);
            }
        }
        Some(v)
    };
    let front = groups(head, !gap)?;
    let back = if gap { groups(tail, true)? } else { Vec::new() };
    let n = front.len() + back.len();
    if (gap && n > 7) || (!gap && n != 8) {
        return None;
    }
    let mut seg = [0u16; 8];
    seg[..front.len()].copy_from_slice(&front);
    seg[8 - back.len()..].copy_from_slice(&back);
    let mut out = [0u8; 16];
    for (i, g) in seg.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&g.to_be_bytes());
    }
    Some(out)
}

/// the value of a typed parameter for a word (WORD, LINE and NAME are decided elsewhere)
fn typed_value(p: Param, s: &str) -> Option<Val> {
    match p {
        Param::Num(lo, hi) => {
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
                return None;
            }
            let v: i64 = s.parse().ok()?;
            if (lo..=hi).contains(&v) { Some(Val::Num(v)) } else { None }
        }
        Param::Ipv4 => parse_ipv4(s).map(Val::Ipv4),
        Param::Mask => parse_mask(s).map(Val::Mask),
        Param::Prefix => {
            let (a, l) = s.split_once('/')?;
            let a = parse_ipv4(a)?;
            if l.is_empty() || l.len() > 2 || !l.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let l: u8 = l.parse().ok()?;
            if l <= 32 { Some(Val::Prefix(a, l)) } else { None }
        }
        Param::Mac => parse_mac(s).map(Val::Mac),
        Param::Ipv6 => parse_ipv6(s).map(Val::Ipv6),
        Param::Prefix6 => {
            let (a, l) = s.split_once('/')?;
            let a = parse_ipv6(a)?;
            if l.is_empty() || l.len() > 3 || !l.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let l: u8 = l.parse().ok()?;
            if l <= 128 { Some(Val::Prefix6(a, l)) } else { None }
        }
        Param::Word | Param::Line | Param::Name(_) => None,
    }
}

/// the type part of a name and its number: the number is the tail of digits, "/", "." and ":"
/// (or "*") that starts with a digit, so types may hold digits themselves
/// ("GigabitEthernet0/0/0.10" -> "GigabitEthernet", "Dot11Radio0" -> "Dot11Radio",
/// "Loopback*" -> "Loopback")
fn split_name(s: &str) -> (&str, &str) {
    let b = s.as_bytes();
    let mut at = b.len();
    while at > 0 && (b[at - 1].is_ascii_digit() || b"/.:*".contains(&b[at - 1])) {
        at -= 1;
    }
    while at < b.len() && !(b[at].is_ascii_digit() || b[at] == b'*') {
        at += 1;
    }
    (&s[..at], &s[at..])
}

/// a name against a pattern where "*" is one or more digits (ASCII case ignored)
fn name_fits(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.as_bytes(), name.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < p.len() {
        if p[i] == b'*' {
            let start = j;
            while j < n.len() && n[j].is_ascii_digit() {
                j += 1;
            }
            if j == start {
                return false;
            }
            i += 1;
        } else {
            if j >= n.len() || !p[i].eq_ignore_ascii_case(&n[j]) {
                return false;
            }
            i += 1;
            j += 1;
        }
    }
    j == n.len()
}

/// how a NAME parameter takes words
enum NameFit {
    /// the full name, words used
    Name(String, usize),
    /// the type is known, the number is missing (end of the line)
    NeedNumber,
    /// no type of the list starts like this (error at the word)
    NoType,
    /// several types start like this
    AmbiguousType,
    /// the type fits, the number does not (error at the number)
    BadNumber(usize),
}

/// the distinct types of a list of names, in order
fn name_types(names: &[String]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for n in names {
        let t = split_name(n).0;
        if !t.is_empty() && !v.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            v.push(String::from(t));
        }
    }
    v
}

/// the type a partial or whole type word stands for
fn pick_type<'a>(types: &'a [String], t: &str) -> Result<&'a str, bool> {
    if let Some(x) = types.iter().find(|x| x.eq_ignore_ascii_case(t)) {
        return Ok(x);
    }
    let m: Vec<&String> = types.iter().filter(|x| starts_ci(x, t)).collect();
    match m.len() {
        1 => Ok(m[0]),
        0 => Err(false),
        _ => Err(true),
    }
}

fn fit_name(names: &[String], toks: &[Tok], i: usize) -> NameFit {
    let t = toks[i];
    let (ty, num) = split_name(t.s);
    if ty.is_empty() {
        return NameFit::NoType;
    }
    let types = name_types(names);
    let full_ty = match pick_type(&types, ty) {
        Ok(x) => x,
        Err(true) => return NameFit::AmbiguousType,
        Err(false) => return NameFit::NoType,
    };
    let (num, used, num_at) = if num.is_empty() {
        match toks.get(i + 1) {
            Some(n) if n.s.starts_with(|c: char| c.is_ascii_digit()) => (n.s, 2, n.at),
            Some(n) => return NameFit::BadNumber(n.at),
            None => return NameFit::NeedNumber,
        }
    } else {
        (num, 1, t.at + ty.len())
    };
    let want = format!("{}{}", full_ty, num);
    if let Some(n) = names.iter().find(|n| !n.contains('*') && n.eq_ignore_ascii_case(&want)) {
        return NameFit::Name(n.clone(), used);
    }
    if names.iter().any(|n| n.contains('*') && name_fits(n, &want)) {
        return NameFit::Name(want, used);
    }
    NameFit::BadNumber(num_at)
}

/// one step of a walk: the node a word leads to
enum Pick {
    One { node: NodeId, val: Val, used: usize },
    None(usize),
    Ambiguous,
    Incomplete,
}

/// a tree of commands in modes
#[derive(Clone, Debug)]
pub struct Tree<C> {
    nodes: Vec<Node<C>>,
    modes: Vec<Mode>,
}

impl<C: Copy> Default for Tree<C> {
    fn default() -> Tree<C> {
        Tree::new()
    }
}

impl<C: Copy> Tree<C> {
    pub fn new() -> Tree<C> {
        Tree { nodes: Vec::new(), modes: Vec::new() }
    }

    fn new_node(&mut self, word: &str, param: Option<Param>, key: &str) -> NodeId {
        self.nodes.push(Node { word: String::from(word), param, key: String::from(key), help: String::new(), next: Vec::new(), also: None, cmd: None, no_cmd: None, flags: 0 });
        (self.nodes.len() - 1) as NodeId
    }

    /// a new mode; `no`: a "no" keyword negates its commands (`no_help` is its help text)
    pub fn mode(&mut self, prompt: &str, title: &str, parent: Option<ModeId>, no_help: Option<&str>) -> ModeId {
        let root = self.new_node("", None, "");
        let no = no_help.map(|h| {
            let n = self.new_node("no", None, "no");
            self.nodes[n as usize].help = String::from(h);
            n
        });
        self.modes.push(Mode { prompt: String::from(prompt), title: String::from(title), parent, root, no });
        (self.modes.len() - 1) as ModeId
    }
    pub fn mode_info(&self, m: ModeId) -> &Mode {
        &self.modes[m as usize]
    }
    /// the mode with this prompt suffix ("(config-ext-nacl)#"; the first one made)
    pub fn find_mode(&self, prompt: &str) -> Option<ModeId> {
        self.modes.iter().position(|m| m.prompt == prompt).map(|i| i as ModeId)
    }
    /// the prompt of a mode for a host name ("R1(config)#")
    pub fn prompt(&self, m: ModeId, host: &str) -> String {
        format!("{}{}", host, self.modes[m as usize].prompt)
    }

    /// the node a path word stands for under `at`, made when missing
    fn step(&mut self, at: NodeId, w: &str) -> NodeId {
        let (body, key) = match w.split_once('=') {
            Some((b, k)) => (b, k),
            None => (w, ""),
        };
        let (word, param) = match Param::of(body) {
            Some(p) => ("", Some(p)),
            None => (body, None),
        };
        let key = if key.is_empty() { body.to_ascii_lowercase() } else { String::from(key) };
        let found = self.nodes[at as usize].next.iter().copied().find(|&n| {
            let x = &self.nodes[n as usize];
            x.param == param && x.word.eq_ignore_ascii_case(word) && x.key == key
        });
        match found {
            Some(n) => n,
            None => {
                let n = self.new_node(word, param, &key);
                self.nodes[at as usize].next.push(n);
                n
            }
        }
    }

    /// the node at the end of a path (made when missing)
    fn path(&mut self, mode: ModeId, path: &str) -> NodeId {
        let mut at = self.modes[mode as usize].root;
        for w in path.split_whitespace() {
            at = self.step(at, w);
        }
        at
    }

    /// adds a command path: `help` describes its last word; `cmd` makes the command complete
    /// there; `flags` (F_NO, F_HIDDEN) go to the last word
    pub fn add(&mut self, mode: ModeId, path: &str, help: &str, cmd: Option<C>, flags: u8) -> NodeId {
        let n = self.path(mode, path);
        let node = &mut self.nodes[n as usize];
        if !help.is_empty() {
            node.help = String::from(help);
        }
        if cmd.is_some() {
            node.cmd = cmd;
        }
        node.flags |= flags;
        n
    }
    /// the no-form of a command may end at `path` ("no ip address" without an address)
    pub fn add_no(&mut self, mode: ModeId, path: &str, cmd: C) {
        let n = self.path(mode, path);
        self.nodes[n as usize].no_cmd = Some(cmd);
    }
    /// after `path`, the words that follow `target` may follow as well (a list of options in
    /// any order: "ping A repeat 5 size 100")
    pub fn also(&mut self, mode: ModeId, path: &str, target: &str) {
        let t = self.path(mode, target);
        let n = self.path(mode, path);
        if n != t {
            self.nodes[n as usize].also = Some(t);
        }
    }

    /// the nodes that may follow `at` (`no`: the no keyword of `mode` stands there)
    fn children(&self, mode: ModeId, at: NodeId) -> Vec<NodeId> {
        let m = &self.modes[mode as usize];
        if Some(at) == m.no {
            return self.nodes[m.root as usize].next.iter().copied().filter(|&n| self.nodes[n as usize].flags & F_NO != 0).collect();
        }
        let node = &self.nodes[at as usize];
        let mut v = node.next.clone();
        if at == m.root
            && let Some(no) = m.no
        {
            v.push(no);
        }
        if let Some(a) = node.also {
            for &n in &self.nodes[a as usize].next {
                if !v.contains(&n) {
                    v.push(n);
                }
            }
        }
        v
    }

    /// the node word `i` leads to from `cands`
    fn pick(&self, cands: &[NodeId], toks: &[Tok], i: usize, line: &str, names: &dyn Names) -> Pick {
        let t = toks[i];
        let mut exact = None;
        let mut fits: Vec<(NodeId, Val, usize)> = Vec::new();
        let mut loose: Vec<(NodeId, Val)> = Vec::new();
        let mut far = t.at;
        let mut need_number = false;
        let mut ambiguous_type = false;
        for &c in cands {
            let n = &self.nodes[c as usize];
            match n.param {
                None => {
                    if n.word.eq_ignore_ascii_case(t.s) {
                        exact = Some(c);
                    } else if starts_ci(&n.word, t.s) {
                        fits.push((c, Val::Key(n.word.clone()), 1));
                    } else {
                        far = far.max(t.at + common(&n.word, t.s));
                    }
                }
                Some(p) => {
                    let v = match p {
                        Param::Num(..) | Param::Ipv4 | Param::Mask | Param::Prefix | Param::Mac | Param::Ipv6 | Param::Prefix6 => typed_value(p, t.s),
                        Param::Word => {
                            loose.push((c, Val::Word(String::from(t.s))));
                            None
                        }
                        Param::Line => {
                            loose.push((c, Val::Line(String::from(line[t.at..].trim_end()))));
                            None
                        }
                        Param::Name(list) => match fit_name(&names.names(list), toks, i) {
                            NameFit::Name(s, used) => {
                                fits.push((c, Val::Name(s), used));
                                None
                            }
                            NameFit::NeedNumber => {
                                need_number = true;
                                None
                            }
                            NameFit::NoType => None,
                            NameFit::AmbiguousType => {
                                ambiguous_type = true;
                                None
                            }
                            NameFit::BadNumber(at) => {
                                far = far.max(at);
                                None
                            }
                        },
                    };
                    if let Some(v) = v {
                        fits.push((c, v, 1));
                    }
                }
            }
        }
        if let Some(e) = exact {
            return Pick::One { node: e, val: Val::Key(self.nodes[e as usize].word.clone()), used: 1 };
        }
        if fits.len() == 1 && !ambiguous_type {
            let (node, val, used) = fits.remove(0);
            return Pick::One { node, val, used };
        }
        if fits.len() > 1 || ambiguous_type {
            return Pick::Ambiguous;
        }
        if need_number {
            return Pick::Incomplete;
        }
        if let Some((node, val)) = loose.into_iter().next() {
            return Pick::One { node, val, used: 1 };
        }
        Pick::None(far)
    }

    /// walks the words of a line: (node, no-form, values, words used) or the error
    fn walk(&self, mode: ModeId, line: &str, toks: &[Tok], names: &dyn Names) -> Result<(NodeId, bool, Vec<(String, Val)>), Error> {
        let m = &self.modes[mode as usize];
        let mut at = m.root;
        let mut no = false;
        let mut vals = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            let cands = self.children(mode, at);
            match self.pick(&cands, toks, i, line, names) {
                Pick::One { node, val, used } => {
                    if Some(node) == m.no {
                        no = true;
                    } else {
                        vals.push((self.nodes[node as usize].key.clone(), val));
                    }
                    at = node;
                    if self.nodes[node as usize].param == Some(Param::Line) {
                        return Ok((at, no, vals));
                    }
                    i += used;
                }
                Pick::None(far) => return Err(Error::Invalid(far)),
                Pick::Ambiguous => return Err(Error::Ambiguous(toks[i].at)),
                Pick::Incomplete => return Err(Error::Incomplete),
            }
        }
        Ok((at, no, vals))
    }

    /// the command a line of `mode` is
    pub fn parse(&self, mode: ModeId, line: &str, names: &dyn Names) -> Result<Parsed<C>, Error> {
        let toks = tokens(line);
        if toks.is_empty() {
            return Err(Error::Empty);
        }
        let (at, no, vals) = self.walk(mode, line, &toks, names)?;
        let n = &self.nodes[at as usize];
        let cmd = if no { n.no_cmd.or(n.cmd) } else { n.cmd };
        match cmd {
            Some(cmd) => Ok(Parsed { cmd, no, vals }),
            None => Err(Error::Incomplete),
        }
    }

    /// the labels and help of the nodes that may follow `at`
    fn listing(&self, mode: ModeId, at: NodeId, names: &dyn Names) -> Vec<(String, String)> {
        let mut items: Vec<(String, String)> = Vec::new();
        for c in self.children(mode, at) {
            let n = &self.nodes[c as usize];
            if n.flags & F_HIDDEN != 0 {
                continue;
            }
            match n.param {
                None => items.push((n.word.clone(), n.help.clone())),
                Some(Param::Name(list)) => {
                    for ty in name_types(&names.names(list)) {
                        let h = names.type_help(list, &ty);
                        if !items.iter().any(|(l, _)| *l == ty) {
                            items.push((ty, h));
                        }
                    }
                }
                Some(p) => items.push((p.label(), n.help.clone())),
            }
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items.dedup_by(|a, b| a.0 == b.0);
        items
    }

    fn complete_here(&self, at: NodeId, no: bool) -> bool {
        let n = &self.nodes[at as usize];
        n.cmd.is_some() || (no && n.no_cmd.is_some())
    }

    /// the answer to "?" typed right after `line` (a partly typed last word lists what it may
    /// become; after a space, what may follow)
    pub fn help(&self, mode: ModeId, line: &str, names: &dyn Names) -> Help {
        let toks = tokens(line);
        let partial = !line.is_empty() && !line.ends_with(|c: char| c.is_ascii_whitespace());
        let done = if partial { &toks[..toks.len() - 1] } else { &toks[..] };
        let (at, no, _) = match self.walk(mode, line, done, names) {
            Ok(x) => x,
            Err(e) => return Help::Error(e),
        };
        if self.nodes[at as usize].param == Some(Param::Line) && !done.is_empty() {
            return Help::List { title: None, items: Vec::new(), cr: true };
        }
        if partial {
            let part = toks[toks.len() - 1].s;
            let mut ws: Vec<String> = Vec::new();
            for c in self.children(mode, at) {
                let n = &self.nodes[c as usize];
                if n.flags & F_HIDDEN != 0 {
                    continue;
                }
                match n.param {
                    None if starts_ci(&n.word, part) => ws.push(n.word.clone()),
                    None => {}
                    Some(Param::Name(list)) => {
                        let (ty, num) = split_name(part);
                        if num.is_empty() {
                            for t in name_types(&names.names(list)) {
                                if starts_ci(&t, ty) {
                                    ws.push(t);
                                }
                            }
                        }
                    }
                    Some(p) => {
                        if typed_value(p, part).is_some() {
                            ws.push(p.label());
                        }
                    }
                }
            }
            ws.sort();
            ws.dedup();
            return if ws.is_empty() { Help::Unrecognized } else { Help::Words(ws) };
        }
        let title = if toks.is_empty() { Some(self.modes[mode as usize].title.clone()) } else { None };
        Help::List { title, items: self.listing(mode, at, names), cr: self.complete_here(at, no) }
    }

    /// Tab: the last, partly typed word written out when only one keyword or name type fits
    pub fn complete(&self, mode: ModeId, line: &str, names: &dyn Names) -> Completion {
        let toks = tokens(line);
        if line.is_empty() || line.ends_with(|c: char| c.is_ascii_whitespace()) || toks.is_empty() {
            return Completion::default();
        }
        let last = toks[toks.len() - 1];
        let Ok((at, _, _)) = self.walk(mode, line, &toks[..toks.len() - 1], names) else {
            return Completion::default();
        };
        let mut words: Vec<String> = Vec::new();
        let mut types: Vec<String> = Vec::new();
        for c in self.children(mode, at) {
            let n = &self.nodes[c as usize];
            if n.flags & F_HIDDEN != 0 && !n.word.eq_ignore_ascii_case(last.s) {
                continue;
            }
            match n.param {
                None if starts_ci(&n.word, last.s) => words.push(n.word.clone()),
                Some(Param::Name(list)) => {
                    let (ty, num) = split_name(last.s);
                    if num.is_empty() && !ty.is_empty() {
                        for t in name_types(&names.names(list)) {
                            if starts_ci(&t, ty) && !types.contains(&t) {
                                types.push(t);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let exact = words.iter().find(|w| w.eq_ignore_ascii_case(last.s)).cloned();
        let mut choices: Vec<String> = words.iter().chain(types.iter()).cloned().collect();
        choices.sort();
        let head = &line[..last.at];
        let out = match (exact, words.len() + types.len()) {
            (Some(w), _) => Some(format!("{}{} ", head, w)),
            (None, 1) if words.len() == 1 => Some(format!("{}{} ", head, words[0])),
            (None, 1) => Some(format!("{}{}", head, types[0])),
            _ => None,
        };
        Completion { line: out, choices }
    }

    /// does the first word of a line name a command of `mode` (a keyword or the "no" of the
    /// mode, abbreviations included)?  An IOS-like EXEC answers unknown first words with a host
    /// name lookup instead of the marker.
    pub fn knows_first(&self, mode: ModeId, line: &str) -> bool {
        let Some(t) = tokens(line).first().copied() else {
            return false;
        };
        let m = &self.modes[mode as usize];
        self.children(mode, m.root).iter().any(|&c| {
            let n = &self.nodes[c as usize];
            n.param.is_some() || starts_ci(&n.word, t.s)
        })
    }
}

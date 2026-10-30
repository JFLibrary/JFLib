// imf -- Internet messages (RFC 5322) with the part of MIME (RFC 2045, 2046, 2047) that plain
// text mail needs: header fields with folding, addresses with display names, the Date form,
// Message-ID, a text/plain body in UTF-8 as 7bit, 8bit or quoted-printable, multipart bodies
// (a bounce's multipart/report of RFC 3464 and 6522) and base64
//
//   Field { name, value }        one header field; the value unfolded (fold() writes it again)
//   Message { fields, body }     parse(bytes) (CRLF or LF; the header ends at the first empty
//                                line), get(name) / get_all / set / add / prepend / remove,
//                                to_bytes() (CRLF, every field folded at 78 columns), text()
//                                (the body as text: transfer encoding, charset and multipart
//                                undone), subject() / from() / to() / cc() / date()
//   Mailbox { name, addr }       parse_mailbox("Anna Muster <anna@a.lab>"), parse_list("a@x, B
//                                <b@y>"), to_header() (a display name quoted or as an encoded
//                                word); addr_ok(addr), split_addr(addr) -> (local, domain),
//                                split_list(text): the parts between the commas, as typed
//   params(value)                "text/plain; charset=UTF-8" -> ("text/plain", [("charset",
//                                "UTF-8")]); param(value, name)
//   date(unix) / parse_date(text) the Date form of 3.3 ("Thu, 01 Jan 2026 00:00:05 +0000"),
//                                read back to seconds since 1970 (UTC, any zone it names)
//   message_id(left, domain)     "<left@domain>"
//   encode_word / decode_words   RFC 2047 encoded words (UTF-8, Q and B; ISO-8859-1 read too)
//   qp_encode / qp_decode        quoted-printable of RFC 2045 6.7 (soft line breaks at 76)
//   b64_encode / b64_lines / b64_decode   base64 of RFC 4648 (MIME lines of 76, padding)
//   Draft { from, to, cc, subject, date, id, in_reply_to, references, text }
//                                to_bytes(eight_bit): the message a mail program sends
//                                (MIME-Version 1.0, text/plain in UTF-8; 7bit for ASCII, 8bit
//                                when the server takes 8BITMIME, else quoted-printable)
//   fold(name, value)            "Name: value\r\n" with continuation lines that start with a
//                                space (2.2.3)
//
// Lines of a header field are at most 78 characters where there is whitespace to break at
// (2.1.1); a word longer than that stays whole.  Display names and subjects with characters
// outside ASCII travel as encoded words, so a header stays 7-bit.  Addresses are not
// normalised: the local part keeps its case, the domain is compared without it by the callers.
// This is no full MIME implementation: nested multiparts are searched for their first text
// part, other parts (attachments) are named but not decoded.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// a header field
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    /// unfolded, without the space after the colon
    pub value: String,
}

/// a message: its header fields in order and the body bytes
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub fields: Vec<Field>,
    pub body: Vec<u8>,
}

/// the position after the header (the empty line) and where the header ends
fn header_end(b: &[u8]) -> (usize, usize) {
    let mut i = 0;
    // an empty line at the very start: no header
    if b.starts_with(b"\r\n") {
        return (0, 2);
    }
    if b.starts_with(b"\n") {
        return (0, 1);
    }
    while i < b.len() {
        if b[i] == b'\n' {
            if b[i + 1..].starts_with(b"\r\n") {
                return (i + 1, i + 3);
            }
            if b[i + 1..].starts_with(b"\n") {
                return (i + 1, i + 2);
            }
        }
        i += 1;
    }
    (b.len(), b.len())
}

impl Message {
    pub fn new() -> Message {
        Message::default()
    }
    /// a message as it travels or lies in a mailbox: header lines (folded lines unfolded),
    /// the empty line, the body; a line without a colon in the header is left out
    pub fn parse(b: &[u8]) -> Message {
        let (hend, bstart) = header_end(b);
        let head = String::from_utf8_lossy(&b[..hend]);
        let mut fields: Vec<Field> = Vec::new();
        for line in head.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            if line.starts_with([' ', '\t']) {
                // a continuation: the line end goes, the whitespace stays (unfolding, 2.2.3)
                if let Some(f) = fields.last_mut() {
                    f.value.push_str(line);
                }
                continue;
            }
            let Some((name, value)) = line.split_once(':') else { continue };
            let name = name.trim_end();
            if name.is_empty() || !name.bytes().all(|c| (33..=126).contains(&c)) {
                continue;
            }
            fields.push(Field { name: String::from(name), value: String::from(value.trim_start()) });
        }
        for f in &mut fields {
            let t = f.value.trim_end().len();
            f.value.truncate(t);
        }
        Message { fields, body: Vec::from(&b[bstart.min(b.len())..]) }
    }
    /// the first field of a name (any case)
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|f| f.name.eq_ignore_ascii_case(name)).map(|f| f.value.as_str())
    }
    /// every field of a name, in order
    pub fn get_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.fields.iter().filter(move |f| f.name.eq_ignore_ascii_case(name)).map(|f| f.value.as_str())
    }
    /// replaces the first field of a name (or adds it at the end)
    pub fn set(&mut self, name: &str, value: &str) {
        match self.fields.iter_mut().find(|f| f.name.eq_ignore_ascii_case(name)) {
            Some(f) => f.value = String::from(value),
            None => self.add(name, value),
        }
    }
    pub fn add(&mut self, name: &str, value: &str) {
        self.fields.push(Field { name: String::from(name), value: String::from(value) });
    }
    /// a field at the top (trace fields: Received, Return-Path)
    pub fn prepend(&mut self, name: &str, value: &str) {
        self.fields.insert(0, Field { name: String::from(name), value: String::from(value) });
    }
    pub fn remove(&mut self, name: &str) {
        self.fields.retain(|f| !f.name.eq_ignore_ascii_case(name));
    }
    /// the header as it travels (CRLF, folded), without the empty line
    pub fn header_bytes(&self) -> Vec<u8> {
        let mut s = String::new();
        for f in &self.fields {
            s.push_str(&fold(&f.name, &f.value));
        }
        s.into_bytes()
    }
    /// header, empty line, body (the body as it is)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = self.header_bytes();
        v.extend_from_slice(b"\r\n");
        v.extend_from_slice(&self.body);
        v
    }
    /// the subject with its encoded words decoded
    pub fn subject(&self) -> String {
        self.get("Subject").map(decode_words).unwrap_or_default()
    }
    /// the first mailbox of From
    pub fn from(&self) -> Option<Mailbox> {
        self.get("From").and_then(|v| parse_list(v).into_iter().next())
    }
    pub fn to(&self) -> Vec<Mailbox> {
        self.get_all("To").flat_map(parse_list).collect()
    }
    pub fn cc(&self) -> Vec<Mailbox> {
        self.get_all("Cc").flat_map(parse_list).collect()
    }
    /// the Date field in seconds since 1970
    pub fn date(&self) -> Option<i64> {
        self.get("Date").and_then(parse_date)
    }
    /// the body as text (line ends "\n"): the first text/plain part of a multipart, its
    /// transfer encoding and charset undone
    pub fn text(&self) -> String {
        part_text(&self.fields, &self.body, 0)
    }
}

/// the text of a body with the content fields of its header
fn part_text(fields: &[Field], body: &[u8], depth: u8) -> String {
    let get = |n: &str| fields.iter().find(|f| f.name.eq_ignore_ascii_case(n)).map(|f| f.value.as_str());
    let ct = get("Content-Type").unwrap_or("text/plain");
    let (kind, ps) = params(ct);
    if kind.starts_with("multipart/") && depth < 4 {
        if let Some(bd) = ps.iter().find(|p| p.0.eq_ignore_ascii_case("boundary")).map(|p| p.1.clone()) {
            let parts = split_multipart(body, &bd);
            // the first text/plain part, else the first part
            let pick = parts.iter().map(|p| Message::parse(p)).find(|m| params(m.get("Content-Type").unwrap_or("text/plain")).0.eq_ignore_ascii_case("text/plain"));
            if let Some(m) = pick.or_else(|| parts.first().map(|p| Message::parse(p))) {
                return part_text(&m.fields, &m.body, depth + 1);
            }
        }
    }
    let cte = get("Content-Transfer-Encoding").unwrap_or("7bit").trim().to_ascii_lowercase();
    let raw = match cte.as_str() {
        "quoted-printable" => qp_decode(body),
        "base64" => b64_decode(body).unwrap_or_default(),
        _ => Vec::from(body),
    };
    let charset = ps.iter().find(|p| p.0.eq_ignore_ascii_case("charset")).map(|p| p.1.to_ascii_lowercase()).unwrap_or_default();
    let text = if charset == "iso-8859-1" || charset == "latin1" { raw.iter().map(|&c| c as char).collect() } else { String::from_utf8_lossy(&raw).into_owned() };
    text.replace("\r\n", "\n")
}

/// the parts of a multipart body between its boundary lines (RFC 2046 5.1.1)
pub fn split_multipart(text: &[u8], boundary: &str) -> Vec<Vec<u8>> {
    let mark = format!("--{}", boundary);
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    let mut p = 0;
    while p < text.len() {
        let e = text[p..].iter().position(|&c| c == b'\n').map_or(text.len(), |i| p + i + 1);
        let line = &text[p..e];
        let bare = trim_eol(line);
        if bare.starts_with(mark.as_bytes()) {
            let rest = &bare[mark.len()..];
            if let Some(mut c) = cur.take() {
                // the line end before the boundary belongs to it
                if c.ends_with(b"\r\n") {
                    c.truncate(c.len() - 2);
                } else if c.ends_with(b"\n") {
                    c.pop();
                }
                parts.push(c);
            }
            if rest.starts_with(b"--") {
                break;
            }
            cur = Some(Vec::new());
        } else if let Some(c) = cur.as_mut() {
            c.extend_from_slice(line);
        }
        p = e;
    }
    parts
}

fn trim_eol(l: &[u8]) -> &[u8] {
    let l = l.strip_suffix(b"\n").unwrap_or(l);
    let l = l.strip_suffix(b"\r").unwrap_or(l);
    let mut e = l.len();
    while e > 0 && (l[e - 1] == b' ' || l[e - 1] == b'\t') {
        e -= 1;
    }
    &l[..e]
}

/// "Name: value\r\n" folded at whitespace so that no line is longer than 78 characters where
/// that is possible
pub fn fold(name: &str, value: &str) -> String {
    if value.is_empty() {
        return format!("{}:\r\n", name);
    }
    let mut out = String::new();
    let mut line = format!("{}:", name);
    let mut first = true;
    for w in value.split(' ') {
        if w.is_empty() && !first {
            // runs of spaces keep their width
            line.push(' ');
            continue;
        }
        let need = line.chars().count() + 1 + w.chars().count();
        if need > 78 && line.trim_end().len() > name.len() + 1 {
            out.push_str(line.trim_end());
            out.push_str("\r\n");
            line = format!(" {}", w);
        } else {
            line.push(' ');
            line.push_str(w);
        }
        first = false;
    }
    out.push_str(&line);
    out.push_str("\r\n");
    out
}

/// removes the line ends of folded lines (CRLF or LF before whitespace)
pub fn unfold(s: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\r' && b.get(i + 1) == Some(&'\n') && matches!(b.get(i + 2), Some(' ' | '\t')) {
            i += 2;
            continue;
        }
        if b[i] == '\n' && matches!(b.get(i + 1), Some(' ' | '\t')) {
            i += 1;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// the value and parameters of a field like Content-Type: ("text/plain", [("charset",
/// "UTF-8")]); names in lower case, quotes removed
pub fn params(value: &str) -> (String, Vec<(String, String)>) {
    let mut it = split_outside_quotes(value, ';').into_iter();
    let main = it.next().map(|s| String::from(s.trim())).unwrap_or_default();
    let mut ps = Vec::new();
    for p in it {
        let Some((n, v)) = p.split_once('=') else { continue };
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
        ps.push((n.trim().to_ascii_lowercase(), String::from(v)));
    }
    (main.to_ascii_lowercase(), ps)
}

/// one parameter of such a field
pub fn param(value: &str, name: &str) -> Option<String> {
    params(value).1.into_iter().find(|p| p.0.eq_ignore_ascii_case(name)).map(|p| p.1)
}

/// the parts of an address list between its commas (not those inside quotes, angle brackets
/// or comments), as typed
pub fn split_list(s: &str) -> Vec<&str> {
    split_outside_quotes(s, ',')
}

/// splits at `sep` where it is not inside quotes, angle brackets or parentheses
fn split_outside_quotes(s: &str, sep: char) -> Vec<&str> {
    let mut v = Vec::new();
    let (mut q, mut angle, mut paren, mut esc) = (false, false, 0u32, false);
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' if q || paren > 0 => esc = true,
            '"' if paren == 0 => q = !q,
            '<' if !q && paren == 0 => angle = true,
            '>' if !q && paren == 0 => angle = false,
            '(' if !q => paren += 1,
            ')' if !q && paren > 0 => paren -= 1,
            c if c == sep && !q && !angle && paren == 0 => {
                v.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    v.push(&s[start..]);
    v
}

// ---------------------------------------------------------------- addresses

/// an address with its display name (empty when there is none)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mailbox {
    pub name: String,
    pub addr: String,
}

/// the characters of an atom (RFC 5322 3.2.3)
fn atext(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c)
}

impl Mailbox {
    pub fn new(name: &str, addr: &str) -> Mailbox {
        Mailbox { name: String::from(name), addr: String::from(addr) }
    }
    /// as a header writes it: "addr", "Name <addr>", "\"Muster, Anna\" <addr>" or an encoded
    /// word for a name outside ASCII
    pub fn to_header(&self) -> String {
        let n = self.name.trim();
        if n.is_empty() {
            return self.addr.clone();
        }
        let shown = if !n.is_ascii() {
            encode_word(n)
        } else if n.split(' ').all(|w| !w.is_empty() && w.chars().all(atext)) {
            String::from(n)
        } else {
            format!("\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\""))
        };
        format!("{} <{}>", shown, self.addr)
    }
    /// the name when there is one, else the address
    pub fn shown(&self) -> &str {
        if self.name.trim().is_empty() { &self.addr } else { self.name.trim() }
    }
}

/// a mailbox: "Anna <anna@a.lab>", "\"Muster, Anna\" <anna@a.lab>", "anna@a.lab", "anna@a.lab
/// (Anna)"; None when no address can be read
pub fn parse_mailbox(s: &str) -> Option<Mailbox> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let (Some(a), Some(b)) = (s.rfind('<'), s.rfind('>'))
        && a < b
    {
        let addr = s[a + 1..b].trim();
        let raw = s[..a].trim();
        let name = unquote(raw);
        return addr_ok(addr).then(|| Mailbox { name: decode_words(&name), addr: String::from(addr) });
    }
    // "addr (comment)": the comment as the name
    let (addr, name) = match (s.find('('), s.rfind(')')) {
        (Some(a), Some(b)) if a < b => (s[..a].trim(), s[a + 1..b].trim()),
        _ => (s, ""),
    };
    addr_ok(addr).then(|| Mailbox { name: decode_words(name), addr: String::from(addr) })
}

/// a display name without its quotes and escapes
fn unquote(s: &str) -> String {
    let s = s.trim();
    match s.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        Some(inner) => {
            let mut out = String::new();
            let mut esc = false;
            for c in inner.chars() {
                if esc {
                    out.push(c);
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else {
                    out.push(c);
                }
            }
            out
        }
        None => String::from(s),
    }
}

/// the mailboxes of an address list ("a@x, B <b@y>"; a group "Team: a@x, b@y;" gives its
/// members, "undisclosed-recipients:;" none)
pub fn parse_list(s: &str) -> Vec<Mailbox> {
    let mut v = Vec::new();
    for part in split_outside_quotes(s, ',') {
        let mut p = part.trim();
        // a group's name and its end
        if let Some(i) = p.find(':')
            && !p[..i].contains(['<', '"', '@'])
        {
            p = p[i + 1..].trim();
        }
        let p = p.trim_end_matches(';').trim();
        if let Some(m) = parse_mailbox(p) {
            v.push(m);
        }
    }
    v
}

/// the address has the form local@domain: a dot-atom or a quoted local part, a domain of
/// labels (letters, digits, hyphens) or an address literal in brackets
pub fn addr_ok(a: &str) -> bool {
    let Some((local, domain)) = split_addr(a) else { return false };
    if local.is_empty() || local.len() > 64 || domain.is_empty() || domain.len() > 255 {
        return false;
    }
    let local_ok = if local.starts_with('"') {
        local.len() >= 2 && local.ends_with('"')
    } else {
        local.split('.').all(|w| !w.is_empty() && w.chars().all(atext))
    };
    let domain_ok = if domain.starts_with('[') {
        domain.ends_with(']')
    } else {
        domain.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !l.starts_with('-') && !l.ends_with('-'))
    };
    local_ok && domain_ok
}

/// (local part, domain) at the last '@'
pub fn split_addr(a: &str) -> Option<(&str, &str)> {
    let i = a.rfind('@')?;
    Some((&a[..i], &a[i + 1..]))
}

// ---------------------------------------------------------------- dates and ids

const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// (year, month, day) of a day count since 1970-01-01 (proleptic Gregorian calendar)
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

/// the day count of a date
fn days_of(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "Thu, 01 Jan 2026 00:00:05 +0000" of seconds since 1970 (UTC)
pub fn date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    format!("{}, {:02} {} {:04} {:02}:{:02}:{:02} +0000", DAYS[days.rem_euclid(7) as usize], d, MONTHS[(m - 1) as usize], y, secs / 3600, secs / 60 % 60, secs % 60)
}

/// the seconds since 1970 of a Date field: the day of the week is optional, the zone may be
/// +hhmm / -hhmm or one of the obsolete names (UT, GMT, Z, EST ...)
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.split('(').next().unwrap_or(s);
    let s = match s.split_once(',') {
        Some((_, rest)) => rest,
        None => s,
    };
    let w: Vec<&str> = s.split_whitespace().collect();
    if w.len() < 4 {
        return None;
    }
    let d: u32 = w[0].parse().ok()?;
    let m = MONTHS.iter().position(|x| x.eq_ignore_ascii_case(w[1]))? as u32 + 1;
    let mut y: i64 = w[2].parse().ok()?;
    if w[2].len() == 2 {
        y += if y < 50 { 2000 } else { 1900 };
    }
    let t: Vec<&str> = w[3].split(':').collect();
    if t.len() < 2 || !(1..=31).contains(&d) {
        return None;
    }
    let hh: i64 = t[0].parse().ok()?;
    let mm: i64 = t[1].parse().ok()?;
    let ss: i64 = t.get(2).map_or(Some(0), |x| x.parse().ok())?;
    let zone = w.get(4).copied().unwrap_or("+0000");
    let off = match zone.as_bytes().first() {
        Some(b'+') | Some(b'-') if zone.len() == 5 => {
            let v: i64 = zone[1..].parse().ok()?;
            let o = v / 100 * 3600 + v % 100 * 60;
            if zone.starts_with('-') { -o } else { o }
        }
        _ => match zone.to_ascii_uppercase().as_str() {
            "EDT" => -4 * 3600,
            "EST" | "CDT" => -5 * 3600,
            "CST" | "MDT" => -6 * 3600,
            "MST" | "PDT" => -7 * 3600,
            "PST" => -8 * 3600,
            _ => 0,
        },
    };
    Some(days_of(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - off)
}

/// a Message-ID: "<left@domain>"
pub fn message_id(left: &str, domain: &str) -> String {
    format!("<{}@{}>", left, domain)
}

// ---------------------------------------------------------------- encoded words (RFC 2047)

fn hex(n: u8) -> char {
    b"0123456789ABCDEF"[(n & 15) as usize] as char
}

fn unhex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// text for a header: as it is in ASCII, else encoded words "=?UTF-8?Q?...?=" of at most 75
/// characters each (separated by a space, a character never split)
pub fn encode_word(text: &str) -> String {
    if text.is_ascii() && !text.contains("=?") {
        return String::from(text);
    }
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        let mut enc = String::new();
        let mut buf = [0u8; 4];
        for &b in c.encode_utf8(&mut buf).as_bytes() {
            match b {
                b' ' => enc.push('_'),
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'!' | b'*' | b'+' | b'-' | b'/' => enc.push(b as char),
                _ => {
                    enc.push('=');
                    enc.push(hex(b >> 4));
                    enc.push(hex(b));
                }
            }
        }
        // "=?UTF-8?Q?" + "?=" take 12 of the 75
        if cur.len() + enc.len() > 63 {
            words.push(format!("=?UTF-8?Q?{}?=", cur));
            cur.clear();
        }
        cur.push_str(&enc);
    }
    if !cur.is_empty() || words.is_empty() {
        words.push(format!("=?UTF-8?Q?{}?=", cur));
    }
    words.join(" ")
}

/// one encoded word's text, or None when it is none
fn decode_one(w: &str) -> Option<String> {
    let inner = w.strip_prefix("=?")?.strip_suffix("?=")?;
    let mut parts = inner.splitn(3, '?');
    let charset = parts.next()?.split('*').next()?.to_ascii_lowercase();
    let enc = parts.next()?.to_ascii_uppercase();
    let data = parts.next()?;
    let bytes: Vec<u8> = match enc.as_str() {
        "B" => b64_decode(data.as_bytes())?,
        "Q" => {
            let b = data.as_bytes();
            let mut v = Vec::new();
            let mut i = 0;
            while i < b.len() {
                match b[i] {
                    b'_' => v.push(b' '),
                    b'=' if i + 2 < b.len() => {
                        v.push(unhex(b[i + 1])? << 4 | unhex(b[i + 2])?);
                        i += 2;
                    }
                    c => v.push(c),
                }
                i += 1;
            }
            v
        }
        _ => return None,
    };
    Some(if charset == "iso-8859-1" || charset == "latin1" { bytes.iter().map(|&c| c as char).collect() } else { String::from_utf8_lossy(&bytes).into_owned() })
}

/// a header value with its encoded words decoded (whitespace between two encoded words goes)
pub fn decode_words(s: &str) -> String {
    if !s.contains("=?") {
        return String::from(s);
    }
    let mut out = String::new();
    let mut prev_encoded = false;
    let mut pending_space = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        // whitespace
        let ws = rest.len() - rest.trim_start().len();
        if ws > 0 {
            pending_space.push_str(&rest[..ws]);
            rest = &rest[ws..];
            continue;
        }
        let end = rest.find([' ', '\t']).unwrap_or(rest.len());
        let tok = &rest[..end];
        match decode_one(tok) {
            Some(t) => {
                if !prev_encoded {
                    out.push_str(&pending_space);
                }
                out.push_str(&t);
                prev_encoded = true;
            }
            None => {
                out.push_str(&pending_space);
                out.push_str(tok);
                prev_encoded = false;
            }
        }
        pending_space.clear();
        rest = &rest[end..];
    }
    out
}

// ---------------------------------------------------------------- quoted-printable, base64

/// quoted-printable (RFC 2045 6.7): lines end in CRLF, a line longer than 76 breaks softly
/// ("=" at its end), "=", control and 8-bit bytes as "=XX", spaces at a line end too
pub fn qp_encode(text: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let s = text.strip_suffix(b"\n").unwrap_or(text);
    for (k, raw) in s.split(|&c| c == b'\n').enumerate() {
        if k > 0 {
            out.extend_from_slice(b"\r\n");
        }
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let mut col = 0;
        for (i, &b) in line.iter().enumerate() {
            let last = i + 1 == line.len();
            let lit = ((33..=126).contains(&b) && b != b'=') || ((b == b' ' || b == b'\t') && !last);
            let n = if lit { 1 } else { 3 };
            if col + n > 75 {
                out.extend_from_slice(b"=\r\n");
                col = 0;
            }
            if lit {
                out.push(b);
            } else {
                out.push(b'=');
                out.push(hex(b >> 4) as u8);
                out.push(hex(b) as u8);
            }
            col += n;
        }
    }
    if text.ends_with(b"\n") {
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// undoes quoted-printable: "=XX" bytes, soft line breaks, whitespace at line ends
pub fn qp_decode(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut lines = b.split(|&c| c == b'\n').peekable();
    while let Some(raw) = lines.next() {
        let more = lines.peek().is_some();
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let mut e = line.len();
        while e > 0 && (line[e - 1] == b' ' || line[e - 1] == b'\t') {
            e -= 1;
        }
        let line = &line[..e];
        let (line, soft) = match line.strip_suffix(b"=") {
            Some(l) => (l, true),
            None => (line, false),
        };
        let mut i = 0;
        while i < line.len() {
            if line[i] == b'='
                && let (Some(h), Some(l)) = (line.get(i + 1).and_then(|&c| unhex(c)), line.get(i + 2).and_then(|&c| unhex(c)))
            {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
            out.push(line[i]);
            i += 1;
        }
        if more && !soft {
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// base64 with padding, one line
pub fn b64_encode(data: &[u8]) -> String {
    let mut s = String::new();
    for ch in data.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if ch.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if ch.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    s
}

/// base64 in MIME lines of 76 characters with CRLF
pub fn b64_lines(data: &[u8]) -> String {
    let all = b64_encode(data);
    let mut s = String::new();
    for ch in all.as_bytes().chunks(76) {
        s.push_str(core::str::from_utf8(ch).unwrap_or(""));
        s.push_str("\r\n");
    }
    s
}

/// base64 back to bytes; whitespace and line ends are skipped, None for other characters
pub fn b64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- a mail program's message

/// what a mail program puts into a new message
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub from: Mailbox,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub subject: String,
    /// seconds since 1970
    pub date: i64,
    /// the Message-ID with its angle brackets
    pub id: String,
    /// the Message-ID of the message answered (empty: none)
    pub in_reply_to: String,
    /// the References of a reply: the ids before, separated by spaces
    pub references: String,
    /// the text with "\n" line ends
    pub text: String,
}

impl Draft {
    /// the message (CRLF): 7bit when the text is ASCII, else 8bit when the server takes
    /// 8BITMIME, else quoted-printable; lines over 998 bytes always go quoted-printable
    pub fn to_bytes(&self, eight_bit: bool) -> Vec<u8> {
        let mut m = Message::new();
        m.add("From", &self.from.to_header());
        let list = |v: &[Mailbox]| v.iter().map(Mailbox::to_header).collect::<Vec<_>>().join(", ");
        if !self.to.is_empty() {
            m.add("To", &list(&self.to));
        }
        if !self.cc.is_empty() {
            m.add("Cc", &list(&self.cc));
        }
        m.add("Subject", &encode_word(&self.subject));
        m.add("Date", &date(self.date));
        m.add("Message-ID", &self.id);
        if !self.in_reply_to.is_empty() {
            m.add("In-Reply-To", &self.in_reply_to);
        }
        if !self.references.is_empty() {
            m.add("References", &self.references);
        }
        m.add("MIME-Version", "1.0");
        m.add("Content-Type", "text/plain; charset=UTF-8");
        let text = self.text.replace("\r\n", "\n");
        let long = text.split('\n').any(|l| l.len() > 998);
        let (cte, body) = if text.is_ascii() && !long {
            ("7bit", crlf(text.as_bytes()))
        } else if eight_bit && !long {
            ("8bit", crlf(text.as_bytes()))
        } else {
            ("quoted-printable", qp_encode(text.as_bytes()))
        };
        m.add("Content-Transfer-Encoding", cte);
        m.body = body;
        if !m.body.is_empty() && !m.body.ends_with(b"\r\n") {
            m.body.extend_from_slice(b"\r\n");
        }
        m.to_bytes()
    }
}

/// text with "\n" (or CRLF) line ends as CRLF lines
pub fn crlf(b: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(b.len() + b.len() / 32);
    let mut prev = 0u8;
    for &c in b {
        if c == b'\n' && prev != b'\r' {
            v.push(b'\r');
        }
        v.push(c);
        prev = c;
    }
    v
}

/// CRLF line ends as "\n" (a mailbox file keeps its messages that way)
pub fn lf(b: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        v.push(b[i]);
        i += 1;
    }
    v
}

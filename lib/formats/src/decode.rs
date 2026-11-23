// decode -- formats::encoding's Decoder: the decoders of the WHATWG Encoding Standard for input in pieces,
// UTF-8, UTF-16, the single-byte sets, the replacement encoding and the multi-byte encodings, each the
// standard's algorithm byte by byte (with a fast path for ASCII where no sequence is open).
//
// The standard lets a decoder "restore" bytes it has read: they are read again, before the rest of the input.
// They wait in a small queue here, so a sequence may end in one piece of input and be read again from the next.
use alloc::string::String;

use crate::cjk;
use crate::encoding::Label;
use crate::multibyte::{big5_code_point, gb18030_range_code_point, MultiByte};

/// A decoder for input in pieces: `decode(piece, last, out)` appends the text of each piece to `out`; a
/// sequence cut between two pieces is finished by the next one.  The output is the same as decoding the whole
/// input at once with `decode_label` (without its byte-order mark rule; see `Decoder::sniffing`).
///
/// ```ignore
/// let mut d = Decoder::new(Label::Multi(MultiByte::ShiftJis));
/// let mut text = String::new();
/// d.decode(&[0x82], false, &mut text);
/// d.decode(&[0xA0], true, &mut text);
/// assert_eq!(text, "あ");
/// ```
#[derive(Clone, Debug)]
pub struct Decoder {
    label: Label,
    state: State,
    queue: Queue,
    // the byte-order mark rule: Some(bytes read so far) until it is decided
    sniff: Option<([u8; 3], u8)>,
    malformed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Utf8 { cp: u32, seen: u8, need: u8, lower: u8, upper: u8 },
    Utf16 { le: bool, lead: Option<u8>, surrogate: Option<u16> },
    Single,
    Replacement { done: bool },
    ShiftJis { lead: u8 },
    EucJp { lead: u8, jis0212: bool },
    Iso2022Jp { state: Jp, output: Jp, lead: u8, flag: bool },
    Gb18030 { first: u8, second: u8, third: u8 },
    Big5 { lead: u8 },
    EucKr { lead: u8 },
}

// the states of the ISO-2022-JP decoder
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Jp {
    Ascii,
    Roman,
    Katakana,
    LeadByte,
    TrailByte,
    EscapeStart,
    Escape,
}

// what a step gives
enum Out {
    Continue,
    Finished,
    Error,
    Char(u32),
    Pair(u32, u32),
}

// bytes restored by a step, read before the rest of the input (front first)
#[derive(Clone, Copy, Debug, Default)]
struct Queue {
    b: [u8; 8],
    n: u8,
}

impl Queue {
    fn pop(&mut self) -> Option<u8> {
        if self.n == 0 {
            return None;
        }
        let x = self.b[0];
        self.b.copy_within(1..self.n as usize, 0);
        self.n -= 1;
        Some(x)
    }

    // put bytes in front of the queue (at most 3 at a time; the queue never holds more than 6)
    fn restore(&mut self, bytes: &[u8]) {
        let (n, k) = (self.n as usize, bytes.len());
        debug_assert!(n + k <= self.b.len());
        if n + k > self.b.len() {
            return;
        }
        self.b.copy_within(0..n, k);
        self.b[..k].copy_from_slice(bytes);
        self.n += k as u8;
    }
}

fn fresh(label: Label) -> State {
    match label {
        Label::Utf8 => State::Utf8 { cp: 0, seen: 0, need: 0, lower: 0x80, upper: 0xBF },
        Label::Utf16Le => State::Utf16 { le: true, lead: None, surrogate: None },
        Label::Utf16Be => State::Utf16 { le: false, lead: None, surrogate: None },
        Label::Single(_) => State::Single,
        Label::Replacement => State::Replacement { done: false },
        Label::Multi(m) => match m {
            MultiByte::ShiftJis => State::ShiftJis { lead: 0 },
            MultiByte::EucJp => State::EucJp { lead: 0, jis0212: false },
            MultiByte::Iso2022Jp => State::Iso2022Jp { state: Jp::Ascii, output: Jp::Ascii, lead: 0, flag: false },
            MultiByte::Gbk | MultiByte::Gb18030 => State::Gb18030 { first: 0, second: 0, third: 0 },
            MultiByte::Big5 => State::Big5 { lead: 0 },
            MultiByte::EucKr => State::EucKr { lead: 0 },
        },
    }
}

impl Decoder {
    /// A decoder of the encoding a label names (`encoding::label`).
    pub fn new(label: Label) -> Decoder {
        Decoder { label, state: fresh(label), queue: Queue::default(), sniff: None, malformed: false }
    }

    /// A decoder with the standard's byte-order mark rule (its "decode"): a UTF-8, UTF-16LE or UTF-16BE
    /// byte-order mark at the start wins over the label and is dropped.
    pub fn sniffing(label: Label) -> Decoder {
        Decoder { sniff: Some(([0; 3], 0)), ..Decoder::new(label) }
    }

    /// The encoding decoded: the label's, or the byte-order mark's once one was found.
    pub fn label(&self) -> Label {
        self.label
    }

    /// Whether some input was malformed (a U+FFFD in the output came from an error, not from the input).
    pub fn malformed(&self) -> bool {
        self.malformed
    }

    /// Append the text of `input` to `out`.  `last`: no more input follows; an open sequence becomes U+FFFD,
    /// and a following call starts a new input in the encoding `label` names (a byte-order mark is not looked
    /// for again).  The output grows by at most three bytes per input byte, and by six for what an earlier
    /// piece left open.
    pub fn decode(&mut self, input: &[u8], last: bool, out: &mut String) {
        let mut input = input;
        // the byte-order mark: hold the first bytes while they may still become one
        let mut held = [0u8; 3];
        let mut held_n = 0;
        if let Some((mut b, mut n)) = self.sniff {
            while (n as usize) < 3 && !input.is_empty() && maybe_bom(&b[..n as usize]) {
                b[n as usize] = input[0];
                n += 1;
                input = &input[1..];
            }
            let s = &b[..n as usize];
            let bom = if s.starts_with(&[0xEF, 0xBB, 0xBF]) {
                Some((Label::Utf8, 3))
            } else if s.starts_with(&[0xFF, 0xFE]) {
                Some((Label::Utf16Le, 2))
            } else if s.starts_with(&[0xFE, 0xFF]) {
                Some((Label::Utf16Be, 2))
            } else {
                None
            };
            if bom.is_none() && maybe_bom(s) && !last {
                self.sniff = Some((b, n));
                return;
            }
            self.sniff = None;
            let skip = match bom {
                Some((l, k)) => {
                    self.label = l;
                    self.state = fresh(l);
                    k
                }
                None => 0,
            };
            held_n = s.len() - skip;
            held[..held_n].copy_from_slice(&s[skip..]);
        }
        if held_n > 0 {
            self.run(&held[..held_n], last && input.is_empty(), out);
        }
        if !input.is_empty() || held_n == 0 {
            self.run(input, last, out);
        }
    }

    fn run(&mut self, input: &[u8], last: bool, out: &mut String) {
        if let Label::Single(c) = self.label {
            for &b in input {
                let ch = c.decode_char(b);
                self.malformed |= ch.is_none();
                out.push(ch.unwrap_or(char::REPLACEMENT_CHARACTER));
            }
            return;
        }
        if let State::Replacement { done } = &mut self.state {
            if !*done && !input.is_empty() {
                *done = true;
                self.malformed = true;
                out.push(char::REPLACEMENT_CHARACTER);
            }
            if last {
                self.state = State::Replacement { done: false };
            }
            return;
        }
        out.reserve(input.len());
        let mut i = 0;
        loop {
            let b = if let Some(b) = self.queue.pop() {
                Some(b)
            } else if i < input.len() {
                // where no sequence is open: valid UTF-8 at once, ASCII as itself (in ISO-2022-JP's ASCII state
                // but SO, SI and ESC)
                let run = match self.state {
                    State::Utf8 { need: 0, .. } => match core::str::from_utf8(&input[i..]) {
                        Ok(s) => s.len(),
                        Err(e) => e.valid_up_to(),
                    },
                    State::Iso2022Jp { state: Jp::Ascii, ref mut flag, .. } => {
                        let n = input[i..].iter().position(|&x| x >= 0x80 || matches!(x, 0x0E | 0x0F | 0x1B)).unwrap_or(input.len() - i);
                        if n > 0 {
                            *flag = false;
                        }
                        n
                    }
                    _ if self.idle() => input[i..].iter().position(|&x| x >= 0x80).unwrap_or(input.len() - i),
                    _ => 0,
                };
                if run > 0 {
                    out.push_str(core::str::from_utf8(&input[i..i + run]).unwrap_or(""));
                    i += run;
                    continue;
                }
                i += 1;
                Some(input[i - 1])
            } else if last {
                None
            } else {
                return;
            };
            match self.step(b) {
                Out::Continue => {}
                Out::Finished => {
                    self.state = fresh(self.label);
                    return;
                }
                Out::Error => {
                    self.malformed = true;
                    out.push(char::REPLACEMENT_CHARACTER);
                }
                Out::Char(c) => out.push(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER)),
                Out::Pair(a, c) => {
                    out.push(char::from_u32(a).unwrap_or(char::REPLACEMENT_CHARACTER));
                    out.push(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
                }
            }
        }
    }

    // no sequence open, ASCII is ASCII
    fn idle(&self) -> bool {
        match self.state {
            State::Utf8 { need, .. } => need == 0,
            State::ShiftJis { lead } | State::Big5 { lead } | State::EucKr { lead } | State::EucJp { lead, .. } => lead == 0,
            State::Gb18030 { first, .. } => first == 0,
            _ => false,
        }
    }

    // one step of the standard's decoder: a byte, or None for the end of the input
    fn step(&mut self, byte: Option<u8>) -> Out {
        let q = &mut self.queue;
        match &mut self.state {
            State::Utf8 { cp, seen, need, lower, upper } => {
                let Some(b) = byte else {
                    if *need != 0 {
                        *need = 0;
                        return Out::Error;
                    }
                    return Out::Finished;
                };
                if *need == 0 {
                    match b {
                        0x00..=0x7F => return Out::Char(b as u32),
                        0xC2..=0xDF => {
                            *need = 1;
                            *cp = (b & 0x1F) as u32;
                        }
                        0xE0..=0xEF => {
                            if b == 0xE0 {
                                *lower = 0xA0;
                            }
                            if b == 0xED {
                                *upper = 0x9F;
                            }
                            *need = 2;
                            *cp = (b & 0xF) as u32;
                        }
                        0xF0..=0xF4 => {
                            if b == 0xF0 {
                                *lower = 0x90;
                            }
                            if b == 0xF4 {
                                *upper = 0x8F;
                            }
                            *need = 3;
                            *cp = (b & 0x7) as u32;
                        }
                        _ => return Out::Error,
                    }
                    return Out::Continue;
                }
                if b < *lower || b > *upper {
                    (*cp, *need, *seen, *lower, *upper) = (0, 0, 0, 0x80, 0xBF);
                    q.restore(&[b]);
                    return Out::Error;
                }
                (*lower, *upper) = (0x80, 0xBF);
                *cp = (*cp << 6) | (b & 0x3F) as u32;
                *seen += 1;
                if seen != need {
                    return Out::Continue;
                }
                let c = *cp;
                (*cp, *need, *seen) = (0, 0, 0);
                Out::Char(c)
            }
            State::Utf16 { le, lead, surrogate } => {
                let Some(b) = byte else {
                    if lead.is_some() || surrogate.is_some() {
                        (*lead, *surrogate) = (None, None);
                        return Out::Error;
                    }
                    return Out::Finished;
                };
                let Some(l) = lead.take() else {
                    *lead = Some(b);
                    return Out::Continue;
                };
                let unit = if *le { (b as u16) << 8 | l as u16 } else { (l as u16) << 8 | b as u16 };
                if let Some(s) = surrogate.take() {
                    if (0xDC00..=0xDFFF).contains(&unit) {
                        return Out::Char(0x10000 + ((s as u32 - 0xD800) << 10) + (unit as u32 - 0xDC00));
                    }
                    let (hi, lo) = ((unit >> 8) as u8, unit as u8);
                    q.restore(&if *le { [lo, hi] } else { [hi, lo] });
                    return Out::Error;
                }
                match unit {
                    0xD800..=0xDBFF => {
                        *surrogate = Some(unit);
                        Out::Continue
                    }
                    0xDC00..=0xDFFF => Out::Error,
                    _ => Out::Char(unit as u32),
                }
            }
            State::Single | State::Replacement { .. } => Out::Finished,
            State::ShiftJis { lead } => {
                let Some(b) = byte else {
                    return end(lead);
                };
                if *lead != 0 {
                    let l = core::mem::take(lead) as usize;
                    let offset = if b < 0x7F { 0x40 } else { 0x41 };
                    let lead_offset = if l < 0xA0 { 0x81 } else { 0xC1 };
                    if matches!(b, 0x40..=0x7E | 0x80..=0xFC) {
                        let p = (l - lead_offset) * 188 + b as usize - offset;
                        if (8836..=10715).contains(&p) {
                            return Out::Char(0xE000 - 8836 + p as u32);
                        }
                        let c = cjk::JIS0208.get(p);
                        if c != 0 {
                            return Out::Char(c as u32);
                        }
                    }
                    return error_keep_ascii(b, q);
                }
                match b {
                    0x00..=0x80 => Out::Char(b as u32),
                    0xA1..=0xDF => Out::Char(0xFF61 - 0xA1 + b as u32),
                    0x81..=0x9F | 0xE0..=0xFC => {
                        *lead = b;
                        Out::Continue
                    }
                    _ => Out::Error,
                }
            }
            State::EucJp { lead, jis0212 } => {
                let Some(b) = byte else {
                    return end(lead);
                };
                if *lead == 0x8E && (0xA1..=0xDF).contains(&b) {
                    *lead = 0;
                    return Out::Char(0xFF61 - 0xA1 + b as u32);
                }
                if *lead == 0x8F && (0xA1..=0xFE).contains(&b) {
                    *jis0212 = true;
                    *lead = b;
                    return Out::Continue;
                }
                if *lead != 0 {
                    let l = core::mem::take(lead);
                    let mut c = 0;
                    if (0xA1..=0xFE).contains(&l) && (0xA1..=0xFE).contains(&b) {
                        let p = (l - 0xA1) as usize * 94 + (b - 0xA1) as usize;
                        c = if *jis0212 { cjk::JIS0212.get(p) } else { cjk::JIS0208.get(p) };
                    }
                    *jis0212 = false;
                    if c != 0 {
                        return Out::Char(c as u32);
                    }
                    return error_keep_ascii(b, q);
                }
                match b {
                    0x00..=0x7F => Out::Char(b as u32),
                    0x8E | 0x8F | 0xA1..=0xFE => {
                        *lead = b;
                        Out::Continue
                    }
                    _ => Out::Error,
                }
            }
            State::Iso2022Jp { state, output, lead, flag } => iso2022jp(state, output, lead, flag, byte, q),
            State::Gb18030 { first, second, third } => {
                let Some(b) = byte else {
                    if *first == 0 && *second == 0 && *third == 0 {
                        return Out::Finished;
                    }
                    (*first, *second, *third) = (0, 0, 0);
                    return Out::Error;
                };
                if *third != 0 {
                    if !b.is_ascii_digit() {
                        q.restore(&[*second, *third, b]);
                        (*first, *second, *third) = (0, 0, 0);
                        return Out::Error;
                    }
                    let p = (*first as u32 - 0x81) * 12600 + (*second as u32 - 0x30) * 1260 + (*third as u32 - 0x81) * 10 + b as u32 - 0x30;
                    (*first, *second, *third) = (0, 0, 0);
                    return match gb18030_range_code_point(p) {
                        Some(c) => Out::Char(c),
                        None => Out::Error,
                    };
                }
                if *second != 0 {
                    if (0x81..=0xFE).contains(&b) {
                        *third = b;
                        return Out::Continue;
                    }
                    q.restore(&[*second, b]);
                    (*first, *second) = (0, 0);
                    return Out::Error;
                }
                if *first != 0 {
                    if b.is_ascii_digit() {
                        *second = b;
                        return Out::Continue;
                    }
                    let l = core::mem::take(first) as usize;
                    let offset = if b < 0x7F { 0x40 } else { 0x41 };
                    if matches!(b, 0x40..=0x7E | 0x80..=0xFE) {
                        let c = cjk::GB18030.get((l - 0x81) * 190 + b as usize - offset);
                        if c != 0 {
                            return Out::Char(c as u32);
                        }
                    }
                    return error_keep_ascii(b, q);
                }
                match b {
                    0x00..=0x7F => Out::Char(b as u32),
                    0x80 => Out::Char(0x20AC),
                    0x81..=0xFE => {
                        *first = b;
                        Out::Continue
                    }
                    _ => Out::Error,
                }
            }
            State::Big5 { lead } => {
                let Some(b) = byte else {
                    return end(lead);
                };
                if *lead != 0 {
                    let l = core::mem::take(lead) as usize;
                    let offset = if b < 0x7F { 0x40 } else { 0x62 };
                    if matches!(b, 0x40..=0x7E | 0xA1..=0xFE) {
                        let p = (l - 0x81) * 157 + b as usize - offset;
                        match p {
                            1133 => return Out::Pair(0x00CA, 0x0304),
                            1135 => return Out::Pair(0x00CA, 0x030C),
                            1164 => return Out::Pair(0x00EA, 0x0304),
                            1166 => return Out::Pair(0x00EA, 0x030C),
                            _ => {}
                        }
                        let c = big5_code_point(p);
                        if c != 0 {
                            return Out::Char(c);
                        }
                    }
                    return error_keep_ascii(b, q);
                }
                match b {
                    0x00..=0x7F => Out::Char(b as u32),
                    0x81..=0xFE => {
                        *lead = b;
                        Out::Continue
                    }
                    _ => Out::Error,
                }
            }
            State::EucKr { lead } => {
                let Some(b) = byte else {
                    return end(lead);
                };
                if *lead != 0 {
                    let l = core::mem::take(lead) as usize;
                    if (0x41..=0xFE).contains(&b) {
                        let c = cjk::EUC_KR.get((l - 0x81) * 190 + b as usize - 0x41);
                        if c != 0 {
                            return Out::Char(c as u32);
                        }
                    }
                    return error_keep_ascii(b, q);
                }
                match b {
                    0x00..=0x7F => Out::Char(b as u32),
                    0x81..=0xFE => {
                        *lead = b;
                        Out::Continue
                    }
                    _ => Out::Error,
                }
            }
        }
    }
}

// whether these first bytes may still become a byte-order mark (a proper beginning of one)
fn maybe_bom(s: &[u8]) -> bool {
    s.len() < 3 && [0xEF, 0xBB, 0xBF].starts_with(s) || s.len() < 2 && ([0xFF, 0xFE].starts_with(s) || [0xFE, 0xFF].starts_with(s))
}

// the end of the input with a lead byte open (an error) or not (finished)
fn end(lead: &mut u8) -> Out {
    if core::mem::take(lead) != 0 { Out::Error } else { Out::Finished }
}

// a lead byte without a character: an ASCII byte after it is read again
fn error_keep_ascii(b: u8, q: &mut Queue) -> Out {
    if b < 0x80 {
        q.restore(&[b]);
    }
    Out::Error
}

fn iso2022jp(state: &mut Jp, output: &mut Jp, lead: &mut u8, flag: &mut bool, byte: Option<u8>, q: &mut Queue) -> Out {
    let esc = |state: &mut Jp| {
        *state = Jp::EscapeStart;
        Out::Continue
    };
    match *state {
        Jp::Ascii => match byte {
            Some(0x1B) => esc(state),
            Some(b @ 0x00..=0x7F) if b != 0x0E && b != 0x0F => {
                *flag = false;
                Out::Char(b as u32)
            }
            None => Out::Finished,
            _ => {
                *flag = false;
                Out::Error
            }
        },
        Jp::Roman => match byte {
            Some(0x1B) => esc(state),
            Some(0x5C) => {
                *flag = false;
                Out::Char(0xA5)
            }
            Some(0x7E) => {
                *flag = false;
                Out::Char(0x203E)
            }
            Some(b @ 0x00..=0x7F) if b != 0x0E && b != 0x0F => {
                *flag = false;
                Out::Char(b as u32)
            }
            None => Out::Finished,
            _ => {
                *flag = false;
                Out::Error
            }
        },
        Jp::Katakana => match byte {
            Some(0x1B) => esc(state),
            Some(b @ 0x21..=0x5F) => {
                *flag = false;
                Out::Char(0xFF61 - 0x21 + b as u32)
            }
            None => Out::Finished,
            _ => {
                *flag = false;
                Out::Error
            }
        },
        Jp::LeadByte => match byte {
            Some(0x1B) => esc(state),
            Some(b @ 0x21..=0x7E) => {
                *flag = false;
                *lead = b;
                *state = Jp::TrailByte;
                Out::Continue
            }
            None => Out::Finished,
            _ => {
                *flag = false;
                Out::Error
            }
        },
        Jp::TrailByte => match byte {
            Some(0x1B) => {
                *state = Jp::EscapeStart;
                Out::Error
            }
            Some(b @ 0x21..=0x7E) => {
                *state = Jp::LeadByte;
                let c = cjk::JIS0208.get((*lead - 0x21) as usize * 94 + (b - 0x21) as usize);
                if c == 0 { Out::Error } else { Out::Char(c as u32) }
            }
            // (the end of the input is read again in the lead byte state: finished)
            _ => {
                *state = Jp::LeadByte;
                Out::Error
            }
        },
        Jp::EscapeStart => {
            if let Some(b @ (0x24 | 0x28)) = byte {
                *lead = b;
                *state = Jp::Escape;
                return Out::Continue;
            }
            if let Some(b) = byte {
                q.restore(&[b]);
            }
            *flag = false;
            *state = *output;
            Out::Error
        }
        Jp::Escape => {
            let l = core::mem::take(lead);
            let next = match (l, byte) {
                (0x28, Some(0x42)) => Some(Jp::Ascii),
                (0x28, Some(0x4A)) => Some(Jp::Roman),
                (0x28, Some(0x49)) => Some(Jp::Katakana),
                (0x24, Some(0x40 | 0x42)) => Some(Jp::LeadByte),
                _ => None,
            };
            if let Some(s) = next {
                *state = s;
                *output = s;
                let was = core::mem::replace(flag, true);
                return if was { Out::Error } else { Out::Continue };
            }
            match byte {
                Some(b) => q.restore(&[l, b]),
                None => q.restore(&[l]),
            }
            *flag = false;
            *state = *output;
            Out::Error
        }
    }
}

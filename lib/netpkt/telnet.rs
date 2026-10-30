// telnet -- the bytes of the Telnet protocol (TCP port 23, RFC 854): commands after IAC, option
// negotiation (RFC 855) without loops (the Q method of RFC 1143), ECHO (RFC 857) and
// SUPPRESS-GO-AHEAD (RFC 858), IAC escaping, the line ends of the network virtual terminal, a
// reader for bytes that arrive in pieces
//
//   IAC, DONT, DO, WONT, WILL, SB, GA, EL, EC, AYT, AO, IP, BRK, DM, NOP, SE, EOR   commands
//   OPT_* (BINARY 0, ECHO 1, SGA 3, STATUS 5, TM 6, TTYPE 24, EOR 25, NAWS 31, TSPEED 32,
//   LFLOW 33, LINEMODE 34, XDISPLOC 35, ENVIRON 36, NEW_ENVIRON 39)            options
//   cmd_name(c), opt_name(o)        "Do", "Will", "Interrupt Process" ...; "Echo",
//                                   "Suppress Go Ahead" ... (as capture tools name them)
//   escape(data)                    IAC doubled (the bytes of a data stream)
//   to_nvt(text)                    "\n" as CR LF, a lone CR as CR NUL, IAC doubled
//   cmd(c) / opt(verb, o)           the bytes IAC c, IAC verb o
//   Item                            Data(bytes) | Cmd(c) | Opt { verb, opt } | Sub { opt, data }
//                                   | Partial(bytes) (a sequence cut off at the end, parse only)
//   Reader::new(nvt) / push(piece)  the items of a stream; a sequence split between two pieces
//                                   waits for the next one; nvt: CR LF becomes "\n" and CR NUL
//                                   "\r" in the data
//   parse(b)                        the items of one buffer with their offsets and lengths,
//                                   the data as it is (a dissector)
//   Options::new(us, him)           the state of every option on both sides (RFC 1143: No, Yes,
//                                   WantNo, WantYes): received(verb, o) gives the answer to send,
//                                   if any (a request is accepted when its bit is in `us` for
//                                   what this side does, `him` for what the peer does; every
//                                   other option is refused), enable_us / disable_us /
//                                   enable_him / disable_him ask for a change, us(o) / him(o)
//                                   tell what is in effect
//
// A data byte 255 travels as IAC IAC.  An answer is only sent for a request that would change
// something, so two sides that both follow these rules never negotiate in a circle.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// interpret as command
pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
/// subnegotiation begins (ends with IAC SE)
pub const SB: u8 = 250;
/// go ahead
pub const GA: u8 = 249;
/// erase line, erase character, are you there, abort output, interrupt process, break
pub const EL: u8 = 248;
pub const EC: u8 = 247;
pub const AYT: u8 = 246;
pub const AO: u8 = 245;
pub const IP: u8 = 244;
pub const BRK: u8 = 243;
/// data mark (the end of urgent data)
pub const DM: u8 = 242;
pub const NOP: u8 = 241;
/// subnegotiation end
pub const SE: u8 = 240;
/// end of record (RFC 885)
pub const EOR: u8 = 239;

pub const OPT_BINARY: u8 = 0;
pub const OPT_ECHO: u8 = 1;
pub const OPT_SGA: u8 = 3;
pub const OPT_STATUS: u8 = 5;
pub const OPT_TM: u8 = 6;
pub const OPT_TTYPE: u8 = 24;
pub const OPT_EOR: u8 = 25;
pub const OPT_NAWS: u8 = 31;
pub const OPT_TSPEED: u8 = 32;
pub const OPT_LFLOW: u8 = 33;
pub const OPT_LINEMODE: u8 = 34;
pub const OPT_XDISPLOC: u8 = 35;
pub const OPT_ENVIRON: u8 = 36;
pub const OPT_NEW_ENVIRON: u8 = 39;

/// the well-known port
pub const PORT: u16 = 23;

/// the name of a command byte
pub fn cmd_name(c: u8) -> &'static str {
    match c {
        IAC => "IAC",
        DONT => "Don't",
        DO => "Do",
        WONT => "Won't",
        WILL => "Will",
        SB => "Suboption",
        GA => "Go Ahead",
        EL => "Erase Line",
        EC => "Erase Character",
        AYT => "Are You There",
        AO => "Abort Output",
        IP => "Interrupt Process",
        BRK => "Break",
        DM => "Data Mark",
        NOP => "No Operation",
        SE => "Suboption End",
        EOR => "End of Record",
        _ => "Unknown command",
    }
}

/// the name of an option
pub fn opt_name(o: u8) -> String {
    String::from(match o {
        OPT_BINARY => "Binary Transmission",
        OPT_ECHO => "Echo",
        2 => "Reconnection",
        OPT_SGA => "Suppress Go Ahead",
        4 => "Approx Message Size Negotiation",
        OPT_STATUS => "Status",
        OPT_TM => "Timing Mark",
        7 => "Remote Controlled Trans and Echo",
        8 => "Output Line Width",
        9 => "Output Page Size",
        17 => "Extended ASCII",
        18 => "Logout",
        OPT_TTYPE => "Terminal Type",
        OPT_EOR => "End of Record",
        OPT_NAWS => "Negotiate About Window Size",
        OPT_TSPEED => "Terminal Speed",
        OPT_LFLOW => "Remote Flow Control",
        OPT_LINEMODE => "Linemode",
        OPT_XDISPLOC => "X Display Location",
        OPT_ENVIRON => "Environment Option",
        37 => "Authentication Option",
        38 => "Encryption Option",
        OPT_NEW_ENVIRON => "New Environment Option",
        255 => "Extended-Options-List",
        _ => return format!("Option {}", o),
    })
}

/// data with every IAC doubled
pub fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    for &b in data {
        if b == IAC {
            out.push(IAC);
        }
        out.push(b);
    }
    out
}

/// text for the network virtual terminal: "\n" becomes CR LF, a lone CR becomes CR NUL, an IAC
/// is doubled
pub fn to_nvt(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + text.len() / 16 + 4);
    let mut i = 0;
    while i < text.len() {
        match text[i] {
            b'\n' => out.extend_from_slice(b"\r\n"),
            b'\r' if text.get(i + 1) == Some(&b'\n') => {
                out.extend_from_slice(b"\r\n");
                i += 1;
            }
            b'\r' => out.extend_from_slice(b"\r\0"),
            IAC => out.extend_from_slice(&[IAC, IAC]),
            b => out.push(b),
        }
        i += 1;
    }
    out
}

/// IAC c (GA, IP, AYT ...)
pub fn cmd(c: u8) -> [u8; 2] {
    [IAC, c]
}

/// IAC verb option (WILL, WONT, DO, DONT)
pub fn opt(verb: u8, o: u8) -> [u8; 3] {
    [IAC, verb, o]
}

/// a piece of a Telnet stream
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// data bytes (IAC IAC already one byte)
    Data(Vec<u8>),
    /// a command without an option (GA, IP, AYT, NOP ...)
    Cmd(u8),
    /// a negotiation: WILL, WONT, DO or DONT and the option
    Opt { verb: u8, opt: u8 },
    /// a subnegotiation: IAC SB opt data IAC SE
    Sub { opt: u8, data: Vec<u8> },
    /// the start of a sequence cut off at the end of the buffer (parse only)
    Partial(Vec<u8>),
}

impl Item {
    /// "Do Echo", "Interrupt Process", "Suboption Terminal Type" (a negotiation or a command)
    pub fn describe(&self) -> String {
        match self {
            Item::Data(d) => format!("{} bytes of data", d.len()),
            Item::Cmd(c) => String::from(cmd_name(*c)),
            Item::Opt { verb, opt } => format!("{} {}", cmd_name(*verb), opt_name(*opt)),
            Item::Sub { opt, data } => format!("Suboption {} ({} bytes)", opt_name(*opt), data.len()),
            Item::Partial(b) => format!("Cut off ({} bytes)", b.len()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Data,
    Iac,
    Verb(u8),
    SbOpt,
    Sb(u8),
    SbIac(u8),
    /// a CR came last (nvt: what follows decides)
    Cr,
}

/// reads a Telnet stream that arrives in pieces
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reader {
    st: St,
    nvt: bool,
    sub: Vec<u8>,
}

impl Reader {
    /// nvt: CR LF becomes "\n" and CR NUL "\r" (a lone CR stays one)
    pub fn new(nvt: bool) -> Reader {
        Reader { st: St::Data, nvt, sub: Vec::new() }
    }

    /// the items of the next piece; what ends inside a sequence waits for the next piece
    pub fn push(&mut self, piece: &[u8]) -> Vec<Item> {
        let mut out: Vec<Item> = Vec::new();
        let mut data: Vec<u8> = Vec::new();
        let flush = |data: &mut Vec<u8>, out: &mut Vec<Item>| {
            if !data.is_empty() {
                out.push(Item::Data(core::mem::take(data)));
            }
        };
        for &b in piece {
            match self.st {
                St::Cr => {
                    self.st = St::Data;
                    match b {
                        b'\n' => {
                            data.push(b'\n');
                            continue;
                        }
                        0 => {
                            data.push(b'\r');
                            continue;
                        }
                        _ => data.push(b'\r'),
                    }
                    self.data_byte(b, &mut data);
                }
                St::Data => self.data_byte(b, &mut data),
                St::Iac => match b {
                    IAC => {
                        data.push(IAC);
                        self.st = St::Data;
                    }
                    WILL | WONT | DO | DONT => self.st = St::Verb(b),
                    SB => self.st = St::SbOpt,
                    c => {
                        flush(&mut data, &mut out);
                        out.push(Item::Cmd(c));
                        self.st = St::Data;
                    }
                },
                St::Verb(v) => {
                    flush(&mut data, &mut out);
                    out.push(Item::Opt { verb: v, opt: b });
                    self.st = St::Data;
                }
                St::SbOpt => {
                    self.sub.clear();
                    self.st = St::Sb(b);
                }
                St::Sb(o) => {
                    if b == IAC {
                        self.st = St::SbIac(o);
                    } else {
                        self.sub.push(b);
                    }
                }
                St::SbIac(o) => match b {
                    SE => {
                        flush(&mut data, &mut out);
                        out.push(Item::Sub { opt: o, data: core::mem::take(&mut self.sub) });
                        self.st = St::Data;
                    }
                    IAC => {
                        self.sub.push(IAC);
                        self.st = St::Sb(o);
                    }
                    // a broken subnegotiation (RFC 855 allows only IAC SE or IAC IAC here):
                    // it ends, the command counts on its own
                    c => {
                        flush(&mut data, &mut out);
                        out.push(Item::Sub { opt: o, data: core::mem::take(&mut self.sub) });
                        out.push(Item::Cmd(c));
                        self.st = St::Data;
                    }
                },
            }
        }
        flush(&mut data, &mut out);
        out
    }

    fn data_byte(&mut self, b: u8, data: &mut Vec<u8>) {
        match b {
            IAC => self.st = St::Iac,
            b'\r' if self.nvt => self.st = St::Cr,
            _ => data.push(b),
        }
    }
}

/// the items of one buffer with their offset and length, the data as it is
pub fn parse(b: &[u8]) -> Vec<(usize, usize, Item)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != IAC || b.get(i + 1) == Some(&IAC) {
            // data up to the next IAC that is no escaped one
            let start = i;
            let mut d = Vec::new();
            while i < b.len() {
                if b[i] == IAC {
                    if b.get(i + 1) == Some(&IAC) {
                        d.push(IAC);
                        i += 2;
                        continue;
                    }
                    break;
                }
                d.push(b[i]);
                i += 1;
            }
            out.push((start, i - start, Item::Data(d)));
            continue;
        }
        let Some(&c) = b.get(i + 1) else {
            out.push((i, 1, Item::Partial(b[i..].to_vec())));
            break;
        };
        match c {
            WILL | WONT | DO | DONT => match b.get(i + 2) {
                Some(&o) => {
                    out.push((i, 3, Item::Opt { verb: c, opt: o }));
                    i += 3;
                }
                None => {
                    out.push((i, b.len() - i, Item::Partial(b[i..].to_vec())));
                    break;
                }
            },
            SB => {
                let Some(&o) = b.get(i + 2) else {
                    out.push((i, b.len() - i, Item::Partial(b[i..].to_vec())));
                    break;
                };
                let mut j = i + 3;
                let mut d = Vec::new();
                let mut end = None;
                while j < b.len() {
                    if b[j] == IAC && b.get(j + 1) == Some(&SE) {
                        end = Some(j + 2);
                        break;
                    }
                    if b[j] == IAC && b.get(j + 1) == Some(&IAC) {
                        d.push(IAC);
                        j += 2;
                        continue;
                    }
                    d.push(b[j]);
                    j += 1;
                }
                match end {
                    Some(e) => {
                        out.push((i, e - i, Item::Sub { opt: o, data: d }));
                        i = e;
                    }
                    None => {
                        out.push((i, b.len() - i, Item::Partial(b[i..].to_vec())));
                        break;
                    }
                }
            }
            _ => {
                out.push((i, 2, Item::Cmd(c)));
                i += 2;
            }
        }
    }
    out
}

// ---------------------------------------------------------------- option negotiation

/// the state of one side of an option (RFC 1143)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Q {
    No,
    Yes,
    /// we asked to switch it off and wait for the answer
    WantNo,
    /// we asked to switch it on and wait for the answer
    WantYes,
}

/// the options of one end of a connection: what this side does (us) and what the peer does
/// (him)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// (option, us, him) of every option that is not No on both sides
    st: Vec<(u8, Q, Q)>,
    /// the options this side agrees to do / lets the peer do (bit o for option o < 64)
    pub allow_us: u64,
    pub allow_him: u64,
}

fn bit(o: u8) -> u64 {
    if o < 64 { 1u64 << o } else { 0 }
}

impl Options {
    /// what this side agrees to do (`us`) and to let the peer do (`him`); the others are refused
    pub fn new(us: &[u8], him: &[u8]) -> Options {
        Options { st: Vec::new(), allow_us: us.iter().fold(0, |m, &o| m | bit(o)), allow_him: him.iter().fold(0, |m, &o| m | bit(o)) }
    }
    fn get(&self, o: u8) -> (Q, Q) {
        self.st.iter().find(|x| x.0 == o).map(|x| (x.1, x.2)).unwrap_or((Q::No, Q::No))
    }
    fn set(&mut self, o: u8, us: Q, him: Q) {
        self.st.retain(|x| x.0 != o);
        if us != Q::No || him != Q::No {
            self.st.push((o, us, him));
            self.st.sort_by_key(|x| x.0);
        }
    }
    /// this side does option o
    pub fn us(&self, o: u8) -> bool {
        self.get(o).0 == Q::Yes
    }
    /// the peer does option o
    pub fn him(&self, o: u8) -> bool {
        self.get(o).1 == Q::Yes
    }
    /// the state of both sides (us, him)
    pub fn state(&self, o: u8) -> (Q, Q) {
        self.get(o)
    }

    /// a negotiation from the peer: the answer to send, if any (RFC 1143 2.6, 2.7)
    pub fn received(&mut self, verb: u8, o: u8) -> Option<[u8; 3]> {
        let (us, him) = self.get(o);
        match verb {
            WILL => match him {
                Q::No if self.allow_him & bit(o) != 0 => {
                    self.set(o, us, Q::Yes);
                    Some(opt(DO, o))
                }
                Q::No => Some(opt(DONT, o)),
                Q::Yes => None,
                // a DONT answered by WILL (an error of the peer) or our DO accepted
                Q::WantNo => {
                    self.set(o, us, Q::No);
                    None
                }
                Q::WantYes => {
                    self.set(o, us, Q::Yes);
                    None
                }
            },
            WONT => match him {
                Q::No => None,
                Q::Yes => {
                    self.set(o, us, Q::No);
                    Some(opt(DONT, o))
                }
                Q::WantNo | Q::WantYes => {
                    self.set(o, us, Q::No);
                    None
                }
            },
            DO => match us {
                Q::No if self.allow_us & bit(o) != 0 => {
                    self.set(o, Q::Yes, him);
                    Some(opt(WILL, o))
                }
                Q::No => Some(opt(WONT, o)),
                Q::Yes => None,
                Q::WantNo => {
                    self.set(o, Q::No, him);
                    None
                }
                Q::WantYes => {
                    self.set(o, Q::Yes, him);
                    None
                }
            },
            DONT => match us {
                Q::No => None,
                Q::Yes => {
                    self.set(o, Q::No, him);
                    Some(opt(WONT, o))
                }
                Q::WantNo | Q::WantYes => {
                    self.set(o, Q::No, him);
                    None
                }
            },
            _ => None,
        }
    }

    /// this side offers to do option o (WILL), unless it does or asked already
    pub fn enable_us(&mut self, o: u8) -> Option<[u8; 3]> {
        let (us, him) = self.get(o);
        if us != Q::No {
            return None;
        }
        self.set(o, Q::WantYes, him);
        Some(opt(WILL, o))
    }
    /// this side stops doing option o (WONT)
    pub fn disable_us(&mut self, o: u8) -> Option<[u8; 3]> {
        let (us, him) = self.get(o);
        if us != Q::Yes {
            return None;
        }
        self.set(o, Q::WantNo, him);
        Some(opt(WONT, o))
    }
    /// this side asks the peer to do option o (DO)
    pub fn enable_him(&mut self, o: u8) -> Option<[u8; 3]> {
        let (us, him) = self.get(o);
        if him != Q::No {
            return None;
        }
        self.set(o, us, Q::WantYes);
        Some(opt(DO, o))
    }
    /// this side asks the peer to stop option o (DONT)
    pub fn disable_him(&mut self, o: u8) -> Option<[u8; 3]> {
        let (us, him) = self.get(o);
        if him != Q::Yes {
            return None;
        }
        self.set(o, us, Q::WantNo);
        Some(opt(DONT, o))
    }
}

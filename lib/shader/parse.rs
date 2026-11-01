// parse -- SPIR-V words into instructions and operands (the grammar of spv::OPS)
//
//   parse(words) -> Result<Vec<Inst>, String>   the instructions after the 5-word header
//   operands(inst, f)                           f(kind, words) per operand, with the masks'
//                                               and decorations' arguments as operands of
//                                               their own kinds (spv module comment)
//   ids(inst) -> Vec<u32>                       every id operand (result type and id excluded)
//   string(words) -> (String, words used)       a literal string

use alloc::string::String;
use alloc::vec::Vec;

use crate::spv::{self, OpInfo};

#[derive(Clone, Copy, Debug)]
pub struct Inst<'a> {
    pub op: u16,
    /// result type id (0: none)
    pub ty: u32,
    /// result id (0: none)
    pub res: u32,
    /// the operands after the result type and id
    pub ops: &'a [u32],
    /// the word offset of the instruction in the module
    pub at: usize,
    pub info: Option<&'static OpInfo>,
}

pub fn parse(words: &[u32]) -> Result<Vec<Inst<'_>>, String> {
    let mut out = Vec::new();
    let mut i = 5;
    while i < words.len() {
        let n = (words[i] >> 16) as usize;
        let op = (words[i] & 0xffff) as u16;
        if n == 0 || i + n > words.len() {
            return Err(alloc::format!("word {}: instruction length {} runs past the end", i, n));
        }
        let info = spv::op_info(op);
        let body = &words[i + 1..i + n];
        let (ty, res, ops) = match info.map(|o| o.res) {
            Some(spv::R_TY) if body.len() >= 2 => (body[0], body[1], &body[2..]),
            Some(spv::R_ID) if !body.is_empty() => (0, body[0], &body[1..]),
            Some(spv::R_TY) | Some(spv::R_ID) => {
                return Err(alloc::format!("word {}: Op{} is too short", i, info.map_or("?", |o| o.name)));
            }
            _ => (0, 0, body),
        };
        out.push(Inst { op, ty, res, ops, at: i, info });
        i += n;
    }
    Ok(out)
}

/// a literal string starting at words[0] -> (text, words it takes)
pub fn string(words: &[u32]) -> (String, usize) {
    let mut bytes = Vec::new();
    for (k, w) in words.iter().enumerate() {
        for b in w.to_le_bytes() {
            if b == 0 {
                return (String::from_utf8_lossy(&bytes).into_owned(), k + 1);
            }
            bytes.push(b);
        }
    }
    (String::from_utf8_lossy(&bytes).into_owned(), words.len())
}

/// the words of a literal string (UTF-8, NUL terminated, zero padded)
pub fn string_words(s: &str) -> Vec<u32> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 4 + 1);
    for k in 0..b.len() / 4 + 1 {
        let mut w = [0u8; 4];
        for (j, x) in w.iter_mut().enumerate() {
            if let Some(&c) = b.get(k * 4 + j) {
                *x = c;
            }
        }
        out.push(u32::from_le_bytes(w));
    }
    out
}

/// walk the operands: f(kind, words).  Kinds as in the spv module comment; the arguments of
/// masks (l, m, I), execution modes (E: n, G: i), decorations (B for BuiltIn, else n) and
/// switch targets (n, i) are reported one by one after their mask / enum; OpSpecConstantOp's
/// opcode as 'o', then its operation's operands.  Unknown opcodes: every word as 'n'.  Returns
/// false when the words do not fit the grammar.
pub fn operands(inst: &Inst, f: &mut dyn FnMut(char, &[u32])) -> bool {
    let Some(info) = inst.info else {
        for k in 0..inst.ops.len() {
            f('n', &inst.ops[k..k + 1]);
        }
        return true;
    };
    let w = inst.ops;
    let g = info.ops.as_bytes();
    let (mut i, mut p) = (0usize, 0usize);
    let mut optional = false;
    while p < g.len() {
        let c = g[p] as char;
        if c == '?' {
            optional = true;
            p += 1;
            continue;
        }
        let star = g.get(p + 1) == Some(&b'*');
        if i >= w.len() {
            return star || optional || c == 'v';
        }
        let used = match c {
            's' => {
                let n = string(&w[i..]).1;
                f('s', &w[i..i + n]);
                n
            }
            'v' => {
                f('v', &w[i..]);
                w.len() - i
            }
            'E' | 'G' => {
                f('E', &w[i..i + 1]);
                for k in i + 1..w.len() {
                    f(if c == 'G' { 'i' } else { 'n' }, &w[k..k + 1]);
                }
                w.len() - i
            }
            'd' => {
                f('d', &w[i..i + 1]);
                let arg = if w[i] == spv::Dec::BuiltIn as u32 { 'B' } else { 'n' };
                for k in i + 1..w.len() {
                    f(arg, &w[k..k + 1]);
                }
                w.len() - i
            }
            'l' | 'm' | 'I' => {
                let table = match c {
                    'l' => spv::LOOP_CTL,
                    'm' => spv::MEM_ACCESS,
                    _ => spv::IMG_OPS,
                };
                f(c, &w[i..i + 1]);
                let mut k = i + 1;
                for &(bit, _, args) in table {
                    if w[i] & bit != 0 {
                        for a in args.chars() {
                            if k >= w.len() {
                                return false;
                            }
                            f(a, &w[k..k + 1]);
                            k += 1;
                        }
                    }
                }
                k - i
            }
            'o' => {
                // OpSpecConstantOp: the opcode, then the operands of its grammar
                f('o', &w[i..i + 1]);
                let op = w[i] as u16;
                let inner = Inst { op, ty: 0, res: 0, ops: &w[i + 1..], at: inst.at, info: spv::op_info(op) };
                if inner.info.is_some_and(|x| x.res == spv::R_TY) {
                    if !operands(&inner, f) {
                        return false;
                    }
                } else {
                    for k in i + 1..w.len() {
                        f('n', &w[k..k + 1]);
                    }
                }
                w.len() - i
            }
            'w' => {
                let mut k = i;
                while k + 1 < w.len() {
                    f('n', &w[k..k + 1]);
                    f('i', &w[k + 1..k + 2]);
                    k += 2;
                }
                if k != w.len() {
                    return false;
                }
                k - i
            }
            _ => {
                f(c, &w[i..i + 1]);
                1
            }
        };
        i += used;
        if !star {
            p += 1;
        } else if i >= w.len() {
            p += 2;
        }
    }
    i == w.len()
}

/// every id operand of an instruction (result type and result id not included)
pub fn ids(inst: &Inst) -> Vec<u32> {
    let mut v = Vec::new();
    operands(inst, &mut |k, w| {
        if k == 'i' {
            v.push(w[0]);
        }
    });
    v
}

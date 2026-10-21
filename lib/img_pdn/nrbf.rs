// nrbf -- MS-NRBF (.NET BinaryFormatter) reader and template-driven writer   [src/img/nrbf.s]
//
// Reader (Nrbf, borrowing the data):
//   Nrbf::parse(data, maxmem) -> Ok(nb) / Err(ERR_*)   one stream (header .. MessageEnd)
//   nb.end()          offset after MessageEnd          nb.root()   root object index (-1 none)
//   nb.nobj()         objects (stream order)           nb.obj(ix)  the object entry
//   nb.class_name(ix) class of object ix               nb.is(ix, name)   instance of `name`
//   nb.member(ix, name) -> value slot                  nb.get_obj / get_int / get_str
//   nb.str_(ix)       string object                    nb.count(ix)  array elements (-1 none)
//   nb.elem / elem_obj   array element i               nb.prim_data(ix)  raw primitive array
//   nb.visit(ix)      first visit (NROF_VISITED)
// Writer (Nw): nw.reserve / bytes / u8 / i32 / i64 / be32 / lps / id / header / class / str /
//   ref_ / nulls / end, take() -> the buffer (None after an error, which sticks in `err`).
//
// Reader design (as the assembly): one pass, recursive descent over the records (nesting <=
// NRBF_MAXDEPTH), every record bounds checked.  Objects go into a table in stream order
// (inline records included), member values and array elements into value slots; primitive
// arrays and strings keep offsets into the data.  Object ids are unique (an open-addressing
// hash, duplicates -> ERR_HDR); MemberReferences are resolved after MessageEnd (dangling ->
// ERR_HDR), so reference cycles are harmless.  The tables count their bytes like the assembly
// (capacity max(2 cap, need, 16) per growth, the hash table doubling without giving the old
// one back) against the optional budget `maxmem` (ERR_LIMIT), so limits trip at the same
// records.  Errors: ERR_TRUNC, ERR_HDR, ERR_UNSUPPORTED (MethodCall / MethodReturn), ERR_LIMIT.

use alloc::vec::Vec;

use img_core::{ERR_HDR, ERR_LIMIT, ERR_TRUNC, ERR_UNSUPPORTED};

use crate::template::{LIBS, Tpl};

// limits (img.inc)
pub const NRBF_MAXDEPTH: u32 = 64; // record nesting
pub const NRBF_MAXOBJ: u32 = 1 << 20; // objects (class instances, strings, arrays)
pub const NRBF_MAXVAL: u64 = 1 << 22; // value slots (members + array elements)
pub const NRBF_MAXCLS: u32 = 1 << 16; // class metadata records
pub const NRBF_MAXMEMB: u64 = 1 << 20; // member descriptions of all classes
pub const NRBF_MAXLIB: u32 = 4096; // BinaryLibrary records
const NRBF_MAXRANK: u32 = 8;
const NRBF_MAXMCOUNT: u32 = 65536; // members of one class

// object kinds (NRK_*)
pub const NRK_CLASS: u8 = 1;
pub const NRK_STRING: u8 = 2;
pub const NRK_PRIMARR: u8 = 3; // ArraySinglePrimitive / BinaryArray of primitives (raw data)
pub const NRK_ARRAY: u8 = 4; // object / string / class arrays (value slots)
pub const NROF_VISITED: u16 = 1;
// value slot kinds (NRVK_*)
pub const NRVK_NULL: u8 = 0;
pub const NRVK_PRIM: u8 = 1;
pub const NRVK_OBJ: u8 = 2;
pub const NRVK_REF: u8 = 3; // unresolved id (only while parsing)
// MS-NRBF PrimitiveTypeEnumeration
pub const NRT_BOOL: u8 = 1;
pub const NRT_BYTE: u8 = 2;
pub const NRT_CHAR: u8 = 3;
pub const NRT_DECIMAL: u8 = 5;
pub const NRT_DOUBLE: u8 = 6;
pub const NRT_I16: u8 = 7;
pub const NRT_I32: u8 = 8;
pub const NRT_I64: u8 = 9;
pub const NRT_I8: u8 = 10;
pub const NRT_SINGLE: u8 = 11;
pub const NRT_TIMESPAN: u8 = 12;
pub const NRT_DATETIME: u8 = 13;
pub const NRT_U16: u8 = 14;
pub const NRT_U32: u8 = 15;
pub const NRT_U64: u8 = 16;
pub const NRT_NULL: u8 = 17;
pub const NRT_STRING: u8 = 18;

// BinaryTypeEnumeration
const BT_PRIM: u8 = 0;
const BT_SYSCLASS: u8 = 3;
const BT_CLASS: u8 = 4;
const BT_PRIMARR: u8 = 7;

// nb_rec modes
const RM_TOP: u32 = 0;
const RM_MEMBER: u32 = 1;
const RM_ELEM: u32 = 2;

// table entry sizes of the assembly (the byte budget counts them)
const NRO_SIZE: u64 = 32;
const NRV_SIZE: u64 = 16;
const NC_SIZE: u64 = 32;
const NM_SIZE: u64 = 32;
const NL_SIZE: u64 = 16;

// primitive type -> byte size (0xFE variable: Char / Decimal / String, 0xFF invalid)  [nb_ptype]
static PTYPE: [u8; 19] = [0xFF, 1, 1, 0xFE, 0xFF, 0xFE, 8, 2, 4, 8, 1, 4, 8, 8, 2, 4, 8, 0, 0xFE];
// primitive type -> integer-like (get_int accepts it)  [nb_pint]
static PINT: [u8; 19] = [0, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0];

/// an object (NRO_*)
#[derive(Clone, Copy, Default)]
pub struct Obj {
    pub id: i32,
    pub kind: u8,  // NRK_*
    pub ptype: u8, // element type of a primitive array (NRT_*)
    pub flags: u16, // NROF_*
    pub cls: u32,  // class index (NRK_CLASS)
    pub nval: u32, // value slots: members (class) / elements (array)
    pub val: u32,  // first value slot
    pub len: u32,  // string bytes / primitive array elements
    pub ptr: usize, // string bytes / primitive array data: offset into the data
}

/// a value slot (NRV_*)
#[derive(Clone, Copy, Default)]
pub struct Val {
    pub kind: u8,  // NRVK_*
    pub ptype: u8, // NRT_* (NRVK_PRIM)
    pub val: u64,  // primitive (sign / zero extended, float bits) or object index
}

// class metadata (NC_*)
#[derive(Clone, Copy, Default)]
struct Cls {
    name: usize,
    nlen: u32,
    nmem: u32,
    mem: u32, // first member description
    lib: i32, // -1 = system class
    typed: u32,
}

// member description (NM_*)
#[derive(Clone, Copy, Default)]
struct Mem {
    name: usize,
    nlen: u32,
    bt: u8, // BinaryTypeEnumeration (0xFF: untyped class)
    pt: u8, // primitive type (BT_PRIM / BT_PRIMARR)
    tname: usize,
    tlen: u32,
    tlib: i32,
}

// library (NL_*: the name is not kept)
#[derive(Clone, Copy, Default)]
struct Lib {
    id: i32,
}

/// a parsed stream (the assembly's NB context)
pub struct Nrbf<'a> {
    d: &'a [u8],
    pos: usize, // read position (after parsing: after MessageEnd)
    obj: Vec<Obj>,
    val: Vec<Val>,
    cls: Vec<Cls>,
    mem: Vec<Mem>,
    lib: Vec<Lib>,
    cap: [u32; 5], // the assembly's table capacities: obj, val, cls, mem, lib
    hash: Vec<u32>, // object index + 1 per slot (0 = empty)
    hmask: u32,
    depth: u32,
    rootid: i32,
    root: i32,
    maxmem: u64,
    used: u64,
    hdr: bool,
}

const T_OBJ: usize = 0;
const T_VAL: usize = 1;
const T_CLS: usize = 2;
const T_MEM: usize = 3;
const T_LIB: usize = 4;
const T_SIZE: [u64; 5] = [NRO_SIZE, NRV_SIZE, NC_SIZE, NM_SIZE, NL_SIZE];

#[inline]
fn rd32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

impl<'a> Nrbf<'a> {
    /// parse the stream at data (SerializationHeader first, MessageEnd last; what follows is
    /// not read); maxmem = byte budget for the tables (0 = only the NRBF_MAX* limits)
    /// [nrbf_parse]
    pub fn parse(data: &'a [u8], maxmem: u64) -> Result<Nrbf<'a>, i32> {
        let mut nb = Nrbf {
            d: data,
            pos: 0,
            obj: Vec::new(),
            val: Vec::new(),
            cls: Vec::new(),
            mem: Vec::new(),
            lib: Vec::new(),
            cap: [0; 5],
            hash: alloc::vec![0u32; 256],
            hmask: 255,
            depth: 0,
            rootid: 0,
            root: -1,
            maxmem,
            used: 1024,
            hdr: false,
        };
        loop {
            let r = nb.rec(RM_TOP, 0);
            if r < 0 {
                return Err(r);
            }
            if r != 0 {
                break;
            }
        }
        nb.resolve()?;
        if !nb.hdr {
            return Err(ERR_HDR);
        }
        let r = nb.lookup(nb.rootid);
        if r < 0 {
            return Err(ERR_HDR);
        }
        nb.root = r;
        Ok(nb)
    }

    // make table t hold at least `need` entries: capacity max(2 cap, need, 16), the added
    // bytes against the budget  [nb_grow]
    fn grow(&mut self, t: usize, need: u32) -> i32 {
        let cap = self.cap[t];
        if need <= cap {
            return 0;
        }
        let nc = (cap as u64 * 2).max(need as u64).max(16);
        let used = (nc - cap as u64) * T_SIZE[t] + self.used;
        if self.maxmem != 0 && used > self.maxmem {
            return ERR_LIMIT;
        }
        let add = nc as usize;
        match t {
            T_OBJ => self.obj.reserve(add - self.obj.len()),
            T_VAL => self.val.reserve(add - self.val.len()),
            T_CLS => self.cls.reserve(add - self.cls.len()),
            T_MEM => self.mem.reserve(add - self.mem.len()),
            _ => self.lib.reserve(add - self.lib.len()),
        }
        self.cap[t] = nc as u32;
        self.used = used;
        0
    }

    // the hash slot of id: its entry or the empty slot where it belongs  [nb_hash_slot]
    #[inline]
    fn hash_slot(&self, id: i32) -> usize {
        let mut h = (id as u32).wrapping_mul(0x9E37_79B1);
        h ^= h >> 15;
        let mut k = h & self.hmask;
        loop {
            let e = self.hash[k as usize];
            if e == 0 || self.obj[e as usize - 1].id == id {
                return k as usize;
            }
            k = (k + 1) & self.hmask;
        }
    }

    // object index of id, -1 when unknown  [nb_lookup]
    fn lookup(&self, id: i32) -> i32 {
        self.hash[self.hash_slot(id)] as i32 - 1
    }

    // double the hash table and insert every object again  [nb_rehash]
    fn rehash(&mut self) -> i32 {
        let nm = self.hmask * 2 + 1;
        let bytes = (nm as u64 + 1) * 4;
        let used = bytes + self.used;
        if self.maxmem != 0 && used > self.maxmem {
            return ERR_LIMIT;
        }
        self.used = used;
        self.hash = alloc::vec![0u32; nm as usize + 1];
        self.hmask = nm;
        for i in 0..self.obj.len() {
            let s = self.hash_slot(self.obj[i].id);
            self.hash[s] = i as u32 + 1;
        }
        0
    }

    // append an object (duplicate id -> ERR_HDR); index or ERR_*  [nb_addobj]
    fn addobj(&mut self, id: i32, kind: u8) -> i32 {
        let n = self.obj.len() as u32;
        if n >= NRBF_MAXOBJ {
            return ERR_LIMIT;
        }
        if (n + 1) * 2 > self.hmask {
            let r = self.rehash();
            if r < 0 {
                return r;
            }
        }
        let r = self.grow(T_OBJ, n + 1);
        if r < 0 {
            return r;
        }
        let s = self.hash_slot(id);
        if self.hash[s] != 0 {
            return ERR_HDR;
        }
        self.hash[s] = n + 1;
        self.obj.push(Obj { id, kind, ..Default::default() });
        n as i32
    }

    // n new zeroed value slots: index of the first or ERR_*  [nb_resv]
    fn resv(&mut self, n: u32) -> i64 {
        let first = self.val.len() as u32;
        let total = first as u64 + n as u64;
        if total > NRBF_MAXVAL {
            return ERR_LIMIT as i64;
        }
        let r = self.grow(T_VAL, total as u32);
        if r < 0 {
            return r as i64;
        }
        self.val.resize(total as usize, Val::default());
        first as i64
    }

    // a LengthPrefixedString at pos: (offset, length), pos advanced; None for a truncated
    // string or a bad 7-bit length (> 5 bytes or > 2^31 - 1)  [nb_lps]
    fn lps(&mut self) -> Option<(usize, u32)> {
        let d = self.d;
        let mut n: u64 = 0;
        let mut sh = 0u32;
        let mut p = self.pos;
        loop {
            if p >= d.len() {
                return None;
            }
            let b = d[p];
            p += 1;
            n |= ((b & 0x7F) as u64) << sh;
            if b & 0x80 == 0 {
                break;
            }
            sh += 7;
            if sh > 28 {
                return None;
            }
        }
        if n > 0x7FFF_FFFF || ((d.len() - p) as u64) < n {
            return None;
        }
        self.pos = p + n as usize;
        Some((p, n as u32))
    }

    // one primitive of type t at pos: value (integers sign / zero extended, floats as raw
    // bits, Char as its UTF-8 bytes, Decimal / String skipped -> 0), pos advanced  [nb_prim]
    fn prim(&mut self, t: u32) -> Result<u64, i32> {
        if t > NRT_STRING as u32 {
            return Err(ERR_HDR);
        }
        let sz = PTYPE[t as usize];
        let d = self.d;
        let p = self.pos;
        if sz == 0xFF {
            return Err(ERR_HDR);
        }
        if sz == 0xFE {
            if t == NRT_CHAR as u32 {
                if p >= d.len() {
                    return Err(ERR_TRUNC);
                }
                let lead = d[p];
                let n = if lead < 0xC0 {
                    1
                } else if lead < 0xE0 {
                    2
                } else if lead < 0xF0 {
                    3
                } else {
                    4
                };
                if d.len() - p < n {
                    return Err(ERR_TRUNC);
                }
                let mut v = 0u64;
                for k in 0..n {
                    v |= (d[p + k] as u64) << (k * 8);
                }
                self.pos = p + n;
                return Ok(v);
            }
            return match self.lps() {
                Some(_) => Ok(0),
                None => Err(ERR_TRUNC),
            };
        }
        let sz = sz as usize;
        if d.len() - p < sz {
            return Err(ERR_TRUNC);
        }
        let t = t as u8;
        let v = match sz {
            1 => {
                if t == NRT_I8 {
                    d[p] as i8 as i64 as u64
                } else {
                    d[p] as u64
                }
            }
            2 => {
                let x = u16::from_le_bytes([d[p], d[p + 1]]);
                if t == NRT_I16 { x as i16 as i64 as u64 } else { x as u64 }
            }
            4 => {
                let x = rd32(d, p);
                if t == NRT_I32 { x as i32 as i64 as u64 } else { x as u64 }
            }
            8 => u64::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3], d[p + 4], d[p + 5], d[p + 6], d[p + 7]]),
            _ => 0, // size 0 (Null)
        };
        self.pos = p + sz;
        Ok(v)
    }

    #[inline]
    fn rd8(&mut self) -> Option<u32> {
        let p = self.pos;
        if p >= self.d.len() {
            return None;
        }
        self.pos = p + 1;
        Some(self.d[p] as u32)
    }

    #[inline]
    fn rd32(&mut self) -> Option<u32> {
        let p = self.pos;
        if self.d.len() - p < 4 {
            return None;
        }
        self.pos = p + 4;
        Some(rd32(self.d, p))
    }

    fn findlib(&self, id: i32) -> i32 {
        self.lib.iter().position(|l| l.id == id).map_or(-1, |k| k as i32)
    }

    // read one record (recursive): < 0 ERR_* | RM_TOP: 1 = MessageEnd, 0 = any other record
    // | RM_MEMBER: 1 (value stored in slot) | RM_ELEM: slots filled from slot on (1, or n
    // for an ObjectNullMultiple; those slots are already null)  [nb_rec]
    fn rec(&mut self, mode: u32, slot: u32) -> i32 {
        self.depth += 1;
        let r = if self.depth > NRBF_MAXDEPTH { ERR_LIMIT } else { self.rec_body(mode, slot) };
        self.depth -= 1;
        r
    }

    fn rec_body(&mut self, mode: u32, slot: u32) -> i32 {
        macro_rules! rd8 {
            () => {
                match self.rd8() {
                    Some(v) => v,
                    None => return ERR_TRUNC,
                }
            };
        }
        macro_rules! rd32 {
            () => {
                match self.rd32() {
                    Some(v) => v,
                    None => return ERR_TRUNC,
                }
            };
        }
        loop {
            let rt = rd8!();
            match rt {
                // ---- 0 SerializationHeader
                0 => {
                    if mode != RM_TOP || self.hdr {
                        return ERR_HDR;
                    }
                    if self.d.len() - self.pos < 16 {
                        return ERR_TRUNC;
                    }
                    self.rootid = rd32(self.d, self.pos) as i32;
                    self.pos += 16;
                    self.hdr = true;
                    return 0;
                }
                // ---- 12 BinaryLibrary
                12 => {
                    if !self.hdr {
                        return ERR_HDR;
                    }
                    let id = rd32!() as i32;
                    if self.lps().is_none() {
                        return ERR_TRUNC;
                    }
                    if self.findlib(id) >= 0 {
                        return ERR_HDR;
                    }
                    let n = self.lib.len() as u32;
                    if n >= NRBF_MAXLIB {
                        return ERR_LIMIT;
                    }
                    let r = self.grow(T_LIB, n + 1);
                    if r < 0 {
                        return r;
                    }
                    self.lib.push(Lib { id });
                    if mode != RM_TOP {
                        continue; // a library may precede a nested value
                    }
                    return 0;
                }
                // ---- 11 MessageEnd
                11 => {
                    if mode != RM_TOP || !self.hdr {
                        return ERR_HDR;
                    }
                    return 1;
                }
                // ---- 2 / 3 / 4 / 5 class records with metadata
                2..=5 => return self.rec_class(mode, slot, rt),
                // ---- 1 ClassWithId
                1 => {
                    if !self.hdr {
                        return ERR_HDR;
                    }
                    let id = rd32!() as i32;
                    let mid = rd32!() as i32;
                    let m = self.lookup(mid);
                    if m < 0 {
                        return ERR_HDR;
                    }
                    let mo = self.obj[m as usize];
                    if mo.kind != NRK_CLASS {
                        return ERR_HDR;
                    }
                    let o = self.addobj(id, NRK_CLASS);
                    if o < 0 {
                        return o;
                    }
                    return self.rec_members(mode, slot, o as u32, mo.cls);
                }
                // ---- 6 BinaryObjectString
                6 => {
                    if !self.hdr {
                        return ERR_HDR;
                    }
                    let id = rd32!() as i32;
                    let Some((p, n)) = self.lps() else { return ERR_TRUNC };
                    let o = self.addobj(id, NRK_STRING);
                    if o < 0 {
                        return o;
                    }
                    let e = &mut self.obj[o as usize];
                    e.ptr = p;
                    e.len = n;
                    return self.store(mode, slot, o as u32);
                }
                // ---- 8 MemberPrimitiveTyped
                8 => {
                    if mode == RM_TOP {
                        return ERR_HDR;
                    }
                    let t = rd8!();
                    let v = match self.prim(t) {
                        Ok(v) => v,
                        Err(e) => return e,
                    };
                    self.val[slot as usize] = Val { kind: NRVK_PRIM, ptype: t as u8, val: v };
                    return 1;
                }
                // ---- 9 MemberReference
                9 => {
                    if mode == RM_TOP {
                        return ERR_HDR;
                    }
                    let id = rd32!() as i32;
                    self.val[slot as usize] = Val { kind: NRVK_REF, ptype: 0, val: id as i64 as u64 };
                    return 1;
                }
                // ---- 10 ObjectNull
                10 => {
                    if mode == RM_TOP {
                        return ERR_HDR;
                    }
                    return 1; // the slot is already null
                }
                // ---- 13 / 14 ObjectNullMultiple(256)
                13 | 14 => {
                    if mode != RM_ELEM {
                        return ERR_HDR;
                    }
                    let n = if rt == 13 { rd8!() } else { rd32!() } as i32;
                    if n <= 0 {
                        return ERR_HDR; // 0 or negative counts
                    }
                    return n;
                }
                // ---- 15 ArraySinglePrimitive
                15 => {
                    if !self.hdr {
                        return ERR_HDR;
                    }
                    let id = rd32!() as i32;
                    let n = rd32!() as i32;
                    if n < 0 {
                        return ERR_HDR;
                    }
                    let t = rd8!();
                    return self.rec_primarr(mode, slot, id, n as u32, t);
                }
                // ---- 16 / 17 ArraySingleObject / ArraySingleString
                16 | 17 => {
                    if !self.hdr {
                        return ERR_HDR;
                    }
                    let id = rd32!() as i32;
                    let n = rd32!() as i32;
                    if n < 0 {
                        return ERR_HDR;
                    }
                    return self.rec_objarr(mode, slot, id, n as u32);
                }
                // ---- 7 BinaryArray
                7 => {
                    if !self.hdr {
                        return ERR_HDR;
                    }
                    let id = rd32!() as i32;
                    let kind = rd8!();
                    if kind > 5 {
                        return ERR_HDR;
                    }
                    let rank = rd32!();
                    if rank.wrapping_sub(1) > NRBF_MAXRANK - 1 {
                        return ERR_HDR;
                    }
                    let mut count: u64 = 1; // product of the lengths
                    for _ in 0..rank {
                        let l = rd32!() as i32;
                        if l < 0 {
                            return ERR_HDR;
                        }
                        count *= l as u64; // <= 2^22 * 2^31: no overflow
                        if count > NRBF_MAXVAL {
                            return ERR_LIMIT;
                        }
                    }
                    if kind >= 3 {
                        // *Offset kinds: rank lower bounds follow
                        let n = rank as usize * 4;
                        if self.d.len() - self.pos < n {
                            return ERR_TRUNC;
                        }
                        self.pos += n;
                    }
                    let bt = rd8!() as u8;
                    if bt > BT_PRIMARR {
                        return ERR_HDR;
                    }
                    match bt {
                        BT_PRIM => {
                            let t = rd8!();
                            return self.rec_primarr(mode, slot, id, count as u32, t);
                        }
                        BT_PRIMARR => {
                            rd8!();
                        }
                        BT_SYSCLASS => {
                            if self.lps().is_none() {
                                return ERR_TRUNC;
                            }
                        }
                        BT_CLASS => {
                            if self.lps().is_none() {
                                return ERR_TRUNC;
                            }
                            rd32!();
                        }
                        _ => {}
                    }
                    return self.rec_objarr(mode, slot, id, count as u32);
                }
                21 | 22 => return ERR_UNSUPPORTED, // MethodCall / MethodReturn
                _ => return ERR_HDR,
            }
        }
    }

    // records 2..5: class metadata, then the member values  [nb_rec .nrc_class]
    fn rec_class(&mut self, mode: u32, slot: u32, rt: u32) -> i32 {
        if !self.hdr {
            return ERR_HDR;
        }
        let Some(oid) = self.rd32() else { return ERR_TRUNC };
        let oid = oid as i32; // object id
        let Some((name, nlen)) = self.lps() else { return ERR_TRUNC };
        let Some(nm) = self.rd32() else { return ERR_TRUNC };
        if nm > NRBF_MAXMCOUNT {
            return ERR_HDR; // also negative counts
        }
        if ((self.d.len() - self.pos) as u64) < nm as u64 {
            return ERR_TRUNC; // every member name takes >= 1 byte
        }
        let ci = self.cls.len() as u32;
        if ci >= NRBF_MAXCLS {
            return ERR_LIMIT;
        }
        let r = self.grow(T_CLS, ci + 1);
        if r < 0 {
            return r;
        }
        let m0 = self.mem.len() as u32;
        let mt = m0 as u64 + nm as u64;
        if mt > NRBF_MAXMEMB {
            return ERR_LIMIT;
        }
        let r = self.grow(T_MEM, mt as u32);
        if r < 0 {
            return r;
        }
        self.mem.resize(mt as usize, Mem::default());
        self.cls.push(Cls { name, nlen, nmem: nm, mem: m0, lib: -1, typed: 0 });
        // member names
        for k in 0..nm {
            let Some((p, n)) = self.lps() else { return ERR_TRUNC };
            let m = &mut self.mem[(m0 + k) as usize];
            m.name = p;
            m.nlen = n;
            m.bt = 0xFF;
        }
        if rt >= 4 {
            // BinaryTypeEnumeration per member, then the additional infos
            if ((self.d.len() - self.pos) as u64) < nm as u64 {
                return ERR_TRUNC;
            }
            for k in 0..nm {
                let b = self.d[self.pos];
                self.pos += 1;
                if b > BT_PRIMARR {
                    return ERR_HDR;
                }
                self.mem[(m0 + k) as usize].bt = b;
            }
            for k in 0..nm {
                let mi = (m0 + k) as usize;
                match self.mem[mi].bt {
                    BT_PRIM | BT_PRIMARR => {
                        let Some(t) = self.rd8() else { return ERR_TRUNC };
                        self.mem[mi].pt = t as u8;
                        if t > NRT_STRING as u32 || PTYPE[t as usize] == 0xFF {
                            return ERR_HDR;
                        }
                    }
                    BT_SYSCLASS => {
                        let Some((p, n)) = self.lps() else { return ERR_TRUNC };
                        let m = &mut self.mem[mi];
                        m.tname = p;
                        m.tlen = n;
                        m.tlib = -1;
                    }
                    BT_CLASS => {
                        let Some((p, n)) = self.lps() else { return ERR_TRUNC };
                        let m = &mut self.mem[mi];
                        m.tname = p;
                        m.tlen = n;
                        let Some(l) = self.rd32() else { return ERR_TRUNC };
                        self.mem[mi].tlib = l as i32;
                    }
                    _ => {}
                }
            }
            self.cls[ci as usize].typed = 1;
        }
        if rt == 3 || rt == 5 {
            let Some(l) = self.rd32() else { return ERR_TRUNC };
            if self.findlib(l as i32) < 0 {
                return ERR_HDR;
            }
            self.cls[ci as usize].lib = l as i32;
        }
        let o = self.addobj(oid, NRK_CLASS);
        if o < 0 {
            return o;
        }
        self.rec_members(mode, slot, o as u32, ci)
    }

    // the member values of object o with class ci, then store it  [nb_rec .nrc_members]
    fn rec_members(&mut self, mode: u32, slot: u32, o: u32, ci: u32) -> i32 {
        let c = self.cls[ci as usize];
        let first = self.resv(c.nmem);
        if first < 0 {
            return first as i32;
        }
        let first = first as u32;
        let e = &mut self.obj[o as usize];
        e.val = first;
        e.nval = c.nmem;
        e.cls = ci;
        for k in 0..c.nmem {
            let m = self.mem[(c.mem + k) as usize];
            if m.bt == BT_PRIM {
                let v = match self.prim(m.pt as u32) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                self.val[(first + k) as usize] = Val { kind: NRVK_PRIM, ptype: m.pt, val: v };
            } else {
                let r = self.rec(RM_MEMBER, first + k);
                if r < 0 {
                    return r;
                }
            }
        }
        self.store(mode, slot, o)
    }

    // a primitive array: id, count n, element type t  [nb_rec .nrc_primarr]
    fn rec_primarr(&mut self, mode: u32, slot: u32, id: i32, n: u32, t: u32) -> i32 {
        if t > NRT_STRING as u32 || t == NRT_NULL as u32 {
            return ERR_HDR; // Null elements take no bytes: refuse
        }
        let sz = PTYPE[t as usize];
        if sz == 0xFF {
            return ERR_HDR;
        }
        let start = self.pos;
        if sz == 0xFE {
            for _ in 0..n {
                if let Err(e) = self.prim(t) {
                    return e; // every element takes >= 1 byte
                }
            }
        } else {
            let bytes = n as u64 * sz as u64;
            if ((self.d.len() - self.pos) as u64) < bytes {
                return ERR_TRUNC;
            }
            self.pos += bytes as usize;
        }
        let o = self.addobj(id, NRK_PRIMARR);
        if o < 0 {
            return o;
        }
        let e = &mut self.obj[o as usize];
        e.ptype = t as u8;
        e.len = n;
        e.ptr = start;
        self.store(mode, slot, o as u32)
    }

    // an array of records: id, count n  [nb_rec .nrc_objarr]
    fn rec_objarr(&mut self, mode: u32, slot: u32, id: i32, n: u32) -> i32 {
        if n as u64 > NRBF_MAXVAL {
            return ERR_LIMIT;
        }
        let o = self.addobj(id, NRK_ARRAY);
        if o < 0 {
            return o;
        }
        let first = self.resv(n);
        if first < 0 {
            return first as i32;
        }
        let first = first as u32;
        let e = &mut self.obj[o as usize];
        e.val = first;
        e.nval = n;
        let mut i = 0u32;
        while i < n {
            let r = self.rec(RM_ELEM, first + i);
            if r < 0 {
                return r;
            }
            i += r as u32; // < 2^22 + 2^31: no carry
            if i > n {
                return ERR_HDR; // null run past the end
            }
        }
        self.store(mode, slot, o as u32)
    }

    // the object o into the caller's slot (RM_MEMBER / RM_ELEM: 1; RM_TOP: 0)  [.nrc_store]
    fn store(&mut self, mode: u32, slot: u32, o: u32) -> i32 {
        if mode == RM_TOP {
            return 0;
        }
        self.val[slot as usize] = Val { kind: NRVK_OBJ, ptype: 0, val: o as u64 };
        1
    }

    // every NRVK_REF slot -> NRVK_OBJ  [nb_resolve]
    fn resolve(&mut self) -> Result<(), i32> {
        for i in 0..self.val.len() {
            if self.val[i].kind == NRVK_REF {
                let o = self.lookup(self.val[i].val as i32);
                if o < 0 {
                    return Err(ERR_HDR);
                }
                self.val[i] = Val { kind: NRVK_OBJ, ptype: 0, val: o as u64 };
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ accessors

    /// offset after MessageEnd  [nrbf_end]
    pub fn end(&self) -> usize {
        self.pos
    }

    /// root object index (-1 none)  [nrbf_root]
    pub fn root(&self) -> i32 {
        self.root
    }

    /// number of objects  [nrbf_nobj]
    pub fn nobj(&self) -> u32 {
        self.obj.len() as u32
    }

    /// the entry of object ix (None out of range)  [nrbf_obj]
    pub fn obj(&self, ix: i32) -> Option<&Obj> {
        if ix < 0 { None } else { self.obj.get(ix as usize) }
    }

    /// class name of object ix (None: not a class instance)  [nrbf_class_name]
    pub fn class_name(&self, ix: i32) -> Option<&'a [u8]> {
        let o = self.obj(ix)?;
        if o.kind != NRK_CLASS {
            return None;
        }
        let c = &self.cls[o.cls as usize];
        Some(&self.d[c.name..c.name + c.nlen as usize])
    }

    /// object ix is an instance of the class `name` (exact match)  [nrbf_is]
    pub fn is(&self, ix: i32, name: &[u8]) -> bool {
        self.class_name(ix) == Some(name)
    }

    /// the value slot of member `name` of class object ix  [nrbf_member]
    pub fn member(&self, ix: i32, name: &[u8]) -> Option<&Val> {
        let o = self.obj(ix)?;
        if o.kind != NRK_CLASS {
            return None;
        }
        let c = &self.cls[o.cls as usize];
        let n = c.nmem.min(o.nval);
        for k in 0..n {
            let m = &self.mem[(c.mem + k) as usize];
            if m.nlen as usize == name.len() && &self.d[m.name..m.name + m.nlen as usize] == name {
                return Some(&self.val[(o.val + k) as usize]);
            }
        }
        None
    }

    /// object index of member `name`, -1 when absent, null or a primitive  [nrbf_get_obj]
    pub fn get_obj(&self, ix: i32, name: &[u8]) -> i32 {
        match self.member(ix, name) {
            Some(v) if v.kind == NRVK_OBJ => v.val as u32 as i32,
            _ => -1,
        }
    }

    /// member `name` as an integer / Boolean primitive (sign / zero extended)  [nrbf_get_int]
    pub fn get_int(&self, ix: i32, name: &[u8]) -> Option<u64> {
        let v = self.member(ix, name)?;
        if v.kind != NRVK_PRIM || v.ptype > NRT_STRING || PINT[v.ptype as usize] == 0 {
            return None;
        }
        Some(v.val)
    }

    /// string object ix  [nrbf_str]
    pub fn str_(&self, ix: i32) -> Option<&'a [u8]> {
        let o = self.obj(ix)?;
        if o.kind != NRK_STRING {
            return None;
        }
        Some(&self.d[o.ptr..o.ptr + o.len as usize])
    }

    /// the string member `name`  [nrbf_get_str]
    pub fn get_str(&self, ix: i32, name: &[u8]) -> Option<&'a [u8]> {
        let o = self.get_obj(ix, name);
        if o < 0 { None } else { self.str_(o) }
    }

    /// elements of array object ix (-1 when ix is not an array)  [nrbf_count]
    pub fn count(&self, ix: i32) -> i64 {
        match self.obj(ix) {
            Some(o) if o.kind == NRK_ARRAY => o.nval as i64,
            Some(o) if o.kind == NRK_PRIMARR => o.len as i64,
            _ => -1,
        }
    }

    /// the value slot of element i of array object ix  [nrbf_elem]
    pub fn elem(&self, ix: i32, i: u32) -> Option<&Val> {
        let o = self.obj(ix)?;
        if o.kind != NRK_ARRAY || i >= o.nval {
            return None;
        }
        Some(&self.val[(o.val + i) as usize])
    }

    /// element i as an object index (-1: none / null / primitive)  [nrbf_elem_obj]
    pub fn elem_obj(&self, ix: i32, i: u32) -> i32 {
        match self.elem(ix, i) {
            Some(v) if v.kind == NRVK_OBJ => v.val as u32 as i32,
            _ => -1,
        }
    }

    /// raw data (from the start of the elements to the end of the stream data) and element
    /// type of primitive array ix; the element count is count(ix)  [nrbf_prim_data]
    pub fn prim_data(&self, ix: i32) -> Option<(&'a [u8], u8)> {
        let o = self.obj(ix)?;
        if o.kind != NRK_PRIMARR {
            return None;
        }
        Some((&self.d[o.ptr..], o.ptype))
    }

    /// true when object ix was not visited before (marks it)  [nrbf_visit]
    pub fn visit(&mut self, ix: i32) -> bool {
        if ix < 0 {
            return false;
        }
        match self.obj.get_mut(ix as usize) {
            Some(o) if o.flags & NROF_VISITED == 0 => {
                o.flags |= NROF_VISITED;
                true
            }
            _ => false,
        }
    }
}

// ================================================================== writer

pub const NW_MAXLIB: usize = 8;
pub const NW_MAXSLOT: usize = 48;

/// the NRBF writer (NW record): output buffer, sticky error, next object id, library ids
/// given out, first object id per template class slot
pub struct Nw {
    pub buf: Vec<u8>,
    pub err: i32, // sticky (ERR_LIMIT / ERR_UNSUPPORTED)
    next: i32,
    libid: [i32; NW_MAXLIB],
    first: [i32; NW_MAXSLOT],
}

impl Nw {
    /// an empty writer with a 64 KB buffer  [nw_init]
    pub fn new() -> Nw {
        Nw { buf: Vec::with_capacity(65536), err: 0, next: 1, libid: [0; NW_MAXLIB], first: [0; NW_MAXSLOT] }
    }

    /// the buffer, or None when an error happened  [nw_take]
    pub fn take(self) -> Option<Vec<u8>> {
        if self.err != 0 { None } else { Some(self.buf) }
    }

    /// n new bytes at the end of the output (zeroed; None after an error)  [nw_reserve]
    pub fn reserve(&mut self, n: usize) -> Option<&mut [u8]> {
        if self.err != 0 {
            return None;
        }
        let l = self.buf.len();
        if l.checked_add(n).is_none() {
            self.err = ERR_LIMIT;
            return None;
        }
        self.buf.resize(l + n, 0);
        Some(&mut self.buf[l..])
    }

    /// append bytes  [nw_bytes]
    pub fn bytes(&mut self, src: &[u8]) {
        if self.err == 0 {
            self.buf.extend_from_slice(src);
        }
    }

    /// [nw_u8]
    pub fn u8(&mut self, v: u32) {
        self.bytes(&[v as u8]);
    }

    /// little-endian 32-bit  [nw_i32]
    pub fn i32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    /// big-endian 32-bit  [nw_be32]
    pub fn be32(&mut self, v: u32) {
        self.bytes(&v.to_be_bytes());
    }

    /// little-endian 64-bit  [nw_i64]
    pub fn i64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    /// LengthPrefixedString (7-bit encoded length, then the bytes)  [nw_lps]
    pub fn lps(&mut self, s: &[u8]) {
        let mut h = [0u8; 10];
        let mut v = s.len() as u64;
        let mut n = 0;
        loop {
            let mut b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                h[n] = b;
                n += 1;
                break;
            }
            b |= 0x80;
            h[n] = b;
            n += 1;
            if n >= 9 {
                h[n] = b; // (lengths >= 2^63: the assembly repeats the last byte)
                n += 1;
                break;
            }
        }
        self.bytes(&h[..n]);
        self.bytes(s);
    }

    /// the next object id (every reference consumes one, like .NET's writer)  [nw_id]
    pub fn id(&mut self) -> i32 {
        let v = self.next;
        self.next = v.wrapping_add(1);
        v
    }

    /// SerializationHeader {root, -1, 1, 0}  [nw_header]
    pub fn header(&mut self, root: i32) {
        let mut h = [0u8; 17];
        h[1..5].copy_from_slice(&root.to_le_bytes());
        h[5..9].copy_from_slice(&(-1i32).to_le_bytes());
        h[9..13].copy_from_slice(&1i32.to_le_bytes());
        self.bytes(&h);
    }

    /// MemberReference  [nw_ref]
    pub fn ref_(&mut self, id: i32) {
        let mut h = [9u8; 5];
        h[1..].copy_from_slice(&id.to_le_bytes());
        self.bytes(&h);
    }

    /// n nulls: nothing, ObjectNull, ObjectNullMultiple256 or ObjectNullMultiple  [nw_nulls]
    pub fn nulls(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        if n == 1 {
            self.bytes(&[10]);
        } else if n < 256 {
            self.bytes(&[13, n as u8]);
        } else {
            let mut h = [14u8; 5];
            h[1..].copy_from_slice(&n.to_le_bytes());
            self.bytes(&h);
        }
    }

    /// MessageEnd  [nw_end]
    pub fn end(&mut self) {
        self.u8(11);
    }

    /// BinaryObjectString with the next id -> the id  [nw_str]
    pub fn str_(&mut self, s: &[u8]) -> i32 {
        let id = self.id();
        let mut h = [6u8; 5];
        h[1..].copy_from_slice(&id.to_le_bytes());
        self.bytes(&h);
        self.lps(s);
        id
    }

    /// class record of object `id` from the template: the first instance of the template's
    /// slot gets BinaryLibrary records for the libraries not written yet and the full record
    /// with the library ids patched in; later ones a ClassWithId(id, first id)  [nw_class]
    pub fn class(&mut self, t: &Tpl, id: i32) {
        let slot = t.slot as usize;
        if slot >= NW_MAXSLOT {
            self.err = ERR_UNSUPPORTED;
            return;
        }
        let f = self.first[slot];
        if f != 0 {
            let mut h = [1u8; 9];
            h[1..5].copy_from_slice(&id.to_le_bytes());
            h[5..9].copy_from_slice(&f.to_le_bytes());
            self.bytes(&h);
            return;
        }
        self.first[slot] = id;
        for &(_, lib) in t.patches {
            let lib = lib as usize;
            if lib >= NW_MAXLIB {
                self.err = ERR_UNSUPPORTED;
                return;
            }
            if self.libid[lib] == 0 {
                let lid = self.id();
                self.libid[lib] = lid;
                let mut h = [12u8; 5];
                h[1..].copy_from_slice(&lid.to_le_bytes());
                self.bytes(&h);
                self.bytes(LIBS[lib]);
            }
        }
        let mut h = [t.rt; 5];
        h[1..].copy_from_slice(&id.to_le_bytes());
        self.bytes(&h);
        let libid = self.libid;
        let Some(body) = self.reserve(t.body.len()) else { return };
        body.copy_from_slice(t.body);
        let mut bad = false;
        for &(off, lib) in t.patches {
            let o = off as usize;
            if o + 4 > body.len() {
                bad = true;
                break;
            }
            body[o..o + 4].copy_from_slice(&libid[lib as usize].to_le_bytes());
        }
        if bad {
            self.err = ERR_UNSUPPORTED;
        }
    }
}

impl Default for Nw {
    fn default() -> Self {
        Self::new()
    }
}

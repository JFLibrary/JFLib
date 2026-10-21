// bmff -- ISO base media file format reader for HEIF / AVIF still images
//                                            [src/img/isobmff.s, ASMLIB/inc/isobmff.inc]
//
//   Bx::parse(data) -> Bx / ERR_*        the file's item records  [isobmff_parse]
//   find(id) -> index or -1              the first item with that id  [isobmff_find]
//   item(id) -> ItemData / ERR_*         the data of an item: a slice of the file for one
//                                        extent, else the extents joined  [isobmff_item]
//   prop(id, type) -> payload / None     the first property of that type associated with
//                                        the item  [isobmff_prop]
//   assoc(index, k) -> (Prop, essential) / None   the k-th association of item index
//                                        (a zero record for an invalid property index)
//                                        [isobmff_assoc]
//   ref_(type, from, k) -> id or -1      the k-th to_item_ID of the references of that type
//                                        from item from (iref order)  [isobmff_ref]
//   rref(type, to, k) -> id or -1        the from_item_ID of the k-th reference of that type
//                                        that points to item to  [isobmff_rref]
//
// The file must start with ftyp and contain a top-level meta box (ERR_HDR otherwise); of
// meta's children the first hdlr / pitm / iloc / iinf (required) / iref / iprp / idat count.
// iinf: infe versions 0..3 (BMFF_ITEM_MAX items).  iloc: versions 0..2, field sizes 0 / 4 /
// 8, extents resolved to file offsets (method 1 through idat) and checked; a length of 0
// means "to the end of the file / idat".  iprp: the first ipco, every ipma (an item keeps
// its first association list).  iref: versions 0 / 1, every reference as one (type, from,
// to) record.  Box sizes 0 (to the end of the parent) and 64-bit sizes are understood; a
// box running past its parent is cut.  Malformed iinf / iloc / ipma: ERR_HDR; iref / ipco end
// at the first bad box.  All offsets are file offsets; four-CCs are compared as stored
// (little-endian load of the four bytes): TAG_* below.  Ids of 2^31 and more read as "none"
// (-1) where the assembly tests the sign.

use alloc::vec::Vec;

use img_core::{ERR_HDR, ERR_LIMIT, ERR_TRUNC, ERR_UNSUPPORTED};

// ------------------------------------------------------------------ limits
const BMFF_ITEM_MAX: u32 = 8192; // items in iinf (ERR_LIMIT beyond)
const BMFF_EXT_MAX: u32 = 1 << 20; // extents in iloc
const BMFF_PROP_MAX: u64 = 32767; // properties in ipco (15-bit ipma index)
const BMFF_TOP_MAX: u32 = 4096; // top-level boxes looked at
const BMFF_META_MAX: u32 = 1024; // children of meta looked at
const BMFF_DATA_MAX: u64 = 0x7FFF_FFFF; // reassembled item data (ERR_LIMIT beyond)

// ------------------------------------------------------------------ construction methods (Item::cm)
pub(crate) const CM_FILE: u32 = 0; // extents are file offsets
pub(crate) const CM_IDAT: u32 = 1; // extents are offsets in idat (resolved to file offsets)
pub(crate) const CM_ITEM: u32 = 2; // item offsets (not supported: ERR_UNSUPPORTED)
pub(crate) const CM_EXTERN: u32 = 0xFD; // data_reference_index != 0 / unknown method
pub(crate) const CM_TRUNC: u32 = 0xFE; // an extent lies outside the file / idat (ERR_TRUNC)
pub(crate) const CM_NONE: u32 = 0xFF; // no iloc entry (ERR_HDR)

// ------------------------------------------------------------------ four-CCs (as stored)
pub const TAG_FTYP: u32 = 0x7079_7466; // "ftyp"
pub const TAG_META: u32 = 0x6174_656D; // "meta"
pub const TAG_HDLR: u32 = 0x726C_6468; // "hdlr"
pub const TAG_PITM: u32 = 0x6D74_6970; // "pitm"
pub const TAG_ILOC: u32 = 0x636F_6C69; // "iloc"
pub const TAG_IINF: u32 = 0x666E_6969; // "iinf"
pub const TAG_IREF: u32 = 0x6665_7269; // "iref"
pub const TAG_IPRP: u32 = 0x7072_7069; // "iprp"
pub const TAG_IDAT: u32 = 0x7461_6469; // "idat"
pub const TAG_INFE: u32 = 0x6566_6E69; // "infe"
pub const TAG_IPCO: u32 = 0x6F63_7069; // "ipco"
pub const TAG_IPMA: u32 = 0x616D_7069; // "ipma"
pub const TAG_MIME: u32 = 0x656D_696D; // "mime"
pub const TAG_URI: u32 = 0x2069_7275; // "uri "
pub const TAG_PICT: u32 = 0x7463_6970; // "pict"
pub const TAG_HVC1: u32 = 0x3163_7668; // "hvc1"
pub const TAG_HEV1: u32 = 0x3176_6568; // "hev1"
pub const TAG_HVCC: u32 = 0x4363_7668; // "hvcC"
pub const TAG_GRID: u32 = 0x6469_7267; // "grid"
pub const TAG_ISPE: u32 = 0x6570_7369; // "ispe"
pub const TAG_IROT: u32 = 0x746F_7269; // "irot"
pub const TAG_IMIR: u32 = 0x7269_6D69; // "imir"
pub const TAG_CLAP: u32 = 0x7061_6C63; // "clap"
pub const TAG_COLR: u32 = 0x726C_6F63; // "colr"
pub const TAG_NCLX: u32 = 0x786C_636E; // "nclx"
pub const TAG_PROF: u32 = 0x666F_7270; // "prof"
pub const TAG_RICC: u32 = 0x4343_4972; // "rICC"
pub const TAG_AUXC: u32 = 0x4378_7561; // "auxC"
pub const TAG_PIXI: u32 = 0x6978_6970; // "pixi"
pub const TAG_AUXL: u32 = 0x6C78_7561; // "auxl"
pub const TAG_DIMG: u32 = 0x676D_6964; // "dimg"
pub const TAG_CDSC: u32 = 0x6373_6463; // "cdsc"
pub const TAG_PREM: u32 = 0x6D65_7270; // "prem"
pub const TAG_EXIF: u32 = 0x6669_7845; // "Exif"
pub const TAG_AV01: u32 = 0x3130_7661; // "av01"
pub const TAG_AV1C: u32 = 0x4331_7661; // "av1C"
pub const TAG_UUID: u32 = 0x6469_7575; // "uuid"

// meta children collected (bx_meta_tags)
const CH_HDLR: usize = 0;
const CH_PITM: usize = 1;
const CH_ILOC: usize = 2;
const CH_IINF: usize = 3;
const CH_IREF: usize = 4;
const CH_IPRP: usize = 5;
const CH_IDAT: usize = 6;
static META_TAGS: [u32; 7] = [TAG_HDLR, TAG_PITM, TAG_ILOC, TAG_IINF, TAG_IREF, TAG_IPRP, TAG_IDAT];

/// an item (BI; flags / prot are kept as in the record, the reader does not use them)
#[derive(Clone, Copy, Default)]
#[allow(dead_code)]
pub(crate) struct Item {
    pub id: u32,
    pub ty: u32,    // item_type (infe version >= 2; 0 for version 0 / 1)
    pub flags: u32, // infe flags (bit 0: hidden)
    pub cm: u32,    // construction method 0 / 1 or CM_*
    pub next: u32,  // extents of the item
    pub ext0: u32,  // first extent (index into exts)
    pub nassoc: u32, // property associations
    pub assoc0: u32, // first association (index into assoc)
    pub ctype: u64, // offset of content_type (mime) / item_uri_type (uri ), 0 = none
    pub ctlen: u32, // its length (no NUL)
    pub prot: u32,  // protection index (0 = not protected)
}

/// a property in ipco (BP)
#[derive(Clone, Copy, Default)]
pub(crate) struct Prop {
    pub ty: u32,  // box type
    pub off: u64, // payload offset (after the box header; FullBoxes start with version / flags)
    pub len: u64, // payload length
}

/// a reference in iref (BR)
#[derive(Clone, Copy)]
pub(crate) struct Ref {
    ty: u32,
    from: u32,
    to: u32,
}

/// the data of an item: a slice of the file, or the extents joined
pub(crate) enum ItemData<'a> {
    File(&'a [u8]),
    Joined(Vec<u8>),
}

impl core::ops::Deref for ItemData<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            ItemData::File(s) => s,
            ItemData::Joined(v) => v,
        }
    }
}

/// the parser context (BX)
pub(crate) struct Bx<'a> {
    pub data: &'a [u8],
    pub primary: u32, // pitm item id (0 = no pitm)
    pub items: Vec<Item>,
    pub exts: Vec<(u64, u64)>, // {offset, length}
    pub props: Vec<Prop>,      // ipma indices 1..len
    pub assoc: Vec<u32>,       // property index (1-based, 0 = none) | essential << 31
    refs: Vec<Ref>,            // iref order
    pub idat: u64,             // offset of the idat payload (0 = none)
    pub idatlen: u64,
    pub major: u32, // ftyp major brand
    pub hdlr: u32,  // hdlr handler type (TAG_PICT for images)
    pub ftyp: u64,  // offset of the ftyp payload (brands from +8)
    pub ftyplen: u64,
}

#[inline]
fn be16(d: &[u8], o: u64) -> u32 {
    let o = o as usize;
    u16::from_be_bytes([d[o], d[o + 1]]) as u32
}

#[inline]
fn be32(d: &[u8], o: u64) -> u32 {
    let o = o as usize;
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

#[inline]
fn le32(d: &[u8], o: u64) -> u32 {
    let o = o as usize;
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// bx_box: the box at pos (end = parent end): (size incl. header, type, header size 8 / 16,
/// +16 for uuid); None when no complete box header lies there; size 0 extends to end, a
/// box running past end is cut at end
fn bbox(d: &[u8], end: u64, pos: u64) -> Option<(u64, u32, u64)> {
    if pos > end {
        return None;
    }
    let left = end - pos;
    if left < 8 {
        return None;
    }
    let mut size = be32(d, pos) as u64;
    let ty = le32(d, pos + 4);
    let mut hdr = 8u64;
    if size == 1 {
        if left < 16 {
            return None;
        }
        let o = pos as usize + 8;
        size = u64::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3], d[o + 4], d[o + 5], d[o + 6], d[o + 7]]);
        hdr = 16;
    } else if size == 0 {
        size = left;
    }
    if ty == TAG_UUID {
        hdr += 16;
    }
    if size < hdr || left < hdr {
        return None;
    }
    Some((size.min(left), ty, hdr))
}

/// bx_cstr: offset of the NUL that ends the string at pos, or end when there is none
fn cstr(d: &[u8], pos: u64, end: u64) -> u64 {
    let mut p = pos;
    while p < end && d[p as usize] != 0 {
        p += 1;
    }
    p
}

/// bx_rdn: big-endian value of n bytes (n = 0 / 1 / 2 / 4 / 8) at pos and pos + n; None
/// when they do not fit before end
fn rdn(d: &[u8], pos: u64, n: u32, end: u64) -> Option<(u64, u64)> {
    let e = pos.checked_add(n as u64)?;
    if e > end {
        return None;
    }
    let v = match n {
        0 => 0,
        1 => d[pos as usize] as u64,
        2 => be16(d, pos) as u64,
        4 => be32(d, pos) as u64,
        _ => {
            let o = pos as usize;
            u64::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3], d[o + 4], d[o + 5], d[o + 6], d[o + 7]])
        }
    };
    Some((v, e))
}

impl<'a> Bx<'a> {
    /// isobmff_parse: parse the file into the item records (see the file header)
    pub fn parse(data: &'a [u8]) -> Result<Bx<'a>, i32> {
        let mut bx = Bx {
            data,
            primary: 0,
            items: Vec::new(),
            exts: Vec::new(),
            props: Vec::new(),
            assoc: Vec::new(),
            refs: Vec::new(),
            idat: 0,
            idatlen: 0,
            major: 0,
            hdlr: 0,
            ftyp: 0,
            ftyplen: 0,
        };
        let len = data.len() as u64;
        // ftyp first
        let Some((size, ty, hdr)) = bbox(data, len, 0) else { return Err(ERR_HDR) };
        if ty != TAG_FTYP || size < hdr + 8 {
            return Err(ERR_HDR);
        }
        bx.major = le32(data, hdr);
        bx.ftyp = hdr;
        bx.ftyplen = size - hdr;
        let mut pos = size;
        let mut n = 0;
        let mut ch = [(0u64, 0u64); 7]; // payload offset / end of the meta children
        loop {
            if n >= BMFF_TOP_MAX {
                return Err(ERR_HDR);
            }
            n += 1;
            let Some((size, ty, hdr)) = bbox(data, len, pos) else { return Err(ERR_HDR) }; // no meta
            let bend = pos + size;
            if ty == TAG_META {
                let mut c = pos + hdr + 4; // first child (after version / flags)
                if c > bend {
                    return Err(ERR_HDR);
                }
                let mut k = 0;
                while k < BMFF_META_MAX {
                    k += 1;
                    let Some((size, ty, hdr)) = bbox(data, bend, c) else { break };
                    let cend = c + size;
                    if let Some(t) = META_TAGS.iter().position(|&t| t == ty)
                        && ch[t].0 == 0
                    {
                        ch[t] = (c + hdr, cend); // only the first of each kind
                    }
                    c = cend;
                }
                break;
            }
            pos = bend;
        }
        // hdlr: version / flags, pre_defined, handler_type
        let (s, e) = ch[CH_HDLR];
        if s != 0 && s + 12 <= e {
            bx.hdlr = le32(data, s + 8);
        }
        let (s, e) = ch[CH_IDAT];
        bx.idat = s;
        bx.idatlen = e - s;
        // iinf (required)
        let (s, e) = ch[CH_IINF];
        if s == 0 {
            return Err(ERR_HDR);
        }
        let r = bx.iinf(s, e);
        if r != 0 {
            return Err(r);
        }
        // pitm: version 0 u16, else u32
        let (s, e) = ch[CH_PITM];
        if s != 0 && s + 6 <= e {
            if data[s as usize] == 0 {
                bx.primary = be16(data, s + 4);
            } else if s + 8 <= e {
                bx.primary = be32(data, s + 4);
            }
        }
        let (s, e) = ch[CH_ILOC];
        if s != 0 {
            let r = bx.iloc(s, e);
            if r != 0 {
                return Err(r);
            }
        }
        let (s, e) = ch[CH_IPRP];
        if s != 0 {
            let r = bx.iprp(s, e);
            if r != 0 {
                return Err(r);
            }
        }
        let (s, e) = ch[CH_IREF];
        if s != 0 {
            bx.iref(s, e);
        }
        Ok(bx)
    }

    /// isobmff_find: index of the (first) item with that id or -1
    pub fn find(&self, id: u32) -> i32 {
        match self.items.iter().position(|it| it.id == id) {
            Some(k) => k as i32,
            None => -1,
        }
    }

    /// bx_findh: find, trying index hint first (boxes usually list the items in iinf order)
    fn findh(&self, id: u32, hint: u32) -> i32 {
        if (hint as usize) < self.items.len() && self.items[hint as usize].id == id {
            return hint as i32;
        }
        self.find(id)
    }

    /// isobmff_item: the data of item id (see the file header): ERR_HDR no such item / no
    /// location, ERR_TRUNC data outside the file, ERR_UNSUPPORTED other construction
    /// methods, ERR_LIMIT more than BMFF_DATA_MAX bytes
    pub fn item(&self, id: u32) -> Result<ItemData<'a>, i32> {
        let k = self.find(id);
        if k < 0 {
            return Err(ERR_HDR);
        }
        let it = self.items[k as usize];
        match it.cm {
            CM_FILE | CM_IDAT => {}
            CM_TRUNC => return Err(ERR_TRUNC),
            CM_NONE => return Err(ERR_HDR),
            _ => return Err(ERR_UNSUPPORTED),
        }
        let ex = &self.exts[it.ext0 as usize..(it.ext0 + it.next) as usize];
        let mut total = 0u64;
        for e in ex {
            total = total.checked_add(e.1).ok_or(ERR_LIMIT)?;
        }
        if total > BMFF_DATA_MAX {
            return Err(ERR_LIMIT);
        }
        if total == 0 {
            return Ok(ItemData::File(&self.data[..0]));
        }
        if ex.len() == 1 {
            let (o, l) = ex[0];
            return Ok(ItemData::File(&self.data[o as usize..(o + l) as usize]));
        }
        let mut v = Vec::with_capacity(total as usize);
        for &(o, l) in ex {
            v.extend_from_slice(&self.data[o as usize..(o + l) as usize]);
        }
        Ok(ItemData::Joined(v))
    }

    /// isobmff_prop: payload of the first property of that type associated with item id
    pub fn prop(&self, id: u32, ty: u32) -> Option<&'a [u8]> {
        let k = self.find(id);
        if k < 0 {
            return None;
        }
        let mut j = 0;
        while let Some((p, _)) = self.assoc(k as u32, j) {
            if p.ty == ty {
                return Some(self.payload(&p));
            }
            j += 1;
        }
        None
    }

    /// the payload bytes of a property
    pub fn payload(&self, p: &Prop) -> &'a [u8] {
        &self.data[p.off as usize..(p.off + p.len) as usize]
    }

    /// isobmff_assoc: the k-th property associated with item index (a zero record for an
    /// invalid property index) and its essential flag; None past the last association
    pub fn assoc(&self, index: u32, k: u32) -> Option<(Prop, bool)> {
        let it = self.items.get(index as usize)?;
        if k >= it.nassoc {
            return None;
        }
        let v = self.assoc[(it.assoc0 + k) as usize];
        let ess = v >> 31 != 0;
        let i = v & 0x7FFF_FFFF;
        if i == 0 || i as usize > self.props.len() {
            return Some((Prop::default(), ess));
        }
        Some((self.props[i as usize - 1], ess))
    }

    /// isobmff_ref: the k-th to_item_ID of the references of that type from item from, or -1
    pub fn ref_(&self, ty: u32, from: u32, k: u32) -> i32 {
        let mut k = k;
        for r in &self.refs {
            if r.ty == ty && r.from == from {
                if k == 0 {
                    return r.to as i32;
                }
                k -= 1;
            }
        }
        -1
    }

    /// isobmff_rref: the from_item_ID of the k-th reference of that type that points to
    /// item to, or -1
    pub fn rref(&self, ty: u32, to: u32, k: u32) -> i32 {
        let mut k = k;
        for r in &self.refs {
            if r.ty == ty && r.to == to {
                if k == 0 {
                    return r.from as i32;
                }
                k -= 1;
            }
        }
        -1
    }

    // ------------------------------------------------------------------ meta children

    /// bx_iinf: item records from iinf (payload pos..end)
    fn iinf(&mut self, pos: u64, end: u64) -> i32 {
        let d = self.data;
        let mut p = pos;
        if p + 6 > end {
            return ERR_HDR;
        }
        let mut cnt;
        if d[p as usize] == 0 {
            cnt = be16(d, p + 4) as u64;
            p += 6;
        } else {
            if p + 8 > end {
                return ERR_HDR;
            }
            cnt = be32(d, p + 4) as u64;
            p += 8;
        }
        if cnt > BMFF_ITEM_MAX as u64 {
            return ERR_LIMIT;
        }
        cnt = cnt.min((end - p) / 12); // at most one entry per 12 bytes
        self.items.reserve(cnt as usize);
        for _ in 0..cnt {
            let Some((size, ty, hdr)) = bbox(d, end, p) else { return ERR_HDR };
            let eend = p + size; // entry end
            if ty == TAG_INFE {
                let c = p + hdr; // payload: version, flags
                if c + 4 <= eend {
                    let mut it = Item { flags: d[c as usize + 3] as u32, ..Default::default() };
                    let v = d[c as usize];
                    let name = match v {
                        0 | 1 if c + 8 <= eend => {
                            // v0 / v1: u16 id, u16 protection
                            it.id = be16(d, c + 4);
                            it.prot = be16(d, c + 6);
                            it.ty = 0;
                            Some(c + 8)
                        }
                        2 if c + 12 <= eend => {
                            // v2: u16 id, u16 protection, type
                            it.id = be16(d, c + 4);
                            it.prot = be16(d, c + 6);
                            it.ty = le32(d, c + 8);
                            Some(c + 12)
                        }
                        3 if c + 14 <= eend => {
                            // v3: u32 id, u16 protection, type
                            it.id = be32(d, c + 4);
                            it.prot = be16(d, c + 8);
                            it.ty = le32(d, c + 10);
                            Some(c + 14)
                        }
                        _ => None,
                    };
                    if let Some(n) = name {
                        // item_name, then content_type (version 0 / 1 and mime) or
                        // item_uri_type (uri )
                        let s = (cstr(d, n, eend) + 1).min(eend);
                        if it.ty == 0 || it.ty == TAG_MIME || it.ty == TAG_URI {
                            it.ctype = s;
                            it.ctlen = (cstr(d, s, eend) - s) as u32;
                        }
                        it.cm = CM_NONE;
                        self.items.push(it);
                    }
                }
            }
            p = eend;
        }
        0
    }

    /// bx_iloc: item locations from iloc: pass 0 counts the extents, pass 1 stores them
    /// (resolved, see the file header)
    fn iloc(&mut self, pos: u64, end: u64) -> i32 {
        let d = self.data;
        if pos + 6 > end {
            return ERR_HDR;
        }
        let ver = d[pos as usize] as u32;
        if ver > 2 {
            return ERR_UNSUPPORTED;
        }
        let idn = if ver == 2 { 4 } else { 2 }; // item_ID size
        let osz = (d[pos as usize + 4] >> 4) as u32;
        let lsz = (d[pos as usize + 4] & 15) as u32;
        let bsz = (d[pos as usize + 5] >> 4) as u32;
        let isz = if ver == 0 { 0 } else { (d[pos as usize + 5] & 15) as u32 };
        // every size must be 0, 4 or 8
        for s in [osz, lsz, bsz, isz] {
            if s != 0 && s != 4 && s != 8 {
                return ERR_HDR;
            }
        }
        let body = pos + 6;
        for pass in 0..2 {
            let mut p = body;
            let mut next = 0u32; // extents
            let mut num = 0u32; // items of this iloc read (findh hint)
            let Some((cnt, np)) = rdn(d, p, idn, end) else { return ERR_HDR };
            p = np;
            for _ in 0..cnt as u32 {
                let Some((id, np)) = rdn(d, p, idn, end) else { return ERR_HDR };
                p = np;
                let mut cm = 0u32;
                if ver != 0 {
                    let Some((v, np)) = rdn(d, p, 2, end) else { return ERR_HDR };
                    p = np;
                    cm = v as u32 & 15;
                }
                let Some((dri, np)) = rdn(d, p, 2, end) else { return ERR_HDR }; // data_reference_index
                p = np;
                let Some((base, np)) = rdn(d, p, bsz, end) else { return ERR_HDR }; // base_offset
                p = np;
                let Some((ecnt, np)) = rdn(d, p, 2, end) else { return ERR_HDR }; // extent_count
                p = np;
                // pass 1: the item record (first iloc entry of the id only)
                let mut item: Option<usize> = None;
                if pass == 1 {
                    let k = self.findh(id as u32, num);
                    num += 1;
                    if k >= 0 && self.items[k as usize].cm == CM_NONE {
                        let it = &mut self.items[k as usize];
                        it.ext0 = next;
                        it.next = ecnt as u32;
                        let mut c = if cm > CM_ITEM { CM_EXTERN } else { cm };
                        if dri != 0 {
                            c = CM_EXTERN;
                        }
                        it.cm = c;
                        item = Some(k as usize);
                    }
                }
                for _ in 0..ecnt as u32 {
                    let Some((_, np)) = rdn(d, p, isz, end) else { return ERR_HDR }; // extent_index (ignored)
                    p = np;
                    let Some((eoff, np)) = rdn(d, p, osz, end) else { return ERR_HDR }; // extent_offset
                    p = np;
                    let Some((elen, np)) = rdn(d, p, lsz, end) else { return ERR_HDR }; // extent_length
                    p = np;
                    if pass == 0 {
                        next += 1;
                        if next > BMFF_EXT_MAX {
                            return ERR_LIMIT;
                        }
                        continue;
                    }
                    // resolve against the file / idat
                    let e = next as usize;
                    next += 1;
                    self.exts[e] = (eoff, elen); // unresolved values for other methods
                    let Some(k) = item else { continue };
                    let (ws, we) = match self.items[k].cm {
                        CM_FILE => (0u64, d.len() as u64),
                        CM_IDAT => (self.idat, self.idat + self.idatlen),
                        _ => continue,
                    };
                    let off = eoff.checked_add(base).and_then(|v| v.checked_add(ws));
                    match off {
                        Some(o) if o <= we => {
                            let left = we - o; // bytes left in the window
                            let l = if elen == 0 { left } else { elen }; // 0 = to the end
                            if l > left {
                                self.items[k].cm = CM_TRUNC;
                            } else {
                                self.exts[e] = (o, l);
                            }
                        }
                        _ => self.items[k].cm = CM_TRUNC,
                    }
                }
            }
            if pass == 0 {
                if next == 0 {
                    return 0;
                }
                self.exts = alloc::vec![(0, 0); next as usize];
            }
        }
        0
    }

    /// bx_iprp: properties (first ipco) and their associations (every ipma) from iprp
    fn iprp(&mut self, pos: u64, end: u64) -> i32 {
        let d = self.data;
        let mut ipco = (0u64, 0u64);
        let mut sum = 0u64; // summed ipma payload bytes
        // children: the first ipco, the size of all ipma boxes
        let mut c = pos;
        let mut k = 0;
        while k < BMFF_META_MAX {
            k += 1;
            let Some((size, ty, hdr)) = bbox(d, end, c) else { break };
            let cend = c + size;
            if ty == TAG_IPMA {
                sum += cend - (c + hdr);
            } else if ty == TAG_IPCO && ipco.0 == 0 {
                ipco = (c + hdr, cend);
            }
            c = cend;
        }
        if ipco.0 == 0 {
            return 0; // no properties
        }
        let cap = ((ipco.1 - ipco.0) >> 3).min(BMFF_PROP_MAX) as usize; // at most one property per 8 bytes
        if cap == 0 {
            return 0;
        }
        self.props.reserve(cap);
        let mut c = ipco.0;
        while self.props.len() < cap {
            let Some((size, ty, hdr)) = bbox(d, ipco.1, c) else { break };
            let cend = c + size;
            self.props.push(Prop { ty, off: c + hdr, len: cend - (c + hdr) });
            c = cend;
        }
        if sum == 0 {
            return 0;
        }
        let mut c = pos;
        let mut k = 0;
        while k < BMFF_META_MAX {
            k += 1;
            let Some((size, ty, hdr)) = bbox(d, end, c) else { return 0 };
            let start = c;
            c += size; // next child
            if ty != TAG_IPMA {
                continue;
            }
            let mut p = start + hdr; // payload
            let iend = c;
            if p + 8 > iend {
                return ERR_HDR;
            }
            let ver = d[p as usize];
            let f16 = d[p as usize + 3] & 1 != 0;
            let cnt = be32(d, p + 4);
            p += 8;
            for e in 0..cnt {
                let Some((id, np)) = rdn(d, p, if ver != 0 { 4 } else { 2 }, iend) else { return ERR_HDR };
                p = np;
                let k = self.findh(id as u32, e);
                let item = if k >= 0 && self.items[k as usize].nassoc == 0 { Some(k as usize) } else { None };
                if p >= iend {
                    return ERR_HDR;
                }
                let n = d[p as usize] as u32;
                p += 1;
                if let Some(i) = item {
                    self.items[i].nassoc = n;
                    self.items[i].assoc0 = self.assoc.len() as u32;
                }
                for _ in 0..n {
                    let (idx, ess);
                    if !f16 {
                        if p >= iend {
                            return ERR_HDR;
                        }
                        let v = d[p as usize] as u32;
                        p += 1;
                        idx = v & 0x7F;
                        ess = v >> 7;
                    } else {
                        if p + 2 > iend {
                            return ERR_HDR;
                        }
                        let v = be16(d, p);
                        p += 2;
                        idx = v & 0x7FFF;
                        ess = v >> 15;
                    }
                    if item.is_some() {
                        self.assoc.push(idx | ess << 31);
                    }
                }
            }
        }
        0
    }

    /// bx_iref: references from iref; ends at the first malformed reference box
    fn iref(&mut self, pos: u64, end: u64) {
        let d = self.data;
        if pos + 4 > end {
            return;
        }
        let idn = if d[pos as usize] != 0 { 4 } else { 2 };
        let mut p = pos + 4;
        loop {
            let Some((size, ty, hdr)) = bbox(d, end, p) else { return };
            let bend = p + size;
            let Some((from, np)) = rdn(d, p + hdr, idn, bend) else { return }; // from_item_ID
            let Some((cnt, np)) = rdn(d, np, 2, bend) else { return }; // reference_count
            let mut q = np;
            for _ in 0..cnt {
                let Some((to, np)) = rdn(d, q, idn, bend) else { return };
                q = np;
                self.refs.push(Ref { ty, from: from as u32, to: to as u32 });
            }
            p = bend;
        }
    }
}

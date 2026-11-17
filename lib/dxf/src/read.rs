// read -- the sections of a DXF: HEADER (the variables lib.rs's Header has), TABLES (LAYER, LTYPE, STYLE, BLOCK_RECORD,
// DIMSTYLE), BLOCKS, ENTITIES; CLASSES, OBJECTS and the rest are skipped.  An entity is its group pairs up to the next
// code 0, gathered into one buffer (EntBuf) and then read by its kind; POLYLINE takes its VERTEXes up to SEQEND, INSERT
// its ATTRIBs.  Counts in a file (vertices, edges, knots) are never trusted for an allocation: lists grow as their
// items come, bounded by Limits.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::pairs::{trim, Pairs};
use crate::strings::{decode_unicode_escapes, Decoder};
use crate::*;

/// an entity's group pairs: codes and the ranges of their values in `text`
#[derive(Default)]
struct EntBuf {
    codes: Vec<i32>,
    spans: Vec<(u32, u32)>,
    text: Vec<u8>,
}

impl EntBuf {
    fn clear(&mut self) {
        self.codes.clear();
        self.spans.clear();
        self.text.clear();
    }

    fn len(&self) -> usize {
        self.codes.len()
    }

    fn raw(&self, i: usize) -> &[u8] {
        let (a, b) = self.spans[i];
        &self.text[a as usize..b as usize]
    }

    /// a float: finite, else None
    fn f(&self, i: usize) -> Option<f64> {
        let s = core::str::from_utf8(trim(self.raw(i))).ok()?;
        let v: f64 = s.parse().ok()?;
        v.is_finite().then_some(v)
    }

    /// an integer (also written as a float with nothing after the point)
    fn int(&self, i: usize) -> Option<i64> {
        let s = core::str::from_utf8(trim(self.raw(i))).ok()?;
        if let Ok(v) = s.parse::<i64>() {
            return Some(v);
        }
        let v: f64 = s.parse().ok()?;
        (v.is_finite() && v == (v as i64) as f64).then_some(v as i64)
    }

    fn hex(&self, i: usize) -> Option<u64> {
        let s = core::str::from_utf8(trim(self.raw(i))).ok()?;
        u64::from_str_radix(s, 16).ok()
    }
}

struct R<'s> {
    p: Pairs<'s>,
    lim: Limits,
    names: Names,
    stats: Stats,
    dec: Decoder,
    header: Header,
    tables: Tables,
    header_sent: bool,
    tables_sent: bool,
    keep_all: bool,
    kept_entities: u64,
    kept_points: u64,
    /// whether the tokenizer holds a pair not yet taken
    have: bool,
    buf: EntBuf,
    /// a VERTEX's or ATTRIB's pairs while its POLYLINE's or INSERT's are being read
    sub: EntBuf,
}

/// the value as upper-case ASCII (kinds, section names)
fn upper(b: &[u8]) -> String {
    let t = trim(b);
    t.iter().map(|c| c.to_ascii_uppercase() as char).collect()
}

pub(crate) fn run(src: &mut dyn FnMut(&mut [u8]) -> usize, lim: &Limits, sink: &mut dyn Sink, keep_all: bool) -> Result<(Stats, Names)> {
    let mut r = R { p: Pairs::new(src, lim.line_bytes), lim: *lim, names: Names::default(), stats: Stats::default(), dec: Decoder::default(), header: Header::default(),
                    tables: Tables::default(), header_sent: false, tables_sent: false, keep_all, kept_entities: 0, kept_points: 0, have: false, buf: EntBuf::default(),
                    sub: EntBuf::default() };
    if crate::is_binary(r.p.peek(22)) {
        return Err(Error::Binary);
    }
    if r.p.peek(3) == [0xEF, 0xBB, 0xBF] {
        r.p.skip(3);
    }
    // the layer "0" is Name(0)
    r.names.intern("0", lim.names)?;
    let res = r.top(sink);
    r.stats.bytes = r.p.bytes;
    r.stats.lines = r.p.lines;
    match res {
        // a file that broke off after a section began: what was read up to there (GDAL does the same), with a note
        Err(Error::Invalid(s)) if !r.stats.sections.is_empty() => r.stats.note(format!("{s}: the rest of the file is not read")),
        other => other?,
    }
    if !r.header_sent {
        sink.header(&r.header, &r.names);
    }
    if !r.tables_sent {
        sink.tables(&r.tables, &r.names);
    }
    Ok((r.stats, r.names))
}

impl<'s> R<'s> {
    /// the current pair, read when it was taken; false at the end of the file
    fn pair(&mut self) -> Result<bool> {
        if !self.have {
            if !self.p.next()? {
                return Ok(false);
            }
            self.stats.pairs += 1;
            self.have = true;
        }
        Ok(true)
    }

    fn take(&mut self) {
        self.have = false;
    }

    fn code(&self) -> i32 {
        self.p.code
    }

    fn intern(&mut self, b: &[u8]) -> Result<Name> {
        let s = decode_unicode_escapes(&self.dec.text(b));
        let s = s.trim_end_matches(['\r', '\n']);
        self.names.intern(s, self.lim.names)
    }

    fn send_header(&mut self, sink: &mut dyn Sink) {
        if !self.header_sent {
            self.header_sent = true;
            sink.header(&self.header, &self.names);
        }
    }

    fn send_tables(&mut self, sink: &mut dyn Sink) {
        self.send_header(sink);
        if !self.tables_sent {
            self.tables_sent = true;
            sink.tables(&self.tables, &self.names);
        }
    }

    fn count(&mut self, w: Where, kind: &str) {
        if let Some(c) = self.stats.kinds.get_mut(&(w, String::from(kind))) {
            *c += 1;
        } else {
            self.stats.kinds.insert((w, String::from(kind)), 1);
        }
    }

    fn top(&mut self, sink: &mut dyn Sink) -> Result<()> {
        loop {
            if !self.pair()? {
                return Ok(());
            }
            let code = self.code();
            let v = upper(self.p.value());
            if code == 0 && v == "EOF" {
                self.take();
                return Ok(());
            }
            let name = if code == 0 && v == "SECTION" {
                self.take();
                if !self.pair()? {
                    return Ok(());
                }
                if self.code() != 2 {
                    self.stats.note(format!("line {}: a SECTION without its name", self.p.lines));
                    continue;
                }
                upper(self.p.value())
            } else if code == 2 && matches!(v.as_str(), "HEADER" | "TABLES" | "BLOCKS" | "ENTITIES" | "CLASSES" | "OBJECTS") {
                // (a section's name without "0 SECTION" before it, as GDAL's text.dxf has it)
                self.stats.note(format!("line {}: the section {v} without 0 SECTION", self.p.lines));
                v
            } else {
                if code == 0 {
                    self.stats.note(format!("line {}: {v} outside a section", self.p.lines));
                }
                self.take();
                continue;
            };
            self.take();
            if self.stats.sections.len() < 64 {
                self.stats.sections.push(name.clone());
            }
            match name.as_str() {
                "HEADER" => self.header_section()?,
                "TABLES" => {
                    self.send_header(sink);
                    self.tables_section()?;
                    self.send_tables(sink);
                }
                "BLOCKS" => {
                    self.send_tables(sink);
                    self.blocks_section(sink)?;
                }
                "ENTITIES" => {
                    self.send_tables(sink);
                    self.entities_section(sink)?;
                }
                _ => self.skip_section()?,
            }
        }
    }

    /// up to 0 ENDSEC (taken) or 0 EOF / the end (not taken)
    fn skip_section(&mut self) -> Result<()> {
        while self.pair()? {
            if self.code() == 0 {
                let v = upper(self.p.value());
                if v == "ENDSEC" {
                    self.take();
                    return Ok(());
                }
                if v == "EOF" {
                    return Ok(());
                }
            }
            self.take();
        }
        Ok(())
    }

    fn header_section(&mut self) -> Result<()> {
        let mut var = String::new();
        let mut h = Header { present: true, ..Header::default() };
        let mut pt = [0.0f64; 3];
        let mut pt_of = String::new();
        while self.pair()? {
            let code = self.code();
            if code == 0 {
                if upper(self.p.value()) == "ENDSEC" {
                    self.take();
                }
                break;
            }
            let raw = trim(self.p.value());
            let s = core::str::from_utf8(raw).unwrap_or("");
            let f = s.parse::<f64>().ok().filter(|v| v.is_finite());
            let i = s.parse::<i64>().ok().or_else(|| f.filter(|v| *v == (*v as i64) as f64).map(|v| v as i64));
            if code == 9 {
                if (pt_of == "$EXTMIN" || pt_of == "$EXTMAX") && !pt_of.is_empty() {
                    if pt_of == "$EXTMIN" {
                        h.extmin = Some(pt);
                    } else {
                        h.extmax = Some(pt);
                    }
                }
                pt_of.clear();
                pt = [0.0; 3];
                var = upper(raw);
                self.take();
                continue;
            }
            match (var.as_str(), code) {
                ("$ACADVER", 1) => h.version = String::from(s),
                ("$DWGCODEPAGE", 3) => h.codepage = String::from(s),
                ("$INSUNITS", 70) => h.insunits = i.unwrap_or(0).clamp(-1000, 1000) as i32,
                ("$LTSCALE", 40) => h.ltscale = f.filter(|v| *v > 0.0).unwrap_or(1.0),
                ("$LWDISPLAY", 290 | 70) => h.lwdisplay = i.map(|v| v != 0),
                ("$LWDEFAULT", 370 | 70) => h.lwdefault = i.unwrap_or(25).clamp(0, 211) as u16,
                ("$PDMODE", 70) => h.pdmode = i.unwrap_or(0).clamp(-1000, 1000) as i32,
                ("$PDSIZE", 40) => h.pdsize = f.unwrap_or(0.0),
                ("$DIMSCALE", 40) => h.dimscale = f.filter(|v| *v > 0.0).unwrap_or(1.0),
                ("$DIMASZ", 40) => h.dimasz = f.filter(|v| *v >= 0.0).unwrap_or(0.18),
                ("$TEXTSIZE", 40) => h.textsize = f.filter(|v| *v > 0.0).unwrap_or(0.2),
                ("$MIRRTEXT", 70) => h.mirrtext = i.unwrap_or(0) != 0,
                ("$EXTMIN" | "$EXTMAX", 10 | 20 | 30) => {
                    pt_of = var.clone();
                    if let Some(v) = f {
                        pt[(code / 10 - 1) as usize] = v;
                    }
                }
                _ => {}
            }
            self.take();
        }
        if pt_of == "$EXTMIN" {
            h.extmin = Some(pt);
        } else if pt_of == "$EXTMAX" {
            h.extmax = Some(pt);
        }
        self.dec = Decoder::of(&h.version, &h.codepage);
        if self.dec.multibyte {
            self.stats.note(format!("the code page {}: no table here, characters above 0x7F read as U+FFFD", h.codepage));
        }
        self.header = h;
        Ok(())
    }

    /// the pairs of the entity whose 0 pair was just taken, up to the next code 0 (not taken) into `which` (false:
    /// buf, true: sub)
    fn gather(&mut self, sub: bool) -> Result<()> {
        let (max_pairs, max_bytes) = (self.lim.entity_pairs, self.lim.entity_bytes);
        let mut n = 0usize;
        {
            let b = if sub { &mut self.sub } else { &mut self.buf };
            b.clear();
        }
        while self.pair()? {
            let code = self.code();
            if code == 0 {
                break;
            }
            n += 1;
            let v = self.p.value();
            let b = if sub { &mut self.sub } else { &mut self.buf };
            if n > max_pairs || b.text.len() + v.len() > max_bytes {
                return Err(Error::Limit(format!("line {}: an entity of more than {max_pairs} pairs or {max_bytes} bytes", self.p.lines)));
            }
            let a = b.text.len() as u32;
            b.text.extend_from_slice(v);
            b.codes.push(code);
            b.spans.push((a, b.text.len() as u32));
            self.have = false;
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ tables

    fn tables_section(&mut self) -> Result<()> {
        let mut entries = 0usize;
        while self.pair()? {
            if self.code() != 0 {
                self.take();
                continue;
            }
            let v = upper(self.p.value());
            match v.as_str() {
                "ENDSEC" => {
                    self.take();
                    return Ok(());
                }
                "EOF" => return Ok(()),
                "TABLE" | "ENDTAB" => {
                    self.take();
                    self.gather(false)?;
                }
                _ => {
                    self.take();
                    self.gather(false)?;
                    entries += 1;
                    if entries > self.lim.table_entries {
                        return Err(Error::Limit(format!("more than {} table entries", self.lim.table_entries)));
                    }
                    match v.as_str() {
                        "LAYER" => self.layer()?,
                        "LTYPE" => self.ltype()?,
                        "STYLE" => self.style()?,
                        "BLOCK_RECORD" => self.block_record()?,
                        "DIMSTYLE" => self.dimstyle()?,
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    /// the first value of a code (before any XDATA)
    fn first(&self, code: i32) -> Option<usize> {
        let b = &self.buf;
        (0..b.len()).take_while(|&i| b.codes[i] < 1000).find(|&i| b.codes[i] == code)
    }

    fn name_of(&mut self, code: i32) -> Result<Option<Name>> {
        match self.first(code) {
            Some(i) => {
                let raw: Vec<u8> = self.buf.raw(i).to_vec();
                Ok(Some(self.intern(&raw)?))
            }
            None => Ok(None),
        }
    }

    fn layer(&mut self) -> Result<()> {
        let Some(name) = self.name_of(2)? else { return Ok(()) };
        let b = &self.buf;
        let aci = self.first(62).and_then(|i| b.int(i)).unwrap_or(7);
        let flags = self.first(70).and_then(|i| b.int(i)).unwrap_or(0);
        let mut color = Color::Aci(aci.unsigned_abs().clamp(1, 255) as u8);
        if let Some(rgb) = self.first(420).and_then(|i| b.int(i)) {
            color = true_color(rgb, Some(aci));
        }
        let lw = lineweight(self.first(370).and_then(|i| b.int(i)).unwrap_or(-3));
        let plot = self.first(290).and_then(|i| b.int(i)).unwrap_or(1) != 0;
        // the transparency in its XDATA: 1001 AcCmTransparency, 1071 value
        let mut transparency = None;
        for i in 0..b.len() {
            if b.codes[i] == 1001 && trim(b.raw(i)).eq_ignore_ascii_case(b"AcCmTransparency") {
                if let Some(v) = (i + 1..b.len()).find(|&k| b.codes[k] == 1071).and_then(|k| b.int(k)) {
                    transparency = transp(v);
                }
            }
        }
        let lt = self.name_of(6)?;
        let continuous = lt.is_some_and(|n| self.names.get(n).eq_ignore_ascii_case("CONTINUOUS"));
        let l = Layer { name, color, on: aci >= 0, frozen: flags & 1 != 0, locked: flags & 4 != 0, plot, linetype: if continuous { None } else { lt }, lineweight: lw,
                        transparency: match transparency {
                            Some(Transparency::Alpha(a)) => Some(a),
                            _ => None,
                        } };
        // (a second entry of a name replaces the first)
        if let Some(old) = self.tables.layers.iter_mut().find(|x| x.name == name) {
            *old = l;
        } else {
            self.tables.layers.push(l);
        }
        Ok(())
    }

    fn ltype(&mut self) -> Result<()> {
        let Some(name) = self.name_of(2)? else { return Ok(()) };
        let b = &self.buf;
        let description = self.first(3).map(|i| self.dec.text(b.raw(i))).unwrap_or_default();
        let total = self.first(40).and_then(|i| b.f(i)).unwrap_or(0.0);
        let mut dashes: Vec<Dash> = Vec::new();
        for i in 0..b.len() {
            match b.codes[i] {
                49 => {
                    if dashes.len() >= 4096 {
                        break;
                    }
                    dashes.push(Dash { length: b.f(i).unwrap_or(0.0), complex: 0, text: None });
                }
                74 => {
                    if let Some(d) = dashes.last_mut() {
                        d.complex = b.int(i).unwrap_or(0).clamp(0, 0xFFFF) as u32;
                    }
                }
                9 => {
                    if let Some(d) = dashes.last_mut() {
                        d.text = Some(self.dec.text(b.raw(i)));
                    }
                }
                c if c >= 1000 => break,
                _ => {}
            }
        }
        let d = LinetypeDef { name, description, dashes, total };
        if let Some(old) = self.tables.linetypes.iter_mut().find(|x| x.name == name) {
            *old = d;
        } else {
            self.tables.linetypes.push(d);
        }
        Ok(())
    }

    fn style(&mut self) -> Result<()> {
        let name = self.name_of(2)?.unwrap_or(Name(0));
        let b = &self.buf;
        let font = self.first(3).map(|i| self.dec.text(trim(b.raw(i)))).unwrap_or_default();
        let bigfont = self.first(4).map(|i| self.dec.text(trim(b.raw(i)))).unwrap_or_default();
        let mut family = None;
        for i in 0..b.len() {
            if b.codes[i] == 1001 && trim(b.raw(i)).eq_ignore_ascii_case(b"ACAD") {
                if let Some(k) = (i + 1..b.len()).find(|&k| b.codes[k] == 1000) {
                    let f = self.dec.text(trim(b.raw(k)));
                    if !f.is_empty() {
                        family = Some(f);
                    }
                }
            }
        }
        let s = StyleDef {
            name,
            font,
            bigfont,
            family,
            height: self.first(40).and_then(|i| b.f(i)).filter(|v| *v >= 0.0).unwrap_or(0.0),
            width: self.first(41).and_then(|i| b.f(i)).filter(|v| *v > 0.0).unwrap_or(1.0),
            oblique: self.first(50).and_then(|i| b.f(i)).unwrap_or(0.0),
            flags: self.first(70).and_then(|i| b.int(i)).unwrap_or(0).clamp(0, 0xFFFF) as u32,
            generation: self.first(71).and_then(|i| b.int(i)).unwrap_or(0).clamp(0, 0xFFFF) as u32,
        };
        // (shape files are styles without a name, kept apart)
        if self.first(2).is_none() || s.flags & 1 != 0 && self.names.get(name).is_empty() {
            return Ok(());
        }
        if let Some(old) = self.tables.styles.iter_mut().find(|x| x.name == name) {
            *old = s;
        } else {
            self.tables.styles.push(s);
        }
        Ok(())
    }

    fn block_record(&mut self) -> Result<()> {
        let Some(name) = self.name_of(2)? else { return Ok(()) };
        let b = &self.buf;
        let r = BlockRecord { name, handle: self.first(5).and_then(|i| b.hex(i)).unwrap_or(0), units: self.first(70).and_then(|i| b.int(i)).unwrap_or(0).clamp(-1000, 1000) as i32 };
        self.tables.block_records.push(r);
        Ok(())
    }

    fn dimstyle(&mut self) -> Result<()> {
        let Some(name) = self.name_of(2)? else { return Ok(()) };
        let b = &self.buf;
        let d = DimStyle { name, arrow_size: self.first(41).and_then(|i| b.f(i)).filter(|v| *v >= 0.0).unwrap_or(0.18),
                           scale: self.first(40).and_then(|i| b.f(i)).filter(|v| *v > 0.0).unwrap_or(1.0) };
        self.tables.dimstyles.push(d);
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ blocks, entities

    fn keep(&mut self, e: &Entity) -> Result<()> {
        self.kept_entities += 1;
        self.kept_points += points_of(e) as u64;
        if self.kept_entities > self.lim.kept_entities || self.kept_points > self.lim.kept_points {
            return Err(Error::Limit(format!("more than {} entities or {} points kept", self.lim.kept_entities, self.lim.kept_points)));
        }
        Ok(())
    }

    fn blocks_section(&mut self, sink: &mut dyn Sink) -> Result<()> {
        let mut blocks = 0usize;
        while self.pair()? {
            if self.code() != 0 {
                self.take();
                continue;
            }
            let v = upper(self.p.value());
            match v.as_str() {
                "ENDSEC" => {
                    self.take();
                    return Ok(());
                }
                "EOF" => return Ok(()),
                "BLOCK" => {
                    self.take();
                    self.count(Where::Blocks, "BLOCK");
                    self.gather(false)?;
                    blocks += 1;
                    if blocks > self.lim.blocks {
                        return Err(Error::Limit(format!("more than {} blocks", self.lim.blocks)));
                    }
                    let name = self.name_of(2)?.unwrap_or(Name(0));
                    let b = &self.buf;
                    let base = [self.first(10).and_then(|i| b.f(i)).unwrap_or(0.0), self.first(20).and_then(|i| b.f(i)).unwrap_or(0.0), self.first(30).and_then(|i| b.f(i)).unwrap_or(0.0)];
                    let flags = self.first(70).and_then(|i| b.int(i)).unwrap_or(0).clamp(0, 0xFFFF) as u32;
                    let xref = self.first(1).map(|i| self.dec.text(b.raw(i))).unwrap_or_default();
                    let paper = self.names.get(name).to_ascii_uppercase().starts_with("*PAPER_SPACE");
                    let mut blk = Block { name, base, flags, xref, entities: Vec::new(), paper };
                    // its entities up to ENDBLK
                    while self.pair()? {
                        if self.code() != 0 {
                            self.take();
                            continue;
                        }
                        let k = upper(self.p.value());
                        if k == "ENDBLK" {
                            self.take();
                            self.count(Where::Blocks, "ENDBLK");
                            self.gather(false)?;
                            break;
                        }
                        if k == "ENDSEC" || k == "EOF" || k == "BLOCK" {
                            self.stats.note(format!("a block without ENDBLK before line {}", self.p.lines));
                            break;
                        }
                        self.take();
                        self.count(Where::Blocks, &k);
                        self.gather(false)?;
                        if let Some(e) = self.entity(&k, Where::Blocks)? {
                            self.keep(&e)?;
                            blk.entities.push(e);
                        }
                    }
                    sink.block(blk, &self.names);
                }
                _ => {
                    // an entity outside a block: read and dropped
                    self.take();
                    self.count(Where::Blocks, &v);
                    self.gather(false)?;
                    let _ = self.entity(&v, Where::Blocks)?;
                    self.stats.note(format!("{v} in BLOCKS outside a block: skipped"));
                }
            }
        }
        Ok(())
    }

    fn entities_section(&mut self, sink: &mut dyn Sink) -> Result<()> {
        while self.pair()? {
            if self.code() != 0 {
                self.take();
                continue;
            }
            let v = upper(self.p.value());
            if v == "ENDSEC" {
                self.take();
                return Ok(());
            }
            if v == "EOF" {
                return Ok(());
            }
            self.take();
            self.count(Where::Entities, &v);
            self.gather(false)?;
            if let Some(e) = self.entity(&v, Where::Entities)? {
                if e.common.paper {
                    self.stats.paper += 1;
                }
                if self.keep_all {
                    self.keep(&e)?;
                }
                sink.entity(e, &self.names);
            }
        }
        Ok(())
    }

    /// what all entities have, from `buf` (up to XDATA)
    fn common(&mut self, kind: &str) -> Result<Common> {
        let mut c = Common { kind: self.names.intern(kind, self.lim.names)?, ..Common::default() };
        let mut aci: Option<i64> = None;
        let mut rgb: Option<i64> = None;
        let mut layer: Option<usize> = None;
        let mut lt: Option<usize> = None;
        let b = &self.buf;
        let mut ex = [0.0, 0.0, 1.0];
        let mut in_group = false;
        for i in 0..b.len() {
            let code = b.codes[i];
            if code >= 1000 {
                break;
            }
            if code == 102 {
                in_group = trim(b.raw(i)).first() == Some(&b'{');
                continue;
            }
            if in_group {
                continue;
            }
            match code {
                5 => c.handle = b.hex(i).unwrap_or(0),
                8 if layer.is_none() => layer = Some(i),
                6 if lt.is_none() => lt = Some(i),
                62 if aci.is_none() => aci = b.int(i),
                420 if rgb.is_none() => rgb = b.int(i),
                370 => c.lineweight = lineweight(b.int(i).unwrap_or(-1)),
                48 => c.ltscale = b.f(i).filter(|v| *v > 0.0).unwrap_or(1.0),
                440 => c.transparency = b.int(i).and_then(transp).unwrap_or(Transparency::ByLayer),
                210 => ex[0] = b.f(i).unwrap_or(0.0),
                220 => ex[1] = b.f(i).unwrap_or(0.0),
                230 => ex[2] = b.f(i).unwrap_or(1.0),
                39 => c.thickness = b.f(i).unwrap_or(0.0),
                60 => c.invisible = b.int(i).unwrap_or(0) != 0,
                67 => c.paper = b.int(i).unwrap_or(0) != 0,
                _ => {}
            }
        }
        let len = (ex[0] * ex[0] + ex[1] * ex[1] + ex[2] * ex[2]).abs();
        c.extrusion = if len > 1e-24 { ex } else { [0.0, 0.0, 1.0] };
        c.color = match (rgb, aci) {
            (Some(v), a) => true_color(v, a),
            (None, Some(0)) => Color::ByBlock,
            (None, Some(v)) if (1..=255).contains(&v) => Color::Aci(v as u8),
            (None, Some(v)) if (-255..=-1).contains(&v) => Color::Aci(v.unsigned_abs() as u8),
            _ => Color::ByLayer,
        };
        if let Some(i) = layer {
            let raw: Vec<u8> = self.buf.raw(i).to_vec();
            c.layer = self.intern(&raw)?;
        }
        if let Some(i) = lt {
            let raw: Vec<u8> = self.buf.raw(i).to_vec();
            let t = upper(&raw);
            c.linetype = match t.as_str() {
                "BYLAYER" | "" => Linetype::ByLayer,
                "BYBLOCK" => Linetype::ByBlock,
                _ => Linetype::Named(self.intern(&raw)?),
            };
        }
        Ok(c)
    }

    /// the entity in `buf` (its 0 pair `kind`, upper case); POLYLINE and INSERT take what follows them
    fn entity(&mut self, kind: &str, w: Where) -> Result<Option<Entity>> {
        let common = self.common(kind)?;
        let k = match kind {
            "LINE" => Kind::Line { a: self.p3(10, [0.0; 3]), b: self.p3(11, [0.0; 3]) },
            "POINT" => Kind::Point { at: self.p3(10, [0.0; 3]), angle: self.fv(50, 0.0) },
            "CIRCLE" => Kind::Circle { center: self.p3(10, [0.0; 3]), radius: self.fv(40, 0.0) },
            "ARC" => Kind::Arc { center: self.p3(10, [0.0; 3]), radius: self.fv(40, 0.0), start: self.fv(50, 0.0), end: self.fv(51, 360.0) },
            "ELLIPSE" => Kind::Ellipse { center: self.p3(10, [0.0; 3]), major: self.p3(11, [1.0, 0.0, 0.0]), ratio: self.fv(40, 1.0), start: self.fv(41, 0.0),
                                         end: self.fv(42, core::f64::consts::TAU) },
            "SPLINE" | "HELIX" => Kind::Spline(self.spline()?),
            "LWPOLYLINE" => Kind::Polyline(self.lwpolyline()?),
            "POLYLINE" => self.polyline(w)?,
            "TEXT" | "ATTRIB" | "ATTDEF" => Kind::Text(self.text(kind)?),
            "MTEXT" => Kind::MText(self.mtext()?),
            "INSERT" => Kind::Insert(self.insert(w)?),
            "HATCH" => Kind::Hatch(self.hatch()?),
            "SOLID" | "TRACE" | "3DFACE" => {
                let p0 = self.p3(10, [0.0; 3]);
                let p1 = self.p3(11, p0);
                let p2 = self.p3(12, p1);
                let p3 = self.p3(13, p2);
                Kind::Quad(Quad { points: [p0, p1, p2, p3], face: kind == "3DFACE", invisible_edges: self.iv(70, 0).clamp(0, 15) as u32 })
            }
            "DIMENSION" | "ARC_DIMENSION" | "LARGE_RADIAL_DIMENSION" => Kind::BlockRef(BlockRef { block: self.name_of(2)?, at: [0.0; 3], x_dir: None }),
            "ACAD_TABLE" => Kind::BlockRef(BlockRef { block: self.name_of(2)?, at: self.p3(10, [0.0; 3]), x_dir: self.first(11).map(|_| self.p3(11, [1.0, 0.0, 0.0])) }),
            "LEADER" => Kind::Leader(self.leader()?),
            "WIPEOUT" => Kind::Wipeout(self.wipeout()?),
            "RAY" | "XLINE" => Kind::Ray { at: self.p3(10, [0.0; 3]), dir: self.p3(11, [1.0, 0.0, 0.0]), both: kind == "XLINE" },
            "SEQEND" | "VERTEX" | "ENDBLK" => {
                if kind != "SEQEND" {
                    self.stats.note(format!("{kind} without its POLYLINE"));
                }
                return Ok(None);
            }
            _ => Kind::Other,
        };
        let e = Entity { common, kind: k };
        if points_of(&e) > self.lim.entity_points {
            return Err(Error::Limit(format!("line {}: an entity of more than {} points", self.p.lines, self.lim.entity_points)));
        }
        Ok(Some(e))
    }

    // the first value of a code in buf (before XDATA), as a float or integer
    fn fv(&self, code: i32, d: f64) -> f64 {
        self.first(code).and_then(|i| self.buf.f(i)).unwrap_or(d)
    }

    fn iv(&self, code: i32, d: i64) -> i64 {
        self.first(code).and_then(|i| self.buf.int(i)).unwrap_or(d)
    }

    /// a point of codes c, c + 10, c + 20 (missing parts: the default's)
    fn p3(&self, c: i32, d: P3) -> P3 {
        [self.fv(c, d[0]), self.fv(c + 10, d[1]), self.fv(c + 20, d[2])]
    }

    fn text_of(&self, i: usize) -> String {
        decode_unicode_escapes(&self.dec.text(self.buf.raw(i)))
    }

    fn spline(&mut self) -> Result<Spline> {
        let b = &self.buf;
        let max = self.lim.entity_points;
        let mut s = Spline { degree: self.iv(71, 3).clamp(0, 64) as u32, flags: self.iv(70, 0).clamp(0, 0xFFFF) as u32, ..Spline::default() };
        let mut tan_s: Option<P3> = None;
        let mut tan_e: Option<P3> = None;
        for i in 0..b.len() {
            let code = b.codes[i];
            if code >= 1000 {
                break;
            }
            let v = b.f(i);
            let full = s.knots.len() + s.control.len() + s.fit.len() + s.weights.len() >= max;
            match code {
                40 if !full => s.knots.push(v.unwrap_or(0.0)),
                41 if !full => s.weights.push(v.unwrap_or(1.0)),
                10 if !full => s.control.push([v.unwrap_or(0.0), 0.0, 0.0]),
                20 => set(s.control.last_mut(), 1, v),
                30 => set(s.control.last_mut(), 2, v),
                11 if !full => s.fit.push([v.unwrap_or(0.0), 0.0, 0.0]),
                21 => set(s.fit.last_mut(), 1, v),
                31 => set(s.fit.last_mut(), 2, v),
                12 => tan_s = Some([v.unwrap_or(0.0), 0.0, 0.0]),
                22 => set(tan_s.as_mut(), 1, v),
                32 => set(tan_s.as_mut(), 2, v),
                13 => tan_e = Some([v.unwrap_or(0.0), 0.0, 0.0]),
                23 => set(tan_e.as_mut(), 1, v),
                33 => set(tan_e.as_mut(), 2, v),
                _ => {}
            }
        }
        s.start_tangent = tan_s;
        s.end_tangent = tan_e;
        Ok(s)
    }

    fn lwpolyline(&mut self) -> Result<Polyline> {
        let b = &self.buf;
        let max = self.lim.entity_points;
        let flags = self.iv(70, 0);
        let mut p = Polyline { closed: flags & 1 != 0, plinegen: flags & 128 != 0, const_width: self.fv(43, 0.0), elevation: self.fv(38, 0.0), ..Polyline::default() };
        for i in 0..b.len() {
            let code = b.codes[i];
            if code >= 1000 {
                break;
            }
            let v = b.f(i);
            match code {
                10 if p.vertices.len() < max => p.vertices.push(Vertex { at: [v.unwrap_or(0.0), 0.0, 0.0], bulge: 0.0, start_width: -1.0, end_width: -1.0 }),
                20 => {
                    if let Some(x) = p.vertices.last_mut() {
                        x.at[1] = v.unwrap_or(0.0);
                    }
                }
                40 | 41 | 42 => {
                    if let Some(x) = p.vertices.last_mut() {
                        let v = v.unwrap_or(0.0);
                        match code {
                            40 => x.start_width = v,
                            41 => x.end_width = v,
                            _ => x.bulge = v,
                        }
                    }
                }
                _ => {}
            }
        }
        for x in &mut p.vertices {
            x.at[2] = p.elevation;
            if x.start_width < 0.0 {
                x.start_width = p.const_width;
            }
            if x.end_width < 0.0 {
                x.end_width = p.const_width;
            }
        }
        Ok(p)
    }

    /// POLYLINE and its VERTEXes up to SEQEND: a polyline or a mesh
    fn polyline(&mut self, w: Where) -> Result<Kind> {
        let flags = self.iv(70, 0);
        let elev = self.fv(30, 0.0);
        let (w0, w1) = (self.fv(40, 0.0), self.fv(41, 0.0));
        let (m, n) = (self.iv(71, 0), self.iv(72, 0));
        let max = self.lim.entity_points;
        let mut verts: Vec<(Vertex, i64, [i32; 4])> = Vec::new();
        while self.pair()? {
            if self.code() != 0 {
                self.take();
                continue;
            }
            let k = upper(self.p.value());
            if k == "VERTEX" {
                self.take();
                self.count(w, "VERTEX");
                self.gather(true)?;
                let s = &self.sub;
                let f = |c: i32, d: f64| (0..s.len()).take_while(|&i| s.codes[i] < 1000).find(|&i| s.codes[i] == c).and_then(|i| s.f(i)).unwrap_or(d);
                let iv = |c: i32| (0..s.len()).take_while(|&i| s.codes[i] < 1000).find(|&i| s.codes[i] == c).and_then(|i| s.int(i)).unwrap_or(0);
                if verts.len() < max {
                    let v = Vertex { at: [f(10, 0.0), f(20, 0.0), f(30, elev)], bulge: f(42, 0.0), start_width: f(40, w0), end_width: f(41, w1) };
                    let face = [iv(71).clamp(-1 << 30, 1 << 30) as i32, iv(72).clamp(-1 << 30, 1 << 30) as i32, iv(73).clamp(-1 << 30, 1 << 30) as i32,
                                iv(74).clamp(-1 << 30, 1 << 30) as i32];
                    verts.push((v, iv(70), face));
                }
                continue;
            }
            if k == "SEQEND" {
                self.take();
                self.count(w, "SEQEND");
                self.gather(true)?;
            }
            break;
        }
        if flags & 64 != 0 {
            let mut mesh = Mesh::default();
            for (v, vf, face) in verts {
                if vf & 64 != 0 || (vf & 128 != 0 && face == [0; 4]) {
                    mesh.vertices.push(v.at);
                } else if vf & 128 != 0 {
                    mesh.faces.push(face);
                }
            }
            return Ok(Kind::Mesh(mesh));
        }
        if flags & 16 != 0 {
            let mesh = Mesh { vertices: verts.iter().map(|v| v.0.at).collect(), faces: Vec::new(), m: m.clamp(0, 1 << 24) as u32, n: n.clamp(0, 1 << 24) as u32,
                              closed_m: flags & 1 != 0, closed_n: flags & 32 != 0 };
            return Ok(Kind::Mesh(mesh));
        }
        let three_d = flags & 8 != 0;
        // spline fitted: the frame's control points (16) are not drawn
        let vertices: Vec<Vertex> = verts.into_iter().filter(|(_, vf, _)| vf & 16 == 0).map(|(mut v, _, _)| {
            if three_d {
                v.bulge = 0.0;
                v.start_width = 0.0;
                v.end_width = 0.0;
            }
            v
        }).collect();
        Ok(Kind::Polyline(Polyline { vertices, closed: flags & 1 != 0, plinegen: flags & 128 != 0, three_d, const_width: w0, elevation: elev }))
    }

    fn text(&mut self, kind: &str) -> Result<Text> {
        let attr = kind != "TEXT";
        let b = &self.buf;
        // ATTRIB and ATTDEF: their vertical alignment is 74 (73 is the field length); an embedded MTEXT after 101
        let stop = (0..b.len()).find(|&i| b.codes[i] == 101 || b.codes[i] >= 1000).unwrap_or(b.len());
        let firstb = |c: i32| (0..stop).find(|&i| b.codes[i] == c);
        let f = |c: i32| firstb(c).and_then(|i| b.f(i));
        let iv = |c: i32| firstb(c).and_then(|i| b.int(i));
        let p = |c: i32| -> Option<P3> {
            let x = f(c)?;
            Some([x, f(c + 10).unwrap_or(0.0), f(c + 20).unwrap_or(0.0)])
        };
        let mut t = Text {
            text: firstb(1).map(|i| self.text_of(i)).unwrap_or_default(),
            style: None,
            at: p(10).unwrap_or([0.0; 3]),
            at2: p(11),
            height: f(40).unwrap_or(0.0),
            rotation: f(50).unwrap_or(0.0),
            width: f(41),
            oblique: f(51),
            generation: iv(71).map(|v| v.clamp(0, 0xFFFF) as u32),
            halign: iv(72).unwrap_or(0).clamp(0, 5) as u32,
            valign: iv(if attr { 74 } else { 73 }).unwrap_or(0).clamp(0, 3) as u32,
            tag: None,
            attr_flags: 0,
            mtext: None,
        };
        if attr {
            t.tag = firstb(2).map(|i| self.text_of(i));
            t.attr_flags = iv(70).unwrap_or(0).clamp(0, 0xFF) as u32;
        }
        let style = firstb(7).map(|i| self.buf.raw(i).to_vec());
        if let Some(raw) = style {
            t.style = Some(self.intern(&raw)?);
        }
        // R2018: an ATTRIB of several lines carries an MTEXT after "101 Embedded Object"
        if attr && stop < self.buf.len() && self.buf.codes[stop] == 101 {
            let b = &self.buf;
            let mut m = MText { spacing: 1.0, attach: 1, ..MText::default() };
            let mut txt = String::new();
            for i in stop + 1..b.len() {
                match b.codes[i] {
                    1 | 3 => txt.push_str(&self.dec.text(b.raw(i))),
                    10 => m.at[0] = b.f(i).unwrap_or(0.0),
                    20 => m.at[1] = b.f(i).unwrap_or(0.0),
                    40 => m.height = b.f(i).unwrap_or(0.0),
                    41 => m.width = b.f(i).unwrap_or(0.0),
                    71 => m.attach = b.int(i).unwrap_or(1).clamp(1, 9) as u32,
                    50 => m.rotation = b.f(i).unwrap_or(0.0),
                    44 => m.spacing = b.f(i).filter(|v| *v > 0.0).unwrap_or(1.0),
                    c if c >= 1000 => break,
                    _ => {}
                }
            }
            m.text = txt;
            m.style = t.style;
            if !m.text.is_empty() {
                t.mtext = Some(alloc::boxed::Box::new(m));
            }
        }
        Ok(t)
    }

    fn mtext(&mut self) -> Result<MText> {
        let b = &self.buf;
        let mut m = MText { attach: 1, direction: 1, spacing_style: 1, spacing: 1.0, ..MText::default() };
        let mut txt: Vec<u8> = Vec::new();
        let mut xd: Option<P3> = None;
        let mut columns = false;
        let mut embedded = false;
        let mut style: Option<Vec<u8>> = None;
        let mut i = 0;
        while i < b.len() {
            let code = b.codes[i];
            if code >= 1000 {
                break;
            }
            let v = b.f(i);
            if code == 101 {
                embedded = true;
                i += 1;
                continue;
            }
            if embedded {
                // R2018's columns after "101 Embedded Object"
                match code {
                    71 => m.column_type = b.int(i).unwrap_or(0).clamp(0, 2) as u32,
                    72 => m.column_count = b.int(i).unwrap_or(0).clamp(0, 1000) as u32,
                    44 => m.column_width = v.unwrap_or(0.0),
                    45 => m.column_gutter = v.unwrap_or(0.0),
                    46 if m.column_heights.len() < 1000 => m.column_heights.push(v.unwrap_or(0.0)),
                    _ => {}
                }
                i += 1;
                continue;
            }
            match code {
                1 | 3 => txt.extend_from_slice(b.raw(i)),
                7 if style.is_none() => style = Some(b.raw(i).to_vec()),
                10 => m.at[0] = v.unwrap_or(0.0),
                20 => m.at[1] = v.unwrap_or(0.0),
                30 => m.at[2] = v.unwrap_or(0.0),
                11 => xd = Some([v.unwrap_or(1.0), 0.0, 0.0]),
                21 => set(xd.as_mut(), 1, v),
                31 => set(xd.as_mut(), 2, v),
                40 => m.height = v.unwrap_or(0.0),
                41 => m.width = v.unwrap_or(0.0),
                46 => m.rect_height = v.unwrap_or(0.0),
                71 => m.attach = b.int(i).unwrap_or(1).clamp(1, 9) as u32,
                72 => m.direction = b.int(i).unwrap_or(1).clamp(0, 6) as u32,
                73 => m.spacing_style = b.int(i).unwrap_or(1).clamp(0, 2) as u32,
                44 => m.spacing = v.filter(|v| *v > 0.0).unwrap_or(1.0),
                50 if columns => {
                    if m.column_heights.len() < 1000 {
                        m.column_heights.push(v.unwrap_or(0.0));
                    }
                }
                50 => m.rotation = v.unwrap_or(0.0),
                90 => m.bg_flags = b.int(i).unwrap_or(0).clamp(0, 0xFFFF) as u32,
                63 => {
                    if let Some(c) = b.int(i).filter(|c| (1..=255).contains(c)) {
                        if !matches!(m.bg_color, Color::Rgb(_)) {
                            m.bg_color = Color::Aci(c as u8);
                        }
                    }
                }
                421 => m.bg_color = Color::Rgb((b.int(i).unwrap_or(0) & 0xFF_FFFF) as u32),
                45 => m.bg_scale = v.unwrap_or(1.5),
                75 => {
                    m.column_type = b.int(i).unwrap_or(0).clamp(0, 2) as u32;
                    columns = true;
                }
                76 => m.column_count = b.int(i).unwrap_or(0).clamp(0, 1000) as u32,
                48 => m.column_width = v.unwrap_or(0.0),
                49 => m.column_gutter = v.unwrap_or(0.0),
                _ => {}
            }
            i += 1;
        }
        // R2007's columns in XDATA: 1000 ACAD_MTEXT_COLUMN_INFO_BEGIN, then pairs of 1070 code / value
        let b = &self.buf;
        if let Some(start) = (0..b.len()).find(|&i| b.codes[i] == 1000 && trim(b.raw(i)) == b"ACAD_MTEXT_COLUMN_INFO_BEGIN") {
            let mut k = start + 1;
            while k + 1 < b.len() && !(b.codes[k] == 1000 && trim(b.raw(k)) == b"ACAD_MTEXT_COLUMN_INFO_END") {
                if b.codes[k] == 1070 {
                    let which = b.int(k).unwrap_or(0);
                    let val = b.f(k + 1).unwrap_or(0.0);
                    match which {
                        75 => m.column_type = (val as i64).clamp(0, 2) as u32,
                        76 => m.column_count = (val as i64).clamp(0, 1000) as u32,
                        48 => m.column_width = val,
                        49 => m.column_gutter = val,
                        50 if m.column_heights.len() < 1000 => m.column_heights.push(val),
                        _ => {}
                    }
                    k += 2;
                } else {
                    k += 1;
                }
            }
        }
        m.x_dir = xd;
        m.text = self.dec.text(&txt);
        if let Some(raw) = style {
            m.style = Some(self.intern(&raw)?);
        }
        Ok(m)
    }

    /// INSERT and its ATTRIBs up to SEQEND
    fn insert(&mut self, w: Where) -> Result<Insert> {
        let block = self.name_of(2)?.unwrap_or(Name(0));
        let cols = self.iv(70, 1);
        let rows = self.iv(71, 1);
        let mut ins = Insert {
            block,
            at: self.p3(10, [0.0; 3]),
            scale: [self.fv(41, 1.0), self.fv(42, 1.0), self.fv(43, 1.0)],
            rotation: self.fv(50, 0.0),
            columns: cols.clamp(1, 1 << 20) as u32,
            rows: rows.clamp(1, 1 << 20) as u32,
            spacing: [self.fv(44, 0.0), self.fv(45, 0.0)],
            attribs: Vec::new(),
        };
        let follow = self.iv(66, 0) != 0;
        // (the insert's own pairs are in buf; ATTRIBs are read into buf in turn, so its values are taken above)
        loop {
            if !self.pair()? || self.code() != 0 {
                break;
            }
            let k = upper(self.p.value());
            if k == "ATTRIB" {
                if !follow {
                    self.stats.note(String::from("ATTRIB after an INSERT without 66 = 1: taken as its attribute"));
                }
                self.take();
                self.count(w, "ATTRIB");
                self.gather(false)?;
                if let Some(e) = self.entity("ATTRIB", w)? {
                    if ins.attribs.len() < 65536 {
                        ins.attribs.push(e);
                    }
                }
                continue;
            }
            if k == "SEQEND" && (follow || !ins.attribs.is_empty()) {
                self.take();
                self.count(w, "SEQEND");
                self.gather(true)?;
            }
            break;
        }
        Ok(ins)
    }

    fn hatch(&mut self) -> Result<Hatch> {
        let b = &self.buf;
        let max = self.lim.entity_points;
        let mut h = Hatch { scale: 1.0, ..Hatch::default() };
        let n = b.len();
        let mut i = 0;
        let mut points = 0usize;
        // the head up to the paths (91)
        while i < n && b.codes[i] != 91 {
            match b.codes[i] {
                2 => h.pattern = self.dec.text(trim(b.raw(i))),
                70 => h.solid = b.int(i).unwrap_or(0) != 0,
                30 => h.elevation = b.f(i).unwrap_or(0.0),
                c if c >= 1000 => break,
                _ => {}
            }
            i += 1;
        }
        // the paths: 92 flags, then a polyline (72 bulges, 73 closed, 93 count, 10 20 [42]) or edges (93 count, 72 type ...)
        if i < n && b.codes[i] == 91 {
            i += 1;
            while i < n && b.codes[i] == 92 {
                let flags = b.int(i).unwrap_or(0).clamp(0, 0xFFFF) as u32;
                i += 1;
                let mut path = Path { flags, edges: Vec::new() };
                if flags & 2 != 0 {
                    let mut closed = false;
                    let mut verts: Vec<[f64; 3]> = Vec::new();
                    while i < n {
                        match b.codes[i] {
                            72 | 93 => {}
                            73 => closed = b.int(i).unwrap_or(0) != 0,
                            10 => {
                                points += 1;
                                if points > max {
                                    break;
                                }
                                verts.push([b.f(i).unwrap_or(0.0), 0.0, 0.0]);
                            }
                            20 => set(verts.last_mut(), 1, b.f(i)),
                            42 => set(verts.last_mut(), 2, b.f(i)),
                            _ => break,
                        }
                        i += 1;
                    }
                    path.edges.push(Edge::Poly { vertices: verts, closed });
                } else {
                    // 93: the number of edges (not trusted); edges begin with 72
                    if i < n && b.codes[i] == 93 {
                        i += 1;
                    }
                    while i < n && b.codes[i] == 72 {
                        let ty = b.int(i).unwrap_or(0);
                        i += 1;
                        points += 1;
                        if points > max {
                            break;
                        }
                        // the edge's own pairs up to the next 72, 97 or 92 (spline edges read in order)
                        let start = i;
                        let edge_end = |from: usize| {
                            let mut k = from;
                            while k < n && !matches!(b.codes[k], 72 | 97 | 92 | 75) {
                                k += 1;
                            }
                            k
                        };
                        let end = if ty == 4 { spline_edge_end(b, start) } else { edge_end(start) };
                        let g = |c: i32| (start..end).find(|&k| b.codes[k] == c).and_then(|k| b.f(k));
                        let gi = |c: i32| (start..end).find(|&k| b.codes[k] == c).and_then(|k| b.int(k));
                        let edge = match ty {
                            1 => Some(Edge::Line { a: [g(10).unwrap_or(0.0), g(20).unwrap_or(0.0)], b: [g(11).unwrap_or(0.0), g(21).unwrap_or(0.0)] }),
                            2 => Some(Edge::Arc { center: [g(10).unwrap_or(0.0), g(20).unwrap_or(0.0)], radius: g(40).unwrap_or(0.0), start: g(50).unwrap_or(0.0), end: g(51).unwrap_or(360.0),
                                                  ccw: gi(73).unwrap_or(1) != 0 }),
                            3 => Some(Edge::Ellipse { center: [g(10).unwrap_or(0.0), g(20).unwrap_or(0.0)], major: [g(11).unwrap_or(1.0), g(21).unwrap_or(0.0)], ratio: g(40).unwrap_or(1.0),
                                                      start: g(50).unwrap_or(0.0), end: g(51).unwrap_or(360.0), ccw: gi(73).unwrap_or(1) != 0 }),
                            4 => {
                                let mut knots = Vec::new();
                                let mut control: Vec<[f64; 2]> = Vec::new();
                                let mut weights = Vec::new();
                                let mut fit: Vec<[f64; 2]> = Vec::new();
                                for k in start..end {
                                    let v = b.f(k);
                                    let full = knots.len() + control.len() + fit.len() + points >= max;
                                    match b.codes[k] {
                                        40 if !full => knots.push(v.unwrap_or(0.0)),
                                        42 if !full => weights.push(v.unwrap_or(1.0)),
                                        10 if !full => control.push([v.unwrap_or(0.0), 0.0]),
                                        20 => {
                                            if let Some(c) = control.last_mut() {
                                                c[1] = v.unwrap_or(0.0);
                                            }
                                        }
                                        11 if !full => fit.push([v.unwrap_or(0.0), 0.0]),
                                        21 => {
                                            if let Some(c) = fit.last_mut() {
                                                c[1] = v.unwrap_or(0.0);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                points += knots.len() + control.len() + fit.len();
                                Some(Edge::Spline { degree: gi(94).unwrap_or(3).clamp(0, 64) as u32, rational: gi(73).unwrap_or(0) != 0, periodic: gi(74).unwrap_or(0) != 0, knots, control, weights, fit })
                            }
                            _ => None,
                        };
                        if let Some(e) = edge {
                            path.edges.push(e);
                        }
                        i = end;
                    }
                }
                // 97: the source objects, 330 each
                while i < n && matches!(b.codes[i], 97 | 330) {
                    i += 1;
                }
                if h.paths.len() < max {
                    h.paths.push(path);
                }
                if points > max {
                    return Err(Error::Limit(format!("line {}: a hatch of more than {max} edges and points", self.p.lines)));
                }
            }
        }
        // the rest: style, pattern, its lines (53 43 44 45 46 79 49...), seeds, gradient
        let mut cur: Option<PatternLine> = None;
        while i < n {
            let code = b.codes[i];
            if code >= 1000 {
                break;
            }
            let v = b.f(i);
            match code {
                75 => h.style = b.int(i).unwrap_or(0).clamp(0, 2) as u32,
                52 => h.angle = v.unwrap_or(0.0),
                41 => h.scale = v.unwrap_or(1.0),
                77 => h.double = b.int(i).unwrap_or(0) != 0,
                53 => {
                    if let Some(l) = cur.take() {
                        h.lines.push(l);
                    }
                    cur = Some(PatternLine { angle: v.unwrap_or(0.0), ..PatternLine::default() });
                }
                43 => set2(cur.as_mut().map(|l| &mut l.base), 0, v),
                44 => set2(cur.as_mut().map(|l| &mut l.base), 1, v),
                45 => set2(cur.as_mut().map(|l| &mut l.offset), 0, v),
                46 => set2(cur.as_mut().map(|l| &mut l.offset), 1, v),
                49 => {
                    if let Some(l) = cur.as_mut() {
                        points += 1;
                        if points <= max {
                            l.dashes.push(v.unwrap_or(0.0));
                        }
                    }
                }
                98 | 47 | 450.. => {
                    if let Some(l) = cur.take() {
                        h.lines.push(l);
                    }
                    if code == 450 {
                        h.gradient = b.int(i).unwrap_or(0) != 0;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if let Some(l) = cur.take() {
            h.lines.push(l);
        }
        // the gradient's colours: 421 (true colour) or 63 after 453
        if h.gradient {
            for k in 0..n {
                match b.codes[k] {
                    421 => h.gradient_colors.push(Color::Rgb((b.int(k).unwrap_or(0) & 0xFF_FFFF) as u32)),
                    63 if h.gradient_colors.len() < 2 => {
                        if let Some(c) = b.int(k).filter(|c| (1..=255).contains(c)) {
                            h.gradient_colors.push(Color::Aci(c as u8));
                        }
                    }
                    c if c >= 1000 => break,
                    _ => {}
                }
            }
        }
        if points > max {
            return Err(Error::Limit(format!("line {}: a hatch of more than {max} edges and points", self.p.lines)));
        }
        Ok(h)
    }

    fn leader(&mut self) -> Result<Leader> {
        let b = &self.buf;
        let max = self.lim.entity_points;
        let mut l = Leader { arrow: self.iv(71, 1) != 0, spline: self.iv(72, 0) != 0, ..Leader::default() };
        for i in 0..b.len() {
            match b.codes[i] {
                10 if l.vertices.len() < max => l.vertices.push([b.f(i).unwrap_or(0.0), 0.0, 0.0]),
                20 => set(l.vertices.last_mut(), 1, b.f(i)),
                30 => set(l.vertices.last_mut(), 2, b.f(i)),
                c if c >= 1000 => break,
                _ => {}
            }
        }
        l.dimstyle = self.name_of(3)?;
        Ok(l)
    }

    fn wipeout(&mut self) -> Result<Wipeout> {
        let b = &self.buf;
        let max = self.lim.entity_points;
        let mut w = Wipeout { at: self.p3(10, [0.0; 3]), u: self.p3(11, [1.0, 0.0, 0.0]), v: self.p3(12, [0.0, 1.0, 0.0]), size: [self.fv(13, 1.0), self.fv(23, 1.0)],
                              clip_type: self.iv(71, 1).clamp(0, 2) as u32, clip: Vec::new() };
        for i in 0..b.len() {
            match b.codes[i] {
                14 if w.clip.len() < max => w.clip.push([b.f(i).unwrap_or(0.0), 0.0]),
                24 => {
                    if let Some(c) = w.clip.last_mut() {
                        c[1] = b.f(i).unwrap_or(0.0);
                    }
                }
                c if c >= 1000 => break,
                _ => {}
            }
        }
        Ok(w)
    }
}

/// a spline edge's pairs end where its last expected item ends: after its fit data (97 n, then 11/21, 12/22, 13/23) or
/// at the next edge (72) or path (92) when it has none
fn spline_edge_end(b: &EntBuf, start: usize) -> usize {
    let n = b.len();
    let mut k = start;
    let mut fit = false;
    while k < n {
        match b.codes[k] {
            72 | 92 | 75 if !fit => return k,
            97 if !fit => {
                // 97 after the control points: the fit data's count (then 11, 12, 13), unless 330s follow (then it counts
                // the path's source objects and is not the edge's); a count of 0 belongs to the edge
                let next = b.codes.get(k + 1).copied().unwrap_or(0);
                if matches!(next, 11 | 12 | 13) {
                    fit = true;
                } else if next == 330 {
                    return k;
                } else {
                    return k + 1;
                }
            }
            11 | 21 | 12 | 22 | 13 | 23 if fit => {}
            _ if fit => return k,
            _ => {}
        }
        k += 1;
    }
    k
}

fn set(p: Option<&mut P3>, k: usize, v: Option<f64>) {
    if let Some(p) = p {
        p[k] = v.unwrap_or(0.0);
    }
}

fn set2(p: Option<&mut [f64; 2]>, k: usize, v: Option<f64>) {
    if let Some(p) = p {
        p[k] = v.unwrap_or(0.0);
    }
}

/// a true colour (420) beside an index (62): the index when the true colour only repeats its palette value (QCAD writes
/// white with 7, which is black on a light background), else the true colour
fn true_color(rgb: i64, aci: Option<i64>) -> Color {
    let v = (rgb & 0xFF_FFFF) as u32;
    match aci.map(|a| a.unsigned_abs()) {
        Some(a @ 1..=255) if crate::aci_rgb(a as u8) == v => Color::Aci(a as u8),
        _ => Color::Rgb(v),
    }
}

fn lineweight(v: i64) -> Lineweight {
    match v {
        -1 => Lineweight::ByLayer,
        -2 => Lineweight::ByBlock,
        -3 => Lineweight::Default,
        0..=211 => Lineweight::Mm100(v as u16),
        _ => Lineweight::Default,
    }
}

/// 440 / AcCmTransparency: 0x01000000 by block, 0x02000000 | alpha
fn transp(v: i64) -> Option<Transparency> {
    let v = v as u64 & 0xFFFF_FFFF;
    match v >> 24 {
        1 => Some(Transparency::ByBlock),
        2 => Some(Transparency::Alpha((v & 0xFF) as u8)),
        0 => Some(Transparency::ByLayer),
        _ => None,
    }
}

/// the points an entity holds (for the limits)
pub(crate) fn points_of(e: &Entity) -> usize {
    match &e.kind {
        Kind::Spline(s) => s.knots.len() + s.control.len() + s.fit.len() + s.weights.len(),
        Kind::Polyline(p) => p.vertices.len(),
        Kind::Mesh(m) => m.vertices.len() + m.faces.len(),
        Kind::Insert(i) => 1 + i.attribs.iter().map(points_of).sum::<usize>(),
        Kind::Hatch(h) => {
            h.paths.iter().map(|p| p.edges.iter().map(|e| match e {
                Edge::Poly { vertices, .. } => vertices.len(),
                Edge::Spline { knots, control, fit, weights, .. } => 1 + knots.len() + control.len() + fit.len() + weights.len(),
                _ => 1,
            }).sum::<usize>()).sum::<usize>() + h.lines.iter().map(|l| 1 + l.dashes.len()).sum::<usize>()
        }
        Kind::Leader(l) => l.vertices.len(),
        Kind::Wipeout(w) => w.clip.len(),
        _ => 1,
    }
}

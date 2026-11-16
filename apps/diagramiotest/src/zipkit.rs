// zipkit -- crafted ZIP archives for the hostile tests: records written as given, also with sizes
// that lie and central records that point at the same local record.  A trimmed copy of the
// builder in apps/codectest/archive_checks.rs (JD1), without the ZIP64 variants.

use alloc::vec::Vec;

use img_core::checksum::crc32;
use img_core::deflate::deflate_raw;

fn p16(z: &mut Vec<u8>, v: u16) {
    z.extend_from_slice(&v.to_le_bytes());
}
fn p32(z: &mut Vec<u8>, v: u32) {
    z.extend_from_slice(&v.to_le_bytes());
}

/// one record as it is to sit in the archive
#[derive(Clone)]
pub struct Rec {
    pub name: Vec<u8>,
    /// the payload as stored
    pub body: Vec<u8>,
    /// the uncompressed size the headers claim
    pub size: u64,
    pub crc: u32,
    pub method: u16,
}

/// a record of `data`, DEFLATE or stored
pub fn rec(name: &str, data: &[u8], deflate: bool) -> Rec {
    let body = if deflate { deflate_raw(data, 6) } else { data.to_vec() };
    Rec { name: name.as_bytes().to_vec(), body, size: data.len() as u64, crc: crc32(0, data), method: if deflate { 8 } else { 0 } }
}

/// the archive of `recs`; `central` lists for each central record the index of its local record
/// (None: one each, in order)
pub fn build(recs: &[Rec], central: Option<&[usize]>) -> Vec<u8> {
    let mut z = Vec::new();
    let mut locals = Vec::new();
    for r in recs {
        locals.push(z.len());
        p32(&mut z, 0x04034b50);
        p16(&mut z, 20);
        p16(&mut z, 0x800);
        p16(&mut z, r.method);
        p16(&mut z, 0);
        p16(&mut z, 33);
        p32(&mut z, r.crc);
        p32(&mut z, r.body.len() as u32);
        p32(&mut z, r.size as u32);
        p16(&mut z, r.name.len() as u16);
        p16(&mut z, 0);
        z.extend_from_slice(&r.name);
        z.extend_from_slice(&r.body);
    }
    let all: Vec<usize> = (0..recs.len()).collect();
    let order = central.unwrap_or(&all);
    let cd = z.len();
    for (k, &i) in order.iter().enumerate() {
        let r = &recs[i];
        p32(&mut z, 0x02014b50);
        p16(&mut z, 20);
        p16(&mut z, 20);
        p16(&mut z, 0x800);
        p16(&mut z, r.method);
        p16(&mut z, 0);
        p16(&mut z, 33);
        p32(&mut z, r.crc);
        p32(&mut z, r.body.len() as u32);
        p32(&mut z, r.size as u32);
        // a distinct name per central record, so only the overlap can refuse it
        let mut name = r.name.clone();
        if k > 0 && central.is_some() {
            name.extend_from_slice(alloc::format!(".{k}").as_bytes());
        }
        p16(&mut z, name.len() as u16);
        p16(&mut z, 0);
        p16(&mut z, 0);
        p16(&mut z, 0);
        p16(&mut z, 0);
        p32(&mut z, 0);
        p32(&mut z, locals[i] as u32);
        z.extend_from_slice(&name);
    }
    let cdsize = z.len() - cd;
    p32(&mut z, 0x06054b50);
    p16(&mut z, 0);
    p16(&mut z, 0);
    p16(&mut z, order.len() as u16);
    p16(&mut z, order.len() as u16);
    p32(&mut z, cdsize as u32);
    p32(&mut z, cd as u32);
    p16(&mut z, 0);
    z
}

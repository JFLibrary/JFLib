// boxes -- the JP2 file format (ITU-T T.800 Annex I; JPX's boxes of T.801 where JP2 readers
// meet them)
//
// The boxes: the signature, the file type, the JP2 header superbox with the image header, the
// colour specifications (enumerated, restricted or any ICC profile; the first one that is
// understood counts), the palette, the component mapping and the channel definitions, and the
// first contiguous codestream.  Other boxes (resolution, XML, UUID, JPX's composition and
// association boxes) are skipped.

use crate::Error;
use alloc::vec::Vec;

pub(crate) const SIGNATURE: [u8; 12] = [0, 0, 0, 12, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

pub(crate) enum Colr {
    Enum(u32),
    Icc(Vec<u8>),
}

pub(crate) struct Palette {
    /// per column: depth and signedness
    pub cols: Vec<(u8, bool)>,
    /// per column the entries
    pub entries: Vec<Vec<i32>>,
}

#[derive(Default)]
pub(crate) struct Jp2 {
    pub colr: Option<Colr>,
    pub palette: Option<Palette>,
    /// per channel: component, palette (true) or direct, palette column
    pub cmap: Vec<(u16, bool, u8)>,
    /// per entry: channel, type, association
    pub cdef: Vec<(u16, u16, u16)>,
}

/// the boxes of `d`: (type, content)
fn boxes(d: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut p = 0usize;
    core::iter::from_fn(move || {
        if p + 8 > d.len() {
            return None;
        }
        let len = u32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]) as u64;
        let ty = [d[p + 4], d[p + 5], d[p + 6], d[p + 7]];
        let (head, len) = match len {
            0 => (8u64, (d.len() - p) as u64),
            1 => {
                if p + 16 > d.len() {
                    return None;
                }
                let mut b = [0u8; 8];
                b.copy_from_slice(&d[p + 8..p + 16]);
                (16u64, u64::from_be_bytes(b))
            }
            l => (8u64, l),
        };
        if len < head {
            return None;
        }
        // len >= head >= 8 and p + 8 <= d.len(): the end moves forward, also for huge lengths
        let end = (p as u64).saturating_add(len).min(d.len() as u64) as usize;
        let body = &d[(p + head as usize).min(end)..end];
        p = end;
        Some((ty, body))
    })
}

fn be(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |v, &x| (v << 8) | x as u32)
}

fn header(d: &[u8], jp2: &mut Jp2) {
    for (ty, b) in boxes(d) {
        match &ty {
            b"colr" if jp2.colr.is_none() && b.len() >= 3 => match b[0] {
                1 if b.len() >= 7 => jp2.colr = Some(Colr::Enum(be(&b[3..7]))),
                2 | 3 => jp2.colr = Some(Colr::Icc(b[3..].to_vec())),
                _ => {}
            },
            b"pclr" if b.len() >= 3 => {
                let ne = ((b[0] as usize) << 8) | b[1] as usize;
                let npc = b[2] as usize;
                if ne == 0 || ne > 1024 || npc == 0 || b.len() < 3 + npc {
                    continue;
                }
                let cols: Vec<(u8, bool)> = b[3..3 + npc].iter().map(|&s| ((s & 0x7F) + 1, s & 0x80 != 0)).collect();
                if cols.iter().any(|c| c.0 > 32) {
                    continue;
                }
                let mut entries = alloc::vec![Vec::with_capacity(ne); npc];
                let mut p = 3 + npc;
                'e: for _ in 0..ne {
                    for (i, &(depth, signed)) in cols.iter().enumerate() {
                        let n = (depth as usize).div_ceil(8);
                        if p + n > b.len() {
                            break 'e;
                        }
                        let raw = be(&b[p..p + n]);
                        p += n;
                        let v = if signed && depth < 32 && raw >> (depth - 1) & 1 == 1 { raw as i64 - (1i64 << depth) } else { raw as i64 };
                        entries[i].push(v as i32);
                    }
                }
                if entries.iter().all(|e| e.len() == ne) {
                    jp2.palette = Some(Palette { cols, entries });
                }
            }
            b"cmap" => {
                jp2.cmap = b.chunks_exact(4).map(|c| ((((c[0] as u16) << 8) | c[1] as u16), c[2] == 1, c[3])).collect();
            }
            b"cdef" if b.len() >= 2 => {
                let n = ((b[0] as usize) << 8) | b[1] as usize;
                jp2.cdef = b[2..]
                    .chunks_exact(6)
                    .take(n)
                    .map(|c| ((((c[0] as u16) << 8) | c[1] as u16), (((c[2] as u16) << 8) | c[3] as u16), (((c[4] as u16) << 8) | c[5] as u16)))
                    .collect();
            }
            _ => {}
        }
    }
}

/// the codestream of a JP2 / JPX file and what its header says
pub(crate) fn parse(d: &[u8]) -> Result<(&[u8], Jp2), Error> {
    if !d.starts_with(&SIGNATURE) {
        return Err(Error::NotJpeg2000);
    }
    let mut jp2 = Jp2::default();
    let mut cs: Option<&[u8]> = None;
    for (ty, b) in boxes(d) {
        match &ty {
            b"jp2h" => header(b, &mut jp2),
            b"jp2c" if cs.is_none() => cs = Some(b),
            _ => {}
        }
    }
    Ok((cs.ok_or(Error::Truncated)?, jp2))
}

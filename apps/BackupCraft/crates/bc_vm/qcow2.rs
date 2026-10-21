// qcow2 -- what a qcow2 image holds, for the images QEMU's incremental backups write: no
// backing file, no encryption, no compressed clusters (those are refused)
//
//   Qcow2::open(path) / present() -> [(guest offset, Some(host offset) | None = zeros)]
//   read(host offset) -> one cluster
//
// Big-endian header: magic "QFI\xfb", version 2 or 3, cluster_bits at 20, size at 24, L1 size
// at 36, L1 offset at 40.  L1 entries point to L2 tables (bits 9..55); an L2 entry holds the
// cluster's host offset (bits 9..55), bit 62 = compressed, bit 0 = reads as zeros.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const MAGIC: u32 = 0x5146_49fb;
const OFFSET_MASK: u64 = 0x00ff_ffff_ffff_fe00;
const COMPRESSED: u64 = 1 << 62;
const ZERO: u64 = 1;

pub struct Qcow2 {
    f: plat::File,
    pub cluster_bits: u32,
    pub size: u64,
    l1: Vec<u64>,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn be64(b: &[u8], at: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[at..at + 8]);
    u64::from_be_bytes(x)
}

impl Qcow2 {
    pub fn open(path: &str) -> Result<Qcow2, String> {
        let f = plat::File::open(path, plat::FO_READ).ok_or_else(|| format!("{}: nicht lesbar", path))?;
        let mut h = [0u8; 104];
        if f.read_at(0, &mut h).is_none_or(|n| n < 72) {
            return Err(format!("{}: zu kurz", path));
        }
        let version = be32(&h, 4);
        if be32(&h, 0) != MAGIC || !(2..=3).contains(&version) {
            return Err(format!("{}: kein qcow2-Abbild", path));
        }
        if be64(&h, 8) != 0 {
            return Err(format!("{}: Abbild mit Basisdatei", path));
        }
        if be32(&h, 32) != 0 {
            return Err(format!("{}: verschlüsseltes Abbild", path));
        }
        if version == 3 && be64(&h, 72) & !0x9 != 0 {
            return Err(format!("{}: nicht unterstützte qcow2-Erweiterung", path));
        }
        let cluster_bits = be32(&h, 20);
        if !(9..=21).contains(&cluster_bits) {
            return Err(format!("{}: ungültige Clustergröße", path));
        }
        let size = be64(&h, 24);
        let l1_size = be32(&h, 36) as usize;
        let l1_off = be64(&h, 40);
        if l1_size > (1 << 24) {
            return Err(format!("{}: L1-Tabelle zu groß", path));
        }
        let mut raw = alloc::vec![0u8; l1_size * 8];
        if f.read_at(l1_off, &mut raw) != Some(raw.len()) {
            return Err(format!("{}: L1-Tabelle nicht lesbar", path));
        }
        let l1 = raw.chunks_exact(8).map(|c| be64(c, 0)).collect();
        Ok(Qcow2 { f, cluster_bits, size, l1 })
    }

    pub fn cluster(&self) -> u64 {
        1u64 << self.cluster_bits
    }

    /// the clusters the image holds, in order: (guest offset, host offset or None for zeros)
    pub fn present(&self) -> Result<Vec<(u64, Option<u64>)>, String> {
        let cs = self.cluster();
        let per_l2 = cs / 8;
        let mut out = Vec::new();
        let mut table = alloc::vec![0u8; cs as usize];
        for (i, e) in self.l1.iter().enumerate() {
            let l2_off = e & OFFSET_MASK;
            if l2_off == 0 {
                continue;
            }
            if self.f.read_at(l2_off, &mut table) != Some(table.len()) {
                return Err(String::from("L2-Tabelle nicht lesbar"));
            }
            for k in 0..per_l2 as usize {
                let x = be64(&table, k * 8);
                if x == 0 {
                    continue;
                }
                if x & COMPRESSED != 0 {
                    return Err(String::from("komprimierte Cluster werden nicht gelesen"));
                }
                let guest = (i as u64 * per_l2 + k as u64) * cs;
                if guest >= self.size {
                    continue;
                }
                let host = x & OFFSET_MASK;
                if x & ZERO != 0 || host == 0 {
                    out.push((guest, None));
                } else {
                    out.push((guest, Some(host)));
                }
            }
        }
        Ok(out)
    }

    /// one cluster's data
    pub fn read(&self, host: u64) -> Result<Vec<u8>, String> {
        let mut b = alloc::vec![0u8; self.cluster() as usize];
        match self.f.read_at(host, &mut b) {
            Some(n) if n == b.len() => Ok(b),
            _ => Err(String::from("Cluster nicht lesbar")),
        }
    }
}

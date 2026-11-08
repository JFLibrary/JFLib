// A SquashFS 4.0 image of a tree, as an AppImage carries it behind its runtime: data blocks of
// 128 KiB compressed with LZ4 (stored when that is not smaller), no fragments, no xattrs; the
// inode, directory and id tables in metadata blocks of 8 KiB stored uncompressed (their sizes are
// then known before their contents, which point at each other).  Owner root, one id.  Read by
// JFLib's AppImage runtime, unsquashfs, squashfuse and the kernel (squashfs-tools, squashfs_fs.h).

use std::collections::BTreeMap;

use crate::lz4;

const BLOCK: usize = 128 * 1024;
const META: usize = 8192;
const META_STORED: usize = META + 2; // a full metadata block with its header
const COMP_LZ4: u16 = 5;
const LZ4_LEGACY: u32 = 1;
const FLAGS: u16 = 0x0001 | 0x0010 | 0x0200 | 0x0400 | 0x0800; // inodes and ids uncompressed, no fragments, no xattrs, compressor options

pub(crate) enum Item {
    File { data: Vec<u8>, mode: u16 },
    Link(String),
}

enum Kind {
    Dir { children: Vec<(String, usize)>, parent: usize },
    File { start: u64, size: usize, blocks: Vec<u32>, mode: u16 },
    Link(String),
}

fn le16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn le32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// a stream of metadata as stored blocks: each 8 KiB behind its header (bit 15: uncompressed)
fn metadata(stream: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(stream.len() + stream.len() / META * 2 + 2);
    for chunk in stream.chunks(META) {
        le16(&mut out, chunk.len() as u16 | 0x8000);
        out.extend_from_slice(chunk);
    }
    out
}

/// where byte `pos` of a metadata stream lies: (start of its block in the table, offset in it)
fn locate(pos: usize) -> (u32, u16) {
    ((pos / META * META_STORED) as u32, (pos % META) as u16)
}

/// the image of `items` (paths with '/', folders made as needed), every inode with `mtime`
pub(crate) fn image(items: &BTreeMap<String, Item>, mtime: u32) -> Result<Vec<u8>, String> {
    // the nodes, the root first; a folder's children sorted by name (the readers look up names in order)
    let mut nodes = vec![Kind::Dir { children: Vec::new(), parent: 0 }];
    let mut dirs: BTreeMap<String, usize> = BTreeMap::from([(String::new(), 0)]);
    let mut out = vec![0u8; 96];
    out.extend(metadata(&[LZ4_LEGACY.to_le_bytes(), 0u32.to_le_bytes()].concat()));
    for (path, item) in items {
        let mut parent = 0;
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let Some((name, folders)) = parts.split_last() else { continue };
        let mut sofar = String::new();
        for f in folders {
            sofar = if sofar.is_empty() { f.to_string() } else { format!("{sofar}/{f}") };
            parent = match dirs.get(&sofar) {
                Some(&d) => d,
                None => {
                    nodes.push(Kind::Dir { children: Vec::new(), parent });
                    let d = nodes.len() - 1;
                    if let Kind::Dir { children, .. } = &mut nodes[parent] {
                        children.push((f.to_string(), d));
                    }
                    dirs.insert(sofar.clone(), d);
                    d
                }
            };
        }
        let kind = match item {
            Item::File { data, mode } => {
                let start = out.len() as u64;
                let mut blocks = Vec::new();
                for chunk in data.chunks(BLOCK) {
                    let z = lz4::compress(chunk);
                    if z.len() < chunk.len() {
                        blocks.push(z.len() as u32);
                        out.extend(z);
                    } else {
                        blocks.push(chunk.len() as u32 | 1 << 24);
                        out.extend_from_slice(chunk);
                    }
                }
                Kind::File { start, size: data.len(), blocks, mode: *mode }
            }
            Item::Link(target) => Kind::Link(target.clone()),
        };
        nodes.push(kind);
        let n = nodes.len() - 1;
        if let Kind::Dir { children, .. } = &mut nodes[parent] {
            children.push((name.to_string(), n));
        }
    }
    for n in &mut nodes {
        if let Kind::Dir { children, .. } = n {
            children.sort();
            if children.windows(2).any(|w| w[0].0 == w[1].0) {
                return Err("squashfs: a name twice in one folder".to_string());
            }
        }
    }
    // where each inode lies in the inode stream (its size is known before its contents)
    let size = |k: &Kind| match k {
        Kind::Dir { .. } => 32,
        Kind::File { blocks, .. } => 32 + 4 * blocks.len(),
        Kind::Link(t) => 24 + t.len(),
    };
    let mut ipos = Vec::with_capacity(nodes.len());
    let mut at = 0;
    for k in &nodes {
        ipos.push(at);
        at += size(k);
    }
    let number = |i: usize| i as u32 + 1;
    // the folders' listings: entries grouped behind headers (one inode block, numbers in i16 reach)
    let mut dstream = Vec::new();
    let mut listing = vec![(0usize, 0usize); nodes.len()];
    for (i, k) in nodes.iter().enumerate() {
        let Kind::Dir { children, .. } = k else { continue };
        let start = dstream.len();
        let mut k0 = 0;
        while k0 < children.len() {
            let (block, _) = locate(ipos[children[k0].1]);
            let base = number(children[k0].1);
            let group: Vec<&(String, usize)> = children[k0..].iter()
                .take_while(|(_, c)| locate(ipos[*c]).0 == block && (number(*c) as i64 - base as i64).abs() < 32768)
                .take(256)
                .collect();
            le32(&mut dstream, group.len() as u32 - 1);
            le32(&mut dstream, block);
            le32(&mut dstream, base);
            for (name, c) in &group {
                if name.is_empty() || name.len() > 256 {
                    return Err(format!("squashfs: the name {name:?} is empty or too long"));
                }
                le16(&mut dstream, locate(ipos[*c]).1);
                le16(&mut dstream, (number(*c) as i64 - base as i64) as i16 as u16);
                le16(&mut dstream, match nodes[*c] { Kind::Dir { .. } => 1, Kind::File { .. } => 2, Kind::Link(_) => 3 });
                le16(&mut dstream, name.len() as u16 - 1);
                dstream.extend_from_slice(name.as_bytes());
            }
            k0 += group.len();
        }
        listing[i] = (start, dstream.len() - start);
        if dstream.len() - start + 3 > 65535 {
            return Err("squashfs: a folder with too many entries for a basic directory inode".to_string());
        }
    }
    // the inodes
    let mut istream = Vec::with_capacity(at);
    for (i, k) in nodes.iter().enumerate() {
        let (kind, mode) = match k {
            Kind::Dir { .. } => (1u16, 0o755u16),
            Kind::File { mode, .. } => (2, *mode),
            Kind::Link(_) => (3, 0o777),
        };
        for v in [kind, mode & 0o7777, 0, 0] {
            le16(&mut istream, v);
        }
        le32(&mut istream, mtime);
        le32(&mut istream, number(i));
        match k {
            Kind::Dir { children, parent } => {
                let (block, offset) = locate(listing[i].0);
                le32(&mut istream, block);
                le32(&mut istream, 2 + children.iter().filter(|(_, c)| matches!(nodes[*c], Kind::Dir { .. })).count() as u32);
                le16(&mut istream, (listing[i].1 + 3) as u16);
                le16(&mut istream, offset);
                le32(&mut istream, if i == 0 { nodes.len() as u32 + 1 } else { number(*parent) });
            }
            Kind::File { start, size, blocks, .. } => {
                let start = u32::try_from(*start).map_err(|_| "squashfs: an image over 4 GiB".to_string())?;
                le32(&mut istream, start);
                le32(&mut istream, u32::MAX); // no fragment
                le32(&mut istream, 0);
                le32(&mut istream, *size as u32);
                for &b in blocks {
                    le32(&mut istream, b);
                }
            }
            Kind::Link(target) => {
                le32(&mut istream, 1);
                le32(&mut istream, target.len() as u32);
                istream.extend_from_slice(target.as_bytes());
            }
        }
    }
    debug_assert_eq!(istream.len(), at);
    let inode_table = out.len() as u64;
    out.extend(metadata(&istream));
    let directory_table = out.len() as u64;
    out.extend(metadata(&dstream));
    let ids = out.len() as u64; // also where a fragment table would start
    out.extend(metadata(&0u32.to_le_bytes()));
    let id_table = out.len() as u64;
    out.extend_from_slice(&ids.to_le_bytes());
    let used = out.len() as u64;
    let (rb, ro) = locate(ipos[0]);
    let mut sb = Vec::with_capacity(96);
    for v in [0x7371_7368u32, nodes.len() as u32, mtime, BLOCK as u32, 0] {
        le32(&mut sb, v);
    }
    for v in [COMP_LZ4, 17, FLAGS, 1, 4, 0] {
        le16(&mut sb, v);
    }
    for v in [(rb as u64) << 16 | ro as u64, used, id_table, u64::MAX, inode_table, directory_table, ids, u64::MAX] {
        sb.extend_from_slice(&v.to_le_bytes());
    }
    out[..96].copy_from_slice(&sb);
    out.resize(out.len().div_ceil(4096) * 4096, 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let mut items = BTreeMap::new();
        items.insert("usr/bin/x".to_string(), Item::File { data: vec![b'x'; 300_000], mode: 0o755 });
        items.insert("usr/share/a.txt".to_string(), Item::File { data: b"hallo".to_vec(), mode: 0o644 });
        items.insert("AppRun".to_string(), Item::Link("usr/bin/x".to_string()));
        let img = image(&items, 0).unwrap();
        assert_eq!(&img[..4], b"hsqs");
        assert_eq!(img.len() % 4096, 0);
        let u = |at: usize| u64::from_le_bytes(img[at..at + 8].try_into().unwrap()) as usize;
        assert_eq!(u32::from_le_bytes(img[4..8].try_into().unwrap()), 7); // root, AppRun, usr, bin, x, share, a.txt
        let (used, ids_at, inodes, dirs) = (u(40), u(48), u(64), u(72));
        assert!(inodes < dirs && dirs < ids_at && ids_at < used);
        assert_eq!(u(ids_at), u(80)); // the id index points at the id block, where fragments would start
        // three blocks of x: 128 KiB, 128 KiB, the rest
        assert_eq!(&img[96..98], &(0x8008u16).to_le_bytes());
    }
}

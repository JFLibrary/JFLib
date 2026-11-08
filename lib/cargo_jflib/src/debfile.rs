// A .deb without dpkg-deb: an ar archive of debian-binary, control.tar.gz and data.tar.gz, the
// tar files in the ustar format with the owner root, folders 0755, programs (ELF, #!) and the
// maintainer scripts 0755, everything else 0644 -- what `dpkg-deb --root-owner-group --build`
// makes of a tree (tools/linux_task.py, fix_modes).

use std::collections::BTreeMap;

use crate::deflate;

/// the files of one tar: path below "./" -> contents (folders are added for every path)
pub(crate) type Tree = BTreeMap<String, Vec<u8>>;

fn octal(field: &mut [u8], v: u64) {
    let s = format!("{:0w$o}", v, w = field.len() - 1);
    field[..s.len()].copy_from_slice(s.as_bytes());
}

fn header(name: &str, size: u64, mode: u32, kind: u8, mtime: u64) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    // ustar: up to 100 bytes of name, the rest of a longer path in the 155 bytes of prefix
    let b = name.as_bytes();
    if b.len() <= 100 {
        h[..b.len()].copy_from_slice(b);
    } else if let Some(cut) = (0..b.len()).rev().find(|&i| b[i] == b'/' && i <= 155 && b.len() - i - 1 <= 100) {
        h[345..345 + cut].copy_from_slice(&b[..cut]);
        h[..b.len() - cut - 1].copy_from_slice(&b[cut + 1..]);
    } else {
        unreachable!("long names are written as GNU long links first");
    }
    octal(&mut h[100..108], mode as u64);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], mtime);
    h[148..156].fill(b' ');
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[265..269].copy_from_slice(b"root");
    h[297..301].copy_from_slice(b"root");
    let sum: u32 = h.iter().map(|&x| x as u32).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    h
}

fn fits(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() <= 100 || (0..b.len()).any(|i| b[i] == b'/' && i <= 155 && b.len() - i - 1 <= 100)
}

fn entry(out: &mut Vec<u8>, name: &str, data: &[u8], mode: u32, kind: u8, mtime: u64) {
    if !fits(name) {
        // GNU tar's long name: an entry ././@LongLink of type L holding the name
        let mut long = name.as_bytes().to_vec();
        long.push(0);
        out.extend(header("././@LongLink", long.len() as u64, 0o644, b'L', mtime));
        out.extend_from_slice(&long);
        out.resize(out.len().div_ceil(512) * 512, 0);
        let mut cut = name.len().min(99);
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        out.extend(header(&name[..cut], data.len() as u64, mode, kind, mtime));
    } else {
        out.extend(header(name, data.len() as u64, mode, kind, mtime));
    }
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(512) * 512, 0);
}

fn executable(name: &str, data: &[u8], scripts: bool) -> bool {
    data.starts_with(b"\x7fELF") || data.starts_with(b"#!") || (scripts && matches!(name, "config" | "postinst" | "prerm" | "postrm" | "preinst"))
}

/// a tar of `tree` below "./" with every folder on the way
pub(crate) fn tar(tree: &Tree, mtime: u64, scripts: bool) -> Vec<u8> {
    let mut dirs = std::collections::BTreeSet::new();
    for path in tree.keys() {
        let mut d = String::new();
        for part in path.split('/').rev().skip(1).collect::<Vec<_>>().into_iter().rev() {
            d.push_str(part);
            d.push('/');
            dirs.insert(d.clone());
        }
    }
    // folders and files in path order, each folder before what lies in it
    let mut all: Vec<(String, Option<&Vec<u8>>)> = dirs.into_iter().map(|d| (d, None)).collect();
    all.extend(tree.iter().map(|(p, d)| (p.clone(), Some(d))));
    all.sort_by(|a, b| a.0.trim_end_matches('/').split('/').cmp(b.0.trim_end_matches('/').split('/')));
    let mut out = Vec::new();
    entry(&mut out, "./", &[], 0o755, b'5', mtime);
    for (path, data) in all {
        match data {
            None => entry(&mut out, &format!("./{path}"), &[], 0o755, b'5', mtime),
            Some(d) => {
                let mode = if executable(path.rsplit('/').next().unwrap_or(&path), d, scripts) { 0o755 } else { 0o644 };
                entry(&mut out, &format!("./{path}"), d, mode, b'0', mtime);
            }
        }
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// the .deb of a package: its control files (DEBIAN/) and its files
pub(crate) fn deb(control: &Tree, data: &Tree, mtime: u64) -> Vec<u8> {
    let members: [(&str, Vec<u8>); 3] = [
        ("debian-binary", b"2.0\n".to_vec()),
        ("control.tar.gz", deflate::gzip(&tar(control, mtime, true))),
        ("data.tar.gz", deflate::gzip(&tar(data, mtime, false))),
    ];
    let mut out = b"!<arch>\n".to_vec();
    for (name, body) in members {
        out.extend(format!("{name:<16}{mtime:<12}0     0     100644  {:<10}`\n", body.len()).into_bytes());
        out.extend_from_slice(&body);
        if body.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_layout() {
        let mut t = Tree::new();
        t.insert("usr/bin/x".into(), b"\x7fELF....".to_vec());
        t.insert("usr/share/doc/x/a-very-long-folder-name-that-goes-on-and-on/and-on-and-on/and-then-some-more/LICENSE-APACHE".into(), b"text".to_vec());
        t.insert(format!("usr/{}/f", "d".repeat(160)), b"long".to_vec());
        let tar = tar(&t, 0, false);
        assert_eq!(tar.len() % 512, 0);
        assert_eq!(&tar[..2], b"./");
        assert_eq!(&tar[257..262], b"ustar");
        let names: Vec<String> = tar.chunks(512).filter(|h| &h[257..262] == b"ustar")
            .map(|h| String::from_utf8_lossy(&h[..100]).trim_end_matches('\0').to_string()).collect();
        assert!(names.iter().position(|n| n == "./usr/").unwrap() < names.iter().position(|n| n == "./usr/bin/x").unwrap());
        assert!(names.iter().any(|n| n == "././@LongLink"));
        // the checksum: the header's bytes with the field as blanks
        let h = &tar[..512];
        let mut sum: u32 = h.iter().map(|&x| x as u32).sum();
        sum -= h[148..156].iter().map(|&x| x as u32).sum::<u32>();
        sum += 8 * b' ' as u32;
        assert_eq!(format!("{sum:06o}"), String::from_utf8_lossy(&h[148..154]));
    }
}

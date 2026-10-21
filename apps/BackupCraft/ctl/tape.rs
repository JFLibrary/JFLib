// tape -- bcctl reads tapes without a server: the sets on tape files, and their files into a
// folder (encrypted sets with the recovery kit)
//
//   bcctl tape list FILE...                          the sets on the tapes
//   bcctl tape extract FILE... --to DIR [--set ID] [--kit FILE]
//                                                    a set's files into DIR (the only set when
//                                                    there is one); every file is checked
//                                                    against its BLAKE3 on the tape; existing
//                                                    files are not replaced
//
// The tapes of a set may be given in any order; its parts are put together by their numbers.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::j::{self, Json};
use bc_tape::{self as bt, SetBegin, SetEnd, SetKey};

use crate::{eprint, opt, out, size, when};

/// a set seen on the tapes: its start, its parts (part -> file, position), whether it ends
struct Seen {
    head: SetBegin,
    parts: BTreeMap<u32, (String, u64)>,
    end: Option<SetEnd>,
}

fn scan(files: &[String]) -> Result<BTreeMap<String, Seen>, String> {
    let mut sets: BTreeMap<String, Seen> = BTreeMap::new();
    for f in files {
        let tape = bt::Tape::open(f, false)?;
        let mut pos = tape.read(0)?.map_or(0, |b| b.next);
        let mut cur: Option<String> = None;
        while let Some(b) = tape.read(pos)? {
            pos = b.next;
            match b.kind {
                bt::K_SET_BEGIN => {
                    let h: SetBegin = b.json().ok_or_else(|| format!("{}: unlesbarer Satzanfang", f))?;
                    let s = sets.entry(h.set_id.clone()).or_insert_with(|| Seen { head: h.clone(), parts: BTreeMap::new(), end: None });
                    if h.part == 1 {
                        s.head = h.clone();
                    }
                    s.parts.insert(h.part, (f.clone(), b.pos));
                    cur = Some(h.set_id);
                }
                bt::K_SET_END => {
                    if let (Some(id), false) = (cur.take(), b.sealed) {
                        if let (Some(s), Some(e)) = (sets.get_mut(&id), b.json::<SetEnd>()) {
                            if e.last {
                                s.end = Some(e);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(sets)
}

fn whole(s: &Seen) -> bool {
    s.parts.keys().enumerate().all(|(k, p)| *p as usize == k + 1)
}

pub fn cmd(a: &[String], pos: &[String]) -> i32 {
    let files: Vec<String> = pos.iter().skip(1).cloned().collect();
    match pos.first().map(String::as_str) {
        Some("list") if !files.is_empty() => match scan(&files) {
            Ok(sets) => {
                for (id, s) in &sets {
                    let h = &s.head;
                    let parts: Vec<String> = s.parts.iter().map(|(p, (f, _))| format!("{}: {}", p, f.rsplit(['/', '\\']).next().unwrap_or(f))).collect();
                    out(&format!("Satz {}  {}  Restore-Point vom {}  ({})\n", id, h.job_name, when(h.point_time), h.host_name));
                    out(&format!("  Pool {}{}, Teile {}\n", h.pool_name, if h.encrypted { ", verschlüsselt" } else { "" }, parts.join(", ")));
                    match &s.end {
                        Some(e) if whole(s) => out(&format!("  {} Dateien, {}, vollständig\n", e.files, size(e.bytes))),
                        _ if h.encrypted => out(&format!("  {} Teile vorhanden (Inhalt verschlüsselt)\n", s.parts.len())),
                        _ => out("  unvollständig: es fehlen Teile\n"),
                    }
                }
                if sets.is_empty() {
                    out("Keine Sätze auf diesen Bändern\n");
                }
                0
            }
            Err(e) => {
                eprint(&format!("bcctl: {}\n", e));
                1
            }
        },
        Some("extract") if !files.is_empty() => {
            let Some(to) = opt(a, "--to") else {
                eprint("bcctl tape extract FILE... --to DIR [--set ID] [--kit FILE]\n");
                return 2;
            };
            match extract(&files, &to, opt(a, "--set"), opt(a, "--kit")) {
                Ok(msg) => {
                    out(&format!("{}\n", msg));
                    0
                }
                Err(e) => {
                    eprint(&format!("bcctl: {}\n", e));
                    1
                }
            }
        }
        _ => {
            eprint("bcctl tape list FILE... | tape extract FILE... --to DIR [--set ID] [--kit FILE]\n");
            2
        }
    }
}

/// the recovery kit's private key (BCRK1-<base64url>)
fn kit_secret(file: &str) -> Result<[u8; 32], String> {
    let b = plat::read_file(file).ok_or_else(|| format!("{} ist nicht lesbar", file))?;
    let t = String::from_utf8_lossy(&b).into_owned();
    let k = t.split_whitespace().find_map(|w| w.strip_prefix("BCRK1-")).ok_or_else(|| format!("{} enthält kein Recovery-Kit (BCRK1-…)", file))?;
    let raw = crypto::b64::decode_url(k).ok_or("Das Recovery-Kit ist beschädigt")?;
    <[u8; 32]>::try_from(raw.as_slice()).map_err(|_| String::from("Das Recovery-Kit ist beschädigt"))
}

/// "C:\Daten\x" -> ["C", "Daten", "x"]; "/srv/x" -> ["srv", "x"]
fn parts_of(path: &str) -> Vec<String> {
    let t = path.replace('\\', "/");
    let t = t.trim_start_matches('/');
    let t = if t.len() >= 2 && t.as_bytes()[1] == b':' { format!("{}{}", &t[..1], &t[2..]) } else { String::from(t) };
    t.split('/').filter(|p| !p.is_empty() && *p != "." && *p != "..").map(String::from).collect()
}

fn extract(files: &[String], to: &str, want: Option<String>, kit: Option<String>) -> Result<String, String> {
    let sets = scan(files)?;
    let id = match want {
        Some(w) => sets.keys().find(|k| k.starts_with(&w)).cloned().ok_or_else(|| format!("Satz {} ist nicht auf diesen Bändern", w))?,
        None if sets.len() == 1 => sets.keys().next().cloned().unwrap_or_default(),
        None => return Err(String::from("Mehrere Sätze auf den Bändern: bitte --set ID angeben (bcctl tape list)")),
    };
    let s = &sets[&id];
    if !whole(s) {
        return Err(format!("Satz {}: es fehlen Teile (vorhanden: {:?})", id, s.parts.keys().collect::<Vec<_>>()));
    }
    let h = &s.head;
    let key = if h.encrypted {
        let sk = kit_secret(&kit.ok_or("Der Satz ist verschlüsselt: bitte --kit FILE mit dem Recovery-Kit angeben")?)?;
        let sealed = crypto::b64::decode_url(&h.recovery).filter(|_| !h.recovery.is_empty()).ok_or("Der Satz wurde ohne Recovery-Kit geschrieben: nur der Server kann ihn öffnen")?;
        let k = crypto::sealbox::open(&sk, bt::AAD_RECOVERY, &sealed).ok_or("Dieses Recovery-Kit öffnet den Satz nicht")?;
        let k = <[u8; 32]>::try_from(k.as_slice()).map_err(|_| String::from("beschädigter Schlüssel"))?;
        Some(SetKey::for_set(k, h).ok_or("Der Schlüssel passt nicht zum Satz")?)
    } else {
        None
    };
    let win = cfg!(windows);
    bc_fs::ensure_dir(to)?;
    let (mut written, mut skipped, mut bad, mut bytes) = (0u64, 0u64, 0u64, 0u64);
    let mut dirs: Vec<(String, bt::Entry)> = Vec::new();
    // the file being written: entry, temp, final name, file, hash
    let mut cur: Option<(bt::Entry, String, String, plat::File, crypto::blake3::Blake3)> = None;
    for (p, (file, at)) in &s.parts {
        let tape = bt::Tape::open(file, false)?;
        let b = tape.read(*at)?.ok_or("Satzanfang fehlt")?;
        let head: SetBegin = b.json().ok_or("unlesbarer Satzanfang")?;
        let mut n = head.block0;
        let mut pos = b.next;
        loop {
            let Some(b) = tape.read(pos)? else { return Err(format!("Teil {}: der Satz endet unerwartet", p)) };
            pos = b.next;
            let plain = match (&key, b.sealed) {
                (Some(k), true) => k.open(n, b.kind, &b.payload).ok_or_else(|| format!("Block {} lässt sich nicht entschlüsseln", n))?,
                (None, false) => b.payload,
                _ => return Err(String::from("Block passt nicht zur Verschlüsselung")),
            };
            n += 1;
            match b.kind {
                bt::K_ENTRY => {
                    let e = j::parse(&plain).and_then(|v| bt::Entry::from_json(&v)).ok_or("unlesbarer Eintrag")?;
                    if let Some((_, tmp, _, f, _)) = cur.take() {
                        drop(f);
                        plat::delete(&tmp);
                    }
                    let full = bt::full_path(&h.roots, &e, h.windows);
                    let names = parts_of(&full);
                    if names.iter().any(|x| !bc_fs::safe_name(x, win)) {
                        eprint(&format!("übersprungen (unzulässiger Name): {}\n", full));
                        continue;
                    }
                    let mut dest = String::from(to);
                    for x in &names {
                        dest = bc_fs::join_os(&dest, x, win);
                    }
                    match e.k.as_str() {
                        "dir" => {
                            bc_fs::ensure_dir(&dest)?;
                            dirs.push((dest, e));
                        }
                        "file" => {
                            let parent = plat::fs::parent(&dest);
                            if !parent.is_empty() {
                                bc_fs::ensure_dir(parent)?;
                            }
                            if plat::fs::lstat(&dest).is_some() {
                                skipped += 1;
                                continue;
                            }
                            let tmp = bc_fs::temp_name(&dest);
                            let f = plat::File::create_new(&plat::fs::long_path(&tmp)).ok_or_else(|| format!("{}: kann nicht geschrieben werden", dest))?;
                            cur = Some((e, tmp, dest, f, crypto::blake3::Blake3::new()));
                        }
                        "link" => {
                            if plat::fs::lstat(&dest).is_none() {
                                if let Err(x) = bc_fs::make_link(&e.link, &dest, false, e.attrs & bc_fs::ATTR_DIRECTORY != 0) {
                                    eprint(&format!("{}: {}\n", dest, x));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                bt::K_DATA => {
                    let (i, off, data) = bt::parse_data(&plain).ok_or("unlesbarer Datenblock")?;
                    if let Some((e, _, dest, f, hash)) = &mut cur {
                        if e.i == i {
                            if f.write_at(off, data) != Some(data.len()) {
                                return Err(format!("{}: Schreibfehler (Speicher voll?)", dest));
                            }
                            hash.update(data);
                            bytes += data.len() as u64;
                        }
                    }
                }
                bt::K_FILE_END => {
                    let fe = j::parse(&plain).and_then(|v| bt::FileEnd::from_json(&v)).ok_or("unlesbares Dateiende")?;
                    if let Some((e, tmp, dest, f, hash)) = cur.take() {
                        if e.i != fe.i || fe.hash.is_empty() || crypto::hex(&hash.finalize()) != fe.hash {
                            drop(f);
                            plat::delete(&tmp);
                            eprint(&format!("{}: die Daten auf dem Band weichen ab\n", dest));
                            bad += 1;
                        } else {
                            bc_fs::finish_file(f, &tmp, &dest, e.atime, e.mtime, e.mode, e.attrs)?;
                            written += 1;
                        }
                    }
                }
                bt::K_SET_END => break,
                _ => {}
            }
        }
    }
    for (d, e) in dirs.iter().rev() {
        bc_fs::apply_meta(d, e.atime, e.mtime, e.mode, 0);
    }
    let msg = format!("{} Dateien ({}) nach {} geschrieben, {} vorhandene übersprungen", written, size(bytes), to, skipped);
    if bad > 0 {
        return Err(format!("{}; {} Dateien weichen ab", msg, bad));
    }
    Ok(msg)
}

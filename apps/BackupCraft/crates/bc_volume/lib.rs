// bc_volume -- whole volumes block by block: the volumes of this computer, a consistent view of
// one (a snapshot), reading it
//
//   list() -> [VolumeInfo]          Windows: the volumes with a drive letter (Get-Volume);
//                                   Linux: block devices with a file system (lsblk)
//   snapshot(volume) -> Snap        Windows: a VSS shadow copy (Win32_ShadowCopy, administrator);
//                                   Linux: an LVM snapshot when the volume is a logical volume
//                                   (lvcreate -s, root); a regular file (an image) is read as it
//                                   is; any other device without a snapshot (the result says so)
//   Snap::open() -> plat::File / release()
//
// The tools are fixed (powershell.exe, lsblk, lvcreate, lvremove, lvs); no command comes from
// the request.  A volume name is checked before it reaches one: a drive letter "C:", a device
// path /dev/..., or an absolute file path.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::j::{self, Value};

#[derive(Clone, Debug, Default)]
pub struct VolumeInfo {
    /// "C:" (Windows) or the device path (Linux)
    pub id: String,
    pub label: String,
    pub fs: String,
    pub size: u64,
    pub free: u64,
    /// "fixed", "removable" (Windows); "disk", "part", "lvm", "loop" (Linux)
    pub kind: String,
    pub mount: String,
}

/// run a fixed tool -> its standard output
fn run(program: &str, args: &[&str], timeout_s: u32) -> Result<String, String> {
    let mut c = plat::process::Command { program, args, cwd: None, env: &[] }.spawn().map_err(|e| format!("{} startet nicht (Fehler {})", program, e))?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let start = plat::time_us();
    loop {
        let mut b = [0u8; 8192];
        while let Ok(Some(n)) = c.read_stdout(&mut b) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&b[..n]);
        }
        while let Ok(Some(n)) = c.read_stderr(&mut b) {
            if n == 0 {
                break;
            }
            if err.len() < 8192 {
                err.extend_from_slice(&b[..n]);
            }
        }
        match c.try_wait() {
            Ok(Some(code)) => {
                // what came after the exit
                while let Ok(Some(n)) = c.read_stdout(&mut b) {
                    if n == 0 {
                        break;
                    }
                    out.extend_from_slice(&b[..n]);
                }
                if code == 0 {
                    return Ok(String::from_utf8_lossy(&out).into_owned());
                }
                let e = String::from_utf8_lossy(&err).into_owned();
                return Err(format!("{} endete mit {}: {}", program, code, e.trim()));
            }
            Ok(None) => {}
            Err(e) => return Err(format!("{}: Fehler {}", program, e)),
        }
        if plat::time_us() - start > timeout_s as u64 * 1_000_000 {
            c.terminate();
            return Err(format!("{} braucht zu lange", program));
        }
        plat::sleep_ms(20);
    }
}

fn powershell(script: &str) -> Result<String, String> {
    run("powershell.exe", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script], 300)
}

/// a volume name a tool may see
pub fn valid(id: &str) -> bool {
    let b = id.as_bytes();
    if cfg!(windows) {
        b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
    } else {
        id.starts_with('/') && !id.contains("..") && id.chars().all(|c| c.is_ascii_alphanumeric() || "/-_.+".contains(c)) && id.len() < 256
    }
}

fn num(v: &Value, k: &str) -> u64 {
    match v.get(k) {
        Some(x) => x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())).unwrap_or(0),
        None => 0,
    }
}

pub fn list() -> Result<Vec<VolumeInfo>, String> {
    if cfg!(windows) {
        let out = powershell("Get-Volume | Where-Object DriveLetter | Select-Object DriveLetter,FileSystemLabel,FileSystem,Size,SizeRemaining,DriveType | ConvertTo-Json -Compress")?;
        let v = j::parse(out.trim().as_bytes()).ok_or("unlesbare Antwort von Get-Volume")?;
        let items: Vec<Value> = match v {
            Value::Array(a) => a,
            Value::Object(_) => alloc::vec![v],
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for i in items {
            let letter = j::str_of(&i, "DriveLetter");
            if letter.is_empty() {
                continue;
            }
            out.push(VolumeInfo {
                id: format!("{}:", letter),
                label: String::from(j::str_of(&i, "FileSystemLabel")),
                fs: String::from(j::str_of(&i, "FileSystem")),
                size: num(&i, "Size"),
                free: num(&i, "SizeRemaining"),
                kind: j::str_of(&i, "DriveType").to_lowercase(),
                mount: format!("{}:\\", letter),
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        return Ok(out);
    }
    let out = run("lsblk", &["-J", "-b", "-o", "PATH,SIZE,TYPE,FSTYPE,MOUNTPOINT,LABEL,FSAVAIL"], 30)?;
    let v = j::parse(out.as_bytes()).ok_or("unlesbare Antwort von lsblk")?;
    let mut list = Vec::new();
    fn walk(v: &Value, out: &mut Vec<VolumeInfo>) {
        for d in v.as_array().unwrap_or(&[]) {
            let fs = j::str_of(d, "fstype");
            let ty = j::str_of(d, "type");
            if !fs.is_empty() && fs != "swap" && ty != "rom" {
                out.push(VolumeInfo {
                    id: String::from(j::str_of(d, "path")),
                    label: String::from(j::str_of(d, "label")),
                    fs: String::from(fs),
                    size: num(d, "size"),
                    free: num(d, "fsavail"),
                    kind: String::from(ty),
                    mount: String::from(j::str_of(d, "mountpoint")),
                });
            }
            if let Some(c) = d.get("children") {
                walk(c, out);
            }
        }
    }
    walk(v.get("blockdevices").unwrap_or(&Value::Null), &mut list);
    Ok(list)
}

#[derive(Clone, Debug)]
enum SnapKind {
    Vss(String),
    Lvm(String),
    None,
}

/// a consistent view of a volume (or the volume itself when there is none)
#[derive(Clone, Debug)]
pub struct Snap {
    /// what is read
    pub device: String,
    pub size: u64,
    /// "VSS", "LVM", "" (no snapshot)
    pub method: String,
    kind: SnapKind,
}

impl Snap {
    pub fn open(&self) -> Result<plat::File, String> {
        plat::File::open(&self.device, plat::FO_READ).ok_or_else(|| format!("{} ist nicht lesbar (Rechte?)", self.device))
    }

    /// remove the snapshot
    pub fn release(&self) -> Result<(), String> {
        match &self.kind {
            SnapKind::Vss(id) => powershell(&format!("Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq '{}' | Remove-CimInstance", id)).map(|_| ()),
            SnapKind::Lvm(lv) => run("lvremove", &["-f", lv], 120).map(|_| ()),
            SnapKind::None => Ok(()),
        }
    }
}

/// the size of a device or file
fn size_of(path: &str) -> Option<u64> {
    if let Some(m) = plat::fs::stat(path).filter(|m| m.is_file()) {
        return Some(m.size);
    }
    let out = run("lsblk", &["-b", "-d", "-n", "-o", "SIZE", path], 30).ok()?;
    out.trim().parse().ok()
}

/// "vg/lv" when the device is an LVM logical volume
fn lvm_name(dev: &str) -> Option<String> {
    let out = run("lvs", &["--noheadings", "-o", "vg_name,lv_name", dev], 30).ok()?;
    let mut it = out.split_whitespace();
    let (vg, lv) = (it.next()?, it.next()?);
    Some(format!("{}/{}", vg, lv))
}

/// a consistent view of the volume `id` (`tag` names the snapshot)
pub fn snapshot(id: &str, tag: &str) -> Result<Snap, String> {
    if !valid(id) {
        return Err(format!("„{}“ ist kein Volume", id));
    }
    if cfg!(windows) {
        let script = format!(
            "$ErrorActionPreference='Stop'; \
             if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {{ [Console]::Error.WriteLine('BC-NOADMIN'); exit 5 }}; \
             $r = Invoke-CimMethod -ClassName Win32_ShadowCopy -MethodName Create -Arguments @{{Volume='{}\\'; Context='ClientAccessible'}}; \
             if ($r.ReturnValue -ne 0) {{ throw \"VSS: Fehler $($r.ReturnValue)\" }}; \
             $s = Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq $r.ShadowID; \
             $p = Get-Partition -DriveLetter '{}'; \
             \"$($r.ShadowID)|$($s.DeviceObject)|$($p.Size)\"",
            id, &id[..1]
        );
        let out = powershell(&script).map_err(|e| {
            if e.contains("BC-NOADMIN") {
                String::from("VSS braucht Administratorrechte (der Dienst läuft als LocalSystem; bc-server von Hand: als Administrator starten)")
            } else {
                e
            }
        })?;
        let parts: Vec<&str> = out.trim().split('|').collect();
        if parts.len() != 3 || !parts[1].starts_with("\\\\?\\GLOBALROOT\\Device\\") {
            return Err(format!("unerwartete Antwort von VSS: {}", out.trim()));
        }
        let size = parts[2].trim().parse().map_err(|_| String::from("Größe des Volumes unbekannt"))?;
        return Ok(Snap { device: String::from(parts[1]), size, method: String::from("VSS"), kind: SnapKind::Vss(String::from(parts[0])) });
    }
    // an image file: read as it is
    if plat::fs::stat(id).is_some_and(|m| m.is_file()) {
        let size = size_of(id).ok_or("Größe unbekannt")?;
        return Ok(Snap { device: String::from(id), size, method: String::new(), kind: SnapKind::None });
    }
    if let Some(lv) = lvm_name(id) {
        let name = format!("bcsnap-{}", tag);
        let vg = lv.split('/').next().unwrap_or("");
        run("lvcreate", &["-s", "-n", &name, "-l", "10%ORIGIN", &lv], 120).map_err(|e| format!("LVM-Snapshot: {}", e))?;
        let dev = format!("/dev/{}/{}", vg, name);
        let size = size_of(&dev).ok_or("Größe des Snapshots unbekannt")?;
        return Ok(Snap { device: dev, size, method: String::from("LVM"), kind: SnapKind::Lvm(format!("{}/{}", vg, name)) });
    }
    let size = size_of(id).ok_or_else(|| format!("{}: Größe unbekannt", id))?;
    Ok(Snap { device: String::from(id), size, method: String::new(), kind: SnapKind::None })
}

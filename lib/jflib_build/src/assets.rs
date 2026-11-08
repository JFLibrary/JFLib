// assets/ as one bundle (JFA1) for app:// (lib/plat/vfs.rs): the format of tools/assetpack.py,
// byte for byte, with the LZ4 of lib/compress.
//
//   "JFA1", u32 count, u32 names_len
//   count entries of 20 bytes, sorted by name (bytewise):
//       u32 name_off, u16 name_len, u8 codec (0 stored, 1 LZ4 block), u8 0,
//       u32 data_off (from the start of the bundle), u32 stored_len, u32 size
//   the names (UTF-8, '/' separated, relative), then the data
//
// Names starting with '.' (files and folders) are left out.  A file is stored LZ4-compressed when
// that saves at least a tenth; every block is decoded again before it is written.

use std::fs;
use std::path::Path;

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let list = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in list {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else { return Err(format!("{}: the name is not UTF-8", path.display())) };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel: Vec<&str> = rel.components().filter_map(|c| c.as_os_str().to_str()).collect();
            let data = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.push((rel.join("/"), data));
        }
    }
    Ok(())
}

fn u32_of(n: usize, what: &str) -> Result<u32, String> {
    u32::try_from(n).map_err(|_| format!("{what}: larger than 4 GiB"))
}

/// the bundle of a folder
pub(crate) fn pack(dir: &Path) -> Result<Vec<u8>, String> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut blobs = Vec::new();
    for (name, data) in &files {
        u32_of(data.len(), name)?;
        let packed = if data.is_empty() { Vec::new() } else { compress::lz4::compress(data) };
        if !data.is_empty() && packed.len() * 10 <= data.len() * 9 {
            if compress::lz4::decompress(&packed, data.len()).as_deref() != Some(data.as_slice()) {
                return Err(format!("{name}: LZ4 round trip failed"));
            }
            blobs.push((1u8, packed));
        } else {
            blobs.push((0u8, data.clone()));
        }
    }
    let pool: usize = files.iter().map(|f| f.0.len()).sum();
    let mut data_off = 12 + 20 * files.len() + pool;
    let mut out = b"JFA1".to_vec();
    out.extend_from_slice(&u32_of(files.len(), "assets")?.to_le_bytes());
    out.extend_from_slice(&u32_of(pool, "the names")?.to_le_bytes());
    let mut name_off = 0;
    for ((name, data), (codec, blob)) in files.iter().zip(&blobs) {
        let len = u16::try_from(name.len()).map_err(|_| format!("{name}: the name is too long"))?;
        out.extend_from_slice(&u32_of(name_off, "the names")?.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&[*codec, 0]);
        out.extend_from_slice(&u32_of(data_off, "the bundle")?.to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        name_off += name.len();
        data_off += blob.len();
    }
    for (name, _) in &files {
        out.extend_from_slice(name.as_bytes());
    }
    for (_, blob) in &blobs {
        out.extend_from_slice(blob);
    }
    u32_of(out.len(), "the bundle")?;
    Ok(out)
}

// res/app.ico -> a compiled resource file (.res) with the icon, which the Windows linkers take as
// an input: what llvm-rc makes of `1 ICON "app.ico"` in build_rs.py, without a resource compiler.

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_EN_US: u16 = 0x0409; // llvm-rc's default LANGUAGE

/// one resource: a 32-byte header with numeric type and name, the data, padded to 4 bytes
fn resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, lang: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    for v in [0xFFFF, kind, 0xFFFF, name] {
        out.extend_from_slice(&u16::to_le_bytes(v));
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&lang.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

/// the .res of an .ico file
pub(crate) fn res(ico: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "res/app.ico is no icon file".to_string();
    if u16_at(ico, 0) != Some(0) || u16_at(ico, 2) != Some(1) {
        return Err(bad());
    }
    let count = u16_at(ico, 4).ok_or_else(bad)?;
    let mut out = Vec::new();
    resource(&mut out, 0, 0, 0, 0, &[]); // the empty entry every .res starts with
    let mut group = Vec::new();
    for v in [0u16, 1, count] {
        group.extend_from_slice(&v.to_le_bytes());
    }
    for i in 0..count as usize {
        let e = 6 + 16 * i;
        let dir = ico.get(e..e + 8).ok_or_else(bad)?;
        let size = u32_at(ico, e + 8).ok_or_else(bad)? as usize;
        let at = u32_at(ico, e + 12).ok_or_else(bad)? as usize;
        let image = ico.get(at..at.checked_add(size).ok_or_else(bad)?).ok_or_else(bad)?;
        resource(&mut out, RT_ICON, i as u16 + 1, 0x1010, LANG_EN_US, image);
        group.extend_from_slice(dir); // width, height, colours, reserved, planes, bit count
        group.extend_from_slice(&(size as u32).to_le_bytes());
        group.extend_from_slice(&(i as u16 + 1).to_le_bytes());
    }
    resource(&mut out, RT_GROUP_ICON, 1, 0x1030, LANG_EN_US, &group);
    Ok(out)
}

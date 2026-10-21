//! Bounded import of Brush Factory / Dynamic Draw settings. Unsupported active
//! controls are reported per preset; applying never silently drops them.
use agent_core::json::{self, Value};
use alloc::{format, string::String, vec::Vec};

#[derive(Clone, Debug)]
pub(crate) struct Preset {
    pub name: String,
    pub image: String,
    pub size: i32,
    pub density: i32,
    pub color: u32,
    pub opacity: u8,
    pub flow: u8,
    pub rotation: i32,
    pub follow: bool,
    pub blend: u32,
    pub unsupported: Vec<String>,
}
fn integer(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str()?.parse().ok())
}
fn boolean(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| match v.as_str()? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}
fn number(v: &Value, key: &str, default: i64, min: i64, max: i64) -> Result<i64, String> {
    let n = v
        .get(key)
        .map_or(Some(default), integer)
        .ok_or_else(|| format!("Ungültiger Zahlenwert: {key}"))?;
    if n < min || n > max {
        return Err(format!("Wert außerhalb des Bereichs: {key}"));
    }
    Ok(n)
}
fn flag(v: &Value, key: &str, default: bool) -> Result<bool, String> {
    v.get(key)
        .map_or(Some(default), boolean)
        .ok_or_else(|| format!("Ungültiger Wahrheitswert: {key}"))
}
fn neutral(v: &Value) -> bool {
    boolean(v) == Some(false)
        || integer(v) == Some(0)
        || v.as_object().is_some_and(|x| x.is_empty())
        || v.as_array().is_some_and(|x| x.is_empty())
        || v.as_str()
            .is_some_and(|s| s.is_empty() || matches!(s, "None" | "Normal"))
}
fn preset(name: &str, value: &Value) -> Result<Preset, String> {
    let fields = value
        .as_object()
        .ok_or_else(|| String::from("Das Pinsel-Preset ist kein Objekt."))?;
    if name.is_empty() || name.len() > 1024 {
        return Err(String::from("Ungültiger Preset-Name."));
    }
    let size = number(value, "BrushSize", 2, 1, 2000)? as i32;
    let mut density = number(value, "BrushDensity", 10, 0, 100)? as i32;
    if flag(value, "AutomaticBrushDensity", true)? {
        density = match size {
            1..=2 => size,
            3..=5 => 3,
            6..=7 => 4,
            8..=20 => 5,
            21..=25 => 6,
            26..=45 => 7,
            46..=80 => 8,
            81..=250 => 9,
            _ => 10,
        };
    }
    let image = match value.get("BrushImagePath") {
        None => String::from("Circle Light"),
        Some(v) => String::from(
            v.as_str()
                .ok_or_else(|| String::from("Ungültiger Bildpfad."))?,
        ),
    };
    if image.len() > 4096 || image.contains('\0') {
        return Err(String::from("Ungültiger Bildpfad."));
    }
    let blend = match value.get("BlendMode").and_then(Value::as_str) {
        Some(s) if s.parse::<i64>().is_err() => [
            "Normal",
            "Multiply",
            "Additive",
            "ColorBurn",
            "ColorDodge",
            "Reflect",
            "Glow",
            "Overlay",
            "Difference",
            "Negation",
            "Lighten",
            "Darken",
            "Screen",
            "Xor",
            "Overwrite",
        ]
        .iter()
        .position(|x| *x == s)
        .ok_or_else(|| String::from("Unbekannter Mischmodus."))?
            as u32,
        _ => number(value, "BlendMode", 0, 0, 14)? as u32,
    };
    let mut result = Preset {
        name: String::from(name),
        image,
        size,
        density: density.max(1),
        blend,
        color: number(
            value,
            "BrushColor",
            -16777216,
            i32::MIN as i64,
            u32::MAX as i64,
        )? as u32,
        opacity: number(value, "BrushOpacity", 255, 0, 255)? as u8,
        flow: number(value, "BrushFlow", 255, 0, 255)? as u8,
        rotation: number(value, "BrushRotation", 0, -180, 360)? as i32,
        follow: flag(value, "DoRotateWithMouse", false)?,
        unsupported: Vec::new(),
    };
    for (key, v) in fields {
        let supported = match key.as_str() {
            "AutomaticBrushDensity"
            | "BrushSize"
            | "BrushDensity"
            | "BrushImagePath"
            | "BlendMode"
            | "BrushColor"
            | "BrushOpacity"
            | "BrushFlow"
            | "BrushRotation"
            | "DoRotateWithMouse" => true,
            "DoColorizeBrush" => boolean(v) == Some(true),
            "ColorInfluenceHue" | "ColorInfluenceSat" | "ColorInfluenceVal" => {
                number(value, "ColorInfluence", 0, 0, 100)? == 0 && boolean(v).is_some()
            }
            "CmbxChosenEffect"
            | "ColorInfluence"
            | "DoDitherDraw"
            | "DoLockAlpha"
            | "DoLockR"
            | "DoLockG"
            | "DoLockB"
            | "DoLockHue"
            | "DoLockSat"
            | "DoLockVal"
            | "FlowChange"
            | "MinDrawDistance"
            | "RandFlowLoss"
            | "RandMaxSize"
            | "RandMinSize"
            | "RandRotLeft"
            | "RandRotRight"
            | "RandHorzShift"
            | "RandVertShift"
            | "RandMaxR"
            | "RandMaxG"
            | "RandMaxB"
            | "RandMinR"
            | "RandMinG"
            | "RandMinB"
            | "RandMaxH"
            | "RandMaxS"
            | "RandMaxV"
            | "RandMinH"
            | "RandMinS"
            | "RandMinV"
            | "SeamlessDrawing"
            | "SizeChange"
            | "RotChange"
            | "Smoothing"
            | "Symmetry"
            | "TabPressureConstraints" => neutral(v),
            _ => false,
        };
        if !supported {
            result.unsupported.push(key.clone());
        }
    }
    Ok(result)
}

fn xml_value(node: &base::xml::Node) -> Result<Value, String> {
    if node.children.is_empty() {
        return Ok(Value::String(node.text.clone()));
    }
    let mut fields = Vec::new();
    for child in &node.children {
        let key = child.name.local.clone();
        if fields.iter().any(|(k, _)| k == &key) {
            return Err(format!("Doppeltes XML-Feld: {key}"));
        }
        fields.push((key, xml_value(child)?));
    }
    Ok(Value::Object(fields))
}
pub(crate) fn parse(data: &[u8]) -> Result<Vec<Preset>, String> {
    if data.len() > 4_194_304 {
        return Err(String::from("Preset-Datei ist größer als 4 MiB."));
    }
    let source = core::str::from_utf8(data)
        .map_err(|_| String::from("Presets müssen UTF-8 sein."))?
        .trim_start_matches('\u{feff}')
        .trim();
    let rows = if source.starts_with('<') {
        let root = base::xml::parse(source)
            .map_err(|_| String::from("Ungültige oder nicht unterstützte Preset-XML-Datei."))?;
        if !matches!(
            root.name.local.as_str(),
            "DynamicDrawSettings" | "BrushFactorySettings"
        ) || !root.name.ns.is_empty()
        {
            return Err(String::from("Kein Brush-Factory-/Dynamic-Draw-Dokument."));
        }
        let containers: Vec<_> = root
            .children
            .iter()
            .filter(|n| n.name.local == "CustomBrushes")
            .collect();
        if containers.len() != 1 {
            return Err(String::from(
                "CustomBrushes fehlt oder ist mehrfach vorhanden.",
            ));
        }
        let mut rows = Vec::new();
        for entry in &containers[0].children {
            let keys: Vec<_> = entry
                .children
                .iter()
                .filter(|n| n.name.local == "Key")
                .collect();
            let values: Vec<_> = entry
                .children
                .iter()
                .filter(|n| n.name.local == "Value")
                .collect();
            if keys.len() != 1 || values.len() != 1 {
                return Err(String::from("Ungültiger XML-Preset-Eintrag."));
            }
            rows.push((keys[0].text.clone(), xml_value(values[0])?));
        }
        rows
    } else {
        let root = json::parse_with_limits(
            source.as_bytes(),
            json::Limits {
                bytes: 4_194_304,
                ..Default::default()
            },
        )
        .map_err(|_| String::from("Ungültige Preset-JSON-Datei."))?;
        root.get("CustomBrushes")
            .and_then(Value::as_object)
            .ok_or_else(|| String::from("CustomBrushes fehlt in der Preset-Datei."))?
            .to_vec()
    };
    if rows.is_empty() || rows.len() > 1024 {
        return Err(String::from(
            "Die Datei muss 1 bis 1024 Pinsel-Presets enthalten.",
        ));
    }
    let mut presets: Vec<Preset> = Vec::new();
    for (name, value) in rows {
        if presets.iter().any(|p| p.name == name) {
            return Err(format!("Doppelter Preset-Name: {name}"));
        }
        presets.push(preset(&name, &value).map_err(|e| format!("{name}: {e}"))?);
    }
    Ok(presets)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_argb_and_controls() {
        let p = parse(br#"{"Version":"4.0","CustomBrushes":{"Ink":{"BrushColor":-65536,"BrushSize":21,"BrushFlow":128,"BrushOpacity":77,"BlendMode":1,"BrushRotation":-90,"DoRotateWithMouse":true}}}"#).unwrap().remove(0);
        assert_eq!(
            (
                p.color, p.size, p.flow, p.opacity, p.blend, p.rotation, p.follow, p.density
            ),
            (0xffff0000, 21, 128, 77, 1, -90, true, 6)
        );
        assert!(p.unsupported.is_empty());
    }
    #[test]
    fn namespaced_datacontract_xml() {
        let p = parse(br#"<?xml version="1.0" encoding="utf-8"?><DynamicDrawSettings xmlns:i="http://www.w3.org/2001/XMLSchema-instance"><CustomBrushes xmlns:a="http://schemas.microsoft.com/2003/10/Serialization/Arrays"><a:KeyValueOfstringBrushSettings><a:Key>Ink &amp; Paper</a:Key><a:Value><BrushImagePath>tip.png</BrushImagePath><BrushSize>13</BrushSize><BlendMode>Multiply</BlendMode><TabPressureConstraints/></a:Value></a:KeyValueOfstringBrushSettings></CustomBrushes></DynamicDrawSettings>"#).unwrap().remove(0);
        assert_eq!(
            (p.name.as_str(), p.size, p.blend, p.image.as_str()),
            ("Ink & Paper", 13, 1, "tip.png")
        );
        assert!(p.unsupported.is_empty());
    }
    #[test]
    fn active_and_unknown_fields_are_reported() {
        let p = parse(br#"{"CustomBrushes":{"A":{"RandMaxSize":30,"FutureField":0,"DoLockAlpha":false,"TabPressureConstraints":{},"DoColorizeBrush":false}}}"#).unwrap().remove(0);
        assert_eq!(
            p.unsupported,
            ["RandMaxSize", "FutureField", "DoColorizeBrush"]
        );
    }
    #[test]
    fn malformed_duplicate_and_out_of_range_are_rejected() {
        for s in [
            r#"{"CustomBrushes":{"A":{"BrushFlow":256}}}"#,
            r#"{"CustomBrushes":{"A":{"BrushSize":1.5}}}"#,
            r#"{"CustomBrushes":{"A":{},"A":{}}}"#,
            r#"{"CustomBrushes":{"A":{"BrushSize":2,"BrushSize":3}}}"#,
            r#"{"CustomBrushes":{"A":{"DoRotateWithMouse":"maybe"}}}"#,
            r#"{"CustomBrushes":{"A":{"BrushImagePath":"\u0000"}}}"#,
            "<!DOCTYPE x><DynamicDrawSettings/>",
        ] {
            assert!(parse(s.as_bytes()).is_err(), "{s}");
        }
    }
    #[test]
    fn manual_density_and_utf8() {
        let p = parse(
            "{\"CustomBrushes\":{\"Öl\":{\"AutomaticBrushDensity\":false,\"BrushDensity\":7}}}"
                .as_bytes(),
        )
        .unwrap()
        .remove(0);
        assert_eq!(p.name, "Öl");
        assert_eq!(p.density, 7);
    }
}

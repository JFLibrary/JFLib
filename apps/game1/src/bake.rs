// bake -- modeltool's .glb into game 1's .g1m (game1_core::map): `game1 --bake IN.glb OUT.g1m`, run by
// apps/game1/tools/content.py for every script in apps/game1/content.  The glTF is read with lib/gltf; the client
// and the server then read only the .g1m (no JFScript, no glTF at start-up).
//
// A node's name says what its part is (docs/GAME1.md, "Karte"):
//   "@NAME"              a marker: the box of its vertices under NAME ("spawn 90", "weg", "muendung"); not drawn,
//                        not collided
//   "NAME #deko"         drawn, not collided;  "NAME #unsichtbar" collided, not drawn;  "NAME #rand" with the rim
//                        light (a material of its own: "MATERIAL+rand")
// Way points ("@weg") are linked where a bot can walk straight from one to the other: a sphere of 0.3 m at 1 m
// above the feet passes, and the ground along the line (a ray down every 0.5 m from 1.9 m above the line of the
// feet) is there and never rises or falls by more than 0.45 m between two samples (a step, not a wall or a ledge).
// Everything is computed from the glTF's f32 positions with lib/physics: the same file bakes to the same bytes.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use game1_core::map::{bounds_of, Marker, MaterialData, ModelData, PartData, MAT_RIM, PART_COLLIDE, PART_DRAW};
use physics::{Filter, Iso, Shape, TriMesh, Vec3};

/// how far apart two way points may be linked (m)
const LINK_MAX: f64 = 26.0;
/// the largest rise or fall between two samples of the ground along a link (m)
const LINK_STEP: f64 = 0.45;

/// a .glb's bytes -> the model or map
pub fn bake(glb: &[u8]) -> Result<ModelData, String> {
    let doc = gltf::load(glb, &mut |_| None, &gltf::Limits::DEFAULT).map_err(|e| format!("glTF: {:?}", e))?;
    let mut out = ModelData { name: doc.scenes.first().map(|s| s.name.clone()).unwrap_or_default(), ..ModelData::default() };
    let mut mats: Vec<(usize, bool)> = Vec::new();
    for node in &doc.nodes {
        let Some(mi) = node.mesh else { continue };
        let Some(mesh) = doc.meshes.get(mi) else { continue };
        let name = node.name.trim();
        if let Some(marker) = name.strip_prefix('@') {
            let pts: Vec<[f32; 3]> = mesh.primitives.iter().flat_map(|p| p.positions.iter().copied()).collect();
            let (min, max) = bounds_of(&pts);
            out.markers.push(Marker { name: String::from(marker.trim()), min, max });
            continue;
        }
        let mut words: Vec<&str> = Vec::new();
        let (mut flags, mut rim) = (PART_DRAW | PART_COLLIDE, false);
        for w in name.split(' ').filter(|w| !w.is_empty()) {
            match w {
                "#deko" => flags &= !PART_COLLIDE,
                "#unsichtbar" => flags &= !PART_DRAW,
                "#rand" => rim = true,
                t if t.starts_with('#') => return Err(format!("{}: unknown tag {}", name, t)),
                _ => words.push(w),
            }
        }
        let pname = words.join(" ");
        for (k, prim) in mesh.primitives.iter().enumerate() {
            let gm = prim.material.unwrap_or(usize::MAX);
            let mat = match mats.iter().position(|m| *m == (gm, rim)) {
                Some(i) => i,
                None => {
                    let (mname, c) = match doc.materials.get(gm) {
                        Some(m) => (m.name.clone(), [m.base_color_factor[0], m.base_color_factor[1], m.base_color_factor[2]]),
                        None => (String::from("grau"), [0.5, 0.5, 0.5]),
                    };
                    let mname = if rim { format!("{}+rand", mname) } else { mname };
                    out.materials.push(MaterialData { name: mname, color: c, flags: if rim { MAT_RIM } else { 0 } });
                    mats.push((gm, rim));
                    mats.len() - 1
                }
            };
            let n = if mesh.primitives.len() > 1 { format!("{} {}", pname, k) } else { pname.clone() };
            out.parts.push(PartData { name: n, material: mat as u16, flags, positions: prim.positions.clone(), normals: prim.normals.clone(), indices: prim.indices.clone() });
        }
    }
    out.links = links(&out)?;
    Ok(out)
}

/// the links between the way points (see the module header)
fn links(m: &ModelData) -> Result<Vec<(u16, u16)>, String> {
    let ways: Vec<usize> = (0..m.markers.len()).filter(|&k| m.markers[k].kind() == "weg").collect();
    if ways.is_empty() {
        return Ok(Vec::new());
    }
    let (p, i) = m.collision_soup();
    let mesh = TriMesh::new(&p, &i).map_err(|e| format!("the collision mesh: {:?}", e))?;
    let mut world = physics::World::new();
    world.add(Shape::mesh(mesh), Iso::IDENTITY);
    let ball = Shape::sphere(0.3);
    let up = Vec3::new(0.0, 1.0, 0.0);
    let mut out = Vec::new();
    for (x, &a) in ways.iter().enumerate() {
        for &b in &ways[x + 1..] {
            let (fa, fb) = (m.markers[a].foot(), m.markers[b].foot());
            let flat = Vec3::new(fb.x - fa.x, 0.0, fb.z - fa.z).length();
            if flat > LINK_MAX || flat < 0.5 {
                continue;
            }
            if world.cast(&ball, &Iso::at(fa + up), fb - fa, &Filter::ALL).is_some() {
                continue;
            }
            if walkable(&world, fa, fb, flat) {
                out.push((a as u16, b as u16));
            }
        }
    }
    Ok(out)
}

/// the ground along the line from a to b: there, and no rise or fall above LINK_STEP between two samples
fn walkable(world: &physics::World, a: Vec3, b: Vec3, flat: f64) -> bool {
    let n = (flat / 0.5) as usize + 1;
    let down = Vec3::new(0.0, -1.0, 0.0);
    let mut last: Option<f64> = None;
    for k in 0..=n {
        let t = k as f64 / n as f64;
        let p = a + (b - a) * t + Vec3::new(0.0, 1.9, 0.0);
        let Some((_, hit)) = world.ray_cast(p, down, 4.0, &Filter::ALL) else { return false };
        let y = p.y - hit.t;
        if last.is_some_and(|l| (y - l).abs() > LINK_STEP) {
            return false;
        }
        last = Some(y);
    }
    true
}

/// `game1 --bake IN OUT`: read, bake, write -> exit code
pub fn run(input: &str, output: &str) -> i32 {
    let Some(b) = plat::read_file(input) else {
        crate::out(&format!("bake: {} cannot be read\n", input));
        return 1;
    };
    match bake(&b) {
        Ok(m) => {
            let bytes = m.encode();
            // (read back: what was written is what the game reads)
            if ModelData::decode(&bytes).as_ref() != Ok(&m) {
                crate::out("bake: the file does not read back the same\n");
                return 1;
            }
            if !plat::write_file(output, &bytes) {
                crate::out(&format!("bake: {} cannot be written\n", output));
                return 1;
            }
            let tris: usize = m.parts.iter().map(|p| p.indices.len() / 3).sum();
            crate::out(&format!(
                "bake: {} -> {} ({} bytes): {} materials, {} parts, {} triangles, {} markers, {} links\n",
                input,
                output,
                bytes.len(),
                m.materials.len(),
                m.parts.len(),
                tris,
                m.markers.len(),
                m.links.len()
            ));
            0
        }
        Err(e) => {
            crate::out(&format!("bake: {}: {}\n", input, e));
            1
        }
    }
}

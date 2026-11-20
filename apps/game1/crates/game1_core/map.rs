// map -- game 1's baked models and maps (".g1m"): what modeltool built from a JFScript description, in a small
// binary form that the client and the server read without JFScript and without glTF (docs/GAME1.md, "Karte")
//
//   ModelData        materials, parts (triangles with normals, a material, flags), markers (named boxes: spawn
//                    points, way points, a muzzle) and the links between way points; encode / decode
//   Map              a map ready to play: its data, the static physics::World of its collidable parts (one
//                    triangle mesh, the same bytes on every target), the spawn points, the way points and their
//                    neighbours; collision_sum() is a checksum of exactly what collides
//   WERFT            the map of part 1, baked from apps/game1/content/werft.jfs
//
// The file (little-endian): "G1M" 0x01, u32 version, a name; u16 materials (name, linear rgb as f32, flags); u16
// parts (name, u16 material, u8 flags, u32 vertices, u32 indices, the positions and normals as f32, the indices as
// u32); u16 markers (name, min and max of its box as f32); u16 links (two marker indices).  A string is a u16 length
// and UTF-8.  The positions are modeltool's f32 as glTF carried them: the client draws and the server collides
// against the very same numbers.
//
// The names of modeltool's parts say what a part is (the bake in apps/game1/src/bake.rs reads them): "@NAME" a
// marker (neither drawn nor collided: "@spawn 90" a spawn point looking 90 degrees left of -Z, "@weg" a way point of
// the bots, "@muendung" a weapon's muzzle), tags after the name: "#deko" drawn but not collided, "#unsichtbar"
// collided but not drawn, "#rand" with the rim light of figures and props (walls and floors have none).

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use physics::{Filter, Iso, Shape, TriMesh, Vec3};

/// a part is drawn
pub const PART_DRAW: u8 = 1;
/// a part collides
pub const PART_COLLIDE: u8 = 2;
/// a material has the rim light
pub const MAT_RIM: u8 = 1;

const MAGIC: [u8; 4] = *b"G1M\x01";
const VERSION: u32 = 1;

/// a material: linear rgb, flags
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialData {
    pub name: String,
    pub color: [f32; 3],
    pub flags: u8,
}

/// a part: triangles of one material
#[derive(Clone, Debug, PartialEq)]
pub struct PartData {
    pub name: String,
    pub material: u16,
    pub flags: u8,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl PartData {
    /// the box of its positions (min, max); zeros without any
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        bounds_of(&self.positions)
    }
}

/// the box of points (min, max); zeros without any
pub fn bounds_of(p: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let Some(first) = p.first() else { return ([0.0; 3], [0.0; 3]) };
    let (mut lo, mut hi) = (*first, *first);
    for v in p {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    (lo, hi)
}

/// a marker: a named box
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub name: String,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Marker {
    /// the middle of its box's bottom (where a spawn point or a way point stands)
    pub fn foot(&self) -> Vec3 {
        Vec3::new((self.min[0] as f64 + self.max[0] as f64) * 0.5, self.min[1] as f64, (self.min[2] as f64 + self.max[2] as f64) * 0.5)
    }

    /// the middle of its box
    pub fn centre(&self) -> Vec3 {
        Vec3::new((self.min[0] as f64 + self.max[0] as f64) * 0.5, (self.min[1] as f64 + self.max[1] as f64) * 0.5, (self.min[2] as f64 + self.max[2] as f64) * 0.5)
    }

    /// the first word of its name ("spawn", "weg", ...)
    pub fn kind(&self) -> &str {
        self.name.split(' ').next().unwrap_or("")
    }

    /// the number after the first word ("spawn 90" -> 90), else 0
    pub fn arg(&self) -> f64 {
        self.name.split(' ').nth(1).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0)
    }
}

/// a baked model or map (see the module header)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelData {
    pub name: String,
    pub materials: Vec<MaterialData>,
    pub parts: Vec<PartData>,
    pub markers: Vec<Marker>,
    pub links: Vec<(u16, u16)>,
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        let b = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
        self.u16(b.len() as u16);
        self.0.extend_from_slice(b);
    }
    fn v3(&mut self, v: [f32; 3]) {
        for x in v {
            self.f32(x);
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| String::from("the file ends too early"))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| String::from("a name is not UTF-8"))
    }
    fn v3(&mut self) -> Result<[f32; 3], String> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
}

impl ModelData {
    /// the file's bytes
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        w.0.extend_from_slice(&MAGIC);
        w.u32(VERSION);
        w.str(&self.name);
        w.u16(self.materials.len() as u16);
        for m in &self.materials {
            w.str(&m.name);
            w.v3(m.color);
            w.u8(m.flags);
        }
        w.u16(self.parts.len() as u16);
        for p in &self.parts {
            w.str(&p.name);
            w.u16(p.material);
            w.u8(p.flags);
            w.u32(p.positions.len() as u32);
            w.u32(p.indices.len() as u32);
            for v in &p.positions {
                w.v3(*v);
            }
            for v in &p.normals {
                w.v3(*v);
            }
            for i in &p.indices {
                w.u32(*i);
            }
        }
        w.u16(self.markers.len() as u16);
        for m in &self.markers {
            w.str(&m.name);
            w.v3(m.min);
            w.v3(m.max);
        }
        w.u16(self.links.len() as u16);
        for (a, b) in &self.links {
            w.u16(*a);
            w.u16(*b);
        }
        w.0
    }

    /// a file read back (Err: what is wrong with it)
    pub fn decode(b: &[u8]) -> Result<ModelData, String> {
        let mut r = R { b, at: 0 };
        if r.take(4)? != MAGIC {
            return Err(String::from("not a .g1m file"));
        }
        let v = r.u32()?;
        if v != VERSION {
            return Err(alloc::format!("version {} (this program reads {})", v, VERSION));
        }
        let mut m = ModelData { name: r.str()?, ..ModelData::default() };
        for _ in 0..r.u16()? {
            m.materials.push(MaterialData { name: r.str()?, color: r.v3()?, flags: r.u8()? });
        }
        for _ in 0..r.u16()? {
            let name = r.str()?;
            let material = r.u16()?;
            let flags = r.u8()?;
            let nv = r.u32()? as usize;
            let ni = r.u32()? as usize;
            // (sizes against what is left: a broken count does not allocate gigabytes)
            if nv.saturating_mul(24).saturating_add(ni.saturating_mul(4)) > b.len() - r.at {
                return Err(alloc::format!("part {}: more data than the file holds", name));
            }
            let mut p = PartData { name, material, flags, positions: Vec::with_capacity(nv), normals: Vec::with_capacity(nv), indices: Vec::with_capacity(ni) };
            for _ in 0..nv {
                p.positions.push(r.v3()?);
            }
            for _ in 0..nv {
                p.normals.push(r.v3()?);
            }
            for _ in 0..ni {
                let i = r.u32()?;
                if i as usize >= nv {
                    return Err(alloc::format!("part {}: index {} beyond its {} vertices", p.name, i, nv));
                }
                p.indices.push(i);
            }
            if p.material as usize >= m.materials.len() || ni % 3 != 0 {
                return Err(alloc::format!("part {}: no such material or not triangles", p.name));
            }
            m.parts.push(p);
        }
        for _ in 0..r.u16()? {
            m.markers.push(Marker { name: r.str()?, min: r.v3()?, max: r.v3()? });
        }
        for _ in 0..r.u16()? {
            let (a, c) = (r.u16()?, r.u16()?);
            if a as usize >= m.markers.len() || c as usize >= m.markers.len() {
                return Err(String::from("a link names no marker"));
            }
            m.links.push((a, c));
        }
        if r.at != b.len() {
            return Err(String::from("bytes after the end"));
        }
        Ok(m)
    }

    /// the part of this name
    pub fn part(&self, name: &str) -> Option<&PartData> {
        self.parts.iter().find(|p| p.name == name)
    }

    /// the marker of this name
    pub fn marker(&self, name: &str) -> Option<&Marker> {
        self.markers.iter().find(|m| m.name == name)
    }

    /// the collidable parts as one triangle soup (positions, indices), in the order of the parts
    pub fn collision_soup(&self) -> (Vec<[f32; 3]>, Vec<u32>) {
        let (mut p, mut i) = (Vec::new(), Vec::new());
        for part in self.parts.iter().filter(|p| p.flags & PART_COLLIDE != 0) {
            let base = p.len() as u32;
            p.extend_from_slice(&part.positions);
            i.extend(part.indices.iter().map(|x| x + base));
        }
        (p, i)
    }
}

/// FNV-1a over bytes (the checksums of game 1)
pub fn fnv(h: u64, b: &[u8]) -> u64 {
    let mut h = h;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// FNV-1a's start
pub const FNV0: u64 = 0xcbf2_9ce4_8422_2325;

/// a spawn point: where the feet stand, the yaw it looks along
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spawn {
    pub feet: Vec3,
    pub yaw: f64,
}

/// a map ready to play (see the module header)
pub struct Map {
    pub data: ModelData,
    /// the static world: one body, the triangle mesh of every collidable part
    pub world: physics::World,
    pub spawns: Vec<Spawn>,
    /// the bots' way points (feet) and each one's neighbours (indices into `points`)
    pub points: Vec<Vec3>,
    pub neighbours: Vec<Vec<usize>>,
    sum: u64,
    triangles: usize,
}

impl Map {
    /// a map from its baked data (Err: no collidable triangles, a broken mesh, no spawn point)
    pub fn new(data: ModelData) -> Result<Map, String> {
        let (p, i) = data.collision_soup();
        let mut sum = FNV0;
        for v in &p {
            for x in v {
                sum = fnv(sum, &x.to_bits().to_le_bytes());
            }
        }
        for x in &i {
            sum = fnv(sum, &x.to_le_bytes());
        }
        let mesh = TriMesh::new(&p, &i).map_err(|e| alloc::format!("the collision mesh: {:?}", e))?;
        let mut world = physics::World::new();
        world.add(Shape::mesh(mesh), Iso::IDENTITY);
        let deg = core::f64::consts::PI / 180.0;
        let spawns: Vec<Spawn> = data.markers.iter().filter(|m| m.kind() == "spawn").map(|m| Spawn { feet: m.foot(), yaw: m.arg() * deg }).collect();
        if spawns.is_empty() {
            return Err(String::from("the map has no spawn point (@spawn)"));
        }
        let ways: Vec<usize> = (0..data.markers.len()).filter(|&k| data.markers[k].kind() == "weg").collect();
        let points: Vec<Vec3> = ways.iter().map(|&k| data.markers[k].foot()).collect();
        let mut neighbours = alloc::vec![Vec::new(); points.len()];
        for (a, b) in &data.links {
            if let (Some(x), Some(y)) = (ways.iter().position(|&k| k == *a as usize), ways.iter().position(|&k| k == *b as usize)) {
                neighbours[x].push(y);
                neighbours[y].push(x);
            }
        }
        for n in neighbours.iter_mut() {
            n.sort_unstable();
            n.dedup();
        }
        Ok(Map { triangles: i.len() / 3, data, world, spawns, points, neighbours, sum })
    }

    /// a map from a .g1m file
    pub fn parse(b: &[u8]) -> Result<Map, String> {
        Map::new(ModelData::decode(b)?)
    }

    /// FNV-1a over the bits of the collision mesh's positions and its indices
    pub fn collision_sum(&self) -> u64 {
        self.sum
    }

    /// the collision mesh's triangles
    pub fn triangles(&self) -> usize {
        self.triangles
    }

    /// is the segment from a to b free of the map (a ray; b itself does not count)
    pub fn clear(&self, a: Vec3, b: Vec3) -> bool {
        let d = b - a;
        let len = d.length();
        if len < 1e-9 {
            return true;
        }
        self.world.ray_cast(a, d * (1.0 / len), len, &Filter::ALL).is_none()
    }
}

/// the map of part 1 (apps/game1/content/werft.jfs, baked by tools/content.py)
pub const WERFT: &[u8] = include_bytes!("../../content/werft.g1m");

/// the map of part 1, parsed once per call (a server parses it once and shares it)
pub fn werft() -> Arc<Map> {
    match Map::parse(WERFT) {
        Ok(m) => Arc::new(m),
        // (the build's own file: a broken one fails the tests long before anybody plays)
        Err(e) => panic!("werft.g1m: {}", e),
    }
}

// host -- the script's view of lib/geom: the JFScript host functions geom.*, profile.*, vec.*, model.* and the
// methods of a mesh.  The script only describes; every vertex is computed here in Rust.  A mesh is a map
// {mesh: id, tris: n} (JFScript has no host objects yet: docs/GEOM.md, "Was JFScript braucht"), whose
// methods reach the host as "map.move" and so on; meshes never change, every operation makes a new one.
// Wrong arguments throw a German message that names the function and the parameter; the VM adds the line
// and column of the call.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use geom::{Axis, Mesh, Vec3};
use script::{Cx, Fail, Host, Value};

/// the names the host answers (compile_with: any other unknown name is an error before the run)
pub const GLOBALS: &[&str] = &["geom", "model", "profile", "vec", "params"];

/// the most triangles of all meshes a run makes, intermediate ones included
pub const ARENA_TRIANGLES: usize = 4_000_000;

/// one host function for the reference (`modeltool --api`) and the messages
pub struct ApiEntry {
    pub name: &'static str,
    pub params: &'static str,
    pub doc: &'static str,
}

/// every host function and method with its parameters (optional ones with their default) and what it does
pub const API: &[ApiEntry] = &[
    ApiEntry { name: "geom.box", params: "b, h, t", doc: "Quader b x h x t (x, y, z) in Metern; steht auf y = 0, mittig über dem Ursprung" },
    ApiEntry { name: "geom.cylinder", params: "r, h, segmente = 32", doc: "Zylinder um die Y-Achse auf y = 0; glatte Seiten, flache Deckel" },
    ApiEntry { name: "geom.cone", params: "r, h, segmente = 32, r_oben = 0", doc: "Kegel (mit r_oben > 0 ein Kegelstumpf) um die Y-Achse auf y = 0" },
    ApiEntry { name: "geom.sphere", params: "r, segmente = 32, ringe = segmente / 2", doc: "Kugel, liegt auf y = 0 (Mitte bei y = r)" },
    ApiEntry { name: "geom.torus", params: "R, r, segmente = 32, seiten = 16", doc: "Torus, liegt flach auf y = 0; R zur Mitte des Rings, r der Ring" },
    ApiEntry { name: "geom.stairs", params: "b, h, t, stufen", doc: "Treppe b breit (x), h hoch, t tief (z); erste Stufe vorn (z = t/2), steigt nach hinten" },
    ApiEntry { name: "geom.arch", params: "b, h, t, staerke, segmente = 16", doc: "Torbogen: zwei Pfeiler der Stärke, Öffnung mit Halbkreis, darüber die Stärke" },
    ApiEntry { name: "geom.extrude", params: "profil, tiefe", doc: "Profil [[x, y], ...] (Ansicht von vorn) entlang Z, mittig um z = 0" },
    ApiEntry { name: "geom.prism", params: "grundriss, h", doc: "Grundriss [[x, z], ...] (Ansicht von oben) von y = 0 um h nach oben" },
    ApiEntry { name: "geom.sweep", params: "profil, pfad, geschlossen = false, glatt = 30", doc: "Profil [[x, y], ...] entlang eines Pfads [[x, y, z], ...]; Ecken auf Gehrung" },
    ApiEntry { name: "geom.lathe", params: "profil, segmente = 32, glatt = 30", doc: "Drehkörper: Profil [[radius, y], ...] um die Y-Achse" },
    ApiEntry { name: "geom.merge", params: "m1, m2, ... | [m1, m2, ...]", doc: "Meshes zu einem zusammenfügen" },
    ApiEntry { name: "m.move", params: "x, y, z | [x, y, z]", doc: "verschoben" },
    ApiEntry { name: "m.place", params: "x, y, z | [x, y, z]", doc: "so verschoben, dass die Mitte der Unterseite der Box bei (x, y, z) liegt" },
    ApiEntry { name: "m.center", params: "", doc: "so verschoben, dass die Mitte der Box im Ursprung liegt" },
    ApiEntry { name: "m.rotate_x", params: "grad", doc: "um die X-Achse gedreht (rechtshändig; Vielfache von 90 exakt); ebenso rotate_y, rotate_z" },
    ApiEntry { name: "m.rotate", params: "grad, [ax, ay, az]", doc: "um eine Achse durch den Ursprung gedreht" },
    ApiEntry { name: "m.scale", params: "s | sx, sy, sz | [sx, sy, sz]", doc: "skaliert um den Ursprung" },
    ApiEntry { name: "m.mirror", params: "\"x\" | \"y\" | \"z\"", doc: "gespiegelt an der Ebene durch den Ursprung (x -> -x für \"x\"), Flächen bleiben außen" },
    ApiEntry { name: "m.flip", params: "", doc: "Flächen umgedreht (innen nach außen)" },
    ApiEntry { name: "m.weld", params: "toleranz = 0", doc: "Ecken an derselben Stelle zu einem Punkt verbunden (eine Hälfte und ihr Spiegelbild zu einer Fläche)" },
    ApiEntry { name: "m.array", params: "anzahl, dx, dy, dz | anzahl, [dx, dy, dz]", doc: "anzahl Kopien, Kopie i um i * (dx, dy, dz) verschoben" },
    ApiEntry { name: "m.radial", params: "anzahl, schritt = 360 / anzahl", doc: "anzahl Kopien um die Y-Achse, Kopie i um i * schritt Grad gedreht" },
    ApiEntry { name: "m.flat", params: "", doc: "flache Normalen (harte Kanten)" },
    ApiEntry { name: "m.smooth", params: "grad = 30", doc: "glatte Normalen über Kanten flacher als grad" },
    ApiEntry { name: "m.bounds", params: "", doc: "{min, max, size, center} in Metern" },
    ApiEntry { name: "m.area", params: "", doc: "Oberfläche in m²" },
    ApiEntry { name: "m.volume", params: "", doc: "Volumen in m³ (bei geschlossenen Meshes)" },
    ApiEntry { name: "m.check", params: "", doc: "{ok, closed, outward, open_edges, flipped_edges, degenerate, triangles}" },
    ApiEntry { name: "m.tris", params: "", doc: "(Feld) die Zahl der Dreiecke" },
    ApiEntry { name: "profile.rect", params: "b, h", doc: "Rechteck um den Ursprung" },
    ApiEntry { name: "profile.circle", params: "r, n = 32", doc: "Vieleck mit n Ecken auf einem Kreis um den Ursprung" },
    ApiEntry { name: "profile.rounded", params: "b, h, r, n = 4", doc: "Rechteck mit abgerundeten Ecken (n Segmente je Ecke)" },
    ApiEntry { name: "profile.arc", params: "r, von, bis, n = 16", doc: "Punkte eines Kreisbogens (Grad), beide Enden dabei: ein offener Linienzug" },
    ApiEntry { name: "profile.move", params: "p, dx, dy", doc: "verschoben" },
    ApiEntry { name: "profile.scale", params: "p, sx, sy = sx", doc: "skaliert um den Ursprung" },
    ApiEntry { name: "profile.rotate", params: "p, grad", doc: "gedreht um den Ursprung" },
    ApiEntry { name: "profile.area", params: "p", doc: "Fläche (positiv gegen den Uhrzeigersinn)" },
    ApiEntry { name: "vec.add", params: "a, b", doc: "a + b (Listen aus 2 oder 3 Zahlen); ebenso vec.sub" },
    ApiEntry { name: "vec.mul", params: "a, s", doc: "a * s" },
    ApiEntry { name: "vec.dot", params: "a, b", doc: "Skalarprodukt" },
    ApiEntry { name: "vec.cross", params: "a, b", doc: "Kreuzprodukt (3D)" },
    ApiEntry { name: "vec.len", params: "a", doc: "Länge; vec.dist(a, b) der Abstand" },
    ApiEntry { name: "vec.norm", params: "a", doc: "Einheitsvektor" },
    ApiEntry { name: "vec.lerp", params: "a, b, t", doc: "a + (b - a) * t" },
    ApiEntry { name: "model.add", params: "m, material = \"grau\", name", doc: "Teil des Modells; material: Name oder {color, roughness, metallic}" },
    ApiEntry { name: "model.material", params: "name, {color: \"#rrggbb\" | [r, g, b], roughness = 0.8, metallic = 0}", doc: "Material festlegen (Farben in sRGB, 0 bis 1)" },
    ApiEntry { name: "model.name", params: "text", doc: "Name des Modells" },
    ApiEntry { name: "model.budget", params: "dreiecke", doc: "höchstens so viele Dreiecke (Prüfung im Bericht)" },
    ApiEntry { name: "model.expect_size", params: "x, y, z, toleranz = 0.001 | [x, y, z], toleranz", doc: "erwartete Maße der Box in Metern (Prüfung im Bericht); nil: diese Achse nicht prüfen" },
    ApiEntry { name: "model.uv_scale", params: "meter = 1", doc: "Meter je UV-Einheit der Box-Projektion" },
    ApiEntry { name: "params", params: "", doc: "Map der Parameter von --param NAME=WERT; params.get(\"b\", 0.8)" },
];

/// the built-in materials: name, sRGB colour, roughness, metallic
pub const PALETTE: &[(&str, [f64; 3], f64, f64)] = &[
    ("grau", [0.60, 0.60, 0.60], 0.8, 0.0),
    ("weiss", [0.93, 0.93, 0.91], 0.8, 0.0),
    ("schwarz", [0.12, 0.12, 0.12], 0.8, 0.0),
    ("holz", [0.63, 0.42, 0.24], 0.75, 0.0),
    ("dunkelholz", [0.36, 0.23, 0.12], 0.75, 0.0),
    ("stein", [0.56, 0.54, 0.50], 0.9, 0.0),
    ("beton", [0.66, 0.66, 0.64], 0.9, 0.0),
    ("ziegel", [0.64, 0.27, 0.18], 0.85, 0.0),
    ("dach", [0.42, 0.16, 0.12], 0.8, 0.0),
    ("metall", [0.72, 0.74, 0.76], 0.35, 1.0),
    ("rot", [0.75, 0.22, 0.17], 0.6, 0.0),
    ("gruen", [0.24, 0.56, 0.25], 0.6, 0.0),
    ("blau", [0.18, 0.44, 0.71], 0.6, 0.0),
    ("gelb", [0.89, 0.70, 0.24], 0.6, 0.0),
    ("glas", [0.66, 0.83, 0.90], 0.1, 0.0),
];

/// a material of the model (colour in sRGB, 0 ..= 1)
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialDef {
    pub name: String,
    pub color: [f64; 3],
    pub roughness: f64,
    pub metallic: f64,
}

/// a part: a mesh with a material
#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    pub mesh: Mesh,
    pub material: usize,
}

/// a rule the script stated (model.expect_size); an axis of NaN is not checked
#[derive(Clone, Debug)]
pub struct Expect {
    pub size: [f64; 3],
    pub tolerance: f64,
}

/// what the script built
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub parts: Vec<Part>,
    /// the materials the parts use, in the order of first use
    pub materials: Vec<MaterialDef>,
    pub budget: Option<u64>,
    pub expects: Vec<Expect>,
    /// metres per UV unit (1)
    pub uv_scale: f64,
}

/// the host of one run
pub struct ModelHost<'a> {
    meshes: Vec<Mesh>,
    arena: usize,
    defined: Vec<MaterialDef>,
    pub model: Model,
    loader: &'a mut dyn FnMut(&str) -> Result<String, String>,
    /// (path, source) of every module the script imported, in order
    pub imports: Vec<(String, String)>,
    /// microseconds spent in host functions (the geometry)
    pub host_us: u64,
}

impl<'a> ModelHost<'a> {
    pub fn new(loader: &'a mut dyn FnMut(&str) -> Result<String, String>) -> ModelHost<'a> {
        ModelHost {
            meshes: Vec::new(),
            arena: 0,
            defined: Vec::new(),
            model: Model { uv_scale: 1.0, ..Model::default() },
            loader,
            imports: Vec::new(),
            host_us: 0,
        }
    }
}

impl Host for ModelHost<'_> {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let t0 = plat::time_us();
        let r = self.dispatch(cx, name, args);
        self.host_us += plat::time_us().saturating_sub(t0);
        r
    }

    fn load(&mut self, cx: &mut Cx, path: &str) -> Result<String, String> {
        let src = (self.loader)(path)?;
        cx.charge(1 + src.len() as u64 / 64);
        self.imports.push((String::from(path), src.clone()));
        Ok(src)
    }
}

// ---------------------------------------------------------------- arguments

/// a value for a message: as print shows it, at most 40 characters, with its type
fn shown(cx: &Cx, v: Value) -> String {
    let mut s = cx.display(v);
    if s.chars().count() > 40 {
        s = s.chars().take(37).collect::<String>() + "...";
    }
    match v {
        Value::Str(_) => format!("str \"{}\"", s),
        Value::Nil => String::from("nil"),
        _ => format!("{} {}", v.type_name(), s),
    }
}

fn throw(cx: &mut Cx, f: &str, msg: &str) -> Fail {
    cx.error(&format!("{}: {}", f, msg))
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Nil)
}

/// a number (int or float)
fn number_of(v: Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(i as f64),
        Value::Float(f) => Some(f),
        _ => None,
    }
}

/// argument i as a finite number
fn num(cx: &mut Cx, f: &str, args: &[Value], i: usize, what: &str) -> Result<f64, Fail> {
    let v = arg(args, i);
    match number_of(v) {
        Some(x) if x.is_finite() => Ok(x),
        _ => {
            let s = shown(cx, v);
            Err(throw(cx, f, &format!("{} muss eine Zahl sein, nicht {}", what, s)))
        }
    }
}

/// argument i as a finite number, `default` when it is missing (nil)
fn num_or(cx: &mut Cx, f: &str, args: &[Value], i: usize, default: f64, what: &str) -> Result<f64, Fail> {
    if matches!(arg(args, i), Value::Nil) { Ok(default) } else { num(cx, f, args, i, what) }
}

/// argument i as a count (an int, or a float without fraction), `default` when it is missing
fn count_or(cx: &mut Cx, f: &str, args: &[Value], i: usize, default: Option<usize>, what: &str) -> Result<usize, Fail> {
    let v = arg(args, i);
    if let (Value::Nil, Some(d)) = (v, default) {
        return Ok(d);
    }
    let n = match v {
        Value::Int(n) if n >= 0 => Some(n as u64),
        Value::Float(x) if x >= 0.0 && x <= 1e9 && x == geom::math::floor(x) => Some(x as u64),
        _ => None,
    };
    match n {
        Some(n) if n <= 1_000_000_000 => Ok(n as usize),
        _ => {
            let s = shown(cx, v);
            Err(throw(cx, f, &format!("{} muss eine ganze Zahl ab 0 sein, nicht {}", what, s)))
        }
    }
}

/// a list of exactly `n` numbers
fn numbers_of(cx: &Cx, v: Value, n: usize) -> Option<Vec<f64>> {
    let l = cx.list(v)?;
    if l.len() != n {
        return None;
    }
    let out: Vec<f64> = l.iter().filter_map(|&x| number_of(x)).filter(|x| x.is_finite()).collect();
    if out.len() == n { Some(out) } else { None }
}

/// a vector from argument i: a list [x, y, z] there, or three numbers at i, i + 1, i + 2
fn vec3(cx: &mut Cx, f: &str, args: &[Value], i: usize, what: &str) -> Result<Vec3, Fail> {
    if let Some(v) = numbers_of(cx, arg(args, i), 3) {
        return Ok(Vec3::new(v[0], v[1], v[2]));
    }
    if matches!(arg(args, i), Value::List(_)) {
        let s = shown(cx, arg(args, i));
        return Err(throw(cx, f, &format!("{} muss eine Liste aus 3 Zahlen sein, nicht {}", what, s)));
    }
    Ok(Vec3::new(num(cx, f, args, i, what)?, num(cx, f, args, i + 1, what)?, num(cx, f, args, i + 2, what)?))
}

/// a list of 2D points [[x, y], ...]
fn points2(cx: &mut Cx, f: &str, v: Value, what: &str) -> Result<Vec<[f64; 2]>, Fail> {
    let items: Option<Vec<Value>> = cx.list(v).map(|l| l.to_vec());
    let Some(items) = items else {
        let s = shown(cx, v);
        return Err(throw(cx, f, &format!("{} muss eine Liste von Punkten [[x, y], ...] sein, nicht {}", what, s)));
    };
    cx.spend(1 + items.len() as u64 / 8)?;
    let mut out = Vec::with_capacity(items.len());
    for (k, &p) in items.iter().enumerate() {
        match numbers_of(cx, p, 2) {
            Some(q) => out.push([q[0], q[1]]),
            None => {
                let s = shown(cx, p);
                return Err(throw(cx, f, &format!("{}[{}] muss ein Punkt [x, y] aus zwei Zahlen sein, nicht {}", what, k, s)));
            }
        }
    }
    Ok(out)
}

/// a list of 3D points [[x, y, z], ...]
fn points3(cx: &mut Cx, f: &str, v: Value, what: &str) -> Result<Vec<Vec3>, Fail> {
    let items: Option<Vec<Value>> = cx.list(v).map(|l| l.to_vec());
    let Some(items) = items else {
        let s = shown(cx, v);
        return Err(throw(cx, f, &format!("{} muss eine Liste von Punkten [[x, y, z], ...] sein, nicht {}", what, s)));
    };
    cx.spend(1 + items.len() as u64 / 8)?;
    let mut out = Vec::with_capacity(items.len());
    for (k, &p) in items.iter().enumerate() {
        match numbers_of(cx, p, 3) {
            Some(q) => out.push(Vec3::new(q[0], q[1], q[2])),
            None => {
                let s = shown(cx, p);
                return Err(throw(cx, f, &format!("{}[{}] muss ein Punkt [x, y, z] aus drei Zahlen sein, nicht {}", what, k, s)));
            }
        }
    }
    Ok(out)
}

/// a new list of floats
fn float_list(cx: &mut Cx, v: &[f64]) -> Result<Value, Fail> {
    cx.new_list(v.iter().map(|&x| Value::Float(x)).collect())
}

/// a new list of 2D points
fn point_list(cx: &mut Cx, pts: &[[f64; 2]]) -> Result<Value, Fail> {
    let mut items = Vec::with_capacity(pts.len());
    for p in pts {
        items.push(float_list(cx, p)?);
    }
    cx.spend(1 + pts.len() as u64 / 8)?;
    cx.new_list(items)
}

/// a map of str keys
fn map_of(cx: &mut Cx, fields: &[(&str, Value)]) -> Result<Value, Fail> {
    let m = cx.new_map()?;
    for &(k, v) in fields {
        let key = cx.new_str(k)?;
        cx.map_set(m, key, v)?;
    }
    Ok(m)
}

fn vec_value(cx: &mut Cx, v: Vec3) -> Result<Value, Fail> {
    float_list(cx, &[v.x, v.y, v.z])
}

// ---------------------------------------------------------------- dispatch

impl ModelHost<'_> {
    fn dispatch(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let (ns, f) = name.split_once('.').unwrap_or(("", name));
        match ns {
            "geom" => match f {
                "box" | "cylinder" | "cone" | "sphere" | "torus" | "stairs" | "arch" | "extrude" | "prism" | "sweep" | "lathe" => {
                    self.construct(cx, name, f, args)
                }
                "merge" => self.merge(cx, name, args),
                _ => {
                    // geom.move(m, ...) and the other methods as functions
                    if self.mesh_id(cx, arg(args, 0)).is_some() {
                        self.method(cx, f, args)
                    } else {
                        Err(Fail::Unknown)
                    }
                }
            },
            "map" if self.mesh_id(cx, arg(args, 0)).is_some() => self.method(cx, f, args),
            "profile" => self.profile(cx, name, f, args),
            "vec" => vector(cx, name, f, args),
            "model" => self.model_fn(cx, name, f, args),
            _ => Err(Fail::Unknown),
        }
    }

    /// the id of a mesh handle
    fn mesh_id(&self, cx: &Cx, v: Value) -> Option<usize> {
        if !matches!(v, Value::Map(_)) {
            return None;
        }
        match cx.map_get_str(v, "mesh") {
            Some(Value::Int(i)) if i >= 0 && (i as usize) < self.meshes.len() => Some(i as usize),
            _ => None,
        }
    }

    /// argument i as a mesh
    fn mesh_arg(&self, cx: &mut Cx, f: &str, args: &[Value], i: usize, what: &str) -> Result<usize, Fail> {
        match self.mesh_id(cx, arg(args, i)) {
            Some(id) => Ok(id),
            None => {
                let s = shown(cx, arg(args, i));
                Err(throw(cx, f, &format!("{} muss ein Mesh sein (von geom.box, geom.extrude, ...), nicht {}", what, s)))
            }
        }
    }

    /// keep a mesh and give the script its handle
    fn handle(&mut self, cx: &mut Cx, f: &str, m: Mesh) -> Result<Value, Fail> {
        let tris = m.triangle_count();
        if self.arena + tris > ARENA_TRIANGLES {
            return Err(throw(cx, f, &format!(
                "zusammen mehr als {} Dreiecke in allen Meshes dieses Laufs (Zwischenstände mitgezählt): weniger Segmente oder Kopien",
                ARENA_TRIANGLES
            )));
        }
        cx.spend(1 + tris as u64 / 64)?;
        self.arena += tris;
        self.meshes.push(m);
        let id = Value::Int(self.meshes.len() as i64 - 1);
        map_of(cx, &[("mesh", id), ("tris", Value::Int(tris as i64))])
    }

    fn construct(&mut self, cx: &mut Cx, name: &str, f: &str, args: &[Value]) -> Result<Value, Fail> {
        let r = match f {
            "box" => {
                let (w, h, d) = (num(cx, name, args, 0, "b")?, num(cx, name, args, 1, "h")?, num(cx, name, args, 2, "t")?);
                geom::cuboid(w, h, d)
            }
            "cylinder" => {
                let (r, h) = (num(cx, name, args, 0, "r")?, num(cx, name, args, 1, "h")?);
                let n = count_or(cx, name, args, 2, Some(32), "segmente")?;
                geom::cylinder(r, h, n)
            }
            "cone" => {
                let (r, h) = (num(cx, name, args, 0, "r")?, num(cx, name, args, 1, "h")?);
                let n = count_or(cx, name, args, 2, Some(32), "segmente")?;
                let top = num_or(cx, name, args, 3, 0.0, "r_oben")?;
                geom::cone(r, h, n, top)
            }
            "sphere" => {
                let r = num(cx, name, args, 0, "r")?;
                let n = count_or(cx, name, args, 1, Some(32), "segmente")?;
                let stacks = count_or(cx, name, args, 2, Some((n / 2).max(2)), "ringe")?;
                geom::sphere(r, n, stacks)
            }
            "torus" => {
                let (big, small) = (num(cx, name, args, 0, "R")?, num(cx, name, args, 1, "r")?);
                let n = count_or(cx, name, args, 2, Some(32), "segmente")?;
                let sides = count_or(cx, name, args, 3, Some(16), "seiten")?;
                geom::torus(big, small, n, sides)
            }
            "stairs" => {
                let (w, h, d) = (num(cx, name, args, 0, "b")?, num(cx, name, args, 1, "h")?, num(cx, name, args, 2, "t")?);
                let steps = count_or(cx, name, args, 3, None, "stufen")?;
                geom::stairs(w, h, d, steps)
            }
            "arch" => {
                let (w, h, d) = (num(cx, name, args, 0, "b")?, num(cx, name, args, 1, "h")?, num(cx, name, args, 2, "t")?);
                let t = num(cx, name, args, 3, "staerke")?;
                let n = count_or(cx, name, args, 4, Some(16), "segmente")?;
                geom::arch(w, h, d, t, n)
            }
            "extrude" => {
                let p = points2(cx, name, arg(args, 0), "profil")?;
                let d = num(cx, name, args, 1, "tiefe")?;
                geom::extrude(&p, d)
            }
            "prism" => {
                let p = points2(cx, name, arg(args, 0), "grundriss")?;
                let h = num(cx, name, args, 1, "h")?;
                geom::prism(&p, h)
            }
            "sweep" => {
                let p = points2(cx, name, arg(args, 0), "profil")?;
                let path = points3(cx, name, arg(args, 1), "pfad")?;
                let closed = arg(args, 2).truthy();
                let smooth = num_or(cx, name, args, 3, geom::DEFAULT_SMOOTH_DEG, "glatt")?;
                geom::sweep(&p, &path, closed, smooth)
            }
            _ => {
                let p = points2(cx, name, arg(args, 0), "profil")?;
                let n = count_or(cx, name, args, 1, Some(32), "segmente")?;
                let smooth = num_or(cx, name, args, 2, geom::DEFAULT_SMOOTH_DEG, "glatt")?;
                geom::lathe(&p, n, smooth)
            }
        };
        match r {
            Ok(m) => self.handle(cx, name, m),
            Err(e) => Err(throw(cx, name, e.message())),
        }
    }

    fn merge(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        // geom.merge(a, b, ...) or geom.merge([a, b, ...])
        let items: Vec<Value> = match (args.len(), cx.list(arg(args, 0)).map(|l| l.to_vec())) {
            (1, Some(l)) => l,
            _ => args.to_vec(),
        };
        if items.is_empty() {
            return Err(throw(cx, name, "braucht mindestens ein Mesh"));
        }
        let mut ids = Vec::with_capacity(items.len());
        for (k, &v) in items.iter().enumerate() {
            match self.mesh_id(cx, v) {
                Some(id) => ids.push(id),
                None => {
                    let s = shown(cx, v);
                    return Err(throw(cx, name, &format!("Teil {} muss ein Mesh sein, nicht {}", k + 1, s)));
                }
            }
        }
        let refs: Vec<&Mesh> = ids.iter().map(|&i| &self.meshes[i]).collect();
        match Mesh::merge(&refs) {
            Ok(m) => self.handle(cx, name, m),
            Err(e) => Err(throw(cx, name, e.message())),
        }
    }

    /// a method of a mesh (args[0] is the mesh)
    fn method(&mut self, cx: &mut Cx, f: &str, args: &[Value]) -> Result<Value, Fail> {
        let name = format!("{}()", f);
        let name = name.as_str();
        let id = self.mesh_arg(cx, name, args, 0, "das Objekt")?;
        let rest = &args[1..];
        let m = &self.meshes[id];
        let out = match f {
            "move" => {
                let v = vec3(cx, name, rest, 0, "die Verschiebung")?;
                m.translated(v)
            }
            "place" => {
                let v = vec3(cx, name, rest, 0, "der Punkt")?;
                let Some(b) = m.bounds() else { return Err(throw(cx, name, "das Mesh ist leer")) };
                let c = b.center();
                m.translated(Vec3::new(v.x - c.x, v.y - b.min.y, v.z - c.z))
            }
            "center" => {
                let Some(b) = m.bounds() else { return Err(throw(cx, name, "das Mesh ist leer")) };
                m.translated(-b.center())
            }
            "rotate_x" | "rotate_y" | "rotate_z" => {
                let deg = num(cx, name, rest, 0, "grad")?;
                let axis = match f {
                    "rotate_x" => Axis::X,
                    "rotate_y" => Axis::Y,
                    _ => Axis::Z,
                };
                m.rotated(axis, deg)
            }
            "rotate" => {
                let deg = num(cx, name, rest, 0, "grad")?;
                let axis = vec3(cx, name, rest, 1, "die Achse")?;
                match geom::Affine::rotation_about(axis, deg) {
                    Some(a) => m.transformed(&a),
                    None => return Err(throw(cx, name, "die Achse darf nicht (0, 0, 0) sein")),
                }
            }
            "scale" => {
                let s = if rest.len() == 1 && number_of(arg(rest, 0)).is_some() {
                    let s = num(cx, name, rest, 0, "der Faktor")?;
                    Vec3::new(s, s, s)
                } else {
                    vec3(cx, name, rest, 0, "die Faktoren")?
                };
                if s.x == 0.0 || s.y == 0.0 || s.z == 0.0 {
                    return Err(throw(cx, name, "ein Faktor 0 macht das Mesh flach"));
                }
                m.scaled(s)
            }
            "mirror" => {
                let axis = match cx.str(arg(rest, 0)) {
                    Some("x") | Some("X") => Axis::X,
                    Some("y") | Some("Y") => Axis::Y,
                    Some("z") | Some("Z") => Axis::Z,
                    _ => {
                        let s = shown(cx, arg(rest, 0));
                        return Err(throw(cx, name, &format!("die Achse muss \"x\", \"y\" oder \"z\" sein, nicht {}", s)));
                    }
                };
                m.mirrored(axis)
            }
            "flip" => m.flipped(),
            "weld" => {
                let tol = num_or(cx, name, rest, 0, 0.0, "toleranz")?;
                if tol < 0.0 {
                    return Err(throw(cx, name, "die Toleranz darf nicht negativ sein"));
                }
                self.meshes[id].weld(tol)
            }
            "array" => {
                let n = count_or(cx, name, rest, 0, None, "anzahl")?;
                let step = vec3(cx, name, rest, 1, "der Abstand")?;
                match self.meshes[id].array(n, step) {
                    Ok(x) => x,
                    Err(e) => return Err(throw(cx, name, e.message())),
                }
            }
            "radial" => {
                let n = count_or(cx, name, rest, 0, None, "anzahl")?;
                let step = num_or(cx, name, rest, 1, 360.0 / n.max(1) as f64, "schritt")?;
                match self.meshes[id].radial(n, step) {
                    Ok(x) => x,
                    Err(e) => return Err(throw(cx, name, e.message())),
                }
            }
            "flat" => m.flat(),
            "smooth" => {
                let deg = num_or(cx, name, rest, 0, geom::DEFAULT_SMOOTH_DEG, "grad")?;
                self.meshes[id].smooth(deg)
            }
            "bounds" => {
                let Some(b) = m.bounds() else { return Ok(Value::Nil) };
                let (lo, hi, size, c) = (vec_value(cx, b.min)?, vec_value(cx, b.max)?, vec_value(cx, b.size())?, vec_value(cx, b.center())?);
                return map_of(cx, &[("min", lo), ("max", hi), ("size", size), ("center", c)]);
            }
            "area" => return Ok(Value::Float(m.area())),
            "volume" => return Ok(Value::Float(m.volume())),
            "check" => {
                let r = m.check();
                cx.spend(1 + r.triangles as u64 / 64)?;
                return map_of(cx, &[
                    ("ok", Value::Bool(r.ok())),
                    ("closed", Value::Bool(r.closed())),
                    ("outward", Value::Bool(r.outward())),
                    ("open_edges", Value::Int(r.open_edges as i64)),
                    ("flipped_edges", Value::Int(r.flipped_edges as i64)),
                    ("degenerate", Value::Int(r.degenerate as i64)),
                    ("triangles", Value::Int(r.triangles as i64)),
                ]);
            }
            _ => {
                let known: Vec<&str> = API.iter().filter_map(|e| e.name.strip_prefix("m.")).collect();
                return Err(cx.error(&format!("Ein Mesh hat keine Methode {} (es gibt: {})", f, known.join(", "))));
            }
        };
        self.handle(cx, name, out)
    }

    fn profile(&mut self, cx: &mut Cx, name: &str, f: &str, args: &[Value]) -> Result<Value, Fail> {
        let pts: Vec<[f64; 2]> = match f {
            "rect" => {
                let (w, h) = (num(cx, name, args, 0, "b")?, num(cx, name, args, 1, "h")?);
                geom::poly::rect(w, h)
            }
            "circle" => {
                let r = num(cx, name, args, 0, "r")?;
                let n = count_or(cx, name, args, 1, Some(32), "n")?;
                if !(3..=geom::MAX_SEGMENTS).contains(&n) {
                    return Err(throw(cx, name, &format!("n muss zwischen 3 und {} liegen", geom::MAX_SEGMENTS)));
                }
                geom::poly::circle(r, n)
            }
            "rounded" => {
                let (w, h, r) = (num(cx, name, args, 0, "b")?, num(cx, name, args, 1, "h")?, num(cx, name, args, 2, "r")?);
                let n = count_or(cx, name, args, 3, Some(4), "n")?;
                if !(1..=256).contains(&n) {
                    return Err(throw(cx, name, "n muss zwischen 1 und 256 liegen"));
                }
                geom::poly::rounded_rect(w, h, r, n)
            }
            "arc" => {
                let r = num(cx, name, args, 0, "r")?;
                let (from, to) = (num(cx, name, args, 1, "von")?, num(cx, name, args, 2, "bis")?);
                let n = count_or(cx, name, args, 3, Some(16), "n")?;
                if !(1..=geom::MAX_SEGMENTS).contains(&n) {
                    return Err(throw(cx, name, &format!("n muss zwischen 1 und {} liegen", geom::MAX_SEGMENTS)));
                }
                geom::poly::arc(r, from, to, n)
            }
            "move" => {
                let p = points2(cx, name, arg(args, 0), "p")?;
                let (dx, dy) = (num(cx, name, args, 1, "dx")?, num(cx, name, args, 2, "dy")?);
                p.iter().map(|q| [q[0] + dx, q[1] + dy]).collect()
            }
            "scale" => {
                let p = points2(cx, name, arg(args, 0), "p")?;
                let sx = num(cx, name, args, 1, "sx")?;
                let sy = num_or(cx, name, args, 2, sx, "sy")?;
                p.iter().map(|q| [q[0] * sx, q[1] * sy]).collect()
            }
            "rotate" => {
                let p = points2(cx, name, arg(args, 0), "p")?;
                let (s, c) = geom::math::sin_cos_deg(num(cx, name, args, 1, "grad")?);
                p.iter().map(|q| [q[0] * c - q[1] * s + 0.0, q[0] * s + q[1] * c + 0.0]).collect()
            }
            "area" => {
                let p = points2(cx, name, arg(args, 0), "p")?;
                return Ok(Value::Float(geom::poly::area(&p)));
            }
            _ => return Err(Fail::Unknown),
        };
        point_list(cx, &pts)
    }

    fn material_by(&mut self, cx: &mut Cx, f: &str, v: Value) -> Result<usize, Fail> {
        let def = match v {
            Value::Nil => self.named_material(cx, f, "grau")?,
            Value::Str(_) => {
                let n = cx.str(v).unwrap_or("").to_string();
                self.named_material(cx, f, &n)?
            }
            Value::Map(_) => {
                let n = format!("material{}", self.model.materials.len() + 1);
                material_from_map(cx, f, &n, v)?
            }
            _ => {
                let s = shown(cx, v);
                return Err(throw(cx, f, &format!("material muss ein Name oder eine Map {{color: ...}} sein, nicht {}", s)));
            }
        };
        if let Some(i) = self.model.materials.iter().position(|m| *m == def) {
            return Ok(i);
        }
        self.model.materials.push(def);
        Ok(self.model.materials.len() - 1)
    }

    fn named_material(&mut self, cx: &mut Cx, f: &str, n: &str) -> Result<MaterialDef, Fail> {
        if let Some(m) = self.defined.iter().find(|m| m.name == n) {
            return Ok(m.clone());
        }
        if let Some(&(name, color, roughness, metallic)) = PALETTE.iter().find(|p| p.0 == n) {
            return Ok(MaterialDef { name: String::from(name), color, roughness, metallic });
        }
        let mut names: Vec<&str> = self.defined.iter().map(|m| m.name.as_str()).collect();
        names.extend(PALETTE.iter().map(|p| p.0));
        Err(throw(cx, f, &format!("das Material \"{}\" gibt es nicht (model.material legt eins fest; es gibt: {})", n, names.join(", "))))
    }

    fn model_fn(&mut self, cx: &mut Cx, name: &str, f: &str, args: &[Value]) -> Result<Value, Fail> {
        match f {
            "add" => {
                let id = self.mesh_arg(cx, name, args, 0, "m")?;
                let material = self.material_by(cx, name, arg(args, 1))?;
                let part_name = match arg(args, 2) {
                    Value::Nil => format!("teil{}", self.model.parts.len() + 1),
                    v => cx.display(v),
                };
                if self.meshes[id].triangle_count() == 0 {
                    return Err(throw(cx, name, "das Mesh hat keine Dreiecke"));
                }
                self.model.parts.push(Part { name: part_name, mesh: self.meshes[id].clone(), material });
                Ok(Value::Int(self.model.parts.len() as i64))
            }
            "material" => {
                let Some(n) = cx.str(arg(args, 0)).map(String::from) else {
                    let s = shown(cx, arg(args, 0));
                    return Err(throw(cx, name, &format!("name muss ein str sein, nicht {}", s)));
                };
                let def = material_from_map(cx, name, &n, arg(args, 1))?;
                self.defined.retain(|m| m.name != n);
                self.defined.push(def);
                cx.new_str(&n)
            }
            "name" => {
                self.model.name = cx.display(arg(args, 0));
                Ok(Value::Nil)
            }
            "budget" => {
                let n = count_or(cx, name, args, 0, None, "dreiecke")?;
                self.model.budget = Some(n as u64);
                Ok(Value::Nil)
            }
            "expect_size" => {
                // x, y, z or [x, y, z]; nil for an axis that is not checked
                let (dims, ti): (Vec<Value>, usize) = match cx.list(arg(args, 0)).map(|l| l.to_vec()) {
                    Some(l) if l.len() == 3 => (l, 1),
                    Some(_) => return Err(throw(cx, name, "die Maße müssen eine Liste [x, y, z] sein")),
                    None => ((0..3).map(|i| arg(args, i)).collect(), 3),
                };
                let mut size = [f64::NAN; 3];
                for (k, &v) in dims.iter().enumerate() {
                    match (v, number_of(v)) {
                        (Value::Nil, _) => {}
                        (_, Some(x)) if x >= 0.0 && x.is_finite() => size[k] = x,
                        _ => {
                            let s = shown(cx, v);
                            return Err(throw(cx, name, &format!("{} muss eine Zahl ab 0 oder nil sein, nicht {}", ["x", "y", "z"][k], s)));
                        }
                    }
                }
                let tol = num_or(cx, name, args, ti, 0.001, "toleranz")?;
                if tol < 0.0 {
                    return Err(throw(cx, name, "die Toleranz darf nicht negativ sein"));
                }
                self.model.expects.push(Expect { size, tolerance: tol });
                Ok(Value::Nil)
            }
            "uv_scale" => {
                let s = num(cx, name, args, 0, "meter")?;
                if !(s > 0.0) {
                    return Err(throw(cx, name, "meter muss größer 0 sein"));
                }
                self.model.uv_scale = s;
                Ok(Value::Nil)
            }
            _ => Err(Fail::Unknown),
        }
    }
}

/// a material from {color, roughness, metallic}
fn material_from_map(cx: &mut Cx, f: &str, name: &str, v: Value) -> Result<MaterialDef, Fail> {
    if !matches!(v, Value::Map(_)) {
        let s = shown(cx, v);
        return Err(throw(cx, f, &format!("das Material muss eine Map {{color: \"#rrggbb\", roughness, metallic}} sein, nicht {}", s)));
    }
    let color_v = cx.map_get_str(v, "color").unwrap_or(Value::Nil);
    let color = if let Some(c) = numbers_of(cx, color_v, 3) {
        [c[0], c[1], c[2]]
    } else if let Some(hex) = cx.str(color_v).and_then(parse_hex) {
        hex
    } else {
        let s = shown(cx, color_v);
        return Err(throw(cx, f, &format!("color muss \"#rrggbb\" oder [r, g, b] (0 bis 1) sein, nicht {}", s)));
    };
    if color.iter().any(|&c| !(0.0..=1.0).contains(&c)) {
        return Err(throw(cx, f, "die Farbanteile müssen zwischen 0 und 1 liegen"));
    }
    let get = |cx: &mut Cx, key: &str, d: f64| -> Result<f64, Fail> {
        match cx.map_get_str(v, key) {
            None | Some(Value::Nil) => Ok(d),
            Some(x) => match number_of(x) {
                Some(n) if (0.0..=1.0).contains(&n) => Ok(n),
                _ => {
                    let s = shown(cx, x);
                    Err(throw(cx, f, &format!("{} muss eine Zahl von 0 bis 1 sein, nicht {}", key, s)))
                }
            },
        }
    };
    let roughness = get(cx, "roughness", 0.8)?;
    let metallic = get(cx, "metallic", 0.0)?;
    Ok(MaterialDef { name: String::from(name), color, roughness, metallic })
}

/// "#rrggbb" (or "rrggbb") as sRGB 0 ..= 1
pub fn parse_hex(s: &str) -> Option<[f64; 3]> {
    let h = s.strip_prefix('#').unwrap_or(s);
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    Some([c(0)?, c(2)?, c(4)?])
}

/// vec.*: small vector helpers on lists of 2 or 3 numbers
fn vector(cx: &mut Cx, name: &str, f: &str, args: &[Value]) -> Result<Value, Fail> {
    let get = |cx: &mut Cx, i: usize, what: &str| -> Result<Vec<f64>, Fail> {
        let v = arg(args, i);
        let n = cx.list(v).map_or(0, |l| l.len());
        match numbers_of(cx, v, n).filter(|_| n == 2 || n == 3) {
            Some(x) => Ok(x),
            None => {
                let s = shown(cx, v);
                Err(throw(cx, name, &format!("{} muss eine Liste aus 2 oder 3 Zahlen sein, nicht {}", what, s)))
            }
        }
    };
    let same = |cx: &mut Cx, a: &[f64], b: &[f64]| -> Result<(), Fail> {
        if a.len() == b.len() { Ok(()) } else { Err(throw(cx, name, "a und b müssen gleich lang sein")) }
    };
    let out: Vec<f64> = match f {
        "add" | "sub" => {
            let (a, b) = (get(cx, 0, "a")?, get(cx, 1, "b")?);
            same(cx, &a, &b)?;
            a.iter().zip(&b).map(|(x, y)| if f == "add" { x + y } else { x - y }).collect()
        }
        "mul" => {
            let a = get(cx, 0, "a")?;
            let s = num(cx, name, args, 1, "s")?;
            a.iter().map(|x| x * s).collect()
        }
        "dot" => {
            let (a, b) = (get(cx, 0, "a")?, get(cx, 1, "b")?);
            same(cx, &a, &b)?;
            return Ok(Value::Float(a.iter().zip(&b).fold(0.0, |s, (x, y)| s + x * y)));
        }
        "cross" => {
            let (a, b) = (get(cx, 0, "a")?, get(cx, 1, "b")?);
            if a.len() != 3 || b.len() != 3 {
                return Err(throw(cx, name, "das Kreuzprodukt braucht zwei Vektoren aus 3 Zahlen"));
            }
            let c = Vec3::new(a[0], a[1], a[2]).cross(Vec3::new(b[0], b[1], b[2]));
            vec![c.x, c.y, c.z]
        }
        "len" => {
            let a = get(cx, 0, "a")?;
            return Ok(Value::Float(geom::math::sqrt(a.iter().fold(0.0, |s, x| s + x * x))));
        }
        "dist" => {
            let (a, b) = (get(cx, 0, "a")?, get(cx, 1, "b")?);
            same(cx, &a, &b)?;
            return Ok(Value::Float(geom::math::sqrt(a.iter().zip(&b).fold(0.0, |s, (x, y)| s + (x - y) * (x - y)))));
        }
        "norm" => {
            let a = get(cx, 0, "a")?;
            let l = geom::math::sqrt(a.iter().fold(0.0, |s, x| s + x * x));
            if !(l > 0.0) {
                return Err(throw(cx, name, "ein Nullvektor hat keine Richtung"));
            }
            a.iter().map(|x| x / l).collect()
        }
        "lerp" => {
            let (a, b) = (get(cx, 0, "a")?, get(cx, 1, "b")?);
            same(cx, &a, &b)?;
            let t = num(cx, name, args, 2, "t")?;
            a.iter().zip(&b).map(|(x, y)| x + (y - x) * t).collect()
        }
        _ => return Err(Fail::Unknown),
    };
    float_list(cx, &out)
}

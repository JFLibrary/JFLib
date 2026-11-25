// host -- the script's view of lib/geom: the JFScript host functions geom.*, profile.*, vec.*, model.* and the
// host objects of JFScript S2 (docs/HOST.md there).  The script only describes; every vertex is computed here in
// Rust.  A mesh is a host object of type "mesh" (Cx::new_object: identity, the script cannot open or change it,
// type(m) is "mesh", print shows "mesh(12 Dreiecke)"); its methods reach Host::method, m.tris Host::get.  Meshes
// never change, every operation makes a new one.  When the collector frees a mesh the script no longer holds,
// Host::drop frees its vertices and its convex cells and the id is used again; a part of the model has its own
// copy (model.add), so a drop never touches it.  The triangle limit of a run (ARENA_TRIANGLES) still counts every
// mesh made, intermediate ones included: it bounds the work of a run, and it does not depend on when the
// collector runs.  A vector is a value type "vec" (Cx::new_value: == compares the numbers exactly) with x, y, z,
// + and - between vectors, * and / with a number, the unary minus and methods; every function that takes a point
// or a vector takes a vec as well as a list [x, y, z] (or three numbers where it did), and the functions that
// give a point or a vector in 3D give a vec (vec.* with lists of two numbers still gives a list).
//
// One description, api(): the functions, the two types with their members and operators, as script::Api with
// TypeScript-like signatures (English names, parameters included: decision G9.3 of JFScript's docs/RICHTUNG.md;
// German texts).  From it come `modeltool --api`, the names compile_with knows, the language services' check of
// the script (run.rs: unknown members of a mesh or a vec as warnings), the most arguments of each function for
// the warning about extra ones, and the messages about members a type does not have.  A signature names the
// longest form (m.move(x, y, z)); the shorter ones (m.move(v)) are in its text.
//
// Wrong arguments throw a message that names the function and the parameter; the VM adds the line and column of
// the call.  What the host accepts but is very likely a mistake (arguments beyond the last parameter, a value for
// `closed` that is no bool, a second model.name / budget / uv_scale or material definition that replaces the
// first) is a warning at the call (cx.warn); the script goes on.  A mesh made by a Boolean operation keeps its
// convex cells (geom::Solid) beside it, moved, turned, scaled and mirrored with it, so it can go into the next
// Boolean operation; any other mesh goes in as one convex solid.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use geom::{Affine, Axis, Mesh, Solid, Vec3};
use script::api::{Api, GlobalKind, Sig, Type};
use script::{Cx, Fail, Host, Object, Operator, Position, Value, Vm};

/// the most triangles of all meshes a run makes, intermediate ones included (also those freed since)
pub const ARENA_TRIANGLES: usize = 4_000_000;

/// The script's interface: every function, value and type with its members, signatures in TypeScript's form and
/// what each does (German).  See the module comment for what is made from it.
pub fn api() -> Api {
    Api::new("modeltool")
        .doc("Modelle aus Regeln und Bausteinen: das Skript beschreibt, lib/geom rechnet. Längen in Metern, Winkel in Grad, Y oben, vorn ist +Z; jeder Grundkörper steht auf y = 0, mittig über dem Ursprung. Ein Mesh ändert sich nie: jede Operation gibt ein neues. Punkte und Vektoren: ein vec oder eine Liste [x, y, z]; Profile: Listen von Punkten [x, y].")
        .namespace("geom", "Grundkörper, Körper aus Profilen, Zusammenfügen und Boolesche Operationen")
        .func("geom.box(w: float, h: float, d: float): mesh", "Quader w x h x d (x, y, z) in Metern; steht auf y = 0, mittig über dem Ursprung")
        .func("geom.cylinder(r: float, h: float, segments: int = 32): mesh", "Zylinder um die Y-Achse auf y = 0; glatte Seiten, flache Deckel")
        .func("geom.cone(r: float, h: float, segments: int = 32, r_top: float = 0): mesh", "Kegel (mit r_top > 0 ein Kegelstumpf) um die Y-Achse auf y = 0")
        .func("geom.sphere(r: float, segments: int = 32, rings: int = segments / 2): mesh", "Kugel, liegt auf y = 0 (Mitte bei y = r)")
        .func("geom.torus(R: float, r: float, segments: int = 32, sides: int = 16): mesh", "Torus, liegt flach auf y = 0; R zur Mitte des Rings, r der Ring")
        .func("geom.stairs(w: float, h: float, d: float, steps: int): mesh", "Treppe w breit (x), h hoch, d tief (z); erste Stufe vorn (z = d/2), steigt nach hinten")
        .func("geom.arch(w: float, h: float, d: float, thickness: float, segments: int = 16): mesh", "Torbogen: zwei Pfeiler der Stärke thickness, Öffnung mit Halbkreis, darüber die Stärke")
        .func("geom.extrude(profile: list, depth: float): mesh", "Profil [[x, y], ...] (Ansicht von vorn) entlang Z, mittig um z = 0")
        .func("geom.prism(footprint: list, h: float): mesh", "Grundriss [[x, z], ...] (Ansicht von oben) von y = 0 um h nach oben")
        .func("geom.sweep(profile: list, path: list, closed: bool = false, smooth: float = 30): mesh", "Profil [[x, y], ...] entlang eines Pfads aus Punkten (vec oder [x, y, z]); Ecken auf Gehrung")
        .func("geom.lathe(profile: list, segments: int = 32, smooth: float = 30): mesh", "Drehkörper: Profil [[radius, y], ...] um die Y-Achse")
        .func("geom.merge(...meshes: mesh): mesh", "Meshes zu einem zusammenfügen (Teile bleiben getrennt, auch wenn sie sich durchdringen); auch geom.merge([m1, m2, ...])")
        .func("geom.union(...meshes: mesh): mesh", "Vereinigung zu einem geschlossenen Körper (konvexe Körper oder Ergebnisse Boolescher Operationen); auch mit einer Liste")
        .func("geom.difference(m: mesh, ...cuts: mesh): mesh", "m ohne die Abzüge (ein Fenster aus einer Wand, ein Loch durch eine Platte); auch geom.difference([m, cut1, ...])")
        .func("geom.intersection(...meshes: mesh): mesh", "was in allen liegt; auch mit einer Liste")
        .namespace("profile", "Profile: Listen von Punkten [x, y]")
        .func("profile.rect(w: float, h: float): list", "Rechteck um den Ursprung")
        .func("profile.circle(r: float, n: int = 32): list", "Vieleck mit n Ecken auf einem Kreis um den Ursprung")
        .func("profile.rounded(w: float, h: float, r: float, n: int = 4): list", "Rechteck mit abgerundeten Ecken (n Segmente je Ecke)")
        .func("profile.arc(r: float, start: float, end: float, n: int = 16): list", "Punkte eines Kreisbogens von start bis end (Grad), beide Enden dabei: ein offener Linienzug")
        .func("profile.move(p: list, dx: float, dy: float): list", "verschoben")
        .func("profile.scale(p: list, sx: float, sy: float = sx): list", "skaliert um den Ursprung")
        .func("profile.rotate(p: list, deg: float): list", "gedreht um den Ursprung")
        .func("profile.area(p: list): float", "Fläche (positiv gegen den Uhrzeigersinn)")
        .func("vec(x: float, y: float, z: float): vec", "ein Vektor oder Punkt (x, y, z); auch vec([x, y, z]); rechnet mit + - * /, liest v.x, v.y, v.z")
        .func("vec.add(a: vec, b: vec): vec", "a + b; nimmt auch Listen [x, y, z] (mit Listen aus 2 Zahlen gibt es eine Liste)")
        .func("vec.sub(a: vec, b: vec): vec", "a - b (wie vec.add)")
        .func("vec.mul(a: vec, s: float): vec", "a * s")
        .func("vec.dot(a: vec, b: vec): float", "Skalarprodukt")
        .func("vec.cross(a: vec, b: vec): vec", "Kreuzprodukt (3D)")
        .func("vec.len(a: vec): float", "Länge")
        .func("vec.dist(a: vec, b: vec): float", "Abstand")
        .func("vec.norm(a: vec): vec", "Einheitsvektor")
        .func("vec.lerp(a: vec, b: vec, t: float): vec", "a + (b - a) * t")
        .namespace("model", "das Modell: Teile, Materialien und die Regeln, die der Bericht prüft")
        .func("model.add(m: mesh, material = \"grau\", name: str = nil): int", "Teil des Modells; material: Name oder {color, roughness, metallic}; ohne name teil1, teil2, ...")
        .func("model.material(name: str, props: map): str", "Material festlegen: props {color: \"#rrggbb\" oder [r, g, b], roughness = 0.8, metallic = 0} (Farben in sRGB, 0 bis 1)")
        .func("model.name(text: str): nil", "Name des Modells")
        .func("model.budget(triangles: int): nil", "höchstens so viele Dreiecke (Prüfung im Bericht, mit der Zeile der Regel)")
        .func("model.expect_size(x: float, y: float, z: float, tolerance: float = 0.001): nil", "erwartete Maße der Box in Metern (Prüfung im Bericht, mit der Zeile der Regel); nil: diese Achse nicht prüfen; auch expect_size([x, y, z], tolerance) oder mit einem vec")
        .func("model.uv_scale(meters: float): nil", "Meter je UV-Einheit der Box-Projektion (ohne Aufruf 1)")
        .value("params: map", "die Parameter von --param NAME=WERT (Zahlen, true und false, sonst Text): params.get(\"b\", 0.8)")
        .typ(
            Type::new("mesh", "Ein Mesh: das Skript kann es nicht öffnen und nicht ändern, jede Methode gibt ein neues; == ist dasselbe Mesh; print zeigt mesh(12 Dreiecke).")
                .prop("readonly tris: int", "die Zahl der Dreiecke")
                .method("move(x: float, y: float, z: float): mesh", "verschoben; auch move(v) mit einem vec oder [x, y, z]")
                .method("place(x: float, y: float, z: float): mesh", "so verschoben, dass die Mitte der Unterseite der Box bei (x, y, z) liegt; auch place(v)")
                .method("center(): mesh", "so verschoben, dass die Mitte der Box im Ursprung liegt")
                .method("rotate_x(deg: float): mesh", "um die X-Achse gedreht (rechtshändig; Vielfache von 90 exakt)")
                .method("rotate_y(deg: float): mesh", "um die Y-Achse gedreht (rechtshändig; Vielfache von 90 exakt)")
                .method("rotate_z(deg: float): mesh", "um die Z-Achse gedreht (rechtshändig; Vielfache von 90 exakt)")
                .method("rotate(deg: float, ax: float, ay: float, az: float): mesh", "um die Achse (ax, ay, az) durch den Ursprung gedreht; auch rotate(deg, axis) mit einem vec oder [ax, ay, az]")
                .method("scale(sx: float, sy: float, sz: float): mesh", "skaliert um den Ursprung; scale(s) in allen Achsen gleich, auch scale(v) mit einem vec oder [sx, sy, sz]")
                .method("mirror(axis: str): mesh", "gespiegelt an der Ebene durch den Ursprung (\"x\": x -> -x; \"y\", \"z\"), Flächen bleiben außen")
                .method("flip(): mesh", "Flächen umgedreht (innen nach außen)")
                .method("weld(tolerance: float = 0): mesh", "Ecken an derselben Stelle zu einem Punkt verbunden (eine Hälfte und ihr Spiegelbild zu einer Fläche)")
                .method("array(count: int, dx: float, dy: float, dz: float): mesh", "count Kopien, Kopie i um i * (dx, dy, dz) verschoben; auch array(count, v)")
                .method("radial(count: int, step: float = 360 / count): mesh", "count Kopien um die Y-Achse, Kopie i um i * step Grad gedreht")
                .method("flat(): mesh", "flache Normalen (harte Kanten)")
                .method("smooth(deg: float = 30): mesh", "glatte Normalen über Kanten flacher als deg Grad")
                .method("bevel(width: float, segments: int = 1, axis: str = nil): mesh", "Kanten und Ecken abgeschrägt (segments 1) oder gerundet (Flächen je Viertelkreis), width = Radius; axis \"x\", \"y\" oder \"z\": nur die Kanten entlang dieser Achse; nur konvexe Körper")
                .method("bounds(): map", "{min, max, size, center} in Metern, jedes ein vec (nil bei einem leeren Mesh)")
                .method("area(): float", "Oberfläche in m²")
                .method("volume(): float", "Volumen in m³ (bei geschlossenen Meshes)")
                .method("check(): map", "{ok, closed, outward, open_edges, flipped_edges, degenerate, triangles}"),
        )
        .typ(
            Type::new("vec", "Ein Vektor oder Punkt (x, y, z) in Metern, unveränderlich: + und - zwischen Vektoren, * und / mit einer Zahl, -v; == vergleicht die Zahlen genau. Statt v[0] heißt es v.x. Wo ein Punkt oder Vektor verlangt ist, geht ein vec wie eine Liste [x, y, z].")
                .value()
                .prop("readonly x: float", "die x-Komponente (nach rechts)")
                .prop("readonly y: float", "die y-Komponente (nach oben)")
                .prop("readonly z: float", "die z-Komponente (nach vorn)")
                .method("length(): float", "die Länge")
                .method("dot(other: vec): float", "das Skalarprodukt")
                .method("cross(other: vec): vec", "das Kreuzprodukt")
                .method("normalized(): vec", "der Einheitsvektor in derselben Richtung (nicht für den Nullvektor)")
                .method("distance(other: vec): float", "der Abstand der beiden Punkte")
                .method("lerp(other: vec, t: float): vec", "v + (other - v) * t")
                .op("vec + vec: vec", "die Summe")
                .op("vec - vec: vec", "die Differenz")
                .op("vec * float: vec", "gestreckt")
                .op("float * vec: vec", "gestreckt")
                .op("vec / float: vec", "geteilt")
                .op("-vec: vec", "umgekehrt"),
        )
}

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
    /// where the script states it (the file as errors name it)
    pub at: Position,
}

/// what the script built
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub parts: Vec<Part>,
    /// the materials the parts use, in the order of first use
    pub materials: Vec<MaterialDef>,
    pub budget: Option<u64>,
    /// where the script set the budget (its last model.budget)
    pub budget_at: Position,
    pub expects: Vec<Expect>,
    /// metres per UV unit (1)
    pub uv_scale: f64,
}

/// the host of one run
pub struct ModelHost<'a> {
    api: &'a Api,
    /// the meshes by id (a freed one is empty); live[id]: the script holds the mesh of that id
    meshes: Vec<Mesh>,
    live: Vec<bool>,
    /// the convex cells of a mesh made by a Boolean operation (moved with it), by mesh id
    solids: Vec<Option<Rc<Solid>>>,
    /// the ids Host::drop gave back, used again last in, first out
    free: Vec<usize>,
    /// the triangles of every mesh made so far (ARENA_TRIANGLES)
    arena: usize,
    /// how many meshes the run made, and the most that lived at once
    pub made: u64,
    pub peak: usize,
    alive: usize,
    /// the materials of model.material with where they were set
    defined: Vec<(MaterialDef, Position)>,
    /// where model.name and model.uv_scale were called last (for the warning when a second call replaces them)
    name_at: Option<Position>,
    uv_at: Option<Position>,
    pub model: Model,
    loader: &'a mut dyn FnMut(&str) -> Result<String, String>,
    /// (path, source) of every module the script imported, in order
    pub imports: Vec<(String, String)>,
    /// microseconds spent in host functions and hooks (the geometry)
    pub host_us: u64,
}

impl<'a> ModelHost<'a> {
    pub fn new(api: &'a Api, loader: &'a mut dyn FnMut(&str) -> Result<String, String>) -> ModelHost<'a> {
        ModelHost {
            api,
            meshes: Vec::new(),
            live: Vec::new(),
            solids: Vec::new(),
            free: Vec::new(),
            arena: 0,
            made: 0,
            peak: 0,
            alive: 0,
            defined: Vec::new(),
            name_at: None,
            uv_at: None,
            model: Model { uv_scale: 1.0, ..Model::default() },
            loader,
            imports: Vec::new(),
            host_us: 0,
        }
    }

    /// f with the time it takes counted as geometry
    fn timed<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let t0 = plat::time_us();
        let r = f(self);
        self.host_us += plat::time_us().saturating_sub(t0);
        r
    }
}

impl Host for ModelHost<'_> {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        self.timed(|h| {
            let r = h.dispatch(cx, name, args);
            if r.is_ok() {
                let sig = match h.api.global(name).map(|g| &g.kind) {
                    Some(GlobalKind::Func(s)) => Some(s),
                    _ => None,
                };
                extra_arguments(cx, sig, name, args.len());
            }
            r
        })
    }

    fn load(&mut self, cx: &mut Cx, path: &str) -> Result<String, String> {
        let src = (self.loader)(path)?;
        cx.charge(1 + src.len() as u64 / 64);
        self.imports.push((String::from(path), src.clone()));
        Ok(src)
    }

    // m.tris, v.x
    fn get(&mut self, cx: &mut Cx, obj: Object, name: &str) -> Result<Value, Fail> {
        match (obj.ty, name) {
            ("mesh", "tris") => Ok(Value::Int(self.meshes.get(obj.id as usize).map_or(0, |m| m.triangle_count()) as i64)),
            ("vec", "x" | "y" | "z") => {
                let i = match name {
                    "x" => 0,
                    "y" => 1,
                    _ => 2,
                };
                Ok(cx.data(obj.value).and_then(|d| d.get(i).copied()).unwrap_or(Value::Nil))
            }
            ("mesh" | "vec", _) => Err(self.no_member(cx, obj.ty, name, false)),
            _ => Err(Fail::Unknown),
        }
    }

    // nothing of a mesh or a vec changes
    fn set(&mut self, cx: &mut Cx, obj: Object, name: &str, _value: Value) -> Result<(), Fail> {
        let Some(t) = self.api.get_type(obj.ty) else { return Err(Fail::Unknown) };
        if t.find_prop(name).is_none() {
            return Err(self.no_member(cx, obj.ty, name, false));
        }
        let new = if obj.ty == "vec" { "; einen neuen macht vec(x, y, z)" } else { "; jede Methode gibt ein neues" };
        Err(cx.error(&format!("{}.{} ist nur lesbar: ein {} ändert sich nie{}", obj.ty, name, obj.ty, new)))
    }

    fn method(&mut self, cx: &mut Cx, obj: Object, name: &str, args: &[Value]) -> Result<Value, Fail> {
        self.timed(|h| {
            let r = match obj.ty {
                "mesh" => h.mesh_method(cx, obj.id as usize, name, args),
                "vec" => h.vec_method(cx, obj.value, name, args),
                _ => Err(Fail::Unknown),
            };
            if r.is_ok() {
                let sig = h.api.get_type(obj.ty).and_then(|t| t.find_method(name)).map(|m| &m.sig);
                extra_arguments(cx, sig, &format!("{}()", name), args.len());
            }
            r
        })
    }

    // the operators of vec: + - between vectors, * / with a number (also 2 * v), the minus
    fn op(&mut self, cx: &mut Cx, op: Operator, a: Value, b: Value) -> Result<Value, Fail> {
        self.timed(|_| {
            let (va, vb) = (vec_data(cx, a), vec_data(cx, b));
            let (sa, sb) = (number_of(a), number_of(b));
            let r: [f64; 3] = match (op, va, vb) {
                (Operator::Add, Some(p), Some(q)) => add3(&p, &q),
                (Operator::Sub, Some(p), Some(q)) => sub3(&p, &q),
                (Operator::Mul, Some(p), None) => match sb {
                    Some(s) => mul3(&p, s),
                    None => return Err(Fail::Unknown),
                },
                // (s * x is x * s, bit for bit)
                (Operator::Mul, None, Some(q)) => match sa {
                    Some(s) => mul3(&q, s),
                    None => return Err(Fail::Unknown),
                },
                (Operator::Div, Some(p), None) => match sb {
                    Some(s) => [p[0] / s, p[1] / s, p[2] / s],
                    None => return Err(Fail::Unknown),
                },
                (Operator::Neg, Some(p), None) => [-p[0], -p[1], -p[2]],
                _ => return Err(Fail::Unknown),
            };
            if !r.iter().all(|x| x.is_finite()) {
                let what = if op == Operator::Neg { String::from("-vec") } else { format!("{} {} {}", cx.type_of(a), op.text(), cx.type_of(b)) };
                return Err(cx.error(&format!("{}: das Ergebnis ist keine endliche Zahl (geteilt durch 0 oder zu groß)", what)));
            }
            new_vec(cx, &r)
        })
    }

    fn to_str(&self, _vm: &Vm, obj: Object) -> Option<String> {
        (obj.ty == "mesh").then(|| format!("mesh({} Dreiecke)", self.meshes.get(obj.id as usize).map_or(0, |m| m.triangle_count())))
    }

    // the script holds this mesh no more: its vertices and cells go, the id is used again (a part of the model has
    // its own copy)
    fn drop(&mut self, obj: Object) {
        let id = obj.id as usize;
        if obj.ty == "mesh" && self.live.get(id) == Some(&true) {
            self.meshes[id] = Mesh::default();
            self.solids[id] = None;
            self.live[id] = false;
            self.alive -= 1;
            self.free.push(id);
        }
    }
}

/// `modeltool --api`: the description as text, the functions in their order (a blank line between namespaces),
/// then each type with its members (a mesh's as m.NAME, a vec's as v.NAME) and operators; then the materials
pub fn api_text() -> String {
    let api = api();
    let mut s = format!("{}\n", api.doc);
    let mut root = "";
    for g in &api.globals {
        let r = g.name.split('.').next().unwrap_or(&g.name);
        if r != root {
            s.push('\n');
            root = r;
        }
        match &g.kind {
            GlobalKind::Func(sig) => s.push_str(&format!("{}\n    {}\n", sig.label(&g.name), g.doc)),
            GlobalKind::Value(t) => s.push_str(&format!("{}: {}\n    {}\n", g.name, t, g.doc)),
            GlobalKind::Namespace => s.push_str(&format!("{}.*: {}\n", g.name, g.doc)),
        }
    }
    for t in &api.types {
        let kind = if t.value { "Werttyp: == vergleicht die Werte" } else { "Objekt: == ist dasselbe" };
        s.push_str(&format!("\n{} ({})\n    {}\n", t.name, kind, t.doc));
        let me = &t.name[..1];
        for p in &t.props {
            let ro = if p.readonly { ", nur lesbar" } else { "" };
            s.push_str(&format!("{}.{}: {}{}\n    {}\n", me, p.name, p.ty, ro, p.doc));
        }
        for m in &t.methods {
            s.push_str(&format!("{}.{}\n    {}\n", me, m.sig.label(&m.name), m.doc));
        }
        for o in &t.ops {
            s.push_str(&format!("{}\n    {}\n", o.label(), o.doc));
        }
    }
    s.push_str("\nMaterialien: ");
    s.push_str(&PALETTE.iter().map(|p| p.0).collect::<Vec<_>>().join(", "));
    s.push('\n');
    s
}

/// "haus.jfs:71" (the line alone when the file is not known)
pub fn place(at: &Position) -> String {
    if at.file.is_empty() { format!("Zeile {}", at.line) } else { format!("{}:{}", at.file, at.line) }
}

/// The most arguments a function or method of this signature reads (its parameters); None for one that takes any
/// number (a rest parameter: merge and the Boolean operations).
pub fn max_args(sig: &Sig) -> Option<usize> {
    if sig.params.iter().any(|p| p.rest) { None } else { Some(sig.params.len()) }
}

/// A warning when a call that worked got more arguments than the function reads: the others were never looked at
/// (`geom.cylinder(0.3, 1.2, 32, 0.1)` with a radius for the top meant for geom.cone).  `shown_as` is how messages
/// name it ("geom.box", "move()").
fn extra_arguments(cx: &mut Cx, sig: Option<&Sig>, shown_as: &str, given: usize) {
    let Some(sig) = sig else { return };
    let Some(max) = max_args(sig) else { return };
    if given <= max {
        return;
    }
    let params = sig.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
    let count = if given == 1 { String::from("1 Argument") } else { format!("{} Argumente", given) };
    let takes = if max == 0 { String::from("nimmt aber keine") } else { format!("nimmt aber höchstens {} ({})", max, params) };
    let rest = match given - max {
        1 if given == 1 => "es bleibt unbeachtet",
        1 => "das letzte bleibt unbeachtet",
        _ => "die übrigen bleiben unbeachtet",
    };
    cx.warn(&format!("{}: {}, {}; {}", shown_as, count, takes, rest));
}

// ---------------------------------------------------------------- arguments

/// a value for a message: as print shows it, at most 40 characters, with its type (a mesh as "mesh": its id says
/// nothing)
fn shown(cx: &Cx, v: Value) -> String {
    if let Some(o) = cx.object(v) {
        if o.ty == "mesh" {
            return String::from("mesh");
        }
    }
    let mut s = cx.display(v);
    if s.chars().count() > 40 {
        s = s.chars().take(37).collect::<String>() + "...";
    }
    match v {
        Value::Str(_) => format!("str \"{}\"", s),
        Value::Nil => String::from("nil"),
        // (a value type's text starts with its name: vec(1.0, 2.0, 3.0))
        Value::Host(_) => s,
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

/// the three numbers of a vec (None for any other value)
fn vec_data(cx: &Cx, v: Value) -> Option<[f64; 3]> {
    if cx.object(v)?.ty != "vec" {
        return None;
    }
    let d = cx.data(v)?;
    Some([number_of(*d.first()?)?, number_of(*d.get(1)?)?, number_of(*d.get(2)?)?])
}

/// a new vec; its numbers are finite (so it can go wherever a point goes, as a list of numbers could)
fn new_vec(cx: &mut Cx, v: &[f64; 3]) -> Result<Value, Fail> {
    if !v.iter().all(|x| x.is_finite()) {
        return Err(cx.error("vec: das Ergebnis ist keine endliche Zahl (zu groß)"));
    }
    cx.new_value("vec", &[Value::Float(v[0]), Value::Float(v[1]), Value::Float(v[2])])
}

/// a point or vector in one value: a vec or a list of three numbers
fn point_of(cx: &Cx, v: Value) -> Option<Vec3> {
    if let Some(p) = vec_data(cx, v) {
        return Some(Vec3::new(p[0], p[1], p[2]));
    }
    numbers_of(cx, v, 3).map(|q| Vec3::new(q[0], q[1], q[2]))
}

/// a vector from argument i: a vec or a list [x, y, z] there, or three numbers at i, i + 1, i + 2
fn vec3(cx: &mut Cx, f: &str, args: &[Value], i: usize, what: &str) -> Result<Vec3, Fail> {
    if let Some(p) = point_of(cx, arg(args, i)) {
        return Ok(p);
    }
    if matches!(arg(args, i), Value::List(_)) {
        let s = shown(cx, arg(args, i));
        return Err(throw(cx, f, &format!("{} muss ein vec oder eine Liste aus 3 Zahlen sein, nicht {}", what, s)));
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

/// a list of 3D points, each a vec or [x, y, z]
fn points3(cx: &mut Cx, f: &str, v: Value, what: &str) -> Result<Vec<Vec3>, Fail> {
    let items: Option<Vec<Value>> = cx.list(v).map(|l| l.to_vec());
    let Some(items) = items else {
        let s = shown(cx, v);
        return Err(throw(cx, f, &format!("{} muss eine Liste von Punkten (vec oder [x, y, z]) sein, nicht {}", what, s)));
    };
    cx.spend(1 + items.len() as u64 / 8)?;
    let mut out = Vec::with_capacity(items.len());
    for (k, &p) in items.iter().enumerate() {
        match point_of(cx, p) {
            Some(q) => out.push(q),
            None => {
                let s = shown(cx, p);
                return Err(throw(cx, f, &format!("{}[{}] muss ein Punkt sein (ein vec oder [x, y, z] aus drei Zahlen), nicht {}", what, k, s)));
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

// ---------------------------------------------------------------- vector arithmetic
// (vec.*, the methods and the operators share these: the same numbers, bit for bit, as vec.* gave on lists)

fn add3(a: &[f64], b: &[f64]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: &[f64], b: &[f64]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul3(a: &[f64], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// the sum of a[i] * b[i] from 0.0
fn dot_n(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).fold(0.0, |s, (x, y)| s + x * y)
}

fn len_n(a: &[f64]) -> f64 {
    geom::math::sqrt(a.iter().fold(0.0, |s, x| s + x * x))
}

fn dist_n(a: &[f64], b: &[f64]) -> f64 {
    geom::math::sqrt(a.iter().zip(b).fold(0.0, |s, (x, y)| s + (x - y) * (x - y)))
}

fn lerp_n(a: &[f64], b: &[f64], t: f64) -> Vec<f64> {
    a.iter().zip(b).map(|(x, y)| x + (y - x) * t).collect()
}

fn cross3(a: &[f64], b: &[f64]) -> [f64; 3] {
    let c = Vec3::new(a[0], a[1], a[2]).cross(Vec3::new(b[0], b[1], b[2]));
    [c.x, c.y, c.z]
}

/// a / |a|; None for a vector without a direction
fn norm_n(a: &[f64]) -> Option<Vec<f64>> {
    let l = len_n(a);
    if l > 0.0 { Some(a.iter().map(|x| x / l).collect()) } else { None }
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
                "union" | "difference" | "intersection" => self.boolean(cx, name, f, args),
                _ => Err(Fail::Unknown),
            },
            "profile" => self.profile(cx, name, f, args),
            "vec" => vector(cx, name, f, args),
            "model" => self.model_fn(cx, name, f, args),
            "" if f == "vec" => {
                // vec(x, y, z) or vec(p) of a vec or a list [x, y, z]
                let v = if args.len() == 1 {
                    match point_of(cx, args[0]) {
                        Some(p) => p,
                        None => {
                            let s = shown(cx, args[0]);
                            return Err(throw(cx, name, &format!("der Punkt muss eine Liste aus 3 Zahlen oder ein vec sein, nicht {}", s)));
                        }
                    }
                } else {
                    Vec3::new(num(cx, name, args, 0, "x")?, num(cx, name, args, 1, "y")?, num(cx, name, args, 2, "z")?)
                };
                new_vec(cx, &[v.x, v.y, v.z])
            }
            _ => Err(Fail::Unknown),
        }
    }

    /// "mesh hat keine Methode spin (es gibt: ...)", or the hint that it is a property (a method) of the type
    fn no_member(&self, cx: &mut Cx, ty: &str, name: &str, call: bool) -> Fail {
        let Some(t) = self.api.get_type(ty) else { return Fail::Unknown };
        let msg = if call && t.find_prop(name).is_some() {
            format!("{} ist eine Eigenschaft von {}, keine Methode – ohne Klammern: .{}", name, ty, name)
        } else if !call && t.find_method(name).is_some() {
            format!("{} ist eine Methode von {}, keine Eigenschaft – mit Klammern: {}(..)", name, ty, name)
        } else if call {
            format!("{} hat keine Methode {} (es gibt: {})", ty, name, t.methods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "))
        } else {
            format!("{} hat keine Eigenschaft {} (es gibt: {})", ty, name, t.props.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "))
        };
        cx.error(&msg)
    }

    /// the id of a mesh the script holds
    fn mesh_id(&self, cx: &Cx, v: Value) -> Option<usize> {
        let o = cx.object(v)?;
        let id = o.id as usize;
        (o.ty == "mesh" && self.live.get(id) == Some(&true)).then_some(id)
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

    /// keep a mesh and give the script its object
    fn handle(&mut self, cx: &mut Cx, f: &str, m: Mesh) -> Result<Value, Fail> {
        self.handle_with(cx, f, m, None)
    }

    /// keep a mesh with its convex cells
    fn handle_with(&mut self, cx: &mut Cx, f: &str, m: Mesh, solid: Option<Rc<Solid>>) -> Result<Value, Fail> {
        let tris = m.triangle_count();
        if self.arena + tris > ARENA_TRIANGLES {
            return Err(throw(cx, f, &format!(
                "zusammen mehr als {} Dreiecke in allen Meshes dieses Laufs (Zwischenstände mitgezählt): weniger Segmente oder Kopien",
                ARENA_TRIANGLES
            )));
        }
        cx.spend(1 + tris as u64 / 64)?;
        self.arena += tris;
        let id = match self.free.pop() {
            Some(i) => {
                self.meshes[i] = m;
                self.solids[i] = solid;
                self.live[i] = true;
                i
            }
            None => {
                self.meshes.push(m);
                self.solids.push(solid);
                self.live.push(true);
                self.meshes.len() - 1
            }
        };
        self.made += 1;
        self.alive += 1;
        self.peak = self.peak.max(self.alive);
        cx.new_object("mesh", id as u64)
    }

    fn construct(&mut self, cx: &mut Cx, name: &str, f: &str, args: &[Value]) -> Result<Value, Fail> {
        let r = match f {
            "box" => {
                let (w, h, d) = (num(cx, name, args, 0, "w")?, num(cx, name, args, 1, "h")?, num(cx, name, args, 2, "d")?);
                geom::cuboid(w, h, d)
            }
            "cylinder" => {
                let (r, h) = (num(cx, name, args, 0, "r")?, num(cx, name, args, 1, "h")?);
                let n = count_or(cx, name, args, 2, Some(32), "segments")?;
                geom::cylinder(r, h, n)
            }
            "cone" => {
                let (r, h) = (num(cx, name, args, 0, "r")?, num(cx, name, args, 1, "h")?);
                let n = count_or(cx, name, args, 2, Some(32), "segments")?;
                let top = num_or(cx, name, args, 3, 0.0, "r_top")?;
                geom::cone(r, h, n, top)
            }
            "sphere" => {
                let r = num(cx, name, args, 0, "r")?;
                let n = count_or(cx, name, args, 1, Some(32), "segments")?;
                let stacks = count_or(cx, name, args, 2, Some((n / 2).max(2)), "rings")?;
                geom::sphere(r, n, stacks)
            }
            "torus" => {
                let (big, small) = (num(cx, name, args, 0, "R")?, num(cx, name, args, 1, "r")?);
                let n = count_or(cx, name, args, 2, Some(32), "segments")?;
                let sides = count_or(cx, name, args, 3, Some(16), "sides")?;
                geom::torus(big, small, n, sides)
            }
            "stairs" => {
                let (w, h, d) = (num(cx, name, args, 0, "w")?, num(cx, name, args, 1, "h")?, num(cx, name, args, 2, "d")?);
                let steps = count_or(cx, name, args, 3, None, "steps")?;
                geom::stairs(w, h, d, steps)
            }
            "arch" => {
                let (w, h, d) = (num(cx, name, args, 0, "w")?, num(cx, name, args, 1, "h")?, num(cx, name, args, 2, "d")?);
                let t = num(cx, name, args, 3, "thickness")?;
                let n = count_or(cx, name, args, 4, Some(16), "segments")?;
                geom::arch(w, h, d, t, n)
            }
            "extrude" => {
                let p = points2(cx, name, arg(args, 0), "profile")?;
                let d = num(cx, name, args, 1, "depth")?;
                geom::extrude(&p, d)
            }
            "prism" => {
                let p = points2(cx, name, arg(args, 0), "footprint")?;
                let h = num(cx, name, args, 1, "h")?;
                geom::prism(&p, h)
            }
            "sweep" => {
                let p = points2(cx, name, arg(args, 0), "profile")?;
                let path = points3(cx, name, arg(args, 1), "path")?;
                // any value is true but nil and false: a number here is very likely the smoothing angle one place
                // too early (geom.sweep(p, path, 30)), and 0 does not mean false
                let closed_v = arg(args, 2);
                let closed = closed_v.truthy();
                if !matches!(closed_v, Value::Nil | Value::Bool(_)) {
                    let s = shown(cx, closed_v);
                    cx.warn(&format!("{}: closed sollte true oder false sein, nicht {} (gilt als {})", name, s, closed));
                }
                let smooth = num_or(cx, name, args, 3, geom::DEFAULT_SMOOTH_DEG, "smooth")?;
                geom::sweep(&p, &path, closed, smooth)
            }
            _ => {
                let p = points2(cx, name, arg(args, 0), "profile")?;
                let n = count_or(cx, name, args, 1, Some(32), "segments")?;
                let smooth = num_or(cx, name, args, 2, geom::DEFAULT_SMOOTH_DEG, "smooth")?;
                geom::lathe(&p, n, smooth)
            }
        };
        match r {
            Ok(m) => self.handle(cx, name, m),
            Err(e) => Err(throw(cx, name, e.message())),
        }
    }

    /// the meshes of merge and the Boolean operations: the arguments, or one list of them
    fn mesh_items(&self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Vec<usize>, Fail> {
        let items: Vec<Value> = match (args.len(), cx.list(arg(args, 0)).map(|l| l.to_vec())) {
            (1, Some(l)) => l,
            _ => args.to_vec(),
        };
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
        Ok(ids)
    }

    fn merge(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        // geom.merge(a, b, ...) or geom.merge([a, b, ...])
        let ids = self.mesh_items(cx, name, args)?;
        if ids.is_empty() {
            return Err(throw(cx, name, "braucht mindestens ein Mesh"));
        }
        let refs: Vec<&Mesh> = ids.iter().map(|&i| &self.meshes[i]).collect();
        match Mesh::merge(&refs) {
            Ok(m) => self.handle(cx, name, m),
            Err(e) => Err(throw(cx, name, e.message())),
        }
    }

    /// the solid of mesh `id`: its cells when a Boolean operation made it, else the mesh as one convex solid
    fn solid_of(&self, cx: &mut Cx, f: &str, id: usize, what: &str) -> Result<Rc<Solid>, Fail> {
        if let Some(s) = &self.solids[id] {
            return Ok(s.clone());
        }
        cx.spend(1 + self.meshes[id].triangle_count() as u64 / 16)?;
        match Solid::from_convex(&self.meshes[id]) {
            Ok(s) => Ok(Rc::new(s)),
            Err(e) => Err(throw(cx, f, &format!("{}: {}", what, e.message()))),
        }
    }

    /// geom.union / difference / intersection of meshes (or one list of them)
    fn boolean(&mut self, cx: &mut Cx, name: &str, f: &str, args: &[Value]) -> Result<Value, Fail> {
        let items = match (args.len(), cx.list(arg(args, 0))) {
            (1, Some(l)) => l.len(),
            _ => args.len(),
        };
        let least = if f == "difference" { 2 } else { 1 };
        if items < least {
            return Err(throw(cx, name, &format!("braucht mindestens {} Meshes", least)));
        }
        let ids = self.mesh_items(cx, name, args)?;
        let mut acc = (*self.solid_of(cx, name, ids[0], "Teil 1")?).clone();
        for (k, &id) in ids.iter().enumerate().skip(1) {
            let o = self.solid_of(cx, name, id, &format!("Teil {}", k + 1))?;
            let r = match f {
                "union" => acc.union(&o),
                "difference" => acc.difference(&o),
                _ => acc.intersection(&o),
            };
            acc = match r {
                Ok(s) => s,
                Err(e) => return Err(throw(cx, name, e.message())),
            };
            cx.spend(1 + acc.cell_count() as u64 * 4)?;
        }
        if acc.is_empty() {
            return Err(throw(cx, name, "das Ergebnis ist leer (nichts übrig)"));
        }
        match acc.to_mesh(geom::DEFAULT_SMOOTH_DEG) {
            Ok(m) => self.handle_with(cx, name, m, Some(Rc::new(acc))),
            Err(e) => Err(throw(cx, name, e.message())),
        }
    }

    /// m.NAME(rest): a method of the mesh `id`
    fn mesh_method(&mut self, cx: &mut Cx, id: usize, f: &str, rest: &[Value]) -> Result<Value, Fail> {
        if self.live.get(id) != Some(&true) {
            return Err(Fail::Unknown);
        }
        let name = format!("{}()", f);
        let name = name.as_str();
        let m = &self.meshes[id];
        // the maps of move, place, center, rotations, scale and mirror (the solid goes along)
        let map: Option<Affine> = match f {
            "move" => Some(Affine::translation(vec3(cx, name, rest, 0, "die Verschiebung")?)),
            "place" => {
                let v = vec3(cx, name, rest, 0, "der Punkt")?;
                let Some(b) = m.bounds() else { return Err(throw(cx, name, "das Mesh ist leer")) };
                let c = b.center();
                Some(Affine::translation(Vec3::new(v.x - c.x, v.y - b.min.y, v.z - c.z)))
            }
            "center" => {
                let Some(b) = m.bounds() else { return Err(throw(cx, name, "das Mesh ist leer")) };
                Some(Affine::translation(-b.center()))
            }
            "rotate_x" | "rotate_y" | "rotate_z" => {
                let deg = num(cx, name, rest, 0, "deg")?;
                let axis = match f {
                    "rotate_x" => Axis::X,
                    "rotate_y" => Axis::Y,
                    _ => Axis::Z,
                };
                Some(Affine::rotation(axis, deg))
            }
            "rotate" => {
                let deg = num(cx, name, rest, 0, "deg")?;
                let axis = vec3(cx, name, rest, 1, "die Achse")?;
                match Affine::rotation_about(axis, deg) {
                    Some(a) => Some(a),
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
                Some(Affine::scale(s))
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
                Some(Affine::mirror(axis))
            }
            _ => None,
        };
        if let Some(a) = map {
            let out = self.meshes[id].transformed(&a);
            let solid = self.solids[id].as_ref().map(|s| Rc::new(s.transformed(&a)));
            return self.handle_with(cx, name, out, solid);
        }
        let m = &self.meshes[id];
        // the same geometry (normals only): the cells stay
        let keep = matches!(f, "flat" | "smooth" | "weld");
        let out = match f {
            "flip" => m.flipped(),
            "weld" => {
                let tol = num_or(cx, name, rest, 0, 0.0, "tolerance")?;
                if tol < 0.0 {
                    return Err(throw(cx, name, "die Toleranz darf nicht negativ sein"));
                }
                self.meshes[id].weld(tol)
            }
            "array" => {
                let n = count_or(cx, name, rest, 0, None, "count")?;
                let step = vec3(cx, name, rest, 1, "der Abstand")?;
                match self.meshes[id].array(n, step) {
                    Ok(x) => x,
                    Err(e) => return Err(throw(cx, name, e.message())),
                }
            }
            "radial" => {
                let n = count_or(cx, name, rest, 0, None, "count")?;
                let step = num_or(cx, name, rest, 1, 360.0 / n.max(1) as f64, "step")?;
                match self.meshes[id].radial(n, step) {
                    Ok(x) => x,
                    Err(e) => return Err(throw(cx, name, e.message())),
                }
            }
            "flat" => m.flat(),
            "smooth" => {
                let deg = num_or(cx, name, rest, 0, geom::DEFAULT_SMOOTH_DEG, "deg")?;
                self.meshes[id].smooth(deg)
            }
            "bevel" => {
                let r = num(cx, name, rest, 0, "width")?;
                let k = count_or(cx, name, rest, 1, Some(1), "segments")?;
                let only = match arg(rest, 2) {
                    Value::Nil => None,
                    v => match cx.str(v) {
                        Some("x") | Some("X") => Some(Axis::X),
                        Some("y") | Some("Y") => Some(Axis::Y),
                        Some("z") | Some("Z") => Some(Axis::Z),
                        _ => {
                            let s = shown(cx, v);
                            return Err(throw(cx, name, &format!("axis muss \"x\", \"y\", \"z\" oder nil sein, nicht {}", s)));
                        }
                    },
                };
                let solid = self.solid_of(cx, name, id, "das Mesh")?;
                cx.spend(1 + (k * k) as u64 * solid.plane_count() as u64 / 4)?;
                return match solid.bevel(r, k, only) {
                    Ok((s, m)) => self.handle_with(cx, name, m, Some(Rc::new(s))),
                    Err(e) => Err(throw(cx, name, e.message())),
                };
            }
            "bounds" => {
                let Some(b) = m.bounds() else { return Ok(Value::Nil) };
                let (size, c) = (b.size(), b.center());
                let lo = new_vec(cx, &[b.min.x, b.min.y, b.min.z])?;
                let hi = new_vec(cx, &[b.max.x, b.max.y, b.max.z])?;
                let size = new_vec(cx, &[size.x, size.y, size.z])?;
                let c = new_vec(cx, &[c.x, c.y, c.z])?;
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
            _ => return Err(self.no_member(cx, "mesh", f, true)),
        };
        let solid = if keep { self.solids[id].clone() } else { None };
        self.handle_with(cx, name, out, solid)
    }

    /// v.NAME(args): a method of a vec
    fn vec_method(&mut self, cx: &mut Cx, v: Value, f: &str, args: &[Value]) -> Result<Value, Fail> {
        let Some(a) = vec_data(cx, v) else { return Err(Fail::Unknown) };
        let name = format!("{}()", f);
        let other = |cx: &mut Cx| -> Result<[f64; 3], Fail> {
            match point_of(cx, arg(args, 0)) {
                Some(p) => Ok([p.x, p.y, p.z]),
                None => {
                    let s = shown(cx, arg(args, 0));
                    Err(throw(cx, &name, &format!("other muss ein vec oder eine Liste aus 3 Zahlen sein, nicht {}", s)))
                }
            }
        };
        match f {
            "length" => Ok(Value::Float(len_n(&a))),
            "dot" => {
                let b = other(cx)?;
                Ok(Value::Float(dot_n(&a, &b)))
            }
            "cross" => {
                let b = other(cx)?;
                new_vec(cx, &cross3(&a, &b))
            }
            "normalized" => match norm_n(&a) {
                Some(n) => new_vec(cx, &[n[0], n[1], n[2]]),
                None => Err(throw(cx, &name, "ein Nullvektor hat keine Richtung")),
            },
            "distance" => {
                let b = other(cx)?;
                Ok(Value::Float(dist_n(&a, &b)))
            }
            "lerp" => {
                let b = other(cx)?;
                let t = num(cx, &name, args, 1, "t")?;
                let r = lerp_n(&a, &b, t);
                new_vec(cx, &[r[0], r[1], r[2]])
            }
            _ => Err(self.no_member(cx, "vec", f, true)),
        }
    }

    fn profile(&mut self, cx: &mut Cx, name: &str, f: &str, args: &[Value]) -> Result<Value, Fail> {
        let pts: Vec<[f64; 2]> = match f {
            "rect" => {
                let (w, h) = (num(cx, name, args, 0, "w")?, num(cx, name, args, 1, "h")?);
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
                let (w, h, r) = (num(cx, name, args, 0, "w")?, num(cx, name, args, 1, "h")?, num(cx, name, args, 2, "r")?);
                let n = count_or(cx, name, args, 3, Some(4), "n")?;
                if !(1..=256).contains(&n) {
                    return Err(throw(cx, name, "n muss zwischen 1 und 256 liegen"));
                }
                geom::poly::rounded_rect(w, h, r, n)
            }
            "arc" => {
                let r = num(cx, name, args, 0, "r")?;
                let (from, to) = (num(cx, name, args, 1, "start")?, num(cx, name, args, 2, "end")?);
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
                let (s, c) = geom::math::sin_cos_deg(num(cx, name, args, 1, "deg")?);
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
        if let Some((m, _)) = self.defined.iter().find(|(m, _)| m.name == n) {
            return Ok(m.clone());
        }
        if let Some(&(name, color, roughness, metallic)) = PALETTE.iter().find(|p| p.0 == n) {
            return Ok(MaterialDef { name: String::from(name), color, roughness, metallic });
        }
        let mut names: Vec<&str> = self.defined.iter().map(|(m, _)| m.name.as_str()).collect();
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
                    v => cx.display_with(v, &*self),
                };
                if self.meshes[id].triangle_count() == 0 {
                    return Err(throw(cx, name, "das Mesh hat keine Dreiecke"));
                }
                // (a copy: the part stays when the script drops the mesh)
                self.model.parts.push(Part { name: part_name, mesh: self.meshes[id].clone(), material });
                Ok(Value::Int(self.model.parts.len() as i64))
            }
            "material" => {
                let Some(n) = cx.str(arg(args, 0)).map(String::from) else {
                    let s = shown(cx, arg(args, 0));
                    return Err(throw(cx, name, &format!("name muss ein str sein, nicht {}", s)));
                };
                let def = material_from_map(cx, name, &n, arg(args, 1))?;
                // a second definition with other values: the first is lost, or the parts added in between keep it
                // (two materials of one name); redefining a built-in one that no part uses yet is what it is for
                if let Some((old, at)) = self.defined.iter().find(|(m, _)| m.name == n) {
                    if *old != def {
                        let msg = format!("{}: das Material \"{}\" ist schon in {} anders festgelegt; ab hier gilt das neue, Teile davor behalten das alte", name, n, place(at));
                        cx.warn(&msg);
                    }
                } else if self.model.materials.iter().any(|m| m.name == n && *m != def) {
                    cx.warn(&format!("{}: das Material \"{}\" haben schon Teile mit anderen Werten; sie behalten sie, das neue gilt erst für Teile danach", name, n));
                }
                let here = cx.position();
                self.defined.retain(|(m, _)| m.name != n);
                self.defined.push((def, here));
                cx.new_str(&n)
            }
            "name" => {
                let n = cx.display_with(arg(args, 0), &*self);
                if let Some(at) = &self.name_at {
                    if self.model.name != n {
                        let msg = format!("{}: \"{}\" ersetzt den Namen \"{}\" aus {}", name, n, self.model.name, place(at));
                        cx.warn(&msg);
                    }
                }
                self.model.name = n;
                self.name_at = Some(cx.position());
                Ok(Value::Nil)
            }
            "budget" => {
                let n = count_or(cx, name, args, 0, None, "triangles")? as u64;
                if let Some(old) = self.model.budget.filter(|&b| b != n) {
                    let msg = format!("{}: {} ersetzt das Budget {} aus {}", name, n, old, place(&self.model.budget_at));
                    cx.warn(&msg);
                }
                self.model.budget = Some(n);
                self.model.budget_at = cx.position();
                Ok(Value::Nil)
            }
            "expect_size" => {
                // x, y, z or [x, y, z] or a vec; nil for an axis that is not checked
                let (dims, ti): (Vec<Value>, usize) = if let Some(p) = vec_data(cx, arg(args, 0)) {
                    (p.iter().map(|&x| Value::Float(x)).collect(), 1)
                } else {
                    match cx.list(arg(args, 0)).map(|l| l.to_vec()) {
                        Some(l) if l.len() == 3 => (l, 1),
                        Some(_) => return Err(throw(cx, name, "die Maße müssen eine Liste [x, y, z] sein")),
                        None => ((0..3).map(|i| arg(args, i)).collect(), 3),
                    }
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
                let tol = num_or(cx, name, args, ti, 0.001, "tolerance")?;
                if tol < 0.0 {
                    return Err(throw(cx, name, "die Toleranz darf nicht negativ sein"));
                }
                self.model.expects.push(Expect { size, tolerance: tol, at: cx.position() });
                Ok(Value::Nil)
            }
            "uv_scale" => {
                let s = num(cx, name, args, 0, "meters")?;
                if !(s > 0.0) {
                    return Err(throw(cx, name, "meters muss größer 0 sein"));
                }
                // (the parts get their UV at the end of the run, all with the last value)
                if let Some(at) = &self.uv_at {
                    if self.model.uv_scale != s {
                        let msg = format!("{}: {} ersetzt {} aus {}; alle Teile bekommen den letzten Wert", name, geom::fnum(s), geom::fnum(self.model.uv_scale), place(at));
                        cx.warn(&msg);
                    }
                }
                self.model.uv_scale = s;
                self.uv_at = Some(cx.position());
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

/// vec.*: vector helpers on vec values and on lists of 2 or 3 numbers; a result in 3D is a vec, in 2D a list
fn vector(cx: &mut Cx, name: &str, f: &str, args: &[Value]) -> Result<Value, Fail> {
    let get = |cx: &mut Cx, i: usize, what: &str| -> Result<Vec<f64>, Fail> {
        let v = arg(args, i);
        if let Some(p) = vec_data(cx, v) {
            return Ok(p.to_vec());
        }
        let n = cx.list(v).map_or(0, |l| l.len());
        match numbers_of(cx, v, n).filter(|_| n == 2 || n == 3) {
            Some(x) => Ok(x),
            None => {
                let s = shown(cx, v);
                Err(throw(cx, name, &format!("{} muss ein vec oder eine Liste aus 2 oder 3 Zahlen sein, nicht {}", what, s)))
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
            return Ok(Value::Float(dot_n(&a, &b)));
        }
        "cross" => {
            let (a, b) = (get(cx, 0, "a")?, get(cx, 1, "b")?);
            if a.len() != 3 || b.len() != 3 {
                return Err(throw(cx, name, "das Kreuzprodukt braucht zwei Vektoren aus 3 Zahlen"));
            }
            cross3(&a, &b).to_vec()
        }
        "len" => {
            let a = get(cx, 0, "a")?;
            return Ok(Value::Float(len_n(&a)));
        }
        "dist" => {
            let (a, b) = (get(cx, 0, "a")?, get(cx, 1, "b")?);
            same(cx, &a, &b)?;
            return Ok(Value::Float(dist_n(&a, &b)));
        }
        "norm" => {
            let a = get(cx, 0, "a")?;
            match norm_n(&a) {
                Some(n) => n,
                None => return Err(throw(cx, name, "ein Nullvektor hat keine Richtung")),
            }
        }
        "lerp" => {
            let (a, b) = (get(cx, 0, "a")?, get(cx, 1, "b")?);
            same(cx, &a, &b)?;
            let t = num(cx, name, args, 2, "t")?;
            lerp_n(&a, &b, t)
        }
        _ => return Err(Fail::Unknown),
    };
    if out.len() == 3 {
        if !out.iter().all(|x| x.is_finite()) {
            return Err(throw(cx, name, "das Ergebnis ist keine endliche Zahl (zu groß)"));
        }
        // (a vec: v.x and the operators; a list of 2 stays a list)
        return new_vec(cx, &[out[0], out[1], out[2]]);
    }
    float_list(cx, &out)
}

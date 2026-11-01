// entry -- the entry points (spec section 13): stage inputs and outputs (built-ins per stage
// and direction with their types, locations, interpolation, invariant, blend_src),
// @workgroup_size, what the stage may use (workgroup variables only in compute, writable
// storage not in vertex, discard / derivatives / implicit-LOD sampling only in fragment,
// barriers only in compute, one immediate variable), unique (group, binding) per entry
// point, and the statically accessed globals and overrides
//
//   entry_points()   fills Module::entry_points (run after every function is checked)

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::{Span};
use crate::check::*;
use crate::ir::{self, *};

/// (built-in, stage, input?, type check)
fn builtin_ok(c: &mut Checker, b: BuiltinValue, stage: ir::Stage, input: bool, t: TypeId) -> Result<(), String> {
    use BuiltinValue::*;
    use ir::Stage::*;
    let v3u = c.t.vec(3, Scalar::U32);
    let v4f = c.t.vec(4, Scalar::F32);
    let (stages, dir_ok, ty_ok, want): (&[ir::Stage], bool, bool, &str) = match b {
        VertexIndex | InstanceIndex => (&[Vertex], input, t == U32, "u32"),
        ClipDistances => (
            &[Vertex],
            !input,
            matches!(c.t.get(t), Type::Array { elem: F32, count: ArrayCount::Fixed(n), .. } if n <= 8),
            "array<f32, N> mit N <= 8",
        ),
        Position => {
            if stage == Vertex {
                (&[Vertex], !input, t == v4f, "vec4<f32>")
            } else {
                (&[Fragment], input, t == v4f, "vec4<f32>")
            }
        }
        FrontFacing => (&[Fragment], input, t == BOOL, "bool"),
        FragDepth => (&[Fragment], !input, t == F32, "f32"),
        PrimitiveIndex | SampleIndex => (&[Fragment], input, t == U32, "u32"),
        SampleMask => (&[Fragment], true, t == U32, "u32"),
        LocalInvocationIndex | GlobalInvocationIndex | WorkgroupIndex => (&[Compute], input, t == U32, "u32"),
        LocalInvocationId | GlobalInvocationId | WorkgroupId | NumWorkgroups => (&[Compute], input, t == v3u, "vec3<u32>"),
        _ => (&[], false, false, "?"),
    };
    let dir = if input { "Eingabe" } else { "Ausgabe" };
    if !stages.contains(&stage) || !dir_ok {
        return Err(format!("`@builtin({})` ist keine {} der {}-Stufe", b.name(), dir, stage.name()));
    }
    if !ty_ok {
        return Err(format!("`@builtin({})` muss vom Typ {} sein, nicht `{}`", b.name(), want, c.tn(t)));
    }
    Ok(())
}

impl<'a> Checker<'a> {
    pub fn entry_points(&mut self) {
        for fi in 0..self.fns.len() {
            let Some(stage) = self.fns[fi].stage else { continue };
            if let Ok(ep) = self.entry_point(fi as u32, stage) {
                self.m.entry_points.push(ep);
            }
        }
    }

    /// one IO datum of a parameter / the return value (with its struct members)
    fn io_list(&mut self, ty: TypeId, io: Io, name: &str, param: Option<u32>, sp: Span, input: bool, stage: ir::Stage, out: &mut Vec<IoVar>) -> R<()> {
        if io.builtin.is_some() || io.location.is_some() {
            return self.io_one(ty, io, String::from(name), param, None, sp, input, stage, out);
        }
        let Type::Struct(si) = self.t.get(ty) else {
            let what = if input { "Der Parameter" } else { "Der Rückgabewert" };
            return Err(self.err(sp, format!("{} `{}` eines Einstiegspunkts braucht `@builtin(…)` oder `@location(…)`", what, name)));
        };
        let members = self.t.structs[si as usize].members.clone();
        let sname = self.t.structs[si as usize].name.clone();
        for (k, m) in members.iter().enumerate() {
            if m.io.builtin.is_none() && m.io.location.is_none() {
                return Err(self.err_note(sp, format!("das Member `{}` von `{}` braucht `@builtin(…)` oder `@location(…)`", m.name, sname), m.span, String::from("hier")));
            }
            if matches!(self.t.get(m.ty), Type::Struct(_)) {
                return Err(self.err_note(sp, format!("geschachtelte Strukturen sind als Ein- oder Ausgabe nicht erlaubt (`{}.{}`)", sname, m.name), m.span, String::from("hier")));
            }
            self.io_one(m.ty, m.io, format!("{}.{}", name, m.name), param, Some(k as u32), m.span, input, stage, out)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn io_one(&mut self, ty: TypeId, io: Io, name: String, param: Option<u32>, member: Option<u32>, sp: Span, input: bool, stage: ir::Stage, out: &mut Vec<IoVar>) -> R<()> {
        if io.builtin.is_some() && io.location.is_some() {
            return Err(self.err(sp, format!("`@builtin` und `@location` schließen sich aus (`{}`)", name)));
        }
        if let Some(b) = io.builtin {
            if let Err(m) = builtin_ok(self, b, stage, input, ty) {
                return Err(self.err(sp, m));
            }
            if let Some(p) = out.iter().find(|v| v.kind == IoKind::Builtin(b)) {
                let pn = p.name.clone();
                return Err(self.err(sp, format!("`@builtin({})` steht zweimal ({} und {})", b.name(), pn, name)));
            }
            out.push(IoVar { name, ty, param, member, kind: IoKind::Builtin(b), invariant: io.invariant, depth_mode: io.depth_mode });
            return Ok(());
        }
        let loc = io.location.unwrap_or(0);
        if stage == ir::Stage::Compute {
            return Err(self.err(sp, String::from("Compute-Shader haben keine benutzerdefinierten Ein- und Ausgaben (`@location`)")));
        }
        let s = match self.t.get(ty) {
            Type::Scalar(s) | Type::Vector(_, s) if s.is_numeric() && !s.is_abstract() => s,
            _ => return Err(self.err(sp, format!("`@location` braucht einen numerischen Skalar oder Vektor, nicht `{}`", self.tn(ty)))),
        };
        if io.blend_src.is_some() && (input || stage != ir::Stage::Fragment) {
            return Err(self.err(sp, String::from("`@blend_src` gibt es nur bei Ausgaben der Fragment-Stufe")));
        }
        if input {
            if let Some(p) = out.iter().find(|v| matches!(v.kind, IoKind::Location { location, .. } if location == loc)) {
                let pn = p.name.clone();
                return Err(self.err(sp, format!("`@location({})` steht zweimal in den Eingaben ({} und {})", loc, pn, name)));
            }
        }
        // interpolation: user-defined vertex outputs and fragment inputs
        let interstage = (stage == ir::Stage::Vertex && !input) || (stage == ir::Stage::Fragment && input);
        let interp = if interstage {
            let int = s.is_int();
            match io.interpolate {
                Some((t, smp)) => {
                    if int && t != InterpType::Flat {
                        return Err(self.err(sp, format!("ganzzahlige Ein- und Ausgaben zwischen den Stufen brauchen `@interpolate(flat)` (`{}`)", name)));
                    }
                    let smp = smp.unwrap_or(if t == InterpType::Flat { InterpSampling::First } else { InterpSampling::Center });
                    Some((t, smp))
                }
                None => {
                    if int {
                        return Err(self.err(sp, format!("ganzzahlige Ein- und Ausgaben zwischen den Stufen brauchen `@interpolate(flat)` (`{}`)", name)));
                    }
                    Some((InterpType::Perspective, InterpSampling::Center))
                }
            }
        } else {
            None
        };
        out.push(IoVar { name, ty, param, member, kind: IoKind::Location { location: loc, blend_src: io.blend_src, interp }, invariant: false, depth_mode: None });
        Ok(())
    }

    fn entry_point(&mut self, fi: u32, stage: ir::Stage) -> R<EntryPoint> {
        let f = self.m.functions[fi as usize].clone();
        let info = self.fns[fi as usize].clone();
        let decl = self.fn_decl[fi as usize];
        let name_sp = decl.name.span;
        let mut inputs = Vec::new();
        let mut ok = true;
        for (k, p) in f.params.iter().enumerate() {
            if self.io_list(p.ty, p.io, &p.name, Some(k as u32), p.span, true, stage, &mut inputs).is_err() {
                ok = false;
            }
        }
        let mut outputs = Vec::new();
        if let Some(rt) = f.ret {
            if stage == ir::Stage::Compute {
                let _ = self.err(name_sp, String::from("ein Compute-Einstiegspunkt hat keinen Rückgabewert"));
                ok = false;
            } else {
                let rsp = decl.ret.as_ref().map(|r| r.1.span).unwrap_or(name_sp);
                if self.io_list(rt, f.ret_io, "return", None, rsp, false, stage, &mut outputs).is_err() {
                    ok = false;
                }
            }
        }
        if stage == ir::Stage::Vertex && ok && !outputs.iter().any(|o| o.kind == IoKind::Builtin(BuiltinValue::Position)) {
            let _ = self.err(name_sp, format!("der Vertex-Shader `{}` muss `@builtin(position)` ausgeben", f.name));
            ok = false;
        }
        // workgroup size
        let mut wg = None;
        let mut wg_ovr = BTreeSet::new();
        if stage == ir::Stage::Compute {
            if let Some(a) = self.attr(&decl.attrs, "workgroup_size") {
                match self.workgroup_size(a.args(), &mut wg_ovr) {
                    Ok(w) => wg = Some(w),
                    Err(_) => ok = false,
                }
            }
        }
        // what the stage may use
        let sname = stage.name();
        if stage != ir::Stage::Fragment && info.uses_discard {
            let _ = self.err(name_sp, format!("`discard` ist nur in Fragment-Shadern erlaubt, `{}` ist ein {}-Einstiegspunkt", f.name, sname));
            ok = false;
        }
        if stage != ir::Stage::Fragment {
            if let Some((usp, n)) = &info.frag_use {
                let _ = self.err_note(name_sp, format!("`{}` gibt es nur in Fragment-Shadern, `{}` ist ein {}-Einstiegspunkt", n, f.name, sname), *usp, String::from("hier benutzt"));
                ok = false;
            }
        }
        if stage != ir::Stage::Compute {
            if let Some((usp, n)) = &info.compute_use {
                let _ = self.err_note(name_sp, format!("`{}` gibt es nur in Compute-Shadern, `{}` ist ein {}-Einstiegspunkt", n, f.name, sname), *usp, String::from("hier benutzt"));
                ok = false;
            }
        }
        let globals: Vec<u32> = info.globals.iter().copied().collect();
        let mut resources: Vec<(u32, Binding)> = Vec::new();
        let mut immediate = None;
        for &g in &globals {
            let gl = self.m.globals[g as usize].clone();
            match gl.space {
                AddressSpace::Workgroup if stage != ir::Stage::Compute => {
                    let _ = self.err_note(name_sp, format!("die workgroup-Variable `{}` gibt es nur in Compute-Shadern", gl.name), gl.span, String::from("hier deklariert"));
                    ok = false;
                }
                AddressSpace::Storage if stage == ir::Stage::Vertex && gl.access == Access::ReadWrite => {
                    let _ = self.err_note(name_sp, format!("Vertex-Shader dürfen Speicherpuffer nur lesen: `{}` ist read_write", gl.name), gl.span, String::from("hier deklariert"));
                    ok = false;
                }
                AddressSpace::Handle if stage == ir::Stage::Vertex && matches!(self.t.get(gl.ty), Type::Texture(_, TexKind::Storage(_, a)) if a != Access::Read) => {
                    let _ = self.err_note(name_sp, format!("Vertex-Shader dürfen Speichertexturen nur lesen: `{}`", gl.name), gl.span, String::from("hier deklariert"));
                    ok = false;
                }
                AddressSpace::Immediate => {
                    if let Some(prev) = immediate {
                        let pname = self.m.globals[prev as usize].name.clone();
                        let _ = self.err_note(name_sp, format!("ein Einstiegspunkt darf nur eine immediate-Variable benutzen (`{}` und `{}`)", pname, gl.name), gl.span, String::from("hier deklariert"));
                        ok = false;
                    }
                    immediate = Some(g);
                }
                _ => {}
            }
            if let Some(b) = gl.binding {
                if let Some(&(pg, _)) = resources.iter().find(|r| r.1 == b) {
                    let pspan = self.m.globals[pg as usize].span;
                    let pname = self.m.globals[pg as usize].name.clone();
                    let _ = self.err_note(gl.span, format!("`{}` und `{}` haben beide @group({}) @binding({}) im Einstiegspunkt `{}`", pname, gl.name, b.group, b.binding, f.name), pspan, String::from("hier"));
                    ok = false;
                }
                resources.push((g, b));
            }
        }
        // the overrides: the body's, the initializers' and array sizes' of used globals, the
        // workgroup size's, and those of their initializers (transitively)
        let mut ovr: BTreeSet<u32> = info.overrides.clone();
        for g in &globals {
            if let Some(s) = self.global_ovr.get(g) {
                ovr.extend(s.iter().copied());
            }
        }
        ovr.extend(wg_ovr);
        let mut work: Vec<u32> = ovr.iter().copied().collect();
        while let Some(o) = work.pop() {
            if let Some(s) = self.override_ovr.get(&o) {
                for &x in s {
                    if ovr.insert(x) {
                        work.push(x);
                    }
                }
            }
        }
        if !ok {
            return Err(E);
        }
        Ok(EntryPoint {
            name: f.name.clone(),
            func: fi,
            stage,
            workgroup_size: wg,
            inputs,
            outputs,
            globals,
            resources,
            overrides: ovr.into_iter().collect(),
            immediate,
        })
    }

    fn workgroup_size(&mut self, args: &[crate::ast::Expr], ovr: &mut BTreeSet<u32>) -> R<[WgSize; 3]> {
        let saved_f = self.f.take();
        let mut tes = Vec::new();
        let mut res = Ok(());
        for a in args {
            match self.value(a) {
                Ok(te) => {
                    if te.stage == Phase::Runtime {
                        res = Err(self.err(a.span, String::from("`@workgroup_size` braucht const- oder override-Ausdrücke")));
                    }
                    tes.push(te);
                }
                Err(e) => res = Err(e),
            }
        }
        self.f = saved_f;
        res?;
        // one type for all: the concrete one, or i32
        if let Some(bad) = tes.iter().find(|t| !matches!(t.ty, I32 | U32 | AINT)) {
            let sp = bad.span;
            return Err(self.err(sp, String::from("die Argumente von `@workgroup_size` müssen alle i32 oder alle u32 sein")));
        }
        let concrete: Vec<TypeId> = tes.iter().map(|t| t.ty).filter(|&t| !self.t.is_abstract(t)).collect();
        let ty = concrete.first().copied().unwrap_or(I32);
        if concrete.iter().any(|&t| t != ty) || (ty != I32 && ty != U32) {
            let sp = args.first().map(|a| a.span).unwrap_or_default().to(args.last().map(|a| a.span).unwrap_or_default());
            return Err(self.err(sp, String::from("die Argumente von `@workgroup_size` müssen alle i32 oder alle u32 sein")));
        }
        let mut out = [WgSize::Const(1); 3];
        let saved_f = self.f.take();
        let mut res = Ok(());
        for (k, te) in tes.into_iter().enumerate() {
            let sp = te.span;
            match self.convert(te, ty) {
                Ok(te) => {
                    if te.stage == Phase::Const {
                        let v = te.val.as_ref().and_then(crate::consteval::as_int).unwrap_or(1);
                        if v <= 0 {
                            res = Err(self.err(sp, format!("`@workgroup_size` muss größer als 0 sein, nicht {}", v)));
                        }
                        out[k] = WgSize::Const(v.clamp(0, u32::MAX as i128) as u32);
                    } else {
                        let id = self.node(&te);
                        collect_overrides(&self.m.exprs, id, ovr);
                        out[k] = WgSize::Override(id);
                    }
                }
                Err(e) => res = Err(e),
            }
        }
        self.f = saved_f;
        res?;
        Ok(out)
    }
}

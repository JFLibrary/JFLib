// pipeline -- what WebGPU checks when it creates a pipeline from a compiled entry point
// (docs/WGSL.md "Overrides" and "Pipeline-Prüfungen")
//
//   output.constants(entry, &[(key, value)]) -> Result<Vec<(u32, f64)>, String>
//        the pipeline constants for lib/gpu (gpu::Constant { id, value }): keys as in WebGPU's
//        GPUProgrammableStage.constants (the override's name, or its @id as a decimal number
//        when it has one), values converted the way WebGPU converts them; every override the
//        entry point uses is then evaluated on the CPU (initializers with the front end's
//        constant evaluator) and every specialization constant of the module gets its final
//        value, the hidden ones (workgroup sizes, array counts) too.  Errors as WebGPU
//        reports them: an unknown key, a value out of the type's range, an override without
//        initializer left unset, an evaluation error (division by zero ...), a workgroup size
//        or array count of 0
//   output.workgroup_size(entry, constants) -> Result<[u32; 3], String>
//   output.check_pipeline(entry, constants, &Limits) -> Result<(), String>   the above plus the
//        limits (workgroup size and invocations, workgroup memory, colour targets, inter-stage
//        locations, immediate bytes) and the storage texture formats WebGPU's core allows
//   check_interface(vertex, fragment) -> Result<(), String>   every fragment input has a vertex
//        output at its location with the same component type and count (at least)
//   Limits::WEBGPU   WebGPU's default limits (what a browser gives without requiredLimits)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::builtins;
use crate::consteval::{self, check_float, comp, comps, CR};
use crate::ast::BinOp;
use crate::ir::*;
use crate::spirv::{EntryOut, Output, ResourceKind, SpecSource};
use crate::types::Types;
use crate::Module;

/// the limits a pipeline is checked against
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_compute_invocations: u32,
    pub max_workgroup_size: [u32; 3],
    pub max_workgroup_storage: u32,
    pub max_color_attachments: u32,
    pub max_inter_stage_variables: u32,
    pub max_immediate_size: u32,
    /// the device offers bgra8unorm-storage
    pub bgra8unorm_storage: bool,
    /// texture-formats-tier2: read_write storage textures of more formats
    pub texture_formats_tier2: bool,
}

impl Limits {
    /// WebGPU's default limits (Chrome 156 without requiredLimits)
    pub const WEBGPU: Limits = Limits {
        max_compute_invocations: 256,
        max_workgroup_size: [256, 256, 64],
        max_workgroup_storage: 16384,
        max_color_attachments: 8,
        max_inter_stage_variables: 16,
        max_immediate_size: 64,
        bgra8unorm_storage: false,
        texture_formats_tier2: false,
    };
}

/// the evaluator of module-scope override-expressions
struct Ev<'a> {
    m: &'a Module,
    t: Types,
    vals: Vec<Option<Value>>,
    busy: Vec<bool>,
}

impl<'a> Ev<'a> {
    fn new(m: &'a Module, vals: Vec<Option<Value>>) -> Ev<'a> {
        Ev { m, t: Types::from_module(&m.types, &m.structs), vals, busy: alloc::vec![false; m.overrides.len()] }
    }

    fn over(&mut self, o: u32) -> CR<Value> {
        let i = o as usize;
        if let Some(v) = self.vals.get(i).cloned().flatten() {
            return Ok(v);
        }
        let ov = &self.m.overrides[i];
        let Some(init) = ov.init else {
            return Err(format!("das Override `{}` hat keinen Wert: es hat keinen Initialisierer, die Pipeline muss es setzen", ov.name));
        };
        if self.busy[i] {
            return Err(format!("das Override `{}` hängt von sich selbst ab", ov.name));
        }
        self.busy[i] = true;
        let v = self.expr(init);
        self.busy[i] = false;
        let v = v.map_err(|e| format!("`{}`: {}", self.m.overrides[i].name, e))?;
        self.vals[i] = Some(v.clone());
        Ok(v)
    }

    fn expr(&mut self, e: ExprId) -> CR<Value> {
        let m = self.m;
        let ex = &m.exprs[e as usize];
        let ty = ex.ty;
        let tyof = |x: ExprId| m.exprs[x as usize].ty;
        match &ex.kind {
            ExprKind::Const(v) => Ok(v.clone()),
            ExprKind::Override(o) => self.over(*o),
            ExprKind::Unary(op, a) => {
                let v = self.expr(*a)?;
                consteval::unary(&self.t, *op, &v, tyof(*a))
            }
            ExprKind::Binary(op, a, b) => {
                let va = self.expr(*a)?;
                if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
                    if va == Value::Bool(*op == BinOp::LogOr) {
                        return Ok(va);
                    }
                    return self.expr(*b);
                }
                let vb = self.expr(*b)?;
                let (ta, tb) = (tyof(*a), tyof(*b));
                let (ka, kb) = (self.t.get(ta), self.t.get(tb));
                let mat = matches!(ka, Type::Matrix(..)) || matches!(kb, Type::Matrix(..));
                if *op == BinOp::Mul && mat && !(self.t.is_scalar(ta) || self.t.is_scalar(tb)) {
                    return consteval::mat_mul(&self.t, &va, ta, &vb, tb);
                }
                if self.t.is_scalar(ta) && !self.t.is_scalar(tb) && !op.is_comparison() && !matches!(op, BinOp::Shl | BinOp::Shr) {
                    // scalar * vector / matrix: component-wise with the scalar first
                    let n = consteval::shape(&self.t, tb).map(|s| s.0).unwrap_or(0);
                    let mut out = Vec::with_capacity(n);
                    for i in 0..n {
                        let ct = consteval::comp_type(&self.t, tb, i);
                        let c = comp(&self.t, &vb, tb, i);
                        out.push(if self.t.is_scalar(ct) { consteval::binary_scalar(*op, &va, &c)? } else {
                            let n2 = consteval::shape(&self.t, ct).map(|s| s.0).unwrap_or(0);
                            let mut o2 = Vec::with_capacity(n2);
                            for j in 0..n2 {
                                o2.push(consteval::binary_scalar(*op, &va, &comp(&self.t, &c, ct, j))?);
                            }
                            Value::Comp(o2)
                        });
                    }
                    return Ok(Value::Comp(out));
                }
                consteval::binary(&self.t, *op, &va, &vb, ta, tb)
            }
            ExprKind::Construct(args) => {
                let mut out = Vec::new();
                let vector = matches!(self.t.get(ty), Type::Vector(..));
                for &a in args {
                    let v = self.expr(a)?;
                    let at = tyof(a);
                    if vector && matches!(self.t.get(at), Type::Vector(..)) {
                        out.extend(comps(&self.t, &v, at));
                    } else {
                        out.push(v);
                    }
                }
                Ok(Value::Comp(out))
            }
            ExprKind::Splat(a) => {
                let v = self.expr(*a)?;
                let n = match self.t.get(ty) {
                    Type::Vector(n, _) => n as usize,
                    _ => 1,
                };
                Ok(Value::Comp(alloc::vec![v; n]))
            }
            ExprKind::Convert(a) => {
                let v = self.expr(*a)?;
                consteval::convert(&mut self.t, &v, tyof(*a), ty, true)
            }
            ExprKind::Bitcast(a) => {
                let v = self.expr(*a)?;
                consteval::bitcast(&self.t, &v, tyof(*a), ty)
            }
            ExprKind::Builtin(b, args) => {
                let mut vs = Vec::with_capacity(args.len());
                for &a in args {
                    vs.push((self.expr(a)?, tyof(a)));
                }
                builtins::eval(&mut self.t, *b, &vs)
            }
            ExprKind::Index(a, i) => {
                let v = self.expr(*a)?;
                let k = self.expr(*i)?;
                let k = consteval::as_int(&k).unwrap_or(-1);
                let n = consteval::shape(&self.t, tyof(*a)).map(|s| s.0).unwrap_or(0);
                if k < 0 || k as usize >= n {
                    return Err(format!("der Index {} liegt außerhalb von 0..{}", k, n.saturating_sub(1)));
                }
                Ok(comp(&self.t, &v, tyof(*a), k as usize))
            }
            ExprKind::Member(a, k) => {
                let v = self.expr(*a)?;
                Ok(comp(&self.t, &v, tyof(*a), *k as usize))
            }
            ExprKind::Swizzle(a, cs, n) => {
                let v = self.expr(*a)?;
                let at = tyof(*a);
                Ok(Value::Comp(cs[..*n as usize].iter().map(|&c| comp(&self.t, &v, at, c as usize)).collect()))
            }
            _ => Err(String::from("das ist kein Override-Ausdruck")),
        }
    }
}

/// the defaults of every override (None: no initializer, or it does not evaluate)
pub(crate) fn default_values(m: &Module) -> Vec<Option<Value>> {
    let mut ev = Ev::new(m, alloc::vec![None; m.overrides.len()]);
    for o in 0..m.overrides.len() as u32 {
        let _ = ev.over(o);
    }
    ev.vals
}

/// a WebGPU pipeline constant converted to the override's type (WebIDL: bool = not 0 and not
/// NaN; integers [EnforceRange]; floats finite and in range, rounded to nearest)
fn convert_constant(name: &str, x: f64, s: Scalar) -> Result<Value, String> {
    let bad = || format!("die Pipeline-Konstante `{}` = {} lässt sich nicht als {} darstellen", name, x, s.name());
    match s {
        Scalar::Bool => Ok(Value::Bool(x != 0.0 && !x.is_nan())),
        Scalar::I32 | Scalar::U32 => {
            if !x.is_finite() {
                return Err(bad());
            }
            let t = crate::math::trunc(x);
            if s == Scalar::I32 {
                if !(-2147483648.0..=2147483647.0).contains(&t) {
                    return Err(bad());
                }
                Ok(Value::I32(t as i32))
            } else {
                if !(0.0..=4294967295.0).contains(&t) {
                    return Err(bad());
                }
                Ok(Value::U32(t as u32))
            }
        }
        _ => check_float(x, s).ok_or_else(bad),
    }
}

/// the overrides given as WebGPU pipeline constants (by name, or by @id when there is one)
fn given_values(m: &Module, given: &[(&str, f64)]) -> Result<Vec<Option<Value>>, String> {
    let mut vals: Vec<Option<Value>> = alloc::vec![None; m.overrides.len()];
    for &(key, x) in given {
        let hit = m.overrides.iter().position(|o| match o.id {
            Some(n) => key.parse::<u32>().ok() == Some(n as u32) && key.bytes().all(|c| c.is_ascii_digit()),
            None => o.name == key,
        });
        let Some(i) = hit else {
            return Err(format!("die Pipeline-Konstante `{}` gibt es im Modul nicht (Overrides mit @id werden über die Zahl gesetzt)", key));
        };
        if vals[i].is_some() {
            return Err(format!("die Pipeline-Konstante `{}` steht zweimal", key));
        }
        let s = crate::spirv::scalar_of(m, m.overrides[i].ty);
        vals[i] = Some(convert_constant(key, x, s)?);
    }
    Ok(vals)
}

/// every override value an entry point needs (given or from its initializer); checks the
/// workgroup size and the override-sized arrays (> 0)
pub(crate) fn override_values(m: &Module, ep: &EntryPoint, given: &[(&str, f64)]) -> Result<Vec<Option<Value>>, String> {
    let vals = given_values(m, given)?;
    let mut ev = Ev::new(m, vals);
    for &o in &ep.overrides {
        ev.over(o)?;
    }
    if let Some(ws) = ep.workgroup_size {
        for w in ws {
            if let WgSize::Override(e) = w {
                let x = ev.expr(e)?;
                if consteval::as_int(&x).unwrap_or(0) <= 0 {
                    return Err(format!("die Workgroup-Größe {} muss größer als 0 sein", consteval::fmt(&x)));
                }
            }
        }
    }
    for g in &ep.globals {
        let gl = &m.globals[*g as usize];
        if let Some(e) = array_count_expr(m, gl.ty) {
            let x = match e {
                Err(o) => ev.over(o)?,
                Ok(e) => ev.expr(e)?,
            };
            if consteval::as_int(&x).unwrap_or(0) <= 0 {
                return Err(format!("die Länge {} des Arrays `{}` muss größer als 0 sein", consteval::fmt(&x), gl.name));
            }
        }
    }
    Ok(ev.vals)
}

/// the override-sized array count inside a workgroup variable's type: Ok(expression) or
/// Err(override)
fn array_count_expr(m: &Module, t: TypeId) -> Option<Result<ExprId, u32>> {
    match *m.ty(t) {
        Type::Array { count: ArrayCount::Override(o), .. } => Some(Err(o)),
        Type::Array { count: ArrayCount::Expr(e), .. } => Some(Ok(e)),
        Type::Array { elem, .. } => array_count_expr(m, elem),
        _ => None,
    }
}

/// a module-scope override-expression as u32 (workgroup sizes, array counts)
pub(crate) fn eval_u32(m: &Module, e: ExprId, vals: &[Option<Value>]) -> Result<u32, String> {
    let mut ev = Ev::new(m, vals.to_vec());
    let v = ev.expr(e)?;
    match consteval::as_int(&v) {
        Some(x) if x > 0 && x <= u32::MAX as i128 => Ok(x as u32),
        _ => Err(format!("{} ist keine gültige Größe", consteval::fmt(&v))),
    }
}

fn value_f64(v: &Value) -> f64 {
    match *v {
        Value::Bool(b) => b as u8 as f64,
        Value::I32(x) => x as f64,
        Value::U32(x) => x as f64,
        Value::F32(x) | Value::F16(x) => x as f64,
        Value::AInt(x) => x as f64,
        Value::AFloat(x) => x,
        _ => 0.0,
    }
}

impl Output {
    fn entry_or(&self, entry: &str) -> Result<&EntryOut, String> {
        self.entry(entry).ok_or_else(|| format!("das Modul hat keinen Einstiegspunkt `{}`", entry))
    }

    /// the pipeline constants (SpecId, value) of an entry point for WebGPU-style constants (see
    /// the module comment); pass them to lib/gpu as Constant { id, value }
    pub fn constants(&self, entry: &str, given: &[(&str, f64)]) -> Result<Vec<(u32, f64)>, String> {
        let e = self.entry_or(entry)?;
        let m = &self.module;
        let ep = m.entry_point(entry).ok_or_else(|| String::from("?"))?;
        let vals = override_values(m, ep, given)?;
        let mut out = Vec::with_capacity(e.spec.len());
        for &(sid, src) in &e.spec {
            let x = match src {
                SpecSource::Override(o) => vals.get(o as usize).cloned().flatten().map(|v| value_f64(&v)).unwrap_or(0.0),
                SpecSource::Expr(x) => eval_u32(m, x, &vals)? as f64,
            };
            out.push((sid, x));
        }
        Ok(out)
    }

    /// the workgroup size of a compute entry point with these pipeline constants
    pub fn workgroup_size(&self, entry: &str, given: &[(&str, f64)]) -> Result<[u32; 3], String> {
        let m = &self.module;
        let ep = m.entry_point(entry).ok_or_else(|| format!("das Modul hat keinen Einstiegspunkt `{}`", entry))?;
        let Some(ws) = ep.workgroup_size else { return Err(format!("`{}` ist kein Compute-Shader", entry)) };
        let vals = override_values(m, ep, given)?;
        let mut out = [1u32; 3];
        for (i, w) in ws.iter().enumerate() {
            out[i] = match *w {
                WgSize::Const(n) => n,
                WgSize::Override(e) => eval_u32(m, e, &vals)?,
            };
        }
        Ok(out)
    }

    /// what WebGPU checks when it creates a pipeline from this entry point (module comment)
    pub fn check_pipeline(&self, entry: &str, given: &[(&str, f64)], lim: &Limits) -> Result<(), String> {
        let e = self.entry_or(entry)?;
        let m = &self.module;
        let ep = m.entry_point(entry).ok_or_else(|| String::from("?"))?;
        let vals = override_values(m, ep, given)?;
        if ep.stage == Stage::Compute {
            let ws = self.workgroup_size(entry, given)?;
            for (i, (&w, &max)) in ws.iter().zip(lim.max_workgroup_size.iter()).enumerate() {
                if w > max {
                    return Err(format!("die Workgroup-Größe {:?} überschreitet in {} das Limit {}", ws, ["x", "y", "z"][i], max));
                }
            }
            let n = ws[0] as u64 * ws[1] as u64 * ws[2] as u64;
            if n > lim.max_compute_invocations as u64 {
                return Err(format!("die Workgroup hat {} Aufrufe, das Limit ist {}", n, lim.max_compute_invocations));
            }
            let mut bytes = 0u64;
            for g in &ep.globals {
                let gl = &m.globals[*g as usize];
                if gl.space == AddressSpace::Workgroup {
                    bytes += size_with(m, gl.ty, &vals).div_ceil(16) * 16;
                }
            }
            if bytes > lim.max_workgroup_storage as u64 {
                return Err(format!("die Workgroup-Variablen brauchen {} Byte, das Limit ist {}", bytes, lim.max_workgroup_storage));
            }
        }
        for io in &e.outputs {
            if let IoKind::Location { location, .. } = io.kind {
                let max = if ep.stage == Stage::Fragment { lim.max_color_attachments } else { lim.max_inter_stage_variables };
                if location >= max {
                    return Err(format!("die Ausgabe `{}` hat @location({}), erlaubt sind 0 bis {}", io.name, location, max - 1));
                }
            }
        }
        if ep.stage == Stage::Fragment {
            for io in &e.inputs {
                if let IoKind::Location { location, .. } = io.kind {
                    if location >= lim.max_inter_stage_variables {
                        return Err(format!("die Eingabe `{}` hat @location({}), erlaubt sind 0 bis {}", io.name, location, lim.max_inter_stage_variables - 1));
                    }
                }
            }
        }
        if e.immediate_size > lim.max_immediate_size {
            return Err(format!("die immediate-Daten haben {} Byte, das Limit ist {}", e.immediate_size, lim.max_immediate_size));
        }
        for r in &e.resources {
            if let ResourceKind::StorageTexture { format, access, .. } = r.kind {
                if format == TexelFormat::Bgra8unorm && !lim.bgra8unorm_storage {
                    return Err(format!("`{}`: bgra8unorm als Speichertextur braucht das Gerätefeature bgra8unorm-storage", r.name));
                }
                if access == Access::ReadWrite && !read_write_format(format, lim.texture_formats_tier2) {
                    return Err(format!("`{}`: das Format {} erlaubt keinen read_write-Zugriff (nur r32float, r32sint, r32uint)", r.name, format.name()));
                }
            }
        }
        Ok(())
    }
}

/// the formats a read_write storage texture may have (core: r32*; tier2 adds more)
fn read_write_format(f: TexelFormat, tier2: bool) -> bool {
    use TexelFormat as T;
    matches!(f, T::R32float | T::R32sint | T::R32uint)
        || tier2 && matches!(f, T::Rgba8unorm | T::Rgba8uint | T::Rgba8sint | T::Rgba16uint | T::Rgba16sint | T::Rgba16float
            | T::Rgba32uint | T::Rgba32sint | T::Rgba32float | T::R8unorm | T::R8uint | T::R8sint | T::R16uint | T::R16sint
            | T::R16float | T::Rg8unorm | T::Rg8uint | T::Rg8sint | T::Rg16uint | T::Rg16sint | T::Rg16float | T::Rg32uint
            | T::Rg32sint | T::Rg32float | T::Rgb10a2uint | T::Rgb10a2unorm | T::Rg11b10ufloat)
}

/// the size of a type in memory with override-sized arrays evaluated
fn size_with(m: &Module, t: TypeId, vals: &[Option<Value>]) -> u64 {
    match *m.ty(t) {
        Type::Array { count, stride, .. } => {
            let n = match count {
                ArrayCount::Fixed(n) => n as u64,
                ArrayCount::Override(o) => vals.get(o as usize).cloned().flatten().and_then(|v| consteval::as_int(&v)).unwrap_or(0).max(0) as u64,
                ArrayCount::Expr(e) => eval_u32(m, e, vals).unwrap_or(0) as u64,
                ArrayCount::Runtime => 0,
            };
            n * stride as u64
        }
        Type::Struct(i) => m.structs[i as usize].size as u64,
        _ => {
            let t2 = Types::from_module(&m.types, &m.structs);
            t2.layout(t).map(|l| l.1 as u64).unwrap_or(4)
        }
    }
}

/// the scalar kind and component count of a stage variable's type
fn io_shape(m: &Module, t: TypeId) -> (Scalar, u8) {
    match *m.ty(t) {
        Type::Scalar(s) => (s, 1),
        Type::Vector(n, s) => (s, n),
        _ => (Scalar::Bool, 0),
    }
}

/// WebGPU's inter-stage rules for a vertex and a fragment entry point: every fragment input
/// at a location has a vertex output there with the same component type, at least as many
/// components and the same interpolation
pub fn check_interface(m: &Module, vs: &EntryOut, fs: &EntryOut) -> Result<(), String> {
    for i in &fs.inputs {
        let IoKind::Location { location, interp, .. } = i.kind else { continue };
        let Some(o) = vs.outputs.iter().find(|o| matches!(o.kind, IoKind::Location { location: l, .. } if l == location)) else {
            return Err(format!("die Fragment-Eingabe bei @location({}) hat keine Vertex-Ausgabe", location));
        };
        let (si, ni) = io_shape(m, i.ty);
        let (so, no) = io_shape(m, o.ty);
        if si != so || no < ni {
            return Err(format!("@location({}): die Vertex-Ausgabe `{}` ({}) passt nicht zur Fragment-Eingabe `{}` ({})", location,
                               o.name, m.type_name(o.ty), i.name, m.type_name(i.ty)));
        }
        if let IoKind::Location { interp: oi, .. } = o.kind {
            if oi != interp {
                return Err(format!("@location({}): die Interpolation der Vertex-Ausgabe und der Fragment-Eingabe unterscheidet sich", location));
            }
        }
    }
    Ok(())
}

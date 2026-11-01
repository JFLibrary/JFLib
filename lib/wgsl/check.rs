// check -- the semantic checker, module level: directives, the module-scope declarations in
// dependency order, name resolution, types, attributes, structs with layout, global
// variables, overrides, consts, function signatures, const_assert; then check_expr.rs (the
// expressions), check_stmt.rs (function bodies, behaviors) and entry.rs (entry points)
//
//   check(src, &ast, &opts) -> Result<Module, Vec<Diagnostic>>
//
// Order: every module-scope declaration's dependencies (the module-scope names it mentions,
// respecting local scopes) are collected first; an iterative depth-first search orders them
// (callees before callers) and reports cycles ("zyklische Abhängigkeit"), so no check
// recurses from one declaration into another.  Errors: each failing construct reports once
// and yields E; whatever depends on it fails silently.  At most MAX_ERRORS errors are kept.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast::{self, Attr, AttrKind, Decl, DeclKind, Expr, ExprKind, Span, StmtKind, VarDecl, VarKind};
use crate::ir::{self, *};
use crate::types::{round_up, Types};
use crate::{feature, Diagnostic, LineMap, Options};

pub const MAX_ERRORS: usize = 60;

/// an error that is already reported
#[derive(Debug, Clone, Copy)]
pub struct E;
pub type R<T> = Result<T, E>;

/// when an expression's value is known
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Phase {
    Const,
    Override,
    Runtime,
}

pub const NONE: u32 = u32::MAX;

/// a checked expression: its IR node (NONE for a constant not yet materialized), type,
/// evaluation stage and, for const-expressions, the value (None inside a short-circuited
/// operand, which is type-checked but not evaluated)
#[derive(Clone, Debug)]
pub struct TE {
    pub id: u32,
    pub ty: TypeId,
    pub stage: Phase,
    pub val: Option<Value>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum DeclInfo {
    Alias(TypeId),
    Struct(TypeId),
    Const(TypeId, Value),
    Override(u32),
    Var(u32),
    Fn(u32),
    Assert,
}

pub enum DeclState {
    Pending,
    Done(DeclInfo),
    Failed,
}

/// what the checker knows about a function for its callers
#[derive(Clone, Debug, Default)]
pub struct FnInfo {
    pub name: String,
    pub params: Vec<TypeId>,
    pub ret: Option<TypeId>,
    pub must_use: bool,
    pub stage: Option<ir::Stage>,
    /// body behavior: false when the body never falls through or returns
    pub returns: bool,
    pub globals: BTreeSet<u32>,
    pub overrides: BTreeSet<u32>,
    pub uses_discard: bool,
    /// the first fragment-only / compute-only built-in reachable (place, name)
    pub frag_use: Option<(Span, String)>,
    pub compute_use: Option<(Span, String)>,
    /// alias analysis: pointer parameters read / written, module-scope variables read /
    /// written (callees included)
    pub ptr_read: Vec<bool>,
    pub ptr_write: Vec<bool>,
    pub g_read: BTreeSet<u32>,
    pub g_write: BTreeSet<u32>,
    pub span: Span,
}

/// a local name
#[derive(Clone, Debug)]
pub enum LocalRef {
    Const(TypeId, Value),
    Let(u32),
    Var(u32),
    Param(u32),
    /// a declaration that failed: uses fail silently (no cascade of "not known")
    Error,
}

/// the root identifier of a memory view (alias analysis)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Root {
    Global(u32),
    Param(u32),
    Local(u32),
}

/// a loop / switch / continuing level for break and continue
pub struct Ctl {
    pub kind: CtlKind,
    /// loops: the scope depth of the body (usize::MAX: no continue check)
    pub depth: usize,
    /// loops: the local uids declared at the top level of the body, in order
    pub decls: Vec<(u32, Span, String)>,
    /// loops: (number of body declarations before the continue, its place)
    pub continues: Vec<(usize, Span)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CtlKind {
    Loop,
    Switch,
    Continuing,
}

/// the state of the function being checked
pub struct FnCtx {
    pub exprs: Vec<ir::Expr>,
    pub locals: Vec<ir::Local>,
    pub local_init: Vec<Option<ExprId>>,
    /// per scope: name, reference, uid (declaration number), place
    pub scopes: Vec<Vec<(String, LocalRef, u32, Span)>>,
    pub next_uid: u32,
    pub params: Vec<ir::Param>,
    pub ret: Option<TypeId>,
    pub ctl: Vec<Ctl>,
    pub in_continuing: u32,
    /// the uids used while checking a continuing block (the innermost one)
    pub cont_uses: Vec<BTreeMap<u32, Span>>,
    pub info: FnInfo,
    pub calls: BTreeSet<u32>,
    pub reads: BTreeSet<Root>,
    pub writes: BTreeSet<Root>,
    pub had_error: bool,
}

pub struct DeclSlot<'a> {
    pub decl: &'a Decl,
    pub state: DeclState,
}

pub struct Checker<'a> {
    pub src: &'a str,
    pub lm: LineMap,
    pub opt: Options,
    /// language features available + enable-extensions turned on
    pub enabled: u32,
    pub diags: Vec<Diagnostic>,
    pub errors: usize,
    pub t: Types,
    pub m: Module,
    pub decls: Vec<DeclSlot<'a>>,
    pub names: BTreeMap<&'a str, usize>,
    pub fns: Vec<FnInfo>,
    pub f: Option<FnCtx>,
    /// put new expression nodes into Module::exprs even inside a function
    pub module_arena: bool,
    /// > 0: inside a short-circuited operand (type-check, do not evaluate)
    pub no_eval: u32,
    /// overrides used by each global's initializer / type, each override's initializer
    pub global_ovr: BTreeMap<u32, BTreeSet<u32>>,
    pub override_ovr: BTreeMap<u32, BTreeSet<u32>>,
    pub override_ids: BTreeMap<u16, (u32, Span)>,
    pub fn_decl: Vec<&'a ast::FnDecl>,
}

const ATTRS: [&str; 18] = [
    "align", "binding", "blend_src", "builtin", "const", "diagnostic", "group", "id", "interpolate", "invariant", "location",
    "must_use", "size", "workgroup_size", "subgroup_size", "vertex", "fragment", "compute",
];

/// the predeclared type names (no template list)
const PREDECL_TYPES: [&str; 13] = [
    "bool", "i32", "u32", "f32", "f16", "sampler", "sampler_comparison", "texture_depth_2d", "texture_depth_2d_array",
    "texture_depth_cube", "texture_depth_cube_array", "texture_depth_multisampled_2d", "texture_external",
];

const GENERATORS: [&str; 26] = [
    "vec2", "vec3", "vec4", "mat2x2", "mat2x3", "mat2x4", "mat3x2", "mat3x3", "mat3x4", "mat4x2", "mat4x3", "mat4x4", "array",
    "atomic", "ptr", "texture_1d", "texture_2d", "texture_2d_array", "texture_3d", "texture_cube", "texture_cube_array",
    "texture_multisampled_2d", "texture_storage_1d", "texture_storage_2d", "texture_storage_2d_array", "texture_storage_3d",
];

const ENUMERANTS: [&str; 9] = ["read", "write", "read_write", "function", "private", "workgroup", "uniform", "storage", "immediate"];

/// a predeclared type alias (vec3f, mat4x4h): (generator, scalar)
pub fn alias_parts(name: &str) -> Option<(&str, Scalar)> {
    if !name.is_ascii() || name.is_empty() {
        return None;
    }
    let (base, s) = name.split_at(name.len() - 1);
    let sc = match s {
        "f" => Scalar::F32,
        "h" => Scalar::F16,
        "i" => Scalar::I32,
        "u" => Scalar::U32,
        _ => return None,
    };
    let is_vec = matches!(base, "vec2" | "vec3" | "vec4");
    let is_mat = base.len() == 6 && base.starts_with("mat") && GENERATORS[3..12].contains(&base);
    if is_vec || (is_mat && matches!(sc, Scalar::F32 | Scalar::F16)) {
        Some((base, sc))
    } else {
        None
    }
}

/// what a name denotes when no declaration has it
pub fn predeclared_kind(name: &str) -> Option<&'static str> {
    if PREDECL_TYPES.contains(&name) || alias_parts(name).is_some() {
        Some("ein Typ")
    } else if GENERATORS.contains(&name) {
        Some("ein Typ-Generator")
    } else if ENUMERANTS.contains(&name) || TEXEL_FORMATS.iter().any(|f| f.0 == name) {
        Some("ein Enumerant")
    } else if crate::builtins::lookup(name).is_some() || name == "bitcast" {
        Some("eine eingebaute Funktion")
    } else {
        None
    }
}

pub fn check(src: &str, ast: &ast::Ast, opt: &Options) -> Result<Module, Vec<Diagnostic>> {
    let mut c = Checker {
        src,
        lm: LineMap::new(src),
        opt: *opt,
        enabled: opt.features & 0xffff,
        diags: Vec::new(),
        errors: 0,
        t: Types::new(),
        m: Module::default(),
        decls: Vec::new(),
        names: BTreeMap::new(),
        fns: Vec::new(),
        f: None,
        module_arena: false,
        no_eval: 0,
        global_ovr: BTreeMap::new(),
        override_ovr: BTreeMap::new(),
        override_ids: BTreeMap::new(),
        fn_decl: Vec::new(),
    };
    c.directives(ast);
    for d in &ast.decls {
        c.decls.push(DeclSlot { decl: d, state: DeclState::Pending });
    }
    c.collect_names();
    let order = c.order();
    for i in order {
        if c.errors >= MAX_ERRORS {
            break;
        }
        if matches!(c.decls[i].state, DeclState::Pending) {
            let st = match c.decl(i) {
                Ok(info) => DeclState::Done(info),
                Err(_) => DeclState::Failed,
            };
            c.decls[i].state = st;
        }
    }
    if c.errors == 0 {
        c.entry_points();
    }
    let mut diags = core::mem::take(&mut c.diags);
    diags.sort_by_key(|d| (d.start, !d.error));
    if c.errors > 0 {
        return Err(diags);
    }
    let mut m = c.m;
    m.types = c.t.list;
    m.structs = c.t.structs;
    m.warnings = diags;
    Ok(m)
}

impl<'a> Checker<'a> {
    // ---- diagnostics

    pub fn err(&mut self, sp: Span, msg: String) -> E {
        self.errors += 1;
        if self.errors <= MAX_ERRORS {
            let d = self.lm.diag(self.src, sp.start, sp.end, true, msg);
            self.diags.push(d);
        }
        if let Some(f) = self.f.as_mut() {
            f.had_error = true;
        }
        E
    }

    pub fn err_note(&mut self, sp: Span, msg: String, nsp: Span, note: String) -> E {
        let e = self.err(sp, msg);
        if self.errors <= MAX_ERRORS {
            let n = self.lm.note(self.src, nsp.start, nsp.end, note);
            if let Some(d) = self.diags.last_mut() {
                d.notes.push(n);
            }
        }
        e
    }

    pub fn warn(&mut self, sp: Span, msg: String) {
        let d = self.lm.diag(self.src, sp.start, sp.end, false, msg);
        self.diags.push(d);
    }

    pub fn has(&self, bit: u32) -> bool {
        self.enabled & bit != 0
    }

    pub fn tn(&self, t: TypeId) -> String {
        self.t.name(t)
    }

    // ---- directives

    fn directives(&mut self, ast: &ast::Ast) {
        let mut seen_filters: Vec<(String, Severity, Span)> = Vec::new();
        for d in &ast.directives {
            match &d.kind {
                ast::DirectiveKind::Enable(names) | ast::DirectiveKind::Requires(names) => {
                    let enable = matches!(d.kind, ast::DirectiveKind::Enable(_));
                    for n in names {
                        let entry = feature::NAMES.iter().find(|e| e.0 == n.name);
                        match entry {
                            None => {
                                let what = if enable { "Erweiterung" } else { "Spracherweiterung" };
                                let _ = self.err(n.span, format!("unbekannte {} `{}`", what, n.name));
                            }
                            Some(&(_, _, lang)) if lang == enable => {
                                let msg = if enable {
                                    format!("`{}` ist eine Spracherweiterung: schreibe `requires {};`", n.name, n.name)
                                } else {
                                    format!("`{}` ist keine Spracherweiterung: schreibe `enable {};`", n.name, n.name)
                                };
                                let _ = self.err(n.span, msg);
                            }
                            Some(&(_, 0, _)) => {
                                let _ = self.err(n.span, format!("`{}` wird von diesem Compiler noch nicht unterstützt", n.name));
                            }
                            Some(&(_, bit, _)) => {
                                if self.opt.features & bit == 0 {
                                    let _ = self.err(n.span, format!("`{}` ist auf diesem Gerät nicht verfügbar", n.name));
                                } else if enable {
                                    self.enabled |= bit;
                                    self.m.enables |= bit;
                                } else {
                                    self.m.requires |= bit;
                                }
                            }
                        }
                    }
                }
                ast::DirectiveKind::Diagnostic(c) => {
                    if let Ok((sev, rule)) = self.diag_control(c) {
                        if let Some(prev) = seen_filters.iter().find(|p| p.0 == rule && p.1 != sev) {
                            let psp = prev.2;
                            let _ = self.err_note(d.span, format!("widersprüchliche Diagnose-Filter für `{}`", rule), psp, String::from("der andere Filter"));
                        }
                        seen_filters.push((rule.clone(), sev, d.span));
                        self.m.diagnostic_filters.push(DiagnosticFilter { severity: sev, rule, range: Span::new(0, self.src.len() as u32), global: true });
                    }
                }
            }
        }
    }

    pub fn diag_control(&mut self, c: &ast::DiagnosticControl) -> R<(Severity, String)> {
        let sev = match c.severity.name.as_str() {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "info" => Severity::Info,
            "off" => Severity::Off,
            _ => {
                return Err(self.err(c.severity.span, format!("unbekannter Schweregrad `{}` (error, warning, info oder off)", c.severity.name)));
            }
        };
        let rule = match &c.rule2 {
            Some(r2) => format!("{}.{}", c.rule.name, r2.name),
            None => {
                if !matches!(c.rule.name.as_str(), "derivative_uniformity" | "subgroup_uniformity") {
                    self.warn(c.rule.span, format!("unbekannte Diagnose-Regel `{}`", c.rule.name));
                }
                c.rule.name.clone()
            }
        };
        Ok((sev, rule))
    }

    /// @diagnostic attributes of a function or statement: recorded with the affected range
    pub fn diag_attrs(&mut self, attrs: &[Attr], range: Span) {
        let mut rules: Vec<(String, Span)> = Vec::new();
        for a in attrs {
            if let AttrKind::Diagnostic(c) = &a.kind {
                if let Ok((sev, rule)) = self.diag_control(c) {
                    if let Some(p) = rules.iter().find(|r| r.0 == rule) {
                        let psp = p.1;
                        let _ = self.err_note(a.span, format!("zwei @diagnostic-Attribute für dieselbe Regel `{}`", rule), psp, String::from("das erste"));
                    }
                    rules.push((rule.clone(), a.span));
                    self.m.diagnostic_filters.push(DiagnosticFilter { severity: sev, rule, range, global: false });
                }
            }
        }
    }

    /// only diagnostic attributes may stand here (statements other than declarations)
    pub fn only_diag(&mut self, attrs: &[Attr], range: Span) {
        for a in attrs {
            if !matches!(a.kind, AttrKind::Diagnostic(_)) {
                let _ = self.err(a.span, format!("`@{}` ist hier nicht erlaubt (vor Anweisungen nur @diagnostic)", a.name.name));
            }
        }
        self.diag_attrs(attrs, range);
    }

    /// unknown attributes, attributes not allowed here, duplicates
    pub fn check_attrs(&mut self, attrs: &[Attr], allowed: &[&str], what: &str) -> R<()> {
        let mut ok = true;
        for (i, a) in attrs.iter().enumerate() {
            let n = a.name.name.as_str();
            if !ATTRS.contains(&n) {
                let _ = self.err(a.span, format!("unbekanntes Attribut `@{}`", n));
                ok = false;
                continue;
            }
            if !allowed.contains(&n) {
                let _ = self.err(a.span, format!("`@{}` ist an {} nicht erlaubt", n, what));
                ok = false;
                continue;
            }
            if n != "diagnostic" {
                if let Some(p) = attrs[..i].iter().find(|p| p.name.name == n) {
                    let _ = self.err_note(a.span, format!("`@{}` steht zweimal", n), p.span, String::from("das erste"));
                    ok = false;
                }
            }
            let argc = a.args().len();
            let parens = matches!(a.kind, AttrKind::Plain { parens: true, .. });
            let bad = match n {
                "const" | "invariant" | "must_use" | "vertex" | "fragment" | "compute" => parens,
                "workgroup_size" => !(1..=3).contains(&argc),
                "interpolate" | "builtin" => !(1..=2).contains(&argc),
                "diagnostic" => false,
                _ => argc != 1,
            };
            if bad {
                let _ = self.err(a.span, format!("falsche Anzahl von Argumenten für `@{}`", n));
                ok = false;
            }
        }
        if ok { Ok(()) } else { Err(E) }
    }

    pub fn attr<'b>(&self, attrs: &'b [Attr], name: &str) -> Option<&'b Attr> {
        attrs.iter().find(|a| a.name.name == name)
    }

    /// the value of a one-argument integer attribute (a const-expression of type i32 / u32)
    pub fn attr_int(&mut self, a: &Attr) -> R<i64> {
        let e = a.args().first().ok_or(E)?;
        let te = self.expr(e)?;
        let te = self.rvalue(te)?;
        if te.stage != Phase::Const {
            return Err(self.err(e.span, format!("das Argument von `@{}` muss ein const-Ausdruck sein", a.name.name)));
        }
        let te = if te.ty == AINT { self.convert(te, I32)? } else { te };
        match te.val {
            Some(Value::I32(x)) => Ok(x as i64),
            Some(Value::U32(x)) => Ok(x as i64),
            _ if te.ty == I32 || te.ty == U32 => Ok(0),
            _ => Err(self.err(e.span, format!("das Argument von `@{}` muss vom Typ i32 oder u32 sein, nicht `{}`", a.name.name, self.tn(te.ty)))),
        }
    }

    pub fn attr_nonneg(&mut self, a: &Attr) -> R<u32> {
        let v = self.attr_int(a)?;
        if v < 0 {
            let sp = a.args().first().map(|e| e.span).unwrap_or(a.span);
            return Err(self.err(sp, format!("`@{}` darf nicht negativ sein ({})", a.name.name, v)));
        }
        Ok(v as u32)
    }

    /// the name inside an attribute argument (`flat`, `position`)
    fn attr_name(&mut self, e: &Expr, what: &str) -> R<String> {
        match &e.kind {
            ExprKind::Ident(id, None) => Ok(id.name.clone()),
            _ => Err(self.err(e.span, format!("erwartet {}", what))),
        }
    }

    /// the IO attributes (builtin, location, blend_src, interpolate, invariant)
    pub fn io_attrs(&mut self, attrs: &[Attr]) -> R<Io> {
        let mut io = Io::default();
        if let Some(a) = self.attr(attrs, "builtin") {
            // (a wrong argument count is reported by check_attrs)
            let Some(a0) = a.args().first() else { return Err(E) };
            let n = self.attr_name(a0, "den Namen eines eingebauten Werts")?;
            let b = match BUILTIN_VALUES.iter().find(|b| b.0 == n) {
                Some(b) => b.1,
                None => return Err(self.err(a0.span, format!("`{}` ist kein eingebauter Wert", n))),
            };
            let need = match b {
                BuiltinValue::ClipDistances => Some((feature::CLIP_DISTANCES, "enable clip_distances;")),
                BuiltinValue::PrimitiveIndex => Some((feature::PRIMITIVE_INDEX, "enable primitive_index;")),
                BuiltinValue::GlobalInvocationIndex | BuiltinValue::WorkgroupIndex => Some((feature::LINEAR_INDEXING, "die Spracherweiterung linear_indexing")),
                BuiltinValue::SubgroupInvocationId | BuiltinValue::SubgroupSize | BuiltinValue::SubgroupId | BuiltinValue::NumSubgroups => {
                    return Err(self.err(a0.span, format!("`{}` gehört zu subgroups, das dieser Compiler noch nicht unterstützt", n)));
                }
                _ => None,
            };
            if let Some((bit, how)) = need {
                if !self.has(bit) {
                    return Err(self.err(a0.span, format!("`{}` braucht {}", n, how)));
                }
            }
            io.builtin = Some(b);
            if let Some(m) = a.args().get(1) {
                let mn = self.attr_name(m, "less oder greater")?;
                if b != BuiltinValue::FragDepth {
                    return Err(self.err(m.span, String::from("ein Tiefenmodus ist nur bei frag_depth erlaubt")));
                }
                if !self.has(feature::FRAGMENT_DEPTH) {
                    return Err(self.err(m.span, String::from("der Tiefenmodus braucht die Spracherweiterung fragment_depth")));
                }
                io.depth_mode = Some(match mn.as_str() {
                    "less" => DepthMode::Less,
                    "greater" => DepthMode::Greater,
                    _ => return Err(self.err(m.span, format!("`{}` ist kein Tiefenmodus (less oder greater)", mn))),
                });
            }
        }
        if let Some(a) = self.attr(attrs, "location") {
            io.location = Some(self.attr_nonneg(a)?);
        }
        if let Some(a) = self.attr(attrs, "blend_src") {
            if !self.has(feature::DUAL_SOURCE_BLENDING) {
                return Err(self.err(a.span, String::from("`@blend_src` braucht `enable dual_source_blending;`")));
            }
            let v = self.attr_nonneg(a)?;
            if v > 1 {
                let sp = a.args().first().map(|x| x.span).unwrap_or(a.span);
                return Err(self.err(sp, format!("`@blend_src` muss 0 oder 1 sein, nicht {}", v)));
            }
            if io.location.is_none() {
                return Err(self.err(a.span, String::from("`@blend_src` braucht auch `@location`")));
            }
            io.blend_src = Some(v);
        }
        if let Some(a) = self.attr(attrs, "interpolate") {
            let Some(a0) = a.args().first() else { return Err(E) };
            let tn = self.attr_name(a0, "perspective, linear oder flat")?;
            let ty = match tn.as_str() {
                "perspective" => InterpType::Perspective,
                "linear" => InterpType::Linear,
                "flat" => InterpType::Flat,
                _ => return Err(self.err(a0.span, format!("`{}` ist keine Interpolationsart (perspective, linear, flat)", tn))),
            };
            let mut samp = None;
            if let Some(s) = a.args().get(1) {
                let sn = self.attr_name(s, "eine Abtastung")?;
                let sv = match sn.as_str() {
                    "center" => InterpSampling::Center,
                    "centroid" => InterpSampling::Centroid,
                    "sample" => InterpSampling::Sample,
                    "first" => InterpSampling::First,
                    "either" => InterpSampling::Either,
                    _ => return Err(self.err(s.span, format!("`{}` ist keine Abtastung (center, centroid, sample, first, either)", sn))),
                };
                let ok = if ty == InterpType::Flat {
                    matches!(sv, InterpSampling::First | InterpSampling::Either)
                } else {
                    matches!(sv, InterpSampling::Center | InterpSampling::Centroid | InterpSampling::Sample)
                };
                if !ok {
                    let allowed = if ty == InterpType::Flat { "first oder either" } else { "center, centroid oder sample" };
                    return Err(self.err(s.span, format!("`{}` passt nicht zu `{}` (erlaubt: {})", sn, tn, allowed)));
                }
                samp = Some(sv);
            }
            if io.location.is_none() {
                return Err(self.err(a.span, String::from("`@interpolate` ist nur zusammen mit `@location` erlaubt")));
            }
            io.interpolate = Some((ty, samp));
        }
        if let Some(a) = self.attr(attrs, "invariant") {
            if io.builtin != Some(BuiltinValue::Position) {
                return Err(self.err(a.span, String::from("`@invariant` ist nur an `@builtin(position)` erlaubt")));
            }
            io.invariant = true;
        }
        Ok(io)
    }

    // ---- names and order

    fn collect_names(&mut self) {
        for i in 0..self.decls.len() {
            let d = self.decls[i].decl;
            if let Some(id) = d.name() {
                let name: &'a str = &id.name;
                if let Some(&p) = self.names.get(name) {
                    let psp = self.decls[p].decl.name().map(|n| n.span).unwrap_or_default();
                    let _ = self.err_note(id.span, format!("`{}` ist schon deklariert", name), psp, String::from("hier zuerst"));
                    self.decls[i].state = DeclState::Failed;
                } else {
                    self.names.insert(name, i);
                }
            }
        }
    }

    /// the module declarations in dependency order (cycles reported, their members failed)
    fn order(&mut self) -> Vec<usize> {
        let n = self.decls.len();
        let mut deps: Vec<Vec<usize>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut w = DepWalk { names: &self.names, scopes: Vec::new(), out: Vec::new() };
            w.decl(self.decls[i].decl);
            let mut v = w.out;
            v.sort_unstable();
            v.dedup();
            deps.push(v);
        }
        // 0 white, 1 on the stack, 2 done
        let mut color = alloc::vec![0u8; n];
        let mut order = Vec::with_capacity(n);
        let mut in_cycle = alloc::vec![false; n];
        for root in 0..n {
            if color[root] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, usize)> = alloc::vec![(root, 0)];
            color[root] = 1;
            while let Some(&mut (v, ref mut k)) = stack.last_mut() {
                if *k < deps[v].len() {
                    let w = deps[v][*k];
                    *k += 1;
                    if color[w] == 0 {
                        color[w] = 1;
                        stack.push((w, 0));
                    } else if color[w] == 1 {
                        // a cycle: the stack from w to v
                        let pos = stack.iter().position(|e| e.0 == w).unwrap_or(0);
                        let members: Vec<usize> = stack[pos..].iter().map(|e| e.0).collect();
                        if !members.iter().any(|&m| in_cycle[m]) {
                            self.report_cycle(&members);
                        }
                        for &m in &members {
                            in_cycle[m] = true;
                        }
                    }
                } else {
                    color[v] = 2;
                    order.push(v);
                    stack.pop();
                }
            }
        }
        for (i, c) in in_cycle.iter().enumerate() {
            if *c {
                self.decls[i].state = DeclState::Failed;
            }
        }
        order
    }

    fn report_cycle(&mut self, members: &[usize]) {
        let names: Vec<String> = members.iter().map(|&m| self.decls[m].decl.name().map(|n| n.name.clone()).unwrap_or_default()).collect();
        let first = self.decls[members[0]].decl.name().map(|n| n.span).unwrap_or_default();
        let msg = if members.len() == 1 {
            let d = self.decls[members[0]].decl;
            if matches!(d.kind, DeclKind::Fn(_)) {
                format!("die Funktion `{}` ruft sich selbst auf (Rekursion ist in WGSL verboten)", names[0])
            } else {
                format!("`{}` hängt von sich selbst ab", names[0])
            }
        } else {
            let shown: Vec<&str> = names.iter().take(6).map(|s| s.as_str()).collect();
            let mut s = shown.join("` → `");
            if names.len() > 6 {
                s.push_str(&format!("` → … ({} weitere) → `", names.len() - 6));
            } else {
                s.push_str("` → `");
            }
            s.push_str(&names[0]);
            format!("zyklische Abhängigkeit: `{}`", s)
        };
        let _ = self.err(first, msg);
        for &m in members[1..].iter().take(6) {
            if let Some(n) = self.decls[m].decl.name() {
                let note = self.lm.note(self.src, n.span.start, n.span.end, format!("`{}` gehört zum Zyklus", n.name));
                if let Some(d) = self.diags.last_mut() {
                    d.notes.push(note);
                }
            }
        }
    }

    /// look a name up: local, module-scope declaration, or None (predeclared or unknown)
    pub fn lookup_local(&mut self, name: &str) -> Option<(LocalRef, u32, Span)> {
        let f = self.f.as_ref()?;
        for s in f.scopes.iter().rev() {
            if let Some(e) = s.iter().rev().find(|e| e.0 == name) {
                return Some((e.1.clone(), e.2, e.3));
            }
        }
        None
    }

    /// a module declaration by name: Ok(None) when the name is not declared, Err when the
    /// declaration failed
    pub fn lookup_decl(&self, name: &str) -> Result<Option<(usize, DeclInfo)>, E> {
        match self.names.get(name) {
            None => Ok(None),
            Some(&i) => match &self.decls[i].state {
                DeclState::Done(info) => Ok(Some((i, info.clone()))),
                _ => Err(E),
            },
        }
    }

    pub fn decl_span(&self, i: usize) -> Span {
        self.decls[i].decl.name().map(|n| n.span).unwrap_or_default()
    }

    /// a name that must be predeclared here (enumerants): None when a declaration shadows it
    pub fn enumerant(&mut self, e: &Expr, what: &str) -> R<String> {
        if let ExprKind::Ident(id, None) = &e.kind {
            let shadowed = self.lookup_local(&id.name).is_some() || self.names.contains_key(id.name.as_str());
            if !shadowed {
                return Ok(id.name.clone());
            }
            return Err(self.err(e.span, format!("`{}` ist hier {} – der Name wird von einer Deklaration verdeckt", id.name, what)));
        }
        Err(self.err(e.span, format!("erwartet {}", what)))
    }

    // ---- types

    pub fn ty(&mut self, e: &Expr) -> R<TypeId> {
        match &e.kind {
            ExprKind::Paren(x) => self.ty(x),
            ExprKind::Ident(id, targs) => {
                if let Some((_, _, sp)) = self.lookup_local(&id.name) {
                    return Err(self.err_note(e.span, format!("`{}` ist kein Typ", id.name), sp, String::from("hier deklariert")));
                }
                match self.lookup_decl(&id.name)? {
                    Some((i, info)) => {
                        let t = match info {
                            DeclInfo::Alias(t) | DeclInfo::Struct(t) => t,
                            _ => {
                                let sp = self.decl_span(i);
                                return Err(self.err_note(e.span, format!("`{}` ist kein Typ", id.name), sp, String::from("hier deklariert")));
                            }
                        };
                        if targs.is_some() {
                            return Err(self.err(e.span, format!("`{}` hat keine Template-Parameter", id.name)));
                        }
                        Ok(t)
                    }
                    None => self.predeclared_type(&id.name, targs.as_deref(), e.span),
                }
            }
            ExprKind::Error => Err(E),
            _ => Err(self.err(e.span, String::from("erwartet einen Typ"))),
        }
    }

    fn need_f16(&mut self, sp: Span) -> R<()> {
        if !self.has(feature::F16) {
            return Err(self.err(sp, String::from("der Typ f16 braucht `enable f16;`")));
        }
        Ok(())
    }

    fn scalar_arg(&mut self, args: &[Expr], sp: Span, generator: &str) -> R<TypeId> {
        if args.len() != 1 {
            return Err(self.err(sp, format!("`{}` braucht genau einen Template-Parameter", generator)));
        }
        self.ty(&args[0])
    }

    fn predeclared_type(&mut self, name: &str, targs: Option<&[Expr]>, sp: Span) -> R<TypeId> {
        let no_targs = |c: &mut Self| -> R<()> {
            if targs.is_some() {
                return Err(c.err(sp, format!("`{}` hat keine Template-Parameter", name)));
            }
            Ok(())
        };
        match name {
            "bool" | "i32" | "u32" | "f32" | "f16" => {
                no_targs(self)?;
                if name == "f16" {
                    self.need_f16(sp)?;
                }
                return Ok(match name {
                    "bool" => BOOL,
                    "i32" => I32,
                    "u32" => U32,
                    "f32" => F32,
                    _ => F16,
                });
            }
            "sampler" | "sampler_comparison" => {
                no_targs(self)?;
                return Ok(self.t.intern(Type::Sampler { comparison: name == "sampler_comparison" }));
            }
            "texture_external" => {
                no_targs(self)?;
                return Ok(self.t.intern(Type::Texture(TexDim::D2, TexKind::External)));
            }
            "texture_depth_multisampled_2d" => {
                no_targs(self)?;
                return Ok(self.t.intern(Type::Texture(TexDim::D2, TexKind::DepthMultisampled)));
            }
            _ => {}
        }
        if let Some(rest) = name.strip_prefix("texture_depth_") {
            let dim = match rest {
                "2d" => TexDim::D2,
                "2d_array" => TexDim::D2Array,
                "cube" => TexDim::Cube,
                "cube_array" => TexDim::CubeArray,
                _ => return Err(self.unknown_type(name, sp)),
            };
            no_targs(self)?;
            return Ok(self.t.intern(Type::Texture(dim, TexKind::Depth)));
        }
        if let Some((base, s)) = alias_parts(name) {
            no_targs(self)?;
            if s == Scalar::F16 {
                self.need_f16(sp)?;
            }
            let n = base.as_bytes()[3] - b'0';
            if base.starts_with("vec") {
                return Ok(self.t.vec(n, s));
            }
            let r = base.as_bytes()[5] - b'0';
            return Ok(self.t.mat(n, r, s));
        }
        if !GENERATORS.contains(&name) {
            return Err(self.unknown_type(name, sp));
        }
        let Some(args) = targs else {
            return Err(self.err(sp, format!("`{}` braucht Template-Parameter (z. B. `{}<f32>`)", name, name)));
        };
        if name.starts_with("vec") {
            let t = self.scalar_arg(args, sp, name)?;
            let n = name.as_bytes()[3] - b'0';
            return match self.t.get(t) {
                Type::Scalar(s) => Ok(self.t.vec(n, s)),
                _ => Err(self.err(args[0].span, format!("die Komponenten eines Vektors müssen Skalare sein, nicht `{}`", self.tn(t)))),
            };
        }
        if name.starts_with("mat") {
            let t = self.scalar_arg(args, sp, name)?;
            let (c, r) = (name.as_bytes()[3] - b'0', name.as_bytes()[5] - b'0');
            return match self.t.get(t) {
                Type::Scalar(s @ (Scalar::F32 | Scalar::F16)) => Ok(self.t.mat(c, r, s)),
                _ => Err(self.err(args[0].span, format!("die Komponenten einer Matrix müssen f32 oder f16 sein, nicht `{}`", self.tn(t)))),
            };
        }
        match name {
            "array" => self.array_type(args, sp),
            "atomic" => {
                let t = self.scalar_arg(args, sp, name)?;
                let ok = t == I32 || t == U32 || (t == self.t.vec(2, Scalar::U32) && self.has(feature::ATOMIC_VEC2U_MIN_MAX));
                if !ok {
                    return Err(self.err(args[0].span, format!("atomic<{}> ist nicht erlaubt (nur i32 oder u32)", self.tn(t))));
                }
                Ok(self.t.intern(Type::Atomic(t)))
            }
            "ptr" => self.ptr_type(args, sp),
            _ => self.texture_type(name, args, sp),
        }
    }

    fn unknown_type(&mut self, name: &str, sp: Span) -> E {
        match predeclared_kind(name) {
            Some(k) if k != "ein Typ" && k != "ein Typ-Generator" => self.err(sp, format!("`{}` ist {}, kein Typ", name, k)),
            _ => self.err(sp, format!("unbekannter Typ `{}`", name)),
        }
    }

    fn array_type(&mut self, args: &[Expr], sp: Span) -> R<TypeId> {
        if args.is_empty() || args.len() > 2 {
            return Err(self.err(sp, String::from("`array` braucht einen Elementtyp und optional eine Anzahl")));
        }
        let elem = self.ty(&args[0])?;
        let esp = args[0].span;
        if !self.t.is_plain(elem) {
            return Err(self.err(esp, format!("`{}` kann kein Array-Element sein", self.tn(elem))));
        }
        if !self.t.creation_fixed(elem) {
            return Err(self.err(esp, format!("das Element `{}` hat keine feste Größe", self.tn(elem))));
        }
        let count = match args.get(1) {
            None => ArrayCount::Runtime,
            Some(ce) => self.array_count(ce)?,
        };
        let t = self.t.array(elem, count);
        if let (ArrayCount::Fixed(n), Type::Array { stride, .. }) = (count, self.t.get(t)) {
            if n as u64 * stride as u64 > u32::MAX as u64 {
                return Err(self.err(sp, format!("`{}` ist größer als 4 GiB", self.tn(t))));
            }
        }
        Ok(t)
    }

    fn array_count(&mut self, ce: &Expr) -> R<ArrayCount> {
        let saved = self.module_arena;
        // an array size is evaluated even inside a short-circuited operand (it types it)
        let saved_ne = self.no_eval;
        self.module_arena = true;
        self.no_eval = 0;
        let r = (|| {
            let te = self.expr(ce)?;
            let te = self.rvalue(te)?;
            if te.stage == Phase::Runtime {
                return Err(self.err(ce.span, String::from("die Anzahl der Elemente muss ein const- oder override-Ausdruck sein")));
            }
            if te.ty != I32 && te.ty != U32 && te.ty != AINT {
                return Err(self.err(ce.span, format!("die Anzahl der Elemente muss i32 oder u32 sein, nicht `{}`", self.tn(te.ty))));
            }
            let te = if te.ty == AINT { self.convert(te, I32)? } else { te };
            if te.stage == Phase::Const {
                let n = match te.val {
                    Some(Value::I32(x)) => x as i64,
                    Some(Value::U32(x)) => x as i64,
                    _ => 1,
                };
                if n <= 0 {
                    return Err(self.err(ce.span, format!("die Anzahl der Elemente muss größer als 0 sein, nicht {}", n)));
                }
                return Ok(ArrayCount::Fixed(n as u32));
            }
            // an override: the same declaration gives the same type
            if let ExprKind::Ident(id, None) = &ce.kind {
                if let Ok(Some((_, DeclInfo::Override(o)))) = self.lookup_decl(&id.name) {
                    return Ok(ArrayCount::Override(o));
                }
            }
            let id = self.node(&te);
            Ok(ArrayCount::Expr(id))
        })();
        self.module_arena = saved;
        self.no_eval = saved_ne;
        r
    }

    /// the default access mode of an address space
    pub fn default_access(space: AddressSpace) -> Access {
        match space {
            AddressSpace::Uniform | AddressSpace::Storage | AddressSpace::Immediate | AddressSpace::Handle => Access::Read,
            _ => Access::ReadWrite,
        }
    }

    pub fn address_space(&mut self, e: &Expr) -> R<AddressSpace> {
        let n = self.enumerant(e, "ein Adressraum")?;
        Ok(match n.as_str() {
            "function" => AddressSpace::Function,
            "private" => AddressSpace::Private,
            "workgroup" => AddressSpace::Workgroup,
            "uniform" => AddressSpace::Uniform,
            "storage" => AddressSpace::Storage,
            "immediate" if self.has(feature::IMMEDIATE_ADDRESS_SPACE) => AddressSpace::Immediate,
            _ => return Err(self.err(e.span, format!("`{}` ist kein Adressraum (function, private, workgroup, uniform, storage)", n))),
        })
    }

    pub fn access_mode(&mut self, e: &Expr) -> R<Access> {
        let n = self.enumerant(e, "ein Zugriffsmodus")?;
        Ok(match n.as_str() {
            "read" => Access::Read,
            "write" => Access::Write,
            "read_write" => Access::ReadWrite,
            _ => return Err(self.err(e.span, format!("`{}` ist kein Zugriffsmodus (read, write, read_write)", n))),
        })
    }

    fn ptr_type(&mut self, args: &[Expr], sp: Span) -> R<TypeId> {
        if args.len() < 2 || args.len() > 3 {
            return Err(self.err(sp, String::from("`ptr` braucht Adressraum, Typ und optional den Zugriffsmodus")));
        }
        let space = self.address_space(&args[0])?;
        let store = self.ty(&args[1])?;
        let access = match args.get(2) {
            Some(a) => {
                if space != AddressSpace::Storage {
                    return Err(self.err(a.span, format!("im Adressraum `{}` darf kein Zugriffsmodus stehen", space.name())));
                }
                let m = self.access_mode(a)?;
                if m == Access::Write {
                    return Err(self.err(a.span, String::from("Speicherpuffer erlauben nur read oder read_write")));
                }
                m
            }
            None => Self::default_access(space),
        };
        self.store_type_ok(space, access, store, args[1].span)?;
        Ok(self.t.ptr(space, store, access))
    }

    /// whether a variable of this address space may have this store type
    pub fn store_type_ok(&mut self, space: AddressSpace, access: Access, t: TypeId, sp: Span) -> R<()> {
        let name = self.tn(t);
        let t_ = &self.t;
        let bad = |c: &mut Self, why: &str| -> R<()> { Err(c.err(sp, format!("`{}` ist im Adressraum `{}` nicht erlaubt: {}", name, space.name(), why))) };
        match space {
            AddressSpace::Function | AddressSpace::Private => {
                if !t_.is_constructible(t) || t_.is_abstract(t) {
                    return bad(self, "der Typ muss konstruierbar sein (keine Atomics, keine Arrays ohne feste Größe)");
                }
            }
            AddressSpace::Workgroup => {
                if !t_.is_plain(t) || !t_.fixed_footprint(t) {
                    return bad(self, "der Typ braucht eine feste Größe");
                }
            }
            AddressSpace::Uniform | AddressSpace::Immediate => {
                if !t_.is_constructible(t) || !t_.is_host_shareable(t) {
                    return bad(self, "der Typ muss konstruierbar und host-teilbar sein (kein bool, keine Atomics, keine Laufzeit-Arrays)");
                }
                if space == AddressSpace::Immediate && t_.contains_array(t) {
                    return bad(self, "Immediate-Daten dürfen keine Arrays enthalten");
                }
                let strict = space == AddressSpace::Uniform && !self.has(feature::UNIFORM_BUFFER_STANDARD_LAYOUT);
                if let Some((msg, msp)) = self.t.layout_error(t, strict) {
                    let s = if msp == Span::default() { sp } else { msp };
                    return Err(self.err_note(sp, msg, s, String::from("hier")));
                }
            }
            AddressSpace::Storage => {
                if !t_.is_host_shareable(t) {
                    return bad(self, "der Typ muss host-teilbar sein (kein bool, keine Texturen)");
                }
                if t_.contains_atomic(t) && access != Access::ReadWrite {
                    return bad(self, "Atomics brauchen den Zugriffsmodus read_write");
                }
                if let Some((msg, msp)) = self.t.layout_error(t, false) {
                    let s = if msp == Span::default() { sp } else { msp };
                    return Err(self.err_note(sp, msg, s, String::from("hier")));
                }
            }
            AddressSpace::Handle => {}
        }
        if space != AddressSpace::Storage && space != AddressSpace::Workgroup && self.t.contains_atomic(t) {
            return Err(self.err(sp, format!("Atomics gibt es nur in storage (read_write) und workgroup, nicht in `{}`", space.name())));
        }
        if self.t.contains_runtime_array(t) && space != AddressSpace::Storage {
            return Err(self.err(sp, format!("Laufzeit-Arrays gibt es nur im Adressraum storage, nicht in `{}`", space.name())));
        }
        Ok(())
    }

    fn texture_type(&mut self, name: &str, args: &[Expr], sp: Span) -> R<TypeId> {
        let rest = name.strip_prefix("texture_").unwrap_or(name);
        if let Some(d) = rest.strip_prefix("storage_") {
            let dim = match d {
                "1d" => TexDim::D1,
                "2d" => TexDim::D2,
                "2d_array" => TexDim::D2Array,
                _ => TexDim::D3,
            };
            if args.len() != 2 {
                return Err(self.err(sp, format!("`{}` braucht Format und Zugriffsmodus", name)));
            }
            let fname = self.enumerant(&args[0], "ein Texelformat")?;
            let Some(&(_, fmt, tier1)) = TEXEL_FORMATS.iter().find(|f| f.0 == fname) else {
                return Err(self.err(args[0].span, format!("`{}` ist kein Texelformat für Speichertexturen", fname)));
            };
            if tier1 && !self.has(feature::TEXTURE_FORMATS_TIER1) {
                return Err(self.err(args[0].span, format!("das Format `{}` braucht die Spracherweiterung texture_formats_tier1", fname)));
            }
            let acc = self.access_mode(&args[1])?;
            if acc != Access::Write && !self.has(feature::READONLY_AND_READWRITE_STORAGE_TEXTURES) {
                return Err(self.err(args[1].span, String::from("Speichertexturen mit read oder read_write brauchen readonly_and_readwrite_storage_textures")));
            }
            return Ok(self.t.intern(Type::Texture(dim, TexKind::Storage(fmt, acc))));
        }
        let t = self.scalar_arg(args, sp, name)?;
        let s = match self.t.get(t) {
            Type::Scalar(s @ (Scalar::F32 | Scalar::I32 | Scalar::U32)) => s,
            _ => return Err(self.err(args[0].span, format!("der Abtasttyp einer Textur muss f32, i32 oder u32 sein, nicht `{}`", self.tn(t)))),
        };
        let (dim, kind) = match rest {
            "1d" => (TexDim::D1, TexKind::Sampled(s)),
            "2d" => (TexDim::D2, TexKind::Sampled(s)),
            "2d_array" => (TexDim::D2Array, TexKind::Sampled(s)),
            "3d" => (TexDim::D3, TexKind::Sampled(s)),
            "cube" => (TexDim::Cube, TexKind::Sampled(s)),
            "cube_array" => (TexDim::CubeArray, TexKind::Sampled(s)),
            _ => (TexDim::D2, TexKind::Multisampled(s)),
        };
        Ok(self.t.intern(Type::Texture(dim, kind)))
    }

    // ---- declarations

    fn decl(&mut self, i: usize) -> R<DeclInfo> {
        let d = self.decls[i].decl;
        match &d.kind {
            DeclKind::Alias { name, ty } => {
                let t = self.ty(ty)?;
                let _ = name;
                Ok(DeclInfo::Alias(t))
            }
            DeclKind::Struct { name, members } => self.struct_decl(name, members, d.span).map(DeclInfo::Struct),
            DeclKind::Var(v) => match v.kind {
                VarKind::Var => self.global_var(v).map(DeclInfo::Var),
                VarKind::Const => self.module_const(v),
                VarKind::Override => self.override_decl(v).map(DeclInfo::Override),
                VarKind::Let => Err(self.err(v.span, String::from("`let` ist nur in Funktionen erlaubt (auf Modulebene: const oder override)"))),
            },
            DeclKind::Fn(f) => self.function(f, d.span).map(DeclInfo::Fn),
            DeclKind::ConstAssert(e) => {
                self.const_assert(e)?;
                Ok(DeclInfo::Assert)
            }
        }
    }

    pub fn const_assert(&mut self, e: &Expr) -> R<()> {
        let te = self.expr(e)?;
        let te = self.rvalue(te)?;
        if te.stage != Phase::Const {
            return Err(self.err(e.span, String::from("`const_assert` braucht einen const-Ausdruck")));
        }
        if te.ty != BOOL {
            return Err(self.err(e.span, format!("`const_assert` braucht einen bool-Ausdruck, nicht `{}`", self.tn(te.ty))));
        }
        if te.val == Some(Value::Bool(false)) {
            return Err(self.err(e.span, String::from("const_assert schlägt fehl: der Ausdruck ist false")));
        }
        Ok(())
    }

    fn struct_decl(&mut self, name: &ast::Ident, members: &[ast::Member], span: Span) -> R<TypeId> {
        if members.is_empty() {
            return Err(self.err(name.span, format!("die Struktur `{}` braucht mindestens ein Member", name.name)));
        }
        let mut ms: Vec<StructMember> = Vec::new();
        let mut ok = true;
        let mut offset = 0u32;
        let mut align = 1u32;
        let mut runtime = false;
        for (k, m) in members.iter().enumerate() {
            if self.check_attrs(&m.attrs, &["align", "size", "builtin", "location", "blend_src", "interpolate", "invariant"], "Struct-Membern").is_err() {
                ok = false;
                continue;
            }
            if let Some(p) = ms.iter().find(|p| p.name == m.name.name) {
                let psp = p.span;
                let _ = self.err_note(m.name.span, format!("das Member `{}` gibt es schon", m.name.name), psp, String::from("hier zuerst"));
                ok = false;
                continue;
            }
            let Ok(t) = self.ty(&m.ty) else {
                ok = false;
                continue;
            };
            let tname = self.tn(t);
            let last = k + 1 == members.len();
            let valid = match self.t.get(t) {
                Type::Scalar(_) | Type::Vector(..) | Type::Matrix(..) | Type::Atomic(_) => true,
                Type::Array { count: ArrayCount::Runtime, .. } => {
                    if !last {
                        let _ = self.err(m.ty.span, String::from("ein Array ohne feste Größe darf nur das letzte Member sein"));
                        ok = false;
                        continue;
                    }
                    runtime = true;
                    true
                }
                Type::Array { count: ArrayCount::Fixed(_), .. } => true,
                Type::Array { .. } => {
                    let _ = self.err(m.ty.span, String::from("Arrays mit override-Größe sind in Strukturen nicht erlaubt"));
                    ok = false;
                    continue;
                }
                Type::Struct(_) => {
                    if !self.t.creation_fixed(t) {
                        let _ = self.err(m.ty.span, format!("die Struktur `{}` hat ein Laufzeit-Array und kann kein Member sein", tname));
                        ok = false;
                        continue;
                    }
                    true
                }
                _ => false,
            };
            if !valid {
                let _ = self.err(m.ty.span, format!("`{}` kann kein Struct-Member sein", tname));
                ok = false;
                continue;
            }
            let Ok(io) = self.io_attrs(&m.attrs) else {
                ok = false;
                continue;
            };
            let (talign, tsize) = self.t.layout(t).unwrap_or((4, 4));
            let mut malign = talign;
            if let Some(a) = self.attr(&m.attrs, "align") {
                let Ok(v) = self.attr_int(a) else {
                    ok = false;
                    continue;
                };
                let asp = a.args()[0].span;
                if v <= 0 || (v & (v - 1)) != 0 {
                    let _ = self.err(asp, format!("`@align` muss eine positive Zweierpotenz sein, nicht {}", v));
                    ok = false;
                    continue;
                }
                if v as u32 % talign != 0 {
                    let _ = self.err(asp, format!("`@align({})` ist kein Vielfaches der Ausrichtung {} von `{}`", v, talign, tname));
                    ok = false;
                    continue;
                }
                malign = v as u32;
            }
            let mut msize = tsize;
            if let Some(a) = self.attr(&m.attrs, "size") {
                let Ok(v) = self.attr_int(a) else {
                    ok = false;
                    continue;
                };
                let asp = a.args()[0].span;
                if runtime && last {
                    let _ = self.err(asp, String::from("`@size` ist an einem Laufzeit-Array nicht erlaubt"));
                    ok = false;
                    continue;
                }
                if v < tsize as i64 {
                    let _ = self.err(asp, format!("`@size({})` ist kleiner als die Größe {} von `{}`", v, tsize, tname));
                    ok = false;
                    continue;
                }
                if v > i32::MAX as i64 {
                    let _ = self.err(asp, String::from("`@size` ist zu groß"));
                    ok = false;
                    continue;
                }
                msize = v as u32;
            }
            offset = round_up(malign, offset);
            ms.push(StructMember { name: m.name.name.clone(), ty: t, offset, size: msize, align: malign, io, span: m.span });
            offset = offset.saturating_add(msize);
            align = align.max(malign);
        }
        if !ok {
            return Err(E);
        }
        // locations must be unique (or form one dual-source pair)
        let locs: Vec<&StructMember> = ms.iter().filter(|m| m.io.location.is_some()).collect();
        if locs.iter().any(|m| m.io.blend_src.is_some()) {
            let pair = locs.len() == 2
                && locs.iter().all(|m| m.io.location == Some(0))
                && locs.iter().any(|m| m.io.blend_src == Some(0))
                && locs.iter().any(|m| m.io.blend_src == Some(1))
                && locs[0].ty == locs[1].ty;
            if !pair {
                return Err(self.err(name.span, String::from("mit `@blend_src` braucht die Struktur genau zwei Member: @location(0) @blend_src(0) und @location(0) @blend_src(1) gleichen Typs")));
            }
        } else {
            for (k, m) in locs.iter().enumerate() {
                if let Some(p) = locs[..k].iter().find(|p| p.io.location == m.io.location) {
                    let (msp, psp) = (m.span, p.span);
                    return Err(self.err_note(msp, format!("`@location({})` steht zweimal in `{}`", m.io.location.unwrap_or(0), name.name), psp, String::from("das erste")));
                }
            }
        }
        let s = Struct { name: name.name.clone(), members: ms, align, size: round_up(align, offset), runtime_sized: runtime, builtin: None, span };
        Ok(self.t.add_struct(s))
    }

    fn global_var(&mut self, v: &VarDecl) -> R<u32> {
        self.check_attrs(&v.attrs, &["group", "binding"], "Variablen")?;
        let (mut space, mut access) = (None, None);
        if let Some(tl) = &v.templ {
            if tl.is_empty() || tl.len() > 2 {
                return Err(self.err(v.span, String::from("`var<…>` nimmt einen Adressraum und optional einen Zugriffsmodus")));
            }
            let s = self.address_space(&tl[0])?;
            space = Some(s);
            if let Some(a) = tl.get(1) {
                if s != AddressSpace::Storage {
                    return Err(self.err(a.span, format!("im Adressraum `{}` darf kein Zugriffsmodus stehen", s.name())));
                }
                let m = self.access_mode(a)?;
                if m == Access::Write {
                    return Err(self.err(a.span, String::from("Speicherpuffer erlauben nur read oder read_write")));
                }
                access = Some(m);
            }
        }
        let declared = match &v.ty {
            Some(t) => Some(self.ty(t)?),
            None => None,
        };
        let space = match space {
            Some(AddressSpace::Function) => {
                return Err(self.err(v.span, String::from("`var<function>` ist nur in Funktionen erlaubt")));
            }
            Some(s) => {
                if declared.is_some_and(|t| self.t.is_handle(t)) {
                    return Err(self.err(v.span, String::from("Texturen und Sampler werden ohne Adressraum deklariert (`var t: texture_2d<f32>;`)")));
                }
                s
            }
            None => {
                if declared.is_some_and(|t| self.t.is_handle(t)) {
                    AddressSpace::Handle
                } else {
                    return Err(self.err(v.name.span, format!("der Variable `{}` fehlt der Adressraum (z. B. `var<private> {}`)", v.name.name, v.name.name)));
                }
            }
        };
        let access = access.unwrap_or(Self::default_access(space));
        // the initializer (private only)
        let mut init_te = None;
        if let Some(ie) = &v.init {
            if space != AddressSpace::Private {
                return Err(self.err(ie.span, format!("Variablen im Adressraum `{}` haben keinen Initialisierer", space.name())));
            }
            let te = self.expr(ie)?;
            let te = self.rvalue(te)?;
            if te.stage == Phase::Runtime {
                return Err(self.err(ie.span, String::from("der Initialisierer einer Modulvariable muss ein const- oder override-Ausdruck sein")));
            }
            init_te = Some(te);
        }
        let ty = match (declared, &init_te) {
            (Some(t), _) => t,
            (None, Some(te)) => self.t.concretize(te.ty),
            (None, None) => return Err(self.err(v.name.span, format!("`{}` braucht einen Typ oder einen Initialisierer", v.name.name))),
        };
        let tsp = v.ty.as_ref().map(|t| t.span).unwrap_or(v.name.span);
        if space != AddressSpace::Handle {
            self.store_type_ok(space, access, ty, tsp)?;
        }
        let init = match init_te {
            Some(te) => {
                let saved = self.module_arena;
                self.module_arena = true;
                let r = self.convert(te, ty);
                let r = r.map(|te| self.node(&te));
                self.module_arena = saved;
                Some(r?)
            }
            None => None,
        };
        // bindings
        let group = self.attr(&v.attrs, "group");
        let binding = self.attr(&v.attrs, "binding");
        let resource = matches!(space, AddressSpace::Uniform | AddressSpace::Storage | AddressSpace::Handle);
        let bind = if resource {
            match (group, binding) {
                (Some(g), Some(b)) => Some(Binding { group: self.attr_nonneg(g)?, binding: self.attr_nonneg(b)? }),
                _ => return Err(self.err(v.name.span, format!("die Ressource `{}` braucht `@group(…)` und `@binding(…)`", v.name.name))),
            }
        } else {
            if let Some(a) = group.or(binding) {
                return Err(self.err(a.span, format!("`@{}` ist nur an Ressourcen (uniform, storage, Texturen, Sampler) erlaubt", a.name.name)));
            }
            None
        };
        let std140 = space != AddressSpace::Uniform || self.t.layout_error(ty, true).is_none();
        let gi = self.m.globals.len() as u32;
        // overrides in the initializer and in override-sized array counts
        let mut ovr = BTreeSet::new();
        if let Some(i) = init {
            collect_overrides(&self.m.exprs, i, &mut ovr);
        }
        if let Type::Array { count, .. } = self.t.get(ty) {
            match count {
                ArrayCount::Override(o) => {
                    ovr.insert(o);
                }
                ArrayCount::Expr(e) => collect_overrides(&self.m.exprs, e, &mut ovr),
                _ => {}
            }
        }
        if !ovr.is_empty() {
            self.global_ovr.insert(gi, ovr);
        }
        if let Type::Array { count: ArrayCount::Override(_) | ArrayCount::Expr(_), .. } = self.t.get(ty) {
            if space != AddressSpace::Workgroup {
                return Err(self.err(tsp, String::from("Arrays mit override-Größe gibt es nur im Adressraum workgroup")));
            }
        }
        self.m.globals.push(Global { name: v.name.name.clone(), space, access, ty, binding: bind, init, std140, span: v.span });
        Ok(gi)
    }

    fn module_const(&mut self, v: &VarDecl) -> R<DeclInfo> {
        self.check_attrs(&v.attrs, &[], "const-Deklarationen")?;
        let (ty, val) = self.const_value(v)?;
        self.m.consts.push(ir::Const { name: v.name.name.clone(), ty, value: val.clone(), span: v.span });
        Ok(DeclInfo::Const(ty, val))
    }

    /// the type and value of a const declaration (module or function scope)
    pub fn const_value(&mut self, v: &VarDecl) -> R<(TypeId, Value)> {
        let ie = v.init.as_ref().ok_or(E)?;
        let te = self.expr(ie)?;
        let te = self.rvalue(te)?;
        if te.stage != Phase::Const {
            return Err(self.err(ie.span, format!("der Initialisierer von `const {}` muss ein const-Ausdruck sein", v.name.name)));
        }
        let te = match &v.ty {
            Some(t) => {
                let t = self.ty(t)?;
                self.convert(te, t)?
            }
            None => te,
        };
        if !self.t.is_constructible(te.ty) {
            return Err(self.err(ie.span, format!("eine Konstante kann nicht vom Typ `{}` sein", self.tn(te.ty))));
        }
        Ok((te.ty, te.val.unwrap_or(Value::Zero)))
    }

    fn override_decl(&mut self, v: &VarDecl) -> R<u32> {
        self.check_attrs(&v.attrs, &["id"], "override-Deklarationen")?;
        let declared = match &v.ty {
            Some(t) => Some(self.ty(t)?),
            None => None,
        };
        let saved = self.module_arena;
        self.module_arena = true;
        let r = (|| {
            let mut init = None;
            let mut ty = declared;
            if let Some(ie) = &v.init {
                let te = self.expr(ie)?;
                let te = self.rvalue(te)?;
                if te.stage == Phase::Runtime {
                    return Err(self.err(ie.span, String::from("der Initialisierer eines override muss ein const- oder override-Ausdruck sein")));
                }
                let t = match ty {
                    Some(t) => t,
                    None => self.t.concretize(te.ty),
                };
                ty = Some(t);
                let te = self.convert(te, t)?;
                init = Some(self.node(&te));
            }
            let Some(ty) = ty else {
                return Err(self.err(v.name.span, format!("`override {}` braucht einen Typ oder einen Initialisierer", v.name.name)));
            };
            if !matches!(self.t.get(ty), Type::Scalar(s) if !s.is_abstract()) {
                let sp = v.ty.as_ref().map(|t| t.span).unwrap_or(v.name.span);
                return Err(self.err(sp, format!("ein override muss einen konkreten Skalartyp haben, nicht `{}`", self.tn(ty))));
            }
            Ok((ty, init))
        })();
        self.module_arena = saved;
        let (ty, init) = r?;
        let mut id = None;
        if let Some(a) = self.attr(&v.attrs, "id") {
            let n = self.attr_nonneg(a)?;
            if n > 65535 {
                return Err(self.err(a.args()[0].span, format!("`@id` muss zwischen 0 und 65535 liegen, nicht {}", n)));
            }
            if let Some(&(_, psp)) = self.override_ids.get(&(n as u16)) {
                return Err(self.err_note(a.span, format!("`@id({})` ist schon vergeben", n), psp, String::from("hier")));
            }
            let oi = self.m.overrides.len() as u32;
            self.override_ids.insert(n as u16, (oi, a.span));
            id = Some(n as u16);
        }
        let oi = self.m.overrides.len() as u32;
        if let Some(i) = init {
            let mut s = BTreeSet::new();
            collect_overrides(&self.m.exprs, i, &mut s);
            self.override_ovr.insert(oi, s);
        }
        self.m.overrides.push(ir::Override { name: v.name.name.clone(), id, ty, init, span: v.span });
        Ok(oi)
    }

    fn function(&mut self, f: &'a ast::FnDecl, span: Span) -> R<u32> {
        let attrs_ok = self.check_attrs(&f.attrs, &["vertex", "fragment", "compute", "workgroup_size", "must_use", "diagnostic", "const", "subgroup_size"], "Funktionen");
        let mut stage = None;
        let mut bad = attrs_ok.is_err();
        for a in &f.attrs {
            let s = match a.name.name.as_str() {
                "vertex" => ir::Stage::Vertex,
                "fragment" => ir::Stage::Fragment,
                "compute" => ir::Stage::Compute,
                "const" => {
                    let _ = self.err(a.span, String::from("`@const` ist eigenen Funktionen nicht erlaubt"));
                    bad = true;
                    continue;
                }
                "subgroup_size" => {
                    let _ = self.err(a.span, String::from("`@subgroup_size` (subgroup_size_control) wird noch nicht unterstützt"));
                    bad = true;
                    continue;
                }
                _ => continue,
            };
            if stage.is_some() {
                let _ = self.err(a.span, String::from("eine Funktion kann nur für eine Shader-Stufe Einstiegspunkt sein"));
                bad = true;
            }
            stage = Some(s);
        }
        self.diag_attrs(&f.attrs, span);
        let entry = stage.is_some();
        // parameters
        let mut params = Vec::new();
        for p in &f.params {
            let allowed: &[&str] = if entry { &["builtin", "location", "interpolate", "invariant", "blend_src"] } else { &[] };
            let what = if entry { "Parametern von Einstiegspunkten" } else { "Parametern (IO-Attribute nur an Einstiegspunkten)" };
            if self.check_attrs(&p.attrs, allowed, what).is_err() {
                bad = true;
            }
            if let Some(q) = params.iter().find(|q: &&ir::Param| q.name == p.name.name) {
                let qsp = q.span;
                let _ = self.err_note(p.name.span, format!("den Parameter `{}` gibt es schon", p.name.name), qsp, String::from("hier zuerst"));
                bad = true;
            }
            let Ok(t) = self.ty(&p.ty) else {
                bad = true;
                params.push(ir::Param { name: p.name.name.clone(), ty: BOOL, io: Io::default(), span: p.span });
                continue;
            };
            let io = if entry { self.io_attrs(&p.attrs).unwrap_or_else(|_| {
                bad = true;
                Io::default()
            }) } else { Io::default() };
            if self.param_type_ok(t, p.ty.span).is_err() {
                bad = true;
            }
            params.push(ir::Param { name: p.name.name.clone(), ty: t, io, span: p.span });
        }
        let mut ret = None;
        let mut ret_io = Io::default();
        if let Some((rattrs, rty)) = &f.ret {
            let allowed: &[&str] = if entry { &["builtin", "location", "interpolate", "invariant", "blend_src"] } else { &[] };
            if self.check_attrs(rattrs, allowed, if entry { "Rückgabetypen von Einstiegspunkten" } else { "Rückgabetypen (IO-Attribute nur an Einstiegspunkten)" }).is_err() {
                bad = true;
            }
            match self.ty(rty) {
                Ok(t) => {
                    if !self.t.is_constructible(t) {
                        let _ = self.err(rty.span, format!("der Rückgabetyp `{}` ist nicht konstruierbar", self.tn(t)));
                        bad = true;
                    }
                    ret = Some(t);
                }
                Err(_) => bad = true,
            }
            if entry {
                match self.io_attrs(rattrs) {
                    Ok(io) => ret_io = io,
                    Err(_) => bad = true,
                }
            }
        }
        let must_use = self.attr(&f.attrs, "must_use").is_some();
        if must_use && ret.is_none() {
            let sp = self.attr(&f.attrs, "must_use").map(|a| a.span).unwrap_or(span);
            let _ = self.err(sp, String::from("`@must_use` braucht eine Funktion mit Rückgabetyp"));
            bad = true;
        }
        let fi = self.fns.len() as u32;
        let info = FnInfo {
            name: f.name.name.clone(),
            params: params.iter().map(|p| p.ty).collect(),
            ret,
            must_use,
            stage,
            returns: true,
            ptr_read: alloc::vec![false; params.len()],
            ptr_write: alloc::vec![false; params.len()],
            span: f.name.span,
            ..Default::default()
        };
        self.fns.push(info.clone());
        self.fn_decl.push(f);
        // the body
        let mut ctx = FnCtx {
            exprs: Vec::new(),
            locals: Vec::new(),
            local_init: Vec::new(),
            scopes: Vec::new(),
            next_uid: 0,
            params: params.clone(),
            ret,
            ctl: Vec::new(),
            in_continuing: 0,
            cont_uses: Vec::new(),
            info,
            calls: BTreeSet::new(),
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            had_error: bad,
        };
        let mut scope = Vec::new();
        for (k, p) in params.iter().enumerate() {
            scope.push((p.name.clone(), LocalRef::Param(k as u32), ctx.next_uid, p.span));
            ctx.next_uid += 1;
        }
        ctx.scopes.push(scope);
        self.f = Some(ctx);
        let body = self.fn_body(&f.body, f.name.span);
        let ctx = self.f.take().unwrap_or_else(|| unreachable_ctx());
        let mut info = ctx.info;
        info.ptr_read = (0..params.len()).map(|k| ctx.reads.contains(&Root::Param(k as u32))).collect();
        info.ptr_write = (0..params.len()).map(|k| ctx.writes.contains(&Root::Param(k as u32))).collect();
        for r in &ctx.reads {
            if let Root::Global(g) = r {
                info.g_read.insert(*g);
            }
        }
        for r in &ctx.writes {
            if let Root::Global(g) = r {
                info.g_write.insert(*g);
            }
        }
        let failed = ctx.had_error || body.is_err();
        if let Ok((returns, _)) = &body {
            info.returns = *returns;
        }
        self.fns[fi as usize] = info.clone();
        // workgroup_size
        if let Some(a) = self.attr(&f.attrs, "workgroup_size") {
            if stage != Some(ir::Stage::Compute) {
                let _ = self.err(a.span, String::from("`@workgroup_size` ist nur an `@compute`-Funktionen erlaubt"));
            }
        } else if stage == Some(ir::Stage::Compute) {
            let _ = self.err(f.name.span, format!("der Compute-Einstiegspunkt `{}` braucht `@workgroup_size(…)`", f.name.name));
        }
        self.m.functions.push(ir::Function {
            name: f.name.name.clone(),
            params,
            ret,
            ret_io,
            stage,
            must_use,
            locals: ctx.locals,
            exprs: ctx.exprs,
            body: body.map(|b| b.1).unwrap_or_default(),
            calls: ctx.calls.into_iter().collect(),
            globals: info.globals.iter().copied().collect(),
            overrides: info.overrides.iter().copied().collect(),
            uses_discard: info.uses_discard,
            span,
        });
        if failed {
            return Err(E);
        }
        Ok(fi)
    }

    /// constructible, pointer (with the address space rules), texture or sampler
    pub fn param_type_ok(&mut self, t: TypeId, sp: Span) -> R<()> {
        match self.t.get(t) {
            Type::Pointer { space, .. } => {
                let ok = matches!(space, AddressSpace::Function | AddressSpace::Private) || self.has(feature::UNRESTRICTED_POINTER_PARAMETERS) && !matches!(space, AddressSpace::Handle);
                if !ok {
                    return Err(self.err(sp, format!("Zeiger-Parameter sind nur in den Adressräumen function und private erlaubt, nicht `{}`", space.name())));
                }
                Ok(())
            }
            Type::Texture(..) | Type::Sampler { .. } => Ok(()),
            _ if self.t.is_constructible(t) => Ok(()),
            _ => Err(self.err(sp, format!("ein Parameter kann nicht vom Typ `{}` sein", self.tn(t)))),
        }
    }
}

fn unreachable_ctx() -> FnCtx {
    FnCtx {
        exprs: Vec::new(),
        locals: Vec::new(),
        local_init: Vec::new(),
        scopes: Vec::new(),
        next_uid: 0,
        params: Vec::new(),
        ret: None,
        ctl: Vec::new(),
        in_continuing: 0,
        cont_uses: Vec::new(),
        info: FnInfo::default(),
        calls: BTreeSet::new(),
        reads: BTreeSet::new(),
        writes: BTreeSet::new(),
        had_error: true,
    }
}

/// the overrides an expression in an arena mentions
pub fn collect_overrides(exprs: &[ir::Expr], e: ExprId, out: &mut BTreeSet<u32>) {
    let mut stack = alloc::vec![e];
    while let Some(i) = stack.pop() {
        let Some(x) = exprs.get(i as usize) else { continue };
        match &x.kind {
            ExprKind_::Override(o) => {
                out.insert(*o);
            }
            k => k.children(&mut stack),
        }
    }
}

use ir::ExprKind as ExprKind_;

impl ir::ExprKind {
    /// push the child expressions
    pub fn children(&self, out: &mut Vec<ExprId>) {
        use ir::ExprKind::*;
        match self {
            Const(_) | Override(_) | Global(_) | Local(_) | Param(_) => {}
            Load(a) | AddrOf(a) | Deref(a) | Member(a, _) | Swizzle(a, ..) | Unary(_, a) | Splat(a) | Convert(a) | Bitcast(a) => out.push(*a),
            Index(a, b) | Binary(_, a, b) => {
                out.push(*a);
                out.push(*b);
            }
            Construct(v) | Call(_, v) | Builtin(_, v) => out.extend_from_slice(v),
        }
    }
}

// ---- the dependency walk

struct DepWalk<'b, 'a> {
    names: &'b BTreeMap<&'a str, usize>,
    scopes: Vec<Vec<String>>,
    out: Vec<usize>,
}

impl DepWalk<'_, '_> {
    fn name(&mut self, n: &str) {
        if self.scopes.iter().any(|s| s.iter().any(|x| x == n)) {
            return;
        }
        if let Some(&i) = self.names.get(n) {
            self.out.push(i);
        }
    }
    fn declare(&mut self, n: &str) {
        if let Some(s) = self.scopes.last_mut() {
            s.push(n.to_string());
        }
    }
    fn exprs(&mut self, v: &[Expr]) {
        for e in v {
            self.expr(e);
        }
    }
    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Ident(id, t) => {
                self.name(&id.name);
                if let Some(t) = t {
                    self.exprs(t);
                }
            }
            ExprKind::Call(id, t, args) => {
                self.name(&id.name);
                if let Some(t) = t {
                    self.exprs(t);
                }
                self.exprs(args);
            }
            ExprKind::Paren(x) | ExprKind::Unary(_, x) | ExprKind::Member(x, _) => self.expr(x),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            _ => {}
        }
    }
    fn attrs(&mut self, a: &[Attr]) {
        for x in a {
            // the names inside @builtin / @interpolate are context-dependent, not references
            if matches!(x.name.name.as_str(), "builtin" | "interpolate" | "diagnostic") {
                continue;
            }
            self.exprs(x.args());
        }
    }
    fn var(&mut self, v: &VarDecl) {
        self.attrs(&v.attrs);
        // var<storage, read>: enumerants, which declarations may shadow -- walk them too
        if let Some(t) = &v.templ {
            self.exprs(t);
        }
        if let Some(t) = &v.ty {
            self.expr(t);
        }
        if let Some(i) = &v.init {
            self.expr(i);
        }
    }
    fn decl(&mut self, d: &Decl) {
        match &d.kind {
            DeclKind::Fn(f) => {
                self.attrs(&f.attrs);
                for p in &f.params {
                    self.attrs(&p.attrs);
                    self.expr(&p.ty);
                }
                if let Some((a, t)) = &f.ret {
                    self.attrs(a);
                    self.expr(t);
                }
                self.scopes.push(f.params.iter().map(|p| p.name.name.clone()).collect());
                self.stmts(&f.body.stmts);
                self.scopes.pop();
            }
            DeclKind::Var(v) => self.var(v),
            DeclKind::Alias { ty, .. } => self.expr(ty),
            DeclKind::Struct { members, .. } => {
                for m in members {
                    self.attrs(&m.attrs);
                    self.expr(&m.ty);
                }
            }
            DeclKind::ConstAssert(e) => self.expr(e),
        }
    }
    fn block(&mut self, b: &ast::Block) {
        self.scopes.push(Vec::new());
        self.stmts(&b.stmts);
        self.scopes.pop();
    }
    fn stmts(&mut self, v: &[ast::Stmt]) {
        for s in v {
            self.stmt(s);
        }
    }
    fn stmt(&mut self, s: &ast::Stmt) {
        match &s.kind {
            StmtKind::Block(b) => self.block(b),
            StmtKind::Return(Some(e)) | StmtKind::Call(e) | StmtKind::Phony(e) | StmtKind::Incr(e) | StmtKind::Decr(e) | StmtKind::ConstAssert(e) => self.expr(e),
            StmtKind::If { cond, then, else_ifs, else_, .. } => {
                self.expr(cond);
                self.block(then);
                for ei in else_ifs {
                    self.expr(&ei.cond);
                    self.block(&ei.body);
                }
                if let Some(b) = else_ {
                    self.block(b);
                }
            }
            StmtKind::Switch { selector, clauses, .. } => {
                self.expr(selector);
                for c in clauses {
                    for sel in &c.sels {
                        if let ast::CaseSel::Expr(e) = sel {
                            self.expr(e);
                        }
                    }
                    self.block(&c.body);
                }
            }
            StmtKind::Loop { body, continuing, .. } => {
                self.scopes.push(Vec::new());
                self.stmts(&body.stmts);
                if let Some(c) = continuing {
                    self.block(&c.body);
                    if let Some(e) = &c.break_if {
                        self.scopes.push(Vec::new());
                        self.stmts(&c.body.stmts);
                        self.expr(e);
                        self.scopes.pop();
                    }
                }
                self.scopes.pop();
            }
            StmtKind::For { init, cond, update, body, .. } => {
                self.scopes.push(Vec::new());
                if let Some(i) = init {
                    self.stmt(i);
                }
                if let Some(c) = cond {
                    self.expr(c);
                }
                if let Some(u) = update {
                    self.stmt(u);
                }
                self.stmts(&body.stmts);
                self.scopes.pop();
            }
            StmtKind::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            StmtKind::Decl(v) => {
                self.var(v);
                self.declare(&v.name.name);
            }
            StmtKind::Assign { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            _ => {}
        }
    }
}

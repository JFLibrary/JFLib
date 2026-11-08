// irtest -- the shape of the IR: no abstract types anywhere, constants folded, the Load Rule
// explicit, references and pointers, overload argument types, splats, matrix columns,
// for / while desugared to Loop, struct layouts (the spec's examples), the override table,
// entry point interfaces (globals, resources, overrides, workgroup sizes), function order

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use wgsl::ast::BinOp;
use wgsl::ir::*;

use crate::T;

fn no_abstract(m: &Module) -> bool {
    let abs = |t: TypeId| matches!(m.ty(t), Type::Scalar(Scalar::AbstractInt | Scalar::AbstractFloat) | Type::Vector(_, Scalar::AbstractInt | Scalar::AbstractFloat) | Type::Matrix(_, _, Scalar::AbstractFloat));
    m.functions.iter().all(|f| f.exprs.iter().all(|e| !abs(e.ty)) && f.locals.iter().all(|l| !abs(l.ty))) && m.exprs.iter().all(|e| !abs(e.ty))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Function {
    m.function(name).unwrap_or(&m.functions[0])
}

/// the IR expression of `return <e>;` (the last statement of f)
fn ret_expr<'m>(f: &'m Function) -> Option<&'m Expr> {
    match f.body.last()? {
        Stmt::Return(Some(e)) => f.exprs.get(*e as usize),
        _ => None,
    }
}

fn kind_name(k: &ExprKind) -> &'static str {
    match k {
        ExprKind::Const(_) => "Const",
        ExprKind::Override(_) => "Override",
        ExprKind::Global(_) => "Global",
        ExprKind::Local(_) => "Local",
        ExprKind::Param(_) => "Param",
        ExprKind::Load(_) => "Load",
        ExprKind::AddrOf(_) => "AddrOf",
        ExprKind::Deref(_) => "Deref",
        ExprKind::Index(..) => "Index",
        ExprKind::Member(..) => "Member",
        ExprKind::Swizzle(..) => "Swizzle",
        ExprKind::Unary(..) => "Unary",
        ExprKind::Binary(..) => "Binary",
        ExprKind::Construct(_) => "Construct",
        ExprKind::Splat(_) => "Splat",
        ExprKind::Convert(_) => "Convert",
        ExprKind::Bitcast(_) => "Bitcast",
        ExprKind::Call(..) => "Call",
        ExprKind::Builtin(..) => "Builtin",
        _ => "?",
    }
}

/// a compact tree of an expression: Load(Member(Deref(Param)))
fn tree(f: &Function, e: ExprId) -> String {
    let x = &f.exprs[e as usize];
    let kids: Vec<ExprId> = match &x.kind {
        ExprKind::Load(a) | ExprKind::AddrOf(a) | ExprKind::Deref(a) | ExprKind::Member(a, _) | ExprKind::Swizzle(a, ..) | ExprKind::Unary(_, a) | ExprKind::Splat(a) | ExprKind::Convert(a) | ExprKind::Bitcast(a) => alloc::vec![*a],
        ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) => alloc::vec![*a, *b],
        ExprKind::Construct(v) | ExprKind::Call(_, v) | ExprKind::Builtin(_, v) => v.clone(),
        _ => Vec::new(),
    };
    if kids.is_empty() {
        return String::from(kind_name(&x.kind));
    }
    format!("{}({})", kind_name(&x.kind), kids.iter().map(|&k| tree(f, k)).collect::<Vec<_>>().join(","))
}

pub fn run(t: &mut T) {
    // the sample of lexparse.rs: every expression concrete
    if let Some(m) = t.pass("sample", crate::lexparse::SAMPLE) {
        t.check("no abstract types in the IR", no_abstract(&m));
        t.check("callees before callers", m.functions.iter().position(|f| f.name == "helper") < m.functions.iter().position(|f| f.name == "main"));
        let main = func(&m, "main");
        t.check("main calls helper and pure", main.calls.len() == 2);
        let ep = m.entry_point("main");
        t.check("main uses data, tile, counter? (data, tile)", ep.is_some_and(|e| e.globals.iter().map(|&g| m.globals[g as usize].name.as_str()).collect::<Vec<_>>() == alloc::vec!["data", "tile"]));
        t.eq("main workgroup size", ep.and_then(|e| e.workgroup_size), Some([WgSize::Const(8), WgSize::Const(8), WgSize::Const(1)]));
        let fs = m.entry_point("fs");
        t.eq("fs resources", fs.map(|e| e.resources.iter().map(|r| (r.1.group, r.1.binding)).collect::<Vec<_>>()), Some(alloc::vec![(0, 0), (0, 2), (0, 3)]));
        t.check("fs input interpolation", fs.is_some_and(|e| e.inputs[0].kind == IoKind::Location { location: 0, blend_src: None, interp: Some((InterpType::Perspective, InterpSampling::Centroid)) }));
        t.check("fs uses discard", m.function("fs").is_some_and(|f| f.uses_discard));
        t.eq("diagnostic filters", m.diagnostic_filters.len(), 1);
        t.check("override ids", m.overrides.iter().map(|o| (o.name.as_str(), o.id)).collect::<Vec<_>>() == alloc::vec![("scale", Some(7)), ("size", None)]);
        t.eq("override type", m.overrides.get(1).map(|o| m.type_name(o.ty)), Some(String::from("u32")));
        let light = m.struct_named("Light");
        t.eq("Light layout", light.map(|s| (s.size, s.align, s.members.iter().map(|x| x.offset).collect::<Vec<_>>())), Some((48, 16, alloc::vec![0, 12, 32])));
        t.check("enables", m.enables & wgsl::feature::F16 != 0 && m.requires & wgsl::feature::POINTER_COMPOSITE_ACCESS != 0);
    }
    // folding, loads, references
    let src = "struct S { m: vec4f }\nvar<private> g: i32 = 3;\n@group(0) @binding(0) var<storage, read_write> buf: array<S>;\noverride ov: f32 = 2.0;\noverride ov2 = ov * 2.0 + 1.0;\nfn folded() -> i32 { return 1 + 2 * 3; }\nfn load() -> i32 { var a = 1; return a; }\nfn glob() -> i32 { return g; }\nfn ptrm(p: ptr<function, S>) -> f32 { return p.m.y; }\nfn idx(i: u32) -> f32 { return buf[i].m[2]; }\nfn conv(x: i32) -> f32 { return f32(x) + 1; }\nfn splat(v: vec3i) -> vec3i { return v + 1; }\nfn fmul(v: vec3f) -> vec3f { return v * 2.0; }\nfn mat(a: f32) -> mat2x2f { return mat2x2f(a, 0.0, 0.0, a); }\nfn ovr() -> f32 { return ov2 + ov; }\nfn sw(v: vec4f) -> vec2f { return v.zx; }\nfn bi(x: u32) -> u32 { return max(1u, x); }\nfn cmp(a: vec2f) -> vec2<bool> { return a < vec2f(); }\nfn neg(a: i32) -> i32 { return -a; }\nfn lit() -> f32 { let a = 2; return f32(a); }\n@compute @workgroup_size(1) fn main() { _ = idx(0u) + f32(glob()) + ovr(); }";
    if let Some(m) = t.pass("ir shapes", src) {
        t.check("ir shapes concrete", no_abstract(&m));
        let tr = |name: &str| ret_expr(func(&m, name)).map(|e| tree(func(&m, name), func(&m, name).exprs.iter().position(|x| x == e).unwrap_or(0) as u32)).unwrap_or_default();
        t.eq("constant folded", ret_expr(func(&m, "folded")).map(|e| e.kind.clone()), Some(ExprKind::Const(Value::I32(7))));
        t.eq("load of a local", tr("load"), String::from("Load(Local)"));
        t.eq("load of a global", tr("glob"), String::from("Load(Global)"));
        t.eq("pointer member", tr("ptrm"), String::from("Load(Member(Member(Deref(Param))))"));
        t.eq("runtime array index", tr("idx"), String::from("Load(Index(Member(Index(Global,Param)),Const))"));
        t.eq("conversion", tr("conv"), String::from("Binary(Convert(Param),Const)"));
        t.eq("splat for integer vectors", tr("splat"), String::from("Binary(Param,Splat(Const))"));
        t.eq("no splat for float multiply", tr("fmul"), String::from("Binary(Param,Const)"));
        t.eq("matrix from scalars by columns", tr("mat"), String::from("Construct(Construct(Param,Const),Construct(Const,Param))"));
        t.eq("overrides symbolic", tr("ovr"), String::from("Binary(Override,Override)"));
        t.eq("swizzle", tr("sw"), String::from("Swizzle(Param)"));
        t.eq("built-in", tr("bi"), String::from("Builtin(Const,Param)"));
        t.eq("comparison", tr("cmp"), String::from("Binary(Param,Const)"));
        t.eq("unary", tr("neg"), String::from("Unary(Param)"));
        let f = func(&m, "bi");
        t.check("built-in argument types", ret_expr(f).is_some_and(|e| matches!(&e.kind, ExprKind::Builtin(Builtin::Max, a) if a.iter().all(|&x| f.exprs[x as usize].ty == U32))));
        let f = func(&m, "cmp");
        t.check("comparison type", ret_expr(f).is_some_and(|e| m.type_name(e.ty) == "vec2<bool>"));
        let f = func(&m, "lit");
        t.check("let folded and concrete", matches!(f.body.first(), Some(Stmt::Let { init, .. }) if f.exprs[*init as usize].kind == ExprKind::Const(Value::I32(2))));
        let f = func(&m, "load");
        t.check("reference type", f.exprs.iter().any(|e| matches!(e.kind, ExprKind::Local(_)) && matches!(m.ty(e.ty), Type::Reference { space: AddressSpace::Function, access: Access::ReadWrite, .. })));
        // the override table: defaults symbolic
        let o2 = &m.overrides[1];
        t.check("override default symbolic", o2.init.is_some_and(|i| matches!(m.exprs[i as usize].kind, ExprKind::Binary(BinOp::Add, ..))));
        t.check("private initializer folded", m.global("g").and_then(|g| g.init).is_some_and(|i| m.exprs[i as usize].kind == ExprKind::Const(Value::I32(3))));
        let ep = m.entry_point("main");
        t.eq("entry overrides (transitive)", ep.map(|e| e.overrides.clone()), Some(alloc::vec![0, 1]));
        t.eq("entry globals", ep.map(|e| e.globals.clone()), Some(alloc::vec![0, 1]));
        t.eq("runtime array stride", m.global("buf").map(|g| m.ty(g.ty).clone()), Some(Type::Array { elem: m.global("buf").map(|g| if let Type::Array { elem, .. } = m.ty(g.ty) { *elem } else { 0 }).unwrap_or(0), count: ArrayCount::Runtime, stride: 16 }));
    }
    // statements: for / while desugared, compound and ++
    let src = "fn f() -> i32 { var s = 0; for (var i = 0; i < 4; i++) { s += i; } while s > 2 { s--; } return s; }";
    if let Some(m) = t.pass("loops desugared", src) {
        let f = func(&m, "f");
        let shape: Vec<&str> = f
            .body
            .iter()
            .map(|s| match s {
                Stmt::Var { .. } => "var",
                Stmt::Block(v) if matches!(v.as_slice(), [Stmt::Var { .. }, Stmt::Loop { .. }]) => "block(var, loop)",
                Stmt::Loop { body, .. } if matches!(body.first(), Some(Stmt::If { else_, .. }) if matches!(else_.as_slice(), [Stmt::Break])) => "loop(if-break, ..)",
                Stmt::Return(_) => "return",
                _ => "?",
            })
            .collect();
        t.eq("for and while as loops", shape, alloc::vec!["var", "block(var, loop)", "loop(if-break, ..)", "return"]);
        let cont = f.body.iter().find_map(|s| match s {
            Stmt::Block(v) => v.iter().find_map(|x| if let Stmt::Loop { continuing, .. } = x { continuing.first().cloned() } else { None }),
            _ => None,
        });
        t.check("i++ is a compound in continuing", matches!(cont, Some(Stmt::Compound { op: BinOp::Add, .. })));
    }
    // swizzle views
    if let Some(m) = t.pass("swizzle view", "fn f() { var v: vec4u; v.yz = vec2u(1u, 2u); }") {
        let f = func(&m, "f");
        let ok = f.body.iter().any(|s| matches!(s, Stmt::Assign { lhs, .. } if matches!(f.exprs[*lhs as usize].kind, ExprKind::Swizzle(_, c, 2) if c[0] == 1 && c[1] == 2) && matches!(m.ty(f.exprs[*lhs as usize].ty), Type::SwizzleView { n: 4, k: 2, .. })));
        t.check("swizzle view assignment", ok);
    }
    // the layout examples of spec 14.4.2
    let src = "struct A { u: f32, v: f32, w: vec2<f32>, x: f32 }\nstruct B { a: vec2<f32>, b: vec3<f32>, c: f32, d: f32, e: A, f: vec3<f32>, g: array<A, 3>, h: i32 }\n@group(0) @binding(0) var<storage, read_write> storage_buffer: B;\nstruct A2 { u: f32, v: f32, w: vec2<f32>, @size(16) x: f32 }\nstruct B2 { a: vec2<f32>, b: vec3<f32>, c: f32, d: f32, @align(16) e: A2, f: vec3<f32>, g: array<A2, 3>, h: i32 }\n@group(0) @binding(1) var<uniform> uniform_buffer: B2;";
    if let Some(m) = t.pass("layout examples", src) {
        let lay = |n: &str| m.struct_named(n).map(|s| (s.align, s.size, s.members.iter().map(|x| x.offset).collect::<Vec<_>>()));
        t.eq("A", lay("A"), Some((8, 24, alloc::vec![0, 4, 8, 16])));
        t.eq("B", lay("B"), Some((16, 160, alloc::vec![0, 16, 28, 32, 40, 64, 80, 152])));
        t.eq("A2", lay("A2"), Some((8, 32, alloc::vec![0, 4, 8, 16])));
        t.eq("B2", lay("B2"), Some((16, 208, alloc::vec![0, 16, 28, 32, 48, 80, 96, 192])));
        t.check("B2 is std140", m.global("uniform_buffer").is_some_and(|g| g.std140));
        let g = m.struct_named("B").map(|s| s.members[6].ty);
        t.check("array<A, 3> stride 24", g.is_some_and(|g| matches!(m.ty(g), Type::Array { stride: 24, count: ArrayCount::Fixed(3), .. })));
    }
    // types and layouts of the basic types
    if let Some(m) = t.pass("type layouts", "enable f16;\nstruct T { a: vec3h, b: f16, c: mat3x3h, d: vec2h, e: atomic<u32>, f: mat4x2f, g: array<vec3f, 2> }\n@group(0) @binding(0) var<storage, read_write> b: T;") {
        let s = m.struct_named("T");
        t.eq("f16 and matrix layout", s.map(|s| s.members.iter().map(|x| (x.offset, x.size, x.align)).collect::<Vec<_>>()), Some(alloc::vec![(0, 6, 8), (6, 2, 2), (8, 24, 8), (32, 4, 4), (36, 4, 4), (40, 32, 8), (80, 32, 16)]));
        t.eq("T size", s.map(|s| (s.size, s.align)), Some((112, 16)));
    }
    // override-sized workgroup arrays and the workgroup size
    let src = "override n: u32 = 16;\noverride m = n * 2u;\nvar<workgroup> a: array<f32, n>;\nvar<workgroup> b: array<f32, m * 2u>;\n@compute @workgroup_size(m, 1, 1) fn main() { a[0] = b[0]; }";
    if let Some(m) = t.pass("override arrays", src) {
        let a = m.global("a").map(|g| m.ty(g.ty).clone());
        t.check("count by override", matches!(a, Some(Type::Array { count: ArrayCount::Override(0), .. })));
        let b = m.global("b").map(|g| m.ty(g.ty).clone());
        t.check("count by expression", matches!(b, Some(Type::Array { count: ArrayCount::Expr(_), .. })));
        let ep = m.entry_point("main");
        t.check("workgroup size by override", ep.is_some_and(|e| matches!(e.workgroup_size, Some([WgSize::Override(_), WgSize::Const(1), WgSize::Const(1)]))));
        t.eq("entry overrides", ep.map(|e| e.overrides.clone()), Some(alloc::vec![0, 1]));
    }
    // the predeclared result structs
    if let Some(m) = t.pass("builtin structs", "fn f(x: vec3f) -> f32 { let a = frexp(x); let b = modf(1.5f); return a.fract.x + f32(a.exp.y) + b.whole; }") {
        t.check("frexp struct", m.structs.iter().any(|s| s.name == "__frexp_result_vec3_f32" && s.builtin == Some(BuiltinStruct::Frexp(3, Scalar::F32))));
        t.check("modf struct", m.structs.iter().any(|s| s.name == "__modf_result_f32"));
    }
    // type names in messages and the IR
    if let Some(m) = t.pass("type names", "@group(0) @binding(0) var t: texture_storage_2d_array<rgba16float, read_write>;\n@group(0) @binding(1) var<storage> s: array<mat3x2f>;\n@group(0) @binding(2) var d: texture_depth_cube_array;\n@group(0) @binding(3) var c: sampler_comparison;\n@compute @workgroup_size(1) fn main() { _ = t; _ = &s; _ = d; _ = c; }") {
        let names: Vec<String> = m.globals.iter().map(|g| m.type_name(g.ty)).collect();
        t.eq("names", names, alloc::vec![String::from("texture_storage_2d_array<rgba16float, read_write>"), String::from("array<mat3x2<f32>>"), String::from("texture_depth_cube_array"), String::from("sampler_comparison")]);
    }
    // f16 bits
    t.eq("f16 bits", (f16_bits(1.0), f16_bits(-2.0), f16_bits(65504.0), f16_bits(5.960464477539063e-8), f16_bits(0.0)), (0x3c00, 0xc000, 0x7bff, 1, 0));
}

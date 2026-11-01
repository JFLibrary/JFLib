// uniformity -- the uniformity analysis of spec section 15.2 over the checked IR: a directed
// graph per function (callees first), tags for its callers, and the diagnostics when a
// collective operation cannot be proven to run in uniform control flow (docs/WGSL.md,
// "Uniformitätsanalyse")
//
//   analyze(&module, src, &linemap, features) -> Vec<Diagnostic>   errors, warnings and infos
//
// Each function gets one graph.  An edge a -> b means "if a must be uniform, b must be too":
// the nodes RTBU[severity] (RequiredToBeUniform.S) start the walks, MBNU (MayBeNonUniform)
// ends them; CF_START, the parameter nodes and VALUE_RETURN are what the tags are made of:
//
//   RTBU[S] reaches MBNU           a uniformity failure with severity S
//   RTBU[S] reaches CF_START       CallSiteRequiredToBeUniform.S
//   RTBU[S] reaches a parameter    ParameterRequiredToBeUniform.S (value or contents)
//   VALUE_RETURN reaches MBNU      ReturnValueMayBeNonUniform; reaches a parameter: the result
//                                  needs that parameter's value or contents to be uniform
//   a pointer parameter's output contents reach MBNU or parameters: the value of the pointee
//                                  after a call (PointerParameterMayBeNonUniform, sources)
//
// Where the spec and Chrome (Tint) differ, the analysis takes the stricter one (never accept
// what Chrome rejects; docs/WGSL.md, "Abweichungen"):
//   * atomics return non-uniform values (the spec's tables do not say so)
//   * a load through a read-only pointer parameter depends on the pointer itself, a contents
//     requirement on a pointer argument also covers the argument's indices
//   * the pointee of a function-space pointer argument after a call depends on the control
//     flow of the call, and keeps its old value when the argument is a partial pointer
//   * entry point parameters that are not uniform built-ins are non-uniform
//
// With subgroup_uniformity the analysis runs once per uniformity scope: the workgroup / draw
// scope (barriers, derivatives) and, only when the module calls a subgroup or quad function,
// the subgroup scope.
//
// Stack: one recursion over statements and one over expressions, each inside the parser's
// depth limits; the walks over the graphs and the explanation of a failure are iterative.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::{BinOp, Span};
use crate::check::filter_at;
use crate::feature;
use crate::ir::*;
use crate::{builtins, Diagnostic, LineMap, Note};

const NONE: u32 = u32::MAX;
type Node = u32;

/// RequiredToBeUniform.error / warning / info
const RTBU: [Node; 3] = [0, 1, 2];
/// MayBeNonUniform
const MBNU: Node = 3;
const CF_START: Node = 4;
const VALUE_RETURN: Node = 5;
/// the parameter nodes start here: value, contents, output contents per parameter
const FIRST: Node = 6;

// the behaviors of spec 9.7 (the checker has computed them on the AST; the IR drops them)
const NEXT: u8 = 1;
const RETURN: u8 = 2;
const BREAK: u8 = 4;
const CONTINUE: u8 = 8;

/// at most this many notes per diagnostic (long chains lose their middle)
const MAX_NOTES: usize = 24;
/// at most this many uniformity warnings and infos per run
const MAX_REPORTS: usize = 32;

/// the uniformity scope of a run of the analysis
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scope {
    /// the workgroup or draw scope; bool: subgroup_uniformity is supported, so the subgroup
    /// and quad functions have no requirements here
    Main(bool),
    Subgroup,
}

/// what a node stands for (only what the explanation of a failure needs)
#[derive(Clone, Copy, PartialEq, Debug)]
enum Info {
    None,
    /// control flow decided by this expression (if, switch, break if, && and ||)
    Cond(ExprId),
    /// a call site that must be in uniform control flow
    CallReq(ExprId),
    /// an argument that must be uniform: call, index, contents of the pointer
    ArgReq(ExprId, u32, bool),
    /// an entry point input read here: expression, parameter
    Input(ExprId, u32),
    /// a read of a mutable module-scope variable: expression, variable
    ReadGlobal(ExprId, u32),
    /// a read through a pointer parameter to mutable memory
    ReadParam(ExprId, u32),
    /// the result of this call may be non-uniform
    CallResult(ExprId),
    /// the pointee of this argument may be non-uniform after the call
    PtrResult(ExprId, u32),
    /// parameter value and pointee
    Param(u32),
    Contents(u32),
}

#[derive(Default, Clone)]
struct ParamTags {
    value_req: Option<u8>,
    contents_req: Option<u8>,
    ret_value: bool,
    ret_contents: bool,
    out_nonuniform: bool,
    out_values: Vec<u32>,
    out_contents: Vec<u32>,
}

/// what a function's callers need to know
#[derive(Default, Clone)]
struct FnTags {
    callsite: Option<u8>,
    ret_nonuniform: bool,
    params: Vec<ParamTags>,
}

/// the memory view a reference or pointer expression ends in
#[derive(Clone, Copy, PartialEq, Debug)]
enum Root {
    /// a function-scope variable or function-space pointer parameter (a value slot)
    Var(u32),
    Global(u32),
    /// a pointer parameter into another address space
    PtrParam(u32),
    Nothing,
}

/// an enclosing loop or switch: what the values of the variables were at each exit
struct Target {
    is_loop: bool,
    /// the undo log's length when it was entered
    mark: usize,
    breaks: Vec<Vec<(u32, Node)>>,
    continues: Vec<Vec<(u32, Node)>>,
}

/// the graph of one function
#[derive(Default)]
struct Graph {
    head: Vec<u32>,
    edges: Vec<(u32, u32)>,
    info: Vec<Info>,
}

impl Graph {
    fn node(&mut self, info: Info) -> Node {
        self.head.push(NONE);
        self.info.push(info);
        (self.head.len() - 1) as Node
    }
    fn edge(&mut self, from: Node, to: Node) {
        if from == MBNU || from == to {
            return;
        }
        let next = self.head[from as usize];
        self.head[from as usize] = self.edges.len() as u32;
        self.edges.push((to, next));
    }
}

/// the state of building one function's graph
struct Build<'a> {
    m: &'a Module,
    f: &'a Function,
    tags: &'a [FnTags],
    scope: Scope,
    filters: &'a Filters,
    g: Graph,
    /// per slot (local, then function-space pointer parameter): the current value node
    /// (let: its value, var: the value analysis of 15.2.5)
    vals: Vec<Node>,
    /// let of pointer type, by local: the root and whether it is a full pointer
    lets: Vec<(Root, bool)>,
    /// undo log of vals: (slot, old value)
    log: Vec<(u32, Node)>,
    stamp: Vec<u32>,
    stamp_now: u32,
    /// the var slots in scope (innermost last)
    live: Vec<u32>,
    targets: Vec<Target>,
    /// a stack for argument values
    argv: Vec<Node>,
}

/// the diagnostic filters per rule
#[derive(Default)]
struct Filters {
    derivative: Vec<DiagnosticFilter>,
    subgroup: Vec<DiagnosticFilter>,
}

impl Filters {
    /// the severity the nearest enclosing filter gives at this place (default: error); None:
    /// off
    fn at(list: &[DiagnosticFilter], at: u32) -> Option<u8> {
        let rule = list.first().map(|f| f.rule.as_str()).unwrap_or("");
        match filter_at(list, rule, at) {
            None | Some(Severity::Error) => Some(0),
            Some(Severity::Warning) => Some(1),
            Some(Severity::Info) => Some(2),
            Some(Severity::Off) => None,
        }
    }
}

/// the derivative and implicit-LOD functions (a derivative_uniformity requirement)
fn is_derivative(b: Builtin) -> bool {
    use Builtin::*;
    matches!(b, Dpdx | DpdxCoarse | DpdxFine | Dpdy | DpdyCoarse | DpdyFine | Fwidth | FwidthCoarse | FwidthFine | TextureSample | TextureSampleBias | TextureSampleCompare)
}

/// the subgroup and quad functions (a subgroup_uniformity requirement)
fn is_subgroup(b: Builtin) -> bool {
    builtins::feature(b) == feature::SUBGROUPS
}

/// at subgroup scope the result of these is as uniform as their arguments (15.2.7)
fn subgroup_result_uniform(b: Builtin) -> bool {
    use Builtin::*;
    matches!(b, SubgroupAdd | SubgroupAll | SubgroupAnd | SubgroupAny | SubgroupBallot | SubgroupBroadcast | SubgroupBroadcastFirst | SubgroupMax | SubgroupMin | SubgroupMul | SubgroupOr | SubgroupXor)
}

fn is_atomic(b: Builtin) -> bool {
    use Builtin::*;
    matches!(b, AtomicLoad | AtomicStore | AtomicAdd | AtomicSub | AtomicMax | AtomicMin | AtomicAnd | AtomicOr | AtomicXor | AtomicExchange | AtomicCompareExchangeWeak | AtomicStoreMin | AtomicStoreMax)
}

/// mutable memory (a load may see another invocation's writes)
fn global_mutable(g: &Global) -> bool {
    match g.space {
        AddressSpace::Storage => g.access != Access::Read,
        AddressSpace::Workgroup | AddressSpace::Private => true,
        _ => false,
    }
}

impl<'a> Build<'a> {
    fn new(m: &'a Module, fi: usize, tags: &'a [FnTags], scope: Scope, filters: &'a Filters, g: Graph) -> Build<'a> {
        let f = &m.functions[fi];
        let mut g = g;
        g.head.clear();
        g.edges.clear();
        g.info.clear();
        for _ in 0..FIRST {
            g.node(Info::None);
        }
        let np = f.params.len();
        for k in 0..np as u32 {
            g.node(Info::Param(k));
            g.node(Info::Contents(k));
            g.node(Info::None);
        }
        let nl = f.locals.len();
        let slots = nl + np;
        let mut b = Build {
            m,
            f,
            tags,
            scope,
            filters,
            g,
            vals: alloc::vec![CF_START; slots],
            lets: alloc::vec![(Root::Nothing, false); nl],
            log: Vec::new(),
            stamp: alloc::vec![0; slots],
            stamp_now: 0,
            live: Vec::new(),
            targets: Vec::new(),
            argv: Vec::new(),
        };
        // a function-space pointer parameter is a variable that starts with its contents
        for k in 0..np {
            if b.is_fn_ptr(f.params[k].ty) {
                let s = nl + k;
                b.vals[s] = FIRST + 3 * k as u32 + 1;
                b.live.push(s as u32);
            }
        }
        b
    }

    fn is_fn_ptr(&self, t: TypeId) -> bool {
        matches!(self.m.types.get(t as usize), Some(Type::Pointer { space: AddressSpace::Function, .. }))
    }

    fn set(&mut self, slot: u32, v: Node) {
        let s = slot as usize;
        self.log.push((slot, self.vals[s]));
        self.vals[s] = v;
    }

    fn rollback(&mut self, mark: usize) {
        while self.log.len() > mark {
            let Some((s, old)) = self.log.pop() else { break };
            self.vals[s as usize] = old;
        }
    }

    /// (slot, current value) of each slot assigned since the mark
    fn changed(&mut self, mark: usize) -> Vec<(u32, Node)> {
        let mut out = Vec::new();
        if self.log.len() > mark {
            self.stamp_now += 1;
            for i in mark..self.log.len() {
                let s = self.log[i].0;
                if self.stamp[s as usize] != self.stamp_now {
                    self.stamp[s as usize] = self.stamp_now;
                    out.push((s, self.vals[s as usize]));
                }
            }
        }
        out
    }

    fn join(&mut self, a: Node, b: Node) -> Node {
        if a == b {
            return a;
        }
        let r = self.g.node(Info::None);
        self.g.edge(r, a);
        self.g.edge(r, b);
        r
    }

    /// a node with an edge to MayBeNonUniform
    fn source(&mut self, info: Info) -> Node {
        let r = self.g.node(info);
        self.g.edge(r, MBNU);
        r
    }

    fn require(&mut self, sev: u8, info: Info, to: Node) {
        let r = self.g.node(info);
        self.g.edge(r, to);
        self.g.edge(RTBU[sev as usize & 3], r);
    }

    // ---- memory views

    /// the root of a reference or pointer expression and whether the expression covers the
    /// whole variable (15.2.5: full reference, full pointer, full swizzle view)
    fn root(&self, e: ExprId) -> (Root, bool) {
        let mut cur = e;
        let mut full = true;
        let nl = self.f.locals.len();
        loop {
            let Some(x) = self.f.exprs.get(cur as usize) else { return (Root::Nothing, false) };
            match &x.kind {
                ExprKind::Local(l) => {
                    return match self.f.locals.get(*l as usize).map(|v| v.kind) {
                        Some(LocalKind::Var) => (Root::Var(*l), full),
                        Some(LocalKind::Let) => match self.lets.get(*l as usize) {
                            Some(&(r, fp)) => (r, full && fp),
                            None => (Root::Nothing, false),
                        },
                        None => (Root::Nothing, false),
                    };
                }
                ExprKind::Param(k) => {
                    let ty = self.f.params.get(*k as usize).map(|p| p.ty).unwrap_or(BOOL);
                    return match self.m.types.get(ty as usize) {
                        Some(Type::Pointer { space: AddressSpace::Function, .. }) => (Root::Var((nl + *k as usize) as u32), full),
                        Some(Type::Pointer { .. }) => (Root::PtrParam(*k), full),
                        _ => (Root::Nothing, false),
                    };
                }
                ExprKind::Global(g) => return (Root::Global(*g), full),
                ExprKind::AddrOf(b) | ExprKind::Deref(b) => cur = *b,
                ExprKind::Index(b, _) | ExprKind::Member(b, _) => {
                    full = false;
                    cur = *b;
                }
                ExprKind::Swizzle(b, comps, n) => {
                    // full only as a permutation of all components of the vector below
                    let width = match self.f.exprs.get(*b as usize).and_then(|y| self.m.types.get(y.ty as usize)) {
                        Some(Type::Reference { store, .. }) => match self.m.types.get(*store as usize) {
                            Some(Type::Vector(w, _)) => *w,
                            _ => 0,
                        },
                        Some(Type::SwizzleView { k, .. }) => *k,
                        _ => 0,
                    };
                    let k = *n as usize;
                    let perm = *n == width && (0..k).all(|i| !comps[..i].contains(&comps[i]));
                    full = full && perm;
                    cur = *b;
                }
                _ => return (Root::Nothing, false),
            }
        }
    }

    // ---- expressions (spec 15.2.8)

    fn val(&mut self, cf: Node, e: ExprId) -> Node {
        let f = self.f;
        let Some(ex) = f.exprs.get(e as usize) else { return cf };
        match &ex.kind {
            ExprKind::Const(_) | ExprKind::Override(_) | ExprKind::Global(_) => cf,
            ExprKind::Local(l) => match f.locals.get(*l as usize).map(|v| v.kind) {
                Some(LocalKind::Let) => {
                    let v = self.vals[*l as usize];
                    self.join(cf, v)
                }
                _ => cf,
            },
            ExprKind::Param(k) => self.param(cf, e, *k),
            ExprKind::Load(r) => self.load(cf, e, *r),
            ExprKind::AddrOf(b)
            | ExprKind::Deref(b)
            | ExprKind::Member(b, _)
            | ExprKind::Swizzle(b, ..)
            | ExprKind::Unary(_, b)
            | ExprKind::Splat(b)
            | ExprKind::Convert(b)
            | ExprKind::Bitcast(b) => self.val(cf, *b),
            ExprKind::Index(b, i) => {
                let a = self.val(cf, *b);
                let c = self.val(cf, *i);
                self.join(a, c)
            }
            ExprKind::Binary(op, a, b) => {
                let va = self.val(cf, *a);
                if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
                    // the right side runs only for some invocations when the left side differs
                    let c = self.g.node(Info::Cond(e));
                    self.g.edge(c, va);
                    return self.val(c, *b);
                }
                let vb = self.val(cf, *b);
                self.join(va, vb)
            }
            ExprKind::Construct(args) => self.join_all(cf, args),
            ExprKind::Call(fi, args) => self.call(cf, e, *fi, args),
            ExprKind::Builtin(b, args) => self.builtin(cf, e, *b, args),
        }
    }

    fn join_all(&mut self, cf: Node, args: &[ExprId]) -> Node {
        let mut r = cf;
        for (i, a) in args.iter().enumerate() {
            let v = self.val(cf, *a);
            r = if i == 0 { v } else { self.join(r, v) };
        }
        r
    }

    /// the built-in values the analysis takes as uniform (15.2.8)
    fn builtin_value_uniform(&self, b: BuiltinValue) -> bool {
        match b {
            BuiltinValue::WorkgroupId | BuiltinValue::NumWorkgroups | BuiltinValue::NumSubgroups => true,
            BuiltinValue::SubgroupSize => self.f.stage == Some(Stage::Compute),
            BuiltinValue::SubgroupId => self.scope == Scope::Subgroup,
            _ => false,
        }
    }

    fn param(&mut self, cf: Node, e: ExprId, k: u32) -> Node {
        let f = self.f;
        let Some(p) = f.params.get(k as usize) else { return cf };
        if f.stage.is_some() {
            // an entry point input: a struct counts as one value
            let uniform = match self.m.types.get(p.ty as usize) {
                Some(Type::Struct(s)) => self.m.structs.get(*s as usize).is_some_and(|st| st.members.iter().all(|mem| mem.io.builtin.is_some_and(|b| self.builtin_value_uniform(b)))),
                _ => p.io.builtin.is_some_and(|b| self.builtin_value_uniform(b)),
            };
            return if uniform { cf } else { self.source(Info::Input(e, k)) };
        }
        let x = FIRST + 3 * k;
        self.join(cf, x)
    }

    fn load(&mut self, cf: Node, e: ExprId, r: ExprId) -> Node {
        let v = self.val(cf, r);
        match self.root(r).0 {
            Root::Var(slot) => {
                let x = self.vals[slot as usize];
                self.join(v, x)
            }
            Root::Global(g) => {
                if self.m.globals.get(g as usize).is_some_and(global_mutable) {
                    self.source(Info::ReadGlobal(e, g))
                } else {
                    v
                }
            }
            Root::PtrParam(k) => {
                let mutable = match self.f.params.get(k as usize).and_then(|p| self.m.types.get(p.ty as usize)) {
                    Some(Type::Pointer { access, .. }) => *access != Access::Read,
                    _ => false,
                };
                if mutable {
                    self.source(Info::ReadParam(e, k))
                } else {
                    v
                }
            }
            Root::Nothing => v,
        }
    }

    // ---- calls (spec 15.2.7)

    fn call(&mut self, cf: Node, e: ExprId, fi: u32, args: &[ExprId]) -> Node {
        let base = self.argv.len();
        for a in args {
            let v = self.val(cf, *a);
            self.argv.push(v);
        }
        let n = args.len();
        let tags: &'a [FnTags] = self.tags;
        let Some(t) = tags.get(fi as usize) else {
            self.argv.truncate(base);
            return cf;
        };
        let callee = self.m.functions.get(fi as usize);
        // the contents of function-space pointer arguments before the call
        for k in 0..n {
            let ty = callee.and_then(|c| c.params.get(k)).map(|p| p.ty).unwrap_or(BOOL);
            let mut c = NONE;
            if self.is_fn_ptr(ty) {
                if let Root::Var(slot) = self.root(args[k]).0 {
                    let old = self.vals[slot as usize];
                    let arg = self.argv[base + k];
                    c = self.join(arg, old);
                }
            }
            self.argv.push(c);
        }
        let has_result = callee.is_some_and(|c| c.ret.is_some());
        let result = if has_result { self.g.node(Info::None) } else { cf };
        if has_result {
            self.g.edge(result, cf);
        }
        if t.ret_nonuniform && has_result {
            let src = self.source(Info::CallResult(e));
            self.g.edge(result, src);
        }
        for (k, p) in t.params.iter().enumerate().take(n) {
            let (arg, contents) = (self.argv[base + k], self.argv[base + n + k]);
            if let Some(s) = p.value_req {
                self.require(s, Info::ArgReq(e, k as u32, false), arg);
            }
            if let (Some(s), true) = (p.contents_req, contents != NONE) {
                self.require(s, Info::ArgReq(e, k as u32, true), contents);
            }
            if has_result {
                if p.ret_value {
                    self.g.edge(result, arg);
                }
                if p.ret_contents && contents != NONE {
                    self.g.edge(result, contents);
                }
            }
        }
        // (the call site last: the newest requirement is the one a failure reports, as in Tint)
        if let Some(s) = t.callsite {
            self.require(s, Info::CallReq(e), cf);
        }
        // the pointees after the call
        for (k, p) in t.params.iter().enumerate().take(n) {
            if self.argv[base + n + k] == NONE {
                continue;
            }
            let (root, full) = self.root(args[k]);
            let Root::Var(slot) = root else { continue };
            let out = self.g.node(Info::PtrResult(e, k as u32));
            self.g.edge(out, cf);
            if p.out_nonuniform {
                self.g.edge(out, MBNU);
            } else {
                for &j in &p.out_values {
                    if let Some(&a) = self.argv.get(base + j as usize) {
                        self.g.edge(out, a);
                    }
                }
                for &j in &p.out_contents {
                    if let Some(&c) = self.argv.get(base + n + j as usize) {
                        if c != NONE {
                            self.g.edge(out, c);
                        }
                    }
                }
            }
            if !full {
                let old = self.vals[slot as usize];
                self.g.edge(out, old);
            }
            self.set(slot, out);
        }
        self.argv.truncate(base);
        result
    }

    fn builtin(&mut self, cf: Node, e: ExprId, b: Builtin, args: &[ExprId]) -> Node {
        use Builtin::*;
        let main_scope = self.scope != Scope::Subgroup;
        let at = self.f.exprs.get(e as usize).map(|x| x.span.start).unwrap_or(0);
        match b {
            WorkgroupBarrier | StorageBarrier | TextureBarrier => {
                if main_scope {
                    self.require(0, Info::CallReq(e), cf);
                }
                cf
            }
            WorkgroupUniformLoad => {
                let v = self.join_all(cf, args);
                if main_scope {
                    self.require(0, Info::ArgReq(e, 0, false), v);
                    self.require(0, Info::CallReq(e), cf);
                }
                v
            }
            _ if is_derivative(b) => {
                self.join_all(cf, args);
                if main_scope {
                    if let Some(s) = Filters::at(&self.filters.derivative, at) {
                        self.require(s, Info::CallReq(e), cf);
                    }
                }
                self.source(Info::CallResult(e))
            }
            TextureLoad => {
                let v = self.join_all(cf, args);
                let rw = args.first().and_then(|t| self.f.exprs.get(*t as usize)).and_then(|x| self.m.types.get(x.ty as usize)).is_some_and(|t| matches!(t, Type::Texture(_, TexKind::Storage(_, Access::ReadWrite))));
                if rw {
                    self.source(Info::CallResult(e))
                } else {
                    v
                }
            }
            _ if is_atomic(b) => {
                self.join_all(cf, args);
                self.source(Info::CallResult(e))
            }
            _ if is_subgroup(b) => self.subgroup(cf, e, b, args),
            _ => self.join_all(cf, args),
        }
    }

    /// a subgroup or quad function: requirements at subgroup scope or when there is only one
    /// scope; the result is non-uniform except for the reductions at subgroup scope
    fn subgroup(&mut self, cf: Node, e: ExprId, b: Builtin, args: &[ExprId]) -> Node {
        let at = self.f.exprs.get(e as usize).map(|x| x.span.start).unwrap_or(0);
        let strict = matches!(self.scope, Scope::Main(false) | Scope::Subgroup);
        let (mut v, mut second) = (cf, cf);
        for (i, a) in args.iter().enumerate() {
            let x = self.val(cf, *a);
            if i == 1 {
                second = x;
            }
            v = if i == 0 { x } else { self.join(v, x) };
        }
        if strict {
            if let Some(s) = Filters::at(&self.filters.subgroup, at) {
                if matches!(b, Builtin::SubgroupShuffleUp | Builtin::SubgroupShuffleDown | Builtin::SubgroupShuffleXor) {
                    self.require(s, Info::ArgReq(e, 1, false), second);
                }
                self.require(s, Info::CallReq(e), cf);
            }
        }
        if self.scope == Scope::Subgroup && subgroup_result_uniform(b) {
            v
        } else {
            self.source(Info::CallResult(e))
        }
    }

    // ---- statements (spec 15.2.6, the value analysis of 15.2.5): (control flow, behavior)

    fn block(&mut self, cf: Node, list: &[Stmt]) -> (Node, u8) {
        let live = self.live.len();
        let r = self.stmts(cf, list);
        self.live.truncate(live);
        r
    }

    fn stmts(&mut self, mut cf: Node, list: &[Stmt]) -> (Node, u8) {
        let mut acc = NEXT;
        for s in list {
            let (c, b) = self.stmt(cf, s);
            cf = c;
            acc = (acc & !NEXT) | b;
            // what follows is statically unreachable
            if b & NEXT == 0 {
                break;
            }
        }
        (cf, acc)
    }

    fn stmt(&mut self, cf: Node, s: &Stmt) -> (Node, u8) {
        match s {
            Stmt::Block(v) => self.block(cf, v),
            Stmt::Var { local, init } => {
                let v = match init {
                    Some(e) => self.val(cf, *e),
                    None => cf,
                };
                self.set(*local, v);
                self.live.push(*local);
                (cf, NEXT)
            }
            Stmt::Let { local, init } => {
                let v = self.val(cf, *init);
                let l = *local as usize;
                self.vals[l] = v;
                let ty = self.f.locals.get(l).map(|x| x.ty).unwrap_or(BOOL);
                if matches!(self.m.types.get(ty as usize), Some(Type::Pointer { .. })) {
                    self.lets[l] = self.root(*init);
                }
                (cf, NEXT)
            }
            Stmt::Assign { lhs, rhs } => {
                let lv = self.val(cf, *lhs);
                let rv = self.val(cf, *rhs);
                self.store(lv, rv, *lhs, false);
                (cf, NEXT)
            }
            Stmt::Compound { lhs, rhs, .. } => {
                let lv = self.val(cf, *lhs);
                let rv = self.val(cf, *rhs);
                self.store(lv, rv, *lhs, true);
                (cf, NEXT)
            }
            Stmt::Phony(e) | Stmt::Call(e) => {
                self.val(cf, *e);
                (cf, NEXT)
            }
            Stmt::If { cond, then, else_ } => self.if_(cf, *cond, then, else_),
            Stmt::Switch { selector, cases } => self.switch(cf, *selector, cases),
            Stmt::Loop { body, continuing, break_if } => self.loop_(cf, body, continuing, *break_if),
            Stmt::Break => {
                let mark = self.targets.last().map(|t| t.mark).unwrap_or(0);
                let state = self.changed(mark);
                if let Some(t) = self.targets.last_mut() {
                    t.breaks.push(state);
                }
                (cf, BREAK)
            }
            Stmt::Continue => {
                let at = self.targets.iter().rposition(|t| t.is_loop);
                if let Some(at) = at {
                    let state = self.changed(self.targets[at].mark);
                    self.targets[at].continues.push(state);
                }
                (cf, CONTINUE)
            }
            Stmt::Return(e) => {
                if let Some(e) = e {
                    let v = self.val(cf, *e);
                    self.g.edge(VALUE_RETURN, v);
                }
                self.pointer_outputs();
                (cf, RETURN)
            }
            Stmt::Discard => (cf, NEXT),
        }
    }

    /// `lhs = rhs` (compound: also the old value)
    fn store(&mut self, lv: Node, rv: Node, lhs: ExprId, compound: bool) {
        let (root, full) = self.root(lhs);
        let Root::Var(slot) = root else { return };
        let n = self.g.node(Info::None);
        self.g.edge(n, rv);
        self.g.edge(n, lv);
        if !full || compound {
            let old = self.vals[slot as usize];
            self.g.edge(n, old);
        }
        self.set(slot, n);
    }

    /// Value_return_i_contents -> the current value, at every way out of the function
    fn pointer_outputs(&mut self) {
        let nl = self.f.locals.len();
        for k in 0..self.f.params.len() {
            if self.is_fn_ptr(self.f.params[k].ty) {
                let v = self.vals[nl + k];
                self.g.edge(FIRST + 3 * k as u32 + 2, v);
            }
        }
    }

    fn if_(&mut self, cf: Node, cond: ExprId, then: &[Stmt], else_: &[Stmt]) -> (Node, u8) {
        let v = self.val(cf, cond);
        let c = self.g.node(Info::Cond(cond));
        self.g.edge(c, v);
        let mark = self.log.len();
        let (cf1, b1) = self.block(c, then);
        let s1 = self.changed(mark);
        self.rollback(mark);
        let (cf2, b2) = self.block(c, else_);
        let s2 = self.changed(mark);
        self.rollback(mark);
        match (b1 & NEXT != 0, b2 & NEXT != 0) {
            (true, true) => self.merge(&[s1, s2]),
            (true, false) => self.apply(&s1),
            (false, true) => self.apply(&s2),
            (false, false) => {}
        }
        let beh = b1 | b2;
        if beh == NEXT {
            return (cf, beh);
        }
        let end = self.join(cf1, cf2);
        (end, beh)
    }

    fn apply(&mut self, state: &[(u32, Node)]) {
        for &(slot, v) in state {
            self.set(slot, v);
        }
    }

    /// after a rollback: the join of several states (a slot a state does not list keeps the
    /// current value in that state)
    fn merge(&mut self, states: &[Vec<(u32, Node)>]) {
        let mut all: Vec<(u32, Node)> = Vec::new();
        for s in states {
            all.extend_from_slice(s);
        }
        if all.is_empty() {
            return;
        }
        all.sort_unstable_by_key(|x| x.0);
        let mut i = 0;
        while i < all.len() {
            let slot = all[i].0;
            let mut k = i;
            let n = self.g.node(Info::None);
            while k < all.len() && all[k].0 == slot {
                self.g.edge(n, all[k].1);
                k += 1;
            }
            if k - i < states.len() {
                let old = self.vals[slot as usize];
                self.g.edge(n, old);
            }
            self.set(slot, n);
            i = k;
        }
    }

    fn switch(&mut self, cf: Node, selector: ExprId, cases: &[Case]) -> (Node, u8) {
        let v = self.val(cf, selector);
        let c = self.g.node(Info::Cond(selector));
        self.g.edge(c, v);
        let mark = self.log.len();
        self.targets.push(Target { is_loop: false, mark, breaks: Vec::new(), continues: Vec::new() });
        let mut beh = 0u8;
        let mut exits: Vec<Vec<(u32, Node)>> = Vec::new();
        let mut ends: Vec<Node> = Vec::with_capacity(cases.len());
        for case in cases {
            let (cfi, b) = self.block(c, &case.body);
            if b & NEXT != 0 {
                exits.push(self.changed(mark));
            }
            self.rollback(mark);
            beh |= b;
            ends.push(cfi);
        }
        if let Some(t) = self.targets.pop() {
            exits.extend(t.breaks);
        }
        self.merge(&exits);
        if beh & BREAK != 0 {
            beh = (beh | NEXT) & !BREAK;
        }
        if beh == NEXT {
            return (cf, beh);
        }
        let end = self.g.node(Info::None);
        for x in ends {
            self.g.edge(end, x);
        }
        (end, beh)
    }

    fn loop_(&mut self, cf: Node, body: &[Stmt], continuing: &[Stmt], break_if: Option<ExprId>) -> (Node, u8) {
        let live_len = self.live.len();
        let m0 = self.log.len();
        // the value of every variable at the start of an iteration: its value before the
        // loop or at the end of the continuing block (edges added below)
        let mut starts: Vec<(u32, Node)> = Vec::with_capacity(live_len);
        for i in 0..live_len {
            let slot = self.live[i];
            let p = self.g.node(Info::None);
            let old = self.vals[slot as usize];
            self.g.edge(p, old);
            self.set(slot, p);
            starts.push((slot, p));
        }
        starts.sort_unstable_by_key(|x| x.0);
        let m1 = self.log.len();
        self.targets.push(Target { is_loop: true, mark: m1, breaks: Vec::new(), continues: Vec::new() });
        let cfp = self.g.node(Info::None);
        self.g.edge(cfp, cf);
        let (cf1, b1) = self.stmts(cfp, body);
        let mut b2 = NEXT;
        let mut has_back = false;
        if b1 & (NEXT | CONTINUE) != 0 {
            has_back = true;
            // the value at the start of the continuing block: the end of the body or a continue
            let conts = self.targets.last_mut().map(|t| core::mem::take(&mut t.continues)).unwrap_or_default();
            if !conts.is_empty() {
                let cur = self.changed(m1);
                self.merge_continue(&conts, &cur, &starts, b1 & NEXT != 0);
            }
            let (mut cf2, b) = self.stmts(cf1, continuing);
            b2 = b;
            if let Some(cond) = break_if {
                let v = self.val(cf2, cond);
                let end = self.g.node(Info::Cond(cond));
                self.g.edge(end, v);
                cf2 = end;
                let state = self.changed(m1);
                if let Some(t) = self.targets.last_mut() {
                    t.breaks.push(state);
                }
                if b2 & NEXT != 0 {
                    b2 |= BREAK;
                }
            }
            self.g.edge(cfp, cf2);
            // the back edges of the value analysis
            if b2 & NEXT != 0 {
                for i in 0..starts.len() {
                    let (slot, p) = starts[i];
                    let v = self.vals[slot as usize];
                    self.g.edge(p, v);
                }
            }
        }
        self.live.truncate(live_len);
        let breaks = self.targets.pop().map(|t| t.breaks).unwrap_or_default();
        self.rollback(m0);
        // the values after the loop: the join over every break
        if !breaks.is_empty() {
            let total = breaks.len();
            let mut all: Vec<(u32, Node)> = Vec::new();
            for b in &breaks {
                all.extend_from_slice(b);
            }
            all.sort_unstable_by_key(|x| x.0);
            let mut at = 0;
            for &(slot, p) in &starts {
                while at < all.len() && all[at].0 < slot {
                    at += 1;
                }
                let begin = at;
                while at < all.len() && all[at].0 == slot {
                    at += 1;
                }
                if at == begin {
                    self.set(slot, p);
                } else {
                    let n = self.g.node(Info::None);
                    for x in &all[begin..at] {
                        self.g.edge(n, x.1);
                    }
                    if at - begin < total {
                        self.g.edge(n, p);
                    }
                    self.set(slot, n);
                }
            }
        }
        // the behavior and the control flow after the loop
        let b2 = b2 & !(CONTINUE | RETURN);
        let beh = if b1 == RETURN {
            RETURN
        } else if (b1 | b2) & BREAK == 0 {
            (b1 | b2) & !(CONTINUE | NEXT)
        } else {
            (b1 | b2 | NEXT) & !(BREAK | CONTINUE)
        };
        let end = if b1 & RETURN == 0 {
            cf
        } else if has_back {
            cfp
        } else {
            cf1
        };
        (end, beh)
    }

    /// the values at the start of the continuing block: each `continue` and, if reachable,
    /// the end of the loop body; a state without a slot has the value of the iteration's start
    fn merge_continue(&mut self, conts: &[Vec<(u32, Node)>], cur: &[(u32, Node)], starts: &[(u32, Node)], next: bool) {
        let mut all: Vec<(u32, Node)> = Vec::new();
        for s in conts {
            all.extend_from_slice(s);
        }
        if next {
            all.extend_from_slice(cur);
        }
        all.sort_unstable_by_key(|x| x.0);
        let total = conts.len() + next as usize;
        let mut i = 0;
        while i < all.len() {
            let slot = all[i].0;
            let n = self.g.node(Info::None);
            let mut k = i;
            while k < all.len() && all[k].0 == slot {
                self.g.edge(n, all[k].1);
                k += 1;
            }
            if k - i < total {
                if let Ok(at) = starts.binary_search_by_key(&slot, |x| x.0) {
                    self.g.edge(n, starts[at].1);
                }
            }
            self.set(slot, n);
            i = k;
        }
    }

    /// the whole function body
    fn run(&mut self) {
        let f = self.f;
        let (_, beh) = self.stmts(CF_START, &f.body);
        if beh & NEXT != 0 {
            self.pointer_outputs();
        }
    }
}

// ---------------------------------------------------------------- the phases per function

/// a walk from `start`, skipping interior nodes visited in this generation: the special nodes
/// (below `first`) it reaches go to `hits`
fn walk(g: &Graph, first: Node, start: Node, seen: &mut [u32], round: u32, hits: &mut Vec<Node>) {
    hits.clear();
    let mut stack = alloc::vec![start];
    while let Some(n) = stack.pop() {
        let mut ed = g.head[n as usize];
        while ed != NONE {
            let (to, next) = g.edges[ed as usize];
            if to < first {
                hits.push(to);
            } else if seen[to as usize] != round {
                seen[to as usize] = round;
                stack.push(to);
            }
            ed = next;
        }
    }
}

/// the tags of a function from its graph; the severities that failed (spec 15.2.3)
fn tags_of(f: &Function, g: &Graph, m: &Module) -> (FnTags, [bool; 3]) {
    let np = f.params.len() as u32;
    let first = FIRST + 3 * np;
    let mut t = FnTags { callsite: None, ret_nonuniform: false, params: alloc::vec![ParamTags::default(); np as usize] };
    let mut failed = [false; 3];
    let mut seen = alloc::vec![0u32; g.head.len()];
    let mut hits = Vec::new();
    for s in 0..3u8 {
        if g.head[RTBU[s as usize] as usize] == NONE {
            continue;
        }
        walk(g, first, RTBU[s as usize], &mut seen, 1, &mut hits);
        if hits.contains(&MBNU) {
            // (lower severities may be discarded: the function is done, as in Tint)
            failed[s as usize] = true;
            break;
        }
        for &h in &hits {
            if h == CF_START {
                t.callsite = t.callsite.or(Some(s));
            } else if h >= FIRST {
                let k = ((h - FIRST) / 3) as usize;
                match (h - FIRST) % 3 {
                    0 => t.params[k].value_req = t.params[k].value_req.or(Some(s)),
                    1 => t.params[k].contents_req = t.params[k].contents_req.or(Some(s)),
                    _ => {}
                }
            }
        }
    }
    if f.ret.is_some() {
        walk(g, first, VALUE_RETURN, &mut seen, 2, &mut hits);
        for &h in &hits {
            if h == MBNU {
                t.ret_nonuniform = true;
            } else if h >= FIRST {
                let k = ((h - FIRST) / 3) as usize;
                match (h - FIRST) % 3 {
                    0 => t.params[k].ret_value = true,
                    1 => t.params[k].ret_contents = true,
                    _ => {}
                }
            }
        }
    }
    let mut round = 2;
    for k in 0..np as usize {
        if !matches!(m.types.get(f.params[k].ty as usize), Some(Type::Pointer { space: AddressSpace::Function, .. })) {
            continue;
        }
        round += 1;
        walk(g, first, FIRST + 3 * k as u32 + 2, &mut seen, round, &mut hits);
        hits.sort_unstable();
        hits.dedup();
        for &h in &hits {
            if h == MBNU {
                t.params[k].out_nonuniform = true;
            } else if h >= FIRST {
                let j = (h - FIRST) / 3;
                match (h - FIRST) % 3 {
                    0 => t.params[k].out_values.push(j),
                    1 => t.params[k].out_contents.push(j),
                    _ => {}
                }
            }
        }
    }
    (t, failed)
}

// ---------------------------------------------------------------- explaining a failure

/// the shared inputs of a run
struct Cx<'a> {
    m: &'a Module,
    src: &'a str,
    lm: &'a LineMap,
    filters: &'a Filters,
    scope: Scope,
}

/// the path RTBU[s] .. target as in Tint: depth first, the newest edge first; target first
fn path_to(g: &Graph, from: Node, target: Node) -> Vec<Node> {
    let mut parent = alloc::vec![NONE; g.head.len()];
    let mut stack = alloc::vec![from];
    parent[from as usize] = from;
    'search: while let Some(n) = stack.pop() {
        let start = stack.len();
        let mut ed = g.head[n as usize];
        while ed != NONE {
            let (to, next) = g.edges[ed as usize];
            if parent[to as usize] == NONE {
                parent[to as usize] = n;
                if to == target {
                    break 'search;
                }
                stack.push(to);
            }
            ed = next;
        }
        // (the list holds the newest edge first; the newest must be explored first)
        stack[start..].reverse();
    }
    let mut out = Vec::new();
    if parent[target as usize] == NONE {
        return out;
    }
    let mut cur = target;
    loop {
        out.push(cur);
        if cur == from {
            break;
        }
        cur = parent[cur as usize];
    }
    out
}

impl<'a> Cx<'a> {
    fn note(&self, f: &Function, e: ExprId, msg: String) -> Note {
        let sp = f.exprs.get(e as usize).map(|x| x.span).unwrap_or(f.span);
        self.lm.note(self.src, sp.start, sp.end, msg)
    }

    fn note_span(&self, sp: Span, msg: String) -> Note {
        self.lm.note(self.src, sp.start, sp.end, msg)
    }

    fn call_name(&self, f: &Function, e: ExprId) -> String {
        match f.exprs.get(e as usize).map(|x| &x.kind) {
            Some(ExprKind::Call(fi, _)) => self.m.functions.get(*fi as usize).map(|c| c.name.clone()).unwrap_or_default(),
            Some(ExprKind::Builtin(b, _)) => String::from(builtins::name(*b)),
            _ => String::new(),
        }
    }

    /// what made a value non-uniform
    fn source_note(&self, f: &Function, info: Info) -> Option<Note> {
        match info {
            Info::Input(e, k) => {
                let p = f.params.get(k as usize)?;
                let text = match p.io.builtin {
                    Some(_) => format!("der eingebaute Wert `{}` von `{}` ist möglicherweise nicht uniform", p.name, f.name),
                    None if matches!(self.m.types.get(p.ty as usize), Some(Type::Struct(_))) => format!("die Eingaben `{}` von `{}` sind möglicherweise nicht uniform", p.name, f.name),
                    None => format!("die Eingabe `{}` von `{}` ist möglicherweise nicht uniform", p.name, f.name),
                };
                Some(self.note(f, e, text))
            }
            Info::ReadGlobal(e, g) => {
                let v = self.m.globals.get(g as usize)?;
                let what = match v.space {
                    AddressSpace::Storage => format!("des `read_write`-Speicherpuffers `{}`", v.name),
                    AddressSpace::Workgroup => format!("der `workgroup`-Variable `{}`", v.name),
                    _ => format!("der privaten Variable `{}`", v.name),
                };
                Some(self.note(f, e, format!("das Lesen {} kann einen nicht uniformen Wert ergeben", what)))
            }
            Info::ReadParam(e, k) => {
                let p = f.params.get(k as usize)?;
                Some(self.note(f, e, format!("das Lesen über den Zeiger `{}` kann einen nicht uniformen Wert ergeben", p.name)))
            }
            Info::CallResult(e) => Some(self.note(f, e, format!("das Ergebnis von `{}` ist möglicherweise nicht uniform", self.call_name(f, e)))),
            Info::PtrResult(e, _) => Some(self.note(f, e, format!("nach dem Aufruf von `{}` ist der Speicher hinter dem Zeiger möglicherweise nicht uniform", self.call_name(f, e)))),
            Info::Param(k) => {
                let p = f.params.get(k as usize)?;
                let text = if matches!(self.m.types.get(p.ty as usize), Some(Type::Pointer { .. })) {
                    format!("der Parameter `{}` von `{}` zeigt möglicherweise auf einen nicht uniformen Wert", p.name, f.name)
                } else {
                    format!("der Parameter `{}` von `{}` ist möglicherweise nicht uniform", p.name, f.name)
                };
                Some(self.note_span(p.span, text))
            }
            Info::Contents(k) => {
                let p = f.params.get(k as usize)?;
                Some(self.note_span(p.span, format!("der Parameter `{}` von `{}` zeigt möglicherweise auf einen nicht uniformen Wert", p.name, f.name)))
            }
            _ => None,
        }
    }

    /// the failure of severity s in function fi
    fn explain(&self, fi: usize, s: u8, tags: &[FnTags]) -> Option<Diagnostic> {
        let mut frames: Vec<Vec<Note>> = Vec::new();
        let g = self.graph_of(fi, tags, Graph::default());
        let (mut fi, mut g, mut s, mut target) = (fi, g, s, MBNU);
        // the first call of a chain of call-site requirements and how long it is
        let mut chain: Option<usize> = None;
        let mut chain_len = 0usize;
        let (main_sp, main_msg) = loop {
            let f = self.m.functions.get(fi)?;
            let path = path_to(&g, RTBU[s as usize], target);
            if path.len() < 2 {
                return None;
            }
            let req = path[path.len() - 2];
            let mut notes: Vec<Note> = Vec::new();
            // the decision nearest to the source (as in Tint)
            let cond = path.iter().skip(1).find_map(|&n| match g.info[n as usize] {
                Info::Cond(e) => Some(e),
                _ => None,
            });
            let source = |this: &Cx, notes: &mut Vec<Note>| {
                if target == CF_START {
                    return;
                }
                if let Some(e) = cond {
                    notes.push(this.note(f, e, String::from("der Kontrollfluss hängt hier von einem möglicherweise nicht uniformen Wert ab")));
                }
                let at = if target == MBNU { path[1] } else { target };
                if let Some(n) = this.source_note(f, g.info[at as usize]) {
                    notes.push(n);
                }
            };
            match g.info[req as usize] {
                Info::CallReq(e) => match f.exprs.get(e as usize).map(|x| (&x.kind, x.span)) {
                    Some((ExprKind::Builtin(b, _), sp)) => {
                        source(self, &mut notes);
                        frames.push(notes);
                        let what = if is_subgroup(*b) && self.scope == Scope::Subgroup { "subgroup-uniformem" } else { "uniformem" };
                        break (sp, format!("`{}` darf nur in {} Kontrollfluss aufgerufen werden", builtins::name(*b), what));
                    }
                    Some((ExprKind::Call(fj, _), _)) => {
                        let fj = *fj as usize;
                        let next = tags.get(fj)?.callsite?;
                        if target != CF_START {
                            let cname = self.call_name(f, e);
                            notes.push(self.note(f, e, format!("über den Aufruf von `{}` in `{}`", cname, f.name)));
                            chain = Some(frames.len());
                            chain_len = 1;
                        } else {
                            chain_len += 1;
                        }
                        source(self, &mut notes);
                        frames.push(notes);
                        g = self.graph_of(fj, tags, g);
                        fi = fj;
                        s = next;
                        target = CF_START;
                    }
                    _ => return None,
                },
                Info::ArgReq(e, k, contents) => match f.exprs.get(e as usize).map(|x| (&x.kind, x.span)) {
                    Some((ExprKind::Builtin(b, args), sp)) => {
                        let at = args.get(k as usize).and_then(|a| f.exprs.get(*a as usize)).map(|a| a.span).unwrap_or(sp);
                        source(self, &mut notes);
                        frames.push(notes);
                        let arg = match b {
                            Builtin::WorkgroupUniformLoad => "p",
                            _ => crate::check_expr::subgroup_arg_name(*b),
                        };
                        break (at, format!("das Argument `{}` von `{}` muss uniform sein", arg, builtins::name(*b)));
                    }
                    Some((ExprKind::Call(fj, args), sp)) => {
                        let fj = *fj as usize;
                        let pt = tags.get(fj)?.params.get(k as usize)?;
                        let next = if contents { pt.contents_req? } else { pt.value_req? };
                        let at = args.get(k as usize).and_then(|a| f.exprs.get(*a as usize)).map(|a| a.span).unwrap_or(sp);
                        let by_ptr = contents || self.m.functions.get(fj).and_then(|c| c.params.get(k as usize)).is_some_and(|q| matches!(self.m.types.get(q.ty as usize), Some(Type::Pointer { .. })));
                        let text = if by_ptr { "hier wird ein möglicherweise nicht uniformer Wert über einen Zeiger übergeben" } else { "hier wird ein möglicherweise nicht uniformer Wert übergeben" };
                        notes.push(self.note_span(at, String::from(text)));
                        source(self, &mut notes);
                        frames.push(notes);
                        g = self.graph_of(fj, tags, g);
                        fi = fj;
                        s = next;
                        target = FIRST + 3 * k + contents as u32;
                        chain = None;
                    }
                    _ => return None,
                },
                _ => return None,
            }
        };
        if let (Some(c), true) = (chain, chain_len > 1) {
            if let Some(n) = frames.get_mut(c).and_then(|x| x.first_mut()) {
                n.msg = format!("indirekt {}", n.msg);
            }
        }
        // innermost first; long chains lose their middle
        let mut notes: Vec<Note> = frames.into_iter().rev().flatten().collect();
        if notes.len() > MAX_NOTES {
            let cut = notes.len() - MAX_NOTES;
            notes.drain(MAX_NOTES / 2..MAX_NOTES / 2 + cut);
        }
        let mut d = self.lm.diag(self.src, main_sp.start, main_sp.end, s == 0, main_msg);
        d.info = s == 2;
        d.notes = notes;
        Some(d)
    }

    fn graph_of(&self, fi: usize, tags: &[FnTags], g: Graph) -> Graph {
        let mut b = Build::new(self.m, fi, tags, self.scope, self.filters, g);
        b.run();
        b.g
    }
}

// ---------------------------------------------------------------- the module

/// one run of the analysis: the diagnostics go to `out`; true when an error was found
fn run(cx: &Cx, out: &mut Vec<Diagnostic>) -> bool {
    let n = cx.m.functions.len();
    let mut tags: Vec<FnTags> = Vec::with_capacity(n);
    let mut g = Graph::default();
    for fi in 0..n {
        let mut b = Build::new(cx.m, fi, &tags, cx.scope, cx.filters, g);
        b.run();
        g = b.g;
        let (t, failed) = tags_of(&cx.m.functions[fi], &g, cx.m);
        for s in 0..3u8 {
            if !failed[s as usize] {
                continue;
            }
            if s == 0 || out.len() < MAX_REPORTS {
                if let Some(d) = cx.explain(fi, s, &tags) {
                    out.push(d);
                }
            }
            if s == 0 {
                return true;
            }
        }
        tags.push(t);
    }
    false
}

/// the analysis of a checked module: the failures as diagnostics (errors, warnings, infos)
pub fn analyze(m: &Module, src: &str, lm: &LineMap, features: u32) -> Vec<Diagnostic> {
    let mut filters = Filters::default();
    for d in &m.diagnostic_filters {
        match d.rule.as_str() {
            "derivative_uniformity" => filters.derivative.push(d.clone()),
            "subgroup_uniformity" => filters.subgroup.push(d.clone()),
            _ => {}
        }
    }
    let scoped = features & feature::SUBGROUP_UNIFORMITY != 0;
    let mut out = Vec::new();
    let main = Cx { m, src, lm, filters: &filters, scope: Scope::Main(scoped) };
    let failed = run(&main, &mut out);
    // the subgroup scope matters only when the module calls a subgroup or quad function
    if !failed && scoped && m.functions.iter().any(|f| f.exprs.iter().any(|e| matches!(&e.kind, ExprKind::Builtin(b, _) if is_subgroup(*b)))) {
        let sub = Cx { m, src, lm, filters: &filters, scope: Scope::Subgroup };
        run(&sub, &mut out);
    }
    out
}

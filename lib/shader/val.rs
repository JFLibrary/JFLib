// val -- a structural validator for SPIR-V modules as Vulkan consumes them.  It checks what
// a module builder can get wrong; it is not spirv-val (docs/SHADER.md lists what it leaves out).
//
//   validate(words) -> Result<(), Vec<Error>>    Error { rule, at (word offset), msg }
//
// Rules (Rule::*):
//   Header      magic, version 1.0 ..= 1.6, schema 0, every id below the bound
//   Grammar     known opcodes, operand words that fit the grammar (spv::OPS)
//   Section     the logical layout: capabilities, extensions, imports, one memory model,
//               entry points, execution modes, debug, annotations, types / constants /
//               globals, functions; nothing module-level inside a function
//   IdUnique    every result id defined once
//   IdDefined   every used id defined, before its use (except names, decorations, entry
//               points, execution modes, labels, phis, called functions), function-local ids
//               only inside their function
//   IdKind      ids of the right kind: result types are types, labels are labels, operands
//               are values, callees are functions, extended sets are imports
//   Type        types well formed (vector / matrix shapes, array lengths -- constants or
//               specialization constants --, runtime arrays last) and type agreement of the
//               instructions shader emits: arithmetic, comparisons, logic, select,
//               conversions, access chains, load / store, composites, shuffles, products,
//               GLSL.std.450, atomics, barriers, images (coordinate sizes, Dref, the image
//               kinds each query takes), derivatives, calls, phis, returns, constants,
//               specialization constants (OpSpecConstantOp: an operation SPIR-V allows for
//               Shader, constant operands, its operation's type rules), variables
//   Decoration  decoration targets of the right kind (Offset on struct members, ArrayStride
//               on arrays, Block on structs, Location on Input / Output, SpecId on scalar
//               specialization constants and each value once, BuiltIn on a constant only as
//               a uvec3 WorkgroupSize ...), member indices
//   Storage     variables: storage class of the pointer, Function variables first in the
//               entry block, Vulkan's resource rules (Block structs in Uniform /
//               StorageBuffer / PushConstant, DescriptorSet + Binding on resources, Location
//               on user Input / Output, images in UniformConstant), no stores to read-only
//               storage, atomics only on buffer / workgroup / image memory
//   BlockLayout Offset on every block member, offsets aligned per std140 (Uniform) / std430
//               (StorageBuffer, PushConstant) and not overlapping, ArrayStride / MatrixStride
//               present, aligned and not smaller than the element, ColMajor / RowMajor on
//               matrices, push constants at most 128 bytes
//   Cfg         blocks start with OpLabel and end with one terminator, merge instructions
//               right before the branch they belong to, merge / continue targets in the
//               function, a merge block per header at most once and after it, conditional
//               branches without a merge only as break / continue, back edges only to loop
//               headers, a back edge per loop, the entry block no branch target, phis first in
//               their block with predecessors as parents
//   Interface   OpEntryPoint: a void function without parameters, the interface lists only
//               globals, before 1.4 only Input / Output ones, from 1.4 each at most once; it
//               contains every global the call tree uses (1.4+) or every Input / Output one
//   ExecMode    LocalSize (or LocalSizeId with 32-bit integer constant operands, or the
//               WorkgroupSize built-in) for GLCompute, id modes only in OpExecutionModeId,
//               OriginUpperLeft (and no OriginLowerLeft) for
//               Fragment, modes for the right models, DepthReplacing when FragDepth is
//               written, fragment-only instructions (discard, demote, derivatives,
//               implicit-LOD sampling) only under Fragment entry points, Workgroup
//               variables only under GLCompute
//   Capability  the capabilities types, instructions, built-ins, images and models need
//   Version     instructions and extensions of the module's SPIR-V version (TerminateInvocation
//               and DemoteToHelperInvocation before 1.6, CopyLogical, composite Select and
//               UConvert in OpSpecConstantOp before 1.4, OpExecutionModeId before 1.2,
//               descriptor indexing before 1.5)

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::layout::{self, Rule as Lay};
use crate::module::{Id, Ty, Types};
use crate::parse::{self, Inst};
use crate::spv::{self, op, Bi, Cap, Dec, Model, Mode, Sc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Header,
    Grammar,
    Section,
    IdUnique,
    IdDefined,
    IdKind,
    Type,
    Decoration,
    Storage,
    BlockLayout,
    Cfg,
    Interface,
    ExecMode,
    Capability,
    Version,
}

#[derive(Clone, Debug)]
pub struct Error {
    pub rule: Rule,
    /// the word offset of the instruction (0: the module)
    pub at: usize,
    pub msg: String,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{:?} (word {}): {}", self.rule, self.at, self.msg)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum K {
    Type,
    Const,
    Global,
    Local,
    Value,
    Label,
    Func,
    ExtSet,
    Other,
}

#[derive(Clone, Copy)]
struct Def {
    k: K,
    inst: usize,
    func: usize,
}

const NOFN: usize = usize::MAX;

struct FnInfo {
    id: u32,
    start: usize,
    end: usize,
    ret: u32,
    globals: BTreeSet<u32>,
    calls: BTreeSet<u32>,
    frag_only: Vec<(usize, &'static str)>,
}

struct V<'a> {
    ins: Vec<Inst<'a>>,
    minor: u32,
    errs: Vec<Error>,
    defs: BTreeMap<u32, Def>,
    tys: BTreeMap<u32, Ty>,
    vty: BTreeMap<u32, u32>,
    consts: BTreeMap<u32, u32>,
    caps: BTreeSet<u32>,
    exts: BTreeSet<String>,
    sets: BTreeMap<u32, String>,
    decs: BTreeMap<u32, Vec<(u32, Vec<u32>)>>,
    mdecs: BTreeMap<(u32, u32), Vec<(u32, Vec<u32>)>>,
    funcs: Vec<FnInfo>,
    need: BTreeMap<u32, usize>,
}

impl Types for V<'_> {
    fn ty(&self, id: u32) -> Option<&Ty> {
        self.tys.get(&id)
    }
    fn const_u32(&self, id: u32) -> Option<u32> {
        self.consts.get(&id).copied()
    }
}

/// a scalar or vector type: (kind 'f' / 'i' / 'b', width, components)
#[derive(Clone, Copy, PartialEq, Eq)]
struct Sv {
    k: char,
    width: u32,
    n: u32,
}

fn is_type_op(o: u16) -> bool {
    (op::TypeVoid..=op::TypeFunction).contains(&o) || o == 39
}

fn is_const_op(o: u16) -> bool {
    (op::ConstantTrue..=op::SpecConstantComposite).contains(&o) || o == 52
}

fn is_terminator(o: u16) -> bool {
    matches!(o, op::Branch | op::BranchConditional | op::Switch | op::Return | op::ReturnValue | op::Kill
        | op::Unreachable | op::TerminateInvocation)
}

/// the logical-layout section of a module-level instruction
fn section(o: u16) -> u32 {
    match o {
        op::Capability => 0,
        op::Extension => 1,
        op::ExtInstImport => 2,
        op::MemoryModel => 3,
        op::EntryPoint => 4,
        op::ExecutionMode | op::ExecutionModeId => 5,
        op::String | op::Source | op::Name | op::MemberName | op::ModuleProcessed => 6,
        op::Decorate | op::MemberDecorate => 7,
        op::Function => 9,
        _ => 8,
    }
}

fn has_dec(list: Option<&Vec<(u32, Vec<u32>)>>, d: Dec) -> Option<&Vec<u32>> {
    list?.iter().find(|x| x.0 == d as u32).map(|x| &x.1)
}

impl<'a> V<'a> {
    fn err(&mut self, rule: Rule, k: usize, msg: String) {
        let at = self.ins.get(k).map_or(0, |i| i.at);
        self.errs.push(Error { rule, at, msg });
    }

    fn name(&self, k: usize) -> &'static str {
        self.ins[k].info.map_or("?", |i| i.name)
    }

    fn kind(&self, id: u32) -> Option<K> {
        self.defs.get(&id).map(|d| d.k)
    }

    fn t(&self, id: u32) -> Option<&Ty> {
        self.tys.get(&id)
    }

    fn sv(&self, t: u32) -> Option<Sv> {
        match self.t(t)? {
            Ty::Bool => Some(Sv { k: 'b', width: 0, n: 1 }),
            Ty::Int { width, .. } => Some(Sv { k: 'i', width: *width, n: 1 }),
            Ty::Float { width } => Some(Sv { k: 'f', width: *width, n: 1 }),
            Ty::Vector { elem, n } => self.sv(elem.0).map(|s| Sv { n: *n, ..s }),
            _ => None,
        }
    }

    fn is_sv(&self, t: u32, k: char) -> bool {
        self.sv(t).is_some_and(|s| s.k == k)
    }

    fn scalar(&self, t: u32) -> u32 {
        match self.t(t) {
            Some(Ty::Vector { elem, .. }) => elem.0,
            _ => t,
        }
    }

    fn sc_of_ptr(&self, t: u32) -> Option<(u32, u32)> {
        match self.t(t)? {
            Ty::Pointer { sc, pointee } => Some((*sc, pointee.0)),
            _ => None,
        }
    }

    /// the type of a value operand (IdKind error when it is no value)
    fn val(&mut self, k: usize, id: u32) -> u32 {
        match self.kind(id) {
            Some(K::Const | K::Global | K::Local | K::Value) => self.vty.get(&id).copied().unwrap_or(0),
            Some(other) => {
                let n = self.name(k);
                self.err(Rule::IdKind, k, format!("Op{}: %{} is a {:?}, not a value", n, id, other));
                0
            }
            None => 0,
        }
    }

    fn want(&mut self, k: usize, cond: bool, msg: &str) {
        if !cond {
            let n = self.name(k);
            self.err(Rule::Type, k, format!("Op{}: {}", n, msg));
        }
    }

    fn need_cap(&mut self, c: Cap, k: usize) {
        self.need.entry(c as u32).or_insert(k);
    }

    // ---------------------------------------------------------------- module structure

    fn structure(&mut self) {
        let mut sec = 0;
        let mut cur = NOFN;
        let mut models = 0;
        for k in 0..self.ins.len() {
            let i = self.ins[k];
            if i.info.is_none() {
                self.err(Rule::Grammar, k, format!("unknown opcode {}", i.op));
            } else if !parse::operands(&i, &mut |_, _| {}) {
                let n = self.name(k);
                self.err(Rule::Grammar, k, format!("Op{}: the operands do not fit its grammar", n));
            }
            if cur == NOFN {
                let s = section(i.op);
                if s < sec {
                    let n = self.name(k);
                    self.err(Rule::Section, k, format!("Op{} is out of the logical layout order", n));
                }
                sec = sec.max(s);
                if i.op == op::MemoryModel {
                    models += 1;
                }
                if i.op == op::FunctionEnd || (s == 8 && i.info.is_some() && !is_type_op(i.op) && !is_const_op(i.op)
                    && !matches!(i.op, op::Variable | op::Undef | op::Line))
                {
                    let n = self.name(k);
                    self.err(Rule::Section, k, format!("Op{} outside a function", n));
                }
                if i.op == op::Function {
                    cur = self.funcs.len();
                    self.funcs.push(FnInfo { id: i.res, start: k, end: k, ret: i.ty, globals: BTreeSet::new(),
                                             calls: BTreeSet::new(), frag_only: Vec::new() });
                }
            } else if i.op == op::FunctionEnd {
                self.funcs[cur].end = k;
                cur = NOFN;
            } else if i.op == op::Function || section(i.op) < 8 || is_type_op(i.op) || is_const_op(i.op) {
                let n = self.name(k);
                self.err(Rule::Section, k, format!("Op{} inside a function", n));
            }
            if i.res != 0 {
                let kd = match i.op {
                    o if is_type_op(o) => K::Type,
                    o if is_const_op(o) => K::Const,
                    op::Variable if cur == NOFN => K::Global,
                    op::Variable => K::Local,
                    op::Label => K::Label,
                    op::Function => K::Func,
                    op::ExtInstImport => K::ExtSet,
                    op::String => K::Other,
                    _ => K::Value,
                };
                if self.defs.contains_key(&i.res) {
                    self.err(Rule::IdUnique, k, format!("%{} is defined twice", i.res));
                } else {
                    let func = if cur == NOFN || i.op == op::Function { NOFN } else { cur };
                    self.defs.insert(i.res, Def { k: kd, inst: k, func });
                }
                if i.ty != 0 {
                    self.vty.insert(i.res, i.ty);
                }
                if kd == K::Type {
                    self.type_def(k);
                }
                if i.op == op::Constant {
                    if let (Some(Ty::Int { width: 32, .. }), [v]) = (self.tys.get(&i.ty), i.ops) {
                        self.consts.insert(i.res, *v);
                    }
                }
            }
            match i.op {
                op::Capability if !i.ops.is_empty() => {
                    self.caps.insert(i.ops[0]);
                }
                op::Extension => {
                    self.exts.insert(parse::string(i.ops).0);
                }
                op::ExtInstImport => {
                    self.sets.insert(i.res, parse::string(i.ops).0);
                }
                _ => {}
            }
        }
        if cur != NOFN {
            let n = self.ins.len().saturating_sub(1);
            self.err(Rule::Section, n, String::from("a function without OpFunctionEnd"));
        }
        if models != 1 {
            self.err(Rule::Section, 0, format!("{} OpMemoryModel instructions (one is required)", models));
        }
    }

    fn type_def(&mut self, k: usize) {
        let i = self.ins[k];
        let o = i.ops;
        let g = |n: usize| o.get(n).copied().unwrap_or(0);
        let t = match i.op {
            op::TypeVoid => Ty::Void,
            op::TypeBool => Ty::Bool,
            op::TypeInt => Ty::Int { width: g(0), signed: g(1) != 0 },
            op::TypeFloat => Ty::Float { width: g(0) },
            op::TypeVector => Ty::Vector { elem: Id(g(0)), n: g(1) },
            op::TypeMatrix => Ty::Matrix { col: Id(g(0)), cols: g(1) },
            op::TypeArray => Ty::Array { elem: Id(g(0)), len: Id(g(1)) },
            op::TypeRuntimeArray => Ty::RuntimeArray { elem: Id(g(0)) },
            op::TypeStruct => Ty::Struct { members: o.iter().map(|&m| Id(m)).collect() },
            op::TypePointer => Ty::Pointer { sc: g(0), pointee: Id(g(1)) },
            op::TypeFunction => Ty::Function { ret: Id(g(0)), params: o.iter().skip(1).map(|&m| Id(m)).collect() },
            op::TypeImage => Ty::Image { sampled: Id(g(0)), dim: g(1), depth: g(2), arrayed: g(3), ms: g(4), usage: g(5),
                                         format: g(6) },
            op::TypeSampler => Ty::Sampler,
            op::TypeSampledImage => Ty::SampledImage { image: Id(g(0)) },
            _ => return,
        };
        self.tys.insert(i.res, t);
    }

    fn uses(&mut self) {
        let mut cur = NOFN;
        for k in 0..self.ins.len() {
            let i = self.ins[k];
            if i.op == op::Function {
                cur = self.funcs.iter().position(|f| f.start == k).unwrap_or(NOFN);
            }
            let fwd_ok = matches!(i.op, op::Name | op::MemberName | op::Decorate | op::MemberDecorate | op::EntryPoint
                | op::ExecutionMode | op::ExecutionModeId | op::Phi | op::Branch | op::BranchConditional | op::Switch
                | op::SelectionMerge | op::LoopMerge | op::FunctionCall);
            let module_ref = cur == NOFN || matches!(i.op, op::Name | op::Decorate | op::MemberName);
            let mut used = parse::ids(&i);
            if i.ty != 0 {
                used.push(i.ty);
            }
            for id in used {
                let Some(d) = self.defs.get(&id).copied() else {
                    let n = self.name(k);
                    self.err(Rule::IdDefined, k, format!("Op{} uses %{}, which is not defined", n, id));
                    continue;
                };
                if d.inst > k && !fwd_ok && d.k != K::Label && d.k != K::Func {
                    let n = self.name(k);
                    self.err(Rule::IdDefined, k, format!("Op{} uses %{} before its definition", n, id));
                }
                if !module_ref && d.func != NOFN && d.func != cur {
                    let n = self.name(k);
                    self.err(Rule::IdDefined, k, format!("Op{} uses %{} of another function", n, id));
                }
                if cur != NOFN && i.op != op::Function {
                    if d.k == K::Global {
                        self.funcs[cur].globals.insert(id);
                    }
                }
            }
            if cur != NOFN && i.op == op::FunctionCall && !i.ops.is_empty() {
                self.funcs[cur].calls.insert(i.ops[0]);
            }
            if i.ty != 0 && self.kind(i.ty).is_some_and(|x| x != K::Type) {
                let n = self.name(k);
                self.err(Rule::IdKind, k, format!("Op{}: the result type %{} is not a type", n, i.ty));
            }
            if i.op == op::FunctionEnd {
                cur = NOFN;
            }
        }
    }

    // ---------------------------------------------------------------- types

    fn types(&mut self) {
        let ids: Vec<u32> = self.tys.keys().copied().collect();
        for id in ids {
            let k = self.defs[&id].inst;
            let t = self.tys[&id].clone();
            let is_ty = |v: &V, x: Id| v.kind(x.0) == Some(K::Type);
            match t {
                Ty::Int { width, .. } => {
                    self.want(k, matches!(width, 8 | 16 | 32 | 64), "integer width must be 8, 16, 32 or 64");
                    match width {
                        8 => self.need_cap(Cap::Int8, k),
                        16 => self.need_cap(Cap::Int16, k),
                        64 => self.need_cap(Cap::Int64, k),
                        _ => {}
                    }
                }
                Ty::Float { width } => {
                    self.want(k, matches!(width, 16 | 32 | 64), "float width must be 16, 32 or 64");
                    match width {
                        16 => self.need_cap(Cap::Float16, k),
                        64 => self.need_cap(Cap::Float64, k),
                        _ => {}
                    }
                }
                Ty::Vector { elem, n } => {
                    let ok = self.sv(elem.0).is_some_and(|s| s.n == 1);
                    self.want(k, ok, "the component type must be a scalar");
                    self.want(k, (2..=4).contains(&n), "a vector has 2, 3 or 4 components");
                }
                Ty::Matrix { col, cols } => {
                    let ok = self.sv(col.0).is_some_and(|s| s.k == 'f' && s.n >= 2);
                    self.want(k, ok, "the column type must be a float vector");
                    self.want(k, (2..=4).contains(&cols), "a matrix has 2, 3 or 4 columns");
                    self.need_cap(Cap::Matrix, k);
                }
                Ty::Array { elem, len } => {
                    let e_ok = is_ty(self, elem) && !matches!(self.t(elem.0), Some(Ty::Void | Ty::RuntimeArray { .. }));
                    self.want(k, e_ok, "the element type must be a sized type");
                    let l = self.consts.get(&len.0).copied();
                    let spec = self.defs.get(&len.0).is_some_and(|d| matches!(self.ins[d.inst].op, op::SpecConstant | op::SpecConstantOp));
                    let int = self.vty.get(&len.0).is_some_and(|&t| matches!(self.t(t), Some(Ty::Int { .. })));
                    if self.kind(len.0) != Some(K::Const) || !(l.is_some() || spec && int) {
                        self.want(k, false, "the length must be a 32-bit integer constant or an integer specialization constant");
                    } else if l.is_some() {
                        self.want(k, l != Some(0), "the length must be at least 1");
                    }
                }
                Ty::RuntimeArray { elem } => {
                    let e_ok = is_ty(self, elem) && !matches!(self.t(elem.0), Some(Ty::Void | Ty::RuntimeArray { .. }));
                    self.want(k, e_ok, "the element type must be a sized type");
                }
                Ty::Struct { members } => {
                    for (n, m) in members.iter().enumerate() {
                        self.want(k, is_ty(self, *m) && !matches!(self.t(m.0), Some(Ty::Void)), "members must be non-void types");
                        if matches!(self.t(m.0), Some(Ty::RuntimeArray { .. })) && n + 1 != members.len() {
                            self.want(k, false, "a runtime array must be the last member");
                        }
                    }
                }
                Ty::Pointer { pointee, .. } => self.want(k, is_ty(self, pointee), "the pointee must be a type"),
                Ty::Function { ret, params } => {
                    self.want(k, is_ty(self, ret), "the return type must be a type");
                    for p in params {
                        self.want(k, is_ty(self, p), "parameter types must be types");
                    }
                }
                Ty::Image { sampled, dim, arrayed, usage, .. } => {
                    let ok = matches!(self.t(sampled.0), Some(Ty::Void | Ty::Int { .. } | Ty::Float { .. }));
                    self.want(k, ok, "the sampled type must be void or a scalar number");
                    self.want(k, usage <= 2, "Sampled must be 0, 1 or 2");
                    match (dim, arrayed, usage) {
                        (0, _, 1) => self.need_cap(Cap::Sampled1D, k),
                        (0, _, _) => self.need_cap(Cap::Image1D, k),
                        (5, _, 1) => self.need_cap(Cap::SampledBuffer, k),
                        (5, _, _) => self.need_cap(Cap::ImageBuffer, k),
                        (3, 1, 1) => self.need_cap(Cap::SampledCubeArray, k),
                        (3, 1, _) => self.need_cap(Cap::ImageCubeArray, k),
                        _ => {}
                    }
                }
                Ty::SampledImage { image } => {
                    self.want(k, matches!(self.t(image.0), Some(Ty::Image { .. })), "the image type must be an OpTypeImage")
                }
                Ty::Void | Ty::Bool | Ty::Sampler => {}
            }
        }
    }

    // ---------------------------------------------------------------- decorations

    fn decorations(&mut self) {
        let mut spec_ids: BTreeMap<u32, u32> = BTreeMap::new();
        for k in 0..self.ins.len() {
            let i = self.ins[k];
            match i.op {
                op::Decorate if i.ops.len() >= 2 => {
                    let (target, d) = (i.ops[0], i.ops[1]);
                    let args = i.ops[2..].to_vec();
                    let kd = self.kind(target);
                    let is_var = matches!(kd, Some(K::Global | K::Local));
                    let ok = match d {
                        x if x == Dec::Block as u32 || x == Dec::BufferBlock as u32 => {
                            matches!(self.t(target), Some(Ty::Struct { .. }))
                        }
                        x if x == Dec::ArrayStride as u32 => {
                            matches!(self.t(target), Some(Ty::Array { .. } | Ty::RuntimeArray { .. } | Ty::Pointer { .. }))
                        }
                        x if x == Dec::Offset as u32 || x == Dec::MatrixStride as u32 || x == Dec::ColMajor as u32
                            || x == Dec::RowMajor as u32 => false,
                        x if x == Dec::Location as u32 || x == Dec::Component as u32 || x == Dec::Flat as u32
                            || x == Dec::NoPerspective as u32 || x == Dec::Centroid as u32 => {
                            let sc = self.vty.get(&target).and_then(|&t| self.sc_of_ptr(t)).map(|p| p.0);
                            kd == Some(K::Global) && matches!(sc, Some(1 | 3))
                        }
                        x if x == Dec::Binding as u32 || x == Dec::DescriptorSet as u32 => {
                            let sc = self.vty.get(&target).and_then(|&t| self.sc_of_ptr(t)).map(|p| p.0);
                            kd == Some(K::Global) && matches!(sc, Some(0 | 2 | 12))
                        }
                        x if x == Dec::BuiltIn as u32 => {
                            let wg = args.first() == Some(&(Bi::WorkgroupSize as u32));
                            if kd == Some(K::Const) || wg {
                                // Vulkan: only WorkgroupSize decorates a constant, a uvec3 of 32-bit ints
                                let t = self.vty.get(&target).copied().unwrap_or(0);
                                wg && kd == Some(K::Const) && self.sv(t).is_some_and(|s| s.k == 'i' && s.n == 3 && s.width == 32)
                            } else {
                                is_var
                            }
                        }
                        x if x == Dec::SpecId as u32 => {
                            let def = self.defs.get(&target).map(|d| self.ins[d.inst].op);
                            if let (Some(&v), true) = (args.first(), def.is_some()) {
                                if let Some(other) = spec_ids.insert(v, target) {
                                    self.err(Rule::Decoration, k, format!("SpecId {} on both %{} and %{}", v, other, target));
                                }
                            }
                            matches!(def, Some(op::SpecConstant | op::SpecConstantTrue | op::SpecConstantFalse))
                        }
                        x if x == Dec::NonUniform as u32 => {
                            self.need_cap(Cap::ShaderNonUniform, k);
                            if self.minor < 5 && !self.exts.contains("SPV_EXT_descriptor_indexing") {
                                self.err(Rule::Version, k, String::from("NonUniform needs SPIR-V 1.5 or SPV_EXT_descriptor_indexing"));
                            }
                            kd == Some(K::Value) || is_var
                        }
                        x if x == Dec::NoContraction as u32 => kd == Some(K::Value),
                        x if x == Dec::NonWritable as u32 || x == Dec::NonReadable as u32 || x == Dec::Restrict as u32
                            || x == Dec::Aliased as u32 || x == Dec::Coherent as u32 || x == Dec::Volatile as u32 => {
                            is_var || kd == Some(K::Value)
                        }
                        _ => kd.is_some(),
                    };
                    if !ok {
                        let dn = spv::name(spv::DECS, d).unwrap_or("?");
                        self.err(Rule::Decoration, k, format!("decoration {} cannot be applied to %{}", dn, target));
                    }
                    self.decs.entry(target).or_default().push((d, args));
                }
                op::MemberDecorate if i.ops.len() >= 3 => {
                    let (target, m, d) = (i.ops[0], i.ops[1], i.ops[2]);
                    let args = i.ops[3..].to_vec();
                    match self.t(target) {
                        Some(Ty::Struct { members }) if (m as usize) < members.len() => {}
                        Some(Ty::Struct { .. }) => {
                            self.err(Rule::Decoration, k, format!("member {} of %{} does not exist", m, target))
                        }
                        _ => self.err(Rule::Decoration, k, format!("OpMemberDecorate on %{}, which is not a struct", target)),
                    }
                    let bad = [Dec::Block, Dec::BufferBlock, Dec::ArrayStride, Dec::DescriptorSet, Dec::Binding]
                        .iter()
                        .any(|&x| x as u32 == d);
                    if bad {
                        let dn = spv::name(spv::DECS, d).unwrap_or("?");
                        self.err(Rule::Decoration, k, format!("decoration {} cannot be applied to a struct member", dn));
                    }
                    self.mdecs.entry((target, m)).or_default().push((d, args));
                }
                op::MemberName if i.ops.len() >= 2 => {
                    let ok = matches!(self.t(i.ops[0]), Some(Ty::Struct { members }) if (i.ops[1] as usize) < members.len());
                    if !ok {
                        self.err(Rule::Decoration, k, format!("OpMemberName: member {} of %{} does not exist", i.ops[1], i.ops[0]));
                    }
                }
                _ => {}
            }
        }
        // built-ins and the capabilities they need
        let builtins: Vec<(u32, u32)> = self.decs.iter()
            .flat_map(|(&t, l)| l.iter().filter(|d| d.0 == Dec::BuiltIn as u32).map(move |d| (t, d.1.first().copied().unwrap_or(0))))
            .chain(self.mdecs.iter().flat_map(|(&(t, _), l)| {
                l.iter().filter(|d| d.0 == Dec::BuiltIn as u32).map(move |d| (t, d.1.first().copied().unwrap_or(0)))
            }))
            .collect();
        for (t, b) in builtins {
            let k = self.defs.get(&t).map_or(0, |d| d.inst);
            match b {
                x if x == Bi::ClipDistance as u32 => self.need_cap(Cap::ClipDistance, k),
                x if x == Bi::CullDistance as u32 => self.need_cap(Cap::CullDistance, k),
                x if x == Bi::BaseVertex as u32 || x == Bi::BaseInstance as u32 || x == Bi::DrawIndex as u32 => {
                    self.need_cap(Cap::DrawParameters, k)
                }
                x if x == Bi::SampleId as u32 || x == Bi::SamplePosition as u32 => self.need_cap(Cap::SampleRateShading, k),
                x if x == Bi::ViewIndex as u32 => self.need_cap(Cap::MultiView, k),
                _ => {}
            }
        }
    }

    fn is_builtin_var(&self, v: u32, pointee: u32) -> bool {
        if has_dec(self.decs.get(&v), Dec::BuiltIn).is_some() {
            return true;
        }
        let mut t = pointee;
        while let Some(Ty::Array { elem, .. }) = self.t(t) {
            t = elem.0;
        }
        match self.t(t) {
            Some(Ty::Struct { members }) => (0..members.len() as u32)
                .any(|m| has_dec(self.mdecs.get(&(t, m)), Dec::BuiltIn).is_some()),
            _ => false,
        }
    }

    // ---------------------------------------------------------------- variables and layouts

    fn variables(&mut self) {
        for k in 0..self.ins.len() {
            let i = self.ins[k];
            if i.op != op::Variable || i.ops.is_empty() {
                continue;
            }
            let sc = i.ops[0];
            let Some((psc, pointee)) = self.sc_of_ptr(i.ty) else {
                self.err(Rule::Type, k, String::from("OpVariable: the result type must be a pointer"));
                continue;
            };
            if psc != sc {
                self.err(Rule::Storage, k, format!("OpVariable: storage class {} differs from its pointer's {}", sc, psc));
            }
            if let Some(&init) = i.ops.get(1) {
                let it = self.val(k, init);
                self.want(k, it == pointee, "the initializer's type must be the pointee type");
            }
            let local = self.defs.get(&i.res).is_some_and(|d| d.k == K::Local);
            if local != (sc == Sc::Function as u32) {
                self.err(Rule::Storage, k, String::from(if local {
                    "a variable inside a function must have storage class Function"
                } else {
                    "storage class Function is only allowed inside functions"
                }));
            }
            if local {
                // only OpVariable (or OpLine) between the entry block's OpLabel and here
                let f = self.defs[&i.res].func;
                let start = self.funcs[f].start;
                let mut j = start + 1;
                while j < k && self.ins[j].op == op::FunctionParameter {
                    j += 1;
                }
                let ok = self.ins.get(j).is_some_and(|x| x.op == op::Label)
                    && self.ins[j + 1..k].iter().all(|x| x.op == op::Variable || x.op == op::Line);
                if !ok {
                    self.err(Rule::Storage, k, String::from("Function variables must come first in the entry block"));
                }
                continue;
            }
            self.vulkan_var(k, i.res, sc, pointee);
        }
    }

    fn vulkan_var(&mut self, k: usize, v: u32, sc: u32, pointee: u32) {
        if matches!(sc, 0 | 2 | 12) && matches!(self.t(pointee), Some(Ty::RuntimeArray { .. })) {
            self.need_cap(Cap::RuntimeDescriptorArray, k);
            if self.minor < 5 && !self.exts.contains("SPV_EXT_descriptor_indexing") {
                self.err(Rule::Version, k, String::from("a runtime array of resources needs SPIR-V 1.5 or SPV_EXT_descriptor_indexing"));
            }
        }
        let set = has_dec(self.decs.get(&v), Dec::DescriptorSet).is_some();
        let bind = has_dec(self.decs.get(&v), Dec::Binding).is_some();
        let mut inner = pointee;
        while let Some(Ty::Array { elem, .. } | Ty::RuntimeArray { elem }) = self.t(inner) {
            inner = elem.0;
        }
        match sc {
            0 => {
                if !matches!(self.t(inner), Some(Ty::Image { .. } | Ty::Sampler | Ty::SampledImage { .. })) {
                    self.err(Rule::Storage, k, String::from("UniformConstant variables must be images or samplers"));
                }
                if !set || !bind {
                    self.err(Rule::Storage, k, format!("the resource %{} needs DescriptorSet and Binding", v));
                }
            }
            2 | 12 | 9 => {
                let block = has_dec(self.decs.get(&inner), Dec::Block).is_some();
                let bblock = has_dec(self.decs.get(&inner), Dec::BufferBlock).is_some();
                let s = if sc == 2 { "Uniform" } else if sc == 12 { "StorageBuffer" } else { "PushConstant" };
                if !matches!(self.t(inner), Some(Ty::Struct { .. })) || !(block || (sc == 2 && bblock && self.minor < 4)) {
                    self.err(Rule::Storage, k, format!("{} variables must point to a struct decorated Block", s));
                } else {
                    let rule = if sc == 2 && !bblock { Lay::Std140 } else { Lay::Std430 };
                    let size = self.block_layout(k, inner, rule, true);
                    if sc == 9 && size > 128 {
                        self.err(Rule::BlockLayout, k, format!("the push-constant block has {} bytes (at most 128)", size));
                    }
                }
                if sc == 9 && (set || bind) {
                    self.err(Rule::Storage, k, String::from("push constants have no DescriptorSet or Binding"));
                }
                if sc != 9 && (!set || !bind) {
                    self.err(Rule::Storage, k, format!("the buffer %{} needs DescriptorSet and Binding", v));
                }
                if self.minor < 3 && sc == 12 && !self.exts.contains("SPV_KHR_storage_buffer_storage_class") {
                    self.err(Rule::Version, k, String::from("StorageBuffer needs SPIR-V 1.3 or SPV_KHR_storage_buffer_storage_class"));
                }
            }
            1 | 3 => {
                let loc = has_dec(self.decs.get(&v), Dec::Location).is_some();
                let member_loc = matches!(self.t(inner), Some(Ty::Struct { members })
                    if (0..members.len() as u32).all(|m| has_dec(self.mdecs.get(&(inner, m)), Dec::Location).is_some()));
                if !loc && !member_loc && !self.is_builtin_var(v, pointee) {
                    self.err(Rule::Storage, k, format!("the Input / Output variable %{} needs a Location or BuiltIn", v));
                }
            }
            _ => {}
        }
    }

    /// the declared size of a type inside a block (strides and offsets as decorated)
    fn decl_size(&self, t: u32, rule: Lay) -> Option<u32> {
        match self.t(t)? {
            Ty::Array { len, elem } => {
                let s = has_dec(self.decs.get(&t), Dec::ArrayStride).and_then(|a| a.first().copied())
                    .or_else(|| layout::stride(self, elem.0, rule))?;
                Some(self.consts.get(&len.0)? * s)
            }
            Ty::Struct { members } => {
                let mut end = 0;
                for m in 0..members.len() as u32 {
                    let off = has_dec(self.mdecs.get(&(t, m)), Dec::Offset).and_then(|a| a.first().copied())?;
                    let mt = members[m as usize].0;
                    let s = match self.t(mt) {
                        Some(Ty::Matrix { cols, .. }) => {
                            cols * has_dec(self.mdecs.get(&(t, m)), Dec::MatrixStride).and_then(|a| a.first().copied())?
                        }
                        Some(Ty::RuntimeArray { .. }) => 0,
                        _ => self.decl_size(mt, rule)?,
                    };
                    end = end.max(off + s);
                }
                Some(end.div_ceil(layout::align(self, t, rule)?) * layout::align(self, t, rule)?)
            }
            _ => layout::size(self, t, rule),
        }
    }

    /// check a block struct (and the structs and arrays inside it) -> its size
    fn block_layout(&mut self, k: usize, s: u32, rule: Lay, top: bool) -> u32 {
        let Some(Ty::Struct { members }) = self.t(s).cloned() else { return 0 };
        let rn = if rule == Lay::Std140 { "std140" } else { "std430" };
        let mut end = 0u32;
        let mut size = 0u32;
        for (m, mt) in members.iter().enumerate() {
            let m = m as u32;
            let md = self.mdecs.get(&(s, m)).cloned();
            let Some(off) = has_dec(md.as_ref(), Dec::Offset).and_then(|a| a.first().copied()) else {
                self.err(Rule::BlockLayout, k, format!("member {} of %{} has no Offset", m, s));
                continue;
            };
            let Some(al) = layout::align(self, mt.0, rule) else {
                self.err(Rule::BlockLayout, k, format!("member {} of %{} has a type without a {} layout", m, s, rn));
                continue;
            };
            if off % al != 0 {
                self.err(Rule::BlockLayout, k, format!("member {} of %{}: Offset {} is not a multiple of its {} alignment {}", m, s, off, rn, al));
            }
            if off < end {
                self.err(Rule::BlockLayout, k, format!("member {} of %{}: Offset {} overlaps the member before (ends at {})", m, s, off, end));
            }
            // matrices (also inside arrays) need ColMajor / RowMajor and MatrixStride
            let mut base = mt.0;
            while let Some(Ty::Array { elem, .. } | Ty::RuntimeArray { elem }) = self.t(base) {
                base = elem.0;
            }
            if let Some(Ty::Matrix { col, .. }) = self.t(base).cloned() {
                let major = has_dec(md.as_ref(), Dec::ColMajor).is_some() || has_dec(md.as_ref(), Dec::RowMajor).is_some();
                if !major {
                    self.err(Rule::BlockLayout, k, format!("matrix member {} of %{} has no ColMajor / RowMajor", m, s));
                }
                match has_dec(md.as_ref(), Dec::MatrixStride).and_then(|a| a.first().copied()) {
                    None => self.err(Rule::BlockLayout, k, format!("matrix member {} of %{} has no MatrixStride", m, s)),
                    Some(ms) => {
                        let ca = layout::matrix_stride(self, base, rule).unwrap_or(16);
                        let csize = layout::size(self, col.0, rule).unwrap_or(0);
                        if ms % ca != 0 || ms < csize {
                            self.err(Rule::BlockLayout, k, format!("matrix member {} of %{}: MatrixStride {} breaks {} (alignment {})", m, s, ms, rn, ca));
                        }
                    }
                }
            }
            self.array_layout(k, mt.0, rule);
            if matches!(self.t(mt.0), Some(Ty::RuntimeArray { .. })) && !top {
                self.err(Rule::BlockLayout, k, format!("a runtime array inside the nested struct %{}", s));
            }
            if let Some(Ty::Struct { .. }) = self.t(base) {
                self.block_layout(k, base, rule, false);
            }
            let ms = match self.t(mt.0) {
                Some(Ty::Matrix { cols, .. }) => {
                    cols * has_dec(md.as_ref(), Dec::MatrixStride).and_then(|a| a.first().copied()).unwrap_or(16)
                }
                Some(Ty::RuntimeArray { .. }) => 0,
                _ => self.decl_size(mt.0, rule).unwrap_or(0),
            };
            end = off + ms;
            size = size.max(end);
        }
        size
    }

    fn array_layout(&mut self, k: usize, t: u32, rule: Lay) {
        let mut t = t;
        while let Some(Ty::Array { elem, .. } | Ty::RuntimeArray { elem }) = self.t(t).cloned() {
            let al = layout::align(self, t, rule).unwrap_or(1);
            let esize = self.decl_size(elem.0, rule);
            match has_dec(self.decs.get(&t), Dec::ArrayStride).and_then(|a| a.first().copied()) {
                None => self.err(Rule::BlockLayout, k, format!("the array type %{} in a block has no ArrayStride", t)),
                Some(st) => {
                    if st % al != 0 || esize.is_some_and(|e| st < e) {
                        let rn = if rule == Lay::Std140 { "std140" } else { "std430" };
                        self.err(Rule::BlockLayout, k, format!("ArrayStride {} of %{} breaks {} (alignment {}, element {} bytes)", st, t, rn, al, esize.unwrap_or(0)));
                    }
                }
            }
            t = elem.0;
        }
    }

    // ---------------------------------------------------------------- instructions

    fn check_access(&mut self, k: usize, i: &Inst) {
        let Some(&base) = i.ops.first() else { return };
        let bt = self.val(k, base);
        let Some((sc, mut t)) = self.sc_of_ptr(bt) else {
            self.want(k, false, "the base must be a pointer");
            return;
        };
        for &x in &i.ops[1..] {
            let xt = self.val(k, x);
            self.want(k, self.sv(xt).is_some_and(|s| s.k == 'i' && s.n == 1), "indices must be integer scalars");
            t = match self.t(t).cloned() {
                Some(Ty::Struct { members }) => match (self.kind(x), self.consts.get(&x)) {
                    (Some(K::Const), Some(&c)) if (c as usize) < members.len() => members[c as usize].0,
                    _ => {
                        self.want(k, false, "a struct index must be a constant within the members");
                        return;
                    }
                },
                Some(Ty::Array { elem, .. } | Ty::RuntimeArray { elem } | Ty::Vector { elem, .. }) => elem.0,
                Some(Ty::Matrix { col, .. }) => col.0,
                _ => {
                    self.want(k, false, "too many indices");
                    return;
                }
            };
        }
        let ok = self.sc_of_ptr(i.ty) == Some((sc, t));
        self.want(k, ok, "the result must point to the selected element in the base's storage class");
    }

    fn walk_literal(&mut self, k: usize, mut t: u32, idx: &[u32]) -> u32 {
        for &x in idx {
            t = match self.t(t).cloned() {
                Some(Ty::Struct { members }) if (x as usize) < members.len() => members[x as usize].0,
                Some(Ty::Vector { elem, n }) if x < n => elem.0,
                Some(Ty::Matrix { col, cols }) if x < cols => col.0,
                Some(Ty::Array { elem, len }) if self.consts.get(&len.0).is_some_and(|&l| x < l) => elem.0,
                _ => {
                    self.want(k, false, "index out of range");
                    return 0;
                }
            };
        }
        t
    }

    fn check_construct(&mut self, k: usize, ty: u32, parts: &[u32], constant: bool) {
        let pts: Vec<u32> = parts.iter().map(|&p| self.val(k, p)).collect();
        if constant {
            for &p in parts {
                let c = self.kind(p) == Some(K::Const);
                self.want(k, c, "constituents of a constant must be constants");
            }
        }
        match self.t(ty).cloned() {
            Some(Ty::Vector { elem, n }) => {
                let mut total = 0;
                for &pt in &pts {
                    let s = self.scalar(pt);
                    self.want(k, s == elem.0 && (!constant || pt == elem.0), "constituents must be the vector's components");
                    total += self.sv(pt).map_or(0, |s| s.n);
                }
                self.want(k, total == n, "the constituents must give exactly the vector's components");
            }
            Some(Ty::Matrix { col, cols }) => {
                self.want(k, pts.len() == cols as usize && pts.iter().all(|&p| p == col.0), "a matrix takes one column per constituent");
            }
            Some(Ty::Array { elem, len }) => {
                let l = self.consts.get(&len.0).copied().unwrap_or(0);
                self.want(k, pts.len() == l as usize && pts.iter().all(|&p| p == elem.0), "an array takes one element per constituent");
            }
            Some(Ty::Struct { members }) => {
                let ok = pts.len() == members.len() && pts.iter().zip(&members).all(|(a, b)| *a == b.0);
                self.want(k, ok, "a struct takes one constituent per member, of its type");
            }
            _ => self.want(k, false, "the result type must be a composite"),
        }
    }

    fn check_glsl(&mut self, k: usize, ty: u32, inst: u32, args: &[u32]) {
        use spv::glsl as g;
        let at: Vec<u32> = args.iter().map(|&a| self.val(k, a)).collect();
        let arity = match inst {
            25 | 26 | 35 | 37..=42 | 48 | 51 | 53 | 67 | 68 | 71 | 77..=80 => 2,
            43..=47 | 49 | 50 | 70 | 72 | 81 => 3,
            1..=81 => 1,
            _ => {
                self.want(k, false, "unknown GLSL.std.450 instruction");
                return;
            }
        };
        self.want(k, at.len() == arity, "wrong number of operands for the GLSL.std.450 instruction");
        match inst {
            g::Length | g::Distance => {
                let ok = self.is_sv(ty, 'f') && self.sv(ty).is_some_and(|s| s.n == 1)
                    && at.iter().all(|&a| self.scalar(a) == ty);
                self.want(k, ok, "Length / Distance: float vectors in, their component type out");
            }
            g::SAbs | g::SSign | g::UMin | g::SMin | g::UMax | g::SMax | g::UClamp | g::SClamp => {
                let ok = self.is_sv(ty, 'i') && at.iter().all(|&a| a == ty);
                self.want(k, ok, "integer operands of the result type");
            }
            g::Cross => {
                let ok = self.sv(ty).is_some_and(|s| s.k == 'f' && s.n == 3) && at.iter().all(|&a| a == ty);
                self.want(k, ok, "Cross: float vec3 operands and result");
            }
            1..=4 | 6 | 8..=32 | 37 | 40 | 43 | 46 | 48..=50 | 69..=71 => {
                let ok = self.is_sv(ty, 'f') && at.iter().all(|&a| a == ty);
                self.want(k, ok, "float operands of the result type");
            }
            _ => {}
        }
    }

    fn check_atomic(&mut self, k: usize, i: &Inst) {
        let o = i.ops;
        let Some(&ptr) = o.first() else { return };
        let pt = self.val(k, ptr);
        let Some((sc, pointee)) = self.sc_of_ptr(pt) else {
            self.want(k, false, "the pointer operand must be a pointer");
            return;
        };
        let int = self.sv(pointee).is_some_and(|s| s.k == 'i' && s.n == 1 && (s.width == 32 || s.width == 64));
        self.want(k, int, "atomics work on 32- or 64-bit integer scalars");
        if self.sv(pointee).is_some_and(|s| s.width == 64) {
            self.need_cap(Cap::Int64Atomics, k);
        }
        if !matches!(sc, 2 | 4 | 11 | 12 | 5349) {
            self.err(Rule::Storage, k, format!("Op{}: atomics need StorageBuffer, Workgroup or Image memory", self.name(k)));
        }
        let nscopes = if i.op == op::AtomicCompareExchange { 3 } else { 2 };
        for &s in o.iter().skip(1).take(nscopes) {
            let c = self.kind(s) == Some(K::Const) && self.consts.contains_key(&s);
            self.want(k, c, "scope and semantics must be 32-bit integer constants");
        }
        if i.ty != 0 {
            self.want(k, i.ty == pointee, "the result type must be the pointee type");
        }
        for &v in o.iter().skip(1 + nscopes) {
            let vt = self.val(k, v);
            self.want(k, vt == pointee, "values must have the pointee type");
        }
    }

    fn image_ty(&self, t: u32) -> Option<(u32, u32, u32, u32)> {
        match self.t(t)? {
            Ty::Image { sampled, dim, usage, format, .. } => Some((sampled.0, *dim, *usage, *format)),
            _ => None,
        }
    }

    /// the coordinate components an image of type t needs (dimensions + array layer)
    fn coord_size(&self, t: u32) -> u32 {
        match self.t(t) {
            Some(Ty::Image { dim, arrayed, .. }) => {
                (match dim {
                    0 | 5 => 1,
                    2 | 3 => 3,
                    _ => 2,
                }) + arrayed
            }
            _ => 0,
        }
    }

    /// the components OpImageQuerySize(Lod) gives for an image of type t
    fn size_comps(&self, t: u32) -> u32 {
        match self.t(t) {
            Some(Ty::Image { dim, arrayed, .. }) => {
                (match dim {
                    0 | 5 => 1,
                    2 => 3,
                    _ => 2,
                }) + arrayed
            }
            _ => 0,
        }
    }

    fn check_image(&mut self, k: usize, i: &Inst) {
        let o = i.ops;
        let Some(&img) = o.first() else { return };
        let it = self.val(k, img);
        let coord = o.get(1).map_or(0, |&c| self.val(k, c));
        let sampled_op = matches!(i.op, op::ImageSampleImplicitLod | op::ImageSampleExplicitLod
            | op::ImageSampleDrefImplicitLod | op::ImageSampleDrefExplicitLod | op::ImageGather | op::ImageDrefGather);
        let image = if sampled_op {
            match self.t(it) {
                Some(Ty::SampledImage { image }) => image.0,
                _ => {
                    self.want(k, false, "the first operand must be a sampled image");
                    return;
                }
            }
        } else {
            it
        };
        let Some((st, dim, usage, format)) = self.image_ty(image) else {
            self.want(k, false, "the first operand must be an image");
            return;
        };
        let texel = |v: &V, t: u32| v.scalar(t) == st && v.sv(t).is_some_and(|s| s.n == 4 || s.n == 1);
        let need = self.coord_size(image);
        let have = self.sv(coord).map_or(0, |s| s.n);
        if i.op != op::ImageWrite && i.op != op::ImageRead && dim != 6 {
            self.want(k, have >= need, "the coordinate has fewer components than the image needs");
        }
        match i.op {
            op::ImageSampleImplicitLod | op::ImageSampleExplicitLod | op::ImageGather => {
                self.want(k, self.is_sv(coord, 'f'), "the coordinate must be float");
                self.want(k, self.sv(i.ty).is_some_and(|s| s.n == 4) && self.scalar(i.ty) == st, "the result must be a vec4 of the sampled type");
                if i.op == op::ImageSampleExplicitLod {
                    let m = o.get(2).copied().unwrap_or(0);
                    self.want(k, m & (spv::img::Lod | spv::img::Grad) != 0, "explicit LOD needs the Lod or Grad operand");
                }
            }
            op::ImageSampleDrefImplicitLod | op::ImageSampleDrefExplicitLod | op::ImageDrefGather => {
                self.want(k, self.is_sv(coord, 'f'), "the coordinate must be float");
                let dref = o.get(2).map_or(0, |&c| self.val(k, c));
                self.want(k, self.sv(dref).is_some_and(|s| s.k == 'f' && s.n == 1 && s.width == 32), "Dref must be a 32-bit float scalar");
                if i.op == op::ImageDrefGather {
                    self.want(k, self.sv(i.ty).is_some_and(|s| s.n == 4) && self.scalar(i.ty) == st, "the result must be a vec4 of the sampled type");
                } else {
                    self.want(k, self.sv(i.ty).is_some_and(|s| s.k == 'f' && s.n == 1) && i.ty == st, "the result must be the sampled float scalar");
                }
                if i.op == op::ImageSampleDrefExplicitLod {
                    let m = o.get(3).copied().unwrap_or(0);
                    self.want(k, m & (spv::img::Lod | spv::img::Grad) != 0, "explicit LOD needs the Lod or Grad operand");
                }
            }
            op::ImageFetch => {
                self.want(k, dim != 3, "OpImageFetch does not take a Cube image");
                self.want(k, usage == 1, "OpImageFetch needs a sampled image type (Sampled 1)");
                self.want(k, self.is_sv(coord, 'i'), "the coordinate must be integer");
                self.want(k, texel(self, i.ty) && self.sv(i.ty).is_some_and(|s| s.n == 4), "the result must be a vec4 of the sampled type");
            }
            op::ImageRead => {
                self.want(k, usage == 2 || dim == 6, "OpImageRead needs a storage image (Sampled 2)");
                self.want(k, self.is_sv(coord, 'i'), "the coordinate must be integer");
                self.want(k, texel(self, i.ty), "the result must be a scalar or vec4 of the sampled type");
                if format == 0 && dim != 6 {
                    self.need_cap(Cap::StorageImageReadWithoutFormat, k);
                }
            }
            op::ImageWrite => {
                self.want(k, usage == 2, "OpImageWrite needs a storage image (Sampled 2)");
                self.want(k, self.is_sv(coord, 'i'), "the coordinate must be integer");
                let tx = o.get(2).map_or(0, |&c| self.val(k, c));
                self.want(k, self.scalar(tx) == st, "the texel must have the sampled component type");
                if format == 0 {
                    self.need_cap(Cap::StorageImageWriteWithoutFormat, k);
                }
            }
            _ => {}
        }
    }

    fn check_inst(&mut self, k: usize, f: usize) {
        let i = self.ins[k];
        self.check_op(k, f, i);
    }

    /// the type rules of instruction i (at k, in function f; NOFN for the operation of an
    /// OpSpecConstantOp, which is never one of the opcodes that need the function)
    fn check_op(&mut self, k: usize, f: usize, i: Inst<'a>) {
        let o = i.ops;
        let ty = i.ty;
        let g = |n: usize| o.get(n).copied().unwrap_or(0);
        match i.op {
            op::FAdd | op::FSub | op::FMul | op::FDiv | op::FRem | op::FMod => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                self.want(k, self.is_sv(ty, 'f'), "the result must be a float scalar or vector");
                self.want(k, a == ty && b == ty, "operands must have the result type");
            }
            op::FNegate => {
                let a = self.val(k, g(0));
                self.want(k, self.is_sv(ty, 'f') && a == ty, "a float operand of the result type");
            }
            op::IAdd | op::ISub | op::IMul | op::UDiv | op::SDiv | op::UMod | op::SRem | op::SMod | op::BitwiseAnd
            | op::BitwiseOr | op::BitwiseXor => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                let r = self.sv(ty);
                self.want(k, r.is_some_and(|s| s.k == 'i'), "the result must be an integer scalar or vector");
                let same = |v: &V, x: u32| v.sv(x).is_some_and(|s| Some(s) == r || r.is_some_and(|r| s.k == 'i' && s.n == r.n && s.width == r.width));
                self.want(k, same(self, a) && same(self, b), "operands must be integers of the result's width and length");
            }
            op::SNegate | op::Not => {
                let a = self.val(k, g(0));
                let ok = self.sv(ty).is_some_and(|r| r.k == 'i' && self.sv(a).is_some_and(|s| s.k == 'i' && s.n == r.n && s.width == r.width));
                self.want(k, ok, "an integer operand of the result's width and length");
            }
            op::ShiftLeftLogical | op::ShiftRightLogical | op::ShiftRightArithmetic => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                let r = self.sv(ty);
                let ok = r.is_some_and(|r| r.k == 'i' && self.sv(a).is_some_and(|s| s.k == 'i' && s.n == r.n && s.width == r.width)
                    && self.sv(b).is_some_and(|s| s.k == 'i' && s.n == r.n));
                self.want(k, ok, "integer base of the result's shape and an integer shift of its length");
            }
            op::VectorTimesScalar => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                let ok = self.sv(ty).is_some_and(|s| s.k == 'f' && s.n > 1) && a == ty && b == self.scalar(ty);
                self.want(k, ok, "a float vector of the result type and its component type");
            }
            op::MatrixTimesScalar => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                let ok = matches!(self.t(ty), Some(Ty::Matrix { col, .. }) if b == self.scalar(col.0)) && a == ty;
                self.want(k, ok, "a matrix of the result type and its component type");
            }
            op::MatrixTimesVector | op::VectorTimesMatrix | op::MatrixTimesMatrix | op::Dot | op::OuterProduct => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                let mat = |v: &V, t: u32| match v.t(t) {
                    Some(Ty::Matrix { col, cols }) => v.sv(col.0).map(|s| (col.0, s.n, *cols)),
                    _ => None,
                };
                let ok = match i.op {
                    op::MatrixTimesVector => mat(self, a).is_some_and(|(c, _, cols)| ty == c && self.sv(b).is_some_and(|s| s.n == cols) && self.scalar(b) == self.scalar(c)),
                    op::VectorTimesMatrix => mat(self, b).is_some_and(|(c, rows, cols)| self.sv(a).is_some_and(|s| s.n == rows) && self.scalar(a) == self.scalar(c)
                        && self.sv(ty).is_some_and(|s| s.n == cols) && self.scalar(ty) == self.scalar(c)),
                    op::MatrixTimesMatrix => match (mat(self, a), mat(self, b), mat(self, ty)) {
                        (Some((ca, _, cola)), Some((cb, rowb, colb)), Some((cr, _, colr))) => {
                            cr == ca && colr == colb && rowb == cola && self.scalar(cb) == self.scalar(ca)
                        }
                        _ => false,
                    },
                    op::Dot => self.is_sv(a, 'f') && a == b && self.scalar(a) == ty && ty != a,
                    _ => mat(self, ty).is_some_and(|(c, _, cols)| a == c && self.sv(b).is_some_and(|s| s.n == cols)),
                };
                self.want(k, ok, "operand and result shapes do not match");
            }
            op::Transpose => {
                let a = self.val(k, g(0));
                let ok = match (self.t(a), self.t(ty)) {
                    (Some(Ty::Matrix { col: ca, cols: na }), Some(Ty::Matrix { col: cr, cols: nr })) => {
                        self.sv(ca.0).map(|s| s.n) == Some(*nr) && self.sv(cr.0).map(|s| s.n) == Some(*na)
                    }
                    _ => false,
                };
                self.want(k, ok, "the result must be the transposed matrix type");
            }
            o2 if (op::IEqual..=op::SLessThanEqual).contains(&o2) || (op::FOrdEqual..=op::FUnordGreaterThanEqual).contains(&o2) => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                let want_k = if o2 >= op::FOrdEqual { 'f' } else { 'i' };
                let (sa, sb) = (self.sv(a), self.sv(b));
                let ok = sa.is_some_and(|x| x.k == want_k) && sb.is_some_and(|y| Some(y.n) == sa.map(|x| x.n) && y.k == want_k
                    && Some(y.width) == sa.map(|x| x.width)) && (want_k == 'i' || a == b);
                self.want(k, ok, "operands of the same numeric type");
                let r = self.sv(ty);
                self.want(k, r.is_some_and(|r| r.k == 'b' && Some(r.n) == sa.map(|x| x.n)), "the result must be bool of the operands' length");
            }
            op::IsNan | op::IsInf | op::IsFinite | op::IsNormal | op::SignBitSet => {
                let a = self.val(k, g(0));
                let ok = self.is_sv(a, 'f') && self.sv(ty).is_some_and(|r| r.k == 'b' && Some(r.n) == self.sv(a).map(|s| s.n));
                self.want(k, ok, "a float operand and a bool result of its length");
            }
            op::LogicalAnd | op::LogicalOr | op::LogicalEqual | op::LogicalNotEqual => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                self.want(k, self.is_sv(ty, 'b') && a == ty && b == ty, "bool operands of the result type");
            }
            op::LogicalNot => {
                let a = self.val(k, g(0));
                self.want(k, self.is_sv(ty, 'b') && a == ty, "a bool operand of the result type");
            }
            op::Any | op::All => {
                let a = self.val(k, g(0));
                let ok = self.sv(a).is_some_and(|s| s.k == 'b' && s.n > 1) && self.sv(ty).is_some_and(|s| s.k == 'b' && s.n == 1);
                self.want(k, ok, "a bool vector in, a bool out");
            }
            op::Select => {
                let (c, a, b) = (self.val(k, g(0)), self.val(k, g(1)), self.val(k, g(2)));
                self.want(k, a == ty && b == ty, "both objects must have the result type");
                let cn = self.sv(c);
                let rn = self.sv(ty);
                let ok = cn.is_some_and(|c| c.k == 'b' && (c.n == 1 || Some(c.n) == rn.map(|r| r.n)));
                self.want(k, ok, "the condition must be bool (or a bool vector of the result's length)");
                if rn.is_none() && self.minor < 4 {
                    self.err(Rule::Version, k, String::from("OpSelect on composites needs SPIR-V 1.4"));
                }
            }
            op::ConvertFToU | op::ConvertFToS | op::ConvertSToF | op::ConvertUToF | op::UConvert | op::SConvert
            | op::FConvert | op::Bitcast => {
                let a = self.val(k, g(0));
                let (sa, sr) = (self.sv(a), self.sv(ty));
                let ok = match (sa, sr) {
                    (Some(x), Some(y)) => match i.op {
                        op::ConvertFToU | op::ConvertFToS => x.k == 'f' && y.k == 'i' && x.n == y.n,
                        op::ConvertSToF | op::ConvertUToF => x.k == 'i' && y.k == 'f' && x.n == y.n,
                        op::UConvert | op::SConvert => x.k == 'i' && y.k == 'i' && x.n == y.n && x.width != y.width,
                        op::FConvert => x.k == 'f' && y.k == 'f' && x.n == y.n && x.width != y.width,
                        _ => x.k != 'b' && y.k != 'b' && x.n * x.width == y.n * y.width,
                    },
                    _ => false,
                };
                self.want(k, ok, "the operand and result types do not fit the conversion");
            }
            op::Load => {
                let pt = self.val(k, g(0));
                let ok = self.sc_of_ptr(pt).is_some_and(|(_, p)| p == ty);
                self.want(k, ok, "the pointer must point to the result type");
            }
            op::Store => {
                let (pt, vt) = (self.val(k, g(0)), self.val(k, g(1)));
                match self.sc_of_ptr(pt) {
                    Some((sc, p)) => {
                        self.want(k, p == vt, "the object must have the pointee type");
                        if matches!(sc, 0 | 1 | 2 | 9) {
                            let sn = spv::name(spv::SCS, sc).unwrap_or("?");
                            self.err(Rule::Storage, k, format!("OpStore to read-only {} memory", sn));
                        }
                    }
                    None => self.want(k, false, "the pointer operand must be a pointer"),
                }
            }
            op::AccessChain | op::InBoundsAccessChain => self.check_access(k, &i),
            op::CompositeConstruct => self.check_construct(k, ty, o, false),
            op::CompositeExtract => {
                let ct = self.val(k, g(0));
                let t = self.walk_literal(k, ct, &o[1.min(o.len())..]);
                self.want(k, t == 0 || t == ty, "the result must have the selected member's type");
            }
            op::CompositeInsert => {
                let (ot, ct) = (self.val(k, g(0)), self.val(k, g(1)));
                let t = self.walk_literal(k, ct, &o[2.min(o.len())..]);
                self.want(k, (t == 0 || t == ot) && ct == ty, "the object must have the selected member's type");
            }
            op::VectorShuffle => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                let (sa, sb, sr) = (self.sv(a), self.sv(b), self.sv(ty));
                let ok = match (sa, sb, sr) {
                    (Some(x), Some(y), Some(r)) => {
                        x.n > 1 && y.n > 1 && self.scalar(a) == self.scalar(ty) && self.scalar(b) == self.scalar(ty)
                            && r.n as usize == o.len() - 2 && o[2..].iter().all(|&c| c < x.n + y.n || c == u32::MAX)
                    }
                    _ => false,
                };
                self.want(k, ok, "vectors of the result's component type, one valid component per result component");
            }
            op::VectorExtractDynamic => {
                let (a, x) = (self.val(k, g(0)), self.val(k, g(1)));
                let ok = self.sv(a).is_some_and(|s| s.n > 1) && self.scalar(a) == ty && self.is_sv(x, 'i');
                self.want(k, ok, "a vector, an integer index, its component type as result");
            }
            op::CopyObject => {
                let a = self.val(k, g(0));
                self.want(k, a == ty, "the operand must have the result type");
            }
            op::ExtInst => {
                let set = g(0);
                if self.kind(set) != Some(K::ExtSet) {
                    self.err(Rule::IdKind, k, format!("OpExtInst: %{} is not an OpExtInstImport", set));
                } else if self.sets.get(&set).map(String::as_str) == Some("GLSL.std.450") {
                    self.check_glsl(k, ty, g(1), &o[2.min(o.len())..]);
                }
            }
            op::AtomicLoad | op::AtomicStore | op::AtomicExchange | op::AtomicCompareExchange | op::AtomicIIncrement
            | op::AtomicIDecrement | op::AtomicIAdd | op::AtomicISub | op::AtomicSMin | op::AtomicUMin | op::AtomicSMax
            | op::AtomicUMax | op::AtomicAnd | op::AtomicOr | op::AtomicXor => self.check_atomic(k, &i),
            op::ControlBarrier | op::MemoryBarrier => {
                for &s in o {
                    let c = self.kind(s) == Some(K::Const) && self.consts.contains_key(&s);
                    self.want(k, c, "scopes and semantics must be 32-bit integer constants");
                }
            }
            op::SampledImage => {
                let (a, b) = (self.val(k, g(0)), self.val(k, g(1)));
                let ok = self.image_ty(a).is_some_and(|(_, dim, usage, _)| usage != 2 && dim != 5)
                    && matches!(self.t(b), Some(Ty::Sampler))
                    && matches!(self.t(ty), Some(Ty::SampledImage { image }) if image.0 == a);
                self.want(k, ok, "a sampled image type of the image and a sampler");
            }
            op::ImageSampleImplicitLod | op::ImageSampleExplicitLod | op::ImageSampleDrefImplicitLod
            | op::ImageSampleDrefExplicitLod | op::ImageFetch | op::ImageGather | op::ImageDrefGather | op::ImageRead
            | op::ImageWrite => {
                self.check_image(k, &i);
                if matches!(i.op, op::ImageSampleImplicitLod | op::ImageSampleDrefImplicitLod) {
                    let n = self.name(k);
                    self.funcs[f].frag_only.push((k, n));
                }
            }
            op::ImageQuerySize | op::ImageQuerySizeLod | op::ImageQueryLevels | op::ImageQuerySamples => {
                self.need_cap(Cap::ImageQuery, k);
                let a = self.val(k, g(0));
                self.want(k, self.image_ty(a).is_some() && self.is_sv(ty, 'i'), "an image in, integers out");
                if let Some(Ty::Image { dim, ms, usage, .. }) = self.t(a).cloned() {
                    let n = self.sv(ty).map_or(0, |s| s.n);
                    match i.op {
                        op::ImageQuerySizeLod => {
                            self.want(k, matches!(dim, 0..=3) && ms == 0, "OpImageQuerySizeLod needs a 1D, 2D, 3D or Cube image without MS");
                            self.want(k, usage == 1, "OpImageQuerySizeLod needs a sampled image (Vulkan: Sampled 1)");
                            let l = self.val(k, g(1));
                            self.want(k, self.sv(l).is_some_and(|s| s.k == 'i' && s.n == 1), "the level of detail must be an integer scalar");
                            self.want(k, n == self.size_comps(a), "the result has one component per dimension (and the layers)");
                        }
                        op::ImageQuerySize => {
                            self.want(k, matches!(dim, 4 | 5) || ms == 1 || usage == 0 || usage == 2,
                                      "OpImageQuerySize needs a storage, multisampled, Rect or Buffer image (OpImageQuerySizeLod for sampled ones)");
                            self.want(k, n == self.size_comps(a), "the result has one component per dimension (and the layers)");
                        }
                        op::ImageQueryLevels => {
                            self.want(k, matches!(dim, 0..=3) && ms == 0 && usage == 1, "OpImageQueryLevels needs a sampled 1D, 2D, 3D or Cube image without MS");
                            self.want(k, n == 1, "the result must be an integer scalar");
                        }
                        _ => {
                            self.want(k, ms == 1, "OpImageQuerySamples needs a multisampled image");
                            self.want(k, n == 1, "the result must be an integer scalar");
                        }
                    }
                }
            }
            op::QuantizeToF16 => {
                let a = self.val(k, g(0));
                self.want(k, self.sv(ty).is_some_and(|s| s.k == 'f' && s.width == 32) && a == ty, "a 32-bit float operand of the result type");
            }
            o2 if (op::DPdx..=op::FwidthCoarse).contains(&o2) => {
                let a = self.val(k, g(0));
                self.want(k, self.sv(ty).is_some_and(|s| s.k == 'f' && s.width == 32) && a == ty, "a 32-bit float operand of the result type");
                if o2 >= op::DPdxFine {
                    self.need_cap(Cap::DerivativeControl, k);
                }
                let n = self.name(k);
                self.funcs[f].frag_only.push((k, n));
            }
            op::Kill | op::DemoteToHelperInvocation | op::TerminateInvocation => {
                let n = self.name(k);
                self.funcs[f].frag_only.push((k, n));
                if i.op == op::DemoteToHelperInvocation {
                    self.need_cap(Cap::DemoteToHelperInvocation, k);
                    if self.minor < 6 && !self.exts.contains("SPV_EXT_demote_to_helper_invocation") {
                        self.err(Rule::Version, k, String::from("OpDemoteToHelperInvocation needs SPIR-V 1.6 or SPV_EXT_demote_to_helper_invocation"));
                    }
                }
                if i.op == op::TerminateInvocation && self.minor < 6 && !self.exts.contains("SPV_KHR_terminate_invocation") {
                    self.err(Rule::Version, k, String::from("OpTerminateInvocation needs SPIR-V 1.6 or SPV_KHR_terminate_invocation"));
                }
            }
            op::CopyLogical if self.minor < 4 => self.err(Rule::Version, k, String::from("OpCopyLogical needs SPIR-V 1.4")),
            op::FunctionCall => {
                let callee = g(0);
                if self.kind(callee) != Some(K::Func) {
                    self.err(Rule::IdKind, k, format!("OpFunctionCall: %{} is not a function", callee));
                    return;
                }
                let fi = self.defs[&callee].inst;
                let fty = self.ins[fi].ops.get(1).copied().unwrap_or(0);
                if let Some(Ty::Function { ret, params }) = self.t(fty).cloned() {
                    self.want(k, ret.0 == ty, "the result type must be the callee's return type");
                    let args: Vec<u32> = o[1..].iter().map(|&a| self.val(k, a)).collect();
                    let ok = args.len() == params.len() && args.iter().zip(&params).all(|(a, p)| *a == p.0);
                    self.want(k, ok, "the arguments must match the callee's parameters");
                }
            }
            op::Phi => {
                for p in o.chunks(2) {
                    let vt = self.val(k, p[0]);
                    self.want(k, vt == ty, "phi values must have the result type");
                    if p.len() < 2 || self.kind(p[1]) != Some(K::Label) {
                        self.err(Rule::IdKind, k, String::from("OpPhi: parents must be labels"));
                    }
                }
            }
            op::Branch | op::BranchConditional | op::Switch | op::SelectionMerge | op::LoopMerge => {
                let labels: Vec<u32> = match i.op {
                    op::Branch => alloc::vec![g(0)],
                    op::BranchConditional => {
                        let c = self.val(k, g(0));
                        self.want(k, self.sv(c).is_some_and(|s| s.k == 'b' && s.n == 1), "the condition must be a bool scalar");
                        alloc::vec![g(1), g(2)]
                    }
                    op::Switch => {
                        let s = self.val(k, g(0));
                        self.want(k, self.sv(s).is_some_and(|s| s.k == 'i' && s.n == 1), "the selector must be an integer scalar");
                        let mut v = alloc::vec![g(1)];
                        v.extend(o.iter().skip(3).step_by(2).copied());
                        v
                    }
                    op::SelectionMerge => alloc::vec![g(0)],
                    _ => alloc::vec![g(0), g(1)],
                };
                for l in labels {
                    if self.kind(l).is_some_and(|x| x != K::Label) {
                        let n = self.name(k);
                        self.err(Rule::IdKind, k, format!("Op{}: %{} is not a label", n, l));
                    }
                }
            }
            op::Return => {
                let ret = self.funcs[f].ret;
                self.want(k, matches!(self.t(ret), Some(Ty::Void)), "OpReturn in a function that returns a value");
            }
            op::ReturnValue => {
                let a = self.val(k, g(0));
                let ret = self.funcs[f].ret;
                self.want(k, a == ret && !matches!(self.t(ret), Some(Ty::Void)), "the value must have the function's return type");
            }
            _ => {}
        }
    }

    fn check_module_inst(&mut self, k: usize) {
        let i = self.ins[k];
        match i.op {
            op::Constant => {
                let w = match self.t(i.ty) {
                    Some(Ty::Int { width, .. } | Ty::Float { width }) => Some(width.div_ceil(32) as usize),
                    _ => None,
                };
                self.want(k, w.is_some(), "the type must be an integer or float scalar");
                self.want(k, w.is_none_or(|w| w == i.ops.len()), "the value has the wrong number of words");
            }
            op::SpecConstant => {
                let w = match self.t(i.ty) {
                    Some(Ty::Int { width, .. } | Ty::Float { width }) => Some(width.div_ceil(32) as usize),
                    _ => None,
                };
                self.want(k, w.is_some(), "the type must be an integer or float scalar");
                self.want(k, w.is_none_or(|w| w == i.ops.len()), "the default has the wrong number of words");
            }
            op::ConstantTrue | op::ConstantFalse | op::SpecConstantTrue | op::SpecConstantFalse => {
                self.want(k, matches!(self.t(i.ty), Some(Ty::Bool)), "the type must be bool")
            }
            op::ConstantComposite | op::SpecConstantComposite => self.check_construct(k, i.ty, i.ops, true),
            op::SpecConstantOp => self.check_spec_op(k),
            op::ConstantNull => {
                let ok = !matches!(self.t(i.ty), None | Some(Ty::Void | Ty::Function { .. } | Ty::RuntimeArray { .. }));
                self.want(k, ok, "no null value of this type")
            }
            _ => {}
        }
    }

    /// OpSpecConstantOp: an operation SPIR-V allows under Shader, on constants, with its
    /// operation's type rules
    fn check_spec_op(&mut self, k: usize) {
        let i = self.ins[k];
        let Some(&opc) = i.ops.first() else { return };
        let opc = opc as u16;
        let allowed = matches!(opc, op::SConvert | op::UConvert | op::FConvert | op::QuantizeToF16 | op::SNegate | op::Not
            | op::IAdd | op::ISub | op::IMul | op::UDiv | op::SDiv | op::UMod | op::SRem | op::SMod | op::ShiftRightLogical
            | op::ShiftRightArithmetic | op::ShiftLeftLogical | op::BitwiseOr | op::BitwiseXor | op::BitwiseAnd
            | op::VectorShuffle | op::CompositeExtract | op::CompositeInsert | op::LogicalOr | op::LogicalAnd | op::LogicalNot
            | op::LogicalEqual | op::LogicalNotEqual | op::Select | op::IEqual | op::INotEqual | op::ULessThan | op::SLessThan
            | op::UGreaterThan | op::SGreaterThan | op::ULessThanEqual | op::SLessThanEqual | op::UGreaterThanEqual
            | op::SGreaterThanEqual);
        if !allowed {
            let n = spv::op_info(opc).map_or("?", |x| x.name);
            self.err(Rule::Type, k, format!("OpSpecConstantOp: Op{} is no operation SPIR-V allows here under Shader", n));
            return;
        }
        if opc == op::UConvert && self.minor < 4 {
            self.err(Rule::Version, k, String::from("OpSpecConstantOp UConvert needs SPIR-V 1.4"));
        }
        let inner = Inst { op: opc, ty: i.ty, res: i.res, ops: &i.ops[1..], at: i.at, info: spv::op_info(opc) };
        for id in parse::ids(&inner) {
            if self.kind(id) != Some(K::Const) {
                self.err(Rule::Type, k, format!("OpSpecConstantOp: the operand %{} is no constant", id));
            }
        }
        self.check_op(k, NOFN, inner);
    }

    fn functions(&mut self) {
        for f in 0..self.funcs.len() {
            let (start, end) = (self.funcs[f].start, self.funcs[f].end);
            let fi = self.ins[start];
            let fty = fi.ops.get(1).copied().unwrap_or(0);
            let params = match self.t(fty).cloned() {
                Some(Ty::Function { ret, params }) => {
                    if ret.0 != fi.ty {
                        self.want(start, false, "the result type must be the function type's return type");
                    }
                    params
                }
                _ => {
                    self.want(start, false, "the function type operand must be an OpTypeFunction");
                    Vec::new()
                }
            };
            let mut pi = 0;
            for k in start + 1..end {
                if self.ins[k].op == op::FunctionParameter {
                    let ok = params.get(pi).is_some_and(|p| p.0 == self.ins[k].ty);
                    self.want(k, ok, "parameters must match the function type");
                    pi += 1;
                } else {
                    self.check_inst(k, f);
                }
            }
            if pi != params.len() {
                self.want(start, false, "the number of parameters differs from the function type");
            }
            self.cfg(f);
        }
    }

    // ---------------------------------------------------------------- control flow

    fn cfg(&mut self, f: usize) {
        struct Blk {
            label: u32,
            term: usize,
            merge: Option<(u16, u32, u32)>,
        }
        let (start, end) = (self.funcs[f].start, self.funcs[f].end);
        let mut blocks: Vec<Blk> = Vec::new();
        let mut open = false;
        let mut k = start + 1;
        while k < end && self.ins[k].op == op::FunctionParameter {
            k += 1;
        }
        let mut phis_ok = false;
        while k < end {
            let i = self.ins[k];
            if i.op == op::Label {
                if open {
                    let l = blocks.last().map_or(0, |b| b.label);
                    self.err(Rule::Cfg, k, format!("block %{} has no terminator", l));
                }
                blocks.push(Blk { label: i.res, term: k, merge: None });
                open = true;
                phis_ok = true;
            } else if !open {
                let n = self.name(k);
                self.err(Rule::Cfg, k, format!("Op{} outside a block (after a terminator or before the first OpLabel)", n));
            } else {
                if i.op == op::Phi && !phis_ok {
                    self.err(Rule::Cfg, k, String::from("OpPhi after other instructions of its block"));
                }
                if i.op != op::Phi && i.op != op::Line {
                    phis_ok = false;
                }
                if i.op == op::SelectionMerge || i.op == op::LoopMerge {
                    let next = self.ins.get(k + 1).map_or(0, |x| x.op);
                    let fits = if i.op == op::SelectionMerge {
                        matches!(next, op::BranchConditional | op::Switch)
                    } else {
                        matches!(next, op::Branch | op::BranchConditional)
                    };
                    if !fits {
                        let n = self.name(k);
                        self.err(Rule::Cfg, k, format!("Op{} must come right before its block's branch", n));
                    }
                    let b = blocks.last_mut().expect("open block");
                    b.merge = Some((i.op, i.ops.first().copied().unwrap_or(0), i.ops.get(1).copied().unwrap_or(0)));
                }
                if is_terminator(i.op) {
                    blocks.last_mut().expect("open block").term = k;
                    open = false;
                }
            }
            k += 1;
        }
        if open {
            let l = blocks.last().map_or(0, |b| b.label);
            self.err(Rule::Cfg, end, format!("block %{} has no terminator", l));
        }
        if blocks.is_empty() {
            self.err(Rule::Cfg, start, String::from("a function without blocks"));
            return;
        }
        let pos: BTreeMap<u32, usize> = blocks.iter().enumerate().map(|(n, b)| (b.label, n)).collect();
        let targets = |v: &V, b: &Blk| -> Vec<u32> {
            let i = v.ins[b.term];
            match i.op {
                op::Branch => i.ops.to_vec(),
                op::BranchConditional => i.ops[1..3.min(i.ops.len())].to_vec(),
                op::Switch => {
                    let mut t = alloc::vec![i.ops.get(1).copied().unwrap_or(0)];
                    t.extend(i.ops.iter().skip(3).step_by(2).copied());
                    t
                }
                _ => Vec::new(),
            }
        };
        let mut merges: BTreeMap<u32, u32> = BTreeMap::new();
        let mut conts: BTreeSet<u32> = BTreeSet::new();
        let mut loops: BTreeSet<u32> = BTreeSet::new();
        for b in &blocks {
            if let Some((mop, m, c)) = b.merge {
                let check = |v: &mut V, l: u32, what: &str| {
                    match pos.get(&l) {
                        None => v.err(Rule::Cfg, b.term, format!("the {} %{} of %{} is no block of the function", what, l, b.label)),
                        Some(&p) if p <= pos[&b.label] && what == "merge block" => {
                            v.err(Rule::Cfg, b.term, format!("the merge block %{} must come after its header %{}", l, b.label))
                        }
                        _ => {}
                    }
                };
                check(self, m, "merge block");
                if let Some(other) = merges.insert(m, b.label) {
                    self.err(Rule::Cfg, b.term, format!("%{} is the merge block of both %{} and %{}", m, other, b.label));
                }
                if mop == op::LoopMerge {
                    check(self, c, "continue target");
                    if c == m {
                        self.err(Rule::Cfg, b.term, format!("loop %{}: the continue target is the merge block", b.label));
                    }
                    conts.insert(c);
                    loops.insert(b.label);
                }
            }
        }
        let mut back: BTreeSet<u32> = BTreeSet::new();
        let mut preds: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for (n, b) in blocks.iter().enumerate() {
            let ts = targets(self, b);
            let mut plain = 0;
            for &t in &ts {
                let Some(&p) = pos.get(&t) else {
                    self.err(Rule::Cfg, b.term, format!("the branch target %{} is no block of the function", t));
                    continue;
                };
                preds.entry(t).or_default().insert(b.label);
                if p == 0 {
                    self.err(Rule::Cfg, b.term, String::from("the entry block must not be a branch target"));
                }
                if p <= n {
                    if loops.contains(&t) {
                        back.insert(t);
                    } else {
                        self.err(Rule::Cfg, b.term, format!("back edge to %{}, which is not a loop header", t));
                    }
                }
                if !merges.contains_key(&t) && !conts.contains(&t) {
                    plain += 1;
                }
            }
            let cond = matches!(self.ins[b.term].op, op::BranchConditional | op::Switch);
            let distinct = ts.iter().collect::<BTreeSet<_>>().len();
            if cond && b.merge.is_none() && plain > 1 && distinct > 1 {
                self.err(Rule::Cfg, b.term, format!("block %{}: a conditional branch without a merge instruction", b.label));
            }
        }
        for &l in &loops {
            if !back.contains(&l) {
                self.err(Rule::Cfg, self.defs.get(&l).map_or(start, |d| d.inst), format!("loop header %{} has no back edge", l));
            }
        }
        for k in start..end {
            let i = self.ins[k];
            if i.op != op::Phi {
                continue;
            }
            let blk = (0..=k).rev().find(|&j| self.ins[j].op == op::Label).map_or(0, |j| self.ins[j].res);
            for p in i.ops.chunks(2) {
                if p.len() == 2 && !preds.get(&blk).is_some_and(|s| s.contains(&p[1])) {
                    self.err(Rule::Cfg, k, format!("OpPhi: %{} is no predecessor of %{}", p[1], blk));
                }
            }
        }
    }

    // ---------------------------------------------------------------- entry points

    fn call_tree(&self, func: u32) -> Vec<usize> {
        let mut seen = BTreeSet::new();
        let mut todo = alloc::vec![func];
        let mut out = Vec::new();
        while let Some(fid) = todo.pop() {
            if !seen.insert(fid) {
                continue;
            }
            if let Some(n) = self.funcs.iter().position(|f| f.id == fid) {
                out.push(n);
                todo.extend(self.funcs[n].calls.iter().copied());
            }
        }
        out
    }

    fn entry_points(&mut self) {
        let mut modes: BTreeMap<u32, Vec<(u32, usize)>> = BTreeMap::new();
        let mut entries: Vec<(usize, u32, u32)> = Vec::new();
        for k in 0..self.ins.len() {
            let i = self.ins[k];
            match i.op {
                op::EntryPoint if i.ops.len() >= 2 => entries.push((k, i.ops[0], i.ops[1])),
                op::ExecutionMode | op::ExecutionModeId if i.ops.len() >= 2 => {
                    modes.entry(i.ops[0]).or_default().push((i.ops[1], k))
                }
                _ => {}
            }
        }
        let mut seen_names = BTreeSet::new();
        let entry_fns: BTreeSet<u32> = entries.iter().map(|e| e.2).collect();
        for (&f, list) in &modes {
            if !entry_fns.contains(&f) {
                let k = list[0].1;
                self.err(Rule::ExecMode, k, format!("OpExecutionMode on %{}, which is no entry point", f));
            }
        }
        for (k, model, func) in entries {
            let i = self.ins[k];
            let (name, nw) = parse::string(&i.ops[2..]);
            let iface: Vec<u32> = i.ops[2 + nw..].to_vec();
            if !seen_names.insert((model, name.clone())) {
                self.err(Rule::Interface, k, format!("two entry points {:?} of the same model", name));
            }
            match model {
                x if x == Model::Geometry as u32 => self.need_cap(Cap::Geometry, k),
                1 | 2 => self.need_cap(Cap::Tessellation, k),
                x if x == Model::TaskEXT as u32 || x == Model::MeshEXT as u32 => self.need_cap(Cap::MeshShadingEXT, k),
                _ => self.need_cap(Cap::Shader, k),
            }
            if self.kind(func) != Some(K::Func) {
                self.err(Rule::IdKind, k, format!("OpEntryPoint: %{} is not a function", func));
                continue;
            }
            let fi = self.defs[&func].inst;
            let fty = self.ins[fi].ops.get(1).copied().unwrap_or(0);
            let ok = matches!(self.t(fty), Some(Ty::Function { ret, params }) if params.is_empty()
                && matches!(self.t(ret.0), Some(Ty::Void)));
            if !ok {
                self.err(Rule::Interface, k, String::from("an entry point must be a void function without parameters"));
            }
            // the interface list
            let mut listed = BTreeSet::new();
            for &v in &iface {
                if self.kind(v) != Some(K::Global) {
                    self.err(Rule::Interface, k, format!("interface %{} is not a global variable", v));
                    continue;
                }
                let sc = self.vty.get(&v).and_then(|&t| self.sc_of_ptr(t)).map_or(0, |p| p.0);
                if self.minor < 4 && sc != 1 && sc != 3 {
                    self.err(Rule::Interface, k, format!("before SPIR-V 1.4 the interface lists only Input / Output variables (%{})", v));
                }
                if !listed.insert(v) && self.minor >= 4 {
                    self.err(Rule::Interface, k, format!("%{} is listed twice in the interface", v));
                }
            }
            let tree = self.call_tree(func);
            let mut used = BTreeSet::new();
            for &n in &tree {
                used.extend(self.funcs[n].globals.iter().copied());
            }
            let mut pushc = 0;
            for &v in &used {
                let sc = self.vty.get(&v).and_then(|&t| self.sc_of_ptr(t)).map_or(0, |p| p.0);
                if (self.minor >= 4 || sc == 1 || sc == 3) && !listed.contains(&v) {
                    self.err(Rule::Interface, k, format!("entry point {:?} uses %{}, which its interface does not list", name, v));
                }
                if sc == 9 {
                    pushc += 1;
                }
                if sc == 4 && model != Model::GLCompute as u32 && model != Model::TaskEXT as u32 && model != Model::MeshEXT as u32 {
                    self.err(Rule::ExecMode, k, format!("Workgroup variable %{} used outside a compute entry point", v));
                }
            }
            if pushc > 1 {
                self.err(Rule::Storage, k, format!("entry point {:?} uses {} push-constant variables (at most one)", name, pushc));
            }
            // execution modes
            let ms: Vec<(u32, usize)> = modes.get(&func).cloned().unwrap_or_default();
            let has = |m: Mode| ms.iter().any(|x| x.0 == m as u32);
            let frag_modes = [Mode::OriginUpperLeft, Mode::OriginLowerLeft, Mode::DepthReplacing, Mode::EarlyFragmentTests,
                              Mode::DepthGreater, Mode::DepthLess, Mode::DepthUnchanged, Mode::PixelCenterInteger];
            for &(m, mk) in &ms {
                let mname = spv::name(spv::MODES, m).unwrap_or("?");
                let frag = frag_modes.iter().any(|&x| x as u32 == m);
                let comp = m == Mode::LocalSize as u32 || m == Mode::LocalSizeId as u32 || m == Mode::LocalSizeHint as u32;
                if (frag && model != Model::Fragment as u32) || (comp && !matches!(model, 5 | 6 | 5364 | 5365)) {
                    self.err(Rule::ExecMode, mk, format!("execution mode {} does not fit the entry point's model", mname));
                }
                let id_mode = matches!(m, 37..=39); // SubgroupsPerWorkgroupId, LocalSizeId, LocalSizeHintId
                let is_id = self.ins[mk].op == op::ExecutionModeId;
                if id_mode != is_id {
                    self.err(Rule::ExecMode, mk, format!("execution mode {} belongs in {}", mname,
                                                         if id_mode { "OpExecutionModeId" } else { "OpExecutionMode" }));
                }
                if is_id && self.minor < 2 {
                    self.err(Rule::Version, mk, String::from("OpExecutionModeId needs SPIR-V 1.2"));
                }
                if m == Mode::LocalSize as u32 {
                    let a = &self.ins[mk].ops[2..];
                    if a.len() != 3 || a.contains(&0) {
                        self.err(Rule::ExecMode, mk, String::from("LocalSize needs three sizes of at least 1"));
                    }
                }
                if m == Mode::LocalSizeId as u32 && is_id {
                    let a: Vec<u32> = self.ins[mk].ops[2..].to_vec();
                    let ok = a.len() == 3 && a.iter().all(|&x| {
                        let t = self.vty.get(&x).copied().unwrap_or(0);
                        self.kind(x) == Some(K::Const) && self.sv(t).is_some_and(|s| s.k == 'i' && s.n == 1 && s.width == 32)
                            && self.consts.get(&x).is_none_or(|&v| v >= 1)
                    });
                    if !ok {
                        self.err(Rule::ExecMode, mk, String::from("LocalSizeId needs three 32-bit integer constants (or specialization constants), plain ones at least 1"));
                    }
                }
            }
            if model == Model::GLCompute as u32 && !has(Mode::LocalSize) && !has(Mode::LocalSizeId) {
                let wg = self.decs.iter().any(|(_, l)| l.iter().any(|d| d.0 == Dec::BuiltIn as u32 && d.1.first() == Some(&(Bi::WorkgroupSize as u32))));
                if !wg {
                    self.err(Rule::ExecMode, k, format!("GLCompute entry point {:?} needs LocalSize", name));
                }
            }
            if model == Model::Fragment as u32 {
                if !has(Mode::OriginUpperLeft) {
                    self.err(Rule::ExecMode, k, format!("Fragment entry point {:?} needs OriginUpperLeft (Vulkan)", name));
                }
                if has(Mode::OriginLowerLeft) {
                    self.err(Rule::ExecMode, k, String::from("Vulkan does not allow OriginLowerLeft"));
                }
                let writes_depth = used.iter().any(|v| has_dec(self.decs.get(v), Dec::BuiltIn)
                    .is_some_and(|a| a.first() == Some(&(Bi::FragDepth as u32))));
                if writes_depth && !has(Mode::DepthReplacing) {
                    self.err(Rule::ExecMode, k, String::from("a fragment shader that writes FragDepth needs DepthReplacing"));
                }
            } else {
                let bad: Vec<(usize, &'static str)> = tree.iter().flat_map(|&n| self.funcs[n].frag_only.iter().copied()).collect();
                for (bk, bn) in bad {
                    let mn = spv::name(spv::MODELS, model).unwrap_or("?");
                    self.err(Rule::ExecMode, bk, format!("Op{} needs the Fragment model (reached from the {} entry point {:?})", bn, mn, name));
                }
            }
        }
    }

    fn capabilities(&mut self) {
        let mut have = self.caps.clone();
        if have.contains(&(Cap::Shader as u32)) {
            have.insert(Cap::Matrix as u32);
        }
        if have.contains(&(Cap::Geometry as u32)) || have.contains(&(Cap::Tessellation as u32)) {
            have.insert(Cap::Shader as u32);
        }
        if have.contains(&(Cap::Int64Atomics as u32)) {
            have.insert(Cap::Int64 as u32);
        }
        let need: Vec<(u32, usize)> = self.need.iter().map(|(&c, &k)| (c, k)).collect();
        for (c, k) in need {
            if !have.contains(&c) {
                let cn = spv::name(spv::CAPS, c).unwrap_or("?");
                self.err(Rule::Capability, k, format!("the capability {} is needed but not declared", cn));
            }
        }
    }
}

/// check a module (the rules of the module comment) -> Ok or every error found
pub fn validate(words: &[u32]) -> Result<(), Vec<Error>> {
    let e = |msg: &str| Err(alloc::vec![Error { rule: Rule::Header, at: 0, msg: String::from(msg) }]);
    if words.len() < 5 {
        return e("fewer than 5 words");
    }
    if words[0] != spv::MAGIC {
        return e("wrong magic number");
    }
    let (major, minor) = (words[1] >> 16, words[1] >> 8 & 0xff);
    if major != 1 || minor > 6 || words[1] & 0xff00_00ff != 0 {
        return e("SPIR-V version must be 1.0 ..= 1.6");
    }
    if words[4] != 0 {
        return e("the schema word must be 0");
    }
    let ins = match parse::parse(words) {
        Ok(v) => v,
        Err(m) => return Err(alloc::vec![Error { rule: Rule::Grammar, at: 0, msg: m }]),
    };
    let mut v = V {
        ins,
        minor,
        errs: Vec::new(),
        defs: BTreeMap::new(),
        tys: BTreeMap::new(),
        vty: BTreeMap::new(),
        consts: BTreeMap::new(),
        caps: BTreeSet::new(),
        exts: BTreeSet::new(),
        sets: BTreeMap::new(),
        decs: BTreeMap::new(),
        mdecs: BTreeMap::new(),
        funcs: Vec::new(),
        need: BTreeMap::new(),
    };
    for k in 0..v.ins.len() {
        let r = v.ins[k].res;
        if r >= words[3] {
            v.err(Rule::Header, k, format!("%{} is not below the bound {}", r, words[3]));
        }
    }
    v.structure();
    if v.errs.iter().any(|e| e.rule == Rule::Grammar || e.rule == Rule::IdUnique) {
        return Err(v.errs);
    }
    v.uses();
    v.types();
    v.decorations();
    v.variables();
    for k in 0..v.ins.len() {
        if v.defs.get(&v.ins[k].res).is_some_and(|d| d.func == NOFN && d.k == K::Const) {
            v.check_module_inst(k);
        }
    }
    v.functions();
    v.entry_points();
    v.capabilities();
    if v.errs.is_empty() { Ok(()) } else { Err(v.errs) }
}

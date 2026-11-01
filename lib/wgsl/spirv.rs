// spirv -- the back end (step G2c of the GPU plan; docs/WGSL.md "Back-End"): the checked IR
// lowered to SPIR-V with lib/shader, one SPIR-V module per entry point
//
//   compile(src, &CompileOptions) -> Result<Output, Vec<Diagnostic>>   check + lower
//   compile_module(&Module, &CompileOptions)   the same for a module checked before
//   Output { entries, overrides, module, warnings }
//        EntryOut: name, stage, words, workgroup size (with the defaults), resources with
//        (group, binding, kind, minimum size), immediate bytes, stage inputs / outputs, the
//        device features the module needs, the specialization constants it declares
//   output.constants(entry, &[(key, value)])   the pipeline constants for lib/gpu (pipeline.rs)
//
// Levels: CompileOptions::minor = shader::BASIS (SPIR-V 1.3, Vulkan 1.1) or shader::VOLL
// (SPIR-V 1.6, Vulkan 1.3).
//
// Types: every WGSL type has a value type (function, private and workgroup memory, SSA values)
// and, when it is host-shareable, a laid-out type (Offset, ArrayStride, MatrixStride; uniform,
// storage and push-constant memory).  SPIR-V 1.4+ (Vulkan's VUID-StandaloneSpirv-None-10684)
// forbids explicit layouts outside those storage classes: at Voll the two are different types and
// loads and stores of composites convert with OpCopyLogical; at Basis (1.3) the laid-out type
// is the value type too.  Buffers: the store type is wrapped in a Block struct { inner } unless
// it is a struct with a runtime-sized array (that struct is the Block: Vulkan allows runtime
// arrays only as the last member of the Block itself).
//
// Overrides: by default every override an entry point uses is a specialization constant
// (SpecId = @id, or OVERRIDE_BASE + its index without @id); override-expressions that size a
// workgroup or an array are hidden u32 specialization constants (HIDDEN_BASE + the expression).
// Output::constants evaluates every override on the CPU (the front end's constant evaluator)
// and returns the values of all of them, so derived overrides need no OpSpecConstantOp.
// CompileOptions::constants substitutes the values instead (Tint's way): no specialization
// constants at all.
//
// The function bodies: spirv_func.rs (statements, expressions, memory, calls, the entry
// point wrapper), spirv_builtin.rs (the built-in functions and textures).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use shader::{Cap, Dec, Dim, Fmt, Id, Sc};

use crate::ir::*;
use crate::{Diagnostic, Module};

/// the SpecId of an override without @id: OVERRIDE_BASE + its index in Module::overrides
pub const OVERRIDE_BASE: u32 = 0x1_0000;
/// the SpecId of a hidden constant (a workgroup size or array count from an override-expression):
/// HIDDEN_BASE + the expression's index in Module::exprs
pub const HIDDEN_BASE: u32 = 0x2_0000;

/// what compile does
#[derive(Clone, Copy, Debug)]
pub struct CompileOptions<'a> {
    /// shader::BASIS (SPIR-V 1.3, Vulkan 1.1) or shader::VOLL (SPIR-V 1.6, Vulkan 1.3); 4 and 5
    /// work as well
    pub minor: u32,
    /// one entry point (None: every entry point of the module)
    pub entry: Option<&'a str>,
    /// None: overrides become specialization constants (set them with Output::constants);
    /// Some: the pipeline constants as WebGPU takes them (key: the override's name, or its @id
    /// as a decimal number), substituted as constants
    pub constants: Option<&'a [(&'a str, f64)]>,
    /// the front end's features (Options::features)
    pub features: u32,
    /// WebGPU's robustness: indices clamped to the array, vector, matrix, runtime array
    /// (arrayLength) and texture bounds, out-of-bounds texture stores skipped
    pub robust: bool,
    /// NoContraction on every float addition, subtraction, multiplication, division and
    /// product: no fused multiply-add, results as the operations are written (WGSL itself
    /// lets an implementation contract)
    pub exact: bool,
}

impl CompileOptions<'_> {
    /// level `minor`, every entry point, specialization constants, robust, contraction allowed
    pub const fn new(minor: u32) -> CompileOptions<'static> {
        CompileOptions { minor, entry: None, constants: None, features: crate::feature::ALL, robust: true, exact: false }
    }
}

impl Default for CompileOptions<'_> {
    fn default() -> Self {
        CompileOptions::new(shader::VOLL)
    }
}

/// what a binding holds
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    UniformBuffer,
    StorageBuffer { read_only: bool },
    /// texture_1d<T> ... texture_cube_array<T>, texture_multisampled_2d, the depth textures
    Texture { dim: TexDim, kind: TexKind },
    StorageTexture { dim: TexDim, format: TexelFormat, access: Access },
    Sampler { comparison: bool },
    /// texture_external: a plain 2D float texture here
    ExternalTexture,
}

/// a resource of an entry point (lib/gpu: one Binding per resource)
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub group: u32,
    pub binding: u32,
    pub name: String,
    pub kind: ResourceKind,
    /// buffers: the smallest binding size the shader may access (a runtime-sized array with one
    /// element; WebGPU's minBindingSize); 0 otherwise
    pub min_size: u64,
}

/// one stage input or output
#[derive(Clone, Debug, PartialEq)]
pub struct StageVar {
    pub name: String,
    pub kind: IoKind,
    /// the WGSL type (Module::types)
    pub ty: TypeId,
}

/// device features a module needs beyond the level's core
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// f16 arithmetic (shaderFloat16)
    pub f16: bool,
    /// f16 in storage buffers / uniform buffers / push constants (the 16-bit storage features)
    pub storage16: bool,
    pub uniform16: bool,
    pub push16: bool,
    /// a uniform buffer whose layout is not std140 (uniformBufferStandardLayout)
    pub uniform_standard_layout: bool,
    /// OpDemoteToHelperInvocation (Voll: discard)
    pub demote: bool,
    /// 64-bit atomics (atomicStoreMin / Max on atomic<vec2<u32>>)
    pub int64_atomics: bool,
    /// storage textures without a format in the shader (bgra8unorm)
    pub storage_without_format: bool,
    /// @builtin(primitive_index): the Geometry capability (geometryShader)
    pub primitive_index: bool,
    /// @builtin(clip_distances) (shaderClipDistance)
    pub clip_distance: bool,
    /// @interpolate(.., sample), @builtin(sample_index) (sampleRateShading)
    pub sample_rate_shading: bool,
    /// subgroup operations on f16 (shaderSubgroupExtendedTypes)
    pub subgroup_extended_types: bool,
    /// the subgroup operations a module uses (SUBGROUP_* of lib/gpu's Features::subgroup_ops):
    /// 1 basic, 2 vote, 4 arithmetic, 8 ballot, 16 shuffle, 32 shuffle relative, 128 quad
    pub subgroup_ops: u32,
}

/// where a specialization constant's value comes from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecSource {
    /// an override (Module::overrides index)
    Override(u32),
    /// a module-scope override-expression as u32 (Module::exprs index)
    Expr(ExprId),
}

/// an entry point compiled
#[derive(Clone, Debug)]
pub struct EntryOut {
    pub name: String,
    pub stage: Stage,
    /// the SPIR-V module of this entry point (its entry point has the same name)
    pub words: Vec<u32>,
    /// compute: the workgroup size with the overrides' defaults (None: an override without
    /// default decides it; Output::workgroup_size computes it for pipeline constants)
    pub workgroup_size: Option<[u32; 3]>,
    /// @subgroup_size with the overrides' defaults (None: not written, or an override without
    /// default; Output::subgroup_size computes it for pipeline constants)
    pub subgroup_size: Option<u32>,
    pub resources: Vec<Resource>,
    /// bytes of the var<immediate> block (0: none); lib/gpu: PipelineLayoutDesc::push_constants
    pub immediate_size: u32,
    pub inputs: Vec<StageVar>,
    pub outputs: Vec<StageVar>,
    pub needs: Needs,
    /// the specialization constants declared: (SpecId, source)
    pub spec: Vec<(u32, SpecSource)>,
    /// the overrides the entry point uses (Module::overrides indices)
    pub overrides: Vec<u32>,
}

/// an override of the module
#[derive(Clone, Debug, PartialEq)]
pub struct OverrideInfo {
    pub name: String,
    /// the SpecId (= @id when written)
    pub id: u32,
    /// whether @id was written (then WebGPU keys the constant by the number, not the name)
    pub explicit_id: bool,
    pub ty: Scalar,
    /// it has an initializer (else a pipeline must set it)
    pub has_default: bool,
}

/// the result of compile
#[derive(Clone, Debug)]
pub struct Output {
    pub entries: Vec<EntryOut>,
    pub overrides: Vec<OverrideInfo>,
    /// the checked module (Output::constants evaluates its overrides)
    pub module: Module,
    pub warnings: Vec<Diagnostic>,
    pub minor: u32,
}

impl Output {
    /// an entry point by name
    pub fn entry(&self, name: &str) -> Option<&EntryOut> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// check and compile WGSL source (see the module comment)
pub fn compile(src: &str, opt: &CompileOptions) -> Result<Output, Vec<Diagnostic>> {
    let m = crate::check_with(src, &crate::Options { features: opt.features })?;
    compile_module(&m, opt)
}

fn diag(msg: String) -> Diagnostic {
    Diagnostic { line: 1, col: 1, end_col: 1, error: true, info: false, msg, start: 0, end: 0, notes: Vec::new() }
}

/// compile a checked module
pub fn compile_module(m: &Module, opt: &CompileOptions) -> Result<Output, Vec<Diagnostic>> {
    if !(3..=6).contains(&opt.minor) {
        return Err(alloc::vec![diag(format!("SPIR-V 1.{} gibt es hier nicht (1.3 bis 1.6)", opt.minor))]);
    }
    let overrides: Vec<OverrideInfo> = m.overrides.iter().enumerate().map(|(i, o)| OverrideInfo {
        name: o.name.clone(),
        id: o.id.map_or(OVERRIDE_BASE + i as u32, |n| n as u32),
        explicit_id: o.id.is_some(),
        ty: scalar_of(m, o.ty),
        has_default: o.init.is_some(),
    }).collect();
    let mut entries = Vec::new();
    let mut errs = Vec::new();
    if let Some(name) = opt.entry {
        if m.entry_point(name).is_none() {
            return Err(alloc::vec![diag(format!("das Modul hat keinen Einstiegspunkt `{}`", name))]);
        }
    }
    for ep in &m.entry_points {
        if opt.entry.is_some_and(|n| n != ep.name) {
            continue;
        }
        // the values of the overrides: substituted, or the defaults of the specialization
        // constants
        let (values, subst) = match opt.constants {
            Some(c) => match crate::pipeline::override_values(m, ep, c) {
                Ok(v) => (v, true),
                Err(e) => {
                    errs.push(diag(e));
                    continue;
                }
            },
            None => (crate::pipeline::default_values(m), false),
        };
        match crate::spirv_func::lower_entry(m, ep, opt, values, subst, &overrides) {
            Ok(e) => entries.push(e),
            Err(e) => errs.push(diag(e)),
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(Output { entries, overrides, module: m.clone(), warnings: m.warnings.clone(), minor: opt.minor })
}

/// compile every function of a module without entry points (a library: the functions are
/// lowered with value and pointer parameters, uncalled, beside an empty compute entry point
/// `library`; functions with texture or sampler parameters are left out).  For tests: SPIR-V
/// validators check every function of a module.
pub fn compile_library(src: &str, opt: &CompileOptions) -> Result<Output, Vec<Diagnostic>> {
    let m = crate::check_with(src, &crate::Options { features: opt.features })?;
    let overrides: Vec<OverrideInfo> = m.overrides.iter().enumerate().map(|(i, o)| OverrideInfo {
        name: o.name.clone(),
        id: o.id.map_or(OVERRIDE_BASE + i as u32, |n| n as u32),
        explicit_id: o.id.is_some(),
        ty: scalar_of(&m, o.ty),
        has_default: o.init.is_some(),
    }).collect();
    let values = crate::pipeline::default_values(&m);
    let e = crate::spirv_func::lower_library(&m, opt, values, &overrides).map_err(|e| alloc::vec![diag(e)])?;
    Ok(Output { entries: alloc::vec![e], overrides, module: m.clone(), warnings: m.warnings.clone(), minor: opt.minor })
}

pub(crate) fn scalar_of(m: &Module, t: TypeId) -> Scalar {
    match *m.ty(t) {
        Type::Scalar(s) | Type::Vector(_, s) | Type::Matrix(_, _, s) => s,
        Type::Atomic(i) => scalar_of(m, i),
        _ => Scalar::U32,
    }
}

// ---------------------------------------------------------------- the generator

/// a step of an access chain: a constant index (struct members, constant array indices) or a
/// dynamic one (an u32 value)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Const(u32),
    Dyn(Id),
}

/// a pointer or reference: the memory object (an OpVariable or a pointer parameter), its
/// storage class, the access chain to the referenced part and that part's WGSL type
#[derive(Clone, Debug)]
pub(crate) struct PtrVal {
    pub base: Id,
    /// the module-scope variable at the root (None: a function variable or parameter)
    pub global: Option<u32>,
    pub sc: Sc,
    /// the WGSL type the base points at (wrapped buffers: the store type; the chain starts
    /// with the wrapper's member 0)
    pub root: TypeId,
    pub chain: Vec<Step>,
    pub ty: TypeId,
}

/// a function specialized for its pointer and handle arguments: (function, per parameter)
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FnKey {
    pub f: u32,
    pub args: Vec<ArgKey>,
}

/// how a parameter is passed: by value; a handle (texture, sampler) or a pointer into a
/// module-scope variable fixed by the specialization (dynamic indices become u32
/// parameters); a pointer into a function variable passed as its root variable plus indices
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArgKey {
    Value,
    Handle(u32),
    /// global, the chain shape (Some: constant index)
    Global(u32, Vec<Option<u32>>),
    /// the root's storage class (Function, or any class in a library module) and WGSL type,
    /// the chain shape
    Local(u32, TypeId, Vec<Option<u32>>),
}

#[derive(Clone, Debug)]
pub(crate) struct GVar {
    pub var: Id,
    pub sc: Sc,
    /// the store type is wrapped in a Block struct: the chain starts with member 0
    pub wrapped: bool,
}

pub(crate) struct Gen<'a> {
    pub m: &'a Module,
    pub b: shader::Module,
    pub minor: u32,
    pub robust: bool,
    pub stage: Stage,
    pub ep: &'a EntryPoint,
    tids: BTreeMap<Type, TypeId>,
    tys: BTreeMap<(TypeId, u8), Id>,
    pub globals: BTreeMap<u32, GVar>,
    over: BTreeMap<u32, Id>,
    hidden: BTreeMap<ExprId, Id>,
    /// the override values: substituted (subst) or the specialization constants' defaults
    pub values: Vec<Option<Value>>,
    pub subst: bool,
    pub spec: Vec<(u32, SpecSource)>,
    pub override_ids: Vec<u32>,
    pub funcs: BTreeMap<FnKey, Id>,
    pub todo: Vec<FnKey>,
    /// Basis: the private flag discard sets (Tint's DemoteToHelper: writes are skipped after it,
    /// the wrapper ends with OpKill)
    pub discard_flag: Option<Id>,
    pub needs: Needs,
    pub errors: Vec<String>,
    /// built-in input variables the wrapper created itself (one per built-in)
    pub builtins: BTreeMap<u32, Id>,
}

impl<'a> Gen<'a> {
    pub fn new(m: &'a Module, ep: &'a EntryPoint, opt: &CompileOptions, values: Vec<Option<Value>>, subst: bool,
               override_ids: Vec<u32>) -> Gen<'a> {
        let mut b = shader::Module::new(opt.minor);
        b.no_contraction = opt.exact;
        let tids = m.types.iter().enumerate().map(|(i, t)| (*t, i as TypeId)).collect();
        Gen {
            m,
            b,
            minor: opt.minor,
            robust: opt.robust,
            stage: ep.stage,
            ep,
            tids,
            tys: BTreeMap::new(),
            globals: BTreeMap::new(),
            over: BTreeMap::new(),
            hidden: BTreeMap::new(),
            values,
            subst,
            spec: Vec::new(),
            override_ids,
            funcs: BTreeMap::new(),
            todo: Vec::new(),
            discard_flag: None,
            needs: Needs::default(),
            errors: Vec::new(),
            builtins: BTreeMap::new(),
        }
    }

    /// the TypeId of a type the module has (vectors and matrices are always there)
    pub fn tid(&self, t: Type) -> Option<TypeId> {
        self.tids.get(&t).copied()
    }

    /// vecN<s> (or s for n = 1) as a TypeId
    pub fn vec_tid(&self, n: u8, s: Scalar) -> TypeId {
        if n == 1 { s.id() } else { self.tid(Type::Vector(n, s)).unwrap_or(s.id()) }
    }

    // ---- types

    pub fn host_shareable(&self, t: TypeId) -> bool {
        match *self.m.ty(t) {
            Type::Scalar(s) | Type::Vector(_, s) | Type::Matrix(_, _, s) => s != Scalar::Bool,
            Type::Atomic(_) => true,
            Type::Array { elem, count, .. } => matches!(count, ArrayCount::Fixed(_) | ArrayCount::Runtime) && self.host_shareable(elem),
            Type::Struct(i) => self.m.structs[i as usize].members.iter().all(|mm| self.host_shareable(mm.ty)),
            _ => false,
        }
    }

    /// the SPIR-V type of values (and of function and private memory): Voll the plain type;
    /// Basis the laid-out type of every host-shareable part (so a value moves between buffers
    /// and functions unchanged), the parts around them plain
    pub fn ty(&mut self, t: TypeId) -> Id {
        let mode = if self.minor < 4 { VALUE } else { PLAIN };
        self.ty_of(t, mode)
    }

    /// the SPIR-V type in uniform, storage and push-constant memory
    pub fn ty_mem(&mut self, t: TypeId) -> Id {
        self.ty_of(t, LAID)
    }

    /// the plain type (no layout decorations): workgroup memory and stage variables at Basis
    pub fn ty_plain(&mut self, t: TypeId) -> Id {
        self.ty_of(t, PLAIN)
    }

    /// whether t's laid-out parts make its value type differ from its plain type at Basis
    pub fn has_layout(&self, t: TypeId) -> bool {
        match *self.m.ty(t) {
            Type::Array { elem, .. } => self.host_shareable(t) || self.has_layout(elem),
            Type::Struct(i) => self.host_shareable(t) || self.m.structs[i as usize].members.iter().any(|mm| self.has_layout(mm.ty)),
            _ => false,
        }
    }

    /// the type of t in memory of storage class sc
    pub fn ty_in(&mut self, t: TypeId, sc: Sc) -> Id {
        if is_buffer(sc) {
            self.ty_mem(t)
        } else if sc == Sc::Workgroup {
            self.ty_plain(t)
        } else {
            self.ty(t)
        }
    }

    /// how loads and stores of t in memory of class sc convert between the memory type and
    /// the value type: Relayout::Copy (Voll: OpCopyLogical between a laid-out buffer type and
    /// the plain value type), Relayout::Manual (Basis: workgroup memory, where Vulkan forbids
    /// explicit layouts, holds the plain type and values have the laid-out one; member by
    /// member), Relayout::None
    pub fn relayout(&self, sc: Sc, t: TypeId) -> Relayout {
        let composite = matches!(*self.m.ty(t), Type::Array { .. } | Type::Struct(_));
        if !composite {
            Relayout::None
        } else if self.minor >= 4 && is_buffer(sc) {
            Relayout::Copy
        } else if self.minor < 4 && sc == Sc::Workgroup && self.has_layout(t) {
            Relayout::Manual
        } else {
            Relayout::None
        }
    }

    pub fn scalar_ty(&mut self, s: Scalar) -> Id {
        match s {
            Scalar::Bool => self.b.ty_bool(),
            Scalar::I32 | Scalar::AbstractInt => self.b.ty_i32(),
            Scalar::U32 => self.b.ty_u32(),
            Scalar::F16 => {
                self.needs.f16 = true;
                self.b.ty_float(16)
            }
            _ => self.b.ty_f32(),
        }
    }

    /// the column stride of a matrix type in memory
    pub fn matrix_stride(&self, t: TypeId) -> u32 {
        match *self.m.ty(t) {
            Type::Matrix(_, r, s) => {
                let e = if s == Scalar::F16 { 2 } else { 4 };
                if r == 2 { 2 * e } else { 4 * e }
            }
            _ => 0,
        }
    }

    /// the matrix inside t (through arrays), for ColMajor / MatrixStride on members
    fn inner_matrix(&self, mut t: TypeId) -> Option<TypeId> {
        loop {
            match *self.m.ty(t) {
                Type::Matrix(..) => return Some(t),
                Type::Array { elem, .. } => t = elem,
                _ => return None,
            }
        }
    }

    fn ty_of(&mut self, t: TypeId, mode: u8) -> Id {
        if let Some(&id) = self.tys.get(&(t, mode)) {
            return id;
        }
        if mode == VALUE && self.host_shareable(t) {
            let id = self.ty_of(t, LAID);
            self.tys.insert((t, mode), id);
            return id;
        }
        let lay = mode == LAID;
        let id = match *self.m.ty(t) {
            Type::Scalar(s) => self.scalar_ty(s),
            Type::Vector(n, s) => {
                let e = self.scalar_ty(s);
                self.b.ty_vec(e, n as u32)
            }
            Type::Matrix(c, r, s) => {
                let e = self.scalar_ty(s);
                let col = self.b.ty_vec(e, r as u32);
                self.b.ty_mat(col, c as u32)
            }
            Type::Atomic(i) => {
                if matches!(self.m.ty(i), Type::Vector(..)) {
                    // atomic<vec2<u32>>: a 64-bit integer (atomicStoreMin / Max)
                    self.needs.int64_atomics = true;
                    self.b.capability(Cap::Int64Atomics);
                    self.b.ty_int(64, false)
                } else {
                    self.ty_of(i, mode)
                }
            }
            Type::Array { elem, count, stride } => {
                let e = self.ty_of(elem, mode);
                match count {
                    ArrayCount::Fixed(n) => {
                        if lay {
                            self.b.ty_array_stride(e, n, stride)
                        } else {
                            self.b.ty_array(e, n)
                        }
                    }
                    ArrayCount::Runtime => self.b.ty_runtime_array(e, Some(stride)),
                    ArrayCount::Override(o) => {
                        let len = self.override_value(o);
                        self.b.ty_array_id(e, len)
                    }
                    ArrayCount::Expr(x) => {
                        let len = self.hidden(x);
                        self.b.ty_array_id(e, len)
                    }
                }
            }
            Type::Struct(i) => {
                let s = &self.m.structs[i as usize];
                let members: Vec<Id> = s.members.iter().map(|mm| mm.ty).collect::<Vec<_>>().into_iter().map(|mt| self.ty_of(mt, mode)).collect();
                let id = self.b.ty_struct(&members);
                let s = &self.m.structs[i as usize];
                self.b.name(id, &s.name);
                for (k, mm) in s.members.iter().enumerate() {
                    self.b.member_name(id, k as u32, &mm.name);
                    if lay {
                        self.b.member_decorate(id, k as u32, Dec::Offset, &[mm.offset]);
                        if let Some(mt) = self.inner_matrix(mm.ty) {
                            let ms = self.matrix_stride(mt);
                            self.b.member_decorate(id, k as u32, Dec::ColMajor, &[]);
                            self.b.member_decorate(id, k as u32, Dec::MatrixStride, &[ms]);
                        }
                    }
                }
                id
            }
            Type::Sampler { .. } => self.b.ty_sampler(),
            Type::Texture(dim, kind) => self.image_ty(dim, kind),
            _ => self.b.ty_void(),
        };
        self.tys.insert((t, mode), id);
        id
    }

    fn image_ty(&mut self, dim: TexDim, kind: TexKind) -> Id {
        let (sd, arrayed) = match dim {
            TexDim::D1 => (Dim::D1, 0),
            TexDim::D2 => (Dim::D2, 0),
            TexDim::D2Array => (Dim::D2, 1),
            TexDim::D3 => (Dim::D3, 0),
            TexDim::Cube => (Dim::Cube, 0),
            TexDim::CubeArray => (Dim::Cube, 1),
        };
        match kind {
            TexKind::Sampled(s) => {
                let st = self.scalar_ty(s);
                self.b.ty_image(st, sd, 0, arrayed, 0, 1, Fmt::Unknown)
            }
            TexKind::Multisampled(s) => {
                let st = self.scalar_ty(s);
                self.b.ty_image(st, Dim::D2, 0, 0, 1, 1, Fmt::Unknown)
            }
            TexKind::Depth => {
                let f = self.b.ty_f32();
                self.b.ty_image(f, sd, 1, arrayed, 0, 1, Fmt::Unknown)
            }
            TexKind::DepthMultisampled => {
                let f = self.b.ty_f32();
                self.b.ty_image(f, Dim::D2, 1, 0, 1, 1, Fmt::Unknown)
            }
            TexKind::Storage(f, _) => {
                let st = self.scalar_ty(f.channel());
                let (fmt, ext) = texel_format(f);
                if ext {
                    self.b.capability(Cap::StorageImageExtendedFormats);
                }
                if fmt == Fmt::Unknown {
                    self.needs.storage_without_format = true;
                }
                self.b.ty_image(st, sd, 0, arrayed, 0, 2, fmt)
            }
            TexKind::External => {
                let f = self.b.ty_f32();
                self.b.ty_image(f, Dim::D2, 0, 0, 0, 1, Fmt::Unknown)
            }
        }
    }

    /// the type of component i of a composite type
    pub fn comp_type(&self, t: TypeId, i: usize) -> TypeId {
        match *self.m.ty(t) {
            Type::Vector(_, s) => s.id(),
            Type::Matrix(_, r, s) => self.vec_tid(r, s),
            Type::Array { elem, .. } => elem,
            Type::Struct(k) => self.m.structs[k as usize].members.get(i).map(|mm| mm.ty).unwrap_or(BOOL),
            Type::Atomic(i) => i,
            _ => t,
        }
    }

    // ---- constants

    /// the constant of a folded value of WGSL type t
    pub fn constant(&mut self, v: &Value, t: TypeId) -> Id {
        match v {
            Value::Bool(x) => self.b.const_bool(*x),
            Value::I32(x) => self.b.const_i32(*x),
            Value::AInt(x) => self.b.const_i32(*x as i32),
            Value::U32(x) => self.b.const_u32(*x),
            Value::F32(x) => self.b.const_f32(*x),
            Value::AFloat(x) => self.b.const_f32(*x as f32),
            Value::F16(x) => {
                let h = self.scalar_ty(Scalar::F16);
                self.b.const_scalar(h, &[f16_bits(*x) as u32])
            }
            Value::Zero => {
                let ty = self.ty(t);
                self.b.const_null(ty)
            }
            Value::Comp(cs) => {
                if cs.len() > 65000 {
                    self.errors.push(format!("die Konstante vom Typ `{}` hat zu viele Elemente für SPIR-V ({})", self.m.type_name(t), cs.len()));
                    let ty = self.ty(t);
                    return self.b.const_null(ty);
                }
                let parts: Vec<Id> = (0..cs.len()).map(|i| {
                    let ct = self.comp_type(t, i);
                    self.constant(&cs[i], ct)
                }).collect();
                let ty = self.ty(t);
                self.b.const_composite(ty, &parts)
            }
        }
    }

    // ---- overrides

    /// the value of an override: its specialization constant (or its substituted constant)
    pub fn override_value(&mut self, o: u32) -> Id {
        if let Some(&id) = self.over.get(&o) {
            return id;
        }
        let ov = &self.m.overrides[o as usize];
        let ty = ov.ty;
        let val = self.values.get(o as usize).cloned().flatten();
        let id = if self.subst {
            let v = val.unwrap_or(Value::Zero);
            self.constant(&v, ty)
        } else {
            let sid = self.override_ids[o as usize];
            let id = match (scalar_of(self.m, ty), val) {
                (Scalar::Bool, v) => self.b.spec_bool(sid, matches!(v, Some(Value::Bool(true)))),
                (Scalar::I32, v) => self.b.spec_i32(sid, if let Some(Value::I32(x)) = v { x } else { 0 }),
                (Scalar::U32, v) => self.b.spec_u32(sid, if let Some(Value::U32(x)) = v { x } else { 0 }),
                (Scalar::F16, v) => {
                    let h = self.scalar_ty(Scalar::F16);
                    let bits = if let Some(Value::F16(x)) = v { f16_bits(x) } else { 0 };
                    self.b.spec_scalar(h, sid, &[bits as u32])
                }
                (_, v) => self.b.spec_f32(sid, if let Some(Value::F32(x)) = v { x } else { 0.0 }),
            };
            self.b.name(id, &self.m.overrides[o as usize].name.clone());
            self.spec.push((sid, SpecSource::Override(o)));
            id
        };
        self.over.insert(o, id);
        id
    }

    /// a module-scope override-expression (workgroup size, array count) as an u32 constant: a
    /// hidden specialization constant (HIDDEN_BASE + e) or the substituted value
    pub fn hidden(&mut self, e: ExprId) -> Id {
        if let Some(&id) = self.hidden.get(&e) {
            return id;
        }
        let v = crate::pipeline::eval_u32(self.m, e, &self.values).ok();
        let id = if self.subst {
            let x = v.unwrap_or(1);
            self.b.const_u32(x)
        } else {
            let sid = HIDDEN_BASE + e;
            self.spec.push((sid, SpecSource::Expr(e)));
            self.b.spec_u32(sid, v.unwrap_or(1).max(1))
        };
        self.hidden.insert(e, id);
        id
    }

    // ---- module-scope variables

    /// the SPIR-V variable of a module-scope variable (declared on first use)
    pub fn global(&mut self, g: u32) -> GVar {
        if let Some(v) = self.globals.get(&g) {
            return v.clone();
        }
        let gl = &self.m.globals[g as usize];
        let (space, ty, access, binding, init, name) = (gl.space, gl.ty, gl.access, gl.binding, gl.init, gl.name.clone());
        let gv = match space {
            AddressSpace::Private => {
                let vt = self.ty(ty);
                let init = match init.map(|e| self.m.exprs[e as usize].kind.clone()) {
                    Some(ExprKind::Const(v)) => self.constant(&v, ty),
                    Some(ExprKind::Override(o)) => self.override_value(o),
                    // other override-expressions: stored by the entry point wrapper
                    _ => self.b.const_null(vt),
                };
                let var = self.b.global(Sc::Private, vt, Some(init));
                GVar { var, sc: Sc::Private, wrapped: false }
            }
            AddressSpace::Workgroup => {
                let vt = self.ty_plain(ty);
                let var = self.b.workgroup(vt);
                GVar { var, sc: Sc::Workgroup, wrapped: false }
            }
            AddressSpace::Uniform | AddressSpace::Storage | AddressSpace::Immediate => {
                if self.contains(ty, Scalar::F16) {
                    match space {
                        AddressSpace::Uniform => {
                            self.needs.uniform16 = true;
                            self.b.capability(Cap::UniformAndStorageBuffer16BitAccess);
                        }
                        AddressSpace::Storage => {
                            self.needs.storage16 = true;
                            self.b.capability(Cap::StorageBuffer16BitAccess);
                        }
                        _ => {
                            self.needs.push16 = true;
                            self.b.capability(Cap::StoragePushConstant16);
                        }
                    }
                }
                if space == AddressSpace::Uniform && !self.std140(g) {
                    self.needs.uniform_standard_layout = true;
                }
                let runtime_struct = matches!(*self.m.ty(ty), Type::Struct(i) if self.m.structs[i as usize].runtime_sized);
                let (block, wrapped) = if runtime_struct {
                    (self.ty_mem(ty), false)
                } else {
                    let inner = self.ty_mem(ty);
                    let w = self.b.ty_struct(&[inner]);
                    self.b.name(w, &format!("{}_block", name));
                    self.b.member_name(w, 0, "inner");
                    self.b.member_decorate(w, 0, Dec::Offset, &[0]);
                    if let Some(mt) = self.inner_matrix(ty) {
                        let ms = self.matrix_stride(mt);
                        self.b.member_decorate(w, 0, Dec::ColMajor, &[]);
                        self.b.member_decorate(w, 0, Dec::MatrixStride, &[ms]);
                    }
                    (w, true)
                };
                let (sc, var) = match space {
                    AddressSpace::Immediate => (Sc::PushConstant, self.b.push_constant(block)),
                    _ => {
                        let sc = if space == AddressSpace::Uniform { Sc::Uniform } else { Sc::StorageBuffer };
                        let bd = binding.unwrap_or(Binding { group: 0, binding: 0 });
                        (sc, self.b.buffer(sc, block, bd.group, bd.binding))
                    }
                };
                if space == AddressSpace::Storage && access == Access::Read {
                    self.b.decorate(var, Dec::NonWritable, &[]);
                }
                GVar { var, sc, wrapped }
            }
            AddressSpace::Handle => {
                let it = self.ty(ty);
                let bd = binding.unwrap_or(Binding { group: 0, binding: 0 });
                let var = self.b.resource(it, bd.group, bd.binding);
                if let Type::Texture(_, TexKind::Storage(_, a)) = *self.m.ty(ty) {
                    match a {
                        Access::Read => self.b.decorate(var, Dec::NonWritable, &[]),
                        Access::Write => self.b.decorate(var, Dec::NonReadable, &[]),
                        Access::ReadWrite => {}
                    }
                }
                GVar { var, sc: Sc::UniformConstant, wrapped: false }
            }
            AddressSpace::Function => {
                let vt = self.ty(ty);
                let var = self.b.global(Sc::Private, vt, None);
                GVar { var, sc: Sc::Private, wrapped: false }
            }
        };
        self.b.name(gv.var, &name);
        self.globals.insert(g, gv.clone());
        gv
    }

    /// a uniform buffer's layout is std140 (Vulkan without uniformBufferStandardLayout): the
    /// strict WGSL uniform rules and no matrix with an 8-byte (or smaller) column stride
    fn std140(&self, g: u32) -> bool {
        let gl = &self.m.globals[g as usize];
        gl.std140 && !self.small_matrix(gl.ty)
    }

    fn small_matrix(&self, t: TypeId) -> bool {
        match *self.m.ty(t) {
            Type::Matrix(..) => self.matrix_stride(t) % 16 != 0,
            Type::Array { elem, .. } => self.small_matrix(elem),
            Type::Struct(i) => self.m.structs[i as usize].members.iter().any(|mm| self.small_matrix(mm.ty)),
            _ => false,
        }
    }

    /// whether t contains the scalar s
    pub fn contains(&self, t: TypeId, s: Scalar) -> bool {
        match *self.m.ty(t) {
            Type::Scalar(x) | Type::Vector(_, x) | Type::Matrix(_, _, x) => x == s,
            Type::Atomic(i) => self.contains(i, s),
            Type::Array { elem, .. } => self.contains(elem, s),
            Type::Struct(i) => self.m.structs[i as usize].members.iter().any(|mm| self.contains(mm.ty, s)),
            _ => false,
        }
    }

    /// the pointer to (all of) a module-scope variable
    pub fn global_ptr(&mut self, g: u32) -> PtrVal {
        let v = self.global(g);
        let ty = self.m.globals[g as usize].ty;
        PtrVal { base: v.var, global: Some(g), sc: v.sc, root: ty, chain: if v.wrapped { alloc::vec![Step::Const(0)] } else { Vec::new() }, ty }
    }

    /// a function specialized for key (built later from the work list)
    pub fn func(&mut self, key: FnKey) -> Id {
        if let Some(&id) = self.funcs.get(&key) {
            return id;
        }
        let id = self.b.id();
        self.funcs.insert(key.clone(), id);
        self.todo.push(key);
        id
    }

    /// the size and alignment of a WGSL type in memory
    pub fn layout(&self, t: TypeId) -> (u32, u32) {
        match *self.m.ty(t) {
            Type::Scalar(s) => (s.size().min(4), s.size().min(4)),
            Type::Atomic(i) => self.layout(i),
            Type::Vector(n, s) => {
                let e = s.size().min(4);
                (if n == 2 { 2 * e } else { 4 * e }, n as u32 * e)
            }
            Type::Matrix(c, r, s) => {
                let (a, sz) = self.layout(self.vec_tid(r, s));
                (a, c as u32 * sz.div_ceil(a) * a)
            }
            Type::Array { elem, count, stride } => {
                let (a, _) = self.layout(elem);
                let n = match count {
                    ArrayCount::Fixed(n) => n,
                    _ => 0,
                };
                (a, n.saturating_mul(stride))
            }
            Type::Struct(i) => {
                let s = &self.m.structs[i as usize];
                (s.align, s.size)
            }
            _ => (4, 4),
        }
    }
}

// the type modes of Gen::ty_of: no layout decorations, laid out (buffers), values at Basis
const PLAIN: u8 = 0;
const LAID: u8 = 1;
const VALUE: u8 = 2;

/// see Gen::relayout
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relayout {
    None,
    Copy,
    Manual,
}

/// uniform, storage buffer or push-constant memory (laid-out types)
pub(crate) fn is_buffer(sc: Sc) -> bool {
    matches!(sc, Sc::Uniform | Sc::StorageBuffer | Sc::PushConstant)
}

/// the SPIR-V image format of a texel format and whether it needs StorageImageExtendedFormats
pub(crate) fn texel_format(f: TexelFormat) -> (Fmt, bool) {
    use TexelFormat as T;
    match f {
        T::Rgba8unorm => (Fmt::Rgba8, false),
        T::Rgba8snorm => (Fmt::Rgba8Snorm, false),
        T::Rgba8uint => (Fmt::Rgba8ui, false),
        T::Rgba8sint => (Fmt::Rgba8i, false),
        T::Rgba16unorm => (Fmt::Rgba16, true),
        T::Rgba16snorm => (Fmt::Rgba16Snorm, true),
        T::Rgba16uint => (Fmt::Rgba16ui, false),
        T::Rgba16sint => (Fmt::Rgba16i, false),
        T::Rgba16float => (Fmt::Rgba16f, false),
        T::Rg8unorm => (Fmt::Rg8, true),
        T::Rg8snorm => (Fmt::Rg8Snorm, true),
        T::Rg8uint => (Fmt::Rg8ui, true),
        T::Rg8sint => (Fmt::Rg8i, true),
        T::Rg16unorm => (Fmt::Rg16, true),
        T::Rg16snorm => (Fmt::Rg16Snorm, true),
        T::Rg16uint => (Fmt::Rg16ui, true),
        T::Rg16sint => (Fmt::Rg16i, true),
        T::Rg16float => (Fmt::Rg16f, true),
        T::R32uint => (Fmt::R32ui, false),
        T::R32sint => (Fmt::R32i, false),
        T::R32float => (Fmt::R32f, false),
        T::Rg32uint => (Fmt::Rg32ui, true),
        T::Rg32sint => (Fmt::Rg32i, true),
        T::Rg32float => (Fmt::Rg32f, true),
        T::Rgba32uint => (Fmt::Rgba32ui, false),
        T::Rgba32sint => (Fmt::Rgba32i, false),
        T::Rgba32float => (Fmt::Rgba32f, false),
        T::Bgra8unorm => (Fmt::Unknown, false),
        T::R8unorm => (Fmt::R8, true),
        T::R8snorm => (Fmt::R8Snorm, true),
        T::R8uint => (Fmt::R8ui, true),
        T::R8sint => (Fmt::R8i, true),
        T::R16unorm => (Fmt::R16, true),
        T::R16snorm => (Fmt::R16Snorm, true),
        T::R16uint => (Fmt::R16ui, true),
        T::R16sint => (Fmt::R16i, true),
        T::R16float => (Fmt::R16f, true),
        T::Rgb10a2unorm => (Fmt::Rgb10A2, true),
        T::Rgb10a2uint => (Fmt::Rgb10a2ui, true),
        T::Rg11b10ufloat => (Fmt::R11fG11fB10f, true),
    }
}

/// the resource list of an entry point
pub(crate) fn resources(g: &Gen) -> Vec<Resource> {
    let m = g.m;
    let mut out = Vec::new();
    for &(gi, bd) in &g.ep.resources {
        let gl = &m.globals[gi as usize];
        let kind = match gl.space {
            AddressSpace::Uniform => ResourceKind::UniformBuffer,
            AddressSpace::Storage => ResourceKind::StorageBuffer { read_only: gl.access == Access::Read },
            _ => match *m.ty(gl.ty) {
                Type::Sampler { comparison } => ResourceKind::Sampler { comparison },
                Type::Texture(_, TexKind::External) => ResourceKind::ExternalTexture,
                Type::Texture(dim, TexKind::Storage(format, access)) => ResourceKind::StorageTexture { dim, format, access },
                Type::Texture(dim, kind) => ResourceKind::Texture { dim, kind },
                _ => ResourceKind::Sampler { comparison: false },
            },
        };
        let min_size = match gl.space {
            AddressSpace::Uniform | AddressSpace::Storage => min_binding_size(g, gl.ty),
            _ => 0,
        };
        out.push(Resource { group: bd.group, binding: bd.binding, name: gl.name.clone(), kind, min_size });
    }
    out.sort_by_key(|r| (r.group, r.binding));
    out
}

/// the smallest binding of a buffer's store type: a runtime-sized array counts one element
fn min_binding_size(g: &Gen, t: TypeId) -> u64 {
    match *g.m.ty(t) {
        Type::Array { count: ArrayCount::Runtime, stride, .. } => stride as u64,
        Type::Struct(i) => {
            let s = &g.m.structs[i as usize];
            match s.members.last() {
                Some(last) if s.runtime_sized => {
                    let st = match *g.m.ty(last.ty) {
                        Type::Array { stride, .. } => stride,
                        _ => 0,
                    };
                    let end = last.offset as u64 + st as u64;
                    end.div_ceil(s.align as u64) * s.align as u64
                }
                _ => s.size as u64,
            }
        }
        _ => g.layout(t).1 as u64,
    }
}

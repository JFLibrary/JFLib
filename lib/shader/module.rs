// module -- the SPIR-V module builder: Module collects capabilities, types, constants,
// globals and functions in their sections and finish() writes the words.
//
//   Module::new(minor)            SPIR-V 1.minor (3: Vulkan 1.1 "Basis", 6: Vulkan 1.3 "Voll");
//                                 Shader capability, GLSL450 memory model
//   types         ty_void/bool/int/u32/i32/float/f32/vec/mat/array/array_stride/runtime_array/
//                 struct_/struct_layout/ptr/func/image/sampler/sampled_image; deduplicated except
//                 structs and strided arrays (their decorations differ)
//   constants     const_u32/i32/f32/bool/scalar/composite/null (deduplicated)
//   spec consts   spec_u32/i32/f32/bool/scalar (SpecId), spec_composite, spec_op
//                 (OpSpecConstantOp); workgroup sizes from them: local_size_id (LocalSizeId),
//                 workgroup_size_builtin (the WorkgroupSize built-in), workgroup_size (the one
//                 the level allows); ty_array_id (override-sized arrays)
//   globals       global, input, output, builtin, buffer, push_constant, workgroup, resource;
//                 local (Function storage, placed in the entry block)
//   functions     begin_function -> (id, params), end_function, label, begin_block
//   control flow  branch, branch_cond, selection_merge, loop_merge, switch, ret, ret_value,
//                 unreachable, discard (OpKill / OpTerminateInvocation), demote, phi,
//                 if_then, if_else, loop_ (structured helpers)
//   instructions  arithmetic, comparisons, logic, select, conversions, access chains,
//                 load / store, composites, shuffles, dot / matrix products, GLSL.std.450,
//                 atomics, barriers, images, derivatives, calls; emit() for any opcode
//   finish()      the words; entry point interfaces are computed here (1.3: the Input /
//                 Output variables the call tree uses, 1.4+: every global it uses)
//
// Ids are handed out in call order, so the same calls give the same words.  Capabilities
// and extensions an instruction or type needs are added on the way (Int64 for 64-bit
// integers, DerivativeControl for Fine / Coarse derivatives ...).  The builder does not check
// types beyond what it needs to infer result types: val::validate does.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use crate::layout::{self, Rule};
use crate::parse;
use crate::spv::{self, op, Bi, Cap, Dec, Dim, Fmt, Model, Mode, Sc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Id(pub u32);

/// a SPIR-V type
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ty {
    Void,
    Bool,
    Int { width: u32, signed: bool },
    Float { width: u32 },
    Vector { elem: Id, n: u32 },
    Matrix { col: Id, cols: u32 },
    /// len: the id of an integer constant
    Array { elem: Id, len: Id },
    RuntimeArray { elem: Id },
    Struct { members: Vec<Id> },
    Pointer { sc: u32, pointee: Id },
    Function { ret: Id, params: Vec<Id> },
    Image { sampled: Id, dim: u32, depth: u32, arrayed: u32, ms: u32, usage: u32, format: u32 },
    Sampler,
    SampledImage { image: Id },
}

/// what the layout helper and the validator need to know about ids
pub trait Types {
    fn ty(&self, id: u32) -> Option<&Ty>;
    /// the value of a 32-bit integer constant
    fn const_u32(&self, id: u32) -> Option<u32>;
}

#[derive(Clone, Debug)]
enum Info {
    Free,
    Type(Ty),
    Value { ty: Id },
    Other,
}

struct Func {
    id: Id,
    head: Vec<u32>,
    vars: Vec<u32>,
    body: Vec<u32>,
    open: bool,
}

/// the labels of the loop being built (loop_): break = branch(merge), continue = branch(cont)
#[derive(Clone, Copy, Debug)]
pub struct Loop {
    pub merge: Id,
    pub cont: Id,
}

pub struct Module {
    minor: u32,
    /// the generator word of the header (0: unregistered tool)
    pub generator: u32,
    /// decorate every float add, sub, mul, div, rem and product with NoContraction (exact
    /// results: no fused multiply-add)
    pub no_contraction: bool,
    next: u32,
    caps: Vec<u32>,
    exts: Vec<String>,
    imports: Vec<u32>,
    glsl: Option<Id>,
    entries: Vec<(u32, Id, String)>,
    modes: Vec<u32>,
    debug: Vec<u32>,
    annot: Vec<u32>,
    annot_seen: BTreeSet<Vec<u32>>,
    globals: Vec<u32>,
    funcs: Vec<u32>,
    info: Vec<Info>,
    consts: BTreeMap<u32, u32>,
    spec: BTreeSet<u32>,
    keys: BTreeMap<Vec<u32>, Id>,
    f: Option<Func>,
}

fn enc(buf: &mut Vec<u32>, opc: u16, args: &[u32]) {
    buf.push(((args.len() as u32 + 1) << 16) | opc as u32);
    buf.extend_from_slice(args);
}

fn is_terminator(o: u16) -> bool {
    matches!(o, op::Branch | op::BranchConditional | op::Switch | op::Return | op::ReturnValue | op::Kill
        | op::Unreachable | op::TerminateInvocation)
}

fn float_arith(o: u16) -> bool {
    matches!(o, op::FAdd | op::FSub | op::FMul | op::FDiv | op::FRem | op::FMod | op::VectorTimesScalar
        | op::MatrixTimesScalar | op::VectorTimesMatrix | op::MatrixTimesVector | op::MatrixTimesMatrix | op::Dot)
}

impl Types for Module {
    fn ty(&self, id: u32) -> Option<&Ty> {
        match self.info.get(id as usize) {
            Some(Info::Type(t)) => Some(t),
            _ => None,
        }
    }
    fn const_u32(&self, id: u32) -> Option<u32> {
        self.consts.get(&id).copied()
    }
}

impl Module {
    /// a module for SPIR-V 1.minor (3 ..= 6) with the Shader capability and the logical
    /// GLSL450 memory model
    pub fn new(minor: u32) -> Module {
        assert!((3..=6).contains(&minor), "shader: SPIR-V 1.{} (1.3 ..= 1.6 are supported)", minor);
        let mut m = Module {
            minor,
            generator: 0,
            no_contraction: false,
            next: 1,
            caps: Vec::new(),
            exts: Vec::new(),
            imports: Vec::new(),
            glsl: None,
            entries: Vec::new(),
            modes: Vec::new(),
            debug: Vec::new(),
            annot: Vec::new(),
            annot_seen: BTreeSet::new(),
            globals: Vec::new(),
            funcs: Vec::new(),
            info: alloc::vec![Info::Free],
            consts: BTreeMap::new(),
            spec: BTreeSet::new(),
            keys: BTreeMap::new(),
            f: None,
        };
        m.capability(Cap::Shader);
        m
    }

    pub fn minor(&self) -> u32 {
        self.minor
    }

    fn alloc(&mut self, info: Info) -> Id {
        let id = Id(self.next);
        self.next += 1;
        self.info.push(info);
        id
    }

    /// a fresh id (for forward references: labels, functions called before they are built)
    pub fn id(&mut self) -> Id {
        self.alloc(Info::Other)
    }

    pub fn capability(&mut self, c: Cap) {
        if !self.caps.contains(&(c as u32)) {
            self.caps.push(c as u32);
        }
    }

    pub fn extension(&mut self, name: &str) {
        if !self.exts.iter().any(|e| e == name) {
            self.exts.push(String::from(name));
        }
    }

    /// the GLSL.std.450 import (created on first use)
    pub fn glsl_set(&mut self) -> Id {
        if let Some(g) = self.glsl {
            return g;
        }
        let g = self.alloc(Info::Other);
        let mut a = alloc::vec![g.0];
        a.extend(parse::string_words("GLSL.std.450"));
        enc(&mut self.imports, op::ExtInstImport, &a);
        self.glsl = Some(g);
        g
    }

    /// the type of a value id
    pub fn type_of(&self, v: Id) -> Id {
        match self.info.get(v.0 as usize) {
            Some(Info::Value { ty }) => *ty,
            _ => panic!("shader: %{} is not a value", v.0),
        }
    }

    /// the description of a type id
    pub fn ty(&self, t: Id) -> &Ty {
        match self.info.get(t.0 as usize) {
            Some(Info::Type(ty)) => ty,
            _ => panic!("shader: %{} is not a type", t.0),
        }
    }

    // ---- debug names and decorations

    pub fn name(&mut self, id: Id, s: &str) {
        let mut a = alloc::vec![id.0];
        a.extend(parse::string_words(s));
        enc(&mut self.debug, op::Name, &a);
    }

    pub fn member_name(&mut self, ty: Id, member: u32, s: &str) {
        let mut a = alloc::vec![ty.0, member];
        a.extend(parse::string_words(s));
        enc(&mut self.debug, op::MemberName, &a);
    }

    /// OpSource (e.g. Lang::WGSL, version 100); no text is embedded
    pub fn source(&mut self, lang: spv::Lang, version: u32) {
        enc(&mut self.debug, op::Source, &[lang as u32, version]);
    }

    fn annotate(&mut self, opc: u16, a: Vec<u32>) {
        let mut key = alloc::vec![opc as u32];
        key.extend_from_slice(&a);
        if self.annot_seen.insert(key) {
            enc(&mut self.annot, opc, &a);
        }
    }

    /// OpDecorate (the same decoration twice is written once)
    pub fn decorate(&mut self, target: Id, d: Dec, args: &[u32]) {
        let mut a = alloc::vec![target.0, d as u32];
        a.extend_from_slice(args);
        self.annotate(op::Decorate, a);
    }

    pub fn member_decorate(&mut self, ty: Id, member: u32, d: Dec, args: &[u32]) {
        let mut a = alloc::vec![ty.0, member, d as u32];
        a.extend_from_slice(args);
        self.annotate(op::MemberDecorate, a);
    }

    // ---- types

    fn ty_key(&mut self, key: Vec<u32>, t: Ty, opc: u16, args: &[u32]) -> Id {
        if let Some(&id) = self.keys.get(&key) {
            return id;
        }
        let id = self.alloc(Info::Type(t));
        let mut a = alloc::vec![id.0];
        a.extend_from_slice(args);
        enc(&mut self.globals, opc, &a);
        self.keys.insert(key, id);
        id
    }

    fn ty_plain(&mut self, t: Ty, opc: u16, args: &[u32]) -> Id {
        let mut key = alloc::vec![opc as u32];
        key.extend_from_slice(args);
        self.ty_key(key, t, opc, args)
    }

    pub fn ty_void(&mut self) -> Id {
        self.ty_plain(Ty::Void, op::TypeVoid, &[])
    }

    pub fn ty_bool(&mut self) -> Id {
        self.ty_plain(Ty::Bool, op::TypeBool, &[])
    }

    /// an integer type; 8, 16 and 64 bits add Int8 / Int16 / Int64
    pub fn ty_int(&mut self, width: u32, signed: bool) -> Id {
        match width {
            8 => self.capability(Cap::Int8),
            16 => self.capability(Cap::Int16),
            64 => self.capability(Cap::Int64),
            _ => {}
        }
        self.ty_plain(Ty::Int { width, signed }, op::TypeInt, &[width, signed as u32])
    }

    pub fn ty_u32(&mut self) -> Id {
        self.ty_int(32, false)
    }

    pub fn ty_i32(&mut self) -> Id {
        self.ty_int(32, true)
    }

    /// a float type; 16 and 64 bits add Float16 / Float64
    pub fn ty_float(&mut self, width: u32) -> Id {
        match width {
            16 => self.capability(Cap::Float16),
            64 => self.capability(Cap::Float64),
            _ => {}
        }
        self.ty_plain(Ty::Float { width }, op::TypeFloat, &[width])
    }

    pub fn ty_f32(&mut self) -> Id {
        self.ty_float(32)
    }

    pub fn ty_vec(&mut self, elem: Id, n: u32) -> Id {
        self.ty_plain(Ty::Vector { elem, n }, op::TypeVector, &[elem.0, n])
    }

    /// a matrix of `cols` columns of the vector type `col`
    pub fn ty_mat(&mut self, col: Id, cols: u32) -> Id {
        self.ty_plain(Ty::Matrix { col, cols }, op::TypeMatrix, &[col.0, cols])
    }

    /// an array of len elements without ArrayStride (Function, Private, Workgroup storage)
    pub fn ty_array(&mut self, elem: Id, len: u32) -> Id {
        let l = self.const_u32(len);
        self.ty_plain(Ty::Array { elem, len: l }, op::TypeArray, &[elem.0, l.0])
    }

    /// an array whose length is the id of an integer constant or specialization constant (WGSL:
    /// a workgroup array sized by an override expression)
    pub fn ty_array_id(&mut self, elem: Id, len: Id) -> Id {
        self.ty_plain(Ty::Array { elem, len }, op::TypeArray, &[elem.0, len.0])
    }

    /// an array decorated with ArrayStride (buffers); deduplicated by element, length, stride
    pub fn ty_array_stride(&mut self, elem: Id, len: u32, stride: u32) -> Id {
        let l = self.const_u32(len);
        let key = alloc::vec![op::TypeArray as u32, elem.0, l.0, u32::MAX, stride];
        let id = self.ty_key(key, Ty::Array { elem, len: l }, op::TypeArray, &[elem.0, l.0]);
        self.decorate(id, Dec::ArrayStride, &[stride]);
        id
    }

    /// an array laid out by `rule` (ArrayStride from the layout helper)
    pub fn ty_array_layout(&mut self, elem: Id, len: u32, rule: Rule) -> Id {
        let s = layout::stride(self, elem.0, rule).expect("shader: element without a layout");
        self.ty_array_stride(elem, len, s)
    }

    /// a runtime array; stride Some(s): decorated with ArrayStride s (buffers)
    pub fn ty_runtime_array(&mut self, elem: Id, stride: Option<u32>) -> Id {
        let mut key = alloc::vec![op::TypeRuntimeArray as u32, elem.0];
        if let Some(s) = stride {
            key.extend_from_slice(&[u32::MAX, s]);
        }
        let id = self.ty_key(key, Ty::RuntimeArray { elem }, op::TypeRuntimeArray, &[elem.0]);
        if let Some(s) = stride {
            self.decorate(id, Dec::ArrayStride, &[s]);
        }
        id
    }

    /// a struct (never deduplicated: two structs may differ in their decorations)
    pub fn ty_struct(&mut self, members: &[Id]) -> Id {
        let id = self.alloc(Info::Type(Ty::Struct { members: members.to_vec() }));
        let mut a = alloc::vec![id.0];
        a.extend(members.iter().map(|m| m.0));
        enc(&mut self.globals, op::TypeStruct, &a);
        id
    }

    /// a named struct laid out by `rule`: OpName / OpMemberName, Offset on every member,
    /// ColMajor and MatrixStride on matrix (and matrix array) members.  Array members must be
    /// built with ty_array_layout / ty_runtime_array(.., Some(stride)) of the same rule.
    pub fn ty_struct_layout(&mut self, name: &str, members: &[(&str, Id)], rule: Rule) -> Id {
        let ids: Vec<Id> = members.iter().map(|m| m.1).collect();
        let raw: Vec<u32> = ids.iter().map(|m| m.0).collect();
        let offs = layout::offsets(self, &raw, rule).expect("shader: struct member without a layout");
        let s = self.ty_struct(&ids);
        self.name(s, name);
        for (k, (mname, mty)) in members.iter().enumerate() {
            self.member_name(s, k as u32, mname);
            self.member_decorate(s, k as u32, Dec::Offset, &[offs[k]]);
            let mut t = *mty;
            while let Ty::Array { elem, .. } | Ty::RuntimeArray { elem } = self.ty(t) {
                t = *elem;
            }
            if let Ty::Matrix { .. } = self.ty(t) {
                let ms = layout::matrix_stride(self, t.0, rule).expect("shader: matrix layout");
                self.member_decorate(s, k as u32, Dec::ColMajor, &[]);
                self.member_decorate(s, k as u32, Dec::MatrixStride, &[ms]);
            }
        }
        s
    }

    pub fn ty_ptr(&mut self, sc: Sc, pointee: Id) -> Id {
        self.ty_plain(Ty::Pointer { sc: sc as u32, pointee }, op::TypePointer, &[sc as u32, pointee.0])
    }

    pub fn ty_func(&mut self, ret: Id, params: &[Id]) -> Id {
        let mut a = alloc::vec![ret.0];
        a.extend(params.iter().map(|p| p.0));
        self.ty_plain(Ty::Function { ret, params: params.to_vec() }, op::TypeFunction, &a)
    }

    /// OpTypeImage; usage 1: sampled, 2: storage image (format needed unless the
    /// *WithoutFormat capabilities are declared)
    #[allow(clippy::too_many_arguments)]
    pub fn ty_image(&mut self, sampled: Id, dim: Dim, depth: u32, arrayed: u32, ms: u32, usage: u32, format: Fmt) -> Id {
        let t = Ty::Image { sampled, dim: dim as u32, depth, arrayed, ms, usage, format: format as u32 };
        match (dim, arrayed, usage) {
            (Dim::D1, _, 1) => self.capability(Cap::Sampled1D),
            (Dim::D1, _, _) => self.capability(Cap::Image1D),
            (Dim::Buffer, _, 1) => self.capability(Cap::SampledBuffer),
            (Dim::Buffer, _, _) => self.capability(Cap::ImageBuffer),
            (Dim::Cube, 1, 1) => self.capability(Cap::SampledCubeArray),
            (Dim::Cube, 1, _) => self.capability(Cap::ImageCubeArray),
            _ => {}
        }
        self.ty_plain(t, op::TypeImage, &[sampled.0, dim as u32, depth, arrayed, ms, usage, format as u32])
    }

    pub fn ty_sampler(&mut self) -> Id {
        self.ty_plain(Ty::Sampler, op::TypeSampler, &[])
    }

    pub fn ty_sampled_image(&mut self, image: Id) -> Id {
        self.ty_plain(Ty::SampledImage { image }, op::TypeSampledImage, &[image.0])
    }

    /// bool (or a bool vector of the same length) for a scalar / vector type
    pub fn bool_like(&mut self, t: Id) -> Id {
        let b = self.ty_bool();
        match *self.ty(t) {
            Ty::Vector { n, .. } => self.ty_vec(b, n),
            _ => b,
        }
    }

    /// the component type of a vector (the type itself for a scalar)
    pub fn scalar_of(&self, t: Id) -> Id {
        match *self.ty(t) {
            Ty::Vector { elem, .. } => elem,
            _ => t,
        }
    }

    // ---- constants

    fn constant(&mut self, opc: u16, ty: Id, args: &[u32]) -> Id {
        let mut key = alloc::vec![opc as u32, ty.0];
        key.extend_from_slice(args);
        if let Some(&id) = self.keys.get(&key) {
            return id;
        }
        let id = self.alloc(Info::Value { ty });
        let mut a = alloc::vec![ty.0, id.0];
        a.extend_from_slice(args);
        enc(&mut self.globals, opc, &a);
        self.keys.insert(key, id);
        id
    }

    /// OpConstant of an int or float type with its value words (low word first)
    pub fn const_scalar(&mut self, ty: Id, words: &[u32]) -> Id {
        let id = self.constant(op::Constant, ty, words);
        if let (Ty::Int { width: 32, .. }, [w]) = (self.ty(ty), words) {
            let w = *w;
            self.consts.insert(id.0, w);
        }
        id
    }

    pub fn const_u32(&mut self, v: u32) -> Id {
        let t = self.ty_u32();
        self.const_scalar(t, &[v])
    }

    pub fn const_i32(&mut self, v: i32) -> Id {
        let t = self.ty_i32();
        self.const_scalar(t, &[v as u32])
    }

    pub fn const_f32(&mut self, v: f32) -> Id {
        let t = self.ty_f32();
        self.const_scalar(t, &[v.to_bits()])
    }

    pub fn const_bool(&mut self, v: bool) -> Id {
        let t = self.ty_bool();
        self.constant(if v { op::ConstantTrue } else { op::ConstantFalse }, t, &[])
    }

    pub fn const_composite(&mut self, ty: Id, parts: &[Id]) -> Id {
        let a: Vec<u32> = parts.iter().map(|p| p.0).collect();
        self.constant(op::ConstantComposite, ty, &a)
    }

    /// a float vector constant
    pub fn const_vec_f32(&mut self, v: &[f32]) -> Id {
        let f = self.ty_f32();
        let t = self.ty_vec(f, v.len() as u32);
        let parts: Vec<Id> = v.iter().map(|&x| self.const_f32(x)).collect();
        self.const_composite(t, &parts)
    }

    pub fn const_null(&mut self, ty: Id) -> Id {
        self.constant(op::ConstantNull, ty, &[])
    }

    // ---- specialization constants (WGSL override)

    /// whether v is a specialization constant (OpSpecConstant*, composites of them, results of
    /// OpSpecConstantOp)
    pub fn is_spec(&self, v: Id) -> bool {
        self.spec.contains(&v.0)
    }

    fn spec_value(&mut self, opc: u16, ty: Id, args: &[u32]) -> Id {
        let id = self.alloc(Info::Value { ty });
        let mut a = alloc::vec![ty.0, id.0];
        a.extend_from_slice(args);
        enc(&mut self.globals, opc, &a);
        self.spec.insert(id.0);
        id
    }

    /// OpSpecConstant of an int or float type with its default value words (low word first),
    /// decorated SpecId spec_id (the pipeline constant's id); never deduplicated
    pub fn spec_scalar(&mut self, ty: Id, spec_id: u32, words: &[u32]) -> Id {
        let id = self.spec_value(op::SpecConstant, ty, words);
        self.decorate(id, Dec::SpecId, &[spec_id]);
        id
    }

    pub fn spec_u32(&mut self, spec_id: u32, default: u32) -> Id {
        let t = self.ty_u32();
        self.spec_scalar(t, spec_id, &[default])
    }

    pub fn spec_i32(&mut self, spec_id: u32, default: i32) -> Id {
        let t = self.ty_i32();
        self.spec_scalar(t, spec_id, &[default as u32])
    }

    pub fn spec_f32(&mut self, spec_id: u32, default: f32) -> Id {
        let t = self.ty_f32();
        self.spec_scalar(t, spec_id, &[default.to_bits()])
    }

    /// OpSpecConstantTrue / OpSpecConstantFalse decorated SpecId spec_id
    pub fn spec_bool(&mut self, spec_id: u32, default: bool) -> Id {
        let t = self.ty_bool();
        let id = self.spec_value(if default { op::SpecConstantTrue } else { op::SpecConstantFalse }, t, &[]);
        self.decorate(id, Dec::SpecId, &[spec_id]);
        id
    }

    /// OpSpecConstantComposite of constants and specialization constants (deduplicated)
    pub fn spec_composite(&mut self, ty: Id, parts: &[Id]) -> Id {
        let a: Vec<u32> = parts.iter().map(|p| p.0).collect();
        let id = self.constant(op::SpecConstantComposite, ty, &a);
        self.spec.insert(id.0);
        id
    }

    /// OpSpecConstantOp: the operation opc on constant ids, then its literals (the indices of
    /// CompositeExtract / CompositeInsert, the components of VectorShuffle).  SPIR-V allows
    /// for Shader: SConvert, UConvert (1.4+), FConvert, QuantizeToF16, SNegate, Not, IAdd,
    /// ISub, IMul, UDiv, SDiv, UMod, SRem, SMod, the shifts, BitwiseOr / Xor / And,
    /// VectorShuffle, CompositeExtract / Insert, LogicalOr / And / Not / Equal / NotEqual,
    /// Select, IEqual, INotEqual and the integer comparisons (val checks it); no float
    /// arithmetic.
    pub fn spec_op(&mut self, ty: Id, opc: u16, ids: &[Id], literals: &[u32]) -> Id {
        let mut a = alloc::vec![opc as u32];
        a.extend(ids.iter().map(|i| i.0));
        a.extend_from_slice(literals);
        self.spec_value(op::SpecConstantOp, ty, &a)
    }

    // ---- global variables

    /// a global OpVariable of `pointee` in storage class sc
    pub fn global(&mut self, sc: Sc, pointee: Id, init: Option<Id>) -> Id {
        let p = self.ty_ptr(sc, pointee);
        let id = self.alloc(Info::Value { ty: p });
        let mut a = alloc::vec![p.0, id.0, sc as u32];
        a.extend(init.map(|i| i.0));
        enc(&mut self.globals, op::Variable, &a);
        id
    }

    pub fn input(&mut self, ty: Id, location: u32) -> Id {
        let v = self.global(Sc::Input, ty, None);
        self.decorate(v, Dec::Location, &[location]);
        v
    }

    pub fn output(&mut self, ty: Id, location: u32) -> Id {
        let v = self.global(Sc::Output, ty, None);
        self.decorate(v, Dec::Location, &[location]);
        v
    }

    /// a built-in Input or Output variable; adds the capability some built-ins need
    pub fn builtin(&mut self, sc: Sc, ty: Id, b: Bi) -> Id {
        match b {
            Bi::ClipDistance => self.capability(Cap::ClipDistance),
            Bi::CullDistance => self.capability(Cap::CullDistance),
            Bi::BaseVertex | Bi::BaseInstance | Bi::DrawIndex => self.capability(Cap::DrawParameters),
            Bi::SampleId | Bi::SamplePosition => self.capability(Cap::SampleRateShading),
            _ => {}
        }
        let v = self.global(sc, ty, None);
        self.decorate(v, Dec::BuiltIn, &[b as u32]);
        v
    }

    /// a uniform (Sc::Uniform) or storage (Sc::StorageBuffer) buffer of a laid-out struct:
    /// Block on the struct, DescriptorSet and Binding on the variable
    pub fn buffer(&mut self, sc: Sc, block: Id, set: u32, binding: u32) -> Id {
        self.decorate(block, Dec::Block, &[]);
        let v = self.global(sc, block, None);
        self.decorate(v, Dec::DescriptorSet, &[set]);
        self.decorate(v, Dec::Binding, &[binding]);
        v
    }

    /// the push-constant block (a struct laid out by Rule::Std430)
    pub fn push_constant(&mut self, block: Id) -> Id {
        self.decorate(block, Dec::Block, &[]);
        self.global(Sc::PushConstant, block, None)
    }

    pub fn workgroup(&mut self, ty: Id) -> Id {
        self.global(Sc::Workgroup, ty, None)
    }

    /// an image, sampler or sampled image (UniformConstant) at set / binding; a runtime array
    /// of them (a Bindless table) adds RuntimeDescriptorArray (before 1.5 with
    /// SPV_EXT_descriptor_indexing)
    pub fn resource(&mut self, ty: Id, set: u32, binding: u32) -> Id {
        if let Ty::RuntimeArray { .. } = self.ty(ty) {
            self.capability(Cap::RuntimeDescriptorArray);
            if self.minor < 5 {
                self.extension("SPV_EXT_descriptor_indexing");
            }
        }
        let v = self.global(Sc::UniformConstant, ty, None);
        self.decorate(v, Dec::DescriptorSet, &[set]);
        self.decorate(v, Dec::Binding, &[binding]);
        v
    }

    // ---- entry points

    /// an entry point; its interface is filled in by finish()
    pub fn entry_point(&mut self, model: Model, func: Id, name: &str) {
        self.entries.push((model as u32, func, String::from(name)));
    }

    pub fn exec_mode(&mut self, func: Id, mode: Mode, args: &[u32]) {
        let mut a = alloc::vec![func.0, mode as u32];
        a.extend_from_slice(args);
        enc(&mut self.modes, op::ExecutionMode, &a);
    }

    /// OpExecutionModeId (SPIR-V 1.2+): a mode whose operands are constant ids (LocalSizeId)
    pub fn exec_mode_id(&mut self, func: Id, mode: Mode, ids: &[Id]) {
        let mut a = alloc::vec![func.0, mode as u32];
        a.extend(ids.iter().map(|i| i.0));
        enc(&mut self.modes, op::ExecutionModeId, &a);
    }

    /// LocalSizeId x y z: the workgroup size of a GLCompute entry point from 32-bit integer
    /// constants or specialization constants.  Vulkan needs maintenance4 for it (core and
    /// required in Vulkan 1.3, so always there at the level Voll).
    pub fn local_size_id(&mut self, func: Id, size: [Id; 3]) {
        self.exec_mode_id(func, Mode::LocalSizeId, &size);
    }

    /// the WorkgroupSize built-in: a uvec3 composite of size (OpSpecConstantComposite when a
    /// part is a specialization constant) decorated BuiltIn WorkgroupSize.  It sets the
    /// workgroup size of every GLCompute entry point of the module and wins over LocalSize.
    pub fn workgroup_size_builtin(&mut self, size: [Id; 3]) -> Id {
        let u = self.ty_u32();
        let uv3 = self.ty_vec(u, 3);
        let c = if size.iter().any(|&p| self.is_spec(p)) { self.spec_composite(uv3, &size) } else { self.const_composite(uv3, &size) };
        self.decorate(c, Dec::BuiltIn, &[Bi::WorkgroupSize as u32]);
        c
    }

    /// a workgroup size of (specialization) constants the way the level allows: LocalSizeId
    /// from SPIR-V 1.6 (Voll, Vulkan 1.3), else the WorkgroupSize built-in (then for every
    /// compute entry point of the module: one module per entry point, as Basis needs it)
    pub fn workgroup_size(&mut self, func: Id, size: [Id; 3]) {
        if self.minor >= 6 {
            self.local_size_id(func, size);
        } else {
            self.workgroup_size_builtin(size);
        }
    }

    // ---- functions and blocks

    /// start a function (and its entry block) -> (function id, parameter ids)
    pub fn begin_function(&mut self, ret: Id, params: &[Id]) -> (Id, Vec<Id>) {
        let id = self.id();
        self.begin_function_id(id, ret, params)
    }

    /// begin_function with an id from id() (a function called before it is built)
    pub fn begin_function_id(&mut self, id: Id, ret: Id, params: &[Id]) -> (Id, Vec<Id>) {
        assert!(self.f.is_none(), "shader: begin_function inside a function");
        let fty = self.ty_func(ret, params);
        self.info[id.0 as usize] = Info::Value { ty: ret };
        let mut head = Vec::new();
        enc(&mut head, op::Function, &[ret.0, id.0, 0, fty.0]);
        let mut ps = Vec::new();
        for &p in params {
            let pid = self.alloc(Info::Value { ty: p });
            enc(&mut head, op::FunctionParameter, &[p.0, pid.0]);
            ps.push(pid);
        }
        let l = self.id();
        enc(&mut head, op::Label, &[l.0]);
        self.f = Some(Func { id, head, vars: Vec::new(), body: Vec::new(), open: true });
        (id, ps)
    }

    pub fn end_function(&mut self) {
        let f = self.f.take().expect("shader: end_function without begin_function");
        assert!(!f.open, "shader: the last block of %{} has no terminator", f.id.0);
        self.funcs.extend_from_slice(&f.head);
        self.funcs.extend_from_slice(&f.vars);
        self.funcs.extend_from_slice(&f.body);
        enc(&mut self.funcs, op::FunctionEnd, &[]);
    }

    /// a new label id (the block starts with begin_block)
    pub fn label(&mut self) -> Id {
        self.id()
    }

    pub fn begin_block(&mut self, l: Id) {
        let f = self.f.as_mut().expect("shader: a block outside a function");
        assert!(!f.open, "shader: begin_block while the previous block is open");
        enc(&mut f.body, op::Label, &[l.0]);
        f.open = true;
    }

    /// whether the current block still needs a terminator
    pub fn block_open(&self) -> bool {
        self.f.as_ref().is_some_and(|f| f.open)
    }

    /// a Function-storage variable (placed at the start of the entry block)
    pub fn local(&mut self, ty: Id, init: Option<Id>) -> Id {
        let p = self.ty_ptr(Sc::Function, ty);
        let id = self.alloc(Info::Value { ty: p });
        let mut a = alloc::vec![p.0, id.0, Sc::Function as u32];
        a.extend(init.map(|i| i.0));
        enc(&mut self.f.as_mut().expect("shader: local outside a function").vars, op::Variable, &a);
        id
    }

    /// any instruction in the current block: ty for opcodes with a result type; the result id
    /// is allocated when the opcode has one (Id(0) otherwise)
    pub fn emit(&mut self, opc: u16, ty: Option<Id>, args: &[u32]) -> Id {
        let res = spv::op_info(opc).map_or(spv::R_NONE, |o| o.res);
        let id = match res {
            spv::R_NONE => Id(0),
            _ => self.alloc(match ty {
                Some(t) => Info::Value { ty: t },
                None => Info::Other,
            }),
        };
        let mut a = Vec::with_capacity(args.len() + 2);
        if res == spv::R_TY {
            a.push(ty.expect("shader: the opcode needs a result type").0);
        }
        if res != spv::R_NONE {
            a.push(id.0);
        }
        a.extend_from_slice(args);
        let f = self.f.as_mut().expect("shader: an instruction outside a function");
        assert!(f.open, "shader: Op{} after the block's terminator", opc);
        enc(&mut f.body, opc, &a);
        if is_terminator(opc) {
            f.open = false;
        }
        if self.no_contraction && float_arith(opc) {
            self.decorate(id, Dec::NoContraction, &[]);
        }
        id
    }

    fn emit_ids(&mut self, opc: u16, ty: Id, ids: &[Id]) -> Id {
        let a: Vec<u32> = ids.iter().map(|i| i.0).collect();
        self.emit(opc, Some(ty), &a)
    }

    // ---- control flow

    pub fn branch(&mut self, target: Id) {
        self.emit(op::Branch, None, &[target.0]);
    }

    pub fn branch_cond(&mut self, c: Id, t: Id, f: Id) {
        self.emit(op::BranchConditional, None, &[c.0, t.0, f.0]);
    }

    /// OpSelectionMerge (ctl: spv::SEL_CTL bits, 0 = None)
    pub fn selection_merge(&mut self, merge: Id, ctl: u32) {
        self.emit(op::SelectionMerge, None, &[merge.0, ctl]);
    }

    /// OpLoopMerge (ctl: spv::LOOP_CTL bits without arguments, 0 = None)
    pub fn loop_merge(&mut self, merge: Id, cont: Id, ctl: u32) {
        self.emit(op::LoopMerge, None, &[merge.0, cont.0, ctl]);
    }

    /// OpSwitch on a 32-bit selector (the caller writes the OpSelectionMerge first)
    pub fn switch(&mut self, sel: Id, default: Id, cases: &[(u32, Id)]) {
        let mut a = alloc::vec![sel.0, default.0];
        for &(v, l) in cases {
            a.extend_from_slice(&[v, l.0]);
        }
        self.emit(op::Switch, None, &a);
    }

    pub fn ret(&mut self) {
        self.emit(op::Return, None, &[]);
    }

    pub fn ret_value(&mut self, v: Id) {
        self.emit(op::ReturnValue, None, &[v.0]);
    }

    pub fn unreachable(&mut self) {
        self.emit(op::Unreachable, None, &[]);
    }

    /// discard the fragment: OpKill before SPIR-V 1.6, OpTerminateInvocation from 1.6
    pub fn discard(&mut self) {
        self.emit(if self.minor >= 6 { op::TerminateInvocation } else { op::Kill }, None, &[]);
    }

    /// make the invocation a helper (not a terminator); before 1.6 through
    /// SPV_EXT_demote_to_helper_invocation
    pub fn demote(&mut self) {
        self.capability(Cap::DemoteToHelperInvocation);
        if self.minor < 6 {
            self.extension("SPV_EXT_demote_to_helper_invocation");
        }
        self.emit(op::DemoteToHelperInvocation, None, &[]);
    }

    /// OpPhi of (value, predecessor label) pairs
    pub fn phi(&mut self, ty: Id, pairs: &[(Id, Id)]) -> Id {
        let a: Vec<u32> = pairs.iter().flat_map(|p| [p.0 .0, p.1 .0]).collect();
        self.emit(op::Phi, Some(ty), &a)
    }

    /// if (c) { then }: a selection construct
    pub fn if_then(&mut self, c: Id, then: impl FnOnce(&mut Module)) {
        let (t, merge) = (self.label(), self.label());
        self.selection_merge(merge, 0);
        self.branch_cond(c, t, merge);
        self.begin_block(t);
        then(self);
        if self.block_open() {
            self.branch(merge);
        }
        self.begin_block(merge);
    }

    /// if (c) { then } else { els }
    pub fn if_else(&mut self, c: Id, then: impl FnOnce(&mut Module), els: impl FnOnce(&mut Module)) {
        let (t, e, merge) = (self.label(), self.label(), self.label());
        self.selection_merge(merge, 0);
        self.branch_cond(c, t, e);
        self.begin_block(t);
        then(self);
        if self.block_open() {
            self.branch(merge);
        }
        self.begin_block(e);
        els(self);
        if self.block_open() {
            self.branch(merge);
        }
        self.begin_block(merge);
    }

    /// while (cond) { body; cont }: header (OpLoopMerge), condition block, body, continue
    /// block, merge block.  body gets the labels for break / continue.
    pub fn loop_(&mut self, cond: impl FnOnce(&mut Module) -> Id, body: impl FnOnce(&mut Module, Loop),
                 cont: impl FnOnce(&mut Module)) {
        let (header, check, b, c, merge) = (self.label(), self.label(), self.label(), self.label(), self.label());
        self.branch(header);
        self.begin_block(header);
        self.loop_merge(merge, c, 0);
        self.branch(check);
        self.begin_block(check);
        let k = cond(self);
        self.branch_cond(k, b, merge);
        self.begin_block(b);
        body(self, Loop { merge, cont: c });
        if self.block_open() {
            self.branch(c);
        }
        self.begin_block(c);
        cont(self);
        self.branch(header);
        self.begin_block(merge);
    }

    // ---- arithmetic (the result type is the first operand's)

    pub fn binary(&mut self, opc: u16, a: Id, b: Id) -> Id {
        let t = self.type_of(a);
        self.emit_ids(opc, t, &[a, b])
    }

    pub fn unary(&mut self, opc: u16, a: Id) -> Id {
        let t = self.type_of(a);
        self.emit_ids(opc, t, &[a])
    }

    pub fn fadd(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::FAdd, a, b)
    }
    pub fn fsub(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::FSub, a, b)
    }
    pub fn fmul(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::FMul, a, b)
    }
    pub fn fdiv(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::FDiv, a, b)
    }
    pub fn fmod(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::FMod, a, b)
    }
    pub fn fneg(&mut self, a: Id) -> Id {
        self.unary(op::FNegate, a)
    }
    pub fn iadd(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::IAdd, a, b)
    }
    pub fn isub(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::ISub, a, b)
    }
    pub fn imul(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::IMul, a, b)
    }
    pub fn udiv(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::UDiv, a, b)
    }
    pub fn sdiv(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::SDiv, a, b)
    }
    pub fn umod(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::UMod, a, b)
    }
    pub fn srem(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::SRem, a, b)
    }
    pub fn ineg(&mut self, a: Id) -> Id {
        self.unary(op::SNegate, a)
    }
    pub fn band(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::BitwiseAnd, a, b)
    }
    pub fn bor(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::BitwiseOr, a, b)
    }
    pub fn bxor(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::BitwiseXor, a, b)
    }
    pub fn bnot(&mut self, a: Id) -> Id {
        self.unary(op::Not, a)
    }
    pub fn shl(&mut self, a: Id, n: Id) -> Id {
        self.binary(op::ShiftLeftLogical, a, n)
    }
    /// logical shift right
    pub fn shr(&mut self, a: Id, n: Id) -> Id {
        self.binary(op::ShiftRightLogical, a, n)
    }
    /// arithmetic shift right
    pub fn sar(&mut self, a: Id, n: Id) -> Id {
        self.binary(op::ShiftRightArithmetic, a, n)
    }

    /// vector * scalar
    pub fn vec_scale(&mut self, v: Id, s: Id) -> Id {
        self.binary(op::VectorTimesScalar, v, s)
    }
    /// matrix * scalar
    pub fn mat_scale(&mut self, m: Id, s: Id) -> Id {
        self.binary(op::MatrixTimesScalar, m, s)
    }

    /// matrix * column vector
    pub fn mat_vec(&mut self, m: Id, v: Id) -> Id {
        let mt = self.type_of(m);
        let Ty::Matrix { col, .. } = *self.ty(mt) else { panic!("shader: mat_vec of a non-matrix") };
        self.emit_ids(op::MatrixTimesVector, col, &[m, v])
    }

    /// row vector * matrix
    pub fn vec_mat(&mut self, v: Id, m: Id) -> Id {
        let mt = self.type_of(m);
        let Ty::Matrix { cols, .. } = *self.ty(mt) else { panic!("shader: vec_mat of a non-matrix") };
        let e = self.scalar_of(self.type_of(v));
        let t = self.ty_vec(e, cols);
        self.emit_ids(op::VectorTimesMatrix, t, &[v, m])
    }

    pub fn mat_mat(&mut self, a: Id, b: Id) -> Id {
        let (at, bt) = (self.type_of(a), self.type_of(b));
        let (Ty::Matrix { col, .. }, Ty::Matrix { cols, .. }) = (self.ty(at).clone(), self.ty(bt).clone()) else {
            panic!("shader: mat_mat of a non-matrix")
        };
        let t = self.ty_mat(col, cols);
        self.emit_ids(op::MatrixTimesMatrix, t, &[a, b])
    }

    pub fn transpose(&mut self, m: Id) -> Id {
        let mt = self.type_of(m);
        let Ty::Matrix { col, cols } = *self.ty(mt) else { panic!("shader: transpose of a non-matrix") };
        let Ty::Vector { elem, n } = *self.ty(col) else { panic!("shader: matrix column") };
        let c = self.ty_vec(elem, cols);
        let t = self.ty_mat(c, n);
        self.emit_ids(op::Transpose, t, &[m])
    }

    pub fn dot(&mut self, a: Id, b: Id) -> Id {
        let t = self.scalar_of(self.type_of(a));
        self.emit_ids(op::Dot, t, &[a, b])
    }

    // ---- comparisons and logic (bool results)

    /// a comparison: the result is bool or a bool vector of the operands' length
    pub fn cmp(&mut self, opc: u16, a: Id, b: Id) -> Id {
        let t = self.type_of(a);
        let bt = self.bool_like(t);
        self.emit_ids(opc, bt, &[a, b])
    }

    pub fn feq(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::FOrdEqual, a, b)
    }
    pub fn fne(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::FUnordNotEqual, a, b)
    }
    pub fn flt(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::FOrdLessThan, a, b)
    }
    pub fn fle(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::FOrdLessThanEqual, a, b)
    }
    pub fn fgt(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::FOrdGreaterThan, a, b)
    }
    pub fn fge(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::FOrdGreaterThanEqual, a, b)
    }
    pub fn ieq(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::IEqual, a, b)
    }
    pub fn ine(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::INotEqual, a, b)
    }
    pub fn ult(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::ULessThan, a, b)
    }
    pub fn ule(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::ULessThanEqual, a, b)
    }
    pub fn uge(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::UGreaterThanEqual, a, b)
    }
    pub fn slt(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::SLessThan, a, b)
    }
    pub fn sge(&mut self, a: Id, b: Id) -> Id {
        self.cmp(op::SGreaterThanEqual, a, b)
    }
    pub fn land(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::LogicalAnd, a, b)
    }
    pub fn lor(&mut self, a: Id, b: Id) -> Id {
        self.binary(op::LogicalOr, a, b)
    }
    pub fn lnot(&mut self, a: Id) -> Id {
        self.unary(op::LogicalNot, a)
    }
    pub fn any(&mut self, v: Id) -> Id {
        let b = self.ty_bool();
        self.emit_ids(op::Any, b, &[v])
    }
    pub fn all(&mut self, v: Id) -> Id {
        let b = self.ty_bool();
        self.emit_ids(op::All, b, &[v])
    }
    /// c ? a : b
    pub fn select(&mut self, c: Id, a: Id, b: Id) -> Id {
        let t = self.type_of(a);
        self.emit_ids(op::Select, t, &[c, a, b])
    }

    // ---- conversions

    /// a conversion opcode (ConvertFToU, ConvertSToF, Bitcast, FConvert ...) to type ty
    pub fn convert(&mut self, opc: u16, ty: Id, a: Id) -> Id {
        self.emit_ids(opc, ty, &[a])
    }
    pub fn u2f(&mut self, ty: Id, a: Id) -> Id {
        self.convert(op::ConvertUToF, ty, a)
    }
    pub fn s2f(&mut self, ty: Id, a: Id) -> Id {
        self.convert(op::ConvertSToF, ty, a)
    }
    pub fn f2u(&mut self, ty: Id, a: Id) -> Id {
        self.convert(op::ConvertFToU, ty, a)
    }
    pub fn f2s(&mut self, ty: Id, a: Id) -> Id {
        self.convert(op::ConvertFToS, ty, a)
    }
    pub fn bitcast(&mut self, ty: Id, a: Id) -> Id {
        self.convert(op::Bitcast, ty, a)
    }

    // ---- memory

    /// OpAccessChain: the result points at the member / element the indices select
    /// (struct indices must be constants)
    pub fn access(&mut self, base: Id, idx: &[Id]) -> Id {
        let pt = self.type_of(base);
        let Ty::Pointer { sc, pointee } = *self.ty(pt) else { panic!("shader: access on a non-pointer") };
        let mut t = pointee;
        for &i in idx {
            t = match self.ty(t).clone() {
                Ty::Struct { members } => {
                    let k = self.consts.get(&i.0).copied().expect("shader: struct index must be a constant");
                    members[k as usize]
                }
                Ty::Array { elem, .. } | Ty::RuntimeArray { elem } | Ty::Vector { elem, .. } => elem,
                Ty::Matrix { col, .. } => col,
                _ => panic!("shader: access into a scalar"),
            };
        }
        let sc = match sc {
            0 => Sc::UniformConstant,
            1 => Sc::Input,
            2 => Sc::Uniform,
            3 => Sc::Output,
            4 => Sc::Workgroup,
            6 => Sc::Private,
            7 => Sc::Function,
            9 => Sc::PushConstant,
            12 => Sc::StorageBuffer,
            _ => panic!("shader: storage class {}", sc),
        };
        let rt = self.ty_ptr(sc, t);
        let mut a = alloc::vec![base];
        a.extend_from_slice(idx);
        self.emit_ids(op::AccessChain, rt, &a)
    }

    /// access with constant u32 indices
    pub fn access_c(&mut self, base: Id, idx: &[u32]) -> Id {
        let ids: Vec<Id> = idx.iter().map(|&i| self.const_u32(i)).collect();
        self.access(base, &ids)
    }

    pub fn load(&mut self, ptr: Id) -> Id {
        let pt = self.type_of(ptr);
        let Ty::Pointer { pointee, .. } = *self.ty(pt) else { panic!("shader: load from a non-pointer") };
        self.emit_ids(op::Load, pointee, &[ptr])
    }

    pub fn store(&mut self, ptr: Id, v: Id) {
        self.emit(op::Store, None, &[ptr.0, v.0]);
    }

    // ---- composites

    pub fn construct(&mut self, ty: Id, parts: &[Id]) -> Id {
        self.emit_ids(op::CompositeConstruct, ty, parts)
    }

    fn member_type(&self, mut t: Id, idx: &[u32]) -> Id {
        for &i in idx {
            t = match self.ty(t) {
                Ty::Struct { members } => members[i as usize],
                Ty::Array { elem, .. } | Ty::Vector { elem, .. } => *elem,
                Ty::Matrix { col, .. } => *col,
                _ => panic!("shader: extract from a scalar"),
            };
        }
        t
    }

    pub fn extract(&mut self, v: Id, idx: &[u32]) -> Id {
        let t = self.member_type(self.type_of(v), idx);
        let mut a = alloc::vec![v.0];
        a.extend_from_slice(idx);
        self.emit(op::CompositeExtract, Some(t), &a)
    }

    pub fn insert(&mut self, v: Id, obj: Id, idx: &[u32]) -> Id {
        let t = self.type_of(v);
        let mut a = alloc::vec![obj.0, v.0];
        a.extend_from_slice(idx);
        self.emit(op::CompositeInsert, Some(t), &a)
    }

    /// OpVectorShuffle: components of a (0..) and b (len(a)..)
    pub fn shuffle(&mut self, a: Id, b: Id, comps: &[u32]) -> Id {
        let e = self.scalar_of(self.type_of(a));
        let t = self.ty_vec(e, comps.len() as u32);
        let mut w = alloc::vec![a.0, b.0];
        w.extend_from_slice(comps);
        self.emit(op::VectorShuffle, Some(t), &w)
    }

    pub fn extract_dynamic(&mut self, v: Id, i: Id) -> Id {
        let t = self.scalar_of(self.type_of(v));
        self.emit_ids(op::VectorExtractDynamic, t, &[v, i])
    }

    // ---- GLSL.std.450

    /// an extended instruction of GLSL.std.450 (spv::glsl::*) with result type ty
    pub fn ext(&mut self, ty: Id, inst: u32, args: &[Id]) -> Id {
        let set = self.glsl_set();
        let mut a = alloc::vec![set.0, inst];
        a.extend(args.iter().map(|x| x.0));
        self.emit(op::ExtInst, Some(ty), &a)
    }

    /// a GLSL.std.450 instruction whose result type is its first argument's (Sin, Floor,
    /// FMin, FClamp, FMix, Normalize, Cross ...)
    pub fn glsl(&mut self, inst: u32, args: &[Id]) -> Id {
        let t = self.type_of(args[0]);
        self.ext(t, inst, args)
    }

    /// GLSL.std.450 Length (the result is the component type)
    pub fn length(&mut self, v: Id) -> Id {
        let t = self.scalar_of(self.type_of(v));
        self.ext(t, spv::glsl::Length, &[v])
    }

    // ---- atomics and barriers

    /// an atomic read-modify-write (AtomicIAdd, AtomicUMax, AtomicExchange ...) on a 32- or
    /// 64-bit integer: scope and semantics become constants
    pub fn atomic(&mut self, opc: u16, ptr: Id, scope: u32, semantics: u32, value: Id) -> Id {
        let t = self.type_of(value);
        if let Ty::Int { width: 64, .. } = self.ty(t) {
            self.capability(Cap::Int64Atomics);
        }
        let (s, m) = (self.const_u32(scope), self.const_u32(semantics));
        self.emit_ids(opc, t, &[ptr, s, m, value])
    }

    pub fn atomic_load(&mut self, ptr: Id, scope: u32, semantics: u32) -> Id {
        let pt = self.type_of(ptr);
        let Ty::Pointer { pointee, .. } = *self.ty(pt) else { panic!("shader: atomic on a non-pointer") };
        let (s, m) = (self.const_u32(scope), self.const_u32(semantics));
        self.emit_ids(op::AtomicLoad, pointee, &[ptr, s, m])
    }

    pub fn atomic_store(&mut self, ptr: Id, scope: u32, semantics: u32, value: Id) {
        let (s, m) = (self.const_u32(scope), self.const_u32(semantics));
        self.emit(op::AtomicStore, None, &[ptr.0, s.0, m.0, value.0]);
    }

    /// OpAtomicCompareExchange -> the original value
    pub fn atomic_cas(&mut self, ptr: Id, scope: u32, eq: u32, neq: u32, value: Id, comparator: Id) -> Id {
        let t = self.type_of(value);
        let (s, e, n) = (self.const_u32(scope), self.const_u32(eq), self.const_u32(neq));
        self.emit_ids(op::AtomicCompareExchange, t, &[ptr, s, e, n, value, comparator])
    }

    /// OpControlBarrier (barrier() of a workgroup: Workgroup, Workgroup,
    /// AcquireRelease | WorkgroupMemory)
    pub fn control_barrier(&mut self, exec: u32, mem: u32, semantics: u32) {
        let (e, m, s) = (self.const_u32(exec), self.const_u32(mem), self.const_u32(semantics));
        self.emit(op::ControlBarrier, None, &[e.0, m.0, s.0]);
    }

    pub fn memory_barrier(&mut self, mem: u32, semantics: u32) {
        let (m, s) = (self.const_u32(mem), self.const_u32(semantics));
        self.emit(op::MemoryBarrier, None, &[m.0, s.0]);
    }

    // ---- images

    fn image_of(&self, v: Id) -> Ty {
        let mut t = self.type_of(v);
        if let Ty::SampledImage { image } = *self.ty(t) {
            t = image;
        }
        self.ty(t).clone()
    }

    fn texel_type(&mut self, v: Id) -> Id {
        let Ty::Image { sampled, .. } = self.image_of(v) else { panic!("shader: not an image") };
        self.ty_vec(sampled, 4)
    }

    pub fn sampled_image(&mut self, image: Id, sampler: Id) -> Id {
        let it = self.type_of(image);
        let t = self.ty_sampled_image(it);
        self.emit_ids(op::SampledImage, t, &[image, sampler])
    }

    /// texture(si, coord): implicit LOD (fragment shaders)
    pub fn sample(&mut self, si: Id, coord: Id) -> Id {
        let t = self.texel_type(si);
        self.emit_ids(op::ImageSampleImplicitLod, t, &[si, coord])
    }

    /// textureLod(si, coord, lod)
    pub fn sample_lod(&mut self, si: Id, coord: Id, lod: Id) -> Id {
        let t = self.texel_type(si);
        self.emit(op::ImageSampleExplicitLod, Some(t), &[si.0, coord.0, spv::img::Lod, lod.0])
    }

    /// texelFetch(image, coord, lod) on an OpImage / sampled-usage image
    pub fn fetch(&mut self, image: Id, coord: Id, lod: Id) -> Id {
        let t = self.texel_type(image);
        self.emit(op::ImageFetch, Some(t), &[image.0, coord.0, spv::img::Lod, lod.0])
    }

    /// imageLoad of a storage image
    pub fn image_read(&mut self, image: Id, coord: Id) -> Id {
        let Ty::Image { format, .. } = self.image_of(image) else { panic!("shader: not an image") };
        if format == Fmt::Unknown as u32 {
            self.capability(Cap::StorageImageReadWithoutFormat);
        }
        let t = self.texel_type(image);
        self.emit_ids(op::ImageRead, t, &[image, coord])
    }

    /// imageStore of a storage image
    pub fn image_write(&mut self, image: Id, coord: Id, texel: Id) {
        let Ty::Image { format, .. } = self.image_of(image) else { panic!("shader: not an image") };
        if format == Fmt::Unknown as u32 {
            self.capability(Cap::StorageImageWriteWithoutFormat);
        }
        self.emit(op::ImageWrite, None, &[image.0, coord.0, texel.0]);
    }

    /// OpImageQuerySize (storage images, no LOD) with result type ty
    pub fn image_size(&mut self, ty: Id, image: Id) -> Id {
        self.capability(Cap::ImageQuery);
        self.emit_ids(op::ImageQuerySize, ty, &[image])
    }

    /// OpImageQuerySizeLod (sampled images)
    pub fn image_size_lod(&mut self, ty: Id, image: Id, lod: Id) -> Id {
        self.capability(Cap::ImageQuery);
        self.emit_ids(op::ImageQuerySizeLod, ty, &[image, lod])
    }

    /// OpImageQueryLevels (textureNumLevels of a sampled image) with result type ty
    pub fn image_levels(&mut self, ty: Id, image: Id) -> Id {
        self.capability(Cap::ImageQuery);
        self.emit_ids(op::ImageQueryLevels, ty, &[image])
    }

    /// textureSampleCompare: OpImageSampleDrefImplicitLod (fragment shaders) -> the float result
    pub fn sample_dref(&mut self, si: Id, coord: Id, dref: Id) -> Id {
        let Ty::Image { sampled, .. } = self.image_of(si) else { panic!("shader: not an image") };
        self.emit_ids(op::ImageSampleDrefImplicitLod, sampled, &[si, coord, dref])
    }

    /// textureSampleCompareLevel: OpImageSampleDrefExplicitLod with Lod lod
    pub fn sample_dref_lod(&mut self, si: Id, coord: Id, dref: Id, lod: Id) -> Id {
        let Ty::Image { sampled, .. } = self.image_of(si) else { panic!("shader: not an image") };
        self.emit(op::ImageSampleDrefExplicitLod, Some(sampled), &[si.0, coord.0, dref.0, spv::img::Lod, lod.0])
    }

    /// decorate v NonUniform (a resource index or the image / sampler loaded with it) and
    /// declare ShaderNonUniform (before 1.5 with SPV_EXT_descriptor_indexing)
    pub fn nonuniform(&mut self, v: Id) {
        self.capability(Cap::ShaderNonUniform);
        if self.minor < 5 {
            self.extension("SPV_EXT_descriptor_indexing");
        }
        self.decorate(v, Dec::NonUniform, &[]);
    }

    // ---- derivatives (fragment shaders)

    /// DPdx, DPdy, Fwidth and their Fine / Coarse forms (those add DerivativeControl)
    pub fn deriv(&mut self, opc: u16, a: Id) -> Id {
        if (op::DPdxFine..=op::FwidthCoarse).contains(&opc) {
            self.capability(Cap::DerivativeControl);
        }
        self.unary(opc, a)
    }
    pub fn dpdx(&mut self, a: Id) -> Id {
        self.deriv(op::DPdx, a)
    }
    pub fn dpdy(&mut self, a: Id) -> Id {
        self.deriv(op::DPdy, a)
    }
    pub fn fwidth(&mut self, a: Id) -> Id {
        self.deriv(op::Fwidth, a)
    }

    // ---- calls

    /// OpFunctionCall of a function built (or begun with begin_function_id) in this module
    pub fn call(&mut self, func: Id, args: &[Id]) -> Id {
        let t = self.type_of(func);
        let mut a = alloc::vec![func];
        a.extend_from_slice(args);
        self.emit_ids(op::FunctionCall, t, &a)
    }

    // ---- output

    /// the globals each function uses and the functions it calls (from its words)
    fn uses(&self) -> BTreeMap<u32, (BTreeSet<u32>, BTreeSet<u32>)> {
        let mut out: BTreeMap<u32, (BTreeSet<u32>, BTreeSet<u32>)> = BTreeMap::new();
        let mut words = alloc::vec![0u32; 5];
        words.extend_from_slice(&self.funcs);
        let globals: BTreeSet<u32> = {
            let mut g = alloc::vec![0u32; 5];
            g.extend_from_slice(&self.globals);
            parse::parse(&g).unwrap_or_default().iter().filter(|i| i.op == op::Variable).map(|i| i.res).collect()
        };
        let mut cur = 0;
        for inst in parse::parse(&words).unwrap_or_default() {
            if inst.op == op::Function {
                cur = inst.res;
                out.entry(cur).or_default();
                continue;
            }
            let e = out.entry(cur).or_default();
            if inst.op == op::FunctionCall {
                e.1.insert(inst.ops[0]);
            }
            for id in parse::ids(&inst) {
                if globals.contains(&id) {
                    e.0.insert(id);
                }
            }
        }
        out
    }

    /// the interface of an entry point: the globals of its call tree (Input / Output only
    /// before SPIR-V 1.4), in id order
    fn interface(&self, func: u32, uses: &BTreeMap<u32, (BTreeSet<u32>, BTreeSet<u32>)>) -> Vec<u32> {
        let (mut seen, mut todo, mut vars) = (BTreeSet::new(), alloc::vec![func], BTreeSet::new());
        while let Some(f) = todo.pop() {
            if !seen.insert(f) {
                continue;
            }
            if let Some((g, c)) = uses.get(&f) {
                vars.extend(g.iter().copied());
                todo.extend(c.iter().copied());
            }
        }
        vars.into_iter()
            .filter(|&v| {
                let Ty::Pointer { sc, .. } = *self.ty(self.type_of(Id(v))) else { return false };
                self.minor >= 4 || sc == Sc::Input as u32 || sc == Sc::Output as u32
            })
            .collect()
    }

    /// the module's words
    pub fn finish(&self) -> Vec<u32> {
        assert!(self.f.is_none(), "shader: finish inside a function");
        let mut w = alloc::vec![spv::MAGIC, spv::version(1, self.minor), self.generator, self.next, 0];
        for &c in &self.caps {
            enc(&mut w, op::Capability, &[c]);
        }
        for e in &self.exts {
            enc(&mut w, op::Extension, &parse::string_words(e));
        }
        w.extend_from_slice(&self.imports);
        enc(&mut w, op::MemoryModel, &[0, 1]);
        let uses = self.uses();
        for (model, func, name) in &self.entries {
            let mut a = alloc::vec![*model, func.0];
            a.extend(parse::string_words(name));
            a.extend(self.interface(func.0, &uses));
            enc(&mut w, op::EntryPoint, &a);
        }
        w.extend_from_slice(&self.modes);
        w.extend_from_slice(&self.debug);
        w.extend_from_slice(&self.annot);
        w.extend_from_slice(&self.globals);
        w.extend_from_slice(&self.funcs);
        w
    }
}

/// words as little-endian bytes (a .spv file)
pub fn to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// a .spv file's bytes as words (None: not a multiple of 4 bytes or no SPIR-V magic; a
/// big-endian file is swapped)
pub fn from_bytes(b: &[u8]) -> Option<Vec<u32>> {
    if b.len() % 4 != 0 || b.len() < 20 {
        return None;
    }
    let le: Vec<u32> = b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if le[0] == spv::MAGIC {
        Some(le)
    } else if le[0].swap_bytes() == spv::MAGIC {
        Some(le.iter().map(|w| w.swap_bytes()).collect())
    } else {
        None
    }
}

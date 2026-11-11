// msl -- SPIR-V to the Metal Shading Language (AP6 in docs/APPLE.md): the Metal backend of
// lib/gpu translates every module with it, so a module from lib/wgsl or from this crate's
// builder runs on Metal as on Vulkan.  It reads the SPIR-V those two write: logical
// addressing, structured control flow, the opcodes of spv::OPS and GLSL.std.450.
//
//   translate(words, &Options) -> Result<Msl, String>   MSL source text and, per entry point,
//        its function name, workgroup size, resource slots, push constants and buffer sizes
//
// Slots (lib/gpu binds by them): per entry point and kind, the resources it uses get buffer,
// texture and sampler indices from 0 in (set, binding) order (buffers below
// FIRST_VERTEX_SLOT); push constants are a `constant` block at PUSH_SLOT; the byte sizes of
// the bound buffer ranges a `constant uint*` at SIZES_SLOT, indexed by a buffer's slot (only
// for entry points with arrayLength: Entry::sizes); vertex buffer i is at vertex_slot(i).
// DrawIndex (Metal has none) is entry DRAW_INDEX_AT of that buffer, which the backend sets per draw
// of a multi-draw (Entry::draw_index).
//
// Specialization constants are substituted (Options::spec, reflect::constant_values): a
// pipeline with constants is a translation of its own.
//
// The translation: one MSL struct per SPIR-V struct; one with Offset decorations is padded to
// the offsets (3-vectors become packed_ types there), arrays are jf_arr<T, N> (value semantics),
// runtime arrays T m[1].  Every SSA value is a function-scope variable assigned where SPIR-V
// defines it (dominance holds, C scopes need not); pointers, images and samplers stay
// expressions.  Structured control flow becomes if / switch / while (true); a branch to a
// loop's continue target repeats the continue construct there.  Resources, built-ins,
// private and workgroup variables are parameters of every function that uses them.  Signed
// integer arithmetic goes through unsigned (it wraps, as SPIR-V and WGSL define it; MSL would
// leave overflow undefined).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::module::{Id, Ty};
use crate::parse::{self, Inst};
use crate::reflect;
use crate::spv::{op, Bi, Dec, Mode, Model, Sc};

/// the buffer slot of the push constants
pub const PUSH_SLOT: u32 = 29;
/// the buffer slot of the bound buffer sizes (bytes, `constant uint*`, index = buffer slot)
pub const SIZES_SLOT: u32 = 30;
/// resource buffers take the slots below this one, vertex buffers this one and above
pub const FIRST_VERTEX_SLOT: u32 = 21;
/// vertex buffer slots a pipeline may use
pub const VERTEX_BUFFERS_MAX: u32 = 8;
/// the entry of the sizes buffer (SIZES_SLOT) that holds DrawIndex: the draw's index in a multi-draw
pub const DRAW_INDEX_AT: u32 = FIRST_VERTEX_SLOT;

/// the buffer slot of vertex buffer i (i < VERTEX_BUFFERS_MAX)
pub const fn vertex_slot(i: u32) -> u32 {
    FIRST_VERTEX_SLOT + VERTEX_BUFFERS_MAX - 1 - i
}

/// what translate does
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    /// the specialization constants a pipeline sets: (SpecId, value bits) as reflect takes them
    pub spec: &'a [(u32, u64)],
    /// one entry point (None: all)
    pub entry: Option<&'a str>,
    /// vertex entry points write [[point_size]] = 1 (a point-list pipeline: WGSL points are one
    /// pixel, Metal wants the size)
    pub point_size: bool,
}

impl Options<'_> {
    pub const DEFAULT: Options<'static> = Options { spec: &[], entry: None, point_size: false };
}

/// what a slot holds
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Buffer,
    Texture,
    Sampler,
}

/// a resource of an entry point and its index in Metal's argument table
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub set: u32,
    pub binding: u32,
    pub kind: SlotKind,
    pub index: u32,
}

/// an entry point of the translation
#[derive(Clone, Debug)]
pub struct Entry {
    /// the SPIR-V name
    pub name: String,
    /// the MSL function's name
    pub function: String,
    /// spv::Model as u32
    pub model: u32,
    /// compute: the workgroup size with the constants substituted (else 1, 1, 1)
    pub workgroup: [u32; 3],
    /// its resources by (set, binding)
    pub slots: Vec<Slot>,
    /// it reads push constants (PUSH_SLOT)
    pub push_constants: bool,
    /// it reads the buffer sizes (SIZES_SLOT)
    pub sizes: bool,
    /// it reads DrawIndex (entry DRAW_INDEX_AT of the sizes buffer; sizes is set too)
    pub draw_index: bool,
    /// vertex: the locations of its inputs ([[attribute(n)]])
    pub vertex_inputs: Vec<u32>,
}

/// the result of translate
#[derive(Clone, Debug)]
pub struct Msl {
    pub source: String,
    pub entries: Vec<Entry>,
}

pub fn translate(words: &[u32], opt: &Options) -> Result<Msl, String> {
    let m = M::new(words, opt)?;
    m.program(opt)
}

// ---------------------------------------------------------------- the module

pub(crate) struct Var {
    /// the pointee type
    pub ty: u32,
    pub sc: u32,
    pub init: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Merge {
    None,
    Sel(u32),
    /// merge, continue target
    Loop(u32, u32),
}

pub(crate) struct Block {
    pub label: u32,
    /// the instructions without label, merge and terminator: ins[first..end]
    pub first: usize,
    pub end: usize,
    pub merge: Merge,
    pub term: usize,
}

pub(crate) struct Func {
    pub ret: u32,
    pub params: Vec<(u32, u32)>,
    pub blocks: Vec<Block>,
    pub at: BTreeMap<u32, usize>,
}

struct EntryPoint {
    model: u32,
    func: u32,
    name: String,
    modes: Vec<(u32, Vec<u32>)>,
}

/// how a struct is laid out: padding bytes or member i
#[derive(Clone, Copy, Debug)]
pub(crate) enum Field {
    Pad(u32),
    Member(usize),
}

/// decorations: (decoration, its arguments)
pub(crate) type Decos<'a> = Vec<(u32, &'a [u32])>;

pub(crate) struct M<'a> {
    pub ins: Vec<Inst<'a>>,
    pub tys: BTreeMap<u32, Ty>,
    pub decos: BTreeMap<u32, Decos<'a>>,
    pub mdecos: BTreeMap<(u32, u32), Decos<'a>>,
    /// module-level constants: their words (reflect::constant_values) and result types
    pub vals: BTreeMap<u32, Vec<u32>>,
    pub cty: BTreeMap<u32, u32>,
    /// OpUndef ids (module level and in functions)
    pub undef: BTreeSet<u32>,
    pub vars: BTreeMap<u32, Var>,
    pub funcs: BTreeMap<u32, Func>,
    pub glsl: BTreeSet<u32>,
    entries: Vec<EntryPoint>,
    /// every result id's type
    pub rty: BTreeMap<u32, u32>,
    /// the layout of each struct with Offset decorations
    pub plans: BTreeMap<u32, Vec<Field>>,
    /// per function: the globals it and its callees use, whether they read buffer sizes
    pub uses: BTreeMap<u32, BTreeSet<u32>>,
    pub sizes: BTreeSet<u32>,
    /// per image type: the access of the storage-texture variables of that type
    pub img_access: BTreeMap<u32, Access>,
    pub words: &'a [u32],
    /// every resource variable's slot, numbered over the whole module in (set, binding) order
    pub slot_of: BTreeMap<u32, (SlotKind, u32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Access {
    Read,
    Write,
    ReadWrite,
}

fn err<T>(s: impl Into<String>) -> Result<T, String> {
    Err(s.into())
}

impl<'a> M<'a> {
    fn new(words: &'a [u32], opt: &Options) -> Result<M<'a>, String> {
        if words.len() < 5 || words[0] != crate::spv::MAGIC {
            return err("not a SPIR-V module (magic number)");
        }
        let ins = parse::parse(words)?;
        let vals = reflect::constant_values(words, opt.spec)?;
        let mut m = M {
            ins: Vec::new(),
            tys: BTreeMap::new(),
            decos: BTreeMap::new(),
            mdecos: BTreeMap::new(),
            vals,
            cty: BTreeMap::new(),
            undef: BTreeSet::new(),
            vars: BTreeMap::new(),
            funcs: BTreeMap::new(),
            glsl: BTreeSet::new(),
            entries: Vec::new(),
            rty: BTreeMap::new(),
            plans: BTreeMap::new(),
            uses: BTreeMap::new(),
            sizes: BTreeSet::new(),
            img_access: BTreeMap::new(),
            words,
            slot_of: BTreeMap::new(),
        };
        let mut cur: Option<(u32, Func)> = None;
        let mut blk: Option<(u32, usize, Merge)> = None;
        for (k, i) in ins.iter().enumerate() {
            let o = i.ops;
            let g = |n: usize| o.get(n).copied().unwrap_or(0);
            if i.res != 0 && i.ty != 0 {
                m.rty.insert(i.res, i.ty);
            }
            if let Some((_, f)) = cur.as_mut() {
                match i.op {
                    op::FunctionParameter => f.params.push((i.res, i.ty)),
                    op::Label => {
                        if blk.is_some() {
                            return err("a block without terminator");
                        }
                        blk = Some((i.res, k + 1, Merge::None));
                    }
                    op::SelectionMerge | op::LoopMerge => {
                        let Some(b) = blk.as_mut() else { return err("a merge outside a block") };
                        b.2 = if i.op == op::SelectionMerge { Merge::Sel(g(0)) } else { Merge::Loop(g(0), g(1)) };
                    }
                    op::Branch | op::BranchConditional | op::Switch | op::Return | op::ReturnValue | op::Kill
                    | op::Unreachable | op::TerminateInvocation => {
                        let Some((label, first, merge)) = blk.take() else { return err("a terminator outside a block") };
                        let mut end = k;
                        if merge != Merge::None {
                            end -= 1;
                        }
                        f.at.insert(label, f.blocks.len());
                        f.blocks.push(Block { label, first, end, merge, term: k });
                    }
                    op::FunctionEnd => {
                        let (id, f) = cur.take().unwrap();
                        m.funcs.insert(id, f);
                    }
                    op::Undef => {
                        m.undef.insert(i.res);
                    }
                    _ => {}
                }
                continue;
            }
            match i.op {
                op::Function => {
                    cur = Some((i.res, Func { ret: i.ty, params: Vec::new(), blocks: Vec::new(), at: BTreeMap::new() }));
                }
                op::ExtInstImport => {
                    let name = parse::string(o).0;
                    if name != "GLSL.std.450" {
                        return Err(format!("extended instruction set {}", name));
                    }
                    m.glsl.insert(i.res);
                }
                op::EntryPoint => {
                    m.entries.push(EntryPoint { model: g(0), func: g(1), name: parse::string(&o[2..]).0, modes: Vec::new() });
                }
                op::ExecutionMode | op::ExecutionModeId => {
                    let f = g(0);
                    let mode = (g(1), o.get(2..).unwrap_or(&[]).to_vec());
                    for e in m.entries.iter_mut().filter(|e| e.func == f) {
                        e.modes.push(mode.clone());
                    }
                }
                op::Decorate if o.len() >= 2 => m.decos.entry(o[0]).or_default().push((o[1], &o[2..])),
                op::MemberDecorate if o.len() >= 3 => m.mdecos.entry((o[0], o[1])).or_default().push((o[2], &o[3..])),
                op::TypeVoid => {
                    m.tys.insert(i.res, Ty::Void);
                }
                op::TypeBool => {
                    m.tys.insert(i.res, Ty::Bool);
                }
                op::TypeInt => {
                    m.tys.insert(i.res, Ty::Int { width: g(0), signed: g(1) != 0 });
                }
                op::TypeFloat => {
                    m.tys.insert(i.res, Ty::Float { width: g(0) });
                }
                op::TypeVector => {
                    m.tys.insert(i.res, Ty::Vector { elem: Id(g(0)), n: g(1) });
                }
                op::TypeMatrix => {
                    m.tys.insert(i.res, Ty::Matrix { col: Id(g(0)), cols: g(1) });
                }
                op::TypeArray => {
                    m.tys.insert(i.res, Ty::Array { elem: Id(g(0)), len: Id(g(1)) });
                }
                op::TypeRuntimeArray => {
                    m.tys.insert(i.res, Ty::RuntimeArray { elem: Id(g(0)) });
                }
                op::TypeStruct => {
                    m.tys.insert(i.res, Ty::Struct { members: o.iter().map(|&x| Id(x)).collect() });
                }
                op::TypePointer => {
                    m.tys.insert(i.res, Ty::Pointer { sc: g(0), pointee: Id(g(1)) });
                }
                op::TypeFunction => {
                    m.tys.insert(i.res, Ty::Function { ret: Id(g(0)), params: o[1..].iter().map(|&x| Id(x)).collect() });
                }
                op::TypeImage => {
                    m.tys.insert(i.res, Ty::Image { sampled: Id(g(0)), dim: g(1), depth: g(2), arrayed: g(3), ms: g(4), usage: g(5), format: g(6) });
                }
                op::TypeSampler => {
                    m.tys.insert(i.res, Ty::Sampler);
                }
                op::TypeSampledImage => {
                    m.tys.insert(i.res, Ty::SampledImage { image: Id(g(0)) });
                }
                op::ConstantTrue | op::ConstantFalse | op::Constant | op::ConstantComposite | op::ConstantNull
                | op::SpecConstantTrue | op::SpecConstantFalse | op::SpecConstant | op::SpecConstantComposite
                | op::SpecConstantOp => {
                    m.cty.insert(i.res, i.ty);
                }
                op::Undef => {
                    m.undef.insert(i.res);
                }
                op::Variable => {
                    let Some(Ty::Pointer { pointee, .. }) = m.tys.get(&i.ty) else { return err("a variable without pointer type") };
                    m.vars.insert(i.res, Var { ty: pointee.0, sc: g(0), init: o.get(1).copied() });
                }
                _ => {}
            }
        }
        if cur.is_some() {
            return err("a function without OpFunctionEnd");
        }
        m.ins = ins;
        m.number_slots()?;
        m.plan_structs()?;
        m.analyze_uses()?;
        for (&id, v) in &m.vars {
            if v.sc != Sc::UniformConstant as u32 {
                continue;
            }
            if let Some(Ty::Image { usage: 2, .. }) = m.tys.get(&v.ty) {
                let a = if m.has_deco(id, Dec::NonWritable) {
                    Access::Read
                } else if m.has_deco(id, Dec::NonReadable) {
                    Access::Write
                } else {
                    Access::ReadWrite
                };
                match m.img_access.get(&v.ty) {
                    Some(b) if *b != a => {
                        m.img_access.insert(v.ty, Access::ReadWrite);
                    }
                    _ => {
                        m.img_access.insert(v.ty, a);
                    }
                }
            }
        }
        Ok(m)
    }

    /// slots for every resource of the module: buffers, textures, samplers each from 0
    fn number_slots(&mut self) -> Result<(), String> {
        let mut res: Vec<(u32, u32, u32)> = Vec::new();
        for (&g, v) in &self.vars {
            if v.sc == Sc::StorageBuffer as u32 || v.sc == Sc::Uniform as u32 || v.sc == Sc::UniformConstant as u32 {
                let set = self.deco(g, Dec::DescriptorSet).and_then(|x| x.first().copied()).unwrap_or(0);
                let b = self.deco(g, Dec::Binding).and_then(|x| x.first().copied()).ok_or_else(|| format!("resource %{} without Binding", g))?;
                res.push((set, b, g));
            }
        }
        res.sort();
        let (mut nb, mut nt, mut ns) = (0u32, 0u32, 0u32);
        for (_, _, g) in res {
            let v = &self.vars[&g];
            let s = if v.sc == Sc::UniformConstant as u32 {
                match self.tys.get(&v.ty) {
                    Some(Ty::Image { .. }) => {
                        nt += 1;
                        (SlotKind::Texture, nt - 1)
                    }
                    Some(Ty::Sampler) => {
                        ns += 1;
                        (SlotKind::Sampler, ns - 1)
                    }
                    _ => return Err(format!("resource %{} of an unsupported type", g)),
                }
            } else {
                nb += 1;
                (SlotKind::Buffer, nb - 1)
            };
            self.slot_of.insert(g, s);
        }
        if nb > FIRST_VERTEX_SLOT {
            return Err(format!("{} buffers (Metal slots for at most {})", nb, FIRST_VERTEX_SLOT));
        }
        if nt > 128 || ns > 16 {
            return err("more textures (128) or samplers (16) than Metal allows");
        }
        Ok(())
    }

    /// the buffer slot of a buffer variable
    pub fn buffer_slot(&self, g: u32) -> Result<u32, String> {
        match self.slot_of.get(&g) {
            Some((SlotKind::Buffer, i)) => Ok(*i),
            _ => Err(format!("%{} is no buffer resource", g)),
        }
    }

    pub fn deco(&self, id: u32, d: Dec) -> Option<&'a [u32]> {
        self.decos.get(&id)?.iter().find(|x| x.0 == d as u32).map(|x| x.1)
    }
    pub fn has_deco(&self, id: u32, d: Dec) -> bool {
        self.deco(id, d).is_some()
    }
    pub fn mdeco(&self, s: u32, member: u32, d: Dec) -> Option<&'a [u32]> {
        self.mdecos.get(&(s, member))?.iter().find(|x| x.0 == d as u32).map(|x| x.1)
    }
    pub fn ty(&self, t: u32) -> Result<&Ty, String> {
        self.tys.get(&t).ok_or_else(|| format!("%{} is no type", t))
    }
    /// a module-level integer constant's value
    pub fn cu32(&self, id: u32) -> Result<u32, String> {
        self.vals.get(&id).and_then(|w| w.first().copied()).ok_or_else(|| format!("%{} is no constant", id))
    }
    pub fn is_const(&self, id: u32) -> bool {
        self.cty.contains_key(&id)
    }

    /// (scalar kind, components) of a numeric or bool scalar / vector type
    pub fn num(&self, t: u32) -> Result<(Ty, u32), String> {
        match self.ty(t)? {
            Ty::Vector { elem, n } => Ok((self.ty(elem.0)?.clone(), *n)),
            x @ (Ty::Bool | Ty::Int { .. } | Ty::Float { .. }) => Ok((x.clone(), 1)),
            _ => Err(format!("%{} is no scalar or vector type", t)),
        }
    }

    // ------------------------------------------------------------ type names

    pub fn scalar_name(s: &Ty) -> Result<&'static str, String> {
        Ok(match s {
            Ty::Bool => "bool",
            Ty::Int { width: 8, signed: true } => "char",
            Ty::Int { width: 8, signed: false } => "uchar",
            Ty::Int { width: 16, signed: true } => "short",
            Ty::Int { width: 16, signed: false } => "ushort",
            Ty::Int { width: 32, signed: true } => "int",
            Ty::Int { width: 32, signed: false } => "uint",
            Ty::Int { width: 64, signed: true } => "long",
            Ty::Int { width: 64, signed: false } => "ulong",
            Ty::Float { width: 16 } => "half",
            Ty::Float { width: 32 } => "float",
            Ty::Float { width: 64 } => return err("64-bit floats (Metal has none)"),
            _ => return err("not a scalar type"),
        })
    }

    /// the name of a scalar or vector type with n components
    pub fn num_name(s: &Ty, n: u32) -> Result<String, String> {
        let b = Self::scalar_name(s)?;
        Ok(if n == 1 { String::from(b) } else { format!("{}{}", b, n) })
    }

    /// the MSL name of a value type
    pub fn tname(&self, t: u32) -> Result<String, String> {
        match self.ty(t)? {
            Ty::Void => Ok(String::from("void")),
            s @ (Ty::Bool | Ty::Int { .. } | Ty::Float { .. }) => Ok(String::from(Self::scalar_name(s)?)),
            Ty::Vector { elem, n } => Self::num_name(self.ty(elem.0)?, *n),
            Ty::Matrix { col, cols } => {
                let Ty::Vector { elem, n } = self.ty(col.0)? else { return err("a matrix column is no vector") };
                let s = self.ty(elem.0)?;
                if !matches!(s, Ty::Float { width: 16 | 32 }) {
                    return err("a matrix of other than f16 / f32");
                }
                Ok(format!("{}{}x{}", Self::scalar_name(s)?, cols, n))
            }
            Ty::Array { elem, len } => {
                let n = self.cu32(len.0)?;
                if n == 0 {
                    return err("an array of length 0 (an override of 0?)");
                }
                Ok(format!("jf_arr<{}, {}>", self.tname(elem.0)?, n))
            }
            Ty::RuntimeArray { .. } => err("a runtime array outside a buffer"),
            Ty::Struct { .. } => Ok(format!("S{}", t)),
            Ty::Image { .. } => self.image_name(t, self.img_access.get(&t).copied().unwrap_or(Access::ReadWrite)),
            Ty::Sampler => Ok(String::from("sampler")),
            Ty::Pointer { .. } => err("a pointer as a value"),
            Ty::Function { .. } => err("a function type as a value"),
            Ty::SampledImage { .. } => err("a sampled image as a value"),
        }
    }

    pub fn image_name(&self, t: u32, access: Access) -> Result<String, String> {
        let Ty::Image { sampled, dim, depth, arrayed, ms, usage, .. } = self.ty(t)?.clone() else { return err("no image type") };
        let elem = Self::scalar_name(self.ty(sampled.0)?)?;
        let d = depth == 1;
        let base = match (dim, d, ms != 0, arrayed != 0) {
            (0, false, false, false) => "texture1d",
            (0, false, false, true) => "texture1d_array",
            (1, false, false, false) => "texture2d",
            (1, false, false, true) => "texture2d_array",
            (1, false, true, false) => "texture2d_ms",
            (1, false, true, true) => "texture2d_ms_array",
            (1, true, false, false) => "depth2d",
            (1, true, false, true) => "depth2d_array",
            (1, true, true, false) => "depth2d_ms",
            (1, true, true, true) => "depth2d_ms_array",
            (2, false, false, false) => "texture3d",
            (3, false, false, false) => "texturecube",
            (3, false, false, true) => "texturecube_array",
            (3, true, false, false) => "depthcube",
            (3, true, false, true) => "depthcube_array",
            _ => return Err(format!("image of dim {} (depth {}, ms {}, arrayed {})", dim, depth, ms, arrayed)),
        };
        if usage == 2 {
            let a = match access {
                Access::Read => "read",
                Access::Write => "write",
                Access::ReadWrite => "read_write",
            };
            Ok(format!("{}<{}, access::{}>", base, elem, a))
        } else {
            Ok(format!("{}<{}>", base, elem))
        }
    }

    // ------------------------------------------------------------ layouts

    /// bytes of a type in MSL (member: 3-vectors packed)
    fn msl_size(&self, t: u32, member: bool) -> Result<u32, String> {
        Ok(match self.ty(t)? {
            Ty::Bool => 1,
            Ty::Int { width, .. } | Ty::Float { width } => width / 8,
            Ty::Vector { elem, n } => {
                let s = self.msl_size(elem.0, false)?;
                s * if *n == 3 && !member { 4 } else { *n }
            }
            Ty::Matrix { col, cols } => self.msl_size(col.0, false)? * cols,
            Ty::Array { elem, len } => self.msl_size(elem.0, false)? * self.cu32(len.0)?,
            Ty::RuntimeArray { .. } => 0,
            Ty::Struct { members } => {
                if let Some(p) = self.plans.get(&t) {
                    let mut n = 0;
                    for f in p {
                        n += match *f {
                            Field::Pad(b) => b,
                            Field::Member(i) => self.msl_size(members[i].0, true)?,
                        };
                    }
                    n
                } else {
                    let (mut off, mut al) = (0u32, 1u32);
                    for m in members {
                        let a = self.msl_align(m.0, false)?;
                        off = off.div_ceil(a) * a + self.msl_size(m.0, false)?;
                        al = al.max(a);
                    }
                    off.div_ceil(al) * al
                }
            }
            _ => return Err(format!("%{} has no size", t)),
        })
    }

    fn msl_align(&self, t: u32, member: bool) -> Result<u32, String> {
        Ok(match self.ty(t)? {
            Ty::Bool => 1,
            Ty::Int { width, .. } | Ty::Float { width } => width / 8,
            Ty::Vector { elem, n } => {
                let s = self.msl_size(elem.0, false)?;
                if *n == 3 && member { s } else { s * if *n == 3 { 4 } else { *n } }
            }
            Ty::Matrix { col, .. } => self.msl_align(col.0, false)?,
            Ty::Array { elem, .. } | Ty::RuntimeArray { elem } => self.msl_align(elem.0, false)?,
            Ty::Struct { members } => {
                let laid = self.plans.contains_key(&t);
                let mut a = 1;
                for m in members {
                    a = a.max(self.msl_align(m.0, laid)?);
                }
                a
            }
            _ => 1,
        })
    }

    /// size and alignment as SPIR-V's explicit layouts (and WGSL) count them
    fn spv_size_align(&self, t: u32) -> Result<(u32, u32), String> {
        Ok(match self.ty(t)? {
            Ty::Bool => (4, 4),
            Ty::Int { width, .. } | Ty::Float { width } => (width / 8, width / 8),
            Ty::Vector { elem, n } => {
                let s = self.spv_size_align(elem.0)?.0;
                (s * n, s * if *n == 3 { 4 } else { *n })
            }
            Ty::Matrix { col, cols } => {
                let (_, a) = self.spv_size_align(col.0)?;
                (a * cols, a)
            }
            Ty::Array { elem, len } => {
                let (s, a) = self.spv_size_align(elem.0)?;
                let stride = self.deco(t, Dec::ArrayStride).and_then(|x| x.first().copied()).unwrap_or(s.div_ceil(a) * a);
                (stride * self.cu32(len.0)?, a)
            }
            Ty::RuntimeArray { elem } => (0, self.spv_size_align(elem.0)?.1),
            Ty::Struct { members } => {
                let (mut end, mut al) = (0u32, 1u32);
                for (k, m) in members.iter().enumerate() {
                    let (s, a) = self.spv_size_align(m.0)?;
                    let off = self.mdeco(t, k as u32, Dec::Offset).and_then(|x| x.first().copied()).unwrap_or(end.div_ceil(a) * a);
                    end = end.max(off + s);
                    al = al.max(a);
                }
                (end.div_ceil(al) * al, al)
            }
            _ => return Err(format!("%{} has no layout", t)),
        })
    }

    /// the padding of every struct with Offset decorations (in type order: members first)
    fn plan_structs(&mut self) -> Result<(), String> {
        // the largest array stride each struct is an element of
        let mut stride: BTreeMap<u32, u32> = BTreeMap::new();
        for (&id, t) in &self.tys {
            if let (Ty::Array { elem, .. } | Ty::RuntimeArray { elem }, Some(s)) = (t, self.deco(id, Dec::ArrayStride).and_then(|x| x.first().copied())) {
                let e = stride.entry(elem.0).or_insert(0);
                *e = (*e).max(s);
            }
        }
        let ids: Vec<u32> = self.tys.iter().filter(|(_, t)| matches!(t, Ty::Struct { .. })).map(|(&i, _)| i).collect();
        for id in ids {
            let Ty::Struct { members } = self.ty(id)?.clone() else { continue };
            if members.is_empty() || self.mdeco(id, 0, Dec::Offset).is_none() {
                continue;
            }
            let mut plan = Vec::new();
            let mut cur = 0u32;
            for (k, mt) in members.iter().enumerate() {
                let Some(off) = self.mdeco(id, k as u32, Dec::Offset).and_then(|x| x.first().copied()) else {
                    return Err(format!("struct %{}: member {} has no Offset", id, k));
                };
                if off < cur {
                    return Err(format!("struct %{}: member {} at {} overlaps the MSL layout (at {})", id, k, off, cur));
                }
                let a = self.msl_align(mt.0, true)?;
                if off % a != 0 {
                    return Err(format!("struct %{}: member {} at {} is not aligned to {} in MSL", id, k, off, a));
                }
                if off > cur {
                    plan.push(Field::Pad(off - cur));
                }
                self.check_strides(id, k as u32, mt.0)?;
                plan.push(Field::Member(k));
                cur = off + self.msl_size(mt.0, true)?;
            }
            let want = self.spv_size_align(id)?.0.max(stride.get(&id).copied().unwrap_or(0));
            if want > cur && !matches!(self.ty(members.last().unwrap().0)?, Ty::RuntimeArray { .. }) {
                plan.push(Field::Pad(want - cur));
            }
            self.plans.insert(id, plan);
        }
        Ok(())
    }

    /// arrays and matrices in a laid-out struct: their strides must be MSL's
    fn check_strides(&self, s: u32, k: u32, t: u32) -> Result<(), String> {
        match self.ty(t)? {
            Ty::Array { elem, .. } | Ty::RuntimeArray { elem } => {
                if let Some(st) = self.deco(t, Dec::ArrayStride).and_then(|x| x.first().copied()) {
                    let sz = self.msl_size(elem.0, false)?;
                    if st != sz {
                        return Err(format!("struct %{} member {}: array stride {} for an element of {} bytes in MSL", s, k, st, sz));
                    }
                }
                if let Ty::Matrix { .. } = self.ty(elem.0)? {
                    self.check_matrix(s, k, elem.0)?;
                }
                Ok(())
            }
            Ty::Matrix { .. } => self.check_matrix(s, k, t),
            _ => Ok(()),
        }
    }

    fn check_matrix(&self, s: u32, k: u32, t: u32) -> Result<(), String> {
        let Ty::Matrix { col, .. } = self.ty(t)? else { return Ok(()) };
        if self.mdeco(s, k, Dec::RowMajor).is_some() {
            return err("row-major matrices");
        }
        if let Some(ms) = self.mdeco(s, k, Dec::MatrixStride).and_then(|x| x.first().copied()) {
            let c = self.msl_size(col.0, false)?;
            if ms != c {
                return Err(format!("struct %{} member {}: matrix stride {} for columns of {} bytes in MSL", s, k, ms, c));
            }
        }
        Ok(())
    }

    /// the struct declaration
    fn struct_decl(&self, id: u32, out: &mut String) -> Result<(), String> {
        let Ty::Struct { members } = self.ty(id)? else { return Ok(()) };
        out.push_str(&format!("struct S{} {{\n", id));
        let fields: Vec<Field> = match self.plans.get(&id) {
            Some(p) => p.clone(),
            None => (0..members.len()).map(Field::Member).collect(),
        };
        let laid = self.plans.contains_key(&id);
        for (fi, f) in fields.into_iter().enumerate() {
            match f {
                Field::Pad(n) => out.push_str(&format!("    char _p{}[{}];\n", fi, n)),
                Field::Member(k) => {
                    let mt = members[k].0;
                    match self.ty(mt)? {
                        Ty::RuntimeArray { elem } => out.push_str(&format!("    {} m{}[1];\n", self.tname(elem.0)?, k)),
                        Ty::Vector { elem, n: 3 } if laid => {
                            out.push_str(&format!("    packed_{}3 m{};\n", Self::scalar_name(self.ty(elem.0)?)?, k))
                        }
                        _ => out.push_str(&format!("    {} m{};\n", self.tname(mt)?, k)),
                    }
                }
            }
        }
        out.push_str("};\n");
        Ok(())
    }

    // ------------------------------------------------------------ constants

    /// an MSL literal of type t from value words (reflect's encoding)
    pub fn lit(&self, t: u32, w: &[u32], pos: &mut usize) -> Result<String, String> {
        let mut next = || -> Result<u32, String> {
            let v = w.get(*pos).copied().ok_or("constant words too short")?;
            *pos += 1;
            Ok(v)
        };
        Ok(match self.ty(t)?.clone() {
            Ty::Bool => String::from(if next()? != 0 { "true" } else { "false" }),
            Ty::Int { width: 64, signed } => {
                let lo = next()? as u64;
                let hi = next()? as u64;
                let v = lo | (hi << 32);
                if signed {
                    let i = v as i64;
                    if i == i64::MIN {
                        String::from("(-9223372036854775807l - 1l)")
                    } else if i < 0 {
                        format!("({}l)", i)
                    } else {
                        format!("{}l", i)
                    }
                } else {
                    format!("{}ul", v)
                }
            }
            Ty::Int { width, signed } => {
                let v = next()?;
                match (width, signed) {
                    (32, true) => {
                        let i = v as i32;
                        if i == i32::MIN {
                            String::from("(-2147483647 - 1)")
                        } else if i < 0 {
                            format!("({})", i)
                        } else {
                            format!("{}", i)
                        }
                    }
                    (32, false) => format!("{}u", v),
                    (16, true) => format!("short({})", v as u16 as i16),
                    (16, false) => format!("ushort({})", v as u16),
                    (8, true) => format!("char({})", v as u8 as i8),
                    (8, false) => format!("uchar({})", v as u8),
                    _ => return Err(format!("{}-bit integers", width)),
                }
            }
            Ty::Float { width: 32 } => f32_lit(f32::from_bits(next()?)),
            Ty::Float { width: 16 } => {
                let h = next()? as u16;
                let f = reflect::f16_to_f32(h);
                if f.is_finite() { format!("half({})", f32_lit(f)) } else { format!("as_type<half>(ushort({:#x}))", h) }
            }
            Ty::Float { .. } => return err("64-bit floats (Metal has none)"),
            Ty::Vector { elem, n } => {
                let mut parts = Vec::new();
                for _ in 0..n {
                    parts.push(self.lit(elem.0, w, pos)?);
                }
                format!("{}({})", self.tname(t)?, parts.join(", "))
            }
            Ty::Matrix { col, cols } => {
                let mut parts = Vec::new();
                for _ in 0..cols {
                    parts.push(self.lit(col.0, w, pos)?);
                }
                format!("{}({})", self.tname(t)?, parts.join(", "))
            }
            Ty::Array { elem, len } => {
                let n = self.cu32(len.0)?;
                let mut parts = Vec::new();
                for _ in 0..n {
                    parts.push(self.lit(elem.0, w, pos)?);
                }
                format!("{}{{{{{}}}}}", self.tname(t)?, parts.join(", "))
            }
            Ty::Struct { members } => {
                let laid = self.plans.contains_key(&t);
                let mut vals = Vec::new();
                for m in &members {
                    match self.ty(m.0)?.clone() {
                        // a packed_ member: a brace list (a constructor would be a global one)
                        Ty::Vector { elem, n: 3 } if laid => {
                            let mut parts = Vec::new();
                            for _ in 0..3 {
                                parts.push(self.lit(elem.0, w, pos)?);
                            }
                            vals.push(format!("{{{}}}", parts.join(", ")));
                        }
                        _ => vals.push(self.lit(m.0, w, pos)?),
                    }
                }
                let fields: Vec<Field> = match self.plans.get(&t) {
                    Some(p) => p.clone(),
                    None => (0..members.len()).map(Field::Member).collect(),
                };
                let parts: Vec<String> = fields.iter().map(|f| match f {
                    Field::Pad(_) => String::from("{}"),
                    Field::Member(k) => vals[*k].clone(),
                }).collect();
                format!("S{}{{{}}}", t, parts.join(", "))
            }
            _ => return Err(format!("a constant of type %{}", t)),
        })
    }

    /// a constant's expression: composites as their program-scope name c<id>
    pub fn cexpr(&self, id: u32) -> Result<String, String> {
        let t = *self.cty.get(&id).or_else(|| self.rty.get(&id)).ok_or_else(|| format!("%{} has no type", id))?;
        if self.undef.contains(&id) {
            return Ok(format!("{}{{}}", self.tname(t)?));
        }
        match self.ty(t)? {
            Ty::Struct { .. } | Ty::Array { .. } if !self.has_matrix(t) => Ok(format!("c{}", id)),
            _ => {
                let w = self.vals.get(&id).ok_or_else(|| format!("%{} has no constant value", id))?;
                self.lit(t, w, &mut 0)
            }
        }
    }

    // ------------------------------------------------------------ uses of globals

    fn analyze_uses(&mut self) -> Result<(), String> {
        let mut calls: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for (&fid, f) in &self.funcs {
            let mut u = BTreeSet::new();
            let mut c = BTreeSet::new();
            for b in &f.blocks {
                for k in b.first..=b.term {
                    let i = &self.ins[k];
                    if i.op == op::FunctionCall {
                        c.insert(i.ops[0]);
                    }
                    if i.op == op::ArrayLength {
                        self.sizes.insert(fid);
                    }
                    for x in parse::ids(i) {
                        if self.vars.contains_key(&x) {
                            u.insert(x);
                        }
                    }
                }
            }
            self.uses.insert(fid, u);
            calls.insert(fid, c);
        }
        // the callees' globals, to a fixpoint (no recursion in shaders: a few rounds)
        for _ in 0..64 {
            let mut changed = false;
            for (f, cs) in &calls {
                for c in cs {
                    let add: Vec<u32> = self.uses.get(c).map(|s| s.iter().copied().collect()).unwrap_or_default();
                    let s = self.uses.get_mut(f).unwrap();
                    for a in add {
                        changed |= s.insert(a);
                    }
                    if self.sizes.contains(c) && !self.sizes.contains(f) {
                        self.sizes.insert(*f);
                        changed = true;
                    }
                }
            }
            if !changed {
                return Ok(());
            }
        }
        err("recursive function calls")
    }

    /// a global as a parameter of a function other than the entry point
    pub fn global_param(&self, g: u32) -> Result<String, String> {
        let v = &self.vars[&g];
        let t = v.ty;
        Ok(match v.sc {
            x if x == Sc::UniformConstant as u32 => match self.ty(t)? {
                Ty::Image { .. } => format!("{} g{}", self.image_name(t, self.var_access(g))?, g),
                _ => format!("{} g{}", self.tname(t)?, g),
            },
            x if x == Sc::StorageBuffer as u32 => format!("device {}& g{}", self.tname(t)?, g),
            x if x == Sc::Uniform as u32 || x == Sc::PushConstant as u32 => format!("constant {}& g{}", self.tname(t)?, g),
            x if x == Sc::Workgroup as u32 => format!("threadgroup {}& g{}", self.tname(t)?, g),
            _ => format!("thread {}& g{}", self.tname(t)?, g),
        })
    }

    pub fn var_access(&self, g: u32) -> Access {
        if self.has_deco(g, Dec::NonWritable) {
            Access::Read
        } else if self.has_deco(g, Dec::NonReadable) {
            Access::Write
        } else {
            Access::ReadWrite
        }
    }

    /// the address space of a storage class for pointers into it
    pub fn space(sc: u32) -> &'static str {
        match sc {
            x if x == Sc::StorageBuffer as u32 => "device",
            x if x == Sc::Uniform as u32 || x == Sc::PushConstant as u32 => "constant",
            x if x == Sc::Workgroup as u32 => "threadgroup",
            _ => "thread",
        }
    }

    // ------------------------------------------------------------ the program

    fn program(&self, opt: &Options) -> Result<Msl, String> {
        let mut out = String::from(PRELUDE);
        for (&id, t) in &self.tys {
            if let Ty::Struct { .. } = t {
                self.struct_decl(id, &mut out)?;
            }
        }
        // composite constants at program scope
        for (&id, &t) in &self.cty {
            if self.undef.contains(&id) {
                continue;
            }
            if let Ty::Struct { .. } | Ty::Array { .. } = self.ty(t)? {
                let Some(w) = self.vals.get(&id) else { continue };
                if self.has_runtime_array(t) || self.has_matrix(t) {
                    continue; // (matrices: their constructors would be global ones; inline instead)
                }
                out.push_str(&format!("constant {} c{} = {};\n", self.tname(t)?, id, self.lit(t, w, &mut 0)?));
            }
        }
        let entry_funcs: BTreeSet<u32> = self.entries.iter().map(|e| e.func).collect();
        let chosen: Vec<&EntryPoint> = self.entries.iter().filter(|e| opt.entry.is_none_or(|n| n == e.name)).collect();
        if chosen.is_empty() {
            return err(match opt.entry {
                Some(_) => "no entry point of this name",
                None => "no entry point",
            });
        }
        // the functions the chosen entry points call
        let mut need: BTreeSet<u32> = BTreeSet::new();
        let mut todo: Vec<u32> = chosen.iter().map(|e| e.func).collect();
        while let Some(f) = todo.pop() {
            let Some(func) = self.funcs.get(&f) else { return Err(format!("entry point function %{} missing", f)) };
            for b in &func.blocks {
                for k in b.first..b.end {
                    let i = &self.ins[k];
                    if i.op == op::FunctionCall && need.insert(i.ops[0]) {
                        todo.push(i.ops[0]);
                    }
                }
            }
        }
        let mut conv = BTreeSet::new();
        let mut protos = String::new();
        let mut bodies = String::new();
        let _ = &entry_funcs;
        for &f in &need {
            let (proto, body) = crate::msl_fn::function(self, f, &mut conv)?;
            protos.push_str(&proto);
            protos.push_str(";\n");
            bodies.push_str(&proto);
            bodies.push('\n');
            bodies.push_str(&body);
        }
        let mut entries = Vec::new();
        let mut ebodies = String::new();
        for (k, e) in chosen.iter().enumerate() {
            let (src, info) = self.entry(e, k, opt, &mut conv)?;
            ebodies.push_str(&src);
            entries.push(info);
        }
        for &(a, b) in &conv {
            out.push_str(&format!("static inline {} jf_conv_{}_{}({} x);\n", self.tname(b)?, a, b, self.tname(a)?));
        }
        for &(a, b) in &conv {
            out.push_str(&self.converter(a, b)?);
        }
        out.push_str(&protos);
        out.push_str(&bodies);
        out.push_str(&ebodies);
        Ok(Msl { source: out, entries })
    }

    fn has_matrix(&self, t: u32) -> bool {
        match self.tys.get(&t) {
            Some(Ty::Matrix { .. }) => true,
            Some(Ty::Struct { members }) => members.iter().any(|m| self.has_matrix(m.0)),
            Some(Ty::Array { elem, .. }) => self.has_matrix(elem.0),
            _ => false,
        }
    }

    fn has_runtime_array(&self, t: u32) -> bool {
        match self.tys.get(&t) {
            Some(Ty::RuntimeArray { .. }) => true,
            Some(Ty::Struct { members }) => members.iter().any(|m| self.has_runtime_array(m.0)),
            Some(Ty::Array { elem, .. }) => self.has_runtime_array(elem.0),
            _ => false,
        }
    }

    /// a function converting composite type a into b of the same shape (OpCopyLogical)
    fn converter(&self, a: u32, b: u32) -> Result<String, String> {
        let mut s = format!("static inline {} jf_conv_{}_{}({} x) {{\n    {} r;\n", self.tname(b)?, a, b, self.tname(a)?, self.tname(b)?);
        match (self.ty(a)?, self.ty(b)?) {
            (Ty::Struct { members: ma }, Ty::Struct { members: mb }) => {
                for (k, (x, y)) in ma.iter().zip(mb.iter()).enumerate() {
                    s.push_str(&format!("    r.m{} = {};\n", k, self.conv_expr(x.0, y.0, &format!("x.m{}", k))?));
                }
            }
            (Ty::Array { elem: ea, len }, Ty::Array { elem: eb, .. }) => {
                let n = self.cu32(len.0)?;
                s.push_str(&format!("    for (uint i = 0; i < {}u; i++) r[i] = {};\n", n, self.conv_expr(ea.0, eb.0, "x[i]")?));
            }
            _ => return err("OpCopyLogical between types of different shape"),
        }
        s.push_str("    return r;\n}\n");
        Ok(s)
    }

    /// the expression converting e of type a to type b (the same, or a converter call)
    pub fn conv_expr(&self, a: u32, b: u32, e: &str) -> Result<String, String> {
        if a == b || self.tname(a)? == self.tname(b)? {
            return Ok(String::from(e));
        }
        match self.ty(a)? {
            Ty::Struct { .. } | Ty::Array { .. } => Ok(format!("jf_conv_{}_{}({})", a, b, e)),
            _ => Ok(String::from(e)),
        }
    }

    /// every converter a pair needs, nested ones first
    pub fn need_conv(&self, a: u32, b: u32, set: &mut BTreeSet<(u32, u32)>) -> Result<(), String> {
        if a == b || self.tname(a)? == self.tname(b)? {
            return Ok(());
        }
        match (self.ty(a)?.clone(), self.ty(b)?.clone()) {
            (Ty::Struct { members: ma }, Ty::Struct { members: mb }) => {
                if ma.len() != mb.len() {
                    return err("OpCopyLogical between structs of different member counts");
                }
                for (x, y) in ma.iter().zip(mb.iter()) {
                    self.need_conv(x.0, y.0, set)?;
                }
            }
            (Ty::Array { elem: ea, .. }, Ty::Array { elem: eb, .. }) => self.need_conv(ea.0, eb.0, set)?,
            _ => return Ok(()),
        }
        set.insert((a, b));
        Ok(())
    }

    // ------------------------------------------------------------ entry points

    fn entry(&self, e: &EntryPoint, k: usize, opt: &Options, conv: &mut BTreeSet<(u32, u32)>) -> Result<(String, Entry), String> {
        let fname = format!("e{}_{}", k, sanitize(&e.name));
        let model = e.model;
        let (vertex, fragment, compute) = (model == Model::Vertex as u32, model == Model::Fragment as u32, model == Model::GLCompute as u32);
        if !(vertex || fragment || compute) {
            return Err(format!("execution model {}", model));
        }
        let globals = self.uses.get(&e.func).cloned().unwrap_or_default();
        let mut params: Vec<String> = Vec::new();
        let mut prologue = String::new();
        let mut in_members = String::new();
        let mut out_members = String::new();
        let mut out_copy = String::new();
        let mut slots = Vec::new();
        let mut vertex_inputs = Vec::new();
        let mut push = false;
        let mut has_psize = false;
        // resources in (set, binding) order, with the module's slot numbers
        let mut res: Vec<(u32, u32, u32)> = Vec::new();
        for &g in &globals {
            if self.slot_of.contains_key(&g) {
                let set = self.deco(g, Dec::DescriptorSet).and_then(|x| x.first().copied()).unwrap_or(0);
                let b = self.deco(g, Dec::Binding).and_then(|x| x.first().copied()).unwrap_or(0);
                res.push((set, b, g));
            }
        }
        res.sort();
        for &(set, binding, g) in &res {
            let v = &self.vars[&g];
            let t = v.ty;
            let (kind, index) = self.slot_of[&g];
            let p = match kind {
                SlotKind::Texture => format!("{} g{} [[texture({})]]", self.image_name(t, self.var_access(g))?, g, index),
                SlotKind::Sampler => format!("sampler g{} [[sampler({})]]", g, index),
                SlotKind::Buffer => {
                    let space = if v.sc == Sc::Uniform as u32 { "constant" } else { "device" };
                    format!("{} {}& g{} [[buffer({})]]", space, self.tname(t)?, g, index)
                }
            };
            slots.push(Slot { set, binding, kind, index });
            params.push(p);
        }
        let mut sizes = self.sizes.contains(&e.func);
        let mut draw_index = false;
        let depth_mode = e.modes.iter().find_map(|(md, _)| {
            if *md == Mode::DepthGreater as u32 {
                Some("greater")
            } else if *md == Mode::DepthLess as u32 {
                Some("less")
            } else {
                None
            }
        }).unwrap_or("any");
        for &g in &globals {
            let v = &self.vars[&g];
            let t = v.ty;
            let tn = self.tname(t)?;
            let builtin = self.deco(g, Dec::BuiltIn).and_then(|x| x.first().copied());
            let loc = self.deco(g, Dec::Location).and_then(|x| x.first().copied());
            match v.sc {
                x if x == Sc::PushConstant as u32 => {
                    push = true;
                    params.push(format!("constant {}& g{} [[buffer({})]]", tn, g, PUSH_SLOT));
                }
                x if x == Sc::Private as u32 => {
                    let init = match v.init {
                        Some(c) => self.cexpr(c)?,
                        None => String::from("{}"),
                    };
                    prologue.push_str(&format!("    {} g{} = {};\n", tn, g, init));
                }
                x if x == Sc::Workgroup as u32 => {
                    // raw storage: threadgroup memory runs no constructors (matrices have some)
                    prologue.push_str(&format!("    threadgroup float4 jf_w{}[(sizeof({}) + 15) / 16];\n", g, tn));
                    prologue.push_str(&format!("    threadgroup {}& g{} = *reinterpret_cast<threadgroup {}*>(jf_w{});\n", tn, g, tn, g));
                }
                x if x == Sc::Input as u32 => {
                    if let Some(b) = builtin {
                        if b == Bi::HelperInvocation as u32 {
                            continue; // read through simd_is_helper_thread() at each load
                        }
                        if b == Bi::DrawIndex as u32 {
                            // (Metal has no draw index: the backend writes it into the sizes buffer per draw)
                            sizes = true;
                            draw_index = true;
                            prologue.push_str(&format!("    {} g{} = {}(jf_sizes[{}]);\n", tn, g, tn, DRAW_INDEX_AT));
                            continue;
                        }
                        let (pt, attr) = builtin_in(b, model)?;
                        params.push(format!("{} jf_b{} [[{}]]", pt, g, attr));
                        if b == Bi::SampleMask as u32 {
                            prologue.push_str(&format!("    {} g{} = {{}}; g{}[0] = {}(jf_b{});\n", tn, g, g, self.elem_name(t)?, g));
                        } else {
                            prologue.push_str(&format!("    {} g{} = {}(jf_b{});\n", tn, g, tn, g));
                        }
                    } else {
                        let l = loc.ok_or_else(|| format!("input %{} without Location or BuiltIn", g))?;
                        if vertex {
                            in_members.push_str(&format!("    {} a{} [[attribute({})]];\n", tn, l, l));
                            vertex_inputs.push(l);
                            prologue.push_str(&format!("    {} g{} = jf_in.a{};\n", tn, g, l));
                        } else {
                            let interp = self.interp(g);
                            in_members.push_str(&format!("    {} l{} [[user(locn{}){}]];\n", tn, l, l, interp));
                            prologue.push_str(&format!("    {} g{} = jf_in.l{};\n", tn, g, l));
                        }
                    }
                }
                x if x == Sc::Output as u32 => {
                    let init = match v.init {
                        Some(c) => self.cexpr(c)?,
                        None => String::from("{}"),
                    };
                    prologue.push_str(&format!("    {} g{} = {};\n", tn, g, init));
                    if let Some(b) = builtin {
                        match b {
                            x if x == Bi::Position as u32 => {
                                let inv = if self.has_deco(g, Dec::Invariant) { ", invariant" } else { "" };
                                out_members.push_str(&format!("    float4 b{} [[position{}]];\n", g, inv));
                                out_copy.push_str(&format!("o.b{} = g{}; ", g, g));
                            }
                            x if x == Bi::PointSize as u32 => {
                                // only for a point list: Metal refuses a point size with other topologies
                                if opt.point_size {
                                    has_psize = true;
                                    out_members.push_str(&format!("    float b{} [[point_size]];\n", g));
                                    out_copy.push_str(&format!("o.b{} = g{}; ", g, g));
                                }
                            }
                            x if x == Bi::ClipDistance as u32 => {
                                let Ty::Array { len, .. } = self.ty(t)? else { return err("ClipDistance is no array") };
                                let n = self.cu32(len.0)?;
                                out_members.push_str(&format!("    float b{} [[clip_distance]] [{}];\n", g, n));
                                for i in 0..n {
                                    out_copy.push_str(&format!("o.b{}[{}] = g{}[{}]; ", g, i, g, i));
                                }
                            }
                            x if x == Bi::FragDepth as u32 => {
                                out_members.push_str(&format!("    float b{} [[depth({})]];\n", g, depth_mode));
                                out_copy.push_str(&format!("o.b{} = g{}; ", g, g));
                            }
                            x if x == Bi::SampleMask as u32 => {
                                out_members.push_str(&format!("    uint b{} [[sample_mask]];\n", g));
                                out_copy.push_str(&format!("o.b{} = uint(g{}[0]); ", g, g));
                            }
                            x if x == Bi::Layer as u32 => {
                                out_members.push_str(&format!("    uint b{} [[render_target_array_index]];\n", g));
                                out_copy.push_str(&format!("o.b{} = uint(g{}); ", g, g));
                            }
                            _ => return Err(format!("output built-in {}", b)),
                        }
                    } else {
                        let l = loc.ok_or_else(|| format!("output %{} without Location or BuiltIn", g))?;
                        if fragment {
                            if l > 7 {
                                return Err(format!("colour output at location {} (Metal: 0 ..= 7)", l));
                            }
                            let idx = self.deco(g, Dec::Index).and_then(|x| x.first().copied()).unwrap_or(0);
                            let attr = if idx != 0 { format!("color({}), index({})", l, idx) } else { format!("color({})", l) };
                            out_members.push_str(&format!("    {} l{}_{} [[{}]];\n", tn, l, idx, attr));
                            out_copy.push_str(&format!("o.l{}_{} = g{}; ", l, idx, g));
                        } else {
                            out_members.push_str(&format!("    {} l{} [[user(locn{})]];\n", tn, l, l));
                            out_copy.push_str(&format!("o.l{} = g{}; ", l, g));
                        }
                    }
                }
                _ => {}
            }
        }
        if vertex && opt.point_size && !has_psize {
            out_members.push_str("    float jf_ps [[point_size]];\n");
            out_copy.push_str("o.jf_ps = 1.0f; ");
        }
        if sizes {
            params.push(format!("constant uint* jf_sizes [[buffer({})]]", SIZES_SLOT));
        }
        let mut src = String::new();
        if !in_members.is_empty() {
            src.push_str(&format!("struct {}_in {{\n{}}};\n", fname, in_members));
            params.insert(0, format!("{}_in jf_in [[stage_in]]", fname));
        }
        let (ret_ty, ret_stmt) = if !out_members.is_empty() {
            src.push_str(&format!("struct {}_out {{\n{}}};\n", fname, out_members));
            (format!("{}_out", fname), format!("{{ {}_out o; {}return o; }}", fname, out_copy))
        } else {
            (String::from("void"), String::from("return;"))
        };
        let (qual, attrs) = if vertex {
            ("vertex", "")
        } else if fragment {
            ("fragment", if e.modes.iter().any(|(md, _)| *md == Mode::EarlyFragmentTests as u32) { "[[early_fragment_tests]] " } else { "" })
        } else {
            ("kernel", "")
        };
        let body = crate::msl_fn::entry_body(self, e.func, &ret_stmt, conv)?;
        src.push_str(&format!("{}{} {} {}({}) {{\n{}{}}}\n", attrs, qual, ret_ty, fname, params.join(", "), prologue, body));
        let workgroup = if compute {
            reflect::workgroup_size(self.words, &e.name, opt.spec)?.unwrap_or([1, 1, 1])
        } else {
            [1, 1, 1]
        };
        Ok((src, Entry { name: e.name.clone(), function: fname, model, workgroup, slots, push_constants: push, sizes, draw_index, vertex_inputs }))
    }

    /// the element type name of an array type
    fn elem_name(&self, t: u32) -> Result<String, String> {
        match self.ty(t)? {
            Ty::Array { elem, .. } | Ty::RuntimeArray { elem } => self.tname(elem.0),
            _ => err("no array type"),
        }
    }

    fn interp(&self, g: u32) -> &'static str {
        let np = self.has_deco(g, Dec::NoPerspective);
        if self.has_deco(g, Dec::Flat) {
            ", flat"
        } else if self.has_deco(g, Dec::Centroid) {
            if np { ", centroid_no_perspective" } else { ", centroid_perspective" }
        } else if self.has_deco(g, Dec::Sample) {
            if np { ", sample_no_perspective" } else { ", sample_perspective" }
        } else if np {
            ", center_no_perspective"
        } else {
            ""
        }
    }
}

/// the parameter type and attribute of an input built-in
fn builtin_in(b: u32, model: u32) -> Result<(&'static str, &'static str), String> {
    let compute = model == Model::GLCompute as u32;
    let fragment = model == Model::Fragment as u32;
    Ok(match b {
        x if x == Bi::VertexIndex as u32 => ("uint", "vertex_id"),
        x if x == Bi::InstanceIndex as u32 => ("uint", "instance_id"),
        x if x == Bi::BaseVertex as u32 => ("uint", "base_vertex"),
        x if x == Bi::BaseInstance as u32 => ("uint", "base_instance"),
        x if x == Bi::FragCoord as u32 => ("float4", "position"),
        x if x == Bi::FrontFacing as u32 => ("bool", "front_facing"),
        x if x == Bi::SampleId as u32 => ("uint", "sample_id"),
        x if x == Bi::SampleMask as u32 => ("uint", "sample_mask"),
        x if x == Bi::PrimitiveId as u32 => ("uint", "primitive_id"),
        x if x == Bi::PointCoord as u32 => ("float2", "point_coord"),
        x if x == Bi::Layer as u32 && fragment => ("uint", "render_target_array_index"),
        x if x == Bi::GlobalInvocationId as u32 => ("uint3", "thread_position_in_grid"),
        x if x == Bi::LocalInvocationId as u32 => ("uint3", "thread_position_in_threadgroup"),
        x if x == Bi::WorkgroupId as u32 => ("uint3", "threadgroup_position_in_grid"),
        x if x == Bi::NumWorkgroups as u32 => ("uint3", "threadgroups_per_grid"),
        x if x == Bi::LocalInvocationIndex as u32 => ("uint", "thread_index_in_threadgroup"),
        x if x == Bi::SubgroupSize as u32 => ("uint", "threads_per_simdgroup"),
        x if x == Bi::SubgroupLocalInvocationId as u32 => ("uint", "thread_index_in_simdgroup"),
        x if x == Bi::SubgroupId as u32 && compute => ("uint", "simdgroup_index_in_threadgroup"),
        x if x == Bi::NumSubgroups as u32 && compute => ("uint", "simdgroups_per_threadgroup"),
        _ => return Err(format!("input built-in {} in model {}", b, model)),
    })
}

/// a float literal that reads back as the same f32
pub(crate) fn f32_lit(f: f32) -> String {
    if f.is_finite() {
        format!("{:?}f", f)
    } else {
        format!("as_type<float>({:#x}u)", f.to_bits())
    }
}

fn sanitize(s: &str) -> String {
    let mut o: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if o.is_empty() {
        o.push('_');
    }
    o
}

const PRELUDE: &str = "\
#include <metal_stdlib>
using namespace metal;

template<typename T, uint N> struct jf_arr {
    T e[N];
    thread T& operator[](uint i) thread { return e[i]; }
    const thread T& operator[](uint i) const thread { return e[i]; }
    device T& operator[](uint i) device { return e[i]; }
    const device T& operator[](uint i) const device { return e[i]; }
    constant T& operator[](uint i) constant { return e[i]; }
    threadgroup T& operator[](uint i) threadgroup { return e[i]; }
    const threadgroup T& operator[](uint i) const threadgroup { return e[i]; }
};
template<typename T> T jf_fmod(T a, T b) { return a - b * floor(a / b); }
// atan2 with IEEE's signed zeros (Metal's gives -pi/2 for atan2(1, -0))
static inline float jf_atan2(float y, float x) {
    if (y == 0.0f) { return signbit(x) ? copysign(M_PI_F, y) : copysign(0.0f, y); }
    if (x == 0.0f) { return copysign(M_PI_2_F, y); }
    return atan2(y, x);
}
static inline half jf_atan2(half y, half x) { return half(jf_atan2(float(y), float(x))); }
// asinh of negative numbers through the positive side (Metal's loses digits there)
static inline float jf_asinh(float x) { return x < 0.0f ? -asinh(-x) : asinh(x); }
static inline half jf_asinh(half x) { return half(jf_asinh(float(x))); }
template<typename T> T jf_smod(T a, T b) { T r = a % b; return select(r, r + b, (r != T(0)) & ((r < T(0)) != (b < T(0)))); }\n";


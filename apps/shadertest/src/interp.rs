// interp -- a small SPIR-V interpreter (one invocation at a time, the instructions the
// benchmark shaders use) and the checks that run move, cull_reset, cull, vs_instanced,
// vs_single and fs_lambert through it: move and cull must equal move_ref / cull_ref bit for
// bit (IEEE add / sub / mul as Vulkan requires them), the vertex and fragment shaders the f64
// maths within a tolerance.  Buffers are the byte images lib/gpu uploads, laid out by the
// modules' own Offset / ArrayStride / MatrixStride decorations.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use shader::bench::{self, Camera, DrawCmd, Object, Push, Shader};
use shader::{op, parse, spv, Bi, Dec, Ty};

use crate::T;

#[derive(Clone, Debug)]
enum Val {
    W(Vec<u32>),
    /// variable, byte offset, pointee type, matrix stride (0: packed)
    P(u32, u32, u32, u32),
}

pub struct Vm<'a> {
    ins: Vec<parse::Inst<'a>>,
    tys: BTreeMap<u32, Ty>,
    consts: BTreeMap<u32, Vec<u32>>,
    offs: BTreeMap<(u32, u32), u32>,
    mstride: BTreeMap<(u32, u32), u32>,
    stride: BTreeMap<u32, u32>,
    builtin: BTreeMap<u32, u32>,
    location: BTreeMap<u32, u32>,
    binding: BTreeMap<u32, u32>,
    vars: BTreeMap<u32, (u32, u32)>,
    mem: BTreeMap<u32, Vec<u8>>,
    vals: Vec<Option<Val>>,
    labels: BTreeMap<u32, usize>,
    rty: Vec<u32>,
    main: usize,
    pub steps: u64,
}

fn f(w: u32) -> f32 {
    f32::from_bits(w)
}

fn b(x: bool) -> u32 {
    x as u32
}

impl<'a> Vm<'a> {
    pub fn new(words: &'a [u32]) -> Vm<'a> {
        let ins = parse::parse(words).expect("interp: parse");
        let mut vm = Vm { ins, tys: BTreeMap::new(), consts: BTreeMap::new(), offs: BTreeMap::new(),
                          mstride: BTreeMap::new(), stride: BTreeMap::new(), builtin: BTreeMap::new(),
                          location: BTreeMap::new(), binding: BTreeMap::new(), vars: BTreeMap::new(),
                          mem: BTreeMap::new(), vals: alloc::vec![None; words[3] as usize], labels: BTreeMap::new(),
                          rty: alloc::vec![0; words[3] as usize], main: 0, steps: 0 };
        let mut entry = 0;
        let mut in_fn = false;
        for k in 0..vm.ins.len() {
            let i = vm.ins[k];
            let o = i.ops;
            let g = |n: usize| o.get(n).copied().unwrap_or(0);
            if i.res != 0 {
                vm.rty[i.res as usize] = i.ty;
            }
            match i.op {
                op::EntryPoint => entry = g(1),
                op::Decorate => match g(1) {
                    x if x == Dec::ArrayStride as u32 => {
                        vm.stride.insert(g(0), g(2));
                    }
                    x if x == Dec::BuiltIn as u32 => {
                        vm.builtin.insert(g(0), g(2));
                    }
                    x if x == Dec::Location as u32 => {
                        vm.location.insert(g(0), g(2));
                    }
                    x if x == Dec::Binding as u32 => {
                        vm.binding.insert(g(0), g(2));
                    }
                    _ => {}
                },
                op::MemberDecorate => match g(2) {
                    x if x == Dec::Offset as u32 => {
                        vm.offs.insert((g(0), g(1)), g(3));
                    }
                    x if x == Dec::MatrixStride as u32 => {
                        vm.mstride.insert((g(0), g(1)), g(3));
                    }
                    _ => {}
                },
                op::TypeVoid => {
                    vm.tys.insert(i.res, Ty::Void);
                }
                op::TypeBool => {
                    vm.tys.insert(i.res, Ty::Bool);
                }
                op::TypeInt => {
                    vm.tys.insert(i.res, Ty::Int { width: g(0), signed: g(1) != 0 });
                }
                op::TypeFloat => {
                    vm.tys.insert(i.res, Ty::Float { width: g(0) });
                }
                op::TypeVector => {
                    vm.tys.insert(i.res, Ty::Vector { elem: shader::Id(g(0)), n: g(1) });
                }
                op::TypeMatrix => {
                    vm.tys.insert(i.res, Ty::Matrix { col: shader::Id(g(0)), cols: g(1) });
                }
                op::TypeArray => {
                    vm.tys.insert(i.res, Ty::Array { elem: shader::Id(g(0)), len: shader::Id(g(1)) });
                }
                op::TypeRuntimeArray => {
                    vm.tys.insert(i.res, Ty::RuntimeArray { elem: shader::Id(g(0)) });
                }
                op::TypeStruct => {
                    vm.tys.insert(i.res, Ty::Struct { members: o.iter().map(|&m| shader::Id(m)).collect() });
                }
                op::TypePointer => {
                    vm.tys.insert(i.res, Ty::Pointer { sc: g(0), pointee: shader::Id(g(1)) });
                }
                op::Constant => {
                    vm.consts.insert(i.res, o.to_vec());
                }
                op::ConstantTrue | op::ConstantFalse => {
                    vm.consts.insert(i.res, alloc::vec![b(i.op == op::ConstantTrue)]);
                }
                op::ConstantComposite => {
                    let v: Vec<u32> = o.iter().flat_map(|c| vm.consts[c].clone()).collect();
                    vm.consts.insert(i.res, v);
                }
                op::ConstantNull => {
                    let n = vm.flat(i.ty);
                    vm.consts.insert(i.res, alloc::vec![0; n as usize]);
                }
                op::Variable if !in_fn => {
                    let Some(Ty::Pointer { pointee, .. }) = vm.tys.get(&i.ty).cloned() else { panic!("interp: variable") };
                    vm.vars.insert(i.res, (g(0), pointee.0));
                    if !matches!(g(0), 2 | 9 | 12) {
                        let n = vm.flat(pointee.0) as usize * 4;
                        vm.mem.insert(i.res, alloc::vec![0; n]);
                    }
                }
                op::Function => {
                    in_fn = true;
                    if i.res == entry || entry == 0 {
                        vm.main = k;
                    }
                }
                op::FunctionEnd => in_fn = false,
                op::Label => {
                    vm.labels.insert(i.res, k);
                }
                _ => {}
            }
        }
        // the entry point is declared before the functions
        if let Some(k) = vm.ins.iter().position(|i| i.op == op::Function && i.res == entry) {
            vm.main = k;
        }
        for (id, w) in vm.consts.clone() {
            vm.vals[id as usize] = Some(Val::W(w));
        }
        for &v in vm.vars.keys() {
            vm.vals[v as usize] = Some(Val::P(v, 0, vm.vars[&v].1, 0));
        }
        vm
    }

    /// 32-bit words of a value of type t
    fn flat(&self, t: u32) -> u32 {
        match &self.tys[&t] {
            Ty::Vector { n, .. } => *n,
            Ty::Matrix { col, cols } => cols * self.flat(col.0),
            Ty::Array { elem, len } => self.consts[&len.0][0] * self.flat(elem.0),
            Ty::Struct { members } => members.iter().map(|m| self.flat(m.0)).sum(),
            _ => 1,
        }
    }

    fn explicit(&self, var: u32) -> bool {
        self.vars.get(&var).is_some_and(|v| matches!(v.0, 2 | 9 | 12))
    }

    fn member_off(&self, s: u32, k: u32, explicit: bool, members: &[shader::Id]) -> u32 {
        if explicit {
            self.offs[&(s, k)]
        } else {
            members[..k as usize].iter().map(|m| 4 * self.flat(m.0)).sum()
        }
    }

    fn elem_stride(&self, arr: u32, elem: u32, explicit: bool) -> u32 {
        if explicit { self.stride[&arr] } else { 4 * self.flat(elem) }
    }

    fn word(&self, var: u32, off: u32) -> Result<u32, String> {
        let m = &self.mem[&var];
        let o = off as usize;
        if o + 4 > m.len() {
            return Err(format!("read of %{} at {} past its {} bytes", var, off, m.len()));
        }
        Ok(u32::from_le_bytes([m[o], m[o + 1], m[o + 2], m[o + 3]]))
    }

    fn put(&mut self, var: u32, off: u32, w: u32) -> Result<(), String> {
        let m = self.mem.get_mut(&var).expect("memory");
        let o = off as usize;
        if o + 4 > m.len() {
            return Err(format!("write to %{} at {} past its {} bytes", var, off, m.len()));
        }
        m[o..o + 4].copy_from_slice(&w.to_le_bytes());
        Ok(())
    }

    /// the words of type t at (var, off); ms: the matrix stride of a matrix member
    fn read(&self, var: u32, off: u32, t: u32, ms: u32, out: &mut Vec<u32>) -> Result<(), String> {
        let ex = self.explicit(var);
        match self.tys[&t].clone() {
            Ty::Vector { n, .. } => {
                for k in 0..n {
                    out.push(self.word(var, off + 4 * k)?);
                }
            }
            Ty::Matrix { col, cols } => {
                let st = if ms > 0 { ms } else { 4 * self.flat(col.0) };
                for c in 0..cols {
                    self.read(var, off + c * st, col.0, 0, out)?;
                }
            }
            Ty::Array { elem, len } => {
                let st = self.elem_stride(t, elem.0, ex);
                for k in 0..self.consts[&len.0][0] {
                    self.read(var, off + k * st, elem.0, ms, out)?;
                }
            }
            Ty::Struct { members } => {
                for (k, m) in members.iter().enumerate() {
                    let o = self.member_off(t, k as u32, ex, &members);
                    let ms = self.mstride.get(&(t, k as u32)).copied().unwrap_or(0);
                    self.read(var, off + o, m.0, ms, out)?;
                }
            }
            _ => out.push(self.word(var, off)?),
        }
        Ok(())
    }

    fn write(&mut self, var: u32, off: u32, t: u32, ms: u32, w: &[u32]) -> Result<usize, String> {
        let ex = self.explicit(var);
        let mut used = 0;
        match self.tys[&t].clone() {
            Ty::Vector { n, .. } => {
                for k in 0..n {
                    self.put(var, off + 4 * k, w[k as usize])?;
                }
                used = n as usize;
            }
            Ty::Matrix { col, cols } => {
                let st = if ms > 0 { ms } else { 4 * self.flat(col.0) };
                for c in 0..cols {
                    used += self.write(var, off + c * st, col.0, 0, &w[used..])?;
                }
            }
            Ty::Array { elem, len } => {
                let st = self.elem_stride(t, elem.0, ex);
                for k in 0..self.consts[&len.0][0] {
                    used += self.write(var, off + k * st, elem.0, ms, &w[used..])?;
                }
            }
            Ty::Struct { members } => {
                for (k, m) in members.iter().enumerate() {
                    let o = self.member_off(t, k as u32, ex, &members);
                    let ms = self.mstride.get(&(t, k as u32)).copied().unwrap_or(0);
                    used += self.write(var, off + o, m.0, ms, &w[used..])?;
                }
            }
            _ => {
                self.put(var, off, w[0])?;
                used = 1;
            }
        }
        Ok(used)
    }

    fn var_of(&self, pred: impl Fn(u32) -> bool) -> u32 {
        *self.vars.keys().find(|&&v| pred(v)).expect("interp: no such variable")
    }

    /// the bytes of the buffer at binding (set 0)
    pub fn buffer(&mut self, binding: u32, bytes: Vec<u8>) {
        let v = self.var_of(|v| self.binding.get(&v) == Some(&binding));
        self.mem.insert(v, bytes);
    }

    pub fn buffer_bytes(&self, binding: u32) -> &[u8] {
        let v = self.var_of(|v| self.binding.get(&v) == Some(&binding));
        &self.mem[&v]
    }

    pub fn push(&mut self, bytes: Vec<u8>) {
        let v = self.var_of(|v| self.vars[&v].0 == 9);
        self.mem.insert(v, bytes);
    }

    /// set a built-in input (a shader without it ignores it)
    pub fn set_builtin(&mut self, bi: Bi, w: &[u32]) {
        let v = self.vars.keys().find(|&&v| self.builtin.get(&v) == Some(&(bi as u32)) && self.vars[&v].0 == 1).copied();
        if let Some(v) = v {
            let t = self.vars[&v].1;
            self.write(v, 0, t, 0, w).expect("builtin");
        }
    }

    pub fn set_input(&mut self, loc: u32, w: &[u32]) {
        let v = self.var_of(|v| self.location.get(&v) == Some(&loc) && self.vars[&v].0 == 1);
        let t = self.vars[&v].1;
        self.write(v, 0, t, 0, w).expect("input");
    }

    fn output_var(&self, v: u32) -> Vec<u32> {
        let mut o = Vec::new();
        self.read(v, 0, self.vars[&v].1, 0, &mut o).expect("output");
        o
    }

    pub fn output_builtin(&self, bi: Bi) -> Vec<u32> {
        self.output_var(self.var_of(|v| self.builtin.get(&v) == Some(&(bi as u32)) && self.vars[&v].0 == 3))
    }

    pub fn output(&self, loc: u32) -> Vec<u32> {
        self.output_var(self.var_of(|v| self.location.get(&v) == Some(&loc) && self.vars[&v].0 == 3))
    }

    fn w(&self, id: u32) -> Result<&Vec<u32>, String> {
        match self.vals.get(id as usize) {
            Some(Some(Val::W(w))) => Ok(w),
            _ => Err(format!("%{} has no value", id)),
        }
    }

    fn p(&self, id: u32) -> Result<(u32, u32, u32, u32), String> {
        match self.vals.get(id as usize) {
            Some(Some(Val::P(v, o, t, ms))) => Ok((*v, *o, *t, *ms)),
            _ => Err(format!("%{} is no pointer", id)),
        }
    }

    fn set(&mut self, id: u32, v: Vec<u32>) {
        self.vals[id as usize] = Some(Val::W(v));
    }

    /// run the entry point once
    pub fn run(&mut self) -> Result<(), String> {
        let (mut pc, mut prev, mut cur) = (self.main + 1, 0u32, 0u32);
        loop {
            self.steps += 1;
            let i = self.ins[pc];
            let o = i.ops;
            match i.op {
                op::Label => {
                    prev = cur;
                    cur = i.res;
                }
                op::Branch => {
                    pc = self.labels[&o[0]];
                    continue;
                }
                op::BranchConditional => {
                    pc = self.labels[&if self.w(o[0])?[0] != 0 { o[1] } else { o[2] }];
                    continue;
                }
                op::Return => return Ok(()),
                op::SelectionMerge | op::LoopMerge | op::Line | op::Nop => {}
                op::Phi => {
                    let v = o.chunks(2).find(|p| p[1] == prev).map(|p| p[0]).ok_or("phi without its predecessor")?;
                    let w = self.w(v)?.clone();
                    self.set(i.res, w);
                }
                op::Variable => {
                    let pointee = self.vars.get(&i.res).map(|v| v.1).unwrap_or_else(|| match &self.tys[&i.ty] {
                        Ty::Pointer { pointee, .. } => pointee.0,
                        _ => 0,
                    });
                    self.vars.insert(i.res, (7, pointee));
                    let n = self.flat(pointee) as usize * 4;
                    self.mem.insert(i.res, alloc::vec![0; n]);
                    if let Some(&init) = o.get(1) {
                        let w = self.w(init)?.clone();
                        self.write(i.res, 0, pointee, 0, &w)?;
                    }
                    self.vals[i.res as usize] = Some(Val::P(i.res, 0, pointee, 0));
                }
                _ => self.exec(&i)?,
            }
            pc += 1;
        }
    }

    fn exec(&mut self, i: &parse::Inst) -> Result<(), String> {
        let o = i.ops;
        let fl = |x: &Vec<u32>| x.iter().map(|&w| f(w)).collect::<Vec<f32>>();
        let r: Vec<u32> = match i.op {
            op::Load => {
                let (v, off, t, ms) = self.p(o[0])?;
                let mut out = Vec::new();
                self.read(v, off, t, ms, &mut out)?;
                out
            }
            op::Store => {
                let (v, off, t, ms) = self.p(o[0])?;
                let w = self.w(o[1])?.clone();
                self.write(v, off, t, ms, &w)?;
                return Ok(());
            }
            op::AccessChain => {
                let (v, mut off, mut t, mut ms) = self.p(o[0])?;
                let ex = self.explicit(v);
                for &x in &o[1..] {
                    let k = self.w(x)?[0];
                    match self.tys[&t].clone() {
                        Ty::Struct { members } => {
                            off += self.member_off(t, k, ex, &members);
                            ms = self.mstride.get(&(t, k)).copied().unwrap_or(0);
                            t = members[k as usize].0;
                        }
                        Ty::Array { elem, .. } | Ty::RuntimeArray { elem } => {
                            off += k * self.elem_stride(t, elem.0, ex);
                            t = elem.0;
                        }
                        Ty::Matrix { col, .. } => {
                            off += k * if ms > 0 { ms } else { 4 * self.flat(col.0) };
                            t = col.0;
                            ms = 0;
                        }
                        Ty::Vector { elem, .. } => {
                            off += 4 * k;
                            t = elem.0;
                        }
                        _ => return Err(String::from("access into a scalar")),
                    }
                }
                self.vals[i.res as usize] = Some(Val::P(v, off, t, ms));
                return Ok(());
            }
            op::CompositeConstruct => {
                let mut v = Vec::new();
                for &p in o {
                    v.extend_from_slice(self.w(p)?);
                }
                v
            }
            op::CompositeExtract => {
                let src = self.w(o[0])?;
                let mut t = self.vals_type(o[0])?;
                let mut at = 0;
                for &k in &o[1..] {
                    match self.tys[&t].clone() {
                        Ty::Struct { members } => {
                            at += members[..k as usize].iter().map(|m| self.flat(m.0)).sum::<u32>();
                            t = members[k as usize].0;
                        }
                        Ty::Vector { elem, .. } | Ty::Array { elem, .. } => {
                            at += k * self.flat(elem.0);
                            t = elem.0;
                        }
                        Ty::Matrix { col, .. } => {
                            at += k * self.flat(col.0);
                            t = col.0;
                        }
                        _ => return Err(String::from("extract from a scalar")),
                    }
                }
                src[at as usize..(at + self.flat(t)) as usize].to_vec()
            }
            op::VectorShuffle => {
                let (a, bb) = (self.w(o[0])?, self.w(o[1])?);
                o[2..].iter().map(|&c| if (c as usize) < a.len() { a[c as usize] } else { bb[c as usize - a.len()] }).collect()
            }
            op::FAdd | op::FSub | op::FMul | op::FDiv => {
                let (a, bb) = (fl(self.w(o[0])?), fl(self.w(o[1])?));
                a.iter().zip(&bb).map(|(x, y)| match i.op {
                    op::FAdd => x + y,
                    op::FSub => x - y,
                    op::FMul => x * y,
                    _ => x / y,
                }.to_bits()).collect()
            }
            op::FNegate => fl(self.w(o[0])?).iter().map(|x| (-x).to_bits()).collect(),
            op::IAdd | op::ISub | op::IMul | op::UDiv | op::UMod | op::BitwiseAnd | op::BitwiseOr | op::BitwiseXor
            | op::ShiftLeftLogical | op::ShiftRightLogical | op::ShiftRightArithmetic => {
                let (a, bb) = (self.w(o[0])?, self.w(o[1])?);
                a.iter().zip(bb.iter()).map(|(&x, &y)| match i.op {
                    op::IAdd => x.wrapping_add(y),
                    op::ISub => x.wrapping_sub(y),
                    op::IMul => x.wrapping_mul(y),
                    op::UDiv => x / y,
                    op::UMod => x % y,
                    op::BitwiseAnd => x & y,
                    op::BitwiseOr => x | y,
                    op::BitwiseXor => x ^ y,
                    op::ShiftLeftLogical => x << y,
                    op::ShiftRightLogical => x >> y,
                    _ => ((x as i32) >> y) as u32,
                }).collect()
            }
            op::Not => self.w(o[0])?.iter().map(|x| !x).collect(),
            op::IEqual | op::INotEqual | op::ULessThan | op::ULessThanEqual | op::UGreaterThan | op::UGreaterThanEqual
            | op::SLessThan | op::SGreaterThanEqual => {
                let (a, bb) = (self.w(o[0])?, self.w(o[1])?);
                a.iter().zip(bb.iter()).map(|(&x, &y)| b(match i.op {
                    op::IEqual => x == y,
                    op::INotEqual => x != y,
                    op::ULessThan => x < y,
                    op::ULessThanEqual => x <= y,
                    op::UGreaterThan => x > y,
                    op::UGreaterThanEqual => x >= y,
                    op::SLessThan => (x as i32) < (y as i32),
                    _ => (x as i32) >= (y as i32),
                })).collect()
            }
            op::FOrdLessThan | op::FOrdLessThanEqual | op::FOrdGreaterThan | op::FOrdGreaterThanEqual | op::FOrdEqual
            | op::FUnordNotEqual => {
                let (a, bb) = (fl(self.w(o[0])?), fl(self.w(o[1])?));
                a.iter().zip(&bb).map(|(x, y)| b(match i.op {
                    op::FOrdLessThan => x < y,
                    op::FOrdLessThanEqual => x <= y,
                    op::FOrdGreaterThan => x > y,
                    op::FOrdGreaterThanEqual => x >= y,
                    op::FOrdEqual => x == y,
                    _ => x != y,
                })).collect()
            }
            op::LogicalAnd | op::LogicalOr => {
                let (a, bb) = (self.w(o[0])?, self.w(o[1])?);
                a.iter().zip(bb.iter()).map(|(&x, &y)| b(if i.op == op::LogicalAnd { x & y != 0 } else { x | y != 0 })).collect()
            }
            op::LogicalNot => self.w(o[0])?.iter().map(|&x| b(x == 0)).collect(),
            op::Select => {
                let (c, a, bb) = (self.w(o[0])?, self.w(o[1])?, self.w(o[2])?);
                (0..a.len()).map(|k| if c[k.min(c.len() - 1)] != 0 { a[k] } else { bb[k] }).collect()
            }
            op::ConvertUToF => self.w(o[0])?.iter().map(|&x| (x as f32).to_bits()).collect(),
            op::ConvertSToF => self.w(o[0])?.iter().map(|&x| (x as i32 as f32).to_bits()).collect(),
            op::ConvertFToU => self.w(o[0])?.iter().map(|&x| f(x) as u32).collect(),
            op::ConvertFToS => self.w(o[0])?.iter().map(|&x| f(x) as i32 as u32).collect(),
            op::Bitcast | op::CopyObject => self.w(o[0])?.clone(),
            op::VectorTimesScalar => {
                let (a, s) = (fl(self.w(o[0])?), f(self.w(o[1])?[0]));
                a.iter().map(|x| (x * s).to_bits()).collect()
            }
            op::MatrixTimesVector => {
                let (m, v) = (fl(self.w(o[0])?), fl(self.w(o[1])?));
                let rows = m.len() / v.len();
                (0..rows).map(|r| {
                    let mut s = 0.0f32;
                    for (c, x) in v.iter().enumerate() {
                        s += m[c * rows + r] * x;
                    }
                    s.to_bits()
                }).collect()
            }
            op::Dot => {
                let (a, bb) = (fl(self.w(o[0])?), fl(self.w(o[1])?));
                alloc::vec![a.iter().zip(&bb).fold(0.0f32, |s, (x, y)| s + x * y).to_bits()]
            }
            op::ExtInst => {
                let args: Vec<Vec<f32>> = o[2..].iter().map(|&a| self.w(a).map(fl)).collect::<Result<_, _>>()?;
                match o[1] {
                    spv::glsl::Floor => args[0].iter().map(|&x| bench::floor(x).to_bits()).collect(),
                    spv::glsl::FMax => args[0].iter().zip(&args[1]).map(|(x, y)| x.max(*y).to_bits()).collect(),
                    spv::glsl::FMin => args[0].iter().zip(&args[1]).map(|(x, y)| x.min(*y).to_bits()).collect(),
                    spv::glsl::Normalize => {
                        let l = rt::fx::sqrt(args[0].iter().fold(0.0f32, |s, x| s + x * x));
                        args[0].iter().map(|x| (x / l).to_bits()).collect()
                    }
                    n => return Err(format!("GLSL.std.450 {} is not interpreted", n)),
                }
            }
            op::AtomicIAdd => {
                let (v, off, _, _) = self.p(o[0])?;
                let old = self.word(v, off)?;
                let add = self.w(o[3])?[0];
                self.put(v, off, old.wrapping_add(add))?;
                alloc::vec![old]
            }
            op::ControlBarrier | op::MemoryBarrier => return Ok(()),
            _ => return Err(format!("Op{} is not interpreted", i.info.map_or("?", |x| x.name))),
        };
        self.set(i.res, r);
        Ok(())
    }

    fn vals_type(&self, id: u32) -> Result<u32, String> {
        self.rty.get(id as usize).copied().filter(|&t| t != 0).ok_or(format!("no type for %{}", id))
    }
}

fn words(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// the rotation columns of the shader (unit quaternion x y z w), in f64
fn rot_cols(q: [f32; 4]) -> [[f64; 3]; 3] {
    let [x, y, z, w] = q.map(|v| v as f64);
    [[1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)],
     [2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)],
     [2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)]]
}

/// the clip position and normal vs_* should give for vertex v of object o
fn vs_expect(o: &Object, cam: &Camera, v: usize) -> ([f64; 4], [f64; 3]) {
    let cv = bench::cube_vertices()[v];
    let r = rot_cols(o.rot);
    let mut world = [0.0f64; 3];
    let mut n = [0.0f64; 3];
    for k in 0..3 {
        for c in 0..3 {
            world[k] += r[c][k] * o.scale as f64 * cv.pos[c] as f64;
            n[k] += r[c][k] * cv.normal[c] as f64;
        }
        world[k] += o.pos[k] as f64;
    }
    let mut clip = [0.0f64; 4];
    for (row, x) in clip.iter_mut().enumerate() {
        *x = (0..3).map(|c| cam.view_proj[c * 4 + row] as f64 * world[c]).sum::<f64>() + cam.view_proj[12 + row] as f64;
    }
    (clip, n)
}

fn close(got: &[u32], want: &[f64], tol: f64) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(&g, &w)| (f(g) as f64 - w).abs() <= tol * w.abs().max(1.0))
}

const COUNT: u32 = 3000;

fn dispatch(vm: &mut Vm, count: u32, wg: u32) -> Result<(), String> {
    for gid in 0..count.div_ceil(wg) * wg {
        vm.set_builtin(Bi::GlobalInvocationId, &[gid, 0, 0]);
        vm.run()?;
    }
    Ok(())
}

pub fn run(t: &mut T) {
    let p = crate::refs::sample_push(COUNT);
    let objs = bench::move_all(&p);
    let cam = crate::refs::sample_camera(p.extent);
    for minor in [3u32, 6] {
        let lv = format!("1.{}", minor);
        // move: every object as move_ref, the sentinel after the last one untouched
        let w = Shader::Move.build(minor);
        let mut vm = Vm::new(&w);
        vm.buffer(bench::B_OBJECTS, alloc::vec![0xABu8; (COUNT as usize + 1) * 48]);
        vm.push(p.bytes());
        let r = dispatch(&mut vm, COUNT, bench::WG_MOVE);
        t.check(&format!("move {} runs: {:?}", lv, r), r.is_ok());
        let got = vm.buffer_bytes(bench::B_OBJECTS);
        let want = bench::objects_bytes(&objs);
        let first_bad = (0..COUNT as usize).find(|&k| got[k * 48..k * 48 + 48] != want[k * 48..k * 48 + 48]);
        if let Some(k) = first_bad {
            crate::out(&format!("   move {} object {}: got {:?}\n      want {:?}\n", lv, k,
                                Object::from_bytes(&got[k * 48..]), objs[k]));
        }
        t.check(&format!("move {} = move_ref bit for bit ({} objects)", lv, COUNT), first_bad.is_none());
        t.check(&format!("move {} leaves the sentinel", lv), got[COUNT as usize * 48..].iter().all(|&x| x == 0xAB));
        if minor == 3 {
            crate::out(&format!("   move: {} instructions per object\n", vm.steps / (COUNT.div_ceil(256) * 256) as u64));
        }
        // cull_reset
        let w = Shader::CullReset.build(minor);
        let mut vm = Vm::new(&w);
        vm.buffer(bench::B_DRAW, alloc::vec![0x5Au8; 20]);
        vm.set_builtin(Bi::GlobalInvocationId, &[0, 0, 0]);
        let r = vm.run();
        t.check(&format!("cull_reset {} runs: {:?}", lv, r), r.is_ok());
        t.eq(&format!("cull_reset {}", lv), DrawCmd::from_bytes(vm.buffer_bytes(bench::B_DRAW)), DrawCmd::reset());
        // cull: the visible list (sorted) and instance_count as cull_ref
        let w = Shader::Cull.build(minor);
        let mut vm = Vm::new(&w);
        vm.buffer(bench::B_OBJECTS, bench::objects_bytes(&objs));
        vm.buffer(bench::B_CAMERA, cam.bytes());
        vm.buffer(bench::B_VISIBLE, alloc::vec![0xFFu8; COUNT as usize * 4]);
        vm.buffer(bench::B_DRAW, DrawCmd::reset().bytes());
        vm.push(p.bytes());
        let r = dispatch(&mut vm, COUNT, bench::WG_CULL);
        t.check(&format!("cull {} runs: {:?}", lv, r), r.is_ok());
        let d = DrawCmd::from_bytes(vm.buffer_bytes(bench::B_DRAW));
        let want = bench::cull_ref(&objs, &cam);
        t.eq(&format!("cull {} instance_count", lv), d.instance_count, want.len() as u32);
        t.eq(&format!("cull {} index_count kept", lv), d.index_count, 36);
        let vb = vm.buffer_bytes(bench::B_VISIBLE);
        let mut list: Vec<u32> = (0..d.instance_count as usize).map(|k| u32::from_le_bytes([vb[4 * k], vb[4 * k + 1], vb[4 * k + 2], vb[4 * k + 3]])).collect();
        list.sort_unstable();
        t.check(&format!("cull {} visible list = cull_ref ({} of {})", lv, want.len(), COUNT), list == want);
        // vs_instanced / vs_single: gl_Position, normal and colour of some vertices
        for (sh, inst) in [(Shader::VsInstanced, true), (Shader::VsSingle, false)] {
            let w = sh.build(minor);
            let mut vm = Vm::new(&w);
            vm.buffer(bench::B_OBJECTS, bench::objects_bytes(&objs));
            vm.buffer(bench::B_CAMERA, cam.bytes());
            vm.buffer(bench::B_CUBE, bench::cube_bytes());
            if inst {
                vm.buffer(bench::B_VISIBLE, want.iter().flat_map(|x| x.to_le_bytes()).collect());
            }
            let mut ok = true;
            for (k, &obj) in want.iter().enumerate().step_by((want.len() / 7).max(1)) {
                if inst {
                    vm.set_builtin(Bi::InstanceIndex, &[k as u32]);
                } else {
                    vm.push(Push { object_index: obj, ..p }.bytes());
                }
                for v in [0usize, 5, 11, 23] {
                    vm.set_builtin(Bi::VertexIndex, &[v as u32]);
                    if let Err(e) = vm.run() {
                        crate::out(&format!("   {} {}: {}\n", sh.name(), lv, e));
                        ok = false;
                        break;
                    }
                    let o = &objs[obj as usize];
                    let (clip, n) = vs_expect(o, &cam, v);
                    let good = close(&vm.output_builtin(Bi::Position), &clip, 1e-4) && close(&vm.output(0), &n, 1e-5)
                        && vm.output(1) == words(&o.color);
                    if !good && ok {
                        crate::out(&format!("   {} {} object {} vertex {}: got {:?}, want {:?}\n", sh.name(), lv, obj, v,
                                            vm.output_builtin(Bi::Position).iter().map(|&x| f(x)).collect::<Vec<_>>(), clip));
                    }
                    ok &= good;
                }
            }
            t.check(&format!("{} {} gl_Position, normal, colour", sh.name(), lv), ok);
        }
        // fs_lambert
        let w = Shader::FsLambert.build(minor);
        let mut vm = Vm::new(&w);
        vm.buffer(bench::B_CAMERA, cam.bytes());
        let mut ok = true;
        for (n, c) in [([0.0f32, 2.0, 0.0], [1.0f32, 0.5, 0.25]), ([1.0, -1.0, 0.5], [0.3, 0.6, 0.9]), ([-0.3, 0.8, 0.5], [1.0, 1.0, 1.0])] {
            vm.set_input(0, &words(&n));
            vm.set_input(1, &words(&c));
            ok &= vm.run().is_ok();
            let l = cam.light_dir;
            let len = ((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]) as f64).sqrt_f64();
            let ndl = ((n[0] * l[0] + n[1] * l[1] + n[2] * l[2]) as f64 / len).max(0.0);
            let k = cam.ambient as f64 + (1.0 - cam.ambient as f64) * ndl;
            let want = [c[0] as f64 * k, c[1] as f64 * k, c[2] as f64 * k, 1.0];
            ok &= close(&vm.output(0), &want, 1e-5);
        }
        t.check(&format!("fs_lambert {} colour", lv), ok);
    }
}

trait SqrtF64 {
    fn sqrt_f64(self) -> f64;
}

impl SqrtF64 for f64 {
    fn sqrt_f64(self) -> f64 {
        rt::fx::sqrt(self as f32) as f64
    }
}

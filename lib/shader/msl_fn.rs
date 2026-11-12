// msl_fn -- the function bodies of the MSL translation (msl.rs): structured control flow back
// into if / switch / while (true), and every instruction as a statement.
//
// Values: each SSA result is a function-scope variable v<id> (declared at the top, assigned
// where SPIR-V defines it).  Pointers stay lvalue expressions (g<id> globals, l<id> function
// variables, p<id> parameters, with .m<k> and [i] for access chains), images and samplers
// their names.
//
// Control flow: region(b, stop) emits blocks from b until stop.  A branch is classified
// against the enclosing constructs: the innermost loop's header (back edge: continue), its
// continue target (the continue construct is emitted in place, then continue), its merge
// (break), the innermost switch's merge (break), the region's stop, or the next block.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::module::Ty;
use crate::msl::{Merge, M};
use crate::spv::{self, op, Bi, Dec, Sc};

#[derive(Clone, Debug)]
enum Ex {
    Val(String),
    /// lvalue, storage class, pointee type
    Ptr(String, u32, u32),
    /// name, image type
    Img(String, u32),
    Smp(String),
    /// image, sampler, image type
    SImg(String, String, u32),
}

#[derive(Clone, Copy)]
enum Cons {
    Loop { header: u32, merge: u32, cont: u32 },
    Switch { merge: u32 },
}

enum Target {
    Stop,
    Seq(u32),
    Jump(&'static str),
    /// the continue construct of the loop with this header
    Cont(u32),
}

enum Next {
    Seq(u32),
    Stop,
    Done,
}

/// a phi: its result and (value, predecessor) pairs
type Phi = (u32, Vec<(u32, u32)>);

struct Fx<'m, 'a> {
    m: &'m M<'a>,
    f: &'m crate::msl::Func,
    ex: BTreeMap<u32, Ex>,
    decl: BTreeMap<u32, String>,
    out: String,
    ind: usize,
    stack: Vec<Cons>,
    ret: String,
    conv: &'m mut BTreeSet<(u32, u32)>,
    /// per block: its phis (result, [(value, predecessor)])
    phis: BTreeMap<u32, Vec<Phi>>,
    depth: u32,
    /// pointers to a buffer of a bindless table: (table variable, the index expression), for arrayLength
    tab: BTreeMap<u32, (u32, String)>,
}

const LETTERS: [&str; 4] = ["x", "y", "z", "w"];

fn err<T>(s: impl Into<String>) -> Result<T, String> {
    Err(s.into())
}

/// a function other than an entry point: (prototype, body)
pub(crate) fn function(m: &M, f: u32, conv: &mut BTreeSet<(u32, u32)>) -> Result<(String, String), String> {
    let func = &m.funcs[&f];
    let mut params = Vec::new();
    let mut ex = BTreeMap::new();
    for &(p, t) in &func.params {
        match m.ty(t)?.clone() {
            Ty::Pointer { sc, pointee } => {
                params.push(format!("{} {}& p{}", M::space(sc), m.tname(pointee.0)?, p));
                ex.insert(p, Ex::Ptr(format!("p{}", p), sc, pointee.0));
            }
            Ty::Image { .. } => {
                params.push(format!("{} p{}", m.tname(t)?, p));
                ex.insert(p, Ex::Img(format!("p{}", p), t));
            }
            Ty::Sampler => {
                params.push(format!("sampler p{}", p));
                ex.insert(p, Ex::Smp(format!("p{}", p)));
            }
            _ => {
                params.push(format!("{} p{}", m.tname(t)?, p));
                ex.insert(p, Ex::Val(format!("p{}", p)));
            }
        }
    }
    for &g in m.uses.get(&f).into_iter().flatten() {
        if m.deco(g, Dec::BuiltIn).and_then(|x| x.first().copied()) == Some(Bi::HelperInvocation as u32) {
            continue;
        }
        params.push(m.global_param(g)?);
    }
    if m.sizes.contains(&f) {
        params.push(String::from("constant uint* jf_sizes"));
    }
    let proto = format!("static {} f{}({})", m.tname(func.ret)?, f, params.join(", "));
    let ret = if matches!(m.ty(func.ret)?, Ty::Void) { String::from("return;") } else { format!("return {}{{}};", m.tname(func.ret)?) };
    let body = body(m, f, ex, ret, conv)?;
    Ok((proto, format!("{{\n{}}}\n", body)))
}

/// the statements of an entry point's function (its prologue comes from msl.rs)
pub(crate) fn entry_body(m: &M, f: u32, ret: &str, conv: &mut BTreeSet<(u32, u32)>) -> Result<String, String> {
    body(m, f, BTreeMap::new(), String::from(ret), conv)
}

fn body(m: &M, f: u32, ex: BTreeMap<u32, Ex>, ret: String, conv: &mut BTreeSet<(u32, u32)>) -> Result<String, String> {
    let func = &m.funcs[&f];
    let Some(first) = func.blocks.first() else { return err("a function without blocks") };
    let mut fx = Fx { m, f: func, ex, decl: BTreeMap::new(), out: String::new(), ind: 1, stack: Vec::new(), ret, conv, phis: BTreeMap::new(), depth: 0, tab: BTreeMap::new() };
    for b in &func.blocks {
        for k in b.first..b.end {
            let i = &m.ins[k];
            if i.op == op::Phi {
                let pairs = i.ops.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect();
                fx.phis.entry(b.label).or_default().push((i.res, pairs));
                fx.decl.insert(i.res, m.tname(i.ty)?);
                fx.ex.insert(i.res, Ex::Val(format!("v{}", i.res)));
            }
        }
    }
    fx.region(first.label, None)?;
    let mut s = String::new();
    for (id, t) in &fx.decl {
        s.push_str(&format!("    {} v{};\n", t, id));
    }
    s.push_str(&fx.out);
    Ok(s)
}

impl Fx<'_, '_> {
    fn line(&mut self, s: &str) {
        for _ in 0..self.ind {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn block(&self, label: u32) -> Result<usize, String> {
        self.f.at.get(&label).copied().ok_or_else(|| format!("branch to %{}, no block", label))
    }

    // ------------------------------------------------------------ control flow

    fn region(&mut self, start: u32, stop: Option<u32>) -> Result<bool, String> {
        self.depth += 1;
        if self.depth > 200 {
            return err("control flow nested too deeply");
        }
        let r = self.region_inner(start, stop);
        self.depth -= 1;
        r
    }

    fn region_inner(&mut self, start: u32, stop: Option<u32>) -> Result<bool, String> {
        let mut b = start;
        let mut steps = 0;
        loop {
            steps += 1;
            if steps > 100_000 {
                return err("control flow does not end");
            }
            if Some(b) == stop {
                return Ok(true);
            }
            let bi = self.block(b)?;
            let blk = &self.f.blocks[bi];
            if blk.first == blk.term && self.m.ins[blk.term].op == op::Unreachable {
                return Ok(false);
            }
            if let Merge::Loop(merge, cont) = blk.merge {
                self.line("while (true) {");
                self.ind += 1;
                self.stack.push(Cons::Loop { header: b, merge, cont });
                self.insts(bi)?;
                if let Next::Seq(t) = self.term(bi, None)? {
                    self.region(t, None)?;
                }
                self.stack.pop();
                self.ind -= 1;
                self.line("}");
                match self.after(merge, stop)? {
                    Next::Seq(t) => {
                        b = t;
                        continue;
                    }
                    Next::Stop => return Ok(true),
                    Next::Done => return Ok(false),
                }
            }
            self.insts(bi)?;
            match self.term(bi, stop)? {
                Next::Seq(t) => b = t,
                Next::Stop => return Ok(true),
                Next::Done => return Ok(false),
            }
        }
    }

    fn classify(&self, t: u32, stop: Option<u32>) -> Result<Target, String> {
        if Some(t) == stop {
            return Ok(Target::Stop);
        }
        let mut switch = false;
        for c in self.stack.iter().rev() {
            match *c {
                Cons::Loop { header, merge, cont } => {
                    if t == header {
                        return Ok(Target::Jump("continue;"));
                    }
                    if t == cont {
                        return Ok(Target::Cont(header));
                    }
                    if t == merge {
                        if switch {
                            return err("a break out of a loop from inside a switch");
                        }
                        return Ok(Target::Jump("break;"));
                    }
                    break;
                }
                Cons::Switch { merge } => {
                    if t == merge {
                        return Ok(Target::Jump("break;"));
                    }
                    switch = true;
                }
            }
        }
        Ok(Target::Seq(t))
    }

    /// the phi copies of an edge
    fn edge(&mut self, from: u32, to: u32) -> Result<(), String> {
        let Some(ps) = self.phis.get(&to).cloned() else { return Ok(()) };
        for (res, pairs) in ps {
            if let Some(&(v, _)) = pairs.iter().find(|p| p.1 == from) {
                let e = self.v(v)?;
                self.line(&format!("v{} = {};", res, e));
            }
        }
        Ok(())
    }

    /// take a branch from block `from` to t: emit what jumps, say what follows
    fn go(&mut self, from: u32, t: u32, stop: Option<u32>) -> Result<Next, String> {
        self.edge(from, t)?;
        self.after(t, stop)
    }

    fn after(&mut self, t: u32, stop: Option<u32>) -> Result<Next, String> {
        match self.classify(t, stop)? {
            Target::Stop => Ok(Next::Stop),
            Target::Seq(t) => Ok(Next::Seq(t)),
            Target::Jump(s) => {
                self.line(s);
                Ok(Next::Done)
            }
            Target::Cont(h) => {
                self.cont(h)?;
                Ok(Next::Done)
            }
        }
    }

    /// the continue construct of the loop with header h, in place
    fn cont(&mut self, h: u32) -> Result<(), String> {
        let Some(&Cons::Loop { cont, .. }) = self.stack.iter().rev().find(|c| matches!(c, Cons::Loop { header, .. } if *header == h)) else {
            return err("continue outside its loop");
        };
        // inside, the branch back to the header is `continue;`
        let saved: Vec<Cons> = self.stack.clone();
        while let Some(c) = self.stack.last() {
            if matches!(c, Cons::Loop { header, .. } if *header == h) {
                break;
            }
            self.stack.pop();
        }
        // the continue construct's own branch to the continue target cannot occur; mark it as
        // a plain block by replacing cont with the header in this loop's entry
        if let Some(Cons::Loop { cont: c, header, .. }) = self.stack.last_mut() {
            *c = *header;
        }
        let r = self.region(cont, None);
        self.stack = saved;
        r.map(|_| ())
    }

    /// a branch target inside a selection with merge m
    fn arm(&mut self, from: u32, t: u32, m: u32) -> Result<(), String> {
        if t == m {
            return self.edge(from, t);
        }
        match self.go(from, t, Some(m))? {
            Next::Seq(t) => {
                self.region(t, Some(m))?;
            }
            Next::Stop | Next::Done => {}
        }
        Ok(())
    }

    fn term(&mut self, bi: usize, stop: Option<u32>) -> Result<Next, String> {
        let blk = &self.f.blocks[bi];
        let (label, merge) = (blk.label, blk.merge);
        let i = self.m.ins[blk.term];
        let o = i.ops;
        match i.op {
            op::Branch => self.go(label, o[0], stop),
            op::BranchConditional => {
                let c = self.v(o[0])?;
                let (t, f) = (o[1], o[2]);
                if let Merge::Sel(m) = merge {
                    if t == m && f == m {
                        self.edge(label, m)?;
                    } else if t == m {
                        self.line(&format!("if (!({})) {{", c));
                        self.ind += 1;
                        self.arm(label, f, m)?;
                        self.ind -= 1;
                        self.line("}");
                        self.edge(label, m)?;
                    } else {
                        self.line(&format!("if ({}) {{", c));
                        self.ind += 1;
                        self.arm(label, t, m)?;
                        self.ind -= 1;
                        if f != m {
                            self.line("} else {");
                            self.ind += 1;
                            self.arm(label, f, m)?;
                            self.ind -= 1;
                        }
                        self.line("}");
                        if f == m {
                            self.edge(label, m)?;
                        }
                    }
                    return self.after(m, stop);
                }
                // no selection: a conditional break / continue / back edge (or a loop header's
                // test); at most one arm goes on
                let ct = self.classify(t, stop)?;
                let cf = self.classify(f, stop)?;
                let falls = |x: &Target| matches!(x, Target::Stop | Target::Seq(_));
                match (falls(&ct), falls(&cf)) {
                    (false, false) => {
                        self.line(&format!("if ({}) {{", c));
                        self.ind += 1;
                        self.go(label, t, stop)?;
                        self.ind -= 1;
                        self.line("} else {");
                        self.ind += 1;
                        self.go(label, f, stop)?;
                        self.ind -= 1;
                        self.line("}");
                        Ok(Next::Done)
                    }
                    (false, true) => {
                        self.line(&format!("if ({}) {{", c));
                        self.ind += 1;
                        self.go(label, t, stop)?;
                        self.ind -= 1;
                        self.line("}");
                        self.go(label, f, stop)
                    }
                    (true, false) => {
                        self.line(&format!("if (!({})) {{", c));
                        self.ind += 1;
                        self.go(label, f, stop)?;
                        self.ind -= 1;
                        self.line("}");
                        self.go(label, t, stop)
                    }
                    (true, true) => {
                        if t == f {
                            self.go(label, t, stop)
                        } else {
                            err("a conditional branch to two blocks without a selection construct")
                        }
                    }
                }
            }
            op::Switch => {
                let Merge::Sel(m) = merge else { return err("OpSwitch without OpSelectionMerge") };
                let sel_t = self.m.rty.get(&o[0]).copied().unwrap_or(0);
                let signed = matches!(self.m.ty(sel_t), Ok(Ty::Int { signed: true, .. }));
                if matches!(self.m.ty(sel_t), Ok(Ty::Int { width: 64, .. })) {
                    return err("a 64-bit switch selector");
                }
                let sel = self.v(o[0])?;
                let default = o[1];
                // targets in first appearance order with their literals
                let mut targets: Vec<(u32, Vec<String>)> = Vec::new();
                for pair in o[2..].chunks(2) {
                    if pair.len() < 2 {
                        continue;
                    }
                    let lit = if signed { format!("{}", pair[0] as i32) } else { format!("{}u", pair[0]) };
                    match targets.iter_mut().find(|x| x.0 == pair[1]) {
                        Some(x) => x.1.push(lit),
                        None => targets.push((pair[1], alloc::vec![lit])),
                    }
                }
                self.line(&format!("switch ({}) {{", sel));
                self.ind += 1;
                self.stack.push(Cons::Switch { merge: m });
                let mut default_done = false;
                for (t, lits) in targets {
                    let mut head: String = lits.iter().map(|l| format!("case {}: ", l)).collect();
                    if t == default {
                        head.push_str("default: ");
                        default_done = true;
                    }
                    self.line(&format!("{}{{", head));
                    self.ind += 1;
                    self.arm(label, t, m)?;
                    self.line("break;");
                    self.ind -= 1;
                    self.line("}");
                }
                if !default_done && default != m {
                    self.line("default: {");
                    self.ind += 1;
                    self.arm(label, default, m)?;
                    self.line("break;");
                    self.ind -= 1;
                    self.line("}");
                }
                self.stack.pop();
                self.ind -= 1;
                self.line("}");
                self.after(m, stop)
            }
            op::Return => {
                let r = self.ret.clone();
                self.line(&r);
                Ok(Next::Done)
            }
            op::ReturnValue => {
                let v = self.v(o[0])?;
                self.line(&format!("return {};", v));
                Ok(Next::Done)
            }
            op::Kill | op::TerminateInvocation => {
                self.line("discard_fragment();");
                let r = self.ret.clone();
                self.line(&r);
                Ok(Next::Done)
            }
            op::Unreachable => Ok(Next::Done),
            _ => Err(format!("terminator Op{}", spv::op_info(i.op).map_or("?", |x| x.name))),
        }
    }

    // ------------------------------------------------------------ values

    fn v(&self, id: u32) -> Result<String, String> {
        match self.ex.get(&id) {
            Some(Ex::Val(s)) => Ok(s.clone()),
            Some(Ex::Img(s, _)) | Some(Ex::Smp(s)) => Ok(s.clone()),
            Some(Ex::Ptr(..)) => Err(format!("pointer %{} used as a value", id)),
            Some(Ex::SImg(..)) => Err(format!("sampled image %{} used as a value", id)),
            None => {
                if self.m.is_const(id) || self.m.undef.contains(&id) {
                    self.m.cexpr(id)
                } else {
                    Err(format!("%{} used before its definition", id))
                }
            }
        }
    }

    fn p(&self, id: u32) -> Result<(String, u32, u32), String> {
        if let Some(Ex::Ptr(s, sc, t)) = self.ex.get(&id) {
            return Ok((s.clone(), *sc, *t));
        }
        if let Some(v) = self.m.vars.get(&id) {
            return Ok((format!("g{}", id), v.sc, v.ty));
        }
        Err(format!("%{} is no pointer", id))
    }

    fn rt(&self, id: u32) -> u32 {
        self.m.rty.get(&id).or_else(|| self.m.cty.get(&id)).copied().unwrap_or(0)
    }

    fn tn(&self, t: u32) -> Result<String, String> {
        self.m.tname(t)
    }

    fn set(&mut self, r: u32, t: u32, e: String) -> Result<(), String> {
        self.decl.insert(r, self.tn(t)?);
        self.line(&format!("v{} = {};", r, e));
        self.ex.insert(r, Ex::Val(format!("v{}", r)));
        Ok(())
    }

    /// value id as type `want` (an MSL name): as_type when the names differ
    fn co(&self, id: u32, want: &str) -> Result<String, String> {
        let have = self.tn(self.rt(id))?;
        let v = self.v(id)?;
        Ok(if have == want { v } else { format!("as_type<{}>({})", want, v) })
    }

    /// the integer type of t with the signedness given (an MSL name)
    fn int_as(&self, t: u32, signed: bool) -> Result<String, String> {
        let (s, n) = self.m.num(t)?;
        match s {
            Ty::Int { width, .. } => M::num_name(&Ty::Int { width, signed }, n),
            _ => Err(format!("%{} is no integer type", t)),
        }
    }

    fn comps(&self, t: u32) -> u32 {
        self.m.num(t).map(|x| x.1).unwrap_or(1)
    }

    /// componentwise: name(f(0), f(1), ..) for vectors, f(0) with ".x" removed for scalars
    fn cw(&self, t: u32, f: impl Fn(&str) -> String) -> Result<String, String> {
        let n = self.comps(t);
        if n == 1 {
            return Ok(f(""));
        }
        let parts: Vec<String> = (0..n as usize).map(|k| f(&format!(".{}", LETTERS[k]))).collect();
        Ok(format!("{}({})", self.tn(t)?, parts.join(", ")))
    }

    fn atomic_ptr(&self, ptr: u32, t: u32) -> Result<String, String> {
        let (lv, sc, _) = self.p(ptr)?;
        let space = if sc == Sc::Workgroup as u32 { "threadgroup" } else { "device" };
        let at = match self.m.ty(t)? {
            Ty::Int { width: 32, signed: true } => "atomic_int",
            Ty::Int { width: 32, signed: false } => "atomic_uint",
            Ty::Int { width: 64, signed: false } => "atomic_ulong",
            Ty::Float { width: 32 } => "atomic_float",
            _ => return err("an atomic of this type"),
        };
        Ok(format!("(({} {}*)&({}))", space, at, lv))
    }

    fn mem_flags(&self, sem: u32) -> Result<String, String> {
        let s = self.m.cu32(sem)?;
        let mut f = Vec::new();
        if s & spv::sem::WorkgroupMemory != 0 {
            f.push("mem_flags::mem_threadgroup");
        }
        if s & (spv::sem::UniformMemory | spv::sem::CrossWorkgroupMemory) != 0 {
            f.push("mem_flags::mem_device");
        }
        if s & spv::sem::ImageMemory != 0 {
            f.push("mem_flags::mem_texture");
        }
        Ok(if f.is_empty() { String::from("mem_flags::mem_none") } else { f.join(" | ") })
    }

    // ------------------------------------------------------------ instructions

    fn insts(&mut self, bi: usize) -> Result<(), String> {
        let (first, end) = (self.f.blocks[bi].first, self.f.blocks[bi].end);
        for k in first..end {
            self.inst(k)?;
        }
        Ok(())
    }

    fn inst(&mut self, k: usize) -> Result<(), String> {
        let i = self.m.ins[k];
        let (t, r, o) = (i.ty, i.res, i.ops);
        match i.op {
            op::Nop | op::Line | op::Name | op::MemberName | op::Phi | op::Source | op::String | op::ModuleProcessed => {}
            op::Undef => {
                self.ex.insert(r, Ex::Val(format!("{}{{}}", self.tn(t)?)));
            }
            op::Variable => {
                let Ty::Pointer { pointee, sc } = self.m.ty(t)?.clone() else { return err("a variable without pointer type") };
                if sc != Sc::Function as u32 {
                    return err("a variable of another storage class than Function in a function");
                }
                let init = match o.get(1) {
                    Some(&c) => self.v(c)?,
                    None => String::from("{}"),
                };
                let tn = self.tn(pointee.0)?;
                self.line(&format!("{} l{} = {};", tn, r, init));
                self.ex.insert(r, Ex::Ptr(format!("l{}", r), sc, pointee.0));
            }
            op::Load => {
                let pid = o[0];
                if let Some(var) = self.m.vars.get(&pid) {
                    if var.sc == Sc::UniformConstant as u32 {
                        match self.m.ty(var.ty)? {
                            Ty::Image { .. } => {
                                self.ex.insert(r, Ex::Img(format!("g{}", pid), var.ty));
                            }
                            Ty::Sampler => {
                                self.ex.insert(r, Ex::Smp(format!("g{}", pid)));
                            }
                            _ => return err("a UniformConstant variable of an unsupported type"),
                        }
                        return Ok(());
                    }
                    if self.m.deco(pid, Dec::BuiltIn).and_then(|x| x.first().copied()) == Some(Bi::HelperInvocation as u32) {
                        return self.set(r, t, String::from("simd_is_helper_thread()"));
                    }
                }
                if let Some(e @ (Ex::Img(..) | Ex::Smp(_))) = self.ex.get(&pid).cloned() {
                    self.ex.insert(r, e);
                    return Ok(());
                }
                let (lv, ..) = self.p(pid)?;
                self.set(r, t, lv)?;
            }
            op::Store => {
                let (lv, ..) = self.p(o[0])?;
                let v = self.v(o[1])?;
                self.line(&format!("{} = {};", lv, v));
            }
            op::CopyMemory => {
                let (d, ..) = self.p(o[0])?;
                let (s, ..) = self.p(o[1])?;
                self.line(&format!("{} = {};", d, s));
            }
            op::AccessChain | op::InBoundsAccessChain => {
                // a bindless table: its element through the argument buffer (msl.rs Table)
                let mut rest = &o[1..];
                let start = match self.m.tables.get(&o[0]).copied() {
                    Some(tb) => {
                        let Some((&first, more)) = rest.split_first() else { return err("an access chain to a whole bindless table") };
                        let i = self.v(first)?;
                        rest = more;
                        match tb {
                            crate::msl::Table::Tex(_) | crate::msl::Table::Smp if !more.is_empty() => {
                                return err("an access chain into an image or sampler");
                            }
                            crate::msl::Table::Tex(it) => {
                                self.ex.insert(r, Ex::Img(format!("g{}[{}].t", o[0], i), it));
                                return Ok(());
                            }
                            crate::msl::Table::Smp => {
                                self.ex.insert(r, Ex::Smp(format!("g{}[{}].s", o[0], i)));
                                return Ok(());
                            }
                            crate::msl::Table::Buf(st) => {
                                if more.is_empty() {
                                    self.tab.insert(r, (o[0], i.clone()));
                                }
                                Some((format!("(*g{}[{}].p)", o[0], i), self.m.vars[&o[0]].sc, st))
                            }
                        }
                    }
                    None => None,
                };
                let (mut lv, sc, mut cur) = match start {
                    Some(s) => s,
                    None => self.p(o[0])?,
                };
                for &idx in rest {
                    match self.m.ty(cur)?.clone() {
                        Ty::Struct { members } => {
                            let k = self.m.cu32(idx)? as usize;
                            let Some(mt) = members.get(k) else { return err("an access chain past a struct's members") };
                            lv = format!("{}.m{}", lv, k);
                            cur = mt.0;
                        }
                        Ty::Array { elem, .. } | Ty::RuntimeArray { elem } => {
                            lv = format!("{}[{}]", lv, self.v(idx)?);
                            cur = elem.0;
                        }
                        Ty::Vector { elem, .. } => {
                            lv = format!("{}[{}]", lv, self.v(idx)?);
                            cur = elem.0;
                        }
                        Ty::Matrix { col, .. } => {
                            lv = format!("{}[{}]", lv, self.v(idx)?);
                            cur = col.0;
                        }
                        _ => return err("an access chain into a scalar"),
                    }
                }
                self.ex.insert(r, Ex::Ptr(lv, sc, cur));
            }
            op::ArrayLength => {
                let g = o[0];
                // (a buffer of a bindless table: its size comes with its address)
                if let Some((tv, i)) = self.tab.get(&g).cloned() {
                    let Some(crate::msl::Table::Buf(st)) = self.m.tables.get(&tv).copied() else { return err("arrayLength of no buffer") };
                    let member = o[1];
                    let off = self.m.mdeco(st, member, Dec::Offset).and_then(|x| x.first().copied()).unwrap_or(0);
                    let Ty::Struct { members } = self.m.ty(st)?.clone() else { return err("arrayLength of no struct") };
                    let at = members.get(member as usize).ok_or("arrayLength of a missing member")?.0;
                    let stride = self.m.deco(at, Dec::ArrayStride).and_then(|x| x.first().copied()).ok_or("a runtime array without ArrayStride")?;
                    let n = format!("g{}[{}].n", tv, i);
                    return self.set(r, t, format!("(({} - min({}, {}u)) / {}u)", n, n, off, stride));
                }
                let Some(var) = self.m.vars.get(&g) else { return err("arrayLength of a pointer that is no buffer variable") };
                let slot = self.m.buffer_slot(g)?;
                let member = o[1];
                let off = self.m.mdeco(var.ty, member, Dec::Offset).and_then(|x| x.first().copied()).unwrap_or(0);
                let Ty::Struct { members } = self.m.ty(var.ty)?.clone() else { return err("arrayLength of no struct") };
                let at = members.get(member as usize).ok_or("arrayLength of a missing member")?.0;
                let stride = self.m.deco(at, Dec::ArrayStride).and_then(|x| x.first().copied()).ok_or("a runtime array without ArrayStride")?;
                self.set(r, t, format!("((jf_sizes[{}] - min(jf_sizes[{}], {}u)) / {}u)", slot, slot, off, stride))?;
            }
            op::CompositeConstruct => {
                let tn = self.tn(t)?;
                match self.m.ty(t)?.clone() {
                    Ty::Vector { .. } | Ty::Matrix { .. } => {
                        let parts: Result<Vec<String>, String> = o.iter().map(|&x| self.v(x)).collect();
                        self.set(r, t, format!("{}({})", tn, parts?.join(", ")))?;
                    }
                    Ty::Array { .. } => {
                        let parts: Result<Vec<String>, String> = o.iter().map(|&x| self.v(x)).collect();
                        self.set(r, t, format!("{}{{{{{}}}}}", tn, parts?.join(", ")))?;
                    }
                    Ty::Struct { .. } => {
                        self.set(r, t, format!("{}{{}}", tn))?;
                        for (k, &x) in o.iter().enumerate() {
                            let v = self.v(x)?;
                            self.line(&format!("v{}.m{} = {};", r, k, v));
                        }
                    }
                    _ => return err("OpCompositeConstruct of a scalar"),
                }
            }
            op::CompositeExtract => {
                let mut e = self.v(o[0])?;
                let mut cur = self.rt(o[0]);
                for &idx in &o[1..] {
                    match self.m.ty(cur)?.clone() {
                        Ty::Struct { members } => {
                            e = format!("{}.m{}", e, idx);
                            cur = members.get(idx as usize).ok_or("extract past a struct's members")?.0;
                        }
                        Ty::Array { elem, .. } => {
                            e = format!("{}[{}]", e, idx);
                            cur = elem.0;
                        }
                        Ty::Vector { elem, .. } => {
                            e = format!("{}.{}", e, LETTERS.get(idx as usize).ok_or("a vector component past w")?);
                            cur = elem.0;
                        }
                        Ty::Matrix { col, .. } => {
                            e = format!("{}[{}]", e, idx);
                            cur = col.0;
                        }
                        _ => return err("extract from a scalar"),
                    }
                }
                self.set(r, t, e)?;
            }
            op::CompositeInsert => {
                let obj = self.v(o[0])?;
                let comp = self.v(o[1])?;
                self.set(r, t, comp)?;
                let mut path = String::new();
                let mut cur = t;
                for &idx in &o[2..] {
                    match self.m.ty(cur)?.clone() {
                        Ty::Struct { members } => {
                            path.push_str(&format!(".m{}", idx));
                            cur = members.get(idx as usize).ok_or("insert past a struct's members")?.0;
                        }
                        Ty::Array { elem, .. } => {
                            path.push_str(&format!("[{}]", idx));
                            cur = elem.0;
                        }
                        Ty::Vector { elem, .. } => {
                            path.push_str(&format!(".{}", LETTERS.get(idx as usize).ok_or("a vector component past w")?));
                            cur = elem.0;
                        }
                        Ty::Matrix { col, .. } => {
                            path.push_str(&format!("[{}]", idx));
                            cur = col.0;
                        }
                        _ => return err("insert into a scalar"),
                    }
                }
                self.line(&format!("v{}{} = {};", r, path, obj));
            }
            op::CopyObject => match self.ex.get(&o[0]).cloned() {
                Some(e @ (Ex::Ptr(..) | Ex::Img(..) | Ex::Smp(_) | Ex::SImg(..))) => {
                    self.ex.insert(r, e);
                }
                _ => {
                    if self.m.vars.contains_key(&o[0]) {
                        let (lv, sc, pt) = self.p(o[0])?;
                        self.ex.insert(r, Ex::Ptr(lv, sc, pt));
                    } else {
                        let v = self.v(o[0])?;
                        self.set(r, t, v)?;
                    }
                }
            },
            op::CopyLogical => {
                let src = self.rt(o[0]);
                self.m.need_conv(src, t, self.conv)?;
                let e = self.m.conv_expr(src, t, &self.v(o[0])?)?;
                self.set(r, t, e)?;
            }
            op::VectorShuffle => {
                let (a, b) = (self.v(o[0])?, self.v(o[1])?);
                let na = self.comps(self.rt(o[0]));
                let mut parts = Vec::new();
                for &c in &o[2..] {
                    parts.push(if c == 0xffff_ffff {
                        format!("{}.x", a)
                    } else if c < na {
                        format!("{}.{}", a, LETTERS[c as usize])
                    } else {
                        format!("{}.{}", b, LETTERS.get((c - na) as usize).ok_or("a shuffle component past w")?)
                    });
                }
                let e = if parts.len() == 1 { parts.remove(0) } else { format!("{}({})", self.tn(t)?, parts.join(", ")) };
                self.set(r, t, e)?;
            }
            op::VectorExtractDynamic => {
                let e = format!("{}[{}]", self.v(o[0])?, self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::VectorInsertDynamic => {
                let (vec, comp, idx) = (self.v(o[0])?, self.v(o[1])?, self.v(o[2])?);
                self.set(r, t, vec)?;
                self.line(&format!("v{}[{}] = {};", r, idx, comp));
            }
            op::SampledImage => {
                let (img, it) = match self.ex.get(&o[0]) {
                    Some(Ex::Img(s, t)) => (s.clone(), *t),
                    _ => return err("OpSampledImage of no image"),
                };
                let smp = match self.ex.get(&o[1]) {
                    Some(Ex::Smp(s)) => s.clone(),
                    _ => return err("OpSampledImage of no sampler"),
                };
                self.ex.insert(r, Ex::SImg(img, smp, it));
            }
            op::Image => match self.ex.get(&o[0]).cloned() {
                Some(Ex::SImg(img, _, it)) => {
                    self.ex.insert(r, Ex::Img(img, it));
                }
                _ => return err("OpImage of no sampled image"),
            },
            op::ImageSampleImplicitLod | op::ImageSampleExplicitLod | op::ImageSampleDrefImplicitLod | op::ImageSampleDrefExplicitLod
            | op::ImageFetch | op::ImageGather | op::ImageDrefGather | op::ImageRead | op::ImageQuerySizeLod | op::ImageQuerySize
            | op::ImageQueryLevels | op::ImageQuerySamples | op::ImageQueryLod => {
                let e = self.image(i.op, t, o)?;
                self.set(r, t, e)?;
            }
            op::ImageWrite => {
                let (img, it) = self.img(o[0])?;
                let (dim, arrayed, sampled) = self.img_info(it)?;
                let (c, layer) = self.int_coord(o[1], dim, arrayed)?;
                let want = format!("{}4", sampled);
                let texel = self.co_value(o[2], &want)?;
                // (Metal's write() outside the image is undefined: such a store is dropped, as WGSL allows)
                let size = self.image_size(&img, dim, "");
                let mut inside = if Self::base_comps(dim) == 1 { format!("{} < {}", c, size) } else { format!("all({} < {})", c, size) };
                if let Some(l) = &layer {
                    inside.push_str(&format!(" && {} < {}.get_array_size()", l, img));
                }
                let layer = layer.map(|l| format!(", {}", l)).unwrap_or_default();
                self.line(&format!("if ({}) {}.write({}, {}{});", inside, img, texel, c, layer));
            }
            op::ConvertFToU | op::ConvertFToS | op::ConvertSToF | op::ConvertUToF | op::UConvert | op::SConvert | op::FConvert => {
                let src = self.rt(o[0]);
                let tn = self.tn(t)?;
                let a = match i.op {
                    op::ConvertSToF | op::SConvert => self.co(o[0], &self.int_as(src, true)?)?,
                    op::ConvertUToF | op::UConvert => self.co(o[0], &self.int_as(src, false)?)?,
                    _ => self.v(o[0])?,
                };
                let e = match i.op {
                    op::ConvertFToS => {
                        let s = self.int_as(t, true)?;
                        if s == tn { format!("{}({})", tn, a) } else { format!("as_type<{}>({}({}))", tn, s, a) }
                    }
                    op::ConvertFToU => {
                        let u = self.int_as(t, false)?;
                        if u == tn { format!("{}({})", tn, a) } else { format!("as_type<{}>({}({}))", tn, u, a) }
                    }
                    _ => format!("{}({})", tn, a),
                };
                self.set(r, t, e)?;
            }
            op::QuantizeToF16 => {
                let n = self.comps(t);
                let h = if n == 1 { String::from("half") } else { format!("half{}", n) };
                let e = format!("{}({}({}))", self.tn(t)?, h, self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::Bitcast => {
                let e = format!("as_type<{}>({})", self.tn(t)?, self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::SNegate => {
                let (tn, u) = (self.tn(t)?, self.int_as(t, false)?);
                let e = format!("as_type<{}>({}({}(0) - {}))", tn, u, u, self.co(o[0], &u)?);
                self.set(r, t, e)?;
            }
            op::FNegate => {
                let e = format!("(-{})", self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::Not => {
                let tn = self.tn(t)?;
                let e = format!("{}(~{})", tn, self.co(o[0], &tn)?);
                self.set(r, t, e)?;
            }
            op::IAdd | op::ISub | op::IMul | op::BitwiseOr | op::BitwiseXor | op::BitwiseAnd => {
                let sym = match i.op {
                    op::IAdd => "+",
                    op::ISub => "-",
                    op::IMul => "*",
                    op::BitwiseOr => "|",
                    op::BitwiseXor => "^",
                    _ => "&",
                };
                let (tn, u) = (self.tn(t)?, self.int_as(t, false)?);
                let e = format!("{}({} {} {})", u, self.co(o[0], &u)?, sym, self.co(o[1], &u)?);
                self.set(r, t, if tn == u { e } else { format!("as_type<{}>({})", tn, e) })?;
            }
            op::UDiv | op::UMod | op::SDiv | op::SRem | op::SMod => {
                let signed = matches!(i.op, op::SDiv | op::SRem | op::SMod);
                let (tn, w) = (self.tn(t)?, self.int_as(t, signed)?);
                let (a, b) = (self.co(o[0], &w)?, self.co(o[1], &w)?);
                let e = match i.op {
                    op::UDiv | op::SDiv => format!("{}({} / {})", w, a, b),
                    op::SMod => format!("jf_smod({}, {})", a, b),
                    _ => format!("{}({} % {})", w, a, b),
                };
                self.set(r, t, if tn == w { e } else { format!("as_type<{}>({})", tn, e) })?;
            }
            op::FAdd | op::FSub | op::FMul | op::FDiv | op::VectorTimesScalar | op::MatrixTimesScalar | op::VectorTimesMatrix
            | op::MatrixTimesVector | op::MatrixTimesMatrix => {
                let sym = match i.op {
                    op::FAdd => "+",
                    op::FSub => "-",
                    op::FDiv => "/",
                    _ => "*",
                };
                let e = format!("({} {} {})", self.v(o[0])?, sym, self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::FRem => {
                let e = format!("fmod({}, {})", self.v(o[0])?, self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::FMod => {
                let e = format!("jf_fmod({}, {})", self.v(o[0])?, self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::OuterProduct => {
                let (a, b) = (self.v(o[0])?, self.v(o[1])?);
                let nb = self.comps(self.rt(o[1]));
                let cols: Vec<String> = (0..nb as usize).map(|k| format!("{} * {}.{}", a, b, LETTERS[k])).collect();
                let e = format!("{}({})", self.tn(t)?, cols.join(", "));
                self.set(r, t, e)?;
            }
            op::Transpose => {
                let e = format!("transpose({})", self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::Dot => {
                let e = format!("dot({}, {})", self.v(o[0])?, self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::ShiftLeftLogical | op::ShiftRightLogical | op::ShiftRightArithmetic => {
                let signed = i.op == op::ShiftRightArithmetic;
                let (tn, w) = (self.tn(t)?, self.int_as(t, signed)?);
                let sym = if i.op == op::ShiftLeftLogical { "<<" } else { ">>" };
                let shift = self.co_int(o[1], &w)?;
                let e = format!("{}({} {} {})", w, self.co(o[0], &w)?, sym, shift);
                self.set(r, t, if tn == w { e } else { format!("as_type<{}>({})", tn, e) })?;
            }
            op::BitFieldInsert => {
                let tn = self.tn(t)?;
                let e = format!("insert_bits({}, {}, uint({}), uint({}))", self.co(o[0], &tn)?, self.co(o[1], &tn)?, self.v(o[2])?, self.v(o[3])?);
                self.set(r, t, e)?;
            }
            op::BitFieldSExtract | op::BitFieldUExtract => {
                let tn = self.tn(t)?;
                let w = self.int_as(t, i.op == op::BitFieldSExtract)?;
                let e = format!("extract_bits({}, uint({}), uint({}))", self.co(o[0], &w)?, self.v(o[1])?, self.v(o[2])?);
                self.set(r, t, if tn == w { e } else { format!("as_type<{}>({})", tn, e) })?;
            }
            op::BitReverse => {
                let tn = self.tn(t)?;
                let e = format!("reverse_bits({})", self.co(o[0], &tn)?);
                self.set(r, t, e)?;
            }
            op::BitCount => {
                let e = format!("{}(popcount({}))", self.tn(t)?, self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::Any | op::All => {
                let n = self.comps(self.rt(o[0]));
                let v = self.v(o[0])?;
                let e = if n == 1 { v } else { format!("{}({})", if i.op == op::Any { "any" } else { "all" }, v) };
                self.set(r, t, e)?;
            }
            op::IsNan | op::IsInf | op::IsFinite | op::IsNormal | op::SignBitSet => {
                let f = match i.op {
                    op::IsNan => "isnan",
                    op::IsInf => "isinf",
                    op::IsFinite => "isfinite",
                    op::IsNormal => "isnormal",
                    _ => "signbit",
                };
                let e = format!("{}({})", f, self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::LogicalEqual | op::LogicalNotEqual => {
                let sym = if i.op == op::LogicalEqual { "==" } else { "!=" };
                let e = format!("({} {} {})", self.v(o[0])?, sym, self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::LogicalAnd | op::LogicalOr => {
                let sym = if i.op == op::LogicalAnd { "&&" } else { "||" };
                let (a, b) = (self.v(o[0])?, self.v(o[1])?);
                let e = self.cw(t, |c| format!("({}{} {} {}{})", a, c, sym, b, c))?;
                self.set(r, t, e)?;
            }
            op::LogicalNot => {
                let a = self.v(o[0])?;
                let e = self.cw(t, |c| format!("(!{}{})", a, c))?;
                self.set(r, t, e)?;
            }
            op::Select => {
                let c = self.v(o[0])?;
                let (a, b) = (self.v(o[1])?, self.v(o[2])?);
                let cond_n = self.comps(self.rt(o[0]));
                let e = match self.m.ty(t)?.clone() {
                    Ty::Vector { elem, .. } if cond_n > 1 => {
                        if matches!(self.m.ty(elem.0)?, Ty::Bool) {
                            self.cw(t, |k| format!("({}{} ? {}{} : {}{})", c, k, a, k, b, k))?
                        } else {
                            format!("select({}, {}, {})", b, a, c)
                        }
                    }
                    _ => format!("({} ? {} : {})", c, a, b),
                };
                self.set(r, t, e)?;
            }
            op::IEqual | op::INotEqual | op::UGreaterThan | op::SGreaterThan | op::UGreaterThanEqual | op::SGreaterThanEqual
            | op::ULessThan | op::SLessThan | op::ULessThanEqual | op::SLessThanEqual => {
                let at = self.rt(o[0]);
                let w = match i.op {
                    op::IEqual | op::INotEqual => self.tn(at)?,
                    op::SGreaterThan | op::SGreaterThanEqual | op::SLessThan | op::SLessThanEqual => self.int_as(at, true)?,
                    _ => self.int_as(at, false)?,
                };
                let sym = match i.op {
                    op::IEqual => "==",
                    op::INotEqual => "!=",
                    op::UGreaterThan | op::SGreaterThan => ">",
                    op::UGreaterThanEqual | op::SGreaterThanEqual => ">=",
                    op::ULessThan | op::SLessThan => "<",
                    _ => "<=",
                };
                let e = format!("({} {} {})", self.co(o[0], &w)?, sym, self.co(o[1], &w)?);
                self.set(r, t, e)?;
            }
            op::FOrdEqual | op::FUnordNotEqual | op::FOrdLessThan | op::FOrdGreaterThan | op::FOrdLessThanEqual
            | op::FOrdGreaterThanEqual => {
                let sym = match i.op {
                    op::FOrdEqual => "==",
                    op::FUnordNotEqual => "!=",
                    op::FOrdLessThan => "<",
                    op::FOrdGreaterThan => ">",
                    op::FOrdLessThanEqual => "<=",
                    _ => ">=",
                };
                let e = format!("({} {} {})", self.v(o[0])?, sym, self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::FOrdNotEqual | op::FUnordEqual => {
                let (a, b) = (self.v(o[0])?, self.v(o[1])?);
                let neg = if i.op == op::FUnordEqual { "!" } else { "" };
                let e = self.cw(t, |c| format!("{}(({}{} < {}{}) || ({}{} > {}{}))", neg, a, c, b, c, a, c, b, c))?;
                self.set(r, t, e)?;
            }
            op::FUnordLessThan | op::FUnordGreaterThan | op::FUnordLessThanEqual | op::FUnordGreaterThanEqual => {
                let sym = match i.op {
                    op::FUnordLessThan => ">=",
                    op::FUnordGreaterThan => "<=",
                    op::FUnordLessThanEqual => ">",
                    _ => "<",
                };
                let (a, b) = (self.v(o[0])?, self.v(o[1])?);
                let e = self.cw(t, |c| format!("(!({}{} {} {}{}))", a, c, sym, b, c))?;
                self.set(r, t, e)?;
            }
            op::DPdx | op::DPdxFine | op::DPdxCoarse => {
                let e = format!("dfdx({})", self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::DPdy | op::DPdyFine | op::DPdyCoarse => {
                let e = format!("dfdy({})", self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::Fwidth | op::FwidthFine | op::FwidthCoarse => {
                let e = format!("fwidth({})", self.v(o[0])?);
                self.set(r, t, e)?;
            }
            op::ExtInst => {
                if !self.m.glsl.contains(&o[0]) {
                    return err("an extended instruction of an unknown set");
                }
                self.glsl(t, r, o[1], &o[2..])?;
            }
            op::FunctionCall => {
                let callee = o[0];
                let mut args = Vec::new();
                for &a in &o[1..] {
                    args.push(match self.ex.get(&a) {
                        Some(Ex::Ptr(s, ..)) => s.clone(),
                        Some(Ex::Img(s, _)) | Some(Ex::Smp(s)) | Some(Ex::Val(s)) => s.clone(),
                        Some(Ex::SImg(..)) => return err("a sampled image as an argument"),
                        None => {
                            if self.m.vars.contains_key(&a) {
                                format!("g{}", a)
                            } else {
                                self.v(a)?
                            }
                        }
                    });
                }
                for &g in self.m.uses.get(&callee).into_iter().flatten() {
                    if self.m.deco(g, Dec::BuiltIn).and_then(|x| x.first().copied()) == Some(Bi::HelperInvocation as u32) {
                        continue;
                    }
                    args.push(format!("g{}", g));
                }
                if self.m.sizes.contains(&callee) {
                    args.push(String::from("jf_sizes"));
                }
                let call = format!("f{}({})", callee, args.join(", "));
                if matches!(self.m.ty(t)?, Ty::Void) {
                    self.line(&format!("{};", call));
                } else {
                    self.set(r, t, call)?;
                }
            }
            op::ControlBarrier => {
                let exec = self.m.cu32(o[0])?;
                let flags = self.mem_flags(o[2])?;
                match exec {
                    x if x == spv::scope::Workgroup => self.line(&format!("threadgroup_barrier({});", flags)),
                    x if x == spv::scope::Subgroup => self.line(&format!("simdgroup_barrier({});", flags)),
                    _ => return Err(format!("a control barrier of scope {}", exec)),
                }
            }
            op::MemoryBarrier => {
                let flags = self.mem_flags(o[1])?;
                self.line(&format!("atomic_thread_fence({}, memory_order_seq_cst, thread_scope_threadgroup);", flags));
            }
            op::AtomicLoad => {
                let p = self.atomic_ptr(o[0], t)?;
                self.set(r, t, format!("atomic_load_explicit({}, memory_order_relaxed)", p))?;
            }
            op::AtomicStore => {
                let (_, _, pt) = self.p(o[0])?;
                let p = self.atomic_ptr(o[0], pt)?;
                let v = self.co(o[3], &self.tn(pt)?)?;
                self.line(&format!("atomic_store_explicit({}, {}, memory_order_relaxed);", p, v));
            }
            op::AtomicExchange | op::AtomicIAdd | op::AtomicISub | op::AtomicSMin | op::AtomicUMin | op::AtomicSMax
            | op::AtomicUMax | op::AtomicAnd | op::AtomicOr | op::AtomicXor | op::AtomicIIncrement | op::AtomicIDecrement => {
                let (_, _, pt) = self.p(o[0])?;
                let p = self.atomic_ptr(o[0], pt)?;
                let ptn = self.tn(pt)?;
                let f = match i.op {
                    op::AtomicExchange => "atomic_exchange_explicit",
                    op::AtomicIAdd | op::AtomicIIncrement => "atomic_fetch_add_explicit",
                    op::AtomicISub | op::AtomicIDecrement => "atomic_fetch_sub_explicit",
                    op::AtomicSMin | op::AtomicUMin => "atomic_fetch_min_explicit",
                    op::AtomicSMax | op::AtomicUMax => "atomic_fetch_max_explicit",
                    op::AtomicAnd => "atomic_fetch_and_explicit",
                    op::AtomicOr => "atomic_fetch_or_explicit",
                    _ => "atomic_fetch_xor_explicit",
                };
                let v = if matches!(i.op, op::AtomicIIncrement | op::AtomicIDecrement) { format!("{}(1)", ptn) } else { self.co(o[3], &ptn)? };
                let tn = self.tn(t)?;
                if ptn == "ulong" && matches!(i.op, op::AtomicUMin | op::AtomicUMax) {
                    // atomicStoreMin / Max (64 bits, Apple8+): no old value in Metal
                    let f = if i.op == op::AtomicUMin { "atomic_min_explicit" } else { "atomic_max_explicit" };
                    self.line(&format!("{}({}, {}, memory_order_relaxed);", f, p, v));
                    return self.set(r, t, format!("{}(0)", tn));
                }
                let call = format!("{}({}, {}, memory_order_relaxed)", f, p, v);
                self.set(r, t, if tn == ptn { call } else { format!("as_type<{}>({})", tn, call) })?;
            }
            op::AtomicCompareExchange => {
                let (_, _, pt) = self.p(o[0])?;
                let p = self.atomic_ptr(o[0], pt)?;
                let ptn = self.tn(pt)?;
                let (val, cmp) = (self.co(o[4], &ptn)?, self.co(o[5], &ptn)?);
                self.decl.insert(r, self.tn(t)?);
                self.line(&format!("{{ {} e = {}; while (!atomic_compare_exchange_weak_explicit({}, &e, {}, memory_order_relaxed, memory_order_relaxed) && e == {}) {{}} v{} = as_type<{}>(e); }}",
                                   ptn, cmp, p, val, cmp, r, self.tn(t)?));
                self.ex.insert(r, Ex::Val(format!("v{}", r)));
            }
            op::GroupNonUniformElect => self.set(r, t, String::from("simd_is_first()"))?,
            op::GroupNonUniformAll | op::GroupNonUniformAny => {
                let f = if i.op == op::GroupNonUniformAll { "simd_all" } else { "simd_any" };
                let e = format!("{}({})", f, self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::GroupNonUniformBroadcast | op::GroupNonUniformShuffle | op::GroupNonUniformShuffleXor | op::GroupNonUniformShuffleUp
            | op::GroupNonUniformShuffleDown | op::GroupNonUniformQuadBroadcast => {
                let f = match i.op {
                    op::GroupNonUniformBroadcast => "simd_broadcast",
                    op::GroupNonUniformShuffle => "simd_shuffle",
                    op::GroupNonUniformShuffleXor => "simd_shuffle_xor",
                    op::GroupNonUniformShuffleUp => "simd_shuffle_up",
                    op::GroupNonUniformShuffleDown => "simd_shuffle_down",
                    _ => "quad_broadcast",
                };
                let e = format!("{}({}, ushort({}))", f, self.v(o[1])?, self.v(o[2])?);
                self.set(r, t, e)?;
            }
            op::GroupNonUniformBroadcastFirst => {
                let e = format!("simd_broadcast_first({})", self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::GroupNonUniformBallot => {
                let e = format!("uint4(as_type<uint2>((ulong)simd_ballot({})), 0u, 0u)", self.v(o[1])?);
                self.set(r, t, e)?;
            }
            op::GroupNonUniformQuadSwap => {
                let d = self.m.cu32(o[2])?;
                let e = format!("quad_shuffle_xor({}, ushort({}))", self.v(o[1])?, d + 1);
                self.set(r, t, e)?;
            }
            op::GroupNonUniformIAdd | op::GroupNonUniformFAdd | op::GroupNonUniformIMul | op::GroupNonUniformFMul
            | op::GroupNonUniformSMin | op::GroupNonUniformUMin | op::GroupNonUniformFMin | op::GroupNonUniformSMax
            | op::GroupNonUniformUMax | op::GroupNonUniformFMax | op::GroupNonUniformBitwiseAnd | op::GroupNonUniformBitwiseOr
            | op::GroupNonUniformBitwiseXor => {
                let group = o[1];
                let base = match i.op {
                    op::GroupNonUniformIAdd | op::GroupNonUniformFAdd => "sum",
                    op::GroupNonUniformIMul | op::GroupNonUniformFMul => "product",
                    op::GroupNonUniformSMin | op::GroupNonUniformUMin | op::GroupNonUniformFMin => "min",
                    op::GroupNonUniformSMax | op::GroupNonUniformUMax | op::GroupNonUniformFMax => "max",
                    op::GroupNonUniformBitwiseAnd => "and",
                    op::GroupNonUniformBitwiseOr => "or",
                    _ => "xor",
                };
                let f = match group {
                    0 => format!("simd_{}", base),
                    1 if matches!(base, "sum" | "product") => format!("simd_prefix_inclusive_{}", base),
                    2 if matches!(base, "sum" | "product") => format!("simd_prefix_exclusive_{}", base),
                    _ => return Err(format!("subgroup operation {} with group operation {}", base, group)),
                };
                let tn = self.tn(t)?;
                let w = match i.op {
                    op::GroupNonUniformSMin | op::GroupNonUniformSMax => self.int_as(t, true)?,
                    op::GroupNonUniformUMin | op::GroupNonUniformUMax => self.int_as(t, false)?,
                    _ => tn.clone(),
                };
                let call = format!("{}({})", f, self.co(o[2], &w)?);
                self.set(r, t, if w == tn { call } else { format!("as_type<{}>({})", tn, call) })?;
            }
            op::DemoteToHelperInvocation => self.line("discard_fragment();"),
            op::IsHelperInvocationEXT => self.set(r, t, String::from("simd_is_helper_thread()"))?,
            _ => return Err(format!("Op{} is not translated", spv::op_info(i.op).map_or("?", |x| x.name))),
        }
        Ok(())
    }

    /// an integer operand in the signedness of `want` (the shift amount: also its width)
    fn co_int(&self, id: u32, want: &str) -> Result<String, String> {
        let have = self.tn(self.rt(id))?;
        let v = self.v(id)?;
        if have == want {
            Ok(v)
        } else {
            Ok(format!("{}({})", want, v))
        }
    }

    /// a value converted to `want` by value for numbers of other kinds (texels)
    fn co_value(&self, id: u32, want: &str) -> Result<String, String> {
        let have = self.tn(self.rt(id))?;
        let v = self.v(id)?;
        Ok(if have == want { v } else { format!("{}({})", want, v) })
    }

    // ------------------------------------------------------------ images

    fn img(&self, id: u32) -> Result<(String, u32), String> {
        match self.ex.get(&id) {
            Some(Ex::Img(s, t)) => Ok((s.clone(), *t)),
            Some(Ex::SImg(s, _, t)) => Ok((s.clone(), *t)),
            _ => Err(format!("%{} is no image", id)),
        }
    }

    /// (dim, arrayed, element type name)
    fn img_info(&self, it: u32) -> Result<(u32, bool, &'static str), String> {
        let Ty::Image { sampled, dim, arrayed, .. } = self.m.ty(it)?.clone() else { return err("no image type") };
        Ok((dim, arrayed != 0, M::scalar_name(self.m.ty(sampled.0)?)?))
    }

    /// the components of a coordinate without the layer
    fn base_comps(dim: u32) -> usize {
        match dim {
            0 => 1,
            1 => 2,
            _ => 3,
        }
    }

    fn swz(e: &str, from: usize, n: usize) -> String {
        let s: String = (from..from + n).map(|k| LETTERS[k]).collect();
        format!("{}.{}", e, s)
    }

    /// an integer coordinate as uint(N) and the layer
    /// the size of an image as the type of its integer coordinate (uint, uint2, uint3) at level lod ("": its first)
    fn image_size(&self, img: &str, dim: u32, lod: &str) -> String {
        let n = Self::base_comps(dim);
        let mut parts = alloc::vec![format!("{}.get_width({})", img, lod)];
        if n >= 2 {
            parts.push(format!("{}.get_height({})", img, lod));
        }
        if n >= 3 {
            parts.push(format!("{}.get_depth({})", img, lod));
        }
        if n == 1 { parts.remove(0) } else { format!("uint{}({})", n, parts.join(", ")) }
    }

    fn int_coord(&self, c: u32, dim: u32, arrayed: bool) -> Result<(String, Option<String>), String> {
        let e = self.v(c)?;
        let n = Self::base_comps(dim);
        let total = self.comps(self.rt(c)) as usize;
        let ut = if n == 1 { String::from("uint") } else { format!("uint{}", n) };
        let base = if total == n { format!("{}({})", ut, e) } else { format!("{}({})", ut, Self::swz(&e, 0, n)) };
        let layer = if arrayed { Some(format!("uint({})", Self::swz(&e, n, 1))) } else { None };
        Ok((base, layer))
    }

    /// a float coordinate and the layer (rounded, clamped to the layers)
    fn float_coord(&self, c: u32, dim: u32, arrayed: bool, img: &str) -> Result<(String, Option<String>), String> {
        let e = self.v(c)?;
        let n = Self::base_comps(dim);
        let total = self.comps(self.rt(c)) as usize;
        let base = if total == n { e.clone() } else { Self::swz(&e, 0, n) };
        let layer = if arrayed {
            Some(format!("uint(clamp(rint({}), 0.0f, float(int({}.get_array_size()) - 1)))", Self::swz(&e, n, 1), img))
        } else {
            None
        };
        Ok((base, layer))
    }

    fn image(&mut self, opc: u16, t: u32, o: &[u32]) -> Result<String, String> {
        let (img, it) = self.img(o[0])?;
        let Ty::Image { dim, depth, arrayed, ms, .. } = self.m.ty(it)?.clone() else { return err("no image type") };
        let (arrayed, depth, ms) = (arrayed != 0, depth == 1, ms != 0);
        let smp = match self.ex.get(&o[0]) {
            Some(Ex::SImg(_, s, _)) => Some(s.clone()),
            _ => None,
        };
        let tn = self.tn(t)?;
        // image operands: mask after the fixed operands, then ids in bit order
        let fixed = match opc {
            op::ImageSampleDrefImplicitLod | op::ImageSampleDrefExplicitLod | op::ImageGather | op::ImageDrefGather => 3,
            op::ImageSampleImplicitLod | op::ImageSampleExplicitLod | op::ImageFetch | op::ImageRead => 2,
            _ => o.len(),
        };
        let mut ops: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        if o.len() > fixed {
            let mask = o[fixed];
            let mut k = fixed + 1;
            for &(bit, _, args) in spv::IMG_OPS {
                if mask & bit != 0 {
                    let n = args.len();
                    ops.insert(bit, o.get(k..k + n).ok_or("image operands too short")?.to_vec());
                    k += n;
                }
            }
        }
        let lod_opt = |fx: &Self| -> Result<String, String> {
            if let Some(v) = ops.get(&spv::img::Lod) {
                return Ok(format!(", level({})", fx.v(v[0])?));
            }
            if let Some(v) = ops.get(&spv::img::Bias) {
                return Ok(format!(", bias({})", fx.v(v[0])?));
            }
            if let Some(v) = ops.get(&spv::img::Grad) {
                let g = match dim {
                    3 => "gradientcube",
                    2 => "gradient3d",
                    _ => "gradient2d",
                };
                return Ok(format!(", {}({}, {})", g, fx.v(v[0])?, fx.v(v[1])?));
            }
            Ok(String::new())
        };
        let offset = match ops.get(&spv::img::ConstOffset) {
            Some(v) => format!(", {}", self.v(v[0])?),
            None => String::new(),
        };
        if ops.contains_key(&0x10) {
            return err("a non-constant texel offset");
        }
        let depth4 = |e: String| if depth { format!("float4({}, 0.0f, 0.0f, 1.0f)", e) } else { e };
        Ok(match opc {
            op::ImageSampleImplicitLod | op::ImageSampleExplicitLod => {
                let s = smp.ok_or("sampling without sampler")?;
                let (c, layer) = self.float_coord(o[1], dim, arrayed, &img)?;
                let layer = layer.map(|l| format!(", {}", l)).unwrap_or_default();
                let lod = if dim == 0 { String::new() } else { lod_opt(self)? };
                let e = format!("{}.sample({}, {}{}{}{})", img, s, c, layer, lod, offset);
                let e = depth4(e);
                if tn.starts_with("float4") || depth { e } else { format!("{}({})", tn, e) }
            }
            op::ImageSampleDrefImplicitLod | op::ImageSampleDrefExplicitLod => {
                let s = smp.ok_or("sampling without sampler")?;
                let (c, layer) = self.float_coord(o[1], dim, arrayed, &img)?;
                let layer = layer.map(|l| format!(", {}", l)).unwrap_or_default();
                format!("{}.sample_compare({}, {}{}, {}{}{})", img, s, c, layer, self.v(o[2])?, lod_opt(self)?, offset)
            }
            op::ImageGather => {
                let s = smp.ok_or("gather without sampler")?;
                let (c, layer) = self.float_coord(o[1], dim, arrayed, &img)?;
                let layer = layer.map(|l| format!(", {}", l)).unwrap_or_default();
                let comp = self.m.cu32(o[2]).unwrap_or(0);
                if depth {
                    format!("{}.gather({}, {}{}{})", img, s, c, layer, offset)
                } else if dim == 3 {
                    format!("{}.gather({}, {}{}, component::{})", img, s, c, layer, LETTERS[comp as usize & 3])
                } else {
                    let off = if offset.is_empty() { String::from(", int2(0)") } else { offset };
                    format!("{}.gather({}, {}{}{}, component::{})", img, s, c, layer, off, LETTERS[comp as usize & 3])
                }
            }
            op::ImageDrefGather => {
                let s = smp.ok_or("gather without sampler")?;
                let (c, layer) = self.float_coord(o[1], dim, arrayed, &img)?;
                let layer = layer.map(|l| format!(", {}", l)).unwrap_or_default();
                format!("{}.gather_compare({}, {}{}, {}{})", img, s, c, layer, self.v(o[2])?, offset)
            }
            op::ImageFetch | op::ImageRead => {
                // Metal's read() outside the image is undefined (values that change from run to run, docs/APPLE.md
                // AP6): level, layer, sample and coordinate are clamped into the image, one of the results WGSL allows
                let (c, layer) = self.int_coord(o[1], dim, arrayed)?;
                let sample = match ops.get(&spv::img::Sample) {
                    Some(v) => Some(self.v(v[0])?),
                    None => None,
                };
                let lod = match ops.get(&spv::img::Lod) {
                    Some(v) if sample.is_none() && dim != 0 && opc != op::ImageRead => {
                        Some(format!("min(uint({}), {}.get_num_mip_levels() - 1u)", self.v(v[0])?, img))
                    }
                    _ => None,
                };
                let c = format!("min({}, {} - 1u)", c, self.image_size(&img, dim, lod.as_deref().unwrap_or("")));
                let layer = layer.map(|l| format!(", min({}, {}.get_array_size() - 1u)", l, img)).unwrap_or_default();
                let extra = match (&sample, &lod) {
                    (Some(sm), _) => format!(", min(uint({}), {}.get_num_samples() - 1u)", sm, img),
                    (None, Some(l)) => format!(", {}", l),
                    _ => String::new(),
                };
                let e = depth4(format!("{}.read({}{}{})", img, c, layer, extra));
                if tn.starts_with("float4") || depth { e } else { format!("{}({})", tn, e) }
            }
            op::ImageQuerySizeLod | op::ImageQuerySize => {
                let lod = if opc == op::ImageQuerySizeLod && dim != 0 && !ms { format!("uint({})", self.v(o[1])?) } else { String::new() };
                let mut parts = alloc::vec![format!("{}.get_width({})", img, lod)];
                if dim != 0 {
                    parts.push(format!("{}.get_height({})", img, lod));
                }
                if dim == 2 {
                    parts.push(format!("{}.get_depth({})", img, lod));
                }
                if arrayed {
                    parts.push(format!("{}.get_array_size()", img));
                }
                let n = parts.len();
                let u = if n == 1 { String::from("uint") } else { format!("uint{}", n) };
                format!("{}({}({}))", tn, u, parts.join(", "))
            }
            op::ImageQueryLevels => format!("{}({}.get_num_mip_levels())", tn, img),
            op::ImageQuerySamples => format!("{}({}.get_num_samples())", tn, img),
            op::ImageQueryLod => {
                let s = smp.ok_or("lod query without sampler")?;
                let c = self.v(o[1])?;
                format!("float2({}.calculate_clamped_lod({}, {}), {}.calculate_unclamped_lod({}, {}))", img, s, c, img, s, c)
            }
            _ => return err("image instruction"),
        })
    }

    // ------------------------------------------------------------ GLSL.std.450

    fn glsl(&mut self, t: u32, r: u32, inst: u32, a: &[u32]) -> Result<(), String> {
        use crate::spv::glsl as g;
        let tn = self.tn(t)?;
        let n = self.comps(t);
        let arg = |fx: &Self, k: usize| -> Result<String, String> { fx.v(*a.get(k).ok_or("too few operands")?) };
        let call = |fx: &Self, f: &str, k: usize| -> Result<String, String> {
            let args: Result<Vec<String>, String> = (0..k).map(|j| arg(fx, j)).collect();
            Ok(format!("{}({})", f, args?.join(", ")))
        };
        let float = matches!(self.m.num(t).map(|x| x.0), Ok(Ty::Float { .. }));
        let e = match inst {
            g::Round => call(self, "round", 1)?,
            g::RoundEven => call(self, "rint", 1)?,
            g::Trunc => call(self, "trunc", 1)?,
            g::FAbs => call(self, "abs", 1)?,
            g::SAbs => {
                let s = self.int_as(t, true)?;
                let e = format!("abs({})", self.co(a[0], &s)?);
                if s == tn { e } else { format!("as_type<{}>({})", tn, e) }
            }
            g::FSign => call(self, "sign", 1)?,
            g::SSign => {
                let s = self.int_as(t, true)?;
                let e = format!("clamp({}, {}(-1), {}(1))", self.co(a[0], &s)?, s, s);
                if s == tn { e } else { format!("as_type<{}>({})", tn, e) }
            }
            g::Floor => call(self, "floor", 1)?,
            g::Ceil => call(self, "ceil", 1)?,
            g::Fract => call(self, "fract", 1)?,
            11 => format!("({} * {}(0.017453292519943295))", arg(self, 0)?, tn), // Radians
            12 => format!("({} * {}(57.29577951308232))", arg(self, 0)?, tn),   // Degrees
            g::Sin => call(self, "sin", 1)?,
            g::Cos => call(self, "cos", 1)?,
            g::Tan => call(self, "tan", 1)?,
            16 => call(self, "asin", 1)?,
            17 => call(self, "acos", 1)?,
            18 => call(self, "atan", 1)?,
            19 => call(self, "sinh", 1)?,
            20 => call(self, "cosh", 1)?,
            21 => call(self, "tanh", 1)?,
            22 => {
                let x = arg(self, 0)?;
                self.cw(t, |c| format!("jf_asinh({}{})", x, c))?
            }
            23 => call(self, "acosh", 1)?,
            24 => call(self, "atanh", 1)?,
            g::Atan2 => {
                let (y, x) = (arg(self, 0)?, arg(self, 1)?);
                self.cw(t, |c| format!("jf_atan2({}{}, {}{})", y, c, x, c))?
            }
            g::Pow => call(self, "pow", 2)?,
            g::Exp => call(self, "exp", 1)?,
            g::Log => call(self, "log", 1)?,
            g::Exp2 => call(self, "exp2", 1)?,
            g::Log2 => call(self, "log2", 1)?,
            g::Sqrt => call(self, "sqrt", 1)?,
            g::InverseSqrt => call(self, "rsqrt", 1)?,
            33 => call(self, "determinant", 1)?,
            35 => {
                // Modf(x, out whole)
                let (lv, ..) = self.p(a[1])?;
                format!("modf({}, {})", arg(self, 0)?, lv)
            }
            36 => {
                // ModfStruct: { fract, whole }
                self.decl.insert(r, tn.clone());
                let x = arg(self, 0)?;
                self.line(&format!("v{}.m0 = modf({}, v{}.m1);", r, x, r));
                self.ex.insert(r, Ex::Val(format!("v{}", r)));
                return Ok(());
            }
            g::FMin => call(self, "fmin", 2)?,
            g::FMax => call(self, "fmax", 2)?,
            79 => call(self, "fmin", 2)?, // NMin
            80 => call(self, "fmax", 2)?, // NMax
            g::UMin | g::UMax | g::SMin | g::SMax => {
                let signed = matches!(inst, g::SMin | g::SMax);
                let w = self.int_as(t, signed)?;
                let f = if matches!(inst, g::UMin | g::SMin) { "min" } else { "max" };
                let e = format!("{}({}, {})", f, self.co(a[0], &w)?, self.co(a[1], &w)?);
                if w == tn { e } else { format!("as_type<{}>({})", tn, e) }
            }
            g::FClamp | 81 => call(self, "clamp", 3)?,
            g::UClamp | g::SClamp => {
                let w = self.int_as(t, inst == g::SClamp)?;
                let e = format!("clamp({}, {}, {})", self.co(a[0], &w)?, self.co(a[1], &w)?, self.co(a[2], &w)?);
                if w == tn { e } else { format!("as_type<{}>({})", tn, e) }
            }
            g::FMix => call(self, "mix", 3)?,
            g::Step => call(self, "step", 2)?,
            g::SmoothStep => call(self, "smoothstep", 3)?,
            g::Fma => call(self, "fma", 3)?,
            51 => {
                // Frexp(x, out exp)
                let (lv, ..) = self.p(a[1])?;
                format!("frexp({}, {})", arg(self, 0)?, lv)
            }
            52 => {
                // FrexpStruct: { fract, exp }
                self.decl.insert(r, tn.clone());
                let x = arg(self, 0)?;
                self.line(&format!("v{}.m0 = frexp({}, v{}.m1);", r, x, r));
                self.ex.insert(r, Ex::Val(format!("v{}", r)));
                return Ok(());
            }
            53 => call(self, "ldexp", 2)?,
            54 => format!("pack_float_to_snorm4x8({})", arg(self, 0)?),
            55 => format!("pack_float_to_unorm4x8({})", arg(self, 0)?),
            56 => format!("pack_float_to_snorm2x16({})", arg(self, 0)?),
            57 => format!("pack_float_to_unorm2x16({})", arg(self, 0)?),
            58 => format!("as_type<uint>(half2({}))", arg(self, 0)?),
            60 => format!("unpack_snorm2x16_to_float({})", arg(self, 0)?),
            61 => format!("unpack_unorm2x16_to_float({})", arg(self, 0)?),
            62 => format!("float2(as_type<half2>({}))", arg(self, 0)?),
            63 => format!("unpack_snorm4x8_to_float({})", arg(self, 0)?),
            64 => format!("unpack_unorm4x8_to_float({})", arg(self, 0)?),
            g::Length => {
                if self.comps(self.rt(a[0])) == 1 { format!("abs({})", arg(self, 0)?) } else { call(self, "length", 1)? }
            }
            g::Distance => {
                if self.comps(self.rt(a[0])) == 1 { format!("abs({} - {})", arg(self, 0)?, arg(self, 1)?) } else { call(self, "distance", 2)? }
            }
            g::Cross => call(self, "cross", 2)?,
            g::Normalize => {
                if n == 1 { format!("sign({})", arg(self, 0)?) } else { call(self, "normalize", 1)? }
            }
            70 => call(self, "faceforward", 3)?,
            g::Reflect => {
                if n == 1 { format!("({0} - 2.0f * {1} * {0} * {1})", arg(self, 0)?, arg(self, 1)?) } else { call(self, "reflect", 2)? }
            }
            72 => call(self, "refract", 3)?,
            73..=75 => {
                // FindILsb, FindSMsb, FindUMsb: -1 for no bit
                let at = self.rt(a[0]);
                let (s, _) = self.m.num(at)?;
                let Ty::Int { width, .. } = s else { return err("a bit search on no integer") };
                let u = self.int_as(at, false)?;
                let ux = self.co(a[0], &u)?;
                let i32t = self.int_as(t, true)?;
                let e = match inst {
                    73 => format!("select({}(ctz({})), {}(-1), {} == {}(0))", i32t, ux, i32t, ux, u),
                    75 => format!("select({}({} - 1) - {}(clz({})), {}(-1), {} == {}(0))", i32t, width, i32t, ux, i32t, ux, u),
                    _ => {
                        let sx = self.co(a[0], &self.int_as(at, true)?)?;
                        let m = format!("as_type<{}>(select({}, {}(~{}), {} < 0))", u, sx, self.int_as(at, true)?, sx, sx);
                        format!("select({}({} - 1) - {}(clz({})), {}(-1), {} == {}(0))", i32t, width, i32t, m, i32t, m, u)
                    }
                };
                if i32t == tn { e } else { format!("as_type<{}>({})", tn, e) }
            }
            _ => return Err(format!("GLSL.std.450 instruction {} ({})", inst, spv::GLSL_NAMES.get(inst as usize).unwrap_or(&"?"))),
        };
        let _ = float;
        self.set(r, t, e)
    }
}

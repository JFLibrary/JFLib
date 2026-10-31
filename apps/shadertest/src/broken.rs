// broken -- the validator: it accepts every benchmark shader (both levels) and the builder's
// feature modules, and rejects one deliberately broken module per rule (cases(), also written
// by --write and run through spirv-val by the sdk group)

use alloc::format;
use alloc::vec::Vec;

use shader::bench::{self, Shader};
use shader::val::{validate, Rule};
use shader::{op, parse, Bi, Dec, Mode, Model, Module, Sc};

use crate::T;

pub struct Case {
    pub name: &'static str,
    pub rule: Rule,
    pub words: Vec<u32>,
    /// spirv-val (Vulkan environment) rejects it as well
    pub spirv_val: bool,
}

/// a module as header + instructions (each with its opcode word)
struct Words {
    head: Vec<u32>,
    ins: Vec<Vec<u32>>,
}

fn split(w: &[u32]) -> Words {
    let mut ins = Vec::new();
    let mut i = 5;
    while i < w.len() {
        let n = (w[i] >> 16) as usize;
        ins.push(w[i..i + n].to_vec());
        i += n;
    }
    Words { head: w[..5].to_vec(), ins }
}

impl Words {
    fn join(&self) -> Vec<u32> {
        let mut out = self.head.clone();
        for x in &self.ins {
            out.push(((x.len() as u32) << 16) | (x[0] & 0xffff));
            out.extend_from_slice(&x[1..]);
        }
        out
    }
    /// the first instruction with opcode o whose words after the opcode word satisfy f
    fn find(&self, o: u16, f: impl Fn(&[u32]) -> bool) -> usize {
        self.ins.iter().position(|x| (x[0] & 0xffff) as u16 == o && f(&x[1..]))
            .unwrap_or_else(|| panic!("shadertest: no Op{} found", o))
    }
}

/// the id an OpName gives `name`
fn named(w: &[u32], name: &str) -> u32 {
    parse::parse(w).unwrap_or_default().iter()
        .find(|i| i.op == op::Name && parse::string(&i.ops[1..]).0 == name)
        .map_or(0, |i| i.ops[0])
}

/// a GLCompute module (LocalSize 1 1 1) with body in its entry function
fn compute(minor: u32, body: impl FnOnce(&mut Module)) -> Vec<u32> {
    let mut m = Module::new(minor);
    let void = m.ty_void();
    let (main, _) = m.begin_function(void, &[]);
    body(&mut m);
    if m.block_open() {
        m.ret();
    }
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &[1, 1, 1]);
    m.finish()
}

fn fragment(minor: u32, body: impl FnOnce(&mut Module)) -> Vec<u32> {
    let mut m = Module::new(minor);
    let void = m.ty_void();
    let (main, _) = m.begin_function(void, &[]);
    body(&mut m);
    if m.block_open() {
        m.ret();
    }
    m.end_function();
    m.entry_point(Model::Fragment, main, "main");
    m.exec_mode(main, Mode::OriginUpperLeft, &[]);
    m.finish()
}

fn edit(w: Vec<u32>, f: impl FnOnce(&mut Words)) -> Vec<u32> {
    let mut x = split(&w);
    f(&mut x);
    x.join()
}

pub fn cases() -> Vec<Case> {
    let tiny = crate::basics::tiny();
    let mv = bench::move_spv(3);
    let cull = bench::cull_spv(3);
    let fs = bench::fs_lambert_spv(3);
    let c = |name, rule, words, spirv_val| Case { name, rule, words, spirv_val };
    let mut v = Vec::new();

    // Header
    let mut w = tiny.clone();
    w[0] = 0x0723_0204;
    v.push(c("header_magic", Rule::Header, w, true));
    let mut w = tiny.clone();
    w[1] = 0x0001_0700;
    v.push(c("header_version", Rule::Header, w, true));
    // Grammar
    v.push(c("grammar_opcode", Rule::Grammar, edit(tiny.clone(), |x| {
        let k = x.find(op::Return, |_| true);
        x.ins.insert(k, alloc::vec![9999]);
    }), true));
    v.push(c("grammar_operands", Rule::Grammar, edit(mv.clone(), |x| {
        let k = x.find(op::TypeVector, |_| true);
        x.ins[k].pop();
    }), true));
    // Section
    v.push(c("section_order", Rule::Section, edit(tiny.clone(), |x| {
        let cap = x.ins.remove(0);
        x.ins.insert(1, cap);
    }), true));
    // IdUnique: OpTypeFunction gets the id of OpTypeVoid
    v.push(c("id_twice", Rule::IdUnique, edit(tiny.clone(), |x| {
        let k = x.find(op::TypeFunction, |_| true);
        x.ins[k][1] = 1;
    }), true));
    // IdDefined: a load from an id nothing defines; a use before the definition
    let n_in = named(&fs, "in_normal");
    v.push(c("id_undefined", Rule::IdDefined, edit(fs.clone(), |x| {
        let bound = x.head[3];
        x.head[3] += 1;
        let k = x.find(op::Load, |o| o[2] == n_in);
        x.ins[k][3] = bound;
    }), true));
    v.push(c("id_forward", Rule::IdDefined, edit(fs.clone(), |x| {
        let k = x.find(op::ExtInst, |_| true);
        x.ins.swap(k - 1, k);
    }), true));
    // IdKind: the result type of OpDot is a constant
    v.push(c("id_kind", Rule::IdKind, edit(fs.clone(), |x| {
        let kc = x.find(op::Constant, |_| true);
        let konst = x.ins[kc][2];
        let k = x.find(op::Dot, |_| true);
        x.ins[k][1] = konst;
    }), true));
    // Type
    v.push(c("type_fadd_int", Rule::Type, compute(3, |m| {
        let (f, u) = (m.ty_f32(), m.ty_u32());
        let (cf, cu) = (m.const_f32(1.0), m.const_u32(1));
        let (lf, lu) = (m.local(f, Some(cf)), m.local(u, Some(cu)));
        let (a, b) = (m.load(lf), m.load(lu));
        m.fadd(a, b);
    }), true));
    v.push(c("type_load", Rule::Type, compute(3, |m| {
        let (f, u) = (m.ty_f32(), m.ty_u32());
        let lf = m.local(f, None);
        m.emit(op::Load, Some(u), &[lf.0]);
    }), true));
    v.push(c("type_extract", Rule::Type, compute(3, |m| {
        let f = m.ty_f32();
        let v4 = m.ty_vec(f, 4);
        let k = m.const_vec_f32(&[1.0, 2.0, 3.0, 4.0]);
        let l = m.local(v4, Some(k));
        let x = m.load(l);
        m.emit(op::CompositeExtract, Some(f), &[x.0, 5]);
    }), true));
    v.push(c("type_vector5", Rule::Type, compute(3, |m| {
        let f = m.ty_f32();
        m.ty_vec(f, 5);
    }), true));
    v.push(c("type_access", Rule::Type, compute(3, |m| {
        let f = m.ty_f32();
        let v4 = m.ty_vec(f, 4);
        let s = m.ty_struct(&[f, v4]);
        let l = m.local(s, None);
        let pf = m.ty_ptr(Sc::Function, f);
        let one = m.const_u32(1);
        m.emit(op::AccessChain, Some(pf), &[l.0, one.0]);
    }), true));
    // Decoration: Block on a variable
    v.push(c("decoration_block_var", Rule::Decoration, compute(3, |m| {
        let u = m.ty_u32();
        let wv = m.workgroup(u);
        m.decorate(wv, Dec::Block, &[]);
        let one = m.const_u32(1);
        m.store(wv, one);
    }), true));
    // Storage
    v.push(c("storage_no_block", Rule::Storage, compute(3, |m| {
        let u = m.ty_u32();
        let s = m.ty_struct_layout("S", &[("x", u)], shader::Rule::Std430);
        let var = m.global(Sc::StorageBuffer, s, None);
        m.decorate(var, Dec::DescriptorSet, &[0]);
        m.decorate(var, Dec::Binding, &[0]);
        let p = m.access_c(var, &[0]);
        let one = m.const_u32(1);
        m.store(p, one);
    }), true));
    let objects = named(&mv, "objects");
    v.push(c("storage_no_binding", Rule::Storage, edit(mv.clone(), |x| {
        let k = x.find(op::Decorate, |o| o[0] == objects && o[1] == Dec::Binding as u32);
        x.ins.remove(k);
    }), true));
    v.push(c("storage_store_input", Rule::Storage, compute(3, |m| {
        let u = m.ty_u32();
        let uv3 = m.ty_vec(u, 3);
        let g = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
        let z = m.const_null(uv3);
        m.store(g, z);
    }), true));
    v.push(c("storage_late_local", Rule::Storage, edit(cull.clone(), |x| {
        let k = x.find(op::Variable, |o| o[2] == Sc::Function as u32);
        let var = x.ins.remove(k);
        let j = (k..x.ins.len()).find(|&j| (x.ins[j][0] & 0xffff) as u16 != op::Variable).unwrap_or(k);
        x.ins.insert(j + 1, var);
    }), true));
    // BlockLayout
    let obj = named(&mv, "Object");
    v.push(c("layout_misaligned", Rule::BlockLayout, edit(mv.clone(), |x| {
        let k = x.find(op::MemberDecorate, |o| o[0] == obj && o[1] == 2 && o[2] == Dec::Offset as u32);
        x.ins[k][4] = 20;
    }), true));
    v.push(c("layout_overlap", Rule::BlockLayout, edit(mv.clone(), |x| {
        let k = x.find(op::MemberDecorate, |o| o[0] == obj && o[1] == 1 && o[2] == Dec::Offset as u32);
        x.ins[k][4] = 8;
    }), true));
    v.push(c("layout_std140_stride", Rule::BlockLayout, edit(cull.clone(), |x| {
        let k = x.find(op::Decorate, |o| o[1] == Dec::ArrayStride as u32 && o[2] == 16);
        x.ins[k][3] = 8;
    }), true));
    let camera = named(&cull, "Camera");
    v.push(c("layout_no_matrix_stride", Rule::BlockLayout, edit(cull.clone(), |x| {
        let k = x.find(op::MemberDecorate, |o| o[0] == camera && o[2] == Dec::MatrixStride as u32);
        x.ins.remove(k);
    }), true));
    v.push(c("layout_push_128", Rule::BlockLayout, compute(3, |m| {
        let f = m.ty_f32();
        let arr = m.ty_array_layout(f, 40, shader::Rule::Std430);
        let s = m.ty_struct_layout("Big", &[("v", arr)], shader::Rule::Std430);
        let pc = m.push_constant(s);
        let p = m.access_c(pc, &[0, 39]);
        m.load(p);
    }), false));
    // Cfg
    v.push(c("cfg_no_merge", Rule::Cfg, edit(cull.clone(), |x| {
        let k = x.find(op::SelectionMerge, |_| true);
        x.ins.remove(k);
    }), true));
    v.push(c("cfg_no_terminator", Rule::Cfg, edit(tiny.clone(), |x| {
        let k = x.find(op::Return, |_| true);
        x.ins.remove(k);
    }), true));
    v.push(c("cfg_back_edge", Rule::Cfg, compute(3, |m| {
        let (l1, l2) = (m.label(), m.label());
        m.branch(l1);
        m.begin_block(l1);
        m.branch(l2);
        m.begin_block(l2);
        m.branch(l1);
    }), true));
    // Interface
    let mv6 = bench::move_spv(6);
    let objects6 = named(&mv6, "objects");
    v.push(c("interface_missing", Rule::Interface, edit(mv6, |x| {
        let k = x.find(op::EntryPoint, |_| true);
        x.ins[k].retain(|&w| w != objects6);
    }), true));
    v.push(c("interface_13_buffer", Rule::Interface, edit(mv.clone(), |x| {
        let k = x.find(op::EntryPoint, |_| true);
        x.ins[k].push(objects);
    }), true));
    // ExecMode
    v.push(c("mode_no_local_size", Rule::ExecMode, edit(tiny.clone(), |x| {
        let k = x.find(op::ExecutionMode, |_| true);
        x.ins.remove(k);
    }), true));
    v.push(c("mode_no_origin", Rule::ExecMode, edit(fs.clone(), |x| {
        let k = x.find(op::ExecutionMode, |_| true);
        x.ins.remove(k);
    }), true));
    v.push(c("mode_derivative_vertex", Rule::ExecMode, {
        let mut m = Module::new(3);
        let void = m.ty_void();
        let f = m.ty_f32();
        let x = m.input(f, 0);
        let (main, _) = m.begin_function(void, &[]);
        let a = m.load(x);
        m.dpdx(a);
        m.ret();
        m.end_function();
        m.entry_point(Model::Vertex, main, "main");
        m.finish()
    }, true));
    // Capability
    v.push(c("capability_int64", Rule::Capability, edit(compute(3, |m| {
        let l = m.ty_int(64, false);
        m.local(l, None);
    }), |x| {
        let k = x.find(op::Capability, |o| o[0] == shader::Cap::Int64 as u32);
        x.ins.remove(k);
    }), true));
    // Version: OpTerminateInvocation in a SPIR-V 1.3 module
    v.push(c("version_terminate", Rule::Version, fragment(3, |m| {
        m.emit(op::TerminateInvocation, None, &[]);
    }), true));
    v
}

pub fn run(t: &mut T) {
    for sh in Shader::ALL {
        for minor in [3u32, 6] {
            match validate(&sh.build(minor)) {
                Ok(()) => t.check("", true),
                Err(e) => {
                    for x in e.iter().take(5) {
                        crate::out(&format!("   {}\n", x));
                    }
                    t.check(&format!("{} 1.{} validates", sh.name(), minor), false);
                }
            }
        }
    }
    t.check("tiny validates", validate(&crate::basics::tiny()).is_ok());
    for minor in [3u32, 6] {
        t.check(&format!("fragment features 1.{} validate", minor), validate(&crate::basics::frag_features(minor)).is_ok());
    }
    t.check("compute features validate", validate(&crate::basics::features()).is_ok());
    let mut rules = alloc::collections::BTreeSet::new();
    for case in cases() {
        match validate(&case.words) {
            Ok(()) => t.check(&format!("{} is rejected", case.name), false),
            Err(e) => {
                let hit = e.iter().any(|x| x.rule == case.rule);
                if !hit {
                    for x in e.iter().take(3) {
                        crate::out(&format!("   {}\n", x));
                    }
                }
                t.check(&format!("{} is rejected by rule {:?}", case.name, case.rule), hit);
                rules.insert(format!("{:?}", case.rule));
            }
        }
    }
    t.eq("every rule has a case", rules.len(), 15);
}

// spec -- specialization constants and the sample modules: what the builder writes per level
// (WorkgroupSize built-in on 1.3, LocalSizeId on 1.6), shader::reflect (the spec constants of a
// module, their values with and without a pipeline's constants, every OpSpecConstantOp operation
// against Rust arithmetic, the undefined cases, workgroup sizes), the half conversions, the
// validator on every module of shader::samples, spec_fill's disassembly against its golden
// files, the samples built twice

use alloc::format;
use alloc::vec::Vec;

use shader::reflect::{self, Kind};
use shader::samples::{self, SpecParams};
use shader::{dis, op, parse, Bi, Dec, Mode, Model, Module};

use crate::T;

fn has_op(w: &[u32], o: u16, f: impl Fn(&[u32]) -> bool) -> bool {
    parse::parse(w).unwrap_or_default().iter().any(|i| i.op == o && f(i.ops))
}

/// a module with one OpSpecConstantOp result per case (ty, op, operands, literals): the values
/// reflect::constant_values gives them
fn ops_module() -> (Vec<u32>, Vec<(&'static str, shader::Id, Vec<u32>)>) {
    let mut m = Module::new(6);
    let (u, i, b, f) = (m.ty_u32(), m.ty_i32(), m.ty_bool(), m.ty_f32());
    let (u16t, i64t, f16t) = (m.ty_int(16, false), m.ty_int(64, true), m.ty_float(16));
    let uv2 = m.ty_vec(u, 2);
    let uv3 = m.ty_vec(u, 3);
    let a = m.spec_u32(0, 100);
    let n = m.spec_i32(1, -7);
    let t = m.spec_bool(2, true);
    let x = m.spec_f32(3, 1.0 / 3.0);
    let (c3, c5, c0) = (m.const_u32(3), m.const_u32(5), m.const_u32(0));
    let i3 = m.const_i32(3);
    let ff = m.const_bool(false);
    let v = m.spec_composite(uv3, &[a, c3, c5]);
    let mut cases = Vec::new();
    let mut k = |m: &mut Module, name: &'static str, ty: shader::Id, o: u16, ids: &[shader::Id], lit: &[u32], want: Vec<u32>| {
        let r = m.spec_op(ty, o, ids, lit);
        cases.push((name, r, want));
    };
    let s = |v: i32| v as u32;
    k(&mut m, "IAdd", u, op::IAdd, &[a, c3], &[], alloc::vec![103]);
    k(&mut m, "ISub", u, op::ISub, &[c3, a], &[], alloc::vec![3u32.wrapping_sub(100)]);
    k(&mut m, "IMul", i, op::IMul, &[n, i3], &[], alloc::vec![s(-21)]);
    k(&mut m, "UDiv", u, op::UDiv, &[a, c3], &[], alloc::vec![33]);
    k(&mut m, "SDiv", i, op::SDiv, &[n, i3], &[], alloc::vec![s(-2)]);
    k(&mut m, "UMod", u, op::UMod, &[a, c3], &[], alloc::vec![1]);
    k(&mut m, "SRem", i, op::SRem, &[n, i3], &[], alloc::vec![s(-1)]);
    k(&mut m, "SMod", i, op::SMod, &[n, i3], &[], alloc::vec![s(2)]);
    k(&mut m, "SNegate", i, op::SNegate, &[n], &[], alloc::vec![7]);
    k(&mut m, "Not", u, op::Not, &[a], &[], alloc::vec![!100u32]);
    k(&mut m, "ShiftLeftLogical", u, op::ShiftLeftLogical, &[a, c3], &[], alloc::vec![800]);
    k(&mut m, "ShiftRightLogical", u, op::ShiftRightLogical, &[a, c3], &[], alloc::vec![12]);
    k(&mut m, "ShiftRightArithmetic", i, op::ShiftRightArithmetic, &[n, c3], &[], alloc::vec![s(-1)]);
    k(&mut m, "BitwiseOr", u, op::BitwiseOr, &[a, c3], &[], alloc::vec![103]);
    k(&mut m, "BitwiseXor", u, op::BitwiseXor, &[a, c5], &[], alloc::vec![97]);
    k(&mut m, "BitwiseAnd", u, op::BitwiseAnd, &[a, c5], &[], alloc::vec![4]);
    k(&mut m, "LogicalOr", b, op::LogicalOr, &[t, ff], &[], alloc::vec![1]);
    k(&mut m, "LogicalAnd", b, op::LogicalAnd, &[t, ff], &[], alloc::vec![0]);
    k(&mut m, "LogicalNot", b, op::LogicalNot, &[t], &[], alloc::vec![0]);
    k(&mut m, "LogicalEqual", b, op::LogicalEqual, &[t, ff], &[], alloc::vec![0]);
    k(&mut m, "LogicalNotEqual", b, op::LogicalNotEqual, &[t, ff], &[], alloc::vec![1]);
    k(&mut m, "Select", u, op::Select, &[t, a, c3], &[], alloc::vec![100]);
    k(&mut m, "IEqual", b, op::IEqual, &[a, c3], &[], alloc::vec![0]);
    k(&mut m, "INotEqual", b, op::INotEqual, &[a, c3], &[], alloc::vec![1]);
    k(&mut m, "ULessThan", b, op::ULessThan, &[c3, a], &[], alloc::vec![1]);
    k(&mut m, "SLessThan", b, op::SLessThan, &[n, i3], &[], alloc::vec![1]);
    k(&mut m, "UGreaterThan", b, op::UGreaterThan, &[c3, a], &[], alloc::vec![0]);
    k(&mut m, "SGreaterThan", b, op::SGreaterThan, &[n, i3], &[], alloc::vec![0]);
    k(&mut m, "ULessThanEqual", b, op::ULessThanEqual, &[c3, c3], &[], alloc::vec![1]);
    k(&mut m, "SLessThanEqual", b, op::SLessThanEqual, &[i3, n], &[], alloc::vec![0]);
    k(&mut m, "UGreaterThanEqual", b, op::UGreaterThanEqual, &[a, c3], &[], alloc::vec![1]);
    k(&mut m, "SGreaterThanEqual", b, op::SGreaterThanEqual, &[n, n], &[], alloc::vec![1]);
    k(&mut m, "UConvert", u16t, op::UConvert, &[a], &[], alloc::vec![100]);
    k(&mut m, "SConvert", i64t, op::SConvert, &[n], &[], alloc::vec![s(-7), u32::MAX]);
    k(&mut m, "FConvert", f16t, op::FConvert, &[x], &[], alloc::vec![0x3555]);
    k(&mut m, "QuantizeToF16", f, op::QuantizeToF16, &[x], &[], alloc::vec![reflect::f16_to_f32(0x3555).to_bits()]);
    k(&mut m, "VectorShuffle", uv2, op::VectorShuffle, &[v, v], &[2, 3], alloc::vec![5, 100]);
    k(&mut m, "CompositeExtract", u, op::CompositeExtract, &[v], &[1], alloc::vec![3]);
    k(&mut m, "CompositeInsert", uv3, op::CompositeInsert, &[c0, v], &[0], alloc::vec![0, 3, 5]);
    let void = m.ty_void();
    let (main, _) = m.begin_function(void, &[]);
    m.ret();
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &[1, 1, 1]);
    (m.finish(), cases)
}

/// a module whose one OpSpecConstantOp is o on (spec u32 0 = 7, constant c)
fn one_op(o: u16, c: u32) -> Vec<u32> {
    let mut m = Module::new(3);
    let u = m.ty_u32();
    let a = m.spec_u32(0, 7);
    let k = m.const_u32(c);
    m.spec_op(u, o, &[a, k], &[]);
    m.finish()
}

pub fn run(t: &mut T) {
    // the builder per level
    let w3 = samples::spec_fill(3);
    let w6 = samples::spec_fill(6);
    let wg_builtin = |w: &[u32]| has_op(w, op::Decorate, |o| o.len() >= 3 && o[1] == Dec::BuiltIn as u32 && o[2] == Bi::WorkgroupSize as u32);
    let local_size_id = |w: &[u32]| has_op(w, op::ExecutionModeId, |o| o.get(1) == Some(&(Mode::LocalSizeId as u32)));
    t.check("1.3: the WorkgroupSize built-in, no LocalSizeId", wg_builtin(&w3) && !local_size_id(&w3));
    t.check("1.6: LocalSizeId, no WorkgroupSize built-in", local_size_id(&w6) && !wg_builtin(&w6));
    t.eq("uses_local_size_id 1.3 / 1.6", (reflect::uses_local_size_id(&w3), reflect::uses_local_size_id(&w6)), (false, true));
    t.check("OpSpecConstantOp in spec_fill", has_op(&w3, op::SpecConstantOp, |_| true));
    // the spec constants of spec_fill
    let d = SpecParams::DEFAULT;
    let sc = reflect::spec_constants(&w3).unwrap_or_default();
    let got: Vec<(u32, Kind, u64)> = sc.iter().map(|c| (c.spec_id, c.kind, c.default)).collect();
    t.eq("spec_constants", got, alloc::vec![
        (0, Kind::Int { width: 32, signed: false }, d.wg as u64),
        (1, Kind::Int { width: 32, signed: false }, d.mul as u64),
        (2, Kind::Int { width: 32, signed: true }, d.add as u32 as u64),
        (3, Kind::Float { width: 32 }, d.scale.to_bits() as u64),
        (4, Kind::Bool, 0),
    ]);
    // workgroup sizes: defaults, overrides, other entry points
    for (w, lv) in [(&w3, "1.3"), (&w6, "1.6")] {
        t.eq(&format!("workgroup_size {} default", lv), reflect::workgroup_size(w, "main", &[]), Ok(Some([64, 1, 1])));
        t.eq(&format!("workgroup_size {} SPEC_WG = 32", lv), reflect::workgroup_size(w, "main", &[(samples::SPEC_WG, 32)]), Ok(Some([32, 1, 1])));
        t.eq(&format!("workgroup_size {} unknown entry point", lv), reflect::workgroup_size(w, "other", &[]), Ok(None));
    }
    t.eq("workgroup_size of LocalSize", reflect::workgroup_size(&crate::basics::doc_example(), "main", &[]), Ok(Some([64, 1, 1])));
    let eps = reflect::entry_points(&w3).unwrap_or_default();
    t.check("entry_points", eps.len() == 1 && eps[0].name == "main" && eps[0].model == Model::GLCompute as u32);
    // every operation against Rust, default values and a changed one
    let (w, cases) = ops_module();
    match reflect::constant_values(&w, &[]) {
        Ok(vals) => {
            for (name, id, want) in &cases {
                t.eq(&format!("OpSpecConstantOp {}", name), vals.get(&id.0).cloned(), Some(want.clone()));
            }
        }
        Err(e) => t.check(&format!("constant_values: {}", e), false),
    }
    if let Ok(vals) = reflect::constant_values(&w, &[(0, 9), (2, 0)]) {
        let get = |name: &str| cases.iter().find(|c| c.0 == name).and_then(|c| vals.get(&c.1 .0).cloned());
        t.eq("IAdd with spec 0 = 9", get("IAdd"), Some(alloc::vec![12]));
        t.eq("Select with spec 2 = false", get("Select"), Some(alloc::vec![3]));
        t.eq("VectorShuffle with spec 0 = 9", get("VectorShuffle"), Some(alloc::vec![5, 9]));
    }
    t.check("the operations module validates", shader::val::validate(&w).is_ok());
    // what SPIR-V leaves undefined is an error
    t.check("UDiv by 0: Err", reflect::constant_values(&one_op(op::UDiv, 0), &[]).is_err());
    t.check("UMod by 0: Err", reflect::constant_values(&one_op(op::UMod, 0), &[]).is_err());
    t.check("shift by 32: Err", reflect::constant_values(&one_op(op::ShiftLeftLogical, 32), &[]).is_err());
    t.check("shift by 31: Ok", reflect::constant_values(&one_op(op::ShiftLeftLogical, 31), &[]).is_ok());
    t.check("FAdd (no Shader operation): Err", reflect::constant_values(&one_op(op::FAdd, 1), &[]).is_err());
    // half conversions
    for (x, h) in [(1.0f32, 0x3c00u16), (-2.0, 0xc000), (65504.0, 0x7bff), (65520.0, 0x7c00), (5.960_464_5e-8, 0x0001),
                   (2.980_232_2e-8, 0x0000), (8.940_697e-8, 0x0002), (-0.0, 0x8000), (f32::INFINITY, 0x7c00),
                   (0.333_333_34, 0x3555), (6.103_515_6e-5, 0x0400)] {
        t.eq(&format!("f16_from_f32({:e})", x), reflect::f16_from_f32(x), h);
    }
    t.check("f16_from_f32(NaN) is a NaN", reflect::f16_from_f32(f32::NAN) & 0x7c00 == 0x7c00 && reflect::f16_from_f32(f32::NAN) & 0x3ff != 0);
    let bad = (0..=0xffffu32).map(|h| h as u16).filter(|h| h & 0x7c00 != 0x7c00 || h & 0x3ff == 0)
        .find(|&h| reflect::f16_from_f32(reflect::f16_to_f32(h)) != h);
    t.eq("every half round-trips through f32", bad, None);
    // the validator accepts every sample module
    for (name, w) in crate::basics::samples() {
        match shader::val::validate(&w) {
            Ok(()) => t.check("", true),
            Err(e) => {
                for x in e.iter().take(5) {
                    crate::out(&format!("   {}: {}\n", name, x));
                }
                t.check(&format!("{} validates", name), false);
            }
        }
    }
    // spec_fill's disassembly (OpSpecConstant*, OpSpecConstantOp, LocalSizeId) = the golden files
    for (w, golden, lv) in [(&w3, include_str!("../golden/spec_fill.spvasm"), "1.3"),
                            (&w6, include_str!("../golden/spec_fill_16.spvasm"), "1.6")] {
        let got = dis::disassemble(w).unwrap_or_default();
        let want = golden.replace('\r', "");
        if got != want {
            let line = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or(0);
            crate::out(&format!("   spec_fill {} line {}: got {:?}, golden {:?}\n", lv, line + 1,
                                got.lines().nth(line).unwrap_or(""), want.lines().nth(line).unwrap_or("")));
        }
        t.check(&format!("spec_fill {} disassembly = golden", lv), got == want);
    }
    // the samples twice: the same words
    let a = crate::basics::samples();
    let b = crate::basics::samples();
    t.check("the samples built twice are equal", a.iter().zip(&b).all(|(x, y)| x.1 == y.1) && a.len() == b.len());
    crate::out(&format!("   samples: {} modules, {} words\n", a.len(), a.iter().map(|x| x.1.len()).sum::<usize>()));
}

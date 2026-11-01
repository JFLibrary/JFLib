// t_spec -- gputest's test of pipeline constants (WGSL override, SPIR-V specialization
// constants; main.rs lists it)
//
//   spec   shader::samples::spec_fill on the GPU as SPIR-V 1.3 (the WorkgroupSize built-in) and
//          1.6 (LocalSizeId, needs maintenance4): with the module's defaults and with pipeline
//          constants for every override (workgroup sizes 1 to the device's limit, which also
//          sizes a workgroup array), each = spec_fill_ref word for word; ShaderModule::constants;
//          constants the module does not declare, twice, out of their type's range, NaN, a
//          workgroup size of 0 or beyond the limits, a missing entry point: Err(Invalid);
//          WebGPU's conversions (1.9 for a u32 is 1, 0.5 for a bool is true)

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use shader::reflect::Kind;
use shader::samples::{self, SpecParams};

use crate::t_compute::{GUARD, readback, storage};
use crate::{T, out, run_wait, setup};

const N: u32 = 1000;

/// the pipeline constants of p
fn constants(p: &SpecParams) -> Vec<Constant> {
    vec![
        Constant { id: samples::SPEC_WG, value: p.wg as f64 },
        Constant { id: samples::SPEC_MUL, value: p.mul as f64 },
        Constant { id: samples::SPEC_ADD, value: p.add as f64 },
        Constant { id: samples::SPEC_SCALE, value: p.scale as f64 },
        Constant { id: samples::SPEC_FLIP, value: if p.flip { 1.0 } else { 0.0 } },
    ]
}

/// run spec_fill with these constants over N invocations -> the N * 4 words and the guard words
fn run_fill(t: &mut T, dev: &Device, m: &ShaderModule, layout: &PipelineLayout, bgl: &BindGroupLayout, c: &[Constant],
            wg: u32, what: &str) -> Option<(Vec<u32>, Vec<u32>)> {
    let pipe = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout, shader: ShaderStage { module: m, entry: "main",
                                                                                                    constants: c } }),
                     &format!("{what}: pipeline"))?;
    let words = (N + 4) * 4;
    let buf = storage(t, dev, words as u64 * 4, BufferUsage::NONE)?;
    let rb = readback(t, dev, words as u64 * 4)?;
    let bg = t.res(dev.create_bind_group(&BindGroupDesc { layout: bgl, entries: &[BindEntry { binding: 0,
        resource: Resource::Buffer { buffer: &buf, offset: 0, size: 0 } }] }), "bind group")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.fill_buffer(&buf, 0, words as u64 * 4, GUARD);
    e.barrier(Access::COPY_WRITE, Access::COMPUTE_WRITE);
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, &bg);
    e.push_constants(0, &[N]);
    e.dispatch(N.div_ceil(wg), 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&buf, 0, &rb, 0, words as u64 * 4);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    run_wait(t, dev, e.finish(), what)?;
    let mut got = vec![0u32; words as usize];
    t.res(rb.read(0, &mut got), "read")?;
    let guard = got.split_off(N as usize * 4);
    Some((got, guard))
}

pub fn spec(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    let lim = *dev.limits();
    let Some(bgl) = t.res(dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer,
                                                                   stages: Stages::COMPUTE }]), "layout") else { return };
    let Some(layout) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 4 }), "layout")
    else { return };
    let max_wg = lim.max_compute_workgroup_size[0].min(lim.max_compute_invocations).min(1024);
    let api = dev.info().api_version;
    for minor in [3u32, 6] {
        let lv = if minor == 3 { "SPIR-V 1.3 (WorkgroupSize built-in)" } else { "SPIR-V 1.6 (LocalSizeId)" };
        if minor == 6 && (api.0, api.1) < (1, 3) {
            out(&format!("SKIP {lv}: Vulkan {}.{}", api.0, api.1));
            continue;
        }
        let Some(m) = t.res(ShaderModule::from_spirv(dev, &samples::spec_fill(minor)), "spec_fill module") else { return };
        let d = SpecParams::DEFAULT;
        let want_consts = vec![(0, Kind::Int { width: 32, signed: false }, d.wg as f64),
                               (1, Kind::Int { width: 32, signed: false }, d.mul as f64),
                               (2, Kind::Int { width: 32, signed: true }, d.add as f64),
                               (3, Kind::Float { width: 32 }, d.scale as f64), (4, Kind::Bool, 0.0)];
        t.ok(m.constants() == want_consts, &format!("{lv}: ShaderModule::constants = the five overrides with their defaults"));
        // the defaults, then every override set
        let cases: Vec<(Option<SpecParams>, &str)> = vec![
            (None, "defaults"),
            (Some(SpecParams { wg: 1, mul: 2, add: -1, scale: 1.0, flip: true }), "wg 1"),
            (Some(SpecParams { wg: 32, mul: 7, add: 1000, scale: -1.25, flip: true }), "wg 32"),
            (Some(SpecParams { wg: 256, mul: 0xFFFF_FFFF, add: i32::MIN, scale: 3.0e-3, flip: false }), "wg 256"),
            (Some(SpecParams { wg: max_wg, mul: 12345, add: i32::MAX, scale: 65536.5, flip: false }), "wg = the limit"),
        ];
        for (p, name) in cases {
            let c = p.map_or(Vec::new(), |p| constants(&p));
            let want_p = p.unwrap_or(d);
            let what = format!("{lv}, {name}");
            let Some((got, guard)) = run_fill(t, dev, &m, &layout, &bgl, &c, want_p.wg, &what) else { continue };
            let want = samples::spec_fill_ref(&want_p, N);
            let bad = (0..want.len()).find(|&k| got[k] != want[k]);
            let at = bad.map_or(alloc::string::String::new(), |k| format!(": word {k} = {:#x}, expected {:#x}", got[k], want[k]));
            t.ok(bad.is_none(), &format!("{what}: {N} invocations = spec_fill_ref ({:?}){at}", want_p));
            t.ok(guard.iter().all(|&w| w == GUARD), &format!("{what}: nothing written past n"));
        }
        // WebGPU's conversions: 1.9 -> 1 for a u32, 0.5 -> true for a bool
        let conv = [Constant { id: samples::SPEC_MUL, value: 1.9 }, Constant { id: samples::SPEC_FLIP, value: 0.5 },
                    Constant { id: samples::SPEC_ADD, value: -2.7 }];
        if let Some((got, _)) = run_fill(t, dev, &m, &layout, &bgl, &conv, d.wg, &format!("{lv}, conversions")) {
            let want = samples::spec_fill_ref(&SpecParams { mul: 1, flip: true, add: -2, ..d }, N);
            t.ok(got == want, &format!("{lv}: u32 1.9 = 1, bool 0.5 = true, i32 -2.7 = -2 (truncated as WebGPU does)"));
        }
        // what lib/gpu refuses
        let bad: Vec<(Vec<Constant>, &str)> = vec![
            (vec![Constant { id: 9, value: 1.0 }], "an id the module lacks"),
            (vec![Constant { id: 1, value: 1.0 }, Constant { id: 1, value: 2.0 }], "the same id twice"),
            (vec![Constant { id: samples::SPEC_MUL, value: -1.0 }], "u32 -1"),
            (vec![Constant { id: samples::SPEC_MUL, value: 4294967296.0 }], "u32 2^32"),
            (vec![Constant { id: samples::SPEC_ADD, value: 2147483648.0 }], "i32 2^31"),
            (vec![Constant { id: samples::SPEC_SCALE, value: 1.0e39 }], "f32 1e39"),
            (vec![Constant { id: samples::SPEC_SCALE, value: f64::NAN }], "f32 NaN"),
            (vec![Constant { id: samples::SPEC_MUL, value: f64::INFINITY }], "u32 infinity"),
            (vec![Constant { id: samples::SPEC_WG, value: 0.0 }], "workgroup size 0"),
            (vec![Constant { id: samples::SPEC_WG, value: lim.max_compute_workgroup_size[0] as f64 + 1.0 }],
             "workgroup size beyond max_compute_workgroup_size"),
        ];
        for (c, name) in bad {
            let r = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage { module: &m,
                                                                                 entry: "main", constants: &c } });
            t.ok(matches!(r, Err(GpuError::Invalid(_))), &format!("{lv}: {name}: Invalid ({:?})", r.as_ref().err()));
        }
        let r = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage::new(&m, "nope") });
        t.ok(matches!(r, Err(GpuError::Invalid(_))), &format!("{lv}: an entry point the module lacks: Invalid"));
    }
}

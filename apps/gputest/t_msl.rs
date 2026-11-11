// t_msl -- (AP6) every shader of the repository through the MSL translation (shader::msl): lib/wgsl
// compiles each WGSL source at both levels (the benchmark shaders, gpu_out.wgsl and render3d's
// shaders robust and not, tools/wgsl_corpus when the program runs in the repository), plus
// lib/shader's own modules (samples, benchmark).  On Metal, ShaderModule::from_spirv translates
// each module and Metal compiles it; elsewhere only the translation runs.  Modules the
// translation refuses on purpose (colour location 8, Bindless) count as expected.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{T, out};

const LIB_WGSL: &[(&str, &str)] = &[
    ("gpu_out", include_str!("../../lib/ui/gpu_out.wgsl")),
    ("render3d_cull", include_str!("../../lib/render3d/cull.wgsl")),
    ("render3d_draw", include_str!("../../lib/render3d/draw.wgsl")),
    ("render3d_hiz", include_str!("../../lib/render3d/hiz.wgsl")),
    ("render3d_scatter", include_str!("../../lib/render3d/scatter.wgsl")),
];

#[cfg(not(target_os = "android"))]
const CORPUS: &str = "tools/wgsl_corpus";

/// what the translation refuses on purpose: (name part, why)
const EXPECTED: &[(&str, &str)] = &[
    ("fragment_location_8", "colour location 8: Metal has 0 ..= 7"),
    ("cs_bindless_sample", "a runtime array of textures (Bindless, level Voll)"),
];

#[cfg(target_os = "android")]
fn corpus() -> Vec<(String, String)> {
    Vec::new() // (no repository on a phone; the test is for Apple anyway)
}

#[cfg(not(target_os = "android"))]
fn corpus() -> Vec<(String, String)> {
    let mut v = Vec::new();
    let Some(dirs) = plat::fs::read_dir(CORPUS) else { return v };
    let mut dirs: Vec<String> = dirs.into_iter().filter(|e| e.kind == plat::fs::FileKind::Dir).map(|e| e.name).collect();
    dirs.sort();
    for d in dirs {
        let Some(files) = plat::fs::read_dir(&plat::fs::join(CORPUS, &d)) else { continue };
        let mut names: Vec<String> = files.into_iter().map(|e| e.name).filter(|n| n.ends_with(".wgsl")).collect();
        names.sort();
        for n in names {
            if let Some(b) = plat::read_file(&plat::fs::join(&plat::fs::join(CORPUS, &d), &n)) {
                v.push((format!("corpus/{}/{}", d, n), String::from_utf8_lossy(&b).into_owned()));
            }
        }
    }
    v
}

/// (name, SPIR-V) of every module
fn modules() -> (Vec<(String, Vec<u32>)>, usize) {
    use shader::Fmt;
    use shader::samples::{self as s, SampleDim, Texel};
    let mut srcs: Vec<(String, String, bool)> = Vec::new();
    for (n, src) in wgsl::bench::ALL {
        srcs.push((format!("bench/{}", n), String::from(src), false));
    }
    for (n, src) in LIB_WGSL {
        srcs.push((String::from(*n), String::from(*src), true));
    }
    let corpus = corpus();
    let ncorpus = corpus.len();
    for (n, src) in corpus {
        srcs.push((n, src, false));
    }
    let mut v: Vec<(String, Vec<u32>)> = Vec::new();
    for (name, src, both) in &srcs {
        for minor in [shader::BASIS, shader::VOLL] {
            for robust in [true, false] {
                if !robust && !both {
                    continue;
                }
                let mut opt = wgsl::CompileOptions::new(minor);
                opt.robust = robust;
                let Ok(o) = wgsl::compile(src, &opt) else { continue };
                for e in o.entries {
                    v.push((format!("{}{} {} 1.{}", name, if robust { "" } else { " (plain)" }, e.name, minor), e.words));
                }
            }
        }
    }
    for minor in [shader::BASIS, shader::VOLL] {
        v.push((format!("samples spec_fill 1.{}", minor), s::spec_fill(minor)));
        v.push((format!("samples vs_fullscreen 1.{}", minor), s::vs_fullscreen(minor)));
        v.push((format!("samples fs_sample 1.{}", minor), s::fs_sample(minor)));
        v.push((format!("samples fs_compare 1.{}", minor), s::fs_compare(minor)));
        v.push((format!("samples cs_query 1.{}", minor), s::cs_query(minor)));
        for k in [Texel::Float, Texel::Uint, Texel::Sint] {
            for arrayed in [false, true] {
                v.push((format!("samples cs_fetch {:?} {} 1.{}", k, arrayed, minor), s::cs_fetch(minor, k, arrayed, false)));
            }
        }
        v.push((format!("samples cs_fetch depth 1.{}", minor), s::cs_fetch(minor, Texel::Float, false, true)));
        for d in [SampleDim::D2, SampleDim::D2Array, SampleDim::Cube, SampleDim::CubeArray] {
            v.push((format!("samples cs_sample {:?} 1.{}", d, minor), s::cs_sample(minor, d)));
        }
        for f in [Fmt::Rgba8, Fmt::Rgba16f, Fmt::R32f, Fmt::Rgba32f, Fmt::R32ui, Fmt::Rgba32ui, Fmt::R32i] {
            v.push((format!("samples cs_storage_write {:?} 1.{}", f, minor), s::cs_storage_write(minor, f)));
            v.push((format!("samples cs_storage_read {:?} 1.{}", f, minor), s::cs_storage_read(minor, f)));
        }
        for f in [Fmt::R32f, Fmt::R32ui, Fmt::R32i] {
            v.push((format!("samples cs_storage_rw {:?} 1.{}", f, minor), s::cs_storage_rw(minor, f)));
        }
        v.push((format!("bench move_spv 1.{}", minor), shader::bench::move_spv(minor)));
        v.push((format!("bench cull_spv 1.{}", minor), shader::bench::cull_spv(minor)));
        v.push((format!("bench cull_reset_spv 1.{}", minor), shader::bench::cull_reset_spv(minor)));
        v.push((format!("bench vs_instanced_spv 1.{}", minor), shader::bench::vs_instanced_spv(minor)));
        v.push((format!("bench vs_single_spv 1.{}", minor), shader::bench::vs_single_spv(minor)));
        v.push((format!("bench fs_lambert_spv 1.{}", minor), shader::bench::fs_lambert_spv(minor)));
    }
    v.push((String::from("samples cs_bindless_sample 1.6"), s::cs_bindless_sample()));
    (v, ncorpus)
}

pub fn msl(t: &mut T) {
    let (mods, ncorpus) = modules();
    out(&format!("  {} modules ({} corpus files{})", mods.len(), ncorpus, if ncorpus == 0 { ": not found, run in the repository" } else { "" }));
    #[cfg(target_vendor = "apple")]
    let Some(g) = crate::setup(t) else { return };
    let (mut ok, mut expected, mut failed) = (0u32, 0u32, 0u32);
    for (name, words) in &mods {
        #[cfg(target_vendor = "apple")]
        let r = gpu::ShaderModule::from_spirv(&g.dev, words).map(|_| ()).map_err(|e| format!("{}", e));
        #[cfg(not(target_vendor = "apple"))]
        let r = shader::msl::translate(words, &shader::msl::Options::DEFAULT).map(|_| ());
        match r {
            Ok(()) => ok += 1,
            Err(e) => {
                if EXPECTED.iter().any(|(k, _)| name.contains(k)) {
                    expected += 1;
                } else {
                    failed += 1;
                    if failed <= 20 {
                        out(&format!("    {}: {}", name, e));
                    }
                }
            }
        }
    }
    let what = if cfg!(target_vendor = "apple") { "translated and compiled by Metal" } else { "translated (no Metal here)" };
    t.ok(failed == 0, &format!("{} of {} modules {}, {} expected refusals, {} failed", ok, mods.len(), what, expected, failed));
}

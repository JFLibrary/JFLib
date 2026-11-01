// bench -- the six benchmark shaders of lib/wgsl/bench pass the front end and have the
// interfaces of the data contract (docs/SHADER.md "Benchmark-Vertrag"): stage, workgroup
// size, bindings and their address spaces / access modes, the immediate block, stage inputs
// and outputs, struct layouts (Object 48, Camera 176, DrawCmd 20, CubeVertex 32, Push 20)

use alloc::format;
use alloc::vec::Vec;

use wgsl::ir::{self, AddressSpace, Access, BuiltinValue, InterpSampling, InterpType, IoKind, Module, Stage, WgSize};

use crate::T;

fn layout(t: &mut T, m: &Module, name: &str, size: u32, offs: &[u32]) {
    match m.struct_named(name) {
        Some(s) => {
            t.eq(&format!("{} size", name), s.size, size);
            let got: Vec<u32> = s.members.iter().map(|x| x.offset).collect();
            t.eq(&format!("{} offsets", name), got.as_slice(), offs);
        }
        None => t.check(&format!("{} declared", name), false),
    }
}

/// (binding, space, access) of the resources an entry point uses
fn bindings(m: &Module, ep: &ir::EntryPoint) -> Vec<(u32, AddressSpace, Access)> {
    ep.resources.iter().map(|&(g, b)| (b.binding, m.globals[g as usize].space, m.globals[g as usize].access)).collect()
}

pub fn run(t: &mut T) {
    for (name, src) in wgsl::bench::ALL {
        let Some(m) = t.pass(name, src) else { continue };
        t.eq(&format!("{} one entry point", name), m.entry_points.len(), 1);
        let Some(ep) = m.entry_point("main") else {
            t.check(&format!("{} main", name), false);
            continue;
        };
        if m.struct_named("Object").is_some() {
            layout(t, &m, "Object", 48, &[0, 12, 16, 32, 44]);
        }
        if m.struct_named("Camera").is_some() {
            layout(t, &m, "Camera", 176, &[0, 64, 160, 172]);
            let g = m.global("camera");
            t.check(&format!("{} camera std140", name), g.is_some_and(|g| g.std140));
        }
        if m.struct_named("Push").is_some() {
            layout(t, &m, "Push", 20, &[0, 4, 8, 12, 16]);
        }
        if m.struct_named("DrawCmd").is_some() {
            layout(t, &m, "DrawCmd", 20, &[0, 4, 8, 12, 16]);
        }
        if m.struct_named("CubeVertex").is_some() {
            layout(t, &m, "CubeVertex", 32, &[0, 16]);
        }
        let b = bindings(&m, ep);
        let imm = ep.immediate.map(|g| m.globals[g as usize].name.as_str());
        match name {
            "move" => {
                t.eq("move stage", ep.stage, Stage::Compute);
                t.eq("move wg", ep.workgroup_size, Some([WgSize::Const(256), WgSize::Const(1), WgSize::Const(1)]));
                t.eq("move bindings", b, alloc::vec![(1, AddressSpace::Storage, Access::ReadWrite)]);
                t.eq("move immediate", imm, Some("pc"));
                t.eq("move inputs", ep.inputs.iter().map(|i| i.kind).collect::<Vec<_>>(), alloc::vec![IoKind::Builtin(BuiltinValue::GlobalInvocationId)]);
            }
            "cull" => {
                t.eq("cull wg", ep.workgroup_size, Some([WgSize::Const(256), WgSize::Const(1), WgSize::Const(1)]));
                t.eq(
                    "cull bindings",
                    b,
                    alloc::vec![
                        (0, AddressSpace::Uniform, Access::Read),
                        (1, AddressSpace::Storage, Access::Read),
                        (2, AddressSpace::Storage, Access::ReadWrite),
                        (3, AddressSpace::Storage, Access::ReadWrite)
                    ],
                );
                t.eq("cull immediate", imm, Some("pc"));
            }
            "cull_reset" => {
                t.eq("cull_reset wg", ep.workgroup_size, Some([WgSize::Const(1), WgSize::Const(1), WgSize::Const(1)]));
                t.eq("cull_reset bindings", b, alloc::vec![(3, AddressSpace::Storage, Access::ReadWrite)]);
                t.eq("cull_reset immediate", imm, None);
            }
            "vs_instanced" | "vs_single" => {
                t.eq(&format!("{} stage", name), ep.stage, Stage::Vertex);
                let want: Vec<(u32, AddressSpace, Access)> = if name == "vs_instanced" {
                    alloc::vec![
                        (0, AddressSpace::Uniform, Access::Read),
                        (1, AddressSpace::Storage, Access::Read),
                        (2, AddressSpace::Storage, Access::Read),
                        (4, AddressSpace::Storage, Access::Read)
                    ]
                } else {
                    alloc::vec![(0, AddressSpace::Uniform, Access::Read), (1, AddressSpace::Storage, Access::Read), (4, AddressSpace::Storage, Access::Read)]
                };
                t.eq(&format!("{} bindings", name), b, want);
                t.eq(&format!("{} immediate", name), imm, if name == "vs_single" { Some("pc") } else { None });
                let ins: Vec<IoKind> = ep.inputs.iter().map(|i| i.kind).collect();
                let want_in = if name == "vs_instanced" {
                    alloc::vec![IoKind::Builtin(BuiltinValue::VertexIndex), IoKind::Builtin(BuiltinValue::InstanceIndex)]
                } else {
                    alloc::vec![IoKind::Builtin(BuiltinValue::VertexIndex)]
                };
                t.eq(&format!("{} inputs", name), ins, want_in);
                let outs: Vec<IoKind> = ep.outputs.iter().map(|i| i.kind).collect();
                t.eq(
                    &format!("{} outputs", name),
                    outs,
                    alloc::vec![
                        IoKind::Builtin(BuiltinValue::Position),
                        IoKind::Location { location: 0, blend_src: None, interp: Some((InterpType::Perspective, InterpSampling::Center)) },
                        IoKind::Location { location: 1, blend_src: None, interp: Some((InterpType::Flat, InterpSampling::First)) },
                    ],
                );
                t.eq(&format!("{} output names", name), ep.outputs.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), alloc::vec!["return.position", "return.normal", "return.color"]);
            }
            _ => {
                t.eq("fs stage", ep.stage, Stage::Fragment);
                t.eq("fs bindings", b, alloc::vec![(0, AddressSpace::Uniform, Access::Read)]);
                let ins: Vec<IoKind> = ep.inputs.iter().map(|i| i.kind).collect();
                t.eq(
                    "fs inputs",
                    ins,
                    alloc::vec![
                        IoKind::Location { location: 0, blend_src: None, interp: Some((InterpType::Perspective, InterpSampling::Center)) },
                        IoKind::Location { location: 1, blend_src: None, interp: Some((InterpType::Flat, InterpSampling::First)) },
                    ],
                );
                t.eq("fs outputs", ep.outputs.iter().map(|i| i.kind).collect::<Vec<_>>(), alloc::vec![IoKind::Location { location: 0, blend_src: None, interp: None }]);
            }
        }
        if t.verbose {
            crate::out(&format!("{}: {} types, {} exprs in main\n", name, m.types.len(), m.function("main").map(|f| f.exprs.len()).unwrap_or(0)));
        }
    }
}

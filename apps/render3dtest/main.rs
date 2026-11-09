// render3dtest -- tests and measurements of lib/render3d (docs/RENDER3D.md), offscreen except `window`.
//
//   render3dtest --test NAME [--validate]    a test on the device's own level and on one limited to Basis:
//        shaders     wgsl::compile of lib/render3d/*.wgsl = the embedded SPIR-V, word for word
//        pixels      simple scenes against a ray caster on the CPU (cpu.rs): colour, lighting, the normals of
//                    a scaled ball, back faces, the depth buffer's values and order with reverse Z, also far away
//        visibility  the compute shader's lists against the exact test on the CPU, for random scenes with a
//                    fixed seed and for objects exactly at each face of the frustum; culled = not culled
//        changes     adding, moving, removing, places used again, material and mesh changed: the image =
//                    that of a scene built anew; only what changed goes up; both ways of uploading
//        many        many meshes and materials in a frame, pools and buffers growing, against the ray caster
//        levels      Basis and Voll draw the same image, byte for byte
//        coords      a scene at the origin and at 1e7 with the camera relative to it: the same image; and
//                    what the origin is for: without it the image falls apart
//        frames      frames in flight with their own cameras and changes, against each drawn alone
//        errors      what the API refuses
//        textures    (G5b) nearest texels, levels and wrap modes exactly, sRGB and the levels against a box
//                    filter of its own (read back too), trilinear within a tolerance, the five size classes,
//                    40 textures in one list, arrays growing, resampled sizes, MASK (tex.rs)
//        mirror      (G5b) mirrored instances against the ray caster, their lists, turned at run time, seen
//                    from inside, their visibility (tex.rs)
//        gltf        (G5b) lib/gltf against files of its own: three forms, contents, broken files, the scene
//                    drawn against the ray caster (gltftest.rs, gltfgen.rs)
//        pbr         (G5c) glTF's BRDF against the ray caster: materials, the metallic-roughness texture, normal maps
//                    (turned, mirrored, mirrored uv, which way they lean), emission, occlusion, exposure, both tone
//                    mappers, the light itself in Rgba16Float, a white furnace; the BRDF's albedo on the CPU (pbr.rs)
//        shadows     (G5c) the sun's cascaded shadow maps against exact shadow rays, at the origin and at 1e7,
//                    MASK with and without the shadow pass's fragment shader, the cascades' snapping on the CPU
//                    and in the map read back (shadowtest.rs)
//        lights      (G5c) point and spot lights against the ray caster (falloff, range, cone; 64 at once, HDR,
//                    changes, 1e7, refusals) and KHR_lights_punctual in lib/gltf: read, placed, drawn, broken (lighttest.rs)
//        env         (G5c) the environment's light: the sky's harmonics and prefiltered levels on the CPU, balls in the
//                    sky against the ray caster, a white furnace, the caller's faces, switched off (envtest.rs)
//        occlusion   (G5d) the camera's occlusion test: with it = without it over a flight with a jump, instances coming
//                    and going, a new origin and size, at 0 and at 1e7, against the ray caster and what the rays see; the
//                    pyramid against the depth buffer's minima; the renderer's own depth buffer (occl.rs)
//        all         all of the above
//        window      a real window (one, briefly; not in `all`): the presented image = the offscreen image
//   render3dtest --bench N [--res 1080p|4k|WxH] [--frames F] [--warmup W] [--inflight K] [--basis]
//                [--moved K] [--meshes M] [--textures T [--tex-size S]] [--verify]    the scene of gputest
//                --bench over the engine (bench.rs); T textures of S texels on the cubes' materials
//   render3dtest --bench-upload              texture upload and mip levels per size class (bench.rs)
//   render3dtest --bench-gltf [--nodes N | --vertices V]   a generated glTF: load time, memory (bench.rs)
//   render3dtest --gen-shaders FILE          lib/render3d/shaders_spv.rs from lib/render3d/*.wgsl
//   render3dtest --gen-dfg FILE              lib/render3d/dfg_table.rs: the split-sum's table (render3d::dfg_table)
//   render3dtest --dump DIR                  the gallery's images per level, for tests/engine_web.mjs
//   In the browser: the same gallery, step by step as the browser answers (web.rs; tests/engine_web.mjs).
//
// Without Vulkan or a usable GPU every test prints SKIP and passes.  Exit code 1 when a check failed.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

// (the browser's test uses the part of them that it needs)
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod common;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod cpu;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod gltfgen;

#[cfg(not(target_arch = "wasm32"))]
mod bench;
#[cfg(not(target_arch = "wasm32"))]
mod gltftest;
#[cfg(not(target_arch = "wasm32"))]
mod pbr;
#[cfg(not(target_arch = "wasm32"))]
mod envtest;
#[cfg(not(target_arch = "wasm32"))]
mod lighttest;
#[cfg(not(target_arch = "wasm32"))]
mod occl;
#[cfg(not(target_arch = "wasm32"))]
mod shadowtest;
#[cfg(not(target_arch = "wasm32"))]
mod tests;
#[cfg(not(target_arch = "wasm32"))]
mod tex;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
rt::main!(tests::run());
#[cfg(target_arch = "wasm32")]
rt::main!(web::start());

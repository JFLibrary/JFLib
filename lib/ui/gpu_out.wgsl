// The shaders of the UI's GPU output (lib/ui/gpu_out.rs): the window's textures to the swapchain
// image, pixel for pixel, opaque, no blending.  lib/ui embeds them precompiled (gpu_out_spv.rs: UI
// programs do not link lib/wgsl); uigputest --test shaders compares the words with what
// wgsl::compile makes of this file (shader::BASIS, no robustness: only pixels of the texture are
// reached, the scissor keeps the draws inside), uigputest --gen-shaders writes gpu_out_spv.rs again.
@group(0) @binding(0) var frame: texture_2d<f32>;

// where the texture's first pixel lies in the window; an image's steps (16.16 source pixels per
// window pixel: Dc::blit_scaled).  A scaled tile: x, y in 24.8, its steps the same way
struct Place {
    x: i32,
    y: i32,
    step_x: u32,
    step_y: u32,
}

var<immediate> place: Place;

// a triangle over the whole target
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// the pixel at this position; the framebuffer's fourth byte is no alpha, the window is opaque
@fragment
fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    return vec4f(textureLoad(frame, vec2i(p.xy) - vec2i(place.x, place.y), 0).rgb, 1.0);
}

// the source pixel of Dc::blit_scaled at this position: (d * step + step / 2) >> 16 in integers
// (d * step stays below the source's side << 16; one to one the step is 65536, the pixel d)
@fragment
fn fs_image(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let d = vec2u(vec2i(p.xy) - vec2i(place.x, place.y));
    let s = vec2u((d.x * place.step_x + (place.step_x >> 1u)) >> 16u, (d.y * place.step_y + (place.step_y >> 1u)) >> 16u);
    return vec4f(textureLoad(frame, s, 0).rgb, 1.0);
}

// a canvas's tile through a transform of the running gesture (lib/ui/canvas.rs): bilinear from four
// texels, no sampler; the tile's border of one texel keeps them inside and joins the tiles
@fragment
fn fs_tile(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let o = vec2f(f32(place.x), f32(place.y)) / 256.0;
    let t = (p.xy - o) * vec2f(f32(place.step_x), f32(place.step_y)) / 65536.0 - vec2f(0.5, 0.5);
    let f = floor(t);
    let w = t - f;
    let a = vec2i(f);
    let top = mix(textureLoad(frame, a, 0).rgb, textureLoad(frame, a + vec2i(1, 0), 0).rgb, w.x);
    let bottom = mix(textureLoad(frame, a + vec2i(0, 1), 0).rgb, textureLoad(frame, a + vec2i(1, 1), 0).rgb, w.x);
    return vec4f(mix(top, bottom, w.y), 1.0);
}

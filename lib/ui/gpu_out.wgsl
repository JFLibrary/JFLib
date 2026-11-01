// The shaders of the UI's GPU output (lib/ui/gpu_out.rs): the window's texture to the swapchain
// image, pixel for pixel.  lib/ui embeds them precompiled (gpu_out_spv.rs: UI programs do not
// link lib/wgsl); uigputest --test shaders compares the words with what wgsl::compile makes of
// this file (shader::BASIS, no robustness: the triangle only reaches pixels of the texture),
// uigputest --gen-shaders writes gpu_out_spv.rs again.
@group(0) @binding(0) var frame: texture_2d<f32>;

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
    return vec4f(textureLoad(frame, vec2i(p.xy), 0).rgb, 1.0);
}

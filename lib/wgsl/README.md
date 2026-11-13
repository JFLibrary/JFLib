# jflib-wgsl

The compiler for WGSL, the shading language of WebGPU, in [JFLib](https://jflib.org). It reads
WGSL text, checks it the way a browser does and translates it to SPIR-V for Vulkan; on macOS and
iOS, [`jflib-gpu`](https://crates.io/crates/jflib-gpu) translates that SPIR-V on to Metal. So an
app writes each shader once: the native targets run what this crate makes of it, and in the
browser WebGPU takes the same text. It is plain Rust, without the standard library and without a
line of `unsafe` code, and its only dependency is
[`jflib-shader`](https://crates.io/crates/jflib-shader), which writes the SPIR-V.

## Checking

The front end does what a browser's WGSL compiler does before it writes any code: lexer, parser,
the semantic checks of every declaration, statement and expression, constant evaluation, the
uniformity analysis and subgroups, ending in a fully typed module (`wgsl::check`, the IR in
`wgsl::ir`). Its yardstick is the W3C specification (the Candidate Recommendation of 2026-10-26).
Where Chrome's compiler judges differently, the crate follows the specification, and its
documentation lists the cases.

Errors come as `Diagnostic`s with line, column and end column (columns count characters), the
byte range, notes that point at related places, and German messages. `Diagnostic::render` adds the
source line with a caret under the place. One run reports several errors, and no input makes the
compiler panic: nesting deeper than its limits is an error, never a stack overflow, and constant
values are capped in size. `wgsl::parse` and `wgsl::print` read and write the syntax tree on its
own, so that printing and parsing again gives the same tree.

`Options::features` says which extensions the device offers; `feature::for_device` turns off what
it lacks, so `enable f16` or `enable subgroups` is an error on a device without them, as in
WebGPU.

## Compiling

`wgsl::compile` makes one SPIR-V module per entry point, for level `Basis` (SPIR-V 1.3, Vulkan 1.1)
or `Voll` (SPIR-V 1.6, Vulkan 1.3). Each entry point comes with what a pipeline needs: its
resources with group, binding, kind and minimum binding size, the push-constant bytes, the
workgroup size, its inputs and outputs and the device features it needs.

```rust
use jflib::{gpu, shader, wgsl};

match wgsl::compile(SRC, &wgsl::CompileOptions::new(shader::VOLL)) {
    Ok(out) => {
        let Some(e) = out.entry("main") else { return };
        let Ok(module) = gpu::ShaderModule::from_spirv(&dev, &e.words) else { return };
        // e.resources: group, binding, name, kind and minimum size for the bind group layouts;
        // e.immediate_size: the push-constant bytes; e.workgroup_size; e.needs
    }
    Err(diags) => {
        for d in &diags {
            let text = d.render(SRC); // the message, the source line and a caret
        }
    }
}
```

WebGPU's robustness is on by default: indices are clamped to their array, vector or matrix,
texture loads into the texture, and stores outside a texture are skipped, so a shader stays inside
its bindings as long as each buffer binding has at least its minimum size.
`CompileOptions::exact` forbids fused multiply-adds where results must be reproducible.

Overrides (WGSL's `override`) become specialization constants. `Output::constants` takes the
values as WebGPU's pipeline takes them, by name or by `@id`, evaluates every override, derived
ones with float arithmetic and built-in functions included, and gives the value of each constant
for `gpu::Constant`. `Output::workgroup_size` gives the size for `dispatch`, and
`Output::check_pipeline` and `wgsl::check_interface` make the checks WebGPU makes when it creates
a pipeline: overrides, limits, storage texture formats and the interface between the stages. With
`CompileOptions::constants` the values are compiled in instead.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::wgsl`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports. The `buffer_view`
extension is not supported yet; the documentation lists what else is missing.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-wgsl/latest/source/CHANGELOG.md) says the
same.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#wgsl): every public
  item of `wgsl`, also on [docs.rs](https://docs.rs/jflib-wgsl)
- [WGSL.md](https://github.com/JFLibrary/JFLib/blob/main/docs/WGSL.md) (in German): the checks,
  the IR, the SPIR-V back end, overrides, robustness, the differences to Chrome and what is
  missing

## License

MIT or Apache-2.0, at your choice.

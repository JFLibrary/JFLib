# jflib-shader

SPIR-V for [JFLib](https://jflib.org): a builder that writes SPIR-V modules from Rust, a
disassembler, a validator, the std140 and std430 layout rules, reflection of finished modules and
a translation to the Metal Shading Language. It is the back end of JFLib's WGSL compiler
[`jflib-wgsl`](https://crates.io/crates/jflib-wgsl), and it is how
[`jflib-gpu`](https://crates.io/crates/jflib-gpu) runs the same shaders on Apple's GPUs. Nothing
here needs a GPU: everything produces or reads words. It is plain Rust, without the standard
library, without a line of `unsafe` code and without dependencies.

## Building modules

`Module::new(minor)` starts a SPIR-V 1.minor module: `shader::BASIS` (1.3, for Vulkan 1.1) or
`shader::VOLL` (1.6, for Vulkan 1.3). The builder has the types, constants, global variables,
functions and structured control flow of a shader and the instructions for arithmetic, memory,
images, atomics, barriers and derivatives. It derives result types from the operands, writes each
plain type and constant once, adds the capabilities and extensions an instruction needs and
handles what changes between SPIR-V versions, such as an entry point's interface list. The same
calls always give the same words.

```rust
use jflib::shader::{self, Bi, Mode, Model, Module, Rule, Sc};

// a compute shader that squares every float of a storage buffer
let mut m = Module::new(shader::BASIS);
let (void, f, u) = (m.ty_void(), m.ty_f32(), m.ty_u32());
let arr = m.ty_runtime_array(f, Some(4));
let block = m.ty_struct_layout("Data", &[("values", arr)], Rule::Std430);
let data = m.buffer(Sc::StorageBuffer, block, 0, 0);
let uv3 = m.ty_vec(u, 3);
let gid = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
let (main, _) = m.begin_function(void, &[]);
let g = m.load(gid);
let i = m.extract(g, &[0]);
let c0 = m.const_u32(0);
let p = m.access(data, &[c0, i]);
let x = m.load(p);
let y = m.fmul(x, x);
m.store(p, y);
m.ret();
m.end_function();
m.entry_point(Model::GLCompute, main, "main");
m.exec_mode(main, Mode::LocalSize, &[64, 1, 1]);
let words: Vec<u32> = m.finish();          // shader::to_bytes(&words): the bytes of a .spv file
```

Specialization constants (WGSL's overrides) are built in as well, with workgroup sizes taken from
them, and `reflect` reads finished words: the entry points, the specialization constants with
their defaults, the values of all constants once a pipeline sets some, and the workgroup size.
jflib-gpu checks a pipeline's constants against the module with it.

## Reading and checking

`dis::disassemble` writes text in the style of `spirv-dis` from the Vulkan SDK, with the same
friendly names. `val::validate` checks what a module builder can get wrong: the header, the
grammar, the order of sections, ids, types, decorations, storage classes, block layouts, the
control flow graph, entry point interfaces, execution modes, capabilities and the rules of each
SPIR-V version. Each error names its rule and the word offset of the instruction. It does not
replace `spirv-val`; its documentation names what it leaves out. `layout` computes alignments,
sizes, strides and offsets after std140 and std430.

## Metal

`msl::translate` turns a module into the Metal Shading Language, together with what a backend
needs per entry point: the MSL function's name, the workgroup size, the slot of each resource,
and whether it reads push constants or buffer sizes. It reads the SPIR-V that jflib-wgsl and this
builder write. Specialization constants are substituted, arrays of images, samplers and storage
buffers become Metal 3 argument buffers, and the translation keeps WGSL's rules where Metal leaves
things undefined: integer overflow wraps, image reads outside an image are clamped into it and
stores outside it are dropped.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::shader`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports. Besides the general
parts it holds the shaders of JFLib's GPU benchmark scene with their data layout and CPU
references (`bench`) and small sample shaders for tests (`samples`).

## What's new in 0.2.0

The crate translates SPIR-V into the Metal Shading Language now (`shader::msl`). That is how
jflib-gpu runs the modules of jflib-wgsl and of this builder on macOS and iOS, unchanged. The
translation substitutes specialization constants, turns bindless arrays into Metal 3 argument
buffers and keeps WGSL's rules for overflow and image access outside the bounds. Everything is in
the [changelog](https://docs.rs/crate/jflib-shader/latest/source/CHANGELOG.md).

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#shader): every public
  item of `shader`, also on [docs.rs](https://docs.rs/jflib-shader)
- [SHADER.md](https://github.com/JFLibrary/JFLib/blob/main/docs/SHADER.md) (in German): the
  builder, specialization constants, the validator's rules, the Metal translation and what is
  missing

## License

MIT or Apache-2.0, at your choice.

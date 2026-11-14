# jflib-simd

The vector instructions under [JFLib](https://jflib.org)'s graphics and image code: SSE2 and AVX2
behind one safe interface, so that a kernel is written once, runs with the best instruction set
the processor has, and gives the same bits either way.

## A building block, not a promise

The crates that draw, blend, filter and decode images stand on this one: `gfx`, `fx`, `text`,
`img_core` and the image codecs. It is reachable as `jflib::simd`, but hidden there and no part of
what JFLib's version number promises
([VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md#the-crates-on-cratesio)):
its interface may change in any release. Use it if it fits your own kernels, knowing that.

## How it works

A kernel implements `simd::Kernel`, a computation generic over `simd::Simd`, the register type and
its operations, named after the SSE2 instructions they stand for. `simd::dispatch` runs it with
`Avx2` when the processor and the system support AVX2, otherwise with `Sse2`; the command-line
switch `--simd sse2` of a JFLib program keeps it at SSE2 to compare results. The results are equal
because `Avx2` uses its 256-bit registers as two independent 128-bit lanes: each lane computes
exactly what SSE2 computes for its data.

```rust
use jflib::simd::{self, Kernel, Simd};

/// dst = dst + src, byte by byte, saturating at 255
struct AddSat<'a> {
    dst: &'a mut [u8],
    src: &'a [u8],
}

impl Kernel for AddSat<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let AddSat { dst, src } = self;
        let nb = S::BYTES;                           // 16 with SSE2, 32 with AVX2
        let full = dst.len().min(src.len()) / nb * nb;
        let mut o = 0;
        while o < full {
            let v = s.addus8(s.load(&dst[o..o + nb]), s.load(&src[o..o + nb]));
            s.store(&mut dst[o..o + nb], v);
            o += nb;
        }
        for (d, &x) in dst[full..].iter_mut().zip(&src[full..]) {
            *d = d.saturating_add(x);
        }
    }
}

simd::dispatch(AddSat { dst: &mut a, src: &b });
```

The loads and stores check the lengths of their slices: a kernel that gets them wrong panics, it
never touches memory outside a slice. The crate holds the `unsafe` the intrinsics need, and the
`Avx2` type exists only after the processor check, so nothing unsafe reaches a kernel.

Besides the kernels there is `copy_stream`, a copy whose stores bypass the caches (on x86-64),
for large results that another thread reads later, and `copy_stream_rows` for a block of rows.

## Platforms

On x86-64 the kernels run with SSE2 or AVX2. On ARM64 NEON takes SSE2's place, in the browser
WebAssembly's SIMD128 when the app is built with it, and plain scalar code everywhere else. Each
of them gives SSE2's results bit for bit, so a kernel computes the same pixels on a phone, in the
browser and on a PC.

## How to get it

It comes along with [`jflib`](https://crates.io/crates/jflib), as `jflib::simd` (hidden in the
documentation, see above). All of JFLib's crates share one version and are released together.

## What's new in 0.2.0

`copy_stream` now ends with the fence its non-temporal stores need, so the copy is complete before
anyone touches it again, the copying thread included; before, the caller had to call
`stream_fence` itself. The new `copy_stream_rows` copies a block of rows with one fence at the
end. Everything is in the
[changelog](https://docs.rs/crate/jflib-simd/latest/source/CHANGELOG.md).

## Documentation

- [The source](https://github.com/JFLibrary/JFLib/blob/main/lib/simd/lib.rs): the crate's
  overview and the `Simd` trait with every operation; also on [docs.rs](https://docs.rs/jflib-simd)
- Not in JFLib's [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md), which
  covers what JFLib promises

## License

MIT or Apache-2.0, at your choice.

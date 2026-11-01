// t_wgsl -- gputest's behaviour test of lib/wgsl's back end (main.rs lists it)
//
//   wgsl   every operator and built-in function of WGSL's numeric core on runtime values: per
//          case a compute shader compiled by lib/wgsl reads typed inputs from a storage
//          buffer, evaluates the expression and writes the result's words; the reference is
//          lib/wgsl's own constant evaluator on the same values (`const a = ..; const r = EXPR;`,
//          read from Module::consts: WGSL defines const and runtime results alike, except the
//          float accuracy, so the tolerances are the spec's accuracy table).  Where a const
//          expression is an error but the runtime result is defined (integer division by 0 and
//          MIN / -1, shift amounts of the bit width or more), a Rust reference of WGSL's runtime
//          rule; float inputs whose result is NaN or infinite are not compared (the evaluator
//          rejects them).  NaN operands of min / max / clamp and the comparisons are reported,
//          not failed (Vulkan without SignedZeroInfNanPreserve need not keep NaN).  At both
//          levels: the device's (Voll: SPIR-V 1.6) and a device limited to Basis (SPIR-V 1.3).
//
// A case: argument types, result type, the expression in a, b, c and the input set.  Results
// compare word for word (integers, bool as 0 / 1, exact float functions; +0 = -0, a denormal
// expected result may come back flushed to 0) or within the tolerance.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use wgsl::ir::Value;

use crate::{T, out, setup};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    I32,
    U32,
    F32,
    Bool,
    V2F,
    V3F,
    V4F,
    V2I,
    V4I,
    V4U,
}

impl Ty {
    fn wgsl(self) -> &'static str {
        match self {
            Ty::I32 => "i32",
            Ty::U32 => "u32",
            Ty::F32 => "f32",
            Ty::Bool => "bool",
            Ty::V2F => "vec2<f32>",
            Ty::V3F => "vec3<f32>",
            Ty::V4F => "vec4<f32>",
            Ty::V2I => "vec2<i32>",
            Ty::V4I => "vec4<i32>",
            Ty::V4U => "vec4<u32>",
        }
    }
    fn words(self) -> usize {
        match self {
            Ty::V2F | Ty::V2I => 2,
            Ty::V3F => 3,
            Ty::V4F | Ty::V4I | Ty::V4U => 4,
            _ => 1,
        }
    }
    /// the scalar: 'i', 'u', 'f', 'b'
    fn kind(self) -> char {
        match self {
            Ty::I32 | Ty::V2I | Ty::V4I => 'i',
            Ty::U32 | Ty::V4U => 'u',
            Ty::Bool => 'b',
            _ => 'f',
        }
    }
    fn scalar_name(self) -> &'static str {
        match self.kind() {
            'i' => "i32",
            'u' => "u32",
            'b' => "bool",
            _ => "f32",
        }
    }
}

/// how a float result compares
#[derive(Clone, Copy, Debug)]
enum Tol {
    Exact,
    /// units in the last place
    Ulp(u32),
    /// absolute error
    Abs(f64),
    /// relative error (and the absolute floor)
    Rel(f64, f64),
    /// e1 % e2 = e1 - e2 * trunc(e1 / e2): trunc may be one off near a multiple, so the error
    /// is at most |e2| (and a little)
    Rem,
    /// f32 -> f16 conversions round to either neighbour (WGSL): each 16-bit half within 1
    Halves,
    /// quantizeToF16 and f32 -> f16: within one f16 step (WGSL: either neighbour), f16
    /// denormals may become 0
    Quantize,
    /// the pack*norm functions: floor(0.5 + k * e) in f32 (the operations' accuracy) may be one
    /// off the exact value at a tie: each field of this many bits within 1
    Fields(u32),
}

/// a runtime rule where the const evaluator rejects the inputs (words in -> words out)
type RtRef = fn(&[u32]) -> Option<Vec<u32>>;

struct Case {
    name: &'static str,
    args: &'static [Ty],
    ret: Ty,
    expr: &'static str,
    inputs: Vec<Vec<u32>>,
    tol: Tol,
    rt: Option<RtRef>,
    /// the shader and the reference need `enable f16;` (run only with Features::shader_f16)
    f16: bool,
}

const I32S: [i32; 19] = [0, 1, -1, 2, -2, 3, 7, -7, 31, 32, 33, 100, -100, i32::MAX, i32::MIN, 123_456_789, -987_654_321, 0x5555_5555, 0x0f0f_0f0f];
const U32S: [u32; 17] = [0, 1, 2, 3, 7, 8, 31, 32, 33, 100, 255, 256, 0x8000_0000, 0xffff_ffff, 123_456_789, 0xdead_beef, 0x00ff_00ff];
const F32S: [f32; 28] = [0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.0, 0.1, 100.0, -100.0, 1e-3, 1e10, -1e10, 3e9, -3e9, 5e9,
                         65504.0, 70000.0, 3.141_592_7, -2.718_281_8, 1e-30, 1e30, 2147483520.0, 2147483648.0, 4294967040.0];
/// small floats for binary and ternary functions (no overflow)
const F32_SMALL: [f32; 12] = [0.0, -0.0, 1.0, -1.0, 0.5, -1.5, 2.5, 3.0, 0.1, 7.25, -100.0, 1e-3];
const ANGLES: [f32; 13] = [0.0, 0.1, -0.1, 0.5, -0.5, 1.0, -1.0, 1.5, -1.5, 2.5, -2.5, 3.1, -3.1];
const POSITIVE: [f32; 12] = [0.001, 0.1, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 10.0, 100.0, 12345.0, 1e10];
const UNIT: [f32; 11] = [-1.0, -0.9, -0.5, -0.25, -0.1, 0.0, 0.1, 0.25, 0.5, 0.9, 1.0];

fn w_i(x: i32) -> u32 {
    x as u32
}
fn w_f(x: f32) -> u32 {
    x.to_bits()
}

fn one<T: Copy>(xs: &[T], f: fn(T) -> u32) -> Vec<Vec<u32>> {
    xs.iter().map(|&x| vec![f(x)]).collect()
}

fn pairs<A: Copy, B: Copy>(xs: &[A], ys: &[B], f: fn(A) -> u32, g: fn(B) -> u32) -> Vec<Vec<u32>> {
    let mut v = Vec::new();
    for &x in xs {
        for &y in ys {
            v.push(vec![f(x), g(y)]);
        }
    }
    v
}

fn triples(xs: &[f32]) -> Vec<Vec<u32>> {
    let mut v = Vec::new();
    for &x in xs {
        for &y in xs {
            for &z in xs {
                v.push(vec![x.to_bits(), y.to_bits(), z.to_bits()]);
            }
        }
    }
    v
}

/// vectors of n floats from consecutive values of xs
fn vecs(xs: &[f32], n: usize, count: usize) -> Vec<Vec<u32>> {
    (0..count).map(|k| (0..n).map(|j| xs[(k * 3 + j * 5) % xs.len()].to_bits()).collect()).collect()
}

// the runtime rules where const-expressions are errors
fn rt_div_i(w: &[u32]) -> Option<Vec<u32>> {
    let (a, b) = (w[0] as i32, w[1] as i32);
    Some(vec![if b == 0 || (a == i32::MIN && b == -1) { a } else { a / b } as u32])
}
fn rt_rem_i(w: &[u32]) -> Option<Vec<u32>> {
    let (a, b) = (w[0] as i32, w[1] as i32);
    Some(vec![if b == 0 || (a == i32::MIN && b == -1) { 0 } else { a % b } as u32])
}
fn rt_div_u(w: &[u32]) -> Option<Vec<u32>> {
    Some(vec![if w[1] == 0 { w[0] } else { w[0] / w[1] }])
}
fn rt_rem_u(w: &[u32]) -> Option<Vec<u32>> {
    Some(vec![if w[1] == 0 { 0 } else { w[0] % w[1] }])
}
fn rt_shl_i(w: &[u32]) -> Option<Vec<u32>> {
    Some(vec![w[0].wrapping_shl(w[1] & 31)])
}
fn rt_shr_i(w: &[u32]) -> Option<Vec<u32>> {
    Some(vec![((w[0] as i32) >> (w[1] & 31)) as u32])
}
fn rt_shr_u(w: &[u32]) -> Option<Vec<u32>> {
    Some(vec![w[0] >> (w[1] & 31)])
}
fn rt_add_i(w: &[u32]) -> Option<Vec<u32>> {
    Some(vec![w[0].wrapping_add(w[1])])
}

fn cases() -> Vec<Case> {
    use Ty::*;
    let c = |name, args: &'static [Ty], ret, expr, inputs, tol| Case { name, args, ret, expr, inputs, tol, rt: None, f16: false };
    let h = |name, args: &'static [Ty], ret, expr, inputs, tol| Case { name, args, ret, expr, inputs, tol, rt: None, f16: true };
    let ii = || pairs(&I32S, &I32S, w_i, w_i);
    let uu = || pairs(&U32S, &U32S, |x| x, |x| x);
    let iu = || pairs(&I32S, &U32S, w_i, |x| x);
    let ff = || pairs(&F32_SMALL, &F32_SMALL, w_f, w_f);
    let fs = || one(&F32S, w_f);
    let is = || one(&I32S, w_i);
    let us = || one(&U32S, |x| x);
    let mut v = vec![
        // integer operators (i32 wraps; division rules)
        Case { rt: Some(rt_add_i), ..c("i32 +", &[I32, I32], I32, "a + b", ii(), Tol::Exact) },
        c("i32 -", &[I32, I32], I32, "a - b", ii(), Tol::Exact),
        c("i32 *", &[I32, I32], I32, "a * b", ii(), Tol::Exact),
        Case { rt: Some(rt_div_i), ..c("i32 /", &[I32, I32], I32, "a / b", ii(), Tol::Exact) },
        Case { rt: Some(rt_rem_i), ..c("i32 %", &[I32, I32], I32, "a % b", ii(), Tol::Exact) },
        c("i32 &", &[I32, I32], I32, "a & b", ii(), Tol::Exact),
        c("i32 |", &[I32, I32], I32, "a | b", ii(), Tol::Exact),
        c("i32 ^", &[I32, I32], I32, "a ^ b", ii(), Tol::Exact),
        Case { rt: Some(rt_shl_i), ..c("i32 <<", &[I32, U32], I32, "a << b", iu(), Tol::Exact) },
        Case { rt: Some(rt_shr_i), ..c("i32 >>", &[I32, U32], I32, "a >> b", iu(), Tol::Exact) },
        c("i32 ~", &[I32], I32, "~a", is(), Tol::Exact),
        c("i32 neg", &[I32], I32, "-a", is(), Tol::Exact),
        c("i32 <", &[I32, I32], Bool, "a < b", ii(), Tol::Exact),
        c("i32 >=", &[I32, I32], Bool, "a >= b", ii(), Tol::Exact),
        c("i32 ==", &[I32, I32], Bool, "a == b", ii(), Tol::Exact),
        c("u32 +", &[U32, U32], U32, "a + b", uu(), Tol::Exact),
        c("u32 -", &[U32, U32], U32, "a - b", uu(), Tol::Exact),
        c("u32 *", &[U32, U32], U32, "a * b", uu(), Tol::Exact),
        Case { rt: Some(rt_div_u), ..c("u32 /", &[U32, U32], U32, "a / b", uu(), Tol::Exact) },
        Case { rt: Some(rt_rem_u), ..c("u32 %", &[U32, U32], U32, "a % b", uu(), Tol::Exact) },
        Case { rt: Some(rt_shl_i), ..c("u32 <<", &[U32, U32], U32, "a << b", uu(), Tol::Exact) },
        Case { rt: Some(rt_shr_u), ..c("u32 >>", &[U32, U32], U32, "a >> b", uu(), Tol::Exact) },
        c("u32 <", &[U32, U32], Bool, "a < b", uu(), Tol::Exact),
        c("u32 !=", &[U32, U32], Bool, "a != b", uu(), Tol::Exact),
        // float operators
        c("f32 +", &[F32, F32], F32, "a + b", ff(), Tol::Exact),
        c("f32 -", &[F32, F32], F32, "a - b", ff(), Tol::Exact),
        c("f32 *", &[F32, F32], F32, "a * b", ff(), Tol::Exact),
        c("f32 /", &[F32, F32], F32, "a / b", ff(), Tol::Ulp(3)),
        c("f32 %", &[F32, F32], F32, "a % b", ff(), Tol::Rem),
        c("f32 neg", &[F32], F32, "-a", fs(), Tol::Exact),
        c("f32 <", &[F32, F32], Bool, "a < b", ff(), Tol::Exact),
        c("f32 <=", &[F32, F32], Bool, "a <= b", ff(), Tol::Exact),
        c("f32 ==", &[F32, F32], Bool, "a == b", ff(), Tol::Exact),
        c("f32 !=", &[F32, F32], Bool, "a != b", ff(), Tol::Exact),
        // conversions (float -> integer saturates)
        c("i32(f32)", &[F32], I32, "i32(a)", fs(), Tol::Exact),
        c("u32(f32)", &[F32], U32, "u32(a)", fs(), Tol::Exact),
        c("f32(i32)", &[I32], F32, "f32(a)", is(), Tol::Exact),
        c("f32(u32)", &[U32], F32, "f32(a)", us(), Tol::Exact),
        c("u32(i32)", &[I32], U32, "u32(a)", is(), Tol::Exact),
        c("bool(f32)", &[F32], Bool, "bool(a)", fs(), Tol::Exact),
        c("bool(i32)", &[I32], Bool, "bool(a)", is(), Tol::Exact),
        c("f32(bool)", &[Bool], F32, "f32(a)", vec![vec![0], vec![1]], Tol::Exact),
        c("bitcast<f32>(u32)", &[U32], F32, "bitcast<f32>(a)", one(&[0u32, 1 << 23, 0x3f80_0000, 0xbf80_0000, 0x4049_0fdb, 0x7f7f_ffff],
                                                                   |x| x), Tol::Exact),
        c("bitcast<i32>(f32)", &[F32], I32, "bitcast<i32>(a)", fs(), Tol::Exact),
        // integer built-ins
        c("abs i32", &[I32], I32, "abs(a)", is(), Tol::Exact),
        c("sign i32", &[I32], I32, "sign(a)", is(), Tol::Exact),
        c("min i32", &[I32, I32], I32, "min(a, b)", ii(), Tol::Exact),
        c("max u32", &[U32, U32], U32, "max(a, b)", uu(), Tol::Exact),
        c("clamp i32", &[I32, I32], I32, "clamp(a, b, 7i)", pairs(&I32S, &[-5, 0, 7], w_i, w_i), Tol::Exact),
        c("clamp u32", &[U32, U32], U32, "clamp(a, b, 100u)", pairs(&U32S, &[0, 7, 100], |x| x, |x| x), Tol::Exact),
        c("countLeadingZeros u32", &[U32], U32, "countLeadingZeros(a)", us(), Tol::Exact),
        c("countLeadingZeros i32", &[I32], I32, "countLeadingZeros(a)", is(), Tol::Exact),
        c("countOneBits", &[U32], U32, "countOneBits(a)", us(), Tol::Exact),
        c("countTrailingZeros u32", &[U32], U32, "countTrailingZeros(a)", us(), Tol::Exact),
        c("countTrailingZeros i32", &[I32], I32, "countTrailingZeros(a)", is(), Tol::Exact),
        c("firstLeadingBit i32", &[I32], I32, "firstLeadingBit(a)", is(), Tol::Exact),
        c("firstLeadingBit u32", &[U32], U32, "firstLeadingBit(a)", us(), Tol::Exact),
        c("firstTrailingBit", &[U32], U32, "firstTrailingBit(a)", us(), Tol::Exact),
        c("reverseBits", &[U32], U32, "reverseBits(a)", us(), Tol::Exact),
        c("extractBits i32", &[I32, U32], I32, "extractBits(a, b, 5u)", iu(), Tol::Exact),
        c("extractBits u32", &[U32, U32], U32, "extractBits(a, 3u, b)", uu(), Tol::Exact),
        c("extractBits i32 (offset + count > 32)", &[I32, U32], I32, "extractBits(a, b, 40u)", iu(), Tol::Exact),
        c("insertBits", &[U32, U32], U32, "insertBits(a, 0x5au, b, 7u)", uu(), Tol::Exact),
        c("insertBits (count over)", &[U32, U32], U32, "insertBits(a, 0xffffffffu, 9u, b)", uu(), Tol::Exact),
        c("dot4U8Packed", &[U32, U32], U32, "dot4U8Packed(a, b)", uu(), Tol::Exact),
        c("dot4I8Packed", &[U32, U32], I32, "dot4I8Packed(a, b)", uu(), Tol::Exact),
        c("pack4xI8", &[V4I], U32, "pack4xI8(a)", vec![vec![1, 0xffff_ffff, 300, (-300i32) as u32], vec![127, 128, 0, 0x8000_0000]],
          Tol::Exact),
        c("pack4xU8", &[V4U], U32, "pack4xU8(a)", vec![vec![1, 255, 256, 0xffff_ffff], vec![0, 7, 300, 128]], Tol::Exact),
        c("pack4xI8Clamp", &[V4I], U32, "pack4xI8Clamp(a)", vec![vec![1, 0xffff_ffff, 300, (-300i32) as u32], vec![127, 128, 0, 0x8000_0000]],
          Tol::Exact),
        c("pack4xU8Clamp", &[V4U], U32, "pack4xU8Clamp(a)", vec![vec![1, 255, 256, 0xffff_ffff], vec![0, 7, 300, 128]], Tol::Exact),
        c("unpack4xI8", &[U32], V4I, "unpack4xI8(a)", us(), Tol::Exact),
        c("unpack4xU8", &[U32], V4U, "unpack4xU8(a)", us(), Tol::Exact),
        c("dot vec2<i32>", &[V2I, V2I], I32, "dot(a, b)", pairs(&I32S[..8], &I32S[..8], w_i, w_i).into_iter()
          .map(|p| vec![p[0], p[1], p[1], p[0]]).collect(), Tol::Exact),
        c("select i32", &[I32, I32, Bool], I32, "select(a, b, c)", vec![vec![1, 2, 0], vec![1, 2, 1], vec![w_i(-5), 9, 1]], Tol::Exact),
        c("select vec3<f32> by bool", &[V3F, V3F, Bool], V3F, "select(a, b, c)",
          vec![vec![w_f(1.0), w_f(2.0), w_f(3.0), w_f(4.0), w_f(5.0), w_f(6.0), 1], vec![w_f(1.0), w_f(2.0), w_f(3.0), w_f(4.0), w_f(5.0), w_f(6.0), 0]],
          Tol::Exact),
        // float built-ins
        c("abs f32", &[F32], F32, "abs(a)", fs(), Tol::Exact),
        c("ceil", &[F32], F32, "ceil(a)", fs(), Tol::Exact),
        c("floor", &[F32], F32, "floor(a)", fs(), Tol::Exact),
        c("trunc", &[F32], F32, "trunc(a)", fs(), Tol::Exact),
        c("round (half to even)", &[F32], F32, "round(a)", one(&[0.5f32, 1.5, 2.5, -0.5, -2.5, 3.49, 1e10, -7.5], w_f), Tol::Exact),
        c("fract", &[F32], F32, "fract(a)", one(&F32_SMALL, w_f), Tol::Ulp(1)),
        c("sign f32", &[F32], F32, "sign(a)", fs(), Tol::Exact),
        c("min f32", &[F32, F32], F32, "min(a, b)", ff(), Tol::Exact),
        c("max f32", &[F32, F32], F32, "max(a, b)", ff(), Tol::Exact),
        c("clamp f32", &[F32, F32, F32], F32, "clamp(a, b, c)", triples(&F32_SMALL[..7]), Tol::Exact),
        c("saturate", &[F32], F32, "saturate(a)", fs(), Tol::Exact),
        c("step", &[F32, F32], F32, "step(a, b)", ff(), Tol::Exact),
        c("smoothstep", &[F32], F32, "smoothstep(-1.0, 3.0, a)", one(&F32_SMALL, w_f), Tol::Abs(2e-6)),
        c("mix", &[F32, F32, F32], F32, "mix(a, b, c)", triples(&[0.0, 1.0, -2.5, 0.25, 3.0]), Tol::Rel(1e-6, 1e-6)),
        c("fma", &[F32, F32, F32], F32, "fma(a, b, c)", triples(&[0.0, 1.0, -2.5, 0.25, 3.0, 0.1]), Tol::Rel(1.2e-7, 1e-7)),
        c("sqrt", &[F32], F32, "sqrt(a)", one(&POSITIVE, w_f), Tol::Ulp(3)),
        c("inverseSqrt", &[F32], F32, "inverseSqrt(a)", one(&POSITIVE, w_f), Tol::Ulp(2)),
        c("exp", &[F32], F32, "exp(a)", one(&[0.0f32, 1.0, -1.0, 0.5, 2.5, -7.0, 10.0], w_f), Tol::Ulp(30)),
        c("exp2", &[F32], F32, "exp2(a)", one(&[0.0f32, 1.0, -1.0, 0.5, 2.5, -7.0, 10.0], w_f), Tol::Ulp(30)),
        c("log", &[F32], F32, "log(a)", one(&POSITIVE, w_f), Tol::Rel(1e-6, 4.76837158203125e-7)),
        c("log2", &[F32], F32, "log2(a)", one(&POSITIVE, w_f), Tol::Rel(1e-6, 4.76837158203125e-7)),
        c("pow", &[F32, F32], F32, "pow(a, b)", pairs(&POSITIVE[..8], &[0.0f32, 1.0, -1.0, 2.5, 0.5], w_f, w_f), Tol::Rel(1e-4, 1e-6)),
        c("sin", &[F32], F32, "sin(a)", one(&ANGLES, w_f), Tol::Abs(4.8828125e-4)),
        c("cos", &[F32], F32, "cos(a)", one(&ANGLES, w_f), Tol::Abs(4.8828125e-4)),
        c("tan", &[F32], F32, "tan(a)", one(&ANGLES[..9], w_f), Tol::Rel(1e-3, 1e-3)),
        c("asin", &[F32], F32, "asin(a)", one(&UNIT, w_f), Tol::Abs(1e-3)),
        c("acos", &[F32], F32, "acos(a)", one(&UNIT, w_f), Tol::Abs(1e-3)),
        c("atan", &[F32], F32, "atan(a)", one(&F32_SMALL, w_f), Tol::Abs(1e-4)),
        // atan2(-0, x < 0) is -pi or pi: WGSL need not keep the sign of zero
        c("atan2", &[F32, F32], F32, "atan2(a, b)", pairs(&F32_SMALL[2..], &F32_SMALL, w_f, w_f), Tol::Abs(1e-4)),
        c("sinh", &[F32], F32, "sinh(a)", one(&UNIT, w_f), Tol::Rel(1e-4, 1e-5)),
        c("cosh", &[F32], F32, "cosh(a)", one(&UNIT, w_f), Tol::Rel(1e-4, 1e-5)),
        c("tanh", &[F32], F32, "tanh(a)", one(&UNIT, w_f), Tol::Rel(1e-4, 1e-5)),
        c("asinh", &[F32], F32, "asinh(a)", one(&F32_SMALL, w_f), Tol::Rel(1e-4, 1e-5)),
        c("acosh", &[F32], F32, "acosh(a)", one(&POSITIVE, w_f), Tol::Rel(1e-4, 1e-4)),
        c("atanh", &[F32], F32, "atanh(a)", one(&UNIT[1..10], w_f), Tol::Rel(1e-4, 1e-5)),
        c("degrees", &[F32], F32, "degrees(a)", one(&ANGLES, w_f), Tol::Ulp(2)),
        c("radians", &[F32], F32, "radians(a)", one(&F32_SMALL, w_f), Tol::Ulp(2)),
        c("ldexp", &[F32, I32], F32, "ldexp(a, b)", pairs(&F32_SMALL, &[0, 1, -1, 7, -20, 30], w_f, w_i), Tol::Exact),
        c("frexp fract", &[F32], F32, "frexp(a).fract", fs(), Tol::Exact),
        c("frexp exp", &[F32], I32, "frexp(a).exp", fs(), Tol::Exact),
        c("modf fract", &[F32], F32, "modf(a).fract", fs(), Tol::Exact),
        c("modf whole", &[F32], F32, "modf(a).whole", fs(), Tol::Exact),
        c("quantizeToF16", &[F32], F32, "quantizeToF16(a)", one(&[0.0f32, 1.0, 0.1, -2.5, 3.141_592_7, 65504.0, 1e-3, 1.0e-6], w_f),
          Tol::Quantize),
        c("length vec3", &[V3F], F32, "length(a)", vecs(&F32_SMALL, 3, 12), Tol::Rel(1e-6, 1e-7)),
        c("distance vec2", &[V2F, V2F], F32, "distance(a, b)", vecs(&F32_SMALL, 4, 12).into_iter().map(|v| v).collect(), Tol::Rel(1e-6, 1e-7)),
        c("normalize vec3", &[V3F], V3F, "normalize(a)", vecs(&F32_SMALL[2..], 3, 10), Tol::Rel(1e-6, 1e-7)),
        c("dot vec3", &[V3F, V3F], F32, "dot(a, b)", vecs(&F32_SMALL, 6, 12), Tol::Rel(1e-6, 1e-6)),
        c("cross", &[V3F, V3F], V3F, "cross(a, b)", vecs(&F32_SMALL, 6, 12), Tol::Rel(1e-6, 1e-6)),
        c("reflect", &[V3F, V3F], V3F, "reflect(a, b)", vecs(&F32_SMALL, 6, 12), Tol::Rel(1e-5, 1e-5)),
        c("faceForward", &[V3F, V3F], V3F, "faceForward(a, b, vec3f(1.0, -1.0, 0.5))", vecs(&F32_SMALL, 6, 12), Tol::Exact),
        c("refract", &[V2F], V2F, "refract(normalize(a), vec2f(0.0, 1.0), 0.75)", vecs(&F32_SMALL[2..], 2, 8), Tol::Rel(1e-4, 1e-5)),
        c("determinant 2x2", &[V4F], F32, "determinant(mat2x2f(a.xy, a.zw))", vecs(&F32_SMALL, 4, 12), Tol::Rel(1e-6, 1e-6)),
        c("pack4x8snorm", &[V4F], U32, "pack4x8snorm(a)", vecs(&UNIT, 4, 12), Tol::Fields(8)),
        c("pack4x8unorm", &[V4F], U32, "pack4x8unorm(a)", vecs(&UNIT, 4, 12), Tol::Fields(8)),
        c("pack2x16snorm", &[V2F], U32, "pack2x16snorm(a)", vecs(&UNIT, 2, 12), Tol::Fields(16)),
        c("pack2x16unorm", &[V2F], U32, "pack2x16unorm(a)", vecs(&UNIT, 2, 12), Tol::Fields(16)),
        c("pack2x16float", &[V2F], U32, "pack2x16float(a)", vecs(&F32_SMALL, 2, 12), Tol::Halves),
        c("unpack4x8snorm", &[U32], V4F, "unpack4x8snorm(a)", us(), Tol::Ulp(1)),
        c("unpack4x8unorm", &[U32], V4F, "unpack4x8unorm(a)", us(), Tol::Ulp(1)),
        c("unpack2x16snorm", &[U32], V2F, "unpack2x16snorm(a)", us(), Tol::Ulp(1)),
        c("unpack2x16unorm", &[U32], V2F, "unpack2x16unorm(a)", us(), Tol::Ulp(1)),
        c("unpack2x16float", &[U32], V2F, "unpack2x16float(a)", one(&[0u32, 0x3c00_3c00, 0xc000_3555, 0x7bff_0001, 0x0400_8400], |x| x),
          Tol::Exact),
        // vectors and matrices in expressions
        c("vec * scalar", &[V3F, F32], V3F, "a * b", vecs(&F32_SMALL, 4, 12).into_iter().map(|v| v).collect(), Tol::Exact),
        c("mat2x2 * vec2", &[V4F, V2F], V2F, "mat2x2f(a.xy, a.zw) * b", vecs(&F32_SMALL, 6, 12), Tol::Rel(1e-6, 1e-6)),
        c("dynamic vector index", &[V4F, U32], F32, "a[b]", (0..12).map(|k| vec![w_f(1.0), w_f(2.0), w_f(3.0), w_f(4.0), k as u32]).collect(),
          Tol::Exact),
        c("all / any", &[V2I], Bool, "all(a > vec2i(0)) || !any(a < vec2i(-5))", pairs(&I32S[..8], &I32S[..8], w_i, w_i), Tol::Exact),
    ];
    // the rule for shifts of the bit width or more is the runtime one only
    for k in v.iter_mut() {
        let _ = k;
    }
    // f16: the f32 inputs converted, the result converted back (f16 precision: 2^-11 relative)
    let hs = || pairs(&F32_SMALL, &F32_SMALL, w_f, w_f);
    v.extend([
        h("f16 +", &[F32, F32], F32, "f32(f16(a) + f16(b))", hs(), Tol::Rel(2e-3, 2e-6)),
        h("f16 *", &[F32, F32], F32, "f32(f16(a) * f16(b))", hs(), Tol::Rel(2e-3, 2e-6)),
        h("f16 /", &[F32, F32], F32, "f32(f16(a) / f16(b))", hs(), Tol::Rel(2e-3, 1e-4)),
        h("f16 conversion", &[F32], F32, "f32(f16(a))", one(&F32S[..20], w_f), Tol::Rel(4.9e-4, 6.2e-5)),
        h("f16 sqrt", &[F32], F32, "f32(sqrt(f16(a)))", one(&POSITIVE[..9], w_f), Tol::Rel(2e-3, 1e-4)),
        h("f16 dot vec3h", &[V3F, V3F], F32, "f32(dot(vec3h(a), vec3h(b)))", vecs(&F32_SMALL, 6, 12), Tol::Rel(4e-3, 4e-3)),
        h("f16 min / max", &[F32, F32], F32, "f32(min(f16(a), f16(b)) + max(f16(a), f16(b)))", hs(), Tol::Rel(1e-3, 1e-4)),
        h("i32(f16)", &[F32], I32, "i32(f16(a))", one(&F32S[..20], w_f), Tol::Exact),
        h("f16 bitcast", &[U32], U32, "bitcast<u32>(bitcast<vec2h>(a) * vec2h(2.0h))", one(&[0u32, 0x3c00_3c00, 0x4000_3555, 0x0001_0400], |x| x),
          Tol::Halves),
    ]);
    v.retain(|k| !k.inputs.is_empty());
    v
}

/// the WGSL literal of an input
fn literal(t: Ty, w: &[u32]) -> String {
    let s = |k: char, x: u32| -> String {
        match k {
            'i' => {
                let v = x as i32;
                if v == i32::MIN { String::from("(-2147483647i - 1i)") } else { format!("{v}i") }
            }
            'u' => format!("{x}u"),
            'b' => String::from(if x != 0 { "true" } else { "false" }),
            _ => format!("{:?}f", f32::from_bits(x)),
        }
    };
    if t.words() == 1 {
        return s(t.kind(), w[0]);
    }
    let parts: Vec<String> = w.iter().map(|&x| s(t.kind(), x)).collect();
    format!("{}({})", t.wgsl(), parts.join(", "))
}

/// the words of a folded constant
fn value_words(v: &Value, out: &mut Vec<u32>) -> bool {
    match v {
        Value::I32(x) => out.push(*x as u32),
        Value::U32(x) => out.push(*x),
        Value::F32(x) => out.push(x.to_bits()),
        Value::Bool(b) => out.push(*b as u32),
        Value::Comp(cs) => {
            for c in cs {
                if !value_words(c, out) {
                    return false;
                }
            }
        }
        _ => return false,
    }
    true
}

/// lib/wgsl's constant evaluator on one input -> the result words (None: an error)
fn const_ref(case: &Case, input: &[u32]) -> Option<Vec<u32>> {
    let mut src = String::from(if case.f16 { "enable f16;\n" } else { "" });
    let mut at = 0;
    for (k, t) in case.args.iter().enumerate() {
        let n = t.words();
        src.push_str(&format!("const {}: {} = {};\n", ["a", "b", "c", "d"][k], t.wgsl(), literal(*t, &input[at..at + n])));
        at += n;
    }
    src.push_str(&format!("const r: {} = {};\n", case.ret.wgsl(), case.expr));
    let m = wgsl::check(&src).ok()?;
    let v = &m.consts.iter().find(|c| c.name == "r")?.value;
    let mut out = Vec::new();
    if !value_words(v, &mut out) {
        return None;
    }
    Some(out)
}

/// the compute shader of a case
fn shader(case: &Case, n: usize) -> String {
    let inw: usize = case.args.iter().map(|t| t.words()).sum();
    let outw = case.ret.words();
    let mut s = String::from(if case.f16 { "enable f16;\n" } else { "" });
    s.push_str("@group(0) @binding(0) var<storage, read> inp: array<u32>;\n");
    s.push_str("@group(0) @binding(1) var<storage, read_write> outp: array<u32>;\n");
    s.push_str("@compute @workgroup_size(64)\nfn main(@builtin(global_invocation_id) gid: vec3u) {\n");
    s.push_str(&format!("    let i = gid.x;\n    if i >= {n}u {{ return; }}\n    let base = i * {inw}u;\n    let ob = i * {outw}u;\n"));
    let load = |k: char, idx: String| -> String {
        match k {
            'i' => format!("bitcast<i32>(inp[{idx}])"),
            'u' => format!("inp[{idx}]"),
            'b' => format!("(inp[{idx}] != 0u)"),
            _ => format!("bitcast<f32>(inp[{idx}])"),
        }
    };
    let mut at = 0;
    for (k, t) in case.args.iter().enumerate() {
        let name = ["a", "b", "c", "d"][k];
        let e = if t.words() == 1 {
            load(t.kind(), format!("base + {at}u"))
        } else {
            let parts: Vec<String> = (0..t.words()).map(|j| load(t.kind(), format!("base + {}u", at + j))).collect();
            format!("{}({})", t.wgsl(), parts.join(", "))
        };
        s.push_str(&format!("    let {name}: {} = {e};\n", t.wgsl()));
        at += t.words();
    }
    s.push_str(&format!("    let r: {} = {};\n", case.ret.wgsl(), case.expr));
    let store = |k: char, v: String| -> String {
        match k {
            'i' | 'f' => format!("bitcast<u32>({v})"),
            'b' => format!("select(0u, 1u, {v})"),
            _ => v,
        }
    };
    if outw == 1 {
        s.push_str(&format!("    outp[ob] = {};\n", store(case.ret.kind(), String::from("r"))));
    } else {
        for j in 0..outw {
            s.push_str(&format!("    outp[ob + {j}u] = {};\n", store(case.ret.kind(), format!("r[{j}]"))));
        }
    }
    s.push_str("}\n");
    let _ = case.ret.scalar_name();
    s
}

fn ulps(a: f32, b: f32) -> u64 {
    let k = |x: f32| -> i64 {
        let b = x.to_bits() as i32 as i64;
        if b < 0 { (i32::MIN as i64) - b } else { b }
    };
    (k(a) - k(b)).unsigned_abs()
}

/// one result word against the reference (input: the case's input words)
fn word_ok(kind: char, tol: Tol, got: u32, want: u32, input: &[u32]) -> bool {
    if got == want {
        return true;
    }
    if let Tol::Halves = tol {
        let d = |s: u32| ((got >> s) as u16 as i32 - (want >> s) as u16 as i32).abs();
        return d(0) <= 1 && d(16) <= 1;
    }
    if let Tol::Fields(bits) = tol {
        let mask = ((1u64 << bits) - 1) as u32;
        return (0..32 / bits).all(|k| {
            let (g, w) = ((got >> (k * bits)) & mask, (want >> (k * bits)) & mask);
            g.abs_diff(w) <= 1 || g.abs_diff(w) == mask
        });
    }
    if kind != 'f' {
        return false;
    }
    if let Tol::Quantize = tol {
        // f16 denormals may become 0; the conversion to f16 may round to either neighbour
        let (g, w) = (f32::from_bits(got), f32::from_bits(want));
        let e = ((w.abs().max(6.103_515_6e-5).to_bits() >> 23) & 0xff) as i32 - 127;
        let ulp16 = f32::from_bits(((e - 10 + 127) as u32) << 23) as f64;
        return (w.abs() < 6.103_515_6e-5 && g == 0.0) || ((g - w).abs() as f64) <= ulp16 * 1.0001;
    }
    let (g, w) = (f32::from_bits(got), f32::from_bits(want));
    if g == w {
        return true; // +0 = -0
    }
    if w != 0.0 && w.abs() < f32::MIN_POSITIVE && g == 0.0 {
        return true; // a denormal flushed
    }
    if !g.is_finite() || !w.is_finite() {
        return false;
    }
    let d = (g as f64 - w as f64).abs();
    match tol {
        Tol::Exact => false,
        Tol::Ulp(n) => ulps(g, w) <= n as u64,
        Tol::Abs(e) => d <= e,
        Tol::Rel(r, a) => d <= a.max(r * (w as f64).abs()),
        Tol::Rem => d <= (f32::from_bits(input[1]) as f64).abs() * (1.0 + 1e-6) + 1e-30,
        Tol::Halves | Tol::Quantize | Tol::Fields(_) => false,
    }
}

/// run every case on one device at one level -> (cases, inputs compared, inputs without
/// reference, failures)
fn run_level(t: &mut T, dev: &Device, minor: u32, what: &str) {
    let b = |binding, kind| Binding { binding, kind, stages: Stages::COMPUTE };
    let Some(bgl) = t.res(dev.create_bind_group_layout(&[b(0, BindingKind::StorageBufferRead), b(1, BindingKind::StorageBuffer)]),
                          "layout") else { return };
    let Some(pl) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "pipeline layout")
    else { return };
    let (mut compared, mut skipped, mut bad_total) = (0usize, 0usize, 0usize);
    let list = cases();
    let f16 = dev.features().shader_f16;
    for case in &list {
        if case.f16 && !f16 {
            out(&format!("  SKIP {what}: {}: no shader_f16", case.name));
            continue;
        }
        let n = case.inputs.len();
        let src = shader(case, n);
        let opt = wgsl::CompileOptions::new(minor);
        let words = match wgsl::compile(&src, &opt) {
            Ok(o) => o.entries.into_iter().next().map(|e| e.words).unwrap_or_default(),
            Err(d) => {
                t.fail(&format!("{what}: {}: compile: {}", case.name, d.first().map(|x| format!("{x}")).unwrap_or_default()));
                continue;
            }
        };
        let Some(m) = t.res(ShaderModule::from_spirv(dev, &words), &format!("{}: module", case.name)) else { continue };
        let Some(pipe) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }),
                               &format!("{}: pipeline", case.name)) else { continue };
        let inp: Vec<u32> = case.inputs.iter().flatten().copied().collect();
        let outw = case.ret.words();
        let ib = t.res(dev.create_buffer(&BufferDesc { size: (inp.len().max(1) * 4) as u64, usage: BufferUsage::STORAGE,
                                                       memory: MemoryKind::Device }), "input buffer");
        let ob = t.res(dev.create_buffer(&BufferDesc { size: (n * outw * 4) as u64, usage: BufferUsage::STORAGE,
                                                       memory: MemoryKind::Device }), "output buffer");
        let (Some(ib), Some(ob)) = (ib, ob) else { continue };
        if t.res(dev.queue().write_buffer(&ib, 0, &inp), "write inputs").is_none() {
            continue;
        }
        let entry = |binding, buffer| BindEntry { binding, resource: Resource::Buffer { buffer, offset: 0, size: 0 } };
        let Some(bg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[entry(0, &ib), entry(1, &ob)] }),
                             "bind group") else { continue };
        let Some(mut e) = t.res(dev.encoder(), "encoder") else { continue };
        e.set_compute_pipeline(&pipe);
        e.set_bind_group(0, &bg);
        e.dispatch((n as u32).div_ceil(64), 1, 1);
        e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
        if crate::run_wait(t, dev, e.finish(), case.name).is_none() {
            continue;
        }
        let mut got = vec![0u32; n * outw];
        if t.res(dev.queue().read_buffer(&ob, 0, &mut got), "read results").is_none() {
            continue;
        }
        let mut bad = Vec::new();
        for (k, input) in case.inputs.iter().enumerate() {
            let want = match const_ref(case, input).or_else(|| case.rt.and_then(|f| f(input))) {
                Some(w) => w,
                None => {
                    skipped += 1;
                    continue;
                }
            };
            compared += 1;
            let g = &got[k * outw..k * outw + outw];
            if !g.iter().zip(&want).all(|(&x, &y)| word_ok(case.ret.kind(), case.tol, x, y, input)) {
                bad.push((k, g.to_vec(), want));
            }
        }
        bad_total += bad.len();
        let detail = bad.first().map_or(String::new(), |(k, g, w)| {
            let args: Vec<String> = {
                let mut at = 0;
                case.args.iter().map(|t| {
                    let n = t.words();
                    let l = literal(*t, &case.inputs[*k][at..at + n]);
                    at += n;
                    l
                }).collect()
            };
            format!(": ({}) -> {:x?}, expected {:x?}", args.join(", "), g, w)
        });
        t.ok(bad.is_empty(), &format!("{what}: {} on {} inputs{}{}", case.name, n,
                                      if bad.is_empty() { String::new() } else { format!(", {} differ", bad.len()) }, detail));
    }
    out(&format!("  {what}: {} cases, {compared} results compared with lib/wgsl's constant evaluator (or the runtime rule), \
                  {skipped} inputs without a defined result, {bad_total} differ", list.len()));
    nan_checks(t, dev, &pl, &bgl, minor, what);
    crate::t_wgsl2::overrides(t, dev, minor, what);
    crate::t_wgsl2::robust(t, dev, minor, what);
    crate::t_wgsl2::workgroup(t, dev, minor, what);
    crate::t_wgsl2::discard(t, dev, minor, what);
    crate::t_wgsl2::textures(t, dev, minor, what);
}

/// NaN operands: WGSL's min / max return the other operand, comparisons are unordered;
/// reported (Vulkan need not preserve NaN without SignedZeroInfNanPreserve)
fn nan_checks(t: &mut T, dev: &Device, pl: &PipelineLayout, bgl: &BindGroupLayout, minor: u32, what: &str) {
    let src = [
        "@group(0) @binding(0) var<storage, read> inp: array<u32>;",
        "@group(0) @binding(1) var<storage, read_write> outp: array<u32>;",
        "@compute @workgroup_size(1) fn main() {",
        "    let nan = bitcast<f32>(inp[0]);",
        "    let one = bitcast<f32>(inp[1]);",
        "    outp[0] = bitcast<u32>(min(nan, one));",
        "    outp[1] = bitcast<u32>(max(one, nan));",
        "    outp[2] = bitcast<u32>(clamp(nan, 0.0, one));",
        "    outp[3] = select(0u, 1u, nan != nan);",
        "    outp[4] = select(0u, 1u, nan == nan);",
        "    outp[5] = select(0u, 1u, nan < one);",
        "    outp[6] = select(0u, 1u, nan >= one);",
        "}",
    ].join("\n");
    let Ok(o) = wgsl::compile(&src, &wgsl::CompileOptions::new(minor)) else {
        t.fail("NaN shader compiles");
        return;
    };
    let words = o.entries.into_iter().next().map(|e| e.words).unwrap_or_default();
    let Some(m) = t.res(ShaderModule::from_spirv(dev, &words), "NaN module") else { return };
    let Some(pipe) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: pl, shader: ShaderStage::new(&m, "main") }),
                           "NaN pipeline") else { return };
    let ib = dev.create_buffer(&BufferDesc { size: 8, usage: BufferUsage::STORAGE, memory: MemoryKind::Device });
    let ob = dev.create_buffer(&BufferDesc { size: 28, usage: BufferUsage::STORAGE, memory: MemoryKind::Device });
    let (Some(ib), Some(ob)) = (t.res(ib, "buffer"), t.res(ob, "buffer")) else { return };
    let _ = dev.queue().write_buffer(&ib, 0, &[0x7fc0_0000u32, 1.0f32.to_bits()]);
    let entry = |binding, buffer| BindEntry { binding, resource: Resource::Buffer { buffer, offset: 0, size: 0 } };
    let Some(bg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: bgl, entries: &[entry(0, &ib), entry(1, &ob)] }), "bind group")
    else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, &bg);
    e.dispatch(1, 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    if crate::run_wait(t, dev, e.finish(), "NaN").is_none() {
        return;
    }
    let mut got = [0u32; 7];
    let _ = dev.queue().read_buffer(&ob, 0, &mut got);
    let one = 1.0f32.to_bits();
    let want = [one, one, 0, 1, 0, 0, 0];
    let names = ["min(NaN, 1) = 1", "max(1, NaN) = 1", "clamp(NaN, 0, 1) = 0", "NaN != NaN", "!(NaN == NaN)", "!(NaN < 1)", "!(NaN >= 1)"];
    let ok: Vec<&str> = names.iter().zip(got.iter().zip(&want)).filter(|(_, (g, w))| g == w).map(|(n, _)| *n).collect();
    let no: Vec<String> = names.iter().zip(got.iter().zip(&want)).filter(|(_, (g, w))| g != w).map(|(n, (g, _))| format!("{n} (got {g:#x})")).collect();
    out(&format!("  info {what}: NaN as WGSL defines it: {} of 7 ({}){}", ok.len(), ok.join(", "),
                 if no.is_empty() { String::new() } else { format!("; not: {}", no.join(", ")) }));
}

/// `--dump FILE`: the cases as JSON lines for tools/wgsl_run.mjs (Chrome runs the same shaders:
/// {"name", "n", "outw", "src", "inputs"})
fn dump(path: &str) -> bool {
    let esc = |s: &str| -> String {
        let mut o = String::new();
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                c => o.push(c),
            }
        }
        o
    };
    let mut text = String::new();
    for case in cases() {
        let n = case.inputs.len();
        let inp: Vec<String> = case.inputs.iter().flatten().map(|w| format!("{w}")).collect();
        text.push_str(&format!("{{\"name\":\"{}\",\"n\":{},\"outw\":{},\"src\":\"{}\",\"inputs\":[{}]}}\n", esc(case.name), n,
                               case.ret.words(), esc(&shader(&case, n)), inp.join(",")));
    }
    plat::write_file(path, text.as_bytes())
}

/// `--chrome FILE`: the results of tools/wgsl_run.mjs (lines `name<TAB>w,w,...`) against the
/// references with the same tolerances -> (cases, results compared, differences)
fn chrome(t: &mut T, path: &str) {
    let Some(b) = plat::read_file(path) else {
        t.fail(&format!("--chrome {path}: cannot read"));
        return;
    };
    let text = String::from_utf8_lossy(&b).into_owned();
    let list = cases();
    let (mut ncases, mut compared, mut bad_total) = (0, 0, 0);
    for line in text.lines() {
        let Some((name, words)) = line.split_once('\t') else { continue };
        let Some(case) = list.iter().find(|c| c.name == name) else { continue };
        let got: Vec<u32> = words.split(',').filter_map(|w| w.trim().parse().ok()).collect();
        let outw = case.ret.words();
        if got.len() != case.inputs.len() * outw {
            out(&format!("  info Chrome: {name}: {} result words, expected {}", got.len(), case.inputs.len() * outw));
            continue;
        }
        ncases += 1;
        let mut bad = Vec::new();
        for (k, input) in case.inputs.iter().enumerate() {
            let Some(want) = const_ref(case, input).or_else(|| case.rt.and_then(|f| f(input))) else { continue };
            compared += 1;
            let g = &got[k * outw..k * outw + outw];
            if !g.iter().zip(&want).all(|(&x, &y)| word_ok(case.ret.kind(), case.tol, x, y, input)) {
                bad.push((k, g.to_vec(), want));
            }
        }
        bad_total += bad.len();
        if let Some((k, g, w)) = bad.first() {
            let mut at = 0;
            let args: Vec<String> = case.args.iter().map(|ty| {
                let n = ty.words();
                let l = literal(*ty, &case.inputs[*k][at..at + n]);
                at += n;
                l
            }).collect();
            out(&format!("  info Chrome (Tint): {name}: {} of {} differ, e.g. ({}) -> {:x?}, expected {:x?}", bad.len(), case.inputs.len(),
                         args.join(", "), g, w));
        }
    }
    out(&format!("  info Chrome (Tint): {ncases} cases, {compared} results compared with lib/wgsl's constant evaluator, {bad_total} differ"));
    t.ok(ncases > 0, &format!("--chrome {path}: results read"));
}

pub fn wgsl(t: &mut T) {
    if let Some(p) = plat::arg_value("--dump") {
        t.ok(dump(&p), &format!("--dump {p}: the cases written"));
    }
    if let Some(p) = plat::arg_value("--chrome") {
        chrome(t, &p);
        return;
    }
    let Some(g) = setup(t) else { return };
    let minor = if g.dev.level() == Level::Voll { shader::VOLL } else { shader::BASIS };
    run_level(t, &g.dev, minor, &format!("SPIR-V 1.{minor}"));
    if minor == shader::VOLL {
        if let Some(basis) = t.res(g.adapter.create_device(&DeviceDesc { max_level: Level::Basis }), "Basis device") {
            run_level(t, &basis, shader::BASIS, "SPIR-V 1.3 (Basis device)");
        }
    }
}

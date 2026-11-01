// consts -- constant evaluation: AbstractInt against i64 (overflow is an error), AbstractFloat
// against f64, f32 results rounded once like Rust's f32 operations, i32 / u32 wrapping,
// conversions (truncating, clamping, rounding), the const built-ins, composites, f16

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use wgsl::ir::Value;

use crate::T;

/// the value of `const r = <e>;`, or the first error message
fn eval(e: &str) -> Result<Value, String> {
    let src = format!("enable f16;\nconst r = {};", e);
    match wgsl::check(&src) {
        Ok(m) => m.consts.last().map(|c| c.value.clone()).ok_or_else(|| String::from("no const")),
        Err(d) => Err(d.first().map(|d| d.msg.clone()).unwrap_or_default()),
    }
}

/// the value of `const r: T = <e>;`
fn eval_as(ty: &str, e: &str) -> Result<Value, String> {
    eval(&format!("{}({})", ty, e))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// an i64 of a random magnitude (small and large numbers both)
    fn int(&mut self) -> i64 {
        let bits = self.next() % 64;
        let v = (self.next() >> (63 - bits.min(62))) as i64;
        if self.next() & 1 == 0 { v } else { -v }
    }
    /// a finite f64 of a random magnitude
    fn float(&mut self) -> f64 {
        let m = (self.next() >> 11) as f64 / (1u64 << 53) as f64 + 0.5;
        let e = (self.next() % 80) as i32 - 40;
        let mut p = 1.0f64;
        for _ in 0..e.unsigned_abs() {
            p = if e > 0 { p * 2.0 } else { p * 0.5 };
        }
        let v = m * p;
        if self.next() & 1 == 0 { v } else { -v }
    }
}

/// a literal for an i64 (the most negative one needs an expression)
fn ilit(v: i64) -> String {
    if v == i64::MIN { String::from("(-9223372036854775807 - 1)") } else if v < 0 { format!("({})", v) } else { format!("{}", v) }
}

fn flit(v: f64, suffix: &str) -> String {
    if v < 0.0 { format!("(-{:e}{})", -v, suffix) } else { format!("{:e}{}", v, suffix) }
}

pub fn run(t: &mut T) {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    // AbstractInt: i64 with overflow errors
    let mut n_ok = 0;
    let mut n_bad = 0;
    for k in 0..300 {
        let (a, b) = (rng.int(), rng.int());
        let (op, want) = match k % 5 {
            0 => ("+", a.checked_add(b)),
            1 => ("-", a.checked_sub(b)),
            2 => ("*", a.checked_mul(b)),
            3 => ("/", if b == 0 { None } else { a.checked_div(b) }),
            _ => ("%", if b == 0 { None } else { a.checked_rem(b) }),
        };
        let got = eval(&format!("{} {} {}", ilit(a), op, ilit(b)));
        match want {
            Some(w) => {
                if got == Ok(Value::AInt(w)) {
                    n_ok += 1;
                } else {
                    n_bad += 1;
                    crate::out(&format!("    {} {} {}: got {:?}, want {}\n", a, op, b, got, w));
                }
            }
            None => {
                if got.is_err() {
                    n_ok += 1;
                } else {
                    n_bad += 1;
                    crate::out(&format!("    {} {} {}: got {:?}, want an error\n", a, op, b, got));
                }
            }
        }
    }
    t.eq("AbstractInt against i64", (n_ok, n_bad), (300, 0));
    // bit operations on AbstractInt
    let (a, b) = (0x0f0f_1234_5678_9abci64, -0x1234_5678i64);
    t.eq("abstract and", eval(&format!("{} & {}", ilit(a), ilit(b))), Ok(Value::AInt(a & b)));
    t.eq("abstract or", eval(&format!("{} | {}", ilit(a), ilit(b))), Ok(Value::AInt(a | b)));
    t.eq("abstract xor", eval(&format!("{} ^ {}", ilit(a), ilit(b))), Ok(Value::AInt(a ^ b)));
    t.eq("abstract not", eval(&format!("~{}", ilit(a))), Ok(Value::AInt(!a)));
    t.eq("abstract shl", eval("3 << 61u"), Ok(Value::AInt(3 << 61)));
    t.eq("abstract shl negative", eval("-1 << 63u"), Ok(Value::AInt(i64::MIN)));
    t.eq("abstract shr", eval("-16 >> 2u"), Ok(Value::AInt(-4)));
    t.eq("abstract shr far", eval("-16 >> 70u"), Ok(Value::AInt(-1)));
    t.eq("abstract min", eval("-9223372036854775807 - 1"), Ok(Value::AInt(i64::MIN)));
    t.check("abstract neg min overflows", eval("-(-9223372036854775807 - 1)").is_err());
    t.check("abstract abs min overflows", eval("abs(-9223372036854775807 - 1)").is_err());
    // AbstractFloat: f64
    let mut n_ok = 0;
    let mut n_bad = 0;
    for k in 0..300 {
        let (a, b) = (rng.float(), rng.float());
        let (op, want) = match k % 4 {
            0 => ("+", a + b),
            1 => ("-", a - b),
            2 => ("*", a * b),
            _ => ("/", a / b),
        };
        let got = eval(&format!("{} {} {}", flit(a, ""), op, flit(b, "")));
        if got == Ok(Value::AFloat(want)) {
            n_ok += 1;
        } else {
            n_bad += 1;
            crate::out(&format!("    {:e} {} {:e}: got {:?}, want {:e}\n", a, op, b, got, want));
        }
    }
    t.eq("AbstractFloat against f64", (n_ok, n_bad), (300, 0));
    // f32: one rounding
    let mut n_ok = 0;
    let mut n_bad = 0;
    for k in 0..300 {
        let (a, b) = (rng.float() as f32, rng.float() as f32);
        let (op, want) = match k % 4 {
            0 => ("+", a + b),
            1 => ("-", a - b),
            2 => ("*", a * b),
            _ => ("/", a / b),
        };
        let got = eval(&format!("{} {} {}", flit(a as f64, "f"), op, flit(b as f64, "f")));
        if got == Ok(Value::F32(want)) {
            n_ok += 1;
        } else {
            n_bad += 1;
            crate::out(&format!("    {:e}f {} {:e}f: got {:?}, want {:e}\n", a, op, b, got, want));
        }
    }
    t.eq("f32 against Rust's f32", (n_ok, n_bad), (300, 0));
    // abstract -> f32 rounds once
    t.eq("0.1 as f32", eval_as("f32", "0.1"), Ok(Value::F32(0.1f32)));
    t.eq("1/3 as f32", eval("f32(1.0 / 3.0)"), Ok(Value::F32(1.0f32 / 3.0)));
    t.eq("2^24 + 1 as f32", eval("f32(16777217)"), Ok(Value::F32(16777216.0)));
    t.eq("f32 sum of abstracts", eval("1.0f + 0.1"), Ok(Value::F32(1.0f32 + 0.1f32)));
    // i32 / u32: wrapping, division rules
    t.eq("i32 wraps", eval("2147483647i + 1i"), Ok(Value::I32(i32::MIN)));
    t.eq("u32 wraps", eval("0u - 1u"), Ok(Value::U32(u32::MAX)));
    t.eq("i32 mul wraps", eval("65536i * 65536i"), Ok(Value::I32(0)));
    t.eq("i32 division truncates", eval("-7i / 2i"), Ok(Value::I32(-3)));
    t.eq("i32 remainder sign", eval("-7i % 2i"), Ok(Value::I32(-1)));
    t.eq("u32 division", eval("7u / 2u"), Ok(Value::U32(3)));
    t.eq("float remainder", eval("-7.5 % 2.0"), Ok(Value::AFloat(-1.5)));
    t.eq("i32 negate min", eval("-i32(-2147483648)"), Ok(Value::I32(i32::MIN)));
    t.eq("u32 shift", eval("1u << 31u"), Ok(Value::U32(0x8000_0000)));
    t.eq("i32 shift right", eval("-8i >> 1u"), Ok(Value::I32(-4)));
    t.eq("-1 * i32(min) wraps", eval("-1 * i32(-2147483648)"), Ok(Value::I32(i32::MIN)));
    // conversions
    t.eq("f32 -> i32 truncates", eval("i32(3.9f)"), Ok(Value::I32(3)));
    t.eq("f32 -> i32 negative", eval("i32(-3.9f)"), Ok(Value::I32(-3)));
    t.eq("f32 -> u32 clamps at 0", eval("u32(-1.0f)"), Ok(Value::U32(0)));
    t.eq("f32 -> u32 clamps", eval("u32(1e20f)"), Ok(Value::U32(4294967040)));
    t.eq("f32 -> i32 clamps", eval("i32(1e20f)"), Ok(Value::I32(2147483520)));
    t.eq("f32 -> i32 clamps low", eval("i32(-1e20f)"), Ok(Value::I32(i32::MIN)));
    t.eq("abstract float -> i32", eval("i32(1e20)"), Ok(Value::I32(i32::MAX)));
    t.eq("i32 -> u32 reinterprets", eval("u32(-1i)"), Ok(Value::U32(u32::MAX)));
    t.eq("u32 -> i32 reinterprets", eval("i32(4294967295u)"), Ok(Value::I32(-1)));
    t.eq("bool -> f32", eval("f32(true)"), Ok(Value::F32(1.0)));
    t.eq("f32 -> bool", eval("bool(-0.0f)"), Ok(Value::Bool(false)));
    t.eq("i32 -> f32 rounds", eval("f32(16777217i)"), Ok(Value::F32(16777216.0)));
    t.eq("u32 -> f16", eval("f16(3u)"), Ok(Value::F16(3.0)));
    t.check("u32 -> f16 out of range", eval("f16(70000u)").is_err());
    t.eq("f32 -> f16 rounds", eval("f16(0.1f)"), Ok(Value::F16(0.0999755859375)));
    t.check("f32 -> f16 overflow", eval("f16(1e5f)").is_err());
    t.eq("vector conversion", eval("vec2i(vec2f(1.5, -2.5))"), Ok(Value::Comp(alloc::vec![Value::I32(1), Value::I32(-2)])));
    t.eq("abstract vector to f32", eval("vec2f(vec2(1, 2))"), Ok(Value::Comp(alloc::vec![Value::F32(1.0), Value::F32(2.0)])));
    // built-ins
    t.eq("sqrt exact", eval("sqrt(2.0)"), Ok(Value::AFloat(1.4142135623730951)));
    t.eq("sqrt f32", eval("sqrt(2.0f)"), Ok(Value::F32(1.4142135)));
    t.eq("sin f32", eval("sin(1.0f)"), Ok(Value::F32(f32::from_bits(0x3f57_6aa4))));
    t.eq("cos f32", eval("cos(1.0f)"), Ok(Value::F32(f32::from_bits(0x3f0a_5140))));
    t.eq("exp f32", eval("exp(1.0f)"), Ok(Value::F32(core::f32::consts::E)));
    t.eq("log2 exact", eval("log2(8.0)"), Ok(Value::AFloat(3.0)));
    t.eq("exp2 exact", eval("exp2(10.0)"), Ok(Value::AFloat(1024.0)));
    t.eq("pow integer", eval("pow(3.0, 4.0)"), Ok(Value::AFloat(81.0)));
    t.eq("pow f32", eval("pow(2.0f, 0.5f)"), Ok(Value::F32(core::f32::consts::SQRT_2)));
    t.eq("atan2", eval("atan2(1.0f, 1.0f)"), Ok(Value::F32(core::f32::consts::FRAC_PI_4)));
    t.eq("acos", eval("acos(-1.0f)"), Ok(Value::F32(core::f32::consts::PI)));
    t.eq("asin", eval("asin(1.0f)"), Ok(Value::F32(core::f32::consts::FRAC_PI_2)));
    t.eq("tanh", eval("tanh(0.5f)"), Ok(Value::F32(0.46211717)));
    t.eq("log abstract int", eval("log2(32)"), Ok(Value::AFloat(5.0)));
    t.eq("floor", eval("floor(-1.5)"), Ok(Value::AFloat(-2.0)));
    t.eq("ceil", eval("ceil(-1.5)"), Ok(Value::AFloat(-1.0)));
    t.eq("round half even", eval("vec4(round(0.5), round(1.5), round(2.5), round(-0.5))"), Ok(Value::Comp(alloc::vec![Value::AFloat(0.0), Value::AFloat(2.0), Value::AFloat(2.0), Value::AFloat(-0.0)])));
    t.eq("trunc", eval("trunc(-2.7)"), Ok(Value::AFloat(-2.0)));
    t.eq("fract", eval("fract(2.75f)"), Ok(Value::F32(0.75)));
    t.eq("clamp int", eval("clamp(7, 0, 5)"), Ok(Value::AInt(5)));
    t.eq("max mixed abstract", eval("max(1, 2.5)"), Ok(Value::AFloat(2.5)));
    t.eq("min u32", eval("min(3u, 2)"), Ok(Value::U32(2)));
    t.eq("abs f32", eval("abs(-2.5f)"), Ok(Value::F32(2.5)));
    t.eq("sign", eval("vec3(sign(-3), sign(0), sign(2))"), Ok(Value::Comp(alloc::vec![Value::AInt(-1), Value::AInt(0), Value::AInt(1)])));
    t.eq("step", eval("step(1.0, 2.0)"), Ok(Value::AFloat(1.0)));
    t.eq("smoothstep", eval("smoothstep(0.0, 1.0, 0.5)"), Ok(Value::AFloat(0.5)));
    t.eq("mix", eval("mix(1.0, 3.0, 0.25)"), Ok(Value::AFloat(1.5)));
    t.eq("fma", eval("fma(2.0, 3.0, 4.0)"), Ok(Value::AFloat(10.0)));
    t.eq("dot", eval("dot(vec3(1, 2, 3), vec3(4, 5, 6))"), Ok(Value::AInt(32)));
    t.eq("dot f32", eval("dot(vec2f(1.5, 2.0), vec2f(2.0, 4.0))"), Ok(Value::F32(11.0)));
    t.eq("cross", eval("cross(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0))"), Ok(Value::Comp(alloc::vec![Value::AFloat(0.0), Value::AFloat(0.0), Value::AFloat(1.0)])));
    t.eq("length", eval("length(vec2(3.0, 4.0))"), Ok(Value::AFloat(5.0)));
    t.eq("distance", eval("distance(vec2f(1.0, 1.0), vec2f(4.0, 5.0))"), Ok(Value::F32(5.0)));
    t.eq("normalize", eval("normalize(vec2(3.0, 4.0))"), Ok(Value::Comp(alloc::vec![Value::AFloat(0.6), Value::AFloat(0.8)])));
    t.eq("determinant", eval("determinant(mat2x2(1.0, 2.0, 3.0, 4.0))"), Ok(Value::AFloat(-2.0)));
    t.eq("determinant 3x3", eval("determinant(mat3x3(2.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 4.0))"), Ok(Value::AFloat(24.0)));
    t.eq("transpose", eval("transpose(mat2x3(1.0, 2.0, 3.0, 4.0, 5.0, 6.0))[1]"), Ok(Value::Comp(alloc::vec![Value::AFloat(2.0), Value::AFloat(5.0)])));
    t.eq("matrix * vector", eval("mat2x2(1.0, 2.0, 3.0, 4.0) * vec2(1.0, 1.0)"), Ok(Value::Comp(alloc::vec![Value::AFloat(4.0), Value::AFloat(6.0)])));
    t.eq("vector * matrix", eval("vec2(1.0, 1.0) * mat2x2(1.0, 2.0, 3.0, 4.0)"), Ok(Value::Comp(alloc::vec![Value::AFloat(3.0), Value::AFloat(7.0)])));
    t.eq("matrix * matrix", eval("(mat2x2(1.0, 2.0, 3.0, 4.0) * mat2x2(1.0, 0.0, 0.0, 1.0))[1]"), Ok(Value::Comp(alloc::vec![Value::AFloat(3.0), Value::AFloat(4.0)])));
    t.eq("matrix * scalar", eval("(mat2x2f(1.0, 2.0, 3.0, 4.0) * 2.0)[1][1]"), Ok(Value::F32(8.0)));
    t.eq("frexp", eval("frexp(12.0).exp"), Ok(Value::AInt(4)));
    t.eq("frexp fraction", eval("frexp(12.0f).fract"), Ok(Value::F32(0.75)));
    t.eq("modf", eval("modf(-2.75).fract"), Ok(Value::AFloat(-0.75)));
    t.eq("modf whole", eval("modf(vec2f(1.5, -2.25)).whole"), Ok(Value::Comp(alloc::vec![Value::F32(1.0), Value::F32(-2.0)])));
    t.eq("ldexp", eval("ldexp(0.75, 4)"), Ok(Value::AFloat(12.0)));
    t.eq("ldexp f32", eval("ldexp(1.0f, -149)"), Ok(Value::F32(f32::from_bits(1))));
    t.eq("countOneBits", eval("countOneBits(0xf0f0u)"), Ok(Value::U32(8)));
    t.eq("countLeadingZeros", eval("countLeadingZeros(1i)"), Ok(Value::I32(31)));
    t.eq("countTrailingZeros", eval("countTrailingZeros(8u)"), Ok(Value::U32(3)));
    t.eq("firstLeadingBit signed", eval("vec3(firstLeadingBit(-1i), firstLeadingBit(-16i), firstLeadingBit(0x80i))"), Ok(Value::Comp(alloc::vec![Value::I32(-1), Value::I32(3), Value::I32(7)])));
    t.eq("firstLeadingBit unsigned", eval("firstLeadingBit(0u)"), Ok(Value::U32(u32::MAX)));
    t.eq("firstTrailingBit", eval("firstTrailingBit(0x50u)"), Ok(Value::U32(4)));
    t.eq("reverseBits", eval("reverseBits(1u)"), Ok(Value::U32(0x8000_0000)));
    t.eq("extractBits signed", eval("extractBits(0xf0i, 4u, 4u)"), Ok(Value::I32(-1)));
    t.eq("extractBits unsigned", eval("extractBits(0xf0u, 4u, 4u)"), Ok(Value::U32(15)));
    t.eq("insertBits", eval("insertBits(0u, 0xffu, 4u, 4u)"), Ok(Value::U32(0xf0)));
    t.eq("insertBits full", eval("insertBits(1u, 7u, 0u, 32u)"), Ok(Value::U32(7)));
    t.eq("pack4x8unorm", eval("pack4x8unorm(vec4f(1.0, 0.0, 0.5, 1.0))"), Ok(Value::U32(0xff80_00ff)));
    t.eq("pack4x8snorm", eval("pack4x8snorm(vec4f(1.0, -1.0, 0.0, 0.5))"), Ok(Value::U32(0x4000_817f)));
    t.eq("pack2x16float", eval("pack2x16float(vec2f(1.0, -2.0))"), Ok(Value::U32(0xc000_3c00)));
    t.eq("pack4xI8", eval("pack4xI8(vec4i(-1, 2, -3, 4))"), Ok(Value::U32(0x04fd_02ff)));
    t.eq("pack4xU8Clamp", eval("pack4xU8Clamp(vec4u(300u, 2u, 3u, 4u))"), Ok(Value::U32(0x0403_02ff)));
    t.eq("unpack4x8unorm", eval("unpack4x8unorm(0xff00ff00u)"), Ok(Value::Comp(alloc::vec![Value::F32(0.0), Value::F32(1.0), Value::F32(0.0), Value::F32(1.0)])));
    t.eq("unpack2x16float", eval("unpack2x16float(0xc0003c00u)"), Ok(Value::Comp(alloc::vec![Value::F32(1.0), Value::F32(-2.0)])));
    t.eq("unpack4xI8", eval("unpack4xI8(0x04fd02ffu)"), Ok(Value::Comp(alloc::vec![Value::I32(-1), Value::I32(2), Value::I32(-3), Value::I32(4)])));
    t.eq("dot4U8Packed", eval("dot4U8Packed(0x01020304u, 0x01010101u)"), Ok(Value::U32(10)));
    t.eq("dot4I8Packed", eval("dot4I8Packed(0xffffffffu, 0x01010101u)"), Ok(Value::I32(-4)));
    t.eq("quantizeToF16", eval("quantizeToF16(0.1f)"), Ok(Value::F32(0.0999755859375)));
    t.eq("select", eval("select(1, 2, true)"), Ok(Value::AInt(2)));
    t.eq("select vector", eval("select(vec2(1, 2), vec2(3, 4), vec2(true, false))"), Ok(Value::Comp(alloc::vec![Value::AInt(3), Value::AInt(2)])));
    t.eq("all", eval("all(vec3(true, true, false))"), Ok(Value::Bool(false)));
    t.eq("any", eval("any(vec2(false, true))"), Ok(Value::Bool(true)));
    t.eq("bitcast", eval("bitcast<u32>(1.0f)"), Ok(Value::U32(0x3f80_0000)));
    t.eq("bitcast f16 pair", eval("bitcast<u32>(vec2h(1.0h, -2.0h))"), Ok(Value::U32(0xc000_3c00)));
    t.eq("bitcast to f16 pair", eval("bitcast<vec2h>(0xc0003c00u)"), Ok(Value::Comp(alloc::vec![Value::F16(1.0), Value::F16(-2.0)])));
    t.check("degrees", matches!(eval("degrees(3.141592653589793)"), Ok(Value::AFloat(x)) if (x - 180.0).abs() < 1e-12));
    t.check("radians", matches!(eval("radians(90.0f)"), Ok(Value::F32(x)) if x == core::f32::consts::FRAC_PI_2));
    t.check("exp abstract", matches!(eval("exp(1.0)"), Ok(Value::AFloat(x)) if (x - core::f64::consts::E).abs() < 1e-15));
    t.check("sin abstract", matches!(eval("sin(1.0)"), Ok(Value::AFloat(x)) if (x - 0.8414709848078965).abs() < 1e-15));
    t.check("atan abstract", matches!(eval("atan(0.5)"), Ok(Value::AFloat(x)) if (x - 0.4636476090008061).abs() < 1e-15));
    t.check("log abstract", matches!(eval("log(10.0)"), Ok(Value::AFloat(x)) if (x - core::f64::consts::LN_10).abs() < 1e-15));
    t.check("cosh abstract", matches!(eval("cosh(2.0)"), Ok(Value::AFloat(x)) if (x - 3.7621956910836314).abs() < 1e-14));
    t.check("asinh abstract", matches!(eval("asinh(0.25)"), Ok(Value::AFloat(x)) if (x - 0.24746646154726346).abs() < 1e-15));
    t.check("acosh abstract", matches!(eval("acosh(3.0)"), Ok(Value::AFloat(x)) if (x - 1.762747174039086).abs() < 1e-15));
    t.check("atanh abstract", matches!(eval("atanh(0.75)"), Ok(Value::AFloat(x)) if (x - 0.9729550745276566).abs() < 1e-15));
    t.check("tan abstract", matches!(eval("tan(1.25)"), Ok(Value::AFloat(x)) if (x - 3.0095696738628313).abs() < 1e-14));
    t.check("sin large", matches!(eval("sin(1000.0)"), Ok(Value::AFloat(x)) if (x - 0.8268795405320025).abs() < 1e-13));
    t.eq("saturate", eval("saturate(vec2(-1.0, 2.0))"), Ok(Value::Comp(alloc::vec![Value::AFloat(0.0), Value::AFloat(1.0)])));
    t.eq("faceForward", eval("faceForward(vec2(1.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 0.0))"), Ok(Value::Comp(alloc::vec![Value::AFloat(-1.0), Value::AFloat(-0.0)])));
    t.eq("reflect", eval("reflect(vec2(1.0, -1.0), vec2(0.0, 1.0))"), Ok(Value::Comp(alloc::vec![Value::AFloat(1.0), Value::AFloat(1.0)])));
    // composites and access
    t.eq("array index", eval("array(1, 2, 3)[1]"), Ok(Value::AInt(2)));
    t.eq("vector swizzle", eval("vec4(1, 2, 3, 4).wzx"), Ok(Value::Comp(alloc::vec![Value::AInt(4), Value::AInt(3), Value::AInt(1)])));
    t.eq("matrix column", eval("mat2x2(1.0, 2.0, 3.0, 4.0)[1]"), Ok(Value::Comp(alloc::vec![Value::AFloat(3.0), Value::AFloat(4.0)])));
    t.eq("zero struct member", eval("frexp(0.0).exp"), Ok(Value::AInt(0)));
    t.eq("vector compare", eval("vec2(1, 2) < vec2(2, 2)"), Ok(Value::Comp(alloc::vec![Value::Bool(true), Value::Bool(false)])));
    t.eq("vector + scalar", eval("vec2(1, 2) + 10"), Ok(Value::Comp(alloc::vec![Value::AInt(11), Value::AInt(12)])));
    t.eq("short-circuit value", eval("false && (1 / 0 == 0)"), Ok(Value::Bool(false)));
    t.eq("zero value", eval("vec3<f32>()"), Ok(Value::Zero));
    t.eq("zero element", eval("array<vec2f, 4>()[3].y"), Ok(Value::F32(0.0)));
    t.eq("big zero array element", eval("array<f32, 65535>()[65534]"), Ok(Value::F32(0.0)));
    // f16
    t.eq("f16 sum rounds", eval("1.0h + 0.0001h"), Ok(Value::F16(1.0)));
    t.eq("f16 product", eval("1.5h * 3.0h"), Ok(Value::F16(4.5)));
    t.check("f16 overflow", eval("60000.0h * 2.0h").is_err());
    t.eq("abstract to f16", eval("f16(0.1)"), Ok(Value::F16(0.0999755859375)));
    // errors with positions are in checks.rs; here a few more values that must fail
    for bad in ["1.0 / 0.0", "sqrt(-4.0f)", "log(-1.0)", "acos(2.0)", "asin(-1.5f)", "atanh(1.0)", "inverseSqrt(0.0)", "exp(1000.0)", "f32(1e39)", "i32(2147483648)", "2147483648i"] {
        t.check(&format!("{} fails", bad), eval(bad).is_err());
    }
    // integers in WGSL literals and back
    let lits: Vec<i64> = (0..20).map(|_| rng.int()).collect();
    let mut all_ok = true;
    for v in lits {
        if eval(&ilit(v)) != Ok(Value::AInt(v)) {
            all_ok = false;
        }
    }
    t.check("integer literals round trip", all_ok);
}

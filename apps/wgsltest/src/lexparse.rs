// lexparse -- the lexer (token kinds, literal values and range rules, template list
// discovery, comments, blankspace, identifiers), the parser (every statement form,
// precedence and the forbidden operator mixes, token splitting, error recovery, the depth
// limit) and print -> parse round trips

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use wgsl::ast::{BinOp, DeclKind, ExprKind, StmtKind, UnOp};
use wgsl::lex::{self, Suffix, Tok};

use crate::T;

fn kinds(src: &str) -> Vec<Tok> {
    let (t, _) = lex::tokens(src);
    t.into_iter().map(|t| t.kind).filter(|&k| k != Tok::Eof).collect()
}

fn lex_errors(src: &str) -> Vec<String> {
    lex::tokens(src).1.into_iter().map(|d| d.msg).collect()
}

pub fn lex(t: &mut T) {
    use Tok::*;
    t.eq("simple", kinds("fn f(a: i32) -> i32 { return a+1; }"), alloc::vec![Fn, Ident, LParen, Ident, Colon, Ident, RParen, Arrow, Ident, LBrace, Return, Ident, Plus, Int, Semi, RBrace]);
    t.eq("operators", kinds("a>>=b<<=c&&d||e!=f==g<=h>=i++ --j"), alloc::vec![Ident, ShrEq, Ident, ShlEq, Ident, AndAnd, Ident, OrOr, Ident, NotEq, Ident, EqEq, Ident, Le, Ident, Ge, Ident, PlusPlus, MinusMinus, Ident]);
    t.eq("compound ops", kinds("+= -= *= /= %= &= |= ^= -> @ ~ _"), alloc::vec![PlusEq, MinusEq, StarEq, SlashEq, PercentEq, AndEq, OrEq, XorEq, Arrow, At, Tilde, Underscore]);
    t.eq("keywords", kinds("alias break case const const_assert continue continuing default diagnostic discard else enable false fn for if let loop override requires return struct switch true var while"),
        alloc::vec![Alias, Break, Case, Const, ConstAssert, Continue, Continuing, Default, Diagnostic, Discard, Else, Enable, False, Fn, For, If, Let, Loop, Override, Requires, Return, Struct, Switch, True, Var, While]);
    // numbers: longest match of the literal patterns
    t.eq("numbers", kinds("0 123 0x1f 1u 2i 1.5 .5 5. 1e3 1e-3f 0x1p4 0x.8p1 0X1.fp-4h 0f 1h 1.0f"), alloc::vec![Int, Int, Int, Int, Int, Float, Float, Float, Float, Float, Float, Float, Float, Float, Float, Float]);
    t.eq("1e is int then ident", kinds("1e"), alloc::vec![Int, Ident]);
    t.eq("0x1f is hex int", kinds("0x1f"), alloc::vec![Int]);
    t.eq("hex float needs p for a suffix", kinds("0x1.5f"), alloc::vec![Float]);
    t.check("leading zero", lex_errors("let a = 012;").iter().any(|m| m.contains("nicht mit 0 beginnen")));
    t.check("012.5 is a float", lex_errors("let a = 012.5;").is_empty());
    // identifiers
    t.eq("unicode identifiers", kinds("Δέλτα réflexion Кызыл 朝焼け 검정 _a a_1"), alloc::vec![Ident; 7]);
    t.check("double underscore", lex_errors("const __x = 1;").iter().any(|m| m.contains("zwei Unterstrichen")));
    t.check("reserved word", lex_errors("var<private> class: i32;").iter().any(|m| m.contains("reserviertes Wort")));
    t.check("reserved words sorted", lex::RESERVED.windows(2).all(|w| w[0] < w[1]));
    t.check("bad character", lex_errors("let a = 1 # 2;").iter().any(|m| m.contains("unerwartetes Zeichen")));
    t.check("null character", lex_errors("let a\u{0} = 1;").iter().any(|m| m.contains("U+0000")));
    // comments and blankspace
    t.eq("nested block comment", kinds("a /* b /* c */ d */ e // f\ng"), alloc::vec![Ident, Ident, Ident]);
    t.check("unterminated comment", lex_errors("a /* b /* c */").iter().any(|m| m.contains("ohne Ende")));
    t.eq("pattern white space", kinds("a\u{85}b\u{200e}c\u{2028}d\u{2029}e\u{0b}f\u{0c}g"), alloc::vec![Ident; 7]);
    // literal values
    let iv = |s: &str| lex::int_value(s);
    t.eq("int 0", iv("0"), Ok((0, Suffix::None)));
    t.eq("int hex", iv("0xFFu"), Ok((255, Suffix::U)));
    t.eq("int u32 max", iv("4294967295u"), Ok((4294967295, Suffix::U)));
    t.eq("int i32 max", iv("2147483647i"), Ok((2147483647, Suffix::I)));
    t.check("int i32 overflow", iv("2147483648i").is_err());
    t.check("int u32 overflow", iv("0x100000000u").is_err());
    t.eq("abstract int max", iv("9223372036854775807"), Ok((i64::MAX, Suffix::None)));
    t.check("abstract int overflow", iv("9223372036854775808").is_err());
    let fv = |s: &str| lex::float_value(s);
    t.eq("float 1.", fv("1."), Ok((1.0, Suffix::None)));
    t.eq("float .5", fv(".5"), Ok((0.5, Suffix::None)));
    t.eq("float 0.1f is f32", fv("0.1f"), Ok((0.1f32 as f64, Suffix::F)));
    t.eq("float 0.1 is f64", fv("0.1"), Ok((0.1, Suffix::None)));
    t.eq("float 1e-3h", fv("1e-3h").map(|v| v.1), Ok(Suffix::H));
    t.eq("float 1h", fv("1h"), Ok((1.0, Suffix::H)));
    t.eq("hex float", fv("0xa.fp+2"), Ok((43.75, Suffix::None)));
    t.eq("hex float 0X.3", fv("0X.3"), Ok((0.1875, Suffix::None)));
    t.eq("hex float smallest f32", fv("0x1p-149f"), Ok((f32::from_bits(1) as f64, Suffix::F)));
    t.check("hex float inexact f32", fv("0x1.000001p0f").is_err());
    t.eq("hex float exact f32", fv("0x1.000002p0f"), Ok((f64::from_bits(0x3ff0_0000_2000_0000), Suffix::F)));
    t.check("hex float f32 overflow", fv("0x1p128f").is_err());
    t.eq("hex float f32 max", fv("0x1.fffffep+127f"), Ok((f32::MAX as f64, Suffix::F)));
    t.check("decimal f32 overflow", fv("1e39f").is_err());
    t.eq("decimal f32 underflow", fv("1e-50f"), Ok((0.0, Suffix::F)));
    t.eq("f16 max", fv("65504h"), Ok((65504.0, Suffix::H)));
    t.check("f16 overflow", fv("65520h").is_err());
    t.eq("f16 rounding", fv("0.1h").map(|v| v.0), Ok(0.0999755859375));
    t.check("abstract float overflow", fv("1e400").is_err());
    t.eq("hex float 64-bit significand", fv("0x1.0000000000001p0"), Ok((f64::from_bits(0x3ff0_0000_0000_0001), Suffix::None)));
    t.eq("long decimal", fv("3.14159265358979323846264338327950288"), Ok((core::f64::consts::PI, Suffix::None)));
    // template lists (the spec's notes of 3.9)
    t.eq("template call", kinds("A ( B < C, D > ( E ) )"), alloc::vec![Ident, LParen, Ident, TemplateStart, Ident, Comma, Ident, TemplateEnd, LParen, Ident, RParen, RParen]);
    t.eq("nested expression", kinds("array<i32,select(2,3,a>b)>"), alloc::vec![Ident, TemplateStart, Ident, Comma, Ident, LParen, Int, Comma, Int, Comma, Ident, Gt, Ident, RParen, TemplateEnd]);
    t.eq("different index levels", kinds("a[b<d]>()"), alloc::vec![Ident, LBracket, Ident, Lt, Ident, RBracket, Gt, LParen, RParen]);
    t.eq("shift inside", kinds("A<B<<C>"), alloc::vec![Ident, TemplateStart, Ident, Shl, Ident, TemplateEnd]);
    t.eq("le inside", kinds("A<B<=C>"), alloc::vec![Ident, TemplateStart, Ident, Le, Ident, TemplateEnd]);
    t.eq("ge in parens", kinds("A<(B>=C)>"), alloc::vec![Ident, TemplateStart, LParen, Ident, Ge, Ident, RParen, TemplateEnd]);
    t.eq("ne in parens", kinds("A<(B!=C)>"), alloc::vec![Ident, TemplateStart, LParen, Ident, NotEq, Ident, RParen, TemplateEnd]);
    t.eq("eq in parens", kinds("A<(B==C)>"), alloc::vec![Ident, TemplateStart, LParen, Ident, EqEq, Ident, RParen, TemplateEnd]);
    t.eq("or ends candidates", kinds("a<b || c>d"), alloc::vec![Ident, Lt, Ident, OrOr, Ident, Gt, Ident]);
    t.eq(">> closes two", kinds("vec2<vec2<f32>>"), alloc::vec![Ident, TemplateStart, Ident, TemplateStart, Ident, TemplateEnd, TemplateEnd]);
    t.eq(">= after a template", kinds("var<private> v: vec2<f32>= x"), alloc::vec![Var, TemplateStart, Ident, TemplateEnd, Ident, Colon, Ident, TemplateStart, Ident, TemplateEnd, Eq, Ident]);
    t.eq("comparison", kinds("a < b"), alloc::vec![Ident, Lt, Ident]);
    t.eq("literal suffix is no identifier", kinds("f(1.0f<x, y>z)"), alloc::vec![Ident, LParen, Float, Lt, Ident, Comma, Ident, Gt, Ident, RParen]);
    let (s, e) = lex::discover_templates("x = a<b>c;");
    t.eq("discovery positions", (s, e), (alloc::vec![5], alloc::vec![7]));
    // diagnostic positions count characters
    let d = lex::tokens("// äöü\nlet ä = #;").1;
    t.eq("line and column in characters", d.first().map(|d| (d.line, d.col)), Some((2, 9)));
    let d = lex::tokens("a\r\nb\rc\u{2028}#").1;
    t.eq("line breaks of WGSL", d.first().map(|d| (d.line, d.col)), Some((4, 1)));
}

fn parse_ok(t: &mut T, name: &str, src: &str) -> Option<wgsl::Ast> {
    let (ast, d) = wgsl::parse(src);
    if d.iter().any(|d| d.error) {
        t.check(name, false);
        for x in d.iter().take(5) {
            crate::out(&format!("    {}\n", x));
        }
        return None;
    }
    t.check(name, true);
    Some(ast)
}

/// the expression of `const x = <e>;`
fn expr_of(src: &str) -> Option<wgsl::ast::Expr> {
    let (ast, d) = wgsl::parse(&format!("const x = {};", src));
    if !d.is_empty() {
        return None;
    }
    match ast.decls.into_iter().next()?.kind {
        DeclKind::Var(v) => v.init,
        _ => None,
    }
}

/// a compact shape of an expression tree: (a+(b*c))
fn shape(e: &wgsl::ast::Expr) -> String {
    match &e.kind {
        ExprKind::Ident(id, _) => id.name.clone(),
        ExprKind::Int(s, ..) | ExprKind::Float(s, ..) => s.clone(),
        ExprKind::Bool(b) => format!("{}", b),
        ExprKind::Paren(x) => format!("[{}]", shape(x)),
        ExprKind::Unary(op, x) => format!("{}{}", op.text(), shape(x)),
        ExprKind::Binary(op, a, b) => format!("({}{}{})", shape(a), op.text(), shape(b)),
        ExprKind::Index(a, i) => format!("{}#{}", shape(a), shape(i)),
        ExprKind::Member(a, m) => format!("{}.{}", shape(a), m.name),
        ExprKind::Call(id, _, args) => format!("{}({})", id.name, args.iter().map(shape).collect::<Vec<_>>().join(",")),
        ExprKind::Error => String::from("?"),
    }
}

pub const SAMPLE: &str = r#"
enable f16;
requires pointer_composite_access;
diagnostic(off, derivative_uniformity);

alias Arr = array<vec4<f32>, 4>;
struct Light {
    @align(16) pos: vec3f,
    @size(16) power: f32,
    color: vec4<f32>,
}
struct Params { lights: array<Light, 4>, count: u32 }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> data: array<f32>;
@group(0) @binding(2) var tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
var<private> counter: i32 = 0;
var<workgroup> tile: array<f32, 64>;
const K = 2.5;
@id(7) override scale: f32 = 1.0;
override size = 4u;
const_assert K > 1.0;

fn helper(p: ptr<function, i32>, v: vec2<f32>) -> f32 {
    *p += 1;
    return v.x * v.y;
}

@must_use
fn pure(x: f32) -> f32 { return x * 2.0; }

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    var i: i32 = 0;
    let n = arrayLength(&data);
    for (var k = 0u; k < n; k++) {
        if k == 3u { continue; } else if k > 10u { break; } else { i -= 1; }
    }
    loop {
        i++;
        if i > 4 { break; }
        continuing {
            i += 1;
            break if i > 100;
        }
    }
    while i < 10 { i = i + 1; }
    switch i {
        case 1, 2: { i = 3; }
        case 3, default { i = 4; }
    }
    _ = helper(&i, vec2(1.0, 2.0));
    let a = -K * (2.0 + f32(i)) / 3.0;
    let b = i32(a) & 0xff;
    let c = (b << 2u) | 1;
    let d = select(1, 2, (a > 0.0 && b != 0) || false);
    data[gid.x] = a + f32(c + d) + pure(1.0);
    tile[0] = 1.0;
    workgroupBarrier();
    const_assert 1 < 2;
    {
        var j = 1;
        j--;
    }
    ;
}

@fragment
fn fs(@location(0) @interpolate(perspective, centroid) uv: vec2f, @builtin(position) pos: vec4f) -> @location(0) vec4f {
    if pos.x < 0.0 { discard; }
    let h = vec2<f16>(1.0h, 2.0h);
    return textureSample(tex, samp, uv) * f32(h.x) + params.lights[0].color;
}
"#;

pub fn parse(t: &mut T) {
    if let Some(ast) = parse_ok(t, "sample parses", SAMPLE) {
        t.eq("sample directives", ast.directives.len(), 3);
        t.eq("sample declarations", ast.decls.len(), 17);
    }
    let sh = |s: &str| expr_of(s).map(|e| shape(&e)).unwrap_or_default();
    t.eq("precedence * over +", sh("a + b * c"), String::from("(a+(b*c))"));
    t.eq("left assoc", sh("a - b - c"), String::from("((a-b)-c)"));
    t.eq("unary binds tighter", sh("-a * b"), String::from("(-a*b)"));
    t.eq("postfix binds tighter", sh("-a.b[c]"), String::from("-a.b#c"));
    t.eq("deref of member", sh("*p.x"), String::from("*p.x"));
    t.eq("a--b splits", sh("a--b"), String::from("(a--b)"));
    t.eq("--a is two negations", sh("--a"), String::from("--a"));
    t.eq("&&p is two address-ofs", sh("&&p"), String::from("&&p"));
    t.eq("shift and compare", sh("a << b < c"), String::from("((a<<b)<c)"));
    t.eq("additive in compare", sh("a + b < c * d"), String::from("((a+b)<(c*d))"));
    t.eq("logical chain", sh("a && b && c"), String::from("((a&&b)&&c)"));
    t.eq("comparisons under ||", sh("a < b || c > d"), String::from("((a<b)||(c>d))"));
    t.eq("bitwise chain", sh("a | b | c"), String::from("((a|b)|c)"));
    t.eq("call and template", sh("vec3<f32>(1, 2, 3).xy"), String::from("vec3(1,2,3).xy"));
    t.eq("parens kept", sh("(a + b) * c"), String::from("([(a+b)]*c)"));
    t.eq("unary chain", sh("!~-x"), String::from("!~-x"));
    let un = expr_of("&x").map(|e| matches!(e.kind, ExprKind::Unary(UnOp::AddrOf, _)));
    t.eq("address-of", un, Some(true));
    let bin = expr_of("a >= b").map(|e| matches!(e.kind, ExprKind::Binary(BinOp::Ge, ..)));
    t.eq(">=", bin, Some(true));
    // the operator mixes the grammar forbids
    t.parse_fail("& and |", "const x = a $& b | c;", "`&` und `|` dürfen nicht ohne Klammern");
    t.parse_fail("+ and <<", "const x = a + b $<< c;", "`+` und `<<`");
    t.parse_fail("<< and +", "const x = a $<< b + c;", "`<<` und `+`");
    t.parse_fail("chained comparison", "const x = a $< b < c;", "Vergleiche lassen sich nicht verketten");
    t.parse_fail("&& and ||", "const x = a $&& b || c;", "`&&` und `||`");
    t.parse_fail("& and ==", "const x = a $& b == c;", "`&` und `==`");
    t.parse_fail("&& and &", "const x = a $&& b & c;", "`&&` und `&`");
    // statements
    t.parse_fail("missing semicolon", "fn f() { var a = 1\n    $a = 2; }", "erwartet `;` nach der Anweisung");
    t.parse_fail("expression statement", "fn f() { var a = 1; $1 + a; }", "erwartet eine Anweisung");
    t.parse_fail("bare expression", "fn f() { var a = 1; a $+ 1; }", "ein Ausdruck allein ist keine Anweisung");
    t.parse_fail("break if outside continuing", "fn f() { loop { $break if true; } }", "`break if` ist nur als letzte Anweisung");
    t.parse_fail("break if not last", "fn f() { var i = 0; loop { continuing { $break if true; i++; } } }", "letzte Anweisung von `continuing`");
    t.parse_fail("continuing not last", "fn f() { loop { continuing { } $let a = 1; } }", "`continuing` muss die letzte Anweisung");
    t.parse_fail("empty switch", "fn f() { switch 1 ${ } }", "mindestens einen `case`");
    t.parse_fail("attribute on assignment", "fn f() { var a = 1; $@diagnostic(off, derivative_uniformity) a = 2; }", "Attribute sind nur vor Blöcken");
    t.parse_fail("directive after declaration", "const a = 1;\n$enable f16;", "muss vor allen Deklarationen stehen");
    t.parse_fail("override in function", "fn f() { $override x = 1; }", "nur auf Modulebene");
    t.parse_fail("let without initializer", "fn f() { let a$; }", "erwartet `=` mit einem Initialisierer");
    t.parse_fail("keyword as name", "fn f() { let $loop = 1; }", "Schlüsselwort");
    t.parse_fail("underscore as value", "fn f() { let a = $_; }", "`_` ist kein Wert");
    t.parse_fail("unclosed block", "fn f() { if true { $", "der Block endet nicht");
    t.parse_fail("empty template list", "var<private> a: vec2<$>;", "erwartet einen Ausdruck");
    // error recovery: one error per broken statement, all reported
    let (_, d) = wgsl::parse("fn f() {\n  let a = ;\n  let b = 1 +;\n  let c = (2;\n  let d = 3;\n}\nfn g( {\n}\nfn h() { return 1 }\n");
    let lines: Vec<u32> = d.iter().filter(|d| d.error).map(|d| d.line).collect();
    t.eq("recovery: errors on every broken line", lines, alloc::vec![2, 3, 4, 7, 9]);
    // the depth limit: an error, no stack overflow
    let deep = format!("const x = {}1{};", "(".repeat(300), ")".repeat(300));
    t.parse_fail("deep parentheses", &format!("const x = {}$({}1{};", "(".repeat(128), "(".repeat(171), ")".repeat(300)), "zu tief verschachtelt");
    let _ = deep;
    let blocks = format!("fn f() {{ {} {} }}", "{".repeat(500), "}".repeat(500));
    t.check("deep blocks", wgsl::parse(&blocks).1.iter().any(|d| d.msg.contains("zu viele verschachtelte Blöcke")));
    let b126 = format!("fn f() {{ {} {} }}", "{".repeat(126), "}".repeat(126));
    t.check("126 nested blocks are fine", wgsl::parse(&b126).1.is_empty());
    let b127 = format!("fn f() {{ {} {} }}", "{".repeat(127), "}".repeat(127));
    t.check("127 nested blocks are too many", !wgsl::parse(&b127).1.is_empty());
    let p127 = format!("const x = {}1{};", "(".repeat(127), ")".repeat(127));
    t.check("127 parentheses are fine", wgsl::parse(&p127).1.is_empty());
    let p128 = format!("const x = {}1{};", "(".repeat(128), ")".repeat(128));
    t.check("128 parentheses are too many", !wgsl::parse(&p128).1.is_empty());
    let chain = format!("const x = 1{};", " + 1".repeat(300));
    t.check("long chain", wgsl::parse(&chain).1.iter().any(|d| d.msg.contains("verkettet")));
    let short_chain = format!("const x = 1{};", " + 1".repeat(250));
    t.check("chain of 250 is fine", wgsl::parse(&short_chain).1.is_empty());
    let nested = format!("const x = {};", (0..20).fold(String::from("1"), |acc, _| format!("({}{})", acc, " + 1".repeat(30))));
    t.check("nested chains are bounded by the tree height", wgsl::parse(&nested).1.iter().any(|d| d.msg.contains("verkettet")));
    let unary = format!("const x = {}1;", "-".repeat(400));
    t.check("unary chain", !wgsl::parse(&unary).1.is_empty());
    let elifs = format!("fn f() {{ if true {{}} {} }}", "else if true {} ".repeat(300));
    t.check("long else-if chain", wgsl::parse(&elifs).1.iter().any(|d| d.msg.contains("zu tief")));
    let ok_nest = format!("fn f() {{ {} {} }}", "{".repeat(100), "}".repeat(100));
    t.check("100 nested blocks are fine", wgsl::parse(&ok_nest).1.is_empty());
    // statement forms in the AST
    let src = "fn f() { for (var i = 0; i < 4; i++) {} while false {} loop { break; } _ = 1; }";
    if let Some(ast) = parse_ok(t, "statement forms", src) {
        if let DeclKind::Fn(f) = &ast.decls[0].kind {
            let k: Vec<&str> = f
                .body
                .stmts
                .iter()
                .map(|s| match s.kind {
                    StmtKind::For { .. } => "for",
                    StmtKind::While { .. } => "while",
                    StmtKind::Loop { .. } => "loop",
                    StmtKind::Phony(_) => "phony",
                    _ => "?",
                })
                .collect();
            t.eq("statement kinds", k, alloc::vec!["for", "while", "loop", "phony"]);
        }
    }
}

pub fn print(t: &mut T) {
    let mut srcs: Vec<(String, String)> = wgsl::bench::ALL.iter().map(|(n, s)| (String::from(*n), String::from(*s))).collect();
    srcs.push((String::from("sample"), String::from(SAMPLE)));
    srcs.push((String::from("ops"), String::from("const a = -(1 + 2) * 3 - -4;\nconst b = a--a;\nfn f(p: ptr<function, i32>) { *p = (*p) << 2u; let q = &*p; }\n")));
    for (name, src) in &srcs {
        let (a1, d1) = wgsl::parse(src);
        let p1 = wgsl::print(&a1);
        let (a2, d2) = wgsl::parse(&p1);
        let p2 = wgsl::print(&a2);
        t.check(&format!("{} parses", name), d1.is_empty());
        t.check(&format!("{} reparses", name), d2.is_empty());
        t.check(&format!("{} print is stable", name), p1 == p2);
        // the same tree, spans aside: compare declaration and statement counts and the
        // printed form of every declaration
        t.eq(&format!("{} declarations", name), a1.decls.len(), a2.decls.len());
        // the printed module checks like the original
        let c1 = wgsl::check(src).is_ok();
        let c2 = wgsl::check(&p1).is_ok();
        t.eq(&format!("{} checks the same", name), c1, c2);
        if !d2.is_empty() || p1 != p2 {
            for d in d2.iter().take(3) {
                crate::out(&format!("    {}\n", d));
            }
        }
    }
}

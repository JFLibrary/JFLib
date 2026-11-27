// funcs -- the functions (FX-04, FX-06 .. FX-11, FX-14, FX-15)
//
// Every function gets its argument nodes unevaluated: IF, AND, OR evaluate what they need,
// GetRef, SETATREF, SETF and SHAPETEXT look at the reference itself, DEPENDSON only names
// cells.  Errors in arguments propagate unless the function is one of the ISERR family.
//
// Functions with side effects (SETF, OPENTEXTWIN, DOCMD, RUNADDON, GOTOPAGE ...) compute 0 when
// a formula is recalculated; their effect happens only when the engine runs the cell as an
// action (Engine::run_action), and only for what needs no foreign code: macros, CALLTHIS and
// add-ons other than the app's own are reported, never run (FX-15).

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use super::color::{self, Color};
use super::engine::Effect;
use super::eval::{target_of, Ev, Keep, R};
use super::math;
use super::parse::{Func, Node};
use super::val::{Target, Val};
use crate::cell::CellError;

macro_rules! need {
    ($n:expr, $lo:expr, $hi:expr) => {
        if $n < $lo || $n > $hi {
            return Err(CellError::Value);
        }
    };
}

pub(crate) fn call(ev: &mut Ev, f: Func, start: u32, n: u32, sheet: u32) -> R {
    let a: Vec<u32> = ev.args(start, n).to_vec();
    let n = a.len();
    let num = |ev: &mut Ev, i: usize| -> Result<f64, CellError> {
        match a.get(i) {
            Some(&x) => ev.num(x),
            None => Err(CellError::Value),
        }
    };
    let opt_num = |ev: &mut Ev, i: usize, d: f64| -> Result<f64, CellError> {
        match a.get(i) {
            Some(&x) if ev.node(x) != Node::Missing => ev.num(x),
            _ => Ok(d),
        }
    };
    let n1 = |ev: &mut Ev, g: fn(f64) -> f64| -> R {
        need!(n, 1, 1);
        let x = num(ev, 0)?;
        finite(g(x))
    };
    use Func::*;
    match f {
        // ---- logic
        IfError => {
            need!(n, 2, 2);
            match ev.eval(a[0]) {
                Ok(Val::Err(_)) | Err(_) => ev.eval(a[1]),
                ok => ok,
            }
        }
        If => {
            need!(n, 2, 3);
            if ev.truth(a[0])? {
                ev.eval(a[1])
            } else if n == 3 {
                ev.eval(a[2])
            } else {
                Ok(Val::bool(false))
            }
        }
        And => {
            let mut r = true;
            for &x in &a {
                r &= ev.truth(x)?;
            }
            Ok(Val::bool(r))
        }
        Or => {
            let mut r = false;
            for &x in &a {
                r |= ev.truth(x)?;
            }
            Ok(Val::bool(r))
        }
        Not => {
            need!(n, 1, 1);
            Ok(Val::bool(!ev.truth(a[0])?))
        }
        True => Ok(Val::bool(true)),
        False => Ok(Val::bool(false)),

        // ---- math
        Abs => n1(ev, f64::abs),
        Sqrt => {
            need!(n, 1, 1);
            let x = num(ev, 0)?;
            if x < 0.0 { Err(CellError::Num) } else { Ok(Val::Num(math::sqrt(x))) }
        }
        Pow => {
            need!(n, 2, 2);
            let (x, y) = (num(ev, 0)?, num(ev, 1)?);
            finite(math::pow(x, y))
        }
        Exp => n1(ev, math::exp),
        Ln => {
            need!(n, 1, 1);
            let x = num(ev, 0)?;
            if x <= 0.0 { Err(CellError::Num) } else { Ok(Val::Num(math::ln(x))) }
        }
        Log10 => {
            need!(n, 1, 1);
            let x = num(ev, 0)?;
            if x <= 0.0 { Err(CellError::Num) } else { Ok(Val::Num(math::log10(x))) }
        }
        Sin => n1(ev, math::sin),
        Cos => n1(ev, math::cos),
        Tan => n1(ev, math::tan),
        Asin => n1(ev, math::asin),
        Acos => n1(ev, math::acos),
        Atan => n1(ev, math::atan),
        Sinh => n1(ev, math::sinh),
        Cosh => n1(ev, math::cosh),
        Tanh => n1(ev, math::tanh),
        Atan2 => {
            need!(n, 2, 2);
            let (y, x) = (num(ev, 0)?, num(ev, 1)?);
            Ok(Val::Num(math::atan2(y, x)))
        }
        Pi => Ok(Val::Num(core::f64::consts::PI)),
        Min | Max | Sum => {
            if n == 0 {
                return Err(CellError::Value);
            }
            let mut acc: Option<f64> = None;
            for &x in &a {
                let v = ev.eval(x)?;
                if matches!(v, Val::Missing) {
                    continue;
                }
                let x = v.num(ev.axis)?;
                acc = Some(match (f, acc) {
                    (_, None) => x,
                    (Min, Some(m)) => m.min(x),
                    (Max, Some(m)) => m.max(x),
                    (_, Some(s)) => s + x,
                });
            }
            Ok(Val::Num(acc.unwrap_or(0.0)))
        }
        Round => {
            need!(n, 1, 2);
            let x = num(ev, 0)?;
            let d = opt_num(ev, 1, 0.0)?;
            let s = math::pow(10.0, math::trunc(d));
            finite(math::round(x * s) / s)
        }
        Trunc => {
            need!(n, 1, 2);
            let x = num(ev, 0)?;
            let d = opt_num(ev, 1, 0.0)?;
            let s = math::pow(10.0, math::trunc(d));
            finite(math::trunc(x * s) / s)
        }
        Int => n1(ev, math::floor),
        IntUp => n1(ev, math::ceil),
        Floor | Ceiling => {
            // away from zero (CEILING) or towards zero (FLOOR) to a multiple of m (default 1
            // with the sign of x); 0 gives 0, signs that differ #NUM! ([MS-VSDX] 2.5.3.18, .60)
            need!(n, 1, 2);
            let x = num(ev, 0)?;
            let m = opt_num(ev, 1, if x < 0.0 { -1.0 } else { 1.0 })?;
            if x == 0.0 || m == 0.0 {
                return Ok(Val::Num(0.0));
            }
            if (x < 0.0) != (m < 0.0) {
                return Err(CellError::Num);
            }
            let q = x / m;
            // a little slack for noise in the quotient
            let q = if (q - math::round(q)).abs() < 1e-9 { math::round(q) } else { q };
            let r = if f == Ceiling { math::ceil(q) } else { math::trunc(q) };
            finite(r * m)
        }
        Modulus => {
            need!(n, 2, 2);
            let (x, y) = (num(ev, 0)?, num(ev, 1)?);
            if y == 0.0 {
                return Err(CellError::DivZero);
            }
            Ok(Val::Num(math::modulus(x, y)))
        }
        Sign => {
            need!(n, 1, 2);
            let x = num(ev, 0)?;
            let fuzz = opt_num(ev, 1, 0.0)?.abs();
            Ok(Val::Num(if x.abs() <= fuzz { 0.0 } else if x > 0.0 { 1.0 } else { -1.0 }))
        }
        Magnitude => {
            need!(n, 4, 4);
            let (ca, x, cb, y) = (num(ev, 0)?, num(ev, 1)?, num(ev, 2)?, num(ev, 3)?);
            Ok(Val::Num(math::sqrt((ca * x) * (ca * x) + (cb * y) * (cb * y))))
        }
        // (JD21) INTERSECTX/Y(x1, y1, angle1, x2, y2, angle2): where the line through (x1, y1) at
        // angle1 meets the one through (x2, y2) at angle2 ([MS-VSDX] 2.5.3.72, .73); parallel
        // lines meet nowhere (#NUM!)
        IntersectX | IntersectY => {
            need!(n, 6, 6);
            let (x1, y1, a1) = (num(ev, 0)?, num(ev, 1)?, num(ev, 2)?);
            let (x2, y2, a2) = (num(ev, 3)?, num(ev, 4)?, num(ev, 5)?);
            let ((s1, c1), (s2, c2)) = (math::sincos(a1), math::sincos(a2));
            let cross = c1 * s2 - s1 * c2;
            if cross.abs() < 1e-12 {
                return Err(CellError::Num);
            }
            // p1 + t d1 = p2 + u d2 -> t = ((p2 - p1) x d2) / (d1 x d2)
            let t = ((x2 - x1) * s2 - (y2 - y1) * c2) / cross;
            finite(if f == IntersectX { x1 + t * c1 } else { y1 + t * s1 })
        }
        Ang360 => n1(ev, |x| math::modulus(x, core::f64::consts::TAU)),
        Deg => n1(ev, |x| x * 180.0 / core::f64::consts::PI),
        Rad => n1(ev, |x| x),
        BitAnd | BitOr | BitXor => {
            need!(n, 2, 2);
            let (x, y) = (int(num(ev, 0)?), int(num(ev, 1)?));
            Ok(Val::Num(match f {
                BitAnd => x & y,
                BitOr => x | y,
                _ => x ^ y,
            } as f64))
        }
        BitNot => {
            need!(n, 1, 1);
            Ok(Val::Num(!int(num(ev, 0)?) as f64))
        }
        Index => {
            need!(n, 2, 4);
            let i = num(ev, 0)?;
            let list = ev.text(a[1])?;
            let delim = if n > 2 && ev.node(a[2]) != Node::Missing { ev.text(a[2])? } else { String::from(";") };
            let i = math::trunc(i);
            let item = if i < 0.0 || delim.is_empty() { None } else { list.split(delim.as_str()).nth(i as usize) };
            match item {
                Some(s) => Ok(Val::str(s)),
                None if n > 3 => ev.eval(a[3]),
                None => Ok(Val::str("")),
            }
        }
        Lookup => {
            need!(n, 2, 3);
            let key = ev.text(a[0])?;
            let list = ev.text(a[1])?;
            let delim = if n > 2 { ev.text(a[2])? } else { String::from(";") };
            if delim.is_empty() {
                return Ok(Val::Num(-1.0));
            }
            let pos = list.split(delim.as_str()).position(|s| s.eq_ignore_ascii_case(&key));
            Ok(Val::Num(pos.map_or(-1.0, |p| p as f64)))
        }
        // (JD15, FX-06) 0 <= x < 1 from the engine's generator, anew each time the cell is
        // computed
        Rand => {
            if n != 0 {
                return Err(CellError::Value);
            }
            Ok(Val::Num(ev.run.random()))
        }

        // ---- geometry
        Pnt | PntX | PntY | Par | Loc | Nurbs | Polyline | Bound | AngleToLoc | AngleToPar | LocToLoc | LocToPar => {
            super::geom::call(ev, f, &a)
        }

        // ---- colour
        Rgb => {
            // truncated, past 255 the low byte ([MS-VSDX] 2.5.3.130)
            need!(n, 3, 3);
            let c = |x: f64| (math::trunc(x).clamp(0.0, 1e9) as i64 & 0xff) as u8;
            Ok(Val::Color(Color::Rgb(c(num(ev, 0)?), c(num(ev, 1)?), c(num(ev, 2)?))))
        }
        Hsl => {
            // the arguments lose their fractions (HSL(.., .., 239.99) is 239), past 255 the
            // low byte, a hue of 241..255 is 240 ([MS-VSDX] 2.5.3.65)
            need!(n, 3, 3);
            let c = |x: f64| (math::trunc(x).clamp(0.0, 1e9) as i64 & 0xff) as i32;
            let (h, s, l) = (c(num(ev, 0)?), c(num(ev, 1)?), c(num(ev, 2)?));
            let (r, g, b) = color::hsl_to_rgb(h.min(240), s, l);
            Ok(Val::Color(Color::Rgb(r, g, b)))
        }
        Red | Green | Blue | Hue | Sat | Lum => {
            need!(n, 1, 1);
            let v = ev.eval(a[0])?;
            let (r, g, b) = ev.rgb(&v)?;
            let (h, s, l) = color::rgb_to_hsl(r, g, b);
            Ok(Val::Num(match f {
                Red => r as f64,
                Green => g as f64,
                Blue => b as f64,
                Hue => h as f64,
                Sat => s as f64,
                _ => l as f64,
            }))
        }
        HueDiff | SatDiff | LumDiff => {
            need!(n, 2, 2);
            let (x, y) = (ev.eval(a[0])?, ev.eval(a[1])?);
            let (p, q) = (ev.rgb(&x)?, ev.rgb(&y)?);
            let (h1, s1, l1) = color::rgb_to_hsl(p.0, p.1, p.2);
            let (h2, s2, l2) = color::rgb_to_hsl(q.0, q.1, q.2);
            Ok(Val::Num(match f {
                HueDiff => (h1 - h2) as f64,
                SatDiff => (s1 - s2) as f64,
                _ => (l1 - l2) as f64,
            }))
        }
        Tint | Shade | Tone | MsoTint | MsoShade => {
            need!(n, 2, 2);
            let v = ev.eval(a[0])?;
            let c = ev.rgb(&v)?;
            let amount = num(ev, 1)?;
            let (r, g, b) = match f {
                Tint => color::tint(c, amount),
                Shade => color::shade(c, amount),
                Tone => color::tone(c, amount),
                MsoTint => color::mso_tint(c, amount),
                _ => color::mso_shade(c, amount),
            };
            Ok(Val::Color(Color::Rgb(r, g, b)))
        }
        Blend => {
            need!(n, 3, 3);
            let (x, y) = (ev.eval(a[0])?, ev.eval(a[1])?);
            let (p, q) = (ev.rgb(&x)?, ev.rgb(&y)?);
            let t = num(ev, 2)?;
            if !(0.0..=1.0).contains(&t) {
                return Err(CellError::Value);
            }
            let (r, g, b) = color::blend(p, q, t);
            Ok(Val::Color(Color::Rgb(r, g, b)))
        }
        ThemeVal | ThemeProp | ThemeRestore | IsThemed | CellIsThemed => super::theme::call(ev, f, &a),
        ThemeGuard | Guard => {
            need!(n, 1, 1);
            ev.eval(a[0])
        }

        // ---- text
        ShapeText => {
            need!(n, 1, 2);
            let t = super::geom::shape_text(ev, a[0])?;
            Ok(Val::Str(Rc::from(t)))
        }
        TextWidth | TextHeight => super::geom::text_size(ev, f, &a),
        Font => {
            need!(n, 1, 1);
            let name = ev.text(a[0])?;
            if is_font_cell(ev.doc.str(ev.cell)) {
                Ok(Val::Font(Rc::from(name)))
            } else {
                // the font's ID in Visio's font table depends on the machine
                Err(ev.keep(Keep::NotComputed("FONT")))
            }
        }
        StrSame | StrSameEx => {
            need!(n, 2, 4);
            let (x, y) = (ev.text(a[0])?, ev.text(a[1])?);
            let ignore = n > 2 && ev.node(a[2]) != Node::Missing && ev.truth(a[2])?;
            Ok(Val::bool(if ignore { x.to_lowercase() == y.to_lowercase() } else { x == y }))
        }
        Len => {
            need!(n, 1, 1);
            Ok(Val::Num(ev.text(a[0])?.chars().count() as f64))
        }
        Left | Right => {
            need!(n, 1, 2);
            let s = ev.text(a[0])?;
            let k = opt_num(ev, 1, 1.0)?.max(0.0) as usize;
            let chars: Vec<char> = s.chars().collect();
            let k = k.min(chars.len());
            let part: String = if f == Left { chars[..k].iter().collect() } else { chars[chars.len() - k..].iter().collect() };
            Ok(Val::Str(Rc::from(part)))
        }
        Mid => {
            need!(n, 3, 3);
            let s = ev.text(a[0])?;
            let (from, k) = (num(ev, 1)?, num(ev, 2)?);
            if from < 1.0 || k < 0.0 {
                return Err(CellError::Value);
            }
            let part: String = s.chars().skip(from as usize - 1).take(k as usize).collect();
            Ok(Val::Str(Rc::from(part)))
        }
        Find => {
            need!(n, 2, 4);
            let (needle, hay) = (ev.text(a[0])?, ev.text(a[1])?);
            let from = opt_num(ev, 2, 1.0)?.max(1.0) as usize - 1;
            let ignore = n > 3 && ev.truth(a[3])?;
            let hay_c: Vec<char> = hay.chars().collect();
            let need_c: Vec<char> = needle.chars().collect();
            let eq = |x: char, y: char| if ignore { x.to_lowercase().eq(y.to_lowercase()) } else { x == y };
            let pos = (from..=hay_c.len().saturating_sub(need_c.len()))
                .find(|&i| i + need_c.len() <= hay_c.len() && need_c.iter().enumerate().all(|(j, &c)| eq(hay_c[i + j], c)));
            match pos {
                Some(p) => Ok(Val::Num((p + 1) as f64)),
                None => Err(CellError::Value),
            }
        }
        Substitute => {
            need!(n, 3, 4);
            let (s, old, new) = (ev.text(a[0])?, ev.text(a[1])?, ev.text(a[2])?);
            if old.is_empty() {
                return Ok(Val::Str(Rc::from(s)));
            }
            if n == 4 {
                let which = num(ev, 3)? as usize;
                let mut out = String::new();
                let mut rest = s.as_str();
                let mut k = 0;
                while let Some(p) = rest.find(old.as_str()) {
                    k += 1;
                    out.push_str(&rest[..p]);
                    out.push_str(if k == which { &new } else { &old });
                    rest = &rest[p + old.len()..];
                }
                out.push_str(rest);
                return Ok(Val::Str(Rc::from(out)));
            }
            Ok(Val::Str(Rc::from(s.replace(old.as_str(), &new))))
        }
        Replace => {
            need!(n, 4, 4);
            let s = ev.text(a[0])?;
            let (from, k) = (num(ev, 1)?, num(ev, 2)?);
            let new = ev.text(a[3])?;
            if from < 1.0 || k < 0.0 {
                return Err(CellError::Value);
            }
            let chars: Vec<char> = s.chars().collect();
            let p = (from as usize - 1).min(chars.len());
            let q = (p + k as usize).min(chars.len());
            let mut out: String = chars[..p].iter().collect();
            out.push_str(&new);
            out.extend(chars[q..].iter());
            Ok(Val::Str(Rc::from(out)))
        }
        Rept => {
            need!(n, 2, 2);
            let s = ev.text(a[0])?;
            let k = num(ev, 1)?;
            if !(0.0..=65536.0).contains(&k) || s.len() * k as usize > 1 << 20 {
                return Err(CellError::Value);
            }
            Ok(Val::Str(Rc::from(s.repeat(k as usize))))
        }
        Lower | Upper | Trim => {
            need!(n, 1, 1);
            let s = ev.text(a[0])?;
            Ok(Val::Str(Rc::from(match f {
                Lower => s.to_lowercase(),
                Upper => s.to_uppercase(),
                _ => s.split_whitespace().collect::<Vec<_>>().join(" "),
            })))
        }
        Char => {
            need!(n, 1, 1);
            let c = num(ev, 0)?;
            match char::from_u32(c as u32) {
                Some(ch) if c >= 0.0 => {
                    let mut s = String::new();
                    s.push(ch);
                    Ok(Val::Str(Rc::from(s)))
                }
                _ => Err(CellError::Value),
            }
        }
        // (JD19) FORMAT(value, picture), FORMATEX(value, picture, lang in, lang out): the
        // picture of field.rs; a value that may carry a length, angle or duration is kept (its
        // unit is not known here)
        Format | FormatEx => {
            need!(n, 2, if f == Format { 2 } else { 4 });
            format_value(ev, a[0], a[1], f)
        }
        EvalText => Err(ev.keep(Keep::NotComputed(f.name().unwrap_or("")))),

        // ---- document and shape information
        MasterName | PageName | Name | TypeDesc | Data1 | Data2 | Data3 | Id | Type | Is1D | PageNumber | PageCount => {
            super::geom::info(ev, f, &a)
        }
        Now | DocCreation | DocLastSave | DocLastPrint | DocLastEdit | Creator | Description | Directory | FileName | Keywords
        | Subject | Title | Manager | Company | Category | HyperlinkBase => {
            if n > 1 {
                return Err(CellError::Value);
            }
            doc_info(ev, f)
        }
        UserUi => {
            need!(n, 2, 3);
            // without a user interface state the default applies
            ev.eval(a[1])
        }
        Ref => Err(CellError::Ref),
        Na => Err(CellError::NotAvailable),
        Arg => {
            need!(n, 1, 2);
            // outside EVALCELL no argument is given: the default, else 0
            if n == 2 { ev.eval(a[1]) } else { Ok(Val::Num(0.0)) }
        }
        IsErr | IsError | IsErrNa | IsErrValue => {
            need!(n, 1, 1);
            let e = match ev.eval(a[0]) {
                Ok(v) => v.err(),
                Err(e) => Some(e),
            };
            Ok(Val::bool(match f {
                IsErr => e.is_some_and(|e| e != CellError::NotAvailable),
                IsError => e.is_some(),
                IsErrNa => e == Some(CellError::NotAvailable),
                _ => e == Some(CellError::Value),
            }))
        }
        DefaultEvent => Ok(Val::Num(0.0)),

        // ---- protection and side effects
        GetRef => {
            need!(n, 1, 1);
            Ok(Val::Target(target_of(ev, a[0])?))
        }
        SheetRef => {
            let s = if sheet == u32::MAX {
                ev.sheet
            } else {
                let r = ev.p.refs[sheet as usize].clone();
                ev.target(&r)?.0
            };
            Ok(Val::Target(Target { sheet: s, addr: None }))
        }
        DependsOn => {
            // names cells to recalculate with; evaluating them records the dependency
            for &x in &a {
                let _ = ev.eval(x);
            }
            Ok(Val::Num(0.0))
        }
        SetAtRef => {
            need!(n, 1, 3);
            let ignore = n > 2 && ev.node(a[2]) != Node::Missing && ev.truth(a[2])?;
            if ignore {
                return Ok(Val::Num(0.0));
            }
            ev.eval(a[0])
        }
        SetAtRefExpr => {
            if n > 1 {
                return Err(CellError::Value);
            }
            match ev.run.set_value.clone() {
                Some(v) => Ok(v),
                None if n == 1 => ev.eval(a[0]),
                None => Ok(Val::Num(0.0)),
            }
        }
        SetAtRefEval => {
            need!(n, 1, 1);
            ev.eval(a[0])
        }
        SetF => {
            need!(n, 2, 2);
            let t = target_of(ev, a[0])?;
            if ev.run.effects.is_some() {
                // the new formula: a string is the formula text, anything else its value
                let v = ev.eval(a[1])?;
                let text = match v.plain() {
                    Val::Str(s) => String::from(&**s),
                    other => formula_of(other)?,
                };
                ev.run.push_effect(Effect::SetFormula(t, text));
            }
            Ok(Val::Num(0.0))
        }

        // ---- actions
        OpenTextWin | OpenSheetWin | OpenGroupWin => {
            let e = match f {
                OpenTextWin => Effect::OpenTextWindow(ev.sheet),
                OpenSheetWin => Effect::OpenShapeSheet(ev.sheet),
                _ => Effect::OpenGroup(ev.sheet),
            };
            ev.run.push_effect(e);
            Ok(Val::Num(0.0))
        }
        GotoPage => {
            need!(n, 1, 1);
            if ev.run.effects.is_some() {
                let s = ev.text(a[0])?;
                ev.run.push_effect(Effect::GotoPage(s));
            }
            Ok(Val::Num(0.0))
        }
        Hyperlink => {
            need!(n, 1, 6);
            if ev.run.effects.is_some() {
                let addr = ev.text(a[0])?;
                let sub = if n > 1 { ev.text(a[1])? } else { String::new() };
                ev.run.push_effect(Effect::Hyperlink(addr, sub));
            }
            Ok(Val::Num(0.0))
        }
        PlaySound => {
            if ev.run.effects.is_some() {
                let s = if n > 0 { ev.text(a[0])? } else { String::new() };
                ev.run.push_effect(Effect::PlaySound(s));
            }
            Ok(Val::Num(0.0))
        }
        DoCmd => {
            need!(n, 1, 1);
            let c = num(ev, 0)?;
            ev.run.push_effect(Effect::Command(c as i64));
            Ok(Val::Num(0.0))
        }
        RunAddon | RunAddonWArgs => {
            if ev.run.effects.is_some() {
                let s = if n > 0 { ev.text(a[0])? } else { String::new() };
                ev.run.push_effect(Effect::AddOn(s));
            }
            Ok(Val::Num(0.0))
        }
        RunMacro | CallThis => {
            if ev.run.effects.is_some() {
                let s = if n > 0 { ev.text(a[0])? } else { String::new() };
                ev.run.push_effect(Effect::Refused(s));
            }
            Ok(Val::Num(0.0))
        }

        // ---- Visio's glue helpers
        XfTrigger => {
            // 1 for the own sheet's cell, 2 for another sheet's (the glued shape's EventXFMod);
            // reading it records the dependency on that shape's moves
            need!(n, 1, 1);
            let t = target_of(ev, a[0])?;
            if let Some(addr) = t.addr {
                let _ = super::engine::cell_value(ev.doc, ev.run, t.sheet, &addr);
            }
            Ok(Val::Num(if t.sheet == ev.sheet { 1.0 } else { 2.0 }))
        }
        // structure (FX-12)
        ContainerCount | ContainerSheetRef | ContainerMemberCount | ListMemberCount | ListOrder | ListSheetRef | CalloutCount
        | CalloutTargetRef | HasCategory => super::sfuncs::call(ev, f, &a),
        // where walking glue puts a connector end: the routing of JD7 computes it
        WalkGlue => Err(ev.keep(Keep::NotComputed("_WALKGLUE"))),
        Unknown(i) => {
            let name = String::from(&*ev.p.names[i as usize]);
            Err(ev.keep(Keep::Unknown(name)))
        }
    }
}

fn finite(x: f64) -> R {
    if x.is_finite() { Ok(Val::Num(x)) } else { Err(CellError::Num) }
}

fn int(x: f64) -> i64 {
    math::trunc(x).clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

// the cells that hold a font (FONT() gives the name there)
fn is_font_cell(cell: &str) -> bool {
    matches!(cell, "Font" | "AsianFont" | "ComplexScriptFont" | "BulletFont")
}

// a value as formula text (SETF with a non-string expression)
fn formula_of(v: &Val) -> Result<String, CellError> {
    Ok(match v {
        Val::Num(x) => super::val::fmt_num(*x),
        Val::Color(Color::Rgb(r, g, b)) => alloc::format!("RGB({r},{g},{b})"),
        Val::Color(Color::Index(i)) => alloc::format!("{i}"),
        Val::Point(x, y) => alloc::format!("PNT({},{})", super::val::fmt_num(*x), super::val::fmt_num(*y)),
        Val::Err(e) => return Err(*e),
        Val::Str(s) => {
            let mut t = String::from("\"");
            t.push_str(&s.replace('"', "\"\""));
            t.push('"');
            t
        }
        other => other.text()?,
    })
}

// (JD19) FORMAT and FORMATEX: a number, a date or a text in a format picture (field::format,
// the document's Env: decimal sign, month names).  Computed only when every cell the value
// reads is a shape data or user cell holding a plain number, a date, an amount of money or a
// text (or the value reads no cell): a length or angle shows in units the engine does not
// carry (Visio formats it in its cell's unit), so such a formula keeps its stored value.
fn format_value(ev: &mut Ev, value: u32, picture: u32, f: Func) -> R {
    let mut unit = None;
    if !plain_inputs(ev, value, &mut unit, 0) {
        return Err(ev.keep(Keep::NotComputed(f.name().unwrap_or(""))));
    }
    let pic = ev.text(picture)?;
    let v = ev.eval(value)?;
    let s = match v.plain() {
        Val::Num(x) => crate::field::format(&crate::field::Shown::Num(*x, unit), &pic, &ev.doc.fields),
        Val::Str(t) => crate::field::format(&crate::field::Shown::Text(t), &pic, &ev.doc.fields),
        Val::Missing => String::new(),
        Val::Err(e) => return Err(*e),
        other => {
            let t = other.text()?;
            crate::field::format(&crate::field::Shown::Text(&t), &pic, &ev.doc.fields)
        }
    };
    Ok(Val::Str(Rc::from(s)))
}

// whether the cells node i reads are shape data or user cells without a length, angle or
// duration; `unit` gets the unit of a single such cell (a date, an amount)
fn plain_inputs(ev: &mut Ev, i: u32, unit: &mut Option<crate::units::Unit>, depth: u32) -> bool {
    use crate::units::Dimension;
    if depth > 64 {
        return false;
    }
    match ev.node(i) {
        Node::Num(_) | Node::Str(_) | Node::Bool(_) | Node::Err(_) | Node::Missing => true,
        Node::Neg(a) | Node::Pct(a) => plain_inputs(ev, a, unit, depth + 1),
        Node::Bin(_, a, b) => plain_inputs(ev, a, unit, depth + 1) && plain_inputs(ev, b, unit, depth + 1),
        Node::Call(_, start, n, _) => {
            let args: Vec<u32> = ev.args(start, n).to_vec();
            args.into_iter().all(|x| plain_inputs(ev, x, unit, depth + 1))
        }
        Node::Ref(r) => {
            let p = ev.p.clone();
            let Some(cr) = p.refs.get(r as usize) else { return false };
            let super::parse::CellPart::Named { section, .. } = &cr.cell else { return false };
            if !matches!(*section, "Prop" | "Property" | "User") {
                return false;
            }
            let Ok((sheet, Some(addr))) = ev.target(cr) else { return false };
            let u = ev.doc.resolve(sheet, &addr).unit();
            match u.map(|u| u.dimension()) {
                None | Some(Dimension::Number | Dimension::Date | Dimension::Typed) => {
                    if u.is_some() {
                        *unit = u;
                    }
                    true
                }
                _ => false,
            }
        }
    }
}

// the document information of text fields (field.rs): the app's clock and the file's path
// (Document::fields), the document's properties; what the engine cannot know here (no clock, a
// drawing not saved yet, no date in the properties) keeps the stored value
fn doc_info(ev: &mut Ev, f: Func) -> R {
    use Func::*;
    let doc = ev.doc;
    let env = &doc.fields;
    let known = |x: f64| x.is_finite().then_some(x);
    let date = |v: Option<f64>| -> Option<Val> { v.map(Val::Num) };
    let path = env.path.as_str();
    let cut = path.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let v = match f {
        Now => date(known(env.now)),
        DocCreation | DocLastSave | DocLastPrint | DocLastEdit => {
            let p = crate::field::props(doc);
            date(match f {
                DocCreation => p.created.or(known(env.created)),
                DocLastSave => p.modified.or(known(env.saved)),
                DocLastPrint => p.printed,
                _ => known(env.edited).or(p.modified).or(known(env.saved)),
            })
        }
        Directory => (!path.is_empty()).then(|| Val::str(&path[..cut])),
        FileName => (!path.is_empty()).then(|| Val::str(&path[cut..])),
        _ => {
            let p = crate::field::props(doc);
            Some(Val::str(match f {
                Creator => &p.creator,
                Description => &p.description,
                Keywords => &p.keywords,
                Subject => &p.subject,
                Title => &p.title,
                Manager => &p.manager,
                Company => &p.company,
                Category => &p.category,
                _ => &p.hyperlink_base,
            }))
        }
    };
    match v {
        Some(v) => Ok(v),
        None => Err(ev.keep(Keep::NotComputed(f.name().unwrap_or("")))),
    }
}

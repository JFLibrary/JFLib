// debug -- the debugger interface of the VM (no UI): breakpoints, steps, pauses, the call
// stack, locals, upvalues and globals, read-only expressions in a frame
//
//   vm.set_breakpoint(file, line, cond) -> Ok(id)   stops before the first statement of `line`
//                       (or of the next line that has one) in a file: Program::name or a
//                       module's path (normalised as import does; a module that is not loaded
//                       yet gets it when it is).  cond: a jfs expression evaluated in the frame
//                       there, the stop happens when it is true; one that fails stops too, with
//                       Pause::error.  A syntax error in cond is the Err
//   vm.remove_breakpoint(id) / clear_breakpoints() / breakpoints()   with each one's hits and
//                       the line it is bound to
//   vm.request_pause()  stop at the next statement (of the running invocation or the next one)
//   vm.step(Step::Into | Over | Out)   how the next run() goes on: to the next statement
//                       (into calls), to the next statement of this function or of a caller,
//                       back into the caller
//   vm.run(..) -> Status::Paused(Pause { reason, file, line, col, breakpoint, error })
//                       stopped before that statement; run() again goes on from there
//   vm.paused()         the pause the invocation waits in (None once it runs again)
//   vm.frames()         the calls, innermost first: function ("" top-level code), file, line,
//                       column
//   vm.locals(i) / upvalues(i) / frame_globals(i)   (name, value) of frame i (0: innermost):
//                       the locals visible there, the variables its closure captured, the
//                       globals of its file (the private _ ones too); globals_of(file)
//   vm.eval(i, expr)    a read-only expression in frame i: literals, names (locals, upvalues,
//                       globals, host values), operators, fields, indexes and len, str, type,
//                       int, float -- no other calls, no functions, no assignment; fuel and
//                       output stay as they are.  The value lives until the next run
//   vm.repr(v) / preview(v, max) / children(v)   for a variables view: the text as inside a
//                       list (strings quoted), at most max characters ("…" at the cut); the
//                       items of a list ("0", ..) or the entries of a map (key text, value)
//   check_expr(src)     whether src is one expression (a condition before it is set)
//   vm.debugging()      whether run() goes one instruction at a time now (below)
//
// Stops happen at statement starts (Program::line_table: the first statement of a line, a for
// loop's head each round, a while loop's condition each round) and, while stepping, at the
// first instruction after a return from the function the step began in (also when an
// exception leaves it).  Over and Out step over what a call does, breakpoints in it still
// stop.  A pause ends the step; a step also ends with its invocation.  Code that a native runs
// through Cx::call (a callback of list.map or list.sort) never stops: the native is in the
// middle of its work.
//
// Cost: without breakpoints, step or pause request (or after clear_breakpoints) the VM runs
// exactly as without a debugger: run() uses exec, whose instruction loop knows nothing of it.
// With one, run() uses exec_debug, which runs exec one instruction at a time and asks
// dbg_stop before each (a lookup of the statement start and of the breakpoints of its line):
// about four times slower, but nothing of it changes fuel, results, output or the collector,
// so a run that pauses and goes on equals one that does not.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::ast::{BinOp, Expr, ExprKind, Stmt, StmtKind, UnOp};
use crate::heap::{self, Obj, Value};
use crate::module::{norm_path, UnitState};
use crate::vm::{Exc, Host, Status, Vm};
use crate::{num, stdlib};

/// how run() goes on from a pause (Vm::step)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// to the next statement, into calls
    Into,
    /// to the next statement of this function (or of a caller after it returns)
    Over,
    /// back into the caller
    Out,
}

/// why the debugger stopped a run
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PauseReason {
    Breakpoint,
    Step,
    /// Vm::request_pause
    Pause,
}

/// where and why a run stopped (Status::Paused)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pause {
    pub reason: PauseReason,
    pub file: String,
    pub line: u32,
    pub col: u32,
    /// the breakpoint's id
    pub breakpoint: Option<u32>,
    /// the condition of the breakpoint failed: why (German)
    pub error: Option<String>,
}

/// a breakpoint as Vm::breakpoints shows it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Breakpoint {
    pub id: u32,
    /// normalised as import does
    pub file: String,
    /// the line it was set on
    pub line: u32,
    pub cond: Option<String>,
    /// how often it stopped
    pub hits: u32,
    /// the line it stops at in the loaded file (None: not loaded, or no statement from `line`
    /// on)
    pub at: Option<u32>,
}

/// a call on the stack (Vm::frames)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FrameInfo {
    /// "" for top-level code (the program's or a module's), "fn" for an anonymous function
    pub func: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone)]
struct Bp {
    id: u32,
    file: String,
    line: u32,
    cond_src: Option<String>,
    cond: Option<Arc<Expr>>,
    hits: u32,
}

/// the debugger's state in a VM
#[derive(Clone, Default)]
pub(crate) struct Dbg {
    bps: Vec<Bp>,
    next_id: u32,
    /// (unit, line, index in bps), sorted: the breakpoints bound to loaded files
    bound: Vec<(u32, u32, usize)>,
    /// the step and the number of frames it began with
    step: Option<(Step, usize)>,
    pause_req: bool,
    /// the next check is the instruction a run paused at: it runs
    skip: bool,
    paused: Option<Pause>,
    /// exec checks every instruction
    pub active: bool,
}

impl Dbg {
    fn update(&mut self) {
        self.active = !self.bps.is_empty() || self.step.is_some() || self.pause_req;
    }

    /// run() starts
    pub(crate) fn run_starts(&mut self) {
        self.paused = None;
        if !self.active {
            self.skip = false;
        }
    }

    /// the invocation is over (done, failed, cancelled)
    pub(crate) fn invocation_ends(&mut self) {
        self.step = None;
        self.skip = false;
        self.paused = None;
        self.update();
    }
}

/// whether src is one expression, as a breakpoint's condition and Vm::eval take it; Err: why
/// not (German)
pub fn check_expr(src: &str) -> Result<(), String> {
    parse_expr(src).map(|_| ())
}

/// a jfs expression (a breakpoint's condition, Vm::eval)
fn parse_expr(src: &str) -> Result<Expr, String> {
    let (prog, d) = crate::parse_partial("", src);
    if let Some(x) = d.first() {
        return Err(x.msg.clone());
    }
    match prog.body.as_slice() {
        [Stmt { kind: StmtKind::Expr(e), .. }] => Ok(e.clone()),
        _ => Err(String::from("Erwartet wird genau ein Ausdruck")),
    }
}

/// the message of an exception of the VM
fn text(e: Exc) -> String {
    match e {
        Exc::Msg(m) | Exc::Fatal(_, m) => m,
        Exc::Throw(_) => String::from("Ausnahme"),
    }
}

fn no_calls() -> String {
    String::from("Im Ausdruck gehen keine Aufrufe außer len, str, type, int und float")
}

impl Vm {
    fn dbg_mut(&mut self) -> &mut Dbg {
        self.dbg.get_or_insert_with(|| Box::new(Dbg::default()))
    }

    // ---- breakpoints, pauses, steps

    /// see the module comment
    pub fn set_breakpoint(&mut self, file: &str, line: u32, cond: Option<&str>) -> Result<u32, String> {
        let cond = cond.map(str::trim).filter(|c| !c.is_empty());
        let ast = match cond {
            Some(c) => Some(Arc::new(parse_expr(c)?)),
            None => None,
        };
        let file = norm_path("", file);
        let d = self.dbg_mut();
        d.next_id += 1;
        let id = d.next_id;
        d.bps.push(Bp { id, file, line: line.max(1), cond_src: cond.map(String::from), cond: ast, hits: 0 });
        d.update();
        self.dbg_rebind();
        Ok(id)
    }

    /// false when there is no such breakpoint
    pub fn remove_breakpoint(&mut self, id: u32) -> bool {
        let Some(d) = self.dbg.as_deref_mut() else { return false };
        let n = d.bps.len();
        d.bps.retain(|b| b.id != id);
        d.update();
        let gone = d.bps.len() != n;
        self.dbg_rebind();
        gone
    }

    pub fn clear_breakpoints(&mut self) {
        if let Some(d) = self.dbg.as_deref_mut() {
            d.bps.clear();
            d.bound.clear();
            d.update();
        }
    }

    pub fn breakpoints(&self) -> Vec<Breakpoint> {
        let Some(d) = self.dbg.as_deref() else { return Vec::new() };
        d.bps
            .iter()
            .enumerate()
            .map(|(i, b)| Breakpoint {
                id: b.id,
                file: b.file.clone(),
                line: b.line,
                cond: b.cond_src.clone(),
                hits: b.hits,
                at: d.bound.iter().find(|x| x.2 == i).map(|x| x.1),
            })
            .collect()
    }

    /// stop at the next statement
    pub fn request_pause(&mut self) {
        let d = self.dbg_mut();
        d.pause_req = true;
        d.update();
    }

    /// how the next run() goes on (from a pause; without one Into stops at the next statement)
    pub fn step(&mut self, kind: Step) {
        let depth = self.frames.len();
        let d = self.dbg_mut();
        d.step = Some((kind, depth));
        d.update();
    }

    /// the pause the pending invocation waits in
    pub fn paused(&self) -> Option<&Pause> {
        self.dbg.as_deref().and_then(|d| d.paused.as_ref())
    }

    /// whether a breakpoint, a step or a pause request makes run() go one instruction at a
    /// time (exec_debug)
    pub fn debugging(&self) -> bool {
        self.dbg.as_deref().is_some_and(|d| d.active)
    }

    /// bind the breakpoints to the loaded files (after a change and an import)
    pub(crate) fn dbg_rebind(&mut self) {
        let Some(d) = self.dbg.as_deref_mut() else { return };
        let mut bound = Vec::new();
        for (bi, b) in d.bps.iter().enumerate() {
            for (u, unit) in self.units.iter().enumerate() {
                if unit.path != b.file || unit.state == UnitState::Failed {
                    continue;
                }
                let lines = self.code.protos.iter().filter(|p| p.unit == u as u32).flat_map(|p| p.stmts.iter().map(|s| s.1));
                if let Some(l) = lines.filter(|&l| l >= b.line).min() {
                    bound.push((u as u32, l, bi));
                }
            }
        }
        bound.sort_unstable();
        d.bound = bound;
    }

    /// run()'s exec while debugging: before every instruction of the invocation (not of what
    /// a native runs through Cx::call) the stops of dbg_stop, then the instruction alone
    pub(crate) fn exec_debug(&mut self, host: &mut dyn Host, fuel: u64) -> Result<Status, Exc> {
        loop {
            if self.used >= fuel {
                return Ok(Status::Yielded);
            }
            let fi = self.frames.len() - 1;
            let pc = self.frames[fi].pc as usize;
            if let Some(p) = self.dbg_stop(fi, pc) {
                return Ok(Status::Paused(p));
            }
            self.native_yield = false;
            match self.exec(host, self.used + 1)? {
                // (the next exec takes new code anyway)
                Status::Yielded if !self.native_yield => self.relink = false,
                s => return Ok(s),
            }
        }
    }

    /// before the instruction at pc of frame fi: the pause when it stops here
    fn dbg_stop(&mut self, fi: usize, pc: usize) -> Option<Pause> {
        let (unit, stmt, (line, col)) = {
            let p = &self.code.protos[self.frames[fi].proto as usize];
            (p.unit, p.stmt_line(pc as u32), p.pos.get(pc).copied().unwrap_or((0, 0)))
        };
        let depth = fi + 1;
        let d = self.dbg.as_deref_mut()?;
        if d.skip {
            d.skip = false;
            return None;
        }
        let mut reason = None;
        if let Some((kind, from)) = d.step {
            let at = stmt.is_some();
            if depth < from || (at && (kind == Step::Into || (kind == Step::Over && depth <= from))) {
                reason = Some(PauseReason::Step);
            }
        }
        if d.pause_req && stmt.is_some() {
            reason = Some(PauseReason::Pause);
        }
        let (mut hit, mut error) = (None, None);
        if let Some(line) = stmt
            && !d.bound.is_empty()
        {
            let i = d.bound.partition_point(|x| (x.0, x.1) < (unit, line));
            let cands: Vec<(usize, Option<Arc<Expr>>)> =
                d.bound[i..].iter().take_while(|x| (x.0, x.1) == (unit, line)).map(|x| (x.2, d.bps[x.2].cond.clone())).collect();
            for (bi, cond) in cands {
                match cond {
                    None => hit = Some(bi),
                    Some(e) => match self.eval_ast(fi, pc as u32, &e) {
                        Ok(v) if v.truthy() => hit = Some(bi),
                        Ok(_) => {}
                        Err(m) => {
                            hit = Some(bi);
                            error = Some(m);
                        }
                    },
                }
                if hit.is_some() {
                    break;
                }
            }
        }
        let d = self.dbg.as_deref_mut()?;
        let mut id = None;
        if let Some(bi) = hit {
            d.bps[bi].hits += 1;
            id = Some(d.bps[bi].id);
            reason = Some(PauseReason::Breakpoint);
        }
        let reason = reason?;
        let pause = Pause { reason, file: self.units[unit as usize].path.clone(), line: stmt.unwrap_or(line), col, breakpoint: id, error };
        d.paused = Some(pause.clone());
        d.skip = true;
        d.step = None;
        d.pause_req = false;
        d.update();
        Some(pause)
    }

    // ---- the stack

    /// (frame index into self.frames, the instruction it is at) of frame i (0: innermost)
    fn frame_at(&self, i: usize) -> Option<(usize, u32)> {
        let n = self.frames.len();
        let fi = n.checked_sub(i + 1)?;
        let f = self.frames[fi];
        let last = self.code.protos[f.proto as usize].pos.len().saturating_sub(1) as u32;
        let at = if fi + 1 == n { f.pc } else { f.pc.saturating_sub(1) };
        Some((fi, at.min(last)))
    }

    /// the calls of the pending invocation, innermost first
    pub fn frames(&self) -> Vec<FrameInfo> {
        (0..self.frames.len())
            .filter_map(|i| {
                let (fi, at) = self.frame_at(i)?;
                let p = &self.code.protos[self.frames[fi].proto as usize];
                let (mut line, col) = p.pos.get(at as usize).copied().unwrap_or((0, 0));
                if i == 0 {
                    line = p.stmt_line(at).unwrap_or(line);
                }
                Some(FrameInfo { func: p.name.clone(), file: self.units[p.unit as usize].path.clone(), line, col })
            })
            .collect()
    }

    fn slot_value(&self, i: usize) -> Value {
        match self.stack.get(i) {
            Some(Value::Cell(c)) => self.heap.cell(*c),
            Some(v) => *v,
            None => Value::Nil,
        }
    }

    /// the locals visible in frame i (an inner one hides an outer one of the same name)
    pub fn locals(&self, frame: usize) -> Vec<(String, Value)> {
        let Some((fi, at)) = self.frame_at(frame) else { return Vec::new() };
        let f = self.frames[fi];
        let p = &self.code.protos[f.proto as usize];
        let mut out: Vec<(String, Value)> = Vec::new();
        for v in p.vars.iter().filter(|v| v.from <= at && at < v.to) {
            let x = self.slot_value(f.base as usize + v.slot as usize);
            match out.iter_mut().find(|e| e.0 == v.name) {
                Some(e) => e.1 = x,
                None => out.push((v.name.clone(), x)),
            }
        }
        out
    }

    /// the variables the closure of frame i captured
    pub fn upvalues(&self, frame: usize) -> Vec<(String, Value)> {
        let Some((fi, _)) = self.frame_at(frame) else { return Vec::new() };
        let f = self.frames[fi];
        let Some(c) = f.closure else { return Vec::new() };
        let p = &self.code.protos[f.proto as usize];
        let ups = &self.heap.closure(c).ups;
        p.up_names.iter().zip(ups.iter()).map(|(n, &cell)| (n.clone(), self.heap.cell(cell))).collect()
    }

    /// the globals of the file of frame i
    pub fn frame_globals(&self, frame: usize) -> Vec<(String, Value)> {
        let Some((fi, _)) = self.frame_at(frame) else { return Vec::new() };
        let u = self.code.protos[self.frames[fi].proto as usize].unit as usize;
        self.unit_globals(u)
    }

    /// the globals of a loaded file (the program's name or a module's path)
    pub fn globals_of(&self, file: &str) -> Vec<(String, Value)> {
        let file = norm_path("", file);
        match self.units.iter().rposition(|u| u.path == file) {
            Some(u) => self.unit_globals(u),
            None => Vec::new(),
        }
    }

    fn unit_globals(&self, u: usize) -> Vec<(String, Value)> {
        let (b, n) = (self.units[u].gbase as usize, self.units[u].gcount as usize);
        (b..b + n).map(|i| (self.code.globals[i].clone(), self.globals[i])).collect()
    }

    // ---- values

    /// a value as inside a list: strings quoted
    pub fn repr(&self, v: Value) -> String {
        if !self.valid(v) {
            return String::from("<ungültig>");
        }
        let mut s = String::new();
        self.write_value(&mut s, v, true, 0, &mut Vec::new());
        s
    }

    /// repr() with at most max characters ("…" at the end when it is cut)
    pub fn preview(&self, v: Value, max: usize) -> String {
        let s = self.repr(v);
        if s.chars().count() <= max {
            return s;
        }
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }

    /// the items of a list ("0", "1" ..) or the entries of a map (the key as text: a str as it
    /// is, others as repr); nothing for other values
    pub fn children(&self, v: Value) -> Vec<(String, Value)> {
        if !self.valid(v) {
            return Vec::new();
        }
        match v {
            Value::List(r) => self.heap.list(r).iter().enumerate().map(|(i, x)| (i.to_string(), *x)).collect(),
            Value::Map(_) => self
                .map_entries(v)
                .into_iter()
                .map(|(k, x)| (self.str(k).map(String::from).unwrap_or_else(|| self.repr(k)), x))
                .collect(),
            _ => Vec::new(),
        }
    }

    // ---- expressions

    /// see the module comment
    pub fn eval(&mut self, frame: usize, expr: &str) -> Result<Value, String> {
        let e = parse_expr(expr)?;
        let Some((fi, at)) = self.frame_at(frame) else {
            return Err(String::from("Diesen Aufruf gibt es nicht"));
        };
        self.eval_ast(fi, at, &e)
    }

    /// an expression in frame fi at instruction `at`; fuel, the stack and what a native keeps
    /// stay as they were
    pub(crate) fn eval_ast(&mut self, fi: usize, at: u32, e: &Expr) -> Result<Value, String> {
        let (used, was, height, roots) = (self.used, self.in_native, self.stack.len(), self.nroots.len());
        // what it allocates stays alive (nroots) while it runs
        self.in_native = true;
        let r = self.ev(fi, at, e);
        self.used = used;
        self.in_native = was;
        self.stack.truncate(height);
        self.nroots.truncate(roots);
        r
    }

    fn lookup(&self, fi: usize, at: u32, name: &str) -> Result<Value, String> {
        let f = self.frames[fi];
        let p = &self.code.protos[f.proto as usize];
        if let Some(v) = p.vars.iter().rev().find(|v| v.name == name && v.from <= at && at < v.to) {
            return Ok(self.slot_value(f.base as usize + v.slot as usize));
        }
        if let Some(c) = f.closure
            && let Some(i) = p.up_names.iter().position(|n| n == name)
        {
            return Ok(self.heap.cell(self.heap.closure(c).ups[i]));
        }
        let u = &self.units[p.unit as usize];
        let (b, n) = (u.gbase as usize, u.gcount as usize);
        if let Some(i) = self.code.globals[b..b + n].iter().position(|g| g == name) {
            return Ok(self.globals[b + i]);
        }
        if let Some(i) = self.code.hosts.iter().position(|h| h.name == name)
            && let Some(v) = self.hostv.get(i).copied().flatten()
        {
            return Ok(v);
        }
        Err(format!("Unbekannter Name {}", name))
    }

    /// two values on the stack through an operator of the VM
    // The evaluator's operators follow vm.rs (arith, compare, get_index, builtin) with its
    // messages, but do not call them: a second caller changes how LLVM inlines them into exec,
    // and exec must stay as fast as without a debugger.

    /// x op y for + - * / % and the comparisons
    fn ev_op(&mut self, op: BinOp, x: Value, y: Value) -> Result<Value, String> {
        let num = |v: Value| match v {
            Value::Int(i) => num::from_i64(i),
            Value::Float(f) => f,
            _ => num::NAN,
        };
        let mismatch = |what: &str| format!("{} {} passt nicht zu {} und {}", what, op.text(), x.type_name(), y.type_name());
        Ok(match op {
            BinOp::Eq => Value::Bool(self.same(x, y)),
            BinOp::Ne => Value::Bool(!self.same(x, y)),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let ord = match (x, y) {
                    (Value::Int(a), Value::Int(b)) => Some(a.cmp(&b)),
                    (Value::Int(a), Value::Float(b)) => num::cmp_int(a, b),
                    (Value::Float(a), Value::Int(b)) => num::cmp_int(b, a).map(core::cmp::Ordering::reverse),
                    (Value::Float(a), Value::Float(b)) => num::cmp(a, b),
                    (Value::Str(_), Value::Str(_)) => Some(self.str(x).cmp(&self.str(y))),
                    _ => return Err(mismatch("Der Vergleich")),
                };
                Value::Bool(match (op, ord) {
                    (_, None) => false,
                    (BinOp::Lt, Some(o)) => o == core::cmp::Ordering::Less,
                    (BinOp::Le, Some(o)) => o != core::cmp::Ordering::Greater,
                    (BinOp::Gt, Some(o)) => o == core::cmp::Ordering::Greater,
                    (_, Some(o)) => o != core::cmp::Ordering::Less,
                })
            }
            _ => match (x, y) {
                (Value::Int(a), Value::Int(b)) => Value::Int(match op {
                    BinOp::Add => a.wrapping_add(b),
                    BinOp::Sub => a.wrapping_sub(b),
                    BinOp::Mul => a.wrapping_mul(b),
                    BinOp::Div | BinOp::Rem if b == 0 => return Err(String::from("Division durch 0")),
                    BinOp::Div => a.wrapping_div(b),
                    _ => a.wrapping_rem(b),
                }),
                (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => {
                    let (a, b) = (num(x), num(y));
                    Value::Float(match op {
                        BinOp::Add => num::add(a, b),
                        BinOp::Sub => num::sub(a, b),
                        BinOp::Mul => num::mul(a, b),
                        BinOp::Div => num::div(a, b),
                        _ => num::rem(a, b),
                    })
                }
                (Value::Str(_), Value::Str(_)) if op == BinOp::Add => {
                    let s = format!("{}{}", self.str(x).unwrap_or(""), self.str(y).unwrap_or(""));
                    stdlib::cap(self, s.len() as u64).map_err(text)?;
                    self.alloc_str(s).map_err(text)?
                }
                (Value::List(a), Value::List(b)) if op == BinOp::Add => {
                    let mut v = self.heap.list(a).clone();
                    v.extend_from_slice(self.heap.list(b));
                    Value::List(self.alloc(Obj::List(v)).map_err(text)?)
                }
                (Value::Bytes(a), Value::Bytes(b)) if op == BinOp::Add => {
                    let mut v = self.heap.bytes_of(a).to_vec();
                    v.extend_from_slice(self.heap.bytes_of(b));
                    Value::Bytes(self.alloc(Obj::Bytes(v.into_boxed_slice())).map_err(text)?)
                }
                _ => return Err(mismatch("Der Operator")),
            },
        })
    }

    /// obj[idx]
    fn ev_index(&mut self, obj: Value, idx: Value) -> Result<Value, String> {
        match (obj, idx) {
            (Value::List(l), Value::Int(i)) => {
                let items = self.heap.list(l);
                usize::try_from(i).ok().and_then(|i| items.get(i)).copied().ok_or_else(|| format!("Index {} liegt außerhalb der Liste (Länge {})", i, items.len()))
            }
            (Value::Map(_), k) => match self.heap.key(k) {
                Some(_) => Ok(self.map_get(obj, k).unwrap_or(Value::Nil)),
                None => Err(format!("Ein Map-Schlüssel muss str, int oder bool sein, nicht {}", k.type_name())),
            },
            (Value::Str(s), Value::Int(i)) => {
                let so = self.heap.str(s);
                match heap::char_at(so, i) {
                    Some(c) => self.alloc_str(String::from(c)).map_err(text),
                    None => Err(format!("Index {} liegt außerhalb des Textes (Länge {})", i, so.chars)),
                }
            }
            (Value::Bytes(b), Value::Int(i)) => {
                let bs = self.heap.bytes_of(b);
                usize::try_from(i).ok().and_then(|i| bs.get(i)).map(|x| Value::Int(*x as i64)).ok_or_else(|| format!("Index {} liegt außerhalb der Bytes (Länge {})", i, bs.len()))
            }
            (Value::List(_) | Value::Str(_) | Value::Bytes(_), i) => Err(format!("Ein Index von {} muss int sein, nicht {}", obj.type_name(), i.type_name())),
            _ => Err(format!("{} lässt sich nicht indizieren", obj.type_name())),
        }
    }

    /// len, str, type, int, float of one value
    fn ev_builtin(&mut self, name: &str, a: Value) -> Result<Value, String> {
        Ok(match name {
            "len" => Value::Int(match a {
                Value::Str(r) => self.heap.str(r).chars as i64,
                Value::Bytes(r) => self.heap.bytes_of(r).len() as i64,
                Value::List(r) => self.heap.list(r).len() as i64,
                Value::Map(r) => self.heap.map(r).live as i64,
                v => return Err(format!("len braucht str, bytes, list oder map, nicht {}", v.type_name())),
            }),
            "type" => self.alloc_str(String::from(a.type_name())).map_err(text)?,
            "str" => match a {
                Value::Str(_) => a,
                _ => {
                    let s = self.display(a);
                    self.alloc_str(s).map_err(text)?
                }
            },
            "int" => match a {
                Value::Int(_) => a,
                Value::Bool(b) => Value::Int(b as i64),
                Value::Float(f) => Value::Int(num::to_i64(f).ok_or_else(|| format!("int(…): {} passt nicht in int", self.display(a)))?),
                Value::Str(_) => {
                    let s = self.str(a).unwrap_or("");
                    Value::Int(stdlib::parse_int(s, None).ok_or_else(|| format!("int(…): \"{}\" ist keine ganze Zahl", s))?)
                }
                v => return Err(format!("int(…) braucht eine Zahl oder einen Text, nicht {}", v.type_name())),
            },
            _ => match a {
                Value::Float(_) => a,
                Value::Int(i) => Value::Float(num::from_i64(i)),
                Value::Str(_) => {
                    let s = self.str(a).unwrap_or("");
                    match s.trim().parse::<f64>() {
                        Ok(f) => Value::Float(if num::is_nan(f) { num::NAN } else { f }),
                        Err(_) => return Err(format!("float(…): \"{}\" ist keine Zahl", s)),
                    }
                }
                v => return Err(format!("float(…) braucht eine Zahl oder einen Text, nicht {}", v.type_name())),
            },
        })
    }

    fn ev(&mut self, fi: usize, at: u32, e: &Expr) -> Result<Value, String> {
        Ok(match &e.kind {
            ExprKind::Nil => Value::Nil,
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Int(i) => Value::Int(*i),
            ExprKind::Float(f) => Value::Float(*f),
            ExprKind::Str(s) => self.alloc_str(s.clone()).map_err(text)?,
            ExprKind::Interp(items) => {
                // (not through concat: exec's helpers stay with one caller, see vm.rs)
                let mut s = String::new();
                for x in items {
                    let v = self.ev(fi, at, x)?;
                    match v {
                        Value::Str(_) => s.push_str(self.str(v).unwrap_or("")),
                        _ => self.write_value(&mut s, v, false, 0, &mut Vec::new()),
                    }
                    stdlib::cap(self, s.len() as u64).map_err(text)?;
                }
                self.alloc_str(s).map_err(text)?
            }
            ExprKind::List(items) => {
                let mut v = Vec::with_capacity(items.len());
                for x in items {
                    v.push(self.ev(fi, at, x)?);
                }
                Value::List(self.alloc(Obj::List(v)).map_err(text)?)
            }
            ExprKind::Map(kv) => {
                let m = Value::Map(self.alloc(Obj::Map(Default::default())).map_err(text)?);
                for (k, v) in kv {
                    let k = self.ev(fi, at, k)?;
                    let v = self.ev(fi, at, v)?;
                    self.set_key(m, k, v).map_err(text)?;
                }
                m
            }
            ExprKind::Var(n) => self.lookup(fi, at, n)?,
            ExprKind::Unary(UnOp::Not, a) => Value::Bool(!self.ev(fi, at, a)?.truthy()),
            ExprKind::Unary(UnOp::Neg, a) => match self.ev(fi, at, a)? {
                Value::Int(i) => Value::Int(i.wrapping_neg()),
                Value::Float(f) => Value::Float(num::neg(f)),
                v => return Err(format!("Das Minus passt nicht zu {}", v.type_name())),
            },
            ExprKind::Binary(op, a, b) => {
                let x = self.ev(fi, at, a)?;
                match op {
                    BinOp::And => return if x.truthy() { self.ev(fi, at, b) } else { Ok(x) },
                    BinOp::Or => return if x.truthy() { Ok(x) } else { self.ev(fi, at, b) },
                    BinOp::Coalesce => return if x != Value::Nil { Ok(x) } else { self.ev(fi, at, b) },
                    _ => {}
                }
                let y = self.ev(fi, at, b)?;
                self.ev_op(*op, x, y)?
            }
            ExprKind::Field(obj, name) => {
                if let ExprKind::Var(base) = &obj.kind
                    && self.lookup(fi, at, base).is_err()
                    && let Some(c) = stdlib::constant(&format!("{}.{}", base, name.text))
                {
                    return Ok(c);
                }
                let v = self.ev(fi, at, obj)?;
                match v {
                    Value::Map(_) => self.map_get_str(v, &name.text).unwrap_or(Value::Nil),
                    _ => return Err(format!("{} hat kein Feld {}", v.type_name(), name.text)),
                }
            }
            ExprKind::Index(obj, i) => {
                let x = self.ev(fi, at, obj)?;
                let y = self.ev(fi, at, i)?;
                self.ev_index(x, y)?
            }
            ExprKind::Call(f, args) => {
                let ExprKind::Var(n) = &f.kind else { return Err(no_calls()) };
                if !matches!(n.as_str(), "len" | "str" | "type" | "int" | "float") || self.lookup(fi, at, n).is_ok() {
                    return Err(no_calls());
                }
                let mut first = Value::Nil;
                for (i, a) in args.iter().enumerate() {
                    let v = self.ev(fi, at, a)?;
                    if i == 0 {
                        first = v;
                    }
                }
                self.ev_builtin(n, first)?
            }
            ExprKind::Method(..) | ExprKind::Fn(_) => return Err(no_calls()),
        })
    }
}

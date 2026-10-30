// code -- the bytecode of compiled scripts: instructions, functions, the compiled Program
//
//   Op        one instruction of the stack machine (12 bytes); the comments give the stack
//             effect.  Slots count from the frame base: the parameters, then the locals of
//             the blocks, then the temporaries.  A local that a closure captures holds a
//             cell; closures keep the cells (Up::Local from the enclosing frame, Up::Up from
//             the enclosing closure).
//   Proto     a function: code, the line and column of every instruction, the exception
//             table (try ranges with their handler and stack height), the upvalue sources,
//             the most stack it needs; for the debugger the statements' first instructions
//             with their lines, the named locals with their slots and ranges, the upvalues'
//             names and the module (unit) it belongs to
//   Program   what compile() returns: functions (0 is the top-level code), constants, the
//             names of the globals and of the host names the script uses; cheap to clone
//             (shared), Send + Sync
//   disasm()  the code as text, for tests and debugging
//   line_table() / break_lines() / functions()   the line table: every instruction's
//             function, line and column (the file is Program::name) and whether a statement
//             starts there; the lines a breakpoint can stop at; the functions' names

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Op {
    /// +1: nil, true, false
    Nil,
    True,
    False,
    /// +1: a small integer
    Int(i32),
    /// +1: constant k (int, float or str)
    Const(u32),
    /// +n: n nils (the slots of a block)
    Reserve(u32),
    /// -1
    Pop,
    /// -n
    PopN(u32),
    /// +1: a copy of the top
    Dup,
    /// +2: a copy of the two top values
    Dup2,
    /// +1 / -1: a local slot
    GetLocal(u32),
    SetLocal(u32),
    /// +1 / -1: a local slot holding a cell
    GetCell(u32),
    SetCell(u32),
    /// 0: slot = a new cell with nil
    NewCell(u32),
    /// 0: slot = a new cell with the slot's value (captured parameters and loop variables)
    BoxLocal(u32),
    /// +1 / -1: an upvalue of the running closure
    GetUp(u32),
    SetUp(u32),
    /// +1 / -1: a global of the script
    GetGlobal(u32),
    SetGlobal(u32),
    /// +1: a host name: its value when the host set one, else a native function value
    GetHost(u32),
    /// +1: host entry e (a dotted name such as config.port): the field of the base's value
    /// when the host set one, else a native function value
    GetHostField(u32),
    /// -1: assignment to an undeclared name (an error)
    AssignHost(u32),
    /// +1: a closure of function p
    Closure(u32),
    /// -1: the binary operators
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// 0
    Neg,
    Not,
    /// -1 or jump keeping the value: && (falsy), || (truthy), ?? (not nil)
    And(u32),
    Or(u32),
    Coalesce(u32),
    /// 0
    Jump(u32),
    /// -1
    JumpIfFalse(u32),
    /// 1 - n: the texts of n values joined (interpolation)
    Concat(u32),
    /// 1 - n
    MakeList(u32),
    /// 1 - 2n: from key, value pairs
    MakeMap(u32),
    /// 0: obj -> obj.name (constant k)
    GetField(u32),
    /// -2: obj, value
    SetField(u32),
    /// -1: obj, index -> obj[index]
    GetIndex,
    /// -3: obj, index, value
    SetIndex,
    /// -argc: callee, args -> result
    Call(u32),
    /// 1 - argc: host entry e called with args
    CallHost(u32, u32),
    /// -argc: obj, args -> obj.name(args) (name: constant k)
    CallMethod(u32, u32),
    /// 1 - argc: host entry e (base.name) called with args
    CallHostMethod(u32, u32),
    /// -1 (ends the function)
    Return,
    /// -1
    Throw,
    /// 0: prepare the iteration over the value in slot b (slots b + 1, b + 2: state)
    IterPrep(u32),
    /// +1 / +2: the next item (key) / index and item (key and value) of the loop in slot b,
    /// or jump to the end
    IterNext(u32, u32),
    IterNext2(u32, u32),
    /// 0: check the bounds of a range in slots b and b + 1
    RangePrep(u32),
    /// +1: the next number of the range in slot b, or jump to the end
    RangeNext(u32, u32),
    /// -1: the Return at the end of a module's top-level code (module.rs puts it there when
    /// it links the module): import's call gets the module's map instead of the value
    EndModule,
}

impl Op {
    /// the change of the stack height (for jumps and conditional ops: when not jumping)
    pub fn effect(self) -> i32 {
        use Op::*;
        match self {
            Nil | True | False | Int(_) | Const(_) | Dup | GetLocal(_) | GetCell(_) | GetUp(_) | GetGlobal(_) | GetHost(_)
            | GetHostField(_) | Closure(_) | IterNext(..) | RangeNext(..) => 1,
            Dup2 | IterNext2(..) => 2,
            Reserve(n) => n as i32,
            PopN(n) => -(n as i32),
            Pop | SetLocal(_) | SetCell(_) | SetUp(_) | SetGlobal(_) | AssignHost(_) | JumpIfFalse(_) | Return | EndModule | Throw
            | Add | Sub | Mul | Div | Rem | Eq | Ne | Lt | Le | Gt | Ge | And(_) | Or(_) | Coalesce(_) | GetIndex => -1,
            NewCell(_) | BoxLocal(_) | Neg | Not | Jump(_) | GetField(_) | IterPrep(_) | RangePrep(_) => 0,
            Concat(n) | MakeList(n) => 1 - n as i32,
            MakeMap(n) => 1 - 2 * n as i32,
            SetField(_) => -2,
            SetIndex => -3,
            Call(a) | CallMethod(_, a) => -(a as i32),
            CallHost(_, a) | CallHostMethod(_, a) => 1 - a as i32,
        }
    }
}

/// a try range: instructions start..end are protected; on an exception the stack is cut to
/// height (from the frame base), the exception pushed, and the handler at target runs
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Handler {
    pub start: u32,
    pub end: u32,
    pub target: u32,
    pub height: u32,
}

/// where a closure gets an upvalue: a slot of the enclosing frame (holding a cell) or an
/// upvalue of the enclosing closure
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Up {
    Local(u32),
    Up(u32),
}

/// a named local of a function: its slot and the instructions from..to (to exclusive) where
/// it is visible (the debugger's locals)
#[derive(Clone, Debug)]
pub(crate) struct Var {
    pub name: String,
    pub slot: u32,
    pub from: u32,
    pub to: u32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Proto {
    /// the name for stack traces ("" for the top-level code, "fn" for anonymous functions)
    pub name: String,
    pub params: u32,
    pub code: Vec<Op>,
    /// line and column of every instruction
    pub pos: Vec<(u32, u32)>,
    pub handlers: Vec<Handler>,
    pub ups: Vec<Up>,
    /// the most slots the function uses (parameters, locals, temporaries)
    pub max_stack: u32,
    /// (pc, line) of the statements' first instructions in code order -- only the first
    /// statement of a line, and the head of a for loop (each round): where the debugger stops
    pub stmts: Vec<(u32, u32)>,
    /// the named locals in the order of their declaration
    pub vars: Vec<Var>,
    /// the names of the upvalues (as ups)
    pub up_names: Vec<String>,
    /// the module the function belongs to: 0 in a Program, the unit in a VM's linked code
    pub unit: u32,
}

impl Proto {
    /// the line of the statement that starts at pc
    pub fn stmt_line(&self, pc: u32) -> Option<u32> {
        self.stmts.binary_search_by_key(&pc, |s| s.0).ok().map(|i| self.stmts[i].1)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Const {
    Int(i64),
    Float(f64),
    Str(String),
}

/// a name the script takes from the host: a plain name (print, config) or base.field for
/// method calls and fields of host names (http.serve, config.port)
#[derive(Clone, Debug)]
pub(crate) struct HostName {
    pub name: String,
    /// base entry and constant of the field name, for dotted names
    pub dotted: Option<(u32, u32)>,
}

#[derive(Clone, Debug)]
pub(crate) struct Code {
    pub name: String,
    pub protos: Vec<Proto>,
    pub consts: Vec<Const>,
    pub globals: Vec<String>,
    pub hosts: Vec<HostName>,
    /// the host's names when compile_with made it (None: compile, compile_named); the
    /// modules a script imports compile the same way
    pub strict: Option<Vec<String>>,
}

/// where an instruction is in the source (Program::line_table); the file is Program::name
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineEntry {
    /// the function (0: the top-level code; Program::functions has the names)
    pub func: u32,
    /// the instruction's index in its function
    pub pc: u32,
    pub line: u32,
    pub col: u32,
    /// a statement starts here: stepping stops here, and a breakpoint of the line
    pub stmt: bool,
}

/// a compiled script (see compile()); clones share the code
#[derive(Clone, Debug)]
pub struct Program {
    pub(crate) code: Arc<Code>,
}

impl Program {
    /// the name given to compile_named()
    pub fn name(&self) -> &str {
        &self.code.name
    }

    /// the script's globals (top-level let and fn) in the order of the source
    pub fn globals(&self) -> Vec<&str> {
        self.code.globals.iter().map(|s| s.as_str()).collect()
    }

    /// the names the script takes from the host (print, http.serve, config ..)
    pub fn host_names(&self) -> Vec<&str> {
        self.code.hosts.iter().map(|h| h.name.as_str()).collect()
    }

    /// the number of instructions of all functions
    pub fn instructions(&self) -> usize {
        self.code.protos.iter().map(|p| p.code.len()).sum()
    }

    /// every instruction of every function in order, with its line and column
    pub fn line_table(&self) -> Vec<LineEntry> {
        let mut v = Vec::with_capacity(self.instructions());
        for (f, p) in self.code.protos.iter().enumerate() {
            for (pc, &(line, col)) in p.pos.iter().enumerate() {
                let stmt = p.stmt_line(pc as u32).is_some();
                v.push(LineEntry { func: f as u32, pc: pc as u32, line, col, stmt });
            }
        }
        v
    }

    /// the lines where a statement starts, sorted: a breakpoint stops there (one on another
    /// line at the next of these)
    pub fn break_lines(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.code.protos.iter().flat_map(|p| p.stmts.iter().map(|s| s.1)).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// the functions' names by their number in line_table ("" for the top-level code, "fn"
    /// for anonymous ones)
    pub fn functions(&self) -> Vec<&str> {
        self.code.protos.iter().map(|p| p.name.as_str()).collect()
    }

    /// the code as text: per function a header, then "pc line:col instruction"
    pub fn disasm(&self) -> String {
        let c = &self.code;
        let mut o = String::new();
        for (i, p) in c.protos.iter().enumerate() {
            let name = if i == 0 { "<main>" } else { p.name.as_str() };
            o.push_str(&format!("fn {} #{} params {} stack {}\n", name, i, p.params, p.max_stack));
            for (n, u) in p.ups.iter().enumerate() {
                o.push_str(&format!("  up {} = {:?}\n", n, u));
            }
            for h in &p.handlers {
                o.push_str(&format!("  try {}..{} -> {} height {}\n", h.start, h.end, h.target, h.height));
            }
            for (pc, op) in p.code.iter().enumerate() {
                let (l, col) = p.pos[pc];
                let note = match *op {
                    Op::Const(k) | Op::GetField(k) | Op::SetField(k) | Op::CallMethod(k, _) => format!("  ; {:?}", c.consts[k as usize]),
                    Op::GetGlobal(g) | Op::SetGlobal(g) => format!("  ; {}", c.globals[g as usize]),
                    Op::GetHost(h) | Op::GetHostField(h) | Op::AssignHost(h) | Op::CallHost(h, _) | Op::CallHostMethod(h, _) => {
                        format!("  ; {}", c.hosts[h as usize].name)
                    }
                    _ => String::new(),
                };
                o.push_str(&format!("  {:4} {:3}:{:<3} {:?}{}\n", pc, l, col, op, note));
            }
        }
        o
    }
}

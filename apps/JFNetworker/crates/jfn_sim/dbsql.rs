// dbsql -- the statements of the database service (db.rs): a small parser of a SQL-shaped language,
// its errors exact and with their place.  It is no SQL engine and no product's dialect
//
//   parse(text) -> Result<Stmt, SqlErr>   one statement (a ";" at the end is allowed)
//   SqlErr { pos, text }                  the place (1: the first character) and what is wrong, German
//                                         like the rest of what the lab's services answer;
//                                         SqlErr::line() "Zeichen 15: Hier fehlt FROM."
//   Stmt                                  Select, Insert, Update, Delete, Begin, Commit, Rollback,
//                                         CreateIndex, DropIndex, Explain
//   Stmt::table() / params()              the table it works on, how many ? it has
//   MAX_LEN, MAX_TOKENS                   the longest statement it reads
//
// What it reads:
//   SELECT * | COUNT(*) | a, b, t.c FROM t [JOIN u ON t.x = u.y] [WHERE cond AND cond ...]
//          [ORDER BY c [ASC|DESC]] [LIMIT n] [FOR UPDATE]
//     cond: col = v, col < v, col <= v, col > v, col >= v, col BETWEEN v AND v; v: a number, 'text', ?
//   INSERT INTO t (a, b) VALUES (v, v)    one row
//   UPDATE t SET a = v, b = b + v, c = c - v [WHERE ...]
//   DELETE FROM t [WHERE ...]
//   BEGIN [ISOLATION LEVEL READ COMMITTED | REPEATABLE READ], COMMIT, ROLLBACK
//   CREATE INDEX ON t (c), DROP INDEX ON t (c)
//   EXPLAIN statement
// Keywords in any case; names as written, their case does not matter to the schema.
//
// Left out, each with its own refusal: OR, NOT, LIKE, IN, NULL, GROUP BY, HAVING, other aggregates,
// subqueries, outer joins, more than one join, CREATE TABLE and ALTER, more than one row per INSERT.
// The grammar has no recursion (EXPLAIN once), so a long text costs time, not stack; beyond MAX_LEN and
// MAX_TOKENS it is refused.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// the longest statement in bytes and in tokens
pub const MAX_LEN: usize = 16 << 10;
pub const MAX_TOKENS: usize = 1024;

/// what is wrong with a statement and where
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlErr {
    /// the character it stands at, from 1
    pub pos: usize,
    pub text: String,
}

impl SqlErr {
    /// "Zeichen 15: Hier fehlt FROM."
    pub fn line(&self) -> String {
        format!("Zeichen {}: {}", self.pos, self.text)
    }
}

/// a value in a statement
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lit {
    Int(i64),
    Text(String),
    /// the n-th ? (from 0)
    Param(u16),
}

/// a column, with its table in a join
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColRef {
    pub table: Option<String>,
    pub name: String,
    /// where it stands in the statement (for errors of names)
    pub pos: usize,
}

/// a comparison
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
    Between,
}

/// a condition; all of a WHERE must hold
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cond {
    pub col: ColRef,
    pub op: Cmp,
    pub val: Lit,
    /// the upper end of BETWEEN
    pub hi: Option<Lit>,
}

/// what a SELECT gives
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum What {
    All,
    Count,
    Cols(Vec<ColRef>),
}

/// a join over an index: for each row of the first table its partners in the second
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub table: String,
    pub left: ColRef,
    pub right: ColRef,
    pub pos: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Select {
    pub what: What,
    pub table: String,
    pub table_pos: usize,
    pub join: Option<Join>,
    pub filter: Vec<Cond>,
    /// the column and "descending"
    pub order: Option<(ColRef, bool)>,
    pub limit: Option<u64>,
    pub for_update: bool,
}

/// what SET gives a column
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetTo {
    Lit(Lit),
    /// the column plus this much (a minus in front: minus)
    Add(Lit, bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set {
    pub col: ColRef,
    pub to: SetTo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Iso {
    ReadCommitted,
    RepeatableRead,
}

impl Iso {
    pub fn name(self) -> &'static str {
        match self {
            Iso::ReadCommitted => "read committed",
            Iso::RepeatableRead => "repeatable read",
        }
    }
    pub fn parse(s: &str) -> Option<Iso> {
        match s {
            "read committed" => Some(Iso::ReadCommitted),
            "repeatable read" => Some(Iso::RepeatableRead),
            _ => None,
        }
    }
}

/// one statement
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    Select(Select),
    Insert { table: String, table_pos: usize, cols: Vec<ColRef>, vals: Vec<Lit> },
    Update { table: String, table_pos: usize, sets: Vec<Set>, filter: Vec<Cond> },
    Delete { table: String, table_pos: usize, filter: Vec<Cond> },
    Begin(Option<Iso>),
    Commit,
    Rollback,
    CreateIndex { table: String, table_pos: usize, col: ColRef },
    DropIndex { table: String, table_pos: usize, col: ColRef },
    Explain(Box<Stmt>),
}

impl Stmt {
    /// the table it works on, and where that stands
    pub fn table(&self) -> Option<(&str, usize)> {
        match self {
            Stmt::Select(s) => Some((&s.table, s.table_pos)),
            Stmt::Insert { table, table_pos, .. } | Stmt::Update { table, table_pos, .. } | Stmt::Delete { table, table_pos, .. } | Stmt::CreateIndex { table, table_pos, .. } | Stmt::DropIndex { table, table_pos, .. } => Some((table, *table_pos)),
            Stmt::Explain(s) => s.table(),
            _ => None,
        }
    }
    /// it changes rows
    pub fn writes(&self) -> bool {
        matches!(self, Stmt::Insert { .. } | Stmt::Update { .. } | Stmt::Delete { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Int(i64),
    Text(String),
    Sym(&'static str),
    End,
}

/// the tokens with their places
fn lex(s: &str) -> Result<Vec<(Tok, usize)>, SqlErr> {
    if s.len() > MAX_LEN {
        return Err(SqlErr { pos: MAX_LEN, text: format!("Der Text ist zu lang: höchstens {} Bytes.", MAX_LEN) });
    }
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    // places count characters
    let place = |byte: usize| s[..byte].chars().count() + 1;
    while i < b.len() {
        let c = b[i];
        let at = i;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if out.len() >= MAX_TOKENS {
            return Err(SqlErr { pos: place(at), text: format!("Die Anweisung ist zu lang: höchstens {} Wörter und Zeichen.", MAX_TOKENS) });
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push((Tok::Word(String::from(&s[at..i])), place(at)));
            continue;
        }
        if c.is_ascii_digit() {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let Ok(v) = s[at..i].parse::<i64>() else {
                return Err(SqlErr { pos: place(at), text: format!("Die Zahl {} ist zu groß.", &s[at..i]) });
            };
            out.push((Tok::Int(v), place(at)));
            continue;
        }
        if c == b'\'' {
            i += 1;
            let mut t = Vec::new();
            loop {
                match b.get(i) {
                    None => return Err(SqlErr { pos: place(at), text: String::from("Hier beginnt ein Text, der nicht endet: es fehlt das schließende '.") }),
                    Some(b'\'') if b.get(i + 1) == Some(&b'\'') => {
                        t.push(b'\'');
                        i += 2;
                    }
                    Some(b'\'') => {
                        i += 1;
                        break;
                    }
                    Some(x) => {
                        t.push(*x);
                        i += 1;
                    }
                }
            }
            out.push((Tok::Text(String::from_utf8_lossy(&t).into_owned()), place(at)));
            continue;
        }
        let two = s.get(at..at + 2).unwrap_or("");
        let sym: Option<&'static str> = match two {
            "<=" => Some("<="),
            ">=" => Some(">="),
            "<>" | "!=" => {
                return Err(SqlErr { pos: place(at), text: String::from("Ungleich gibt es hier nicht: nur =, <, <=, >, >= und BETWEEN.") });
            }
            _ => match c {
                b'*' => Some("*"),
                b',' => Some(","),
                b'(' => Some("("),
                b')' => Some(")"),
                b'=' => Some("="),
                b'<' => Some("<"),
                b'>' => Some(">"),
                b'?' => Some("?"),
                b'.' => Some("."),
                b';' => Some(";"),
                b'+' => Some("+"),
                b'-' => Some("-"),
                _ => None,
            },
        };
        let Some(sym) = sym else {
            let ch = s[at..].chars().next().unwrap_or('?');
            return Err(SqlErr { pos: place(at), text: format!("Das Zeichen {} gibt es hier nicht.", ch) });
        };
        i += sym.len();
        out.push((Tok::Sym(sym), place(at)));
    }
    out.push((Tok::End, place(s.len())));
    Ok(out)
}

/// words that may not be names
const RESERVED: &[&str] = &["select", "from", "where", "and", "or", "not", "order", "by", "limit", "for", "update", "insert", "into", "values", "set", "delete", "begin", "commit", "rollback", "create", "drop", "index", "on", "join", "explain", "between", "asc", "desc", "count"];

struct P {
    toks: Vec<(Tok, usize)>,
    i: usize,
    params: u16,
}

impl P {
    fn peek(&self) -> &Tok {
        &self.toks[self.i.min(self.toks.len() - 1)].0
    }
    fn pos(&self) -> usize {
        self.toks[self.i.min(self.toks.len() - 1)].1
    }
    fn next(&mut self) {
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
    }
    /// what stands here, as an error's words
    fn found(&self) -> String {
        match self.peek() {
            Tok::Word(w) => format!("\u{201e}{}\u{201c}", w),
            Tok::Int(v) => format!("{}", v),
            Tok::Text(t) => format!("'{}'", t),
            Tok::Sym(s) => format!("\u{201e}{}\u{201c}", s),
            Tok::End => String::from("das Ende"),
        }
    }
    fn err<T>(&self, text: String) -> Result<T, SqlErr> {
        Err(SqlErr { pos: self.pos(), text })
    }
    fn want(&self, what: &str) -> SqlErr {
        SqlErr { pos: self.pos(), text: format!("Hier fehlt {}, es steht {} da.", what, self.found()) }
    }
    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Word(w) if w.eq_ignore_ascii_case(kw))
    }
    fn kw(&mut self, kw: &str) -> bool {
        let yes = self.is_kw(kw);
        if yes {
            self.next();
        }
        yes
    }
    fn need_kw(&mut self, kw: &str) -> Result<(), SqlErr> {
        if self.kw(kw) { Ok(()) } else { Err(self.want(kw)) }
    }
    fn sym(&mut self, s: &str) -> bool {
        let yes = matches!(self.peek(), Tok::Sym(x) if *x == s);
        if yes {
            self.next();
        }
        yes
    }
    fn need_sym(&mut self, s: &str) -> Result<(), SqlErr> {
        if self.sym(s) { Ok(()) } else { Err(self.want(s)) }
    }
    /// a name of a table or column
    fn name(&mut self, what: &str) -> Result<(String, usize), SqlErr> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Word(w) if !RESERVED.iter().any(|r| w.eq_ignore_ascii_case(r)) => {
                self.next();
                Ok((w, pos))
            }
            _ => Err(self.want(what)),
        }
    }
    /// a column, with its table in front or not
    fn col(&mut self) -> Result<ColRef, SqlErr> {
        let (a, pos) = self.name("ein Spaltenname")?;
        if self.sym(".") {
            let (b, _) = self.name("ein Spaltenname")?;
            return Ok(ColRef { table: Some(a), name: b, pos });
        }
        Ok(ColRef { table: None, name: a, pos })
    }
    fn lit(&mut self) -> Result<Lit, SqlErr> {
        match self.peek().clone() {
            Tok::Int(v) => {
                self.next();
                Ok(Lit::Int(v))
            }
            Tok::Text(t) => {
                self.next();
                Ok(Lit::Text(t))
            }
            Tok::Sym("?") => {
                self.next();
                self.params += 1;
                Ok(Lit::Param(self.params - 1))
            }
            Tok::Sym("-") => {
                self.next();
                match self.peek().clone() {
                    Tok::Int(v) => {
                        self.next();
                        Ok(Lit::Int(-v))
                    }
                    _ => Err(self.want("eine Zahl")),
                }
            }
            Tok::Word(w) if w.eq_ignore_ascii_case("null") => self.err(String::from("NULL gibt es hier nicht: jede Zeile hat in jeder Spalte einen Wert.")),
            _ => Err(self.want("ein Wert (eine Zahl, 'Text' oder ?)")),
        }
    }
    fn filter(&mut self) -> Result<Vec<Cond>, SqlErr> {
        let mut out = Vec::new();
        if !self.kw("where") {
            return Ok(out);
        }
        loop {
            if self.is_kw("not") {
                return self.err(String::from("NOT gibt es hier nicht."));
            }
            if matches!(self.peek(), Tok::Sym("(")) {
                return self.err(String::from("Klammern gibt es in WHERE nicht: Bedingungen hängen nur mit AND zusammen."));
            }
            let col = self.col()?;
            if self.is_kw("like") || self.is_kw("in") || self.is_kw("is") {
                return self.err(format!("{} gibt es hier nicht: nur =, <, <=, >, >= und BETWEEN.", self.found()));
            }
            let (op, val, hi) = if self.sym("=") {
                (Cmp::Eq, self.lit()?, None)
            } else if self.sym("<=") {
                (Cmp::Le, self.lit()?, None)
            } else if self.sym(">=") {
                (Cmp::Ge, self.lit()?, None)
            } else if self.sym("<") {
                (Cmp::Lt, self.lit()?, None)
            } else if self.sym(">") {
                (Cmp::Gt, self.lit()?, None)
            } else if self.kw("between") {
                let a = self.lit()?;
                self.need_kw("and")?;
                (Cmp::Between, a, Some(self.lit()?))
            } else {
                return Err(self.want("ein Vergleich (=, <, <=, >, >=, BETWEEN)"));
            };
            out.push(Cond { col, op, val, hi });
            if self.is_kw("or") {
                return self.err(String::from("OR gibt es hier nicht: Bedingungen hängen nur mit AND zusammen."));
            }
            if !self.kw("and") {
                return Ok(out);
            }
        }
    }
    fn table_name(&mut self) -> Result<(String, usize), SqlErr> {
        if matches!(self.peek(), Tok::Sym("(")) {
            return self.err(String::from("Unterabfragen gibt es hier nicht."));
        }
        self.name("ein Tabellenname")
    }
    fn select(&mut self) -> Result<Stmt, SqlErr> {
        let what = if self.sym("*") {
            What::All
        } else if self.is_kw("count") {
            self.next();
            self.need_sym("(")?;
            self.need_sym("*")?;
            self.need_sym(")")?;
            What::Count
        } else if self.is_kw("sum") || self.is_kw("avg") || self.is_kw("min") || self.is_kw("max") {
            return self.err(format!("{} gibt es hier nicht: von den Aggregaten nur COUNT(*).", self.found()));
        } else {
            let mut cols = alloc::vec![self.col()?];
            while self.sym(",") {
                cols.push(self.col()?);
            }
            What::Cols(cols)
        };
        self.need_kw("from")?;
        let (table, table_pos) = self.table_name()?;
        let mut join = None;
        if self.is_kw("left") || self.is_kw("right") || self.is_kw("outer") || self.is_kw("inner") || self.is_kw("cross") {
            return self.err(String::from("Von den Joins gibt es nur JOIN ... ON a.x = b.y."));
        }
        let jpos = self.pos();
        if self.kw("join") {
            let (jt, _) = self.table_name()?;
            self.need_kw("on")?;
            let left = self.col()?;
            self.need_sym("=")?;
            let right = self.col()?;
            join = Some(Join { table: jt, left, right, pos: jpos });
            if self.is_kw("join") {
                return self.err(String::from("Mehr als ein JOIN gibt es hier nicht."));
            }
        }
        let filter = self.filter()?;
        if self.is_kw("group") || self.is_kw("having") {
            return self.err(String::from("GROUP BY und HAVING gibt es hier nicht."));
        }
        let mut order = None;
        if self.kw("order") {
            self.need_kw("by")?;
            let c = self.col()?;
            let desc = if self.kw("desc") {
                true
            } else {
                self.kw("asc");
                false
            };
            order = Some((c, desc));
        }
        let mut limit = None;
        if self.kw("limit") {
            match self.peek().clone() {
                Tok::Int(v) if v >= 0 => {
                    self.next();
                    limit = Some(v as u64);
                }
                _ => return Err(self.want("eine Zahl hinter LIMIT")),
            }
        }
        let mut for_update = false;
        if self.kw("for") {
            self.need_kw("update")?;
            for_update = true;
        }
        Ok(Stmt::Select(Select { what, table, table_pos, join, filter, order, limit, for_update }))
    }
    fn insert(&mut self) -> Result<Stmt, SqlErr> {
        self.need_kw("into")?;
        let (table, table_pos) = self.table_name()?;
        self.need_sym("(")?;
        let mut cols = alloc::vec![self.col()?];
        while self.sym(",") {
            cols.push(self.col()?);
        }
        self.need_sym(")")?;
        self.need_kw("values")?;
        let vp = self.pos();
        self.need_sym("(")?;
        let mut vals = alloc::vec![self.lit()?];
        while self.sym(",") {
            vals.push(self.lit()?);
        }
        self.need_sym(")")?;
        if vals.len() != cols.len() {
            return Err(SqlErr { pos: vp, text: format!("{} Spalten, aber {} Werte.", cols.len(), vals.len()) });
        }
        if matches!(self.peek(), Tok::Sym(",")) {
            return self.err(String::from("Ein INSERT nimmt hier nur eine Zeile."));
        }
        Ok(Stmt::Insert { table, table_pos, cols, vals })
    }
    fn update(&mut self) -> Result<Stmt, SqlErr> {
        let (table, table_pos) = self.table_name()?;
        self.need_kw("set")?;
        let mut sets = Vec::new();
        loop {
            let col = self.col()?;
            self.need_sym("=")?;
            // the column itself plus or minus a value, or a value
            let same = matches!(self.peek(), Tok::Word(w) if w.eq_ignore_ascii_case(&col.name));
            let to = if same {
                self.next();
                let minus = if self.sym("+") {
                    false
                } else if self.sym("-") {
                    true
                } else {
                    return Err(self.want("+ oder -"));
                };
                SetTo::Add(self.lit()?, minus)
            } else {
                SetTo::Lit(self.lit()?)
            };
            sets.push(Set { col, to });
            if !self.sym(",") {
                break;
            }
        }
        let filter = self.filter()?;
        Ok(Stmt::Update { table, table_pos, sets, filter })
    }
    /// ON t (c)
    fn index_of(&mut self) -> Result<(String, usize, ColRef), SqlErr> {
        self.need_kw("on")?;
        let (table, table_pos) = self.table_name()?;
        self.need_sym("(")?;
        let col = self.col()?;
        self.need_sym(")")?;
        Ok((table, table_pos, col))
    }
    fn stmt(&mut self, inner: bool) -> Result<Stmt, SqlErr> {
        if self.kw("select") {
            return self.select();
        }
        if self.kw("insert") {
            return self.insert();
        }
        if self.kw("update") {
            return self.update();
        }
        if self.kw("delete") {
            self.need_kw("from")?;
            let (table, table_pos) = self.table_name()?;
            let filter = self.filter()?;
            return Ok(Stmt::Delete { table, table_pos, filter });
        }
        if self.kw("begin") {
            let mut iso = None;
            if self.kw("isolation") {
                self.need_kw("level")?;
                if self.kw("read") {
                    self.need_kw("committed")?;
                    iso = Some(Iso::ReadCommitted);
                } else if self.kw("repeatable") {
                    self.need_kw("read")?;
                    iso = Some(Iso::RepeatableRead);
                } else {
                    return Err(self.want("READ COMMITTED oder REPEATABLE READ"));
                }
            }
            return Ok(Stmt::Begin(iso));
        }
        if self.kw("commit") {
            return Ok(Stmt::Commit);
        }
        if self.kw("rollback") {
            return Ok(Stmt::Rollback);
        }
        if self.kw("create") {
            if self.is_kw("table") {
                return self.err(String::from("CREATE TABLE gibt es hier nicht: Tabellen stehen in der Konfiguration des Dienstes."));
            }
            self.need_kw("index")?;
            let (table, table_pos, col) = self.index_of()?;
            return Ok(Stmt::CreateIndex { table, table_pos, col });
        }
        if self.kw("drop") {
            if self.is_kw("table") {
                return self.err(String::from("DROP TABLE gibt es hier nicht: Tabellen stehen in der Konfiguration des Dienstes."));
            }
            self.need_kw("index")?;
            let (table, table_pos, col) = self.index_of()?;
            return Ok(Stmt::DropIndex { table, table_pos, col });
        }
        if self.is_kw("alter") {
            return self.err(String::from("ALTER gibt es hier nicht: Tabellen stehen in der Konfiguration des Dienstes."));
        }
        if self.is_kw("explain") {
            if inner {
                return self.err(String::from("Ein EXPLAIN in einem EXPLAIN gibt es nicht."));
            }
            self.next();
            let s = self.stmt(true)?;
            if !matches!(s, Stmt::Select(_) | Stmt::Update { .. } | Stmt::Delete { .. }) {
                return Err(SqlErr { pos: self.pos(), text: String::from("EXPLAIN nimmt SELECT, UPDATE und DELETE.") });
            }
            return Ok(Stmt::Explain(Box::new(s)));
        }
        Err(SqlErr { pos: self.pos(), text: format!("Hier fehlt eine Anweisung (SELECT, INSERT, UPDATE, DELETE, BEGIN, COMMIT, ROLLBACK, CREATE INDEX, DROP INDEX, EXPLAIN), es steht {} da.", self.found()) })
    }
}

/// one statement
pub fn parse(text: &str) -> Result<Stmt, SqlErr> {
    let toks = lex(text)?;
    let mut p = P { toks, i: 0, params: 0 };
    let s = p.stmt(false)?;
    p.sym(";");
    if *p.peek() != Tok::End {
        return p.err(format!("Hinter der Anweisung steht noch {}.", p.found()));
    }
    Ok(s)
}

/// how many ? a text has (outside its strings): what a caller must bring
pub fn params_of(s: &Stmt) -> u16 {
    fn lit(l: &Lit, n: &mut u16) {
        if let Lit::Param(i) = l {
            *n = (*n).max(*i + 1);
        }
    }
    fn conds(f: &[Cond], n: &mut u16) {
        for c in f {
            lit(&c.val, n);
            if let Some(h) = &c.hi {
                lit(h, n);
            }
        }
    }
    let mut n = 0;
    match s {
        Stmt::Select(x) => conds(&x.filter, &mut n),
        Stmt::Insert { vals, .. } => vals.iter().for_each(|v| lit(v, &mut n)),
        Stmt::Update { sets, filter, .. } => {
            for set in sets {
                match &set.to {
                    SetTo::Lit(l) | SetTo::Add(l, _) => lit(l, &mut n),
                }
            }
            conds(filter, &mut n);
        }
        Stmt::Delete { filter, .. } => conds(filter, &mut n),
        Stmt::Explain(x) => n = params_of(x),
        _ => {}
    }
    n
}

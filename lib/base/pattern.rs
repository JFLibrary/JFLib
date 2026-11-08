// pattern -- shell wildcards and a small engine for regular expressions (the shells of
// JFNetworker's hosts: globs, find -name, grep)
//
//   glob(pat, s) -> bool          fnmatch(3) without flags: * any run of characters, ? one,
//                                 [abc] [a-z] [!x] [^x] [[:digit:]] one of a set, \x the
//                                 character x; '/' and a leading '.' are ordinary characters
//                                 here (a caller matches path components one by one and hides
//                                 dot files itself)
//   has_glob(s) -> bool           s holds an unescaped *, ? or [ (a word to expand)
//   Regex::new(pat, Syntax::Basic | Syntax::Extended, icase) -> Result<Regex, ReErr>
//   Regex::is_match(s)            the expression matches somewhere in s
//   Regex::find(s) / find_at(s, from) -> Option<(start, end)>   byte offsets of the leftmost
//                                 match, the longest one starting there (POSIX)
//   ReErr::text()                 the error in GNU grep's words ("Unmatched ( or \(")
//
// The expressions are POSIX's basic (BRE) and extended (ERE) syntax without back-references
// and collating elements: characters, ".", bracket expressions with ranges, negation and the
// classes [:alpha:] [:digit:] [:alnum:] [:upper:] [:lower:] [:space:] [:blank:] [:punct:]
// [:xdigit:] [:print:] [:graph:] [:cntrl:]; the anchors ^ and $; the repetitions *, and in
// ERE + ? {m} {m,} {m,n} (BRE: \+ \? \{m,n\} as GNU takes them); groups ( ) and alternatives |
// (BRE: \( \) \|); GNU's \w \W \s \S and the word edges \< \> \b \B.  As GNU does, a
// repetition with nothing before it is an ordinary character, an ERE "{" that starts no
// interval too, and BRE takes ^ and $ as anchors only at the ends of the expression or of a
// group.  Matching runs a Pike VM over the characters: time grows with the text times the
// program, never exponentially; a repetition counts to 255 at most and a program has at most
// 20000 steps (ReErr::TooBig beyond).  Case-insensitive matching compares simple lowercase
// forms.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

// ---------------------------------------------------------------- wildcards

/// a character class name of a bracket expression
fn class_holds(name: &str, c: char) -> Option<bool> {
    Some(match name {
        "alpha" => c.is_alphabetic(),
        "digit" => c.is_ascii_digit(),
        "alnum" => c.is_alphanumeric(),
        "upper" => c.is_uppercase(),
        "lower" => c.is_lowercase(),
        "space" => c.is_whitespace(),
        "blank" => c == ' ' || c == '\t',
        "punct" => c.is_ascii_punctuation(),
        "xdigit" => c.is_ascii_hexdigit(),
        "print" => !c.is_control(),
        "graph" => !c.is_control() && !c.is_whitespace(),
        "cntrl" => c.is_control(),
        _ => return None,
    })
}

/// a bracket expression from just after its '[': the set and the index after its ']' (None:
/// not closed, or a class name that does not exist)
fn bracket(p: &[char], mut i: usize, glob: bool) -> Option<(Set, usize)> {
    let mut set = Set { neg: false, items: Vec::new() };
    if i < p.len() && (p[i] == '^' || (glob && p[i] == '!')) {
        set.neg = true;
        i += 1;
    }
    let start = i;
    loop {
        let c = *p.get(i)?;
        if c == ']' && i > start {
            return Some((set, i + 1));
        }
        if c == '[' && p.get(i + 1) == Some(&':') {
            let rest = &p[i + 2..];
            let end = rest.windows(2).position(|w| w == [':', ']'])?;
            let name: alloc::string::String = rest[..end].iter().collect();
            class_holds(&name, 'a')?;
            set.items.push(Item::Class(name));
            i += 2 + end + 2;
            continue;
        }
        if c == '[' && matches!(p.get(i + 1), Some('.') | Some('=')) {
            return None;
        }
        let mut lo = c;
        i += 1;
        if glob && c == '\\' && i < p.len() {
            lo = p[i];
            i += 1;
        }
        if p.get(i) == Some(&'-') && p.get(i + 1).is_some_and(|&x| x != ']') {
            let mut hi = p[i + 1];
            i += 2;
            if glob && hi == '\\' && i < p.len() {
                hi = p[i];
                i += 1;
            }
            if hi < lo {
                return None;
            }
            set.items.push(Item::Range(lo, hi));
        } else {
            set.items.push(Item::Range(lo, lo));
        }
    }
}

/// the wildcard pattern matches all of s (fnmatch without flags)
pub fn glob(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // the last '*' and where its run ends now: a mismatch makes the run one longer
    let mut star: Option<(usize, usize)> = None;
    loop {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    star = Some((pi, ti));
                    pi += 1;
                    continue;
                }
                '?' if ti < t.len() => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                '[' => {
                    if let Some((set, next)) = bracket(&p, pi + 1, true) {
                        if ti < t.len() && set.holds(t[ti], false) {
                            pi = next;
                            ti += 1;
                            continue;
                        }
                    } else if ti < t.len() && t[ti] == '[' {
                        // an open '[' stands for itself
                        pi += 1;
                        ti += 1;
                        continue;
                    }
                }
                '\\' if pi + 1 < p.len() => {
                    if ti < t.len() && t[ti] == p[pi + 1] {
                        pi += 2;
                        ti += 1;
                        continue;
                    }
                }
                '?' => {}
                c => {
                    if ti < t.len() && t[ti] == c {
                        pi += 1;
                        ti += 1;
                        continue;
                    }
                }
            }
        } else if ti == t.len() {
            return true;
        }
        match star {
            Some((sp, st)) if st < t.len() => {
                star = Some((sp, st + 1));
                pi = sp + 1;
                ti = st + 1;
            }
            _ => return false,
        }
    }
}

/// the word holds an unescaped wildcard (*, ? or [)
pub fn has_glob(s: &str) -> bool {
    let mut esc = false;
    for c in s.chars() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '*' | '?' | '[' => return true,
            _ => {}
        }
    }
    false
}

// ---------------------------------------------------------------- regular expressions

/// which syntax an expression is written in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Syntax {
    /// POSIX basic (grep): \( \) \{ \} \| \+ \? are the operators
    Basic,
    /// POSIX extended (grep -E, egrep)
    Extended,
}

/// why an expression does not compile
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReErr {
    /// an open group without its end
    Paren,
    /// a group's end without its start
    CloseParen,
    /// a bracket expression without its ']'
    Bracket,
    /// a range whose end comes before its start
    Range,
    /// [:name:] with a name that is no class
    Class,
    /// a '\' at the end
    Backslash,
    /// \{ without \}
    Brace,
    /// what is between { and } is no interval
    BraceContent,
    /// a repetition of something that cannot repeat
    Repeat,
    /// the program would be too long
    TooBig,
}

impl ReErr {
    /// GNU grep's message
    pub fn text(self) -> &'static str {
        match self {
            ReErr::Paren => "Unmatched ( or \\(",
            ReErr::CloseParen => "Unmatched ) or \\)",
            ReErr::Bracket => "Unmatched [, [^, [:, [., or [=",
            ReErr::Range => "Invalid range end",
            ReErr::Class => "Invalid character class name",
            ReErr::Backslash => "Trailing backslash",
            ReErr::Brace => "Unmatched \\{",
            ReErr::BraceContent => "Invalid content of \\{\\}",
            ReErr::Repeat => "Invalid preceding regular expression",
            ReErr::TooBig => "Regular expression too big",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    Range(char, char),
    Class(alloc::string::String),
}

/// a set of characters of a bracket expression or of \w \s
#[derive(Clone, Debug, PartialEq, Eq)]
struct Set {
    neg: bool,
    items: Vec<Item>,
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn upper(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

impl Set {
    fn holds(&self, c: char, icase: bool) -> bool {
        let one = |c: char| {
            self.items.iter().any(|it| match it {
                Item::Range(a, b) => *a <= c && c <= *b,
                Item::Class(n) => class_holds(n, c).unwrap_or(false),
            })
        };
        let hit = one(c) || (icase && (one(lower(c)) || one(upper(c))));
        hit != self.neg
    }
    fn word(neg: bool) -> Set {
        Set { neg, items: vec![Item::Class(alloc::string::String::from("alnum")), Item::Range('_', '_')] }
    }
    fn space(neg: bool) -> Set {
        Set { neg, items: vec![Item::Class(alloc::string::String::from("space"))] }
    }
}

/// a zero-width test at a position
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Bol,
    Eol,
    /// \< and \>: a word starts / ends here
    WordStart,
    WordEnd,
    /// \b and \B
    Word,
    NotWord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Empty,
    Char(char),
    Any,
    Set(Set),
    Edge(Edge),
    Cat(Vec<Node>),
    Alt(Vec<Node>),
    Rep(Box<Node>, u32, Option<u32>),
}

const REP_MAX: u32 = 255;
const PROG_MAX: usize = 20_000;

struct Parser<'a> {
    p: &'a [char],
    i: usize,
    ere: bool,
    depth: u32,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.p.get(self.i).copied()
    }
    fn peek2(&self) -> Option<char> {
        self.p.get(self.i + 1).copied()
    }
    /// at "|" (ERE) or "\|" (BRE)
    fn at_alt(&self) -> bool {
        if self.ere { self.peek() == Some('|') } else { self.peek() == Some('\\') && self.peek2() == Some('|') }
    }
    /// at ")" (ERE) or "\)" (BRE)
    fn at_close(&self) -> bool {
        if self.ere { self.peek() == Some(')') } else { self.peek() == Some('\\') && self.peek2() == Some(')') }
    }
    fn alt(&mut self) -> Result<Node, ReErr> {
        let mut alts = vec![self.cat()?];
        while self.at_alt() {
            self.i += if self.ere { 1 } else { 2 };
            alts.push(self.cat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap_or(Node::Empty) } else { Node::Alt(alts) })
    }
    fn cat(&mut self) -> Result<Node, ReErr> {
        let mut v: Vec<Node> = Vec::new();
        let begin = self.i;
        while self.i < self.p.len() && !self.at_alt() && !(self.depth > 0 && self.at_close()) {
            if self.at_close() {
                // ")" without a group: ERE takes it as a character only in GNU's lax way
                return Err(ReErr::CloseParen);
            }
            let first = self.i == begin;
            let atom = self.atom(first)?;
            let atom = self.reps(atom)?;
            v.push(atom);
        }
        Ok(match v.len() {
            0 => Node::Empty,
            1 => v.pop().unwrap_or(Node::Empty),
            _ => Node::Cat(v),
        })
    }
    /// an interval's numbers from just after its "{" up to and with its "}" ("\}" in BRE);
    /// None when it is none (ERE: then "{" is a character)
    fn interval(&mut self) -> Result<Option<(u32, Option<u32>)>, ReErr> {
        let save = self.i;
        let num = |s: &mut Parser| -> Option<u32> {
            let st = s.i;
            while s.peek().is_some_and(|c| c.is_ascii_digit()) {
                s.i += 1;
            }
            if s.i == st {
                return None;
            }
            let t: alloc::string::String = s.p[st..s.i].iter().collect();
            Some(t.parse::<u32>().unwrap_or(u32::MAX))
        };
        let lo = num(self);
        let comma = self.peek() == Some(',');
        if comma {
            self.i += 1;
        }
        let hi = if comma { num(self) } else { lo };
        let closed = if self.ere {
            if self.peek() == Some('}') {
                self.i += 1;
                true
            } else {
                false
            }
        } else if self.peek() == Some('\\') && self.peek2() == Some('}') {
            self.i += 2;
            true
        } else {
            false
        };
        if !closed || (lo.is_none() && !comma) {
            if self.ere {
                self.i = save;
                return Ok(None);
            }
            return Err(if closed { ReErr::BraceContent } else { ReErr::Brace });
        }
        // "{,n}" is "{0,n}" (GNU)
        let lo = lo.unwrap_or(0);
        if lo > REP_MAX || hi.is_some_and(|h| h > REP_MAX || h < lo) {
            return Err(if hi.is_some_and(|h| h < lo) { ReErr::BraceContent } else { ReErr::TooBig });
        }
        Ok(Some((lo, hi)))
    }
    /// the repetitions after an atom
    fn reps(&mut self, mut atom: Node) -> Result<Node, ReErr> {
        loop {
            // an anchor does not repeat: the "*" after it is a character (GNU)
            if matches!(atom, Node::Edge(_)) {
                return Ok(atom);
            }
            let (lo, hi) = match (self.peek(), self.peek2()) {
                (Some('*'), _) => {
                    self.i += 1;
                    (0, None)
                }
                (Some('+'), _) if self.ere => {
                    self.i += 1;
                    (1, None)
                }
                (Some('?'), _) if self.ere => {
                    self.i += 1;
                    (0, Some(1))
                }
                (Some('\\'), Some('+')) if !self.ere => {
                    self.i += 2;
                    (1, None)
                }
                (Some('\\'), Some('?')) if !self.ere => {
                    self.i += 2;
                    (0, Some(1))
                }
                (Some('{'), _) if self.ere => {
                    self.i += 1;
                    match self.interval()? {
                        Some(x) => x,
                        None => {
                            // "{" that starts no interval is a character
                            self.i -= 1;
                            return Ok(atom);
                        }
                    }
                }
                (Some('\\'), Some('{')) if !self.ere => {
                    self.i += 2;
                    match self.interval()? {
                        Some(x) => x,
                        None => return Err(ReErr::Brace),
                    }
                }
                _ => return Ok(atom),
            };
            // a repetition of a repetition that may be empty makes no new strings: x** is x*
            if let Node::Rep(_, 0, None) = atom
                && lo <= 1
            {
                continue;
            }
            atom = Node::Rep(Box::new(atom), lo, hi);
        }
    }
    fn atom(&mut self, first: bool) -> Result<Node, ReErr> {
        let Some(c) = self.peek() else { return Ok(Node::Empty) };
        self.i += 1;
        match c {
            '.' => Ok(Node::Any),
            '[' => {
                let (set, next) = match bracket(self.p, self.i, false) {
                    Some(x) => x,
                    None => return Err(self.bracket_err()),
                };
                self.i = next;
                Ok(Node::Set(set))
            }
            '^' if self.ere || first => Ok(Node::Edge(Edge::Bol)),
            '$' if self.ere || self.i == self.p.len() || self.at_close() || self.at_alt() => Ok(Node::Edge(Edge::Eol)),
            // a repetition with nothing before it stands for itself
            '*' => Ok(Node::Char('*')),
            '+' | '?' if self.ere && first => Ok(Node::Char(c)),
            '{' if self.ere && first => Ok(Node::Char('{')),
            '(' if self.ere => self.group(),
            '\\' => {
                let Some(e) = self.peek() else { return Err(ReErr::Backslash) };
                self.i += 1;
                match e {
                    '(' if !self.ere => self.group(),
                    '{' if !self.ere && first => Ok(Node::Char('{')),
                    'w' => Ok(Node::Set(Set::word(false))),
                    'W' => Ok(Node::Set(Set::word(true))),
                    's' => Ok(Node::Set(Set::space(false))),
                    'S' => Ok(Node::Set(Set::space(true))),
                    '<' => Ok(Node::Edge(Edge::WordStart)),
                    '>' => Ok(Node::Edge(Edge::WordEnd)),
                    'b' => Ok(Node::Edge(Edge::Word)),
                    'B' => Ok(Node::Edge(Edge::NotWord)),
                    'n' => Ok(Node::Char('\n')),
                    't' => Ok(Node::Char('\t')),
                    x => Ok(Node::Char(x)),
                }
            }
            x => Ok(Node::Char(x)),
        }
    }
    fn bracket_err(&self) -> ReErr {
        // a range backwards or a class that does not exist, else the bracket is open
        let rest = &self.p[self.i..];
        let end = rest.iter().skip(1).position(|&c| c == ']').map(|x| x + 1);
        match end {
            Some(e) => {
                let body = &rest[..e];
                if body.windows(2).any(|w| w == ['[', ':']) {
                    ReErr::Class
                } else if body.windows(3).any(|w| w[1] == '-' && w[2] < w[0]) {
                    ReErr::Range
                } else {
                    ReErr::Bracket
                }
            }
            None => ReErr::Bracket,
        }
    }
    fn group(&mut self) -> Result<Node, ReErr> {
        self.depth += 1;
        let inner = self.alt()?;
        self.depth -= 1;
        if !self.at_close() {
            return Err(ReErr::Paren);
        }
        self.i += if self.ere { 1 } else { 2 };
        Ok(inner)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Inst {
    Char(char),
    Any,
    Set(usize),
    Edge(Edge),
    Split(usize, usize),
    Jmp(usize),
    Match,
}

/// a compiled regular expression
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Regex {
    prog: Vec<Inst>,
    sets: Vec<Set>,
    icase: bool,
}

struct Comp {
    prog: Vec<Inst>,
    sets: Vec<Set>,
}

impl Comp {
    fn emit(&mut self, i: Inst) -> Result<usize, ReErr> {
        if self.prog.len() >= PROG_MAX {
            return Err(ReErr::TooBig);
        }
        self.prog.push(i);
        Ok(self.prog.len() - 1)
    }
    fn node(&mut self, n: &Node) -> Result<(), ReErr> {
        match n {
            Node::Empty => {}
            Node::Char(c) => {
                self.emit(Inst::Char(*c))?;
            }
            Node::Any => {
                self.emit(Inst::Any)?;
            }
            Node::Set(s) => {
                self.sets.push(s.clone());
                let k = self.sets.len() - 1;
                self.emit(Inst::Set(k))?;
            }
            Node::Edge(e) => {
                self.emit(Inst::Edge(*e))?;
            }
            Node::Cat(v) => {
                for x in v {
                    self.node(x)?;
                }
            }
            Node::Alt(v) => {
                let mut jumps = Vec::new();
                for (k, x) in v.iter().enumerate() {
                    if k + 1 < v.len() {
                        let split = self.emit(Inst::Split(0, 0))?;
                        self.node(x)?;
                        jumps.push(self.emit(Inst::Jmp(0))?);
                        let next = self.prog.len();
                        self.prog[split] = Inst::Split(split + 1, next);
                    } else {
                        self.node(x)?;
                    }
                }
                let end = self.prog.len();
                for j in jumps {
                    self.prog[j] = Inst::Jmp(end);
                }
            }
            Node::Rep(x, lo, hi) => {
                for _ in 0..*lo {
                    self.node(x)?;
                }
                match hi {
                    None => {
                        // x*
                        let split = self.emit(Inst::Split(0, 0))?;
                        self.node(x)?;
                        self.emit(Inst::Jmp(split))?;
                        let end = self.prog.len();
                        self.prog[split] = Inst::Split(split + 1, end);
                    }
                    Some(h) => {
                        // (x(x(x)?)?)? for the optional ones
                        let mut splits = Vec::new();
                        for _ in *lo..*h {
                            splits.push(self.emit(Inst::Split(0, 0))?);
                            self.node(x)?;
                        }
                        let end = self.prog.len();
                        for s in splits {
                            self.prog[s] = Inst::Split(s + 1, end);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

impl Regex {
    /// compiles an expression; `icase`: upper and lower case are the same
    pub fn new(pat: &str, syntax: Syntax, icase: bool) -> Result<Regex, ReErr> {
        let p: Vec<char> = pat.chars().collect();
        let mut ps = Parser { p: &p, i: 0, ere: syntax == Syntax::Extended, depth: 0 };
        let node = ps.alt()?;
        if ps.i < p.len() {
            return Err(if ps.at_close() { ReErr::CloseParen } else { ReErr::Paren });
        }
        let mut c = Comp { prog: Vec::new(), sets: Vec::new() };
        c.node(&node)?;
        c.emit(Inst::Match)?;
        Ok(Regex { prog: c.prog, sets: c.sets, icase })
    }

    /// the expression matches somewhere in s
    pub fn is_match(&self, s: &str) -> bool {
        self.find(s).is_some()
    }

    /// the leftmost match (the longest of those starting there): byte offsets
    pub fn find(&self, s: &str) -> Option<(usize, usize)> {
        self.find_at(s, 0)
    }

    /// as find, starting the search at byte `from` (the edges still see the text before it)
    pub fn find_at(&self, s: &str, from: usize) -> Option<(usize, usize)> {
        let chars: Vec<(usize, char)> = s.char_indices().collect();
        let start_k = chars.iter().position(|&(b, _)| b >= from).unwrap_or(chars.len());
        // threads: (pc, start byte) in priority order (their starts never decrease along the
        // list); `seen` holds the position each pc was last added at
        let mut cur: Vec<(usize, usize)> = Vec::new();
        let mut next: Vec<(usize, usize)> = Vec::new();
        let mut seen = vec![usize::MAX; self.prog.len()];
        let mut best: Option<(usize, usize)> = None;
        let byte_at = |k: usize| chars.get(k).map_or(s.len(), |x| x.0);
        let ch = |k: usize| chars.get(k).map(|x| x.1);
        let mut k = start_k;
        loop {
            let prev = if k == 0 { None } else { ch(k - 1) };
            let here = ch(k);
            // a new thread at this position while nothing matched yet
            if best.is_none() {
                self.add(&mut cur, &mut seen, k, 0, byte_at(k), prev, here);
            }
            next.clear();
            for &(pc, st) in cur.iter() {
                let step = match &self.prog[pc] {
                    Inst::Match => {
                        let end = byte_at(k);
                        if best.is_none_or(|(bs, be)| st < bs || (st == bs && end > be)) {
                            best = Some((st, end));
                        }
                        false
                    }
                    Inst::Char(c) => here.is_some_and(|h| h == *c || (self.icase && lower(h) == lower(*c))),
                    Inst::Any => here.is_some_and(|h| h != '\n'),
                    Inst::Set(i) => here.is_some_and(|h| self.sets[*i].holds(h, self.icase)),
                    _ => false,
                };
                if step {
                    self.add(&mut next, &mut seen, k + 1, pc + 1, st, here, ch(k + 1));
                }
            }
            // a thread that started after the best match's start cannot win any more
            if let Some((bs, _)) = best {
                next.retain(|t| t.1 <= bs);
            }
            if k >= chars.len() || (next.is_empty() && best.is_some()) {
                break;
            }
            core::mem::swap(&mut cur, &mut next);
            k += 1;
        }
        best
    }

    /// adds a thread at character `k` (prev / here: the characters around the position),
    /// following jumps, splits and edges
    #[allow(clippy::too_many_arguments)]
    fn add(&self, list: &mut Vec<(usize, usize)>, seen: &mut [usize], k: usize, pc: usize, st: usize, prev: Option<char>, here: Option<char>) {
        if seen[pc] == k {
            return;
        }
        seen[pc] = k;
        match &self.prog[pc] {
            Inst::Jmp(t) => self.add(list, seen, k, *t, st, prev, here),
            Inst::Split(a, b) => {
                self.add(list, seen, k, *a, st, prev, here);
                self.add(list, seen, k, *b, st, prev, here);
            }
            Inst::Edge(e) => {
                let ok = match e {
                    Edge::Bol => prev.is_none() || prev == Some('\n'),
                    Edge::Eol => here.is_none() || here == Some('\n'),
                    Edge::WordStart => !is_word(prev) && is_word(here),
                    Edge::WordEnd => is_word(prev) && !is_word(here),
                    Edge::Word => is_word(prev) != is_word(here),
                    Edge::NotWord => is_word(prev) == is_word(here),
                };
                if ok {
                    self.add(list, seen, k, pc + 1, st, prev, here);
                }
            }
            _ => list.push((pc, st)),
        }
    }
}

// sh -- the shell of hosts and servers (jfn-sh): the state of a session, one grammar for every
// command line, the expansion of words, paths relative to the working directory and Tab
// completion; shrun.rs runs what this parses, coreutils.rs has the classic tools
//
//   Shell                         one session (CliDev::sh): the working directory (/root at the
//                                 start, flash:/ on IOS-like devices), the variables, the
//                                 history, $?, the line that runs and the background jobs
//                                 (shrun.rs), the notes ("[1]+  Done ...") for the next prompt.
//                                 Everything of one shell is here: every session of a device
//                                 has its own (cli/ses.rs); prompt_dir: "~", "~/app"
//   parse(line) -> Result<Vec<AndOr>, String>   the grammar below; Err: bash's message
//   expand_history(line, history) -> Result<Option<String>, String>   !! and !N
//   Engine::sh_abs(ni, path)      a path of the session made absolute (the store resolves .
//                                 and ..); sh_prompt_dir: "~", "~/app", "/srv/ftp"
//   Engine::sh_expand(ni, word)   the fields of a word: ~, $NAME ${NAME} $? $$ $! $0 $#,
//                                 quotes, splitting of unquoted expansions, globs
//   Engine::sh_complete(ni, line) Tab: command words (the list of linux.rs), the objects of ip,
//                                 else paths (a trailing / for directories, the common prefix
//                                 of several matches)
//
// The grammar (a subset of POSIX sh, documented in the README):
//   list      and-or ((";" | "&") and-or)* [";" | "&"]     "&": the and-or runs in the
//                                                           background (shrun.rs)
//   and-or    pipeline (("&&" | "||") pipeline)*
//   pipeline  ["!"] command ("|" command)*
//   command   (NAME=VALUE)* (word | redirection)*
//   redirection  [N]> [N]>> [N]< WORD, [N]>&M, &> &>> WORD   (N, M: 1 or 2; 0 for <)
//   words     '...' as it is; "..." with $ expansion and \ before $ ` " \; \ outside quotes
//             takes the next character as it is; # at the start of a word begins a comment
// No subshells ( ), no command substitution $(...) or `...`, no arithmetic, no functions, if,
// for or while: such characters are ordinary ones.  Globs (* ? [...]) match the device's files
// (base::pattern::glob, component by component, dot files only with a leading dot, sorted by
// bytes as bash does with LC_COLLATE=C); a glob that matches nothing stays as it is.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::catalog::CliStyle;

use crate::kernel::{Engine, Ns};

/// the history a shell keeps
pub const HISTORY_MAX: usize = 1000;
/// the PID of the session's shell (services are 100 + their id, as top and ss -p show them)
pub const SHELL_PID: u32 = 100;

/// a variable: its value, exported (env, the environment of programs)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    pub value: String,
    pub export: bool,
}

/// where a redirection goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ROp {
    /// > (truncates)
    Out,
    /// >>
    Append,
    /// <
    In,
    /// N>&M: the word is M
    Dup,
    /// &> and &>>: stdout and stderr
    Both,
    BothAppend,
}

/// a redirection of a command
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Redir {
    pub fd: u8,
    pub op: ROp,
    /// the target as typed (expanded when the command starts)
    pub word: String,
}

/// a command: its words as typed (assignments first) and its redirections
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Simple {
    pub words: Vec<String>,
    pub redirs: Vec<Redir>,
}

/// commands joined by pipes; "!" turns the status around
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pipeline {
    pub bang: bool,
    pub cmds: Vec<Simple>,
}

/// how a pipeline is joined to the one before it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conn {
    First,
    And,
    Or,
}

/// pipelines joined by && and ||, in the background with "&"; `text` is the source (jobs, ps)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AndOr {
    pub parts: Vec<(Conn, Pipeline)>,
    pub bg: bool,
    pub text: String,
}

/// one shell session
#[derive(Clone, Debug)]
pub struct Shell {
    /// the working directory (canonical: "/root", "flash:/")
    pub cwd: String,
    pub vars: BTreeMap<String, Var>,
    pub history: Vec<String>,
    /// $?
    pub status: i32,
    /// the line that runs in the foreground (shrun.rs)
    pub fg: Option<super::shrun::Run>,
    /// the background jobs (shrun.rs), oldest first
    pub bg: Vec<super::shrun::BgJob>,
    /// lines for the next prompt: the ends of background jobs
    pub notes: Vec<String>,
    /// $!: the PID of the last background job
    pub last_bg: Option<u32>,
    /// the output of the line asks the terminal to page it (less, more)
    pub pager: bool,
    /// a command set the status itself (grep without a match: 1)
    pub code: Option<i32>,
    /// the first error line of the last line's failing command (the startup configuration)
    pub err_line: Option<String>,
}

impl Shell {
    /// a new session of a device with this kind of command line
    pub fn new(style: CliStyle) -> Shell {
        let ios = style == CliStyle::Ios;
        let mut vars = BTreeMap::new();
        if !ios {
            for (k, v) in [("HOME", "/root"), ("LOGNAME", "root"), ("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"), ("PWD", "/root"), ("SHELL", "/bin/sh"), ("TERM", "xterm"), ("USER", "root")] {
                vars.insert(String::from(k), Var { value: String::from(v), export: true });
            }
        }
        Shell {
            cwd: String::from(if ios { "flash:/" } else { "/root" }),
            vars,
            history: Vec::new(),
            status: 0,
            fg: None,
            bg: Vec::new(),
            notes: Vec::new(),
            last_bg: None,
            pager: false,
            code: None,
            err_line: None,
        }
    }
    /// a variable's value ("" when unset)
    pub fn var(&self, name: &str) -> &str {
        self.vars.get(name).map_or("", |v| v.value.as_str())
    }
    /// sets a variable (an exported one stays exported)
    pub fn set_var(&mut self, name: &str, value: &str) {
        let export = self.vars.get(name).is_some_and(|v| v.export);
        self.vars.insert(String::from(name), Var { value: String::from(value), export });
    }
    pub fn home(&self) -> &str {
        match self.var("HOME") {
            "" => "/root",
            h => h,
        }
    }
    /// the working directory as the prompt shows it: "~", "~/app", "/srv/ftp"
    pub fn prompt_dir(&self) -> String {
        let home = self.home();
        if self.cwd == home {
            String::from("~")
        } else if let Some(rest) = self.cwd.strip_prefix(home).filter(|r| r.starts_with('/') && home != "/") {
            format!("~{}", rest)
        } else {
            self.cwd.clone()
        }
    }
    /// what the session holds (keyframe.rs)
    pub fn footprint(&self) -> usize {
        let mut b = self.cwd.len() + self.history.iter().map(|h| h.len() + 24).sum::<usize>() + self.notes.iter().map(|n| n.len() + 24).sum::<usize>();
        b += self.vars.iter().map(|(k, v)| k.len() + v.value.len() + 64).sum::<usize>();
        b += self.fg.as_ref().map_or(0, |r| r.footprint());
        b += self.bg.iter().map(|j| j.run.footprint() + j.text.len() + 128).sum::<usize>();
        b
    }
}

/// a name of a variable
pub fn is_name(s: &str) -> bool {
    let mut it = s.chars();
    it.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && it.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// "NAME=value": the name and the value as typed
pub fn assignment(w: &str) -> Option<(&str, &str)> {
    let (n, v) = w.split_once('=')?;
    if is_name(n) { Some((n, v)) } else { None }
}

// ---------------------------------------------------------------- the grammar

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    /// ; & && || |
    Op(&'static str),
    Redir(u8, ROp),
}

fn syntax(t: &str) -> String {
    format!("jfn-sh: syntax error near unexpected token `{}'\n", t)
}

/// the tokens of a line with the byte offset each one starts at (comments left out)
fn lex(line: &str) -> Result<Vec<(usize, Tok)>, String> {
    let b: Vec<(usize, char)> = line.char_indices().collect();
    let mut out: Vec<(usize, Tok)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let (at, c) = b[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let next = b.get(i + 1).map(|x| x.1);
        match c {
            '#' => break,
            ';' => {
                out.push((at, Tok::Op(";")));
                i += 1;
                continue;
            }
            '&' if next == Some('&') => {
                out.push((at, Tok::Op("&&")));
                i += 2;
                continue;
            }
            '&' if next == Some('>') => {
                let app = b.get(i + 2).map(|x| x.1) == Some('>');
                out.push((at, Tok::Redir(1, if app { ROp::BothAppend } else { ROp::Both })));
                i += if app { 3 } else { 2 };
                continue;
            }
            '&' => {
                out.push((at, Tok::Op("&")));
                i += 1;
                continue;
            }
            '|' if next == Some('|') => {
                out.push((at, Tok::Op("||")));
                i += 2;
                continue;
            }
            '|' => {
                out.push((at, Tok::Op("|")));
                i += 1;
                continue;
            }
            '>' | '<' => {
                let (op, len) = redir_op(c, next);
                out.push((at, Tok::Redir(if c == '<' { 0 } else { 1 }, op)));
                i += len;
                continue;
            }
            _ => {}
        }
        // a word up to an unquoted blank or operator
        let start = i;
        let mut q: Option<char> = None;
        while i < b.len() {
            let ch = b[i].1;
            match q {
                Some('\'') => {
                    if ch == '\'' {
                        q = None;
                    }
                }
                Some(_) => {
                    if ch == '\\' {
                        i += 1;
                    } else if ch == '"' {
                        q = None;
                    }
                }
                None => {
                    if ch == '\\' {
                        i += 1;
                    } else if ch == '\'' || ch == '"' {
                        q = Some(ch);
                    } else if ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '<' | '>') {
                        break;
                    }
                }
            }
            i += 1;
        }
        if let Some(qc) = q {
            return Err(format!("jfn-sh: unexpected EOF while looking for matching `{}'\n", qc));
        }
        let end = b.get(i).map_or(line.len(), |x| x.0).min(line.len());
        let word = &line[at..end];
        let _ = start;
        // "2>" and "2>&1": the digits of a word right before > or <
        if let Some(&(_, nc)) = b.get(i)
            && (nc == '>' || nc == '<')
            && word.len() == 1
            && matches!(word, "0" | "1" | "2")
        {
            let fd = word.as_bytes()[0] - b'0';
            let (op, len) = redir_op(nc, b.get(i + 1).map(|x| x.1));
            out.push((at, Tok::Redir(fd, op)));
            i += len;
            continue;
        }
        out.push((at, Tok::Word(String::from(word))));
    }
    Ok(out)
}

/// the redirection at '>' or '<' and how many characters it takes
fn redir_op(c: char, next: Option<char>) -> (ROp, usize) {
    match (c, next) {
        ('>', Some('>')) => (ROp::Append, 2),
        ('>', Some('&')) | ('<', Some('&')) => (ROp::Dup, 2),
        ('>', Some('|')) => (ROp::Out, 2),
        ('>', _) => (ROp::Out, 1),
        _ => (ROp::In, 1),
    }
}

/// the grammar of the header: and-or lists with their background marks
pub fn parse(line: &str) -> Result<Vec<AndOr>, String> {
    let toks = lex(line)?;
    let mut list: Vec<AndOr> = Vec::new();
    let mut cur = AndOr { parts: Vec::new(), bg: false, text: String::new() };
    let mut pipe = Pipeline::default();
    let mut cmd = Simple::default();
    let mut conn = Conn::First;
    let mut start: Option<usize> = None;
    let mut i = 0;
    // the end of the current command: Err when it is empty
    let finish_cmd = |cmd: &mut Simple, pipe: &mut Pipeline, tok: &str| -> Result<(), String> {
        if cmd.words.is_empty() && cmd.redirs.is_empty() {
            return Err(syntax(tok));
        }
        pipe.cmds.push(core::mem::take(cmd));
        Ok(())
    };
    while i < toks.len() {
        let (at, t) = toks[i].clone();
        start.get_or_insert(at);
        match t {
            Tok::Word(w) => {
                if w == "!" && cmd.words.is_empty() && cmd.redirs.is_empty() && pipe.cmds.is_empty() && !pipe.bang {
                    pipe.bang = true;
                } else {
                    cmd.words.push(w);
                }
            }
            Tok::Redir(fd, op) => {
                let Some((_, Tok::Word(w))) = toks.get(i + 1).cloned() else {
                    return Err(syntax(&match toks.get(i + 1) {
                        Some((_, Tok::Op(o))) => String::from(*o),
                        Some((_, Tok::Redir(..))) => String::from(">"),
                        _ => String::from("newline"),
                    }));
                };
                if op == ROp::Dup && !matches!(w.as_str(), "0" | "1" | "2") {
                    return Err(format!("jfn-sh: {}: ambiguous redirect\n", w));
                }
                cmd.redirs.push(Redir { fd, op, word: w });
                i += 1;
            }
            Tok::Op("|") => {
                finish_cmd(&mut cmd, &mut pipe, "|")?;
            }
            Tok::Op(o) => {
                // ; & && || end a pipeline
                if cmd.words.is_empty() && cmd.redirs.is_empty() && pipe.cmds.is_empty() {
                    return Err(syntax(o));
                }
                finish_cmd(&mut cmd, &mut pipe, o)?;
                cur.parts.push((conn, core::mem::take(&mut pipe)));
                match o {
                    "&&" => conn = Conn::And,
                    "||" => conn = Conn::Or,
                    _ => {
                        cur.bg = o == "&";
                        let from = start.unwrap_or(at);
                        cur.text = String::from(line[from..at].trim());
                        list.push(core::mem::replace(&mut cur, AndOr { parts: Vec::new(), bg: false, text: String::new() }));
                        conn = Conn::First;
                        start = None;
                    }
                }
            }
        }
        i += 1;
    }
    if !cmd.words.is_empty() || !cmd.redirs.is_empty() {
        pipe.cmds.push(cmd);
    }
    if !pipe.cmds.is_empty() {
        cur.parts.push((conn, pipe));
    } else if conn != Conn::First || pipe.bang {
        // "a &&" or "a |": the rest of the command is missing
        return Err(String::from("jfn-sh: syntax error: unexpected end of file\n"));
    }
    if !cur.parts.is_empty() {
        let from = start.unwrap_or(0);
        let end = line.find(" #").filter(|&e| e >= from).unwrap_or(line.len());
        cur.text = String::from(line[from..end].trim());
        list.push(cur);
    }
    Ok(list)
}

/// history expansion: "!!" the last line, "!N" line N, "!-N" the Nth line back (outside
/// single quotes); None when the line has none
pub fn expand_history(line: &str, history: &[String]) -> Result<Option<String>, String> {
    let b: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut any = false;
    let mut q: Option<char> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match q {
            Some('\'') => {
                if c == '\'' {
                    q = None;
                }
                out.push(c);
                i += 1;
                continue;
            }
            Some(_) if c == '"' => q = None,
            None if c == '\'' || c == '"' => q = Some(c),
            None if c == '\\' && i + 1 < b.len() => {
                out.push(c);
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            _ => {}
        }
        if c == '!' && i + 1 < b.len() {
            let n = b[i + 1];
            let (event, len): (Option<usize>, usize) = if n == '!' {
                (history.len().checked_sub(1), 2)
            } else if n.is_ascii_digit() || (n == '-' && b.get(i + 2).is_some_and(|d| d.is_ascii_digit())) {
                let mut j = i + 1 + usize::from(n == '-');
                let mut v = 0usize;
                while j < b.len() && b[j].is_ascii_digit() {
                    v = v.saturating_mul(10).saturating_add(b[j] as usize - '0' as usize);
                    j += 1;
                }
                let ev = if n == '-' { history.len().checked_sub(v) } else { v.checked_sub(1).filter(|&k| k < history.len()) };
                (ev, j - i)
            } else {
                out.push(c);
                i += 1;
                continue;
            };
            let typed: String = b[i..i + len].iter().collect();
            match event.and_then(|k| history.get(k)) {
                Some(h) => out.push_str(h),
                None => return Err(format!("jfn-sh: {}: event not found\n", typed)),
            }
            any = true;
            i += len;
            continue;
        }
        out.push(c);
        i += 1;
    }
    Ok(if any { Some(out) } else { None })
}

// ---------------------------------------------------------------- expansion

/// a character of an expanded word: quoted ones are no globs and do not split
#[derive(Clone, Copy)]
struct Ch {
    c: char,
    quoted: bool,
    /// it came from an unquoted expansion (fields split there)
    split: bool,
}

impl Engine {
    /// the shell of node `ni`
    pub(crate) fn sh(&self, ni: usize) -> Option<&Shell> {
        self.cdev(ni).map(|c| &c.sh)
    }
    pub(crate) fn sh_mut(&mut self, ni: usize) -> Option<&mut Shell> {
        self.cdev_mut(ni).map(|c| &mut c.sh)
    }

    /// a path of the session made absolute: relative to the working directory
    pub(crate) fn sh_abs(&self, ni: usize, p: &str) -> String {
        let cwd = self.sh(ni).map_or("/root", |s| s.cwd.as_str());
        if p.starts_with('/') {
            String::from(p)
        } else if cwd == "/" {
            format!("/{}", p)
        } else {
            format!("{}/{}", cwd, p)
        }
    }

    /// the working directory as the prompt shows it: "~", "~/app", "/srv/ftp"
    pub(crate) fn sh_prompt_dir(&self, ni: usize) -> String {
        match self.sh(ni) {
            Some(s) => s.prompt_dir(),
            None => String::from("~"),
        }
    }

    /// the value of a parameter ($NAME, $?, $$, $!, $0, $#, $1 ..)
    fn sh_param(&self, ni: usize, name: &str) -> String {
        let Some(s) = self.sh(ni) else { return String::new() };
        match name {
            "?" => format!("{}", s.status),
            // the shell of the session (cli/ses.rs; the console's is SHELL_PID)
            "$" => format!("{}", self.ses_pid(ni)),
            "!" => s.last_bg.map(|p| format!("{}", p)).unwrap_or_default(),
            "0" => String::from("jfn-sh"),
            "#" => String::from("0"),
            "HOSTNAME" if !s.vars.contains_key("HOSTNAME") => self.host(ni),
            n => String::from(s.var(n)),
        }
    }

    /// the characters of a word with quotes removed and parameters expanded
    fn sh_chars(&self, ni: usize, raw: &str) -> (Vec<Ch>, bool) {
        let b: Vec<char> = raw.chars().collect();
        let mut v: Vec<Ch> = Vec::new();
        let mut quoted_any = false;
        let mut q: Option<char> = None;
        let mut i = 0;
        // ~ at the start: the home directory ("~root" too)
        if b.first() == Some(&'~') {
            let end = b.iter().position(|&c| c == '/').unwrap_or(b.len());
            let user: String = b[1..end].iter().collect();
            let home = match user.as_str() {
                "" | "root" => Some(String::from(self.sh(ni).map_or("/root", |s| s.home()))),
                "user" => Some(String::from("/home/user")),
                _ => None,
            };
            if let Some(h) = home {
                v.extend(h.chars().map(|c| Ch { c, quoted: true, split: false }));
                i = end;
            }
        }
        while i < b.len() {
            let c = b[i];
            match q {
                Some('\'') => {
                    if c == '\'' {
                        q = None;
                    } else {
                        v.push(Ch { c, quoted: true, split: false });
                    }
                    i += 1;
                    continue;
                }
                Some(_) => {
                    if c == '"' {
                        q = None;
                        i += 1;
                        continue;
                    }
                    if c == '\\' && i + 1 < b.len() && matches!(b[i + 1], '$' | '`' | '"' | '\\') {
                        v.push(Ch { c: b[i + 1], quoted: true, split: false });
                        i += 2;
                        continue;
                    }
                }
                None => {
                    if c == '\'' || c == '"' {
                        q = Some(c);
                        quoted_any = true;
                        i += 1;
                        continue;
                    }
                    if c == '\\' {
                        if let Some(&n) = b.get(i + 1) {
                            v.push(Ch { c: n, quoted: true, split: false });
                        }
                        quoted_any = true;
                        i += 2;
                        continue;
                    }
                }
            }
            if c == '$' && i + 1 < b.len() {
                let inq = q.is_some();
                let (name, len): (Option<String>, usize) = if b[i + 1] == '{' {
                    match b[i + 2..].iter().position(|&x| x == '}') {
                        Some(e) => (Some(b[i + 2..i + 2 + e].iter().collect()), e + 3),
                        None => (None, 1),
                    }
                } else if matches!(b[i + 1], '?' | '$' | '!' | '#') || b[i + 1].is_ascii_digit() {
                    (Some(String::from(b[i + 1])), 2)
                } else if b[i + 1].is_ascii_alphabetic() || b[i + 1] == '_' {
                    let mut j = i + 1;
                    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == '_') {
                        j += 1;
                    }
                    (Some(b[i + 1..j].iter().collect()), j - i)
                } else {
                    (None, 1)
                };
                if let Some(n) = name {
                    let val = self.sh_param(ni, &n);
                    v.extend(val.chars().map(|c| Ch { c, quoted: inq, split: !inq }));
                    i += len;
                    continue;
                }
            }
            v.push(Ch { c, quoted: q.is_some(), split: false });
            i += 1;
        }
        (v, quoted_any)
    }

    /// the fields of a word: expanded, split where an unquoted expansion had blanks, globs
    /// replaced by the paths they match (sorted)
    pub(crate) fn sh_expand(&self, ni: usize, raw: &str) -> Vec<String> {
        let (chars, quoted) = self.sh_chars(ni, raw);
        // split on blanks that came from unquoted expansions
        let mut fields: Vec<Vec<Ch>> = alloc::vec![Vec::new()];
        for ch in chars {
            if ch.split && ch.c.is_whitespace() {
                if fields.last().is_some_and(|f| !f.is_empty()) {
                    fields.push(Vec::new());
                }
                continue;
            }
            if let Some(f) = fields.last_mut() {
                f.push(ch);
            }
        }
        if fields.last().is_some_and(|f| f.is_empty()) {
            fields.pop();
        }
        if fields.is_empty() {
            // "" stays an empty argument, an expansion to nothing goes
            return if quoted { alloc::vec![String::new()] } else { Vec::new() };
        }
        let mut out = Vec::new();
        for f in fields {
            let text: String = f.iter().map(|c| c.c).collect();
            if f.iter().any(|c| !c.quoted && matches!(c.c, '*' | '?' | '[')) {
                // the pattern with quoted characters escaped
                let mut pat = String::new();
                for c in &f {
                    if c.quoted && matches!(c.c, '*' | '?' | '[' | ']' | '\\') {
                        pat.push('\\');
                    }
                    pat.push(c.c);
                }
                let hits = self.sh_glob(ni, &pat);
                if hits.is_empty() {
                    out.push(text);
                } else {
                    out.extend(hits);
                }
                continue;
            }
            out.push(text);
        }
        out
    }

    /// the names in a directory of the session (canonical path): the store's, the made files
    /// of /etc (files.rs); with their kind (true: a directory)
    pub(crate) fn sh_entries(&self, ni: usize, dir: &str) -> Vec<(String, bool)> {
        match self.ls_entries(ni, dir) {
            Ok(v) => v.into_iter().map(|(n, s)| (n, s.dir)).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// the paths a glob matches (sorted by bytes; as typed: relative ones stay relative)
    fn sh_glob(&self, ni: usize, pat: &str) -> Vec<String> {
        let abs = pat.starts_with('/');
        let parts: Vec<&str> = pat.split('/').filter(|p| !p.is_empty()).collect();
        // (the path as shown, the canonical path)
        let base = if abs { String::from("/") } else { self.sh(ni).map_or(String::from("/root"), |s| s.cwd.clone()) };
        let mut cur: Vec<(String, String)> = alloc::vec![(String::new(), base)];
        for (k, part) in parts.iter().enumerate() {
            let last = k + 1 == parts.len();
            let mut next = Vec::new();
            for (shown, canon) in &cur {
                let join = |a: &str, b: &str| if a.is_empty() { String::from(b) } else if a.ends_with('/') { format!("{}{}", a, b) } else { format!("{}/{}", a, b) };
                let prefix = if shown.is_empty() && abs { String::from("/") } else { shown.clone() };
                if !base::pattern::has_glob(part) {
                    // a plain component: it must exist
                    let lit = part.replace('\\', "");
                    let c = join(canon, &lit);
                    if lit == "." || lit == ".." || self.sh_exists(ni, &c) {
                        next.push((join(&prefix, &lit), c));
                    }
                    continue;
                }
                let dot = part.starts_with('.') || part.starts_with("\\.");
                for (name, dir) in self.sh_entries(ni, canon) {
                    if name.starts_with('.') && !dot {
                        continue;
                    }
                    if !last && !dir {
                        continue;
                    }
                    if base::pattern::glob(part, &name) {
                        next.push((join(&prefix, &name), join(canon, &name)));
                    }
                }
            }
            cur = next;
            if cur.is_empty() {
                break;
            }
        }
        let mut v: Vec<String> = cur.into_iter().map(|(s, _)| s).filter(|s| !s.is_empty()).collect();
        if pat.ends_with('/') {
            v = v.into_iter().map(|s| format!("{}/", s)).collect();
        }
        v.sort();
        v
    }

    /// something is at a canonical path (the made files of /etc too)
    pub(crate) fn sh_exists(&self, ni: usize, p: &str) -> bool {
        let Some(st) = self.store(ni) else { return false };
        let Ok(c) = st.norm(p) else { return false };
        st.exists(&c) || self.cat_etc(ni, &c).is_some()
    }

    // ------------------------------------------------------------ Tab

    /// Tab of the shell: (the completed line, the choices); None when nothing completes
    pub(crate) fn sh_complete(&self, ni: usize, line: &str, words: &[&str]) -> (Option<String>, Vec<String>) {
        // the last word starts after the last unquoted blank or operator
        let mut start = 0;
        let mut first = true;
        let mut q: Option<char> = None;
        let mut esc = false;
        let mut seen_word = false;
        for (i, c) in line.char_indices() {
            if esc {
                esc = false;
                continue;
            }
            match q {
                Some(x) if c == x => q = None,
                Some(_) => {}
                None if c == '\\' => esc = true,
                None if c == '\'' || c == '"' => q = Some(c),
                None if c.is_whitespace() => {
                    if seen_word && i > start {
                        first = false;
                    }
                    start = i + c.len_utf8();
                }
                None if matches!(c, '|' | ';' | '&') => {
                    first = true;
                    seen_word = false;
                    start = i + 1;
                    continue;
                }
                None if matches!(c, '<' | '>') => {
                    first = false;
                    start = i + 1;
                    continue;
                }
                None => {}
            }
            if !c.is_whitespace() {
                seen_word = true;
            }
        }
        let word = &line[start..];
        // an empty line completes nothing (as before)
        if line.trim().is_empty() {
            return (None, Vec::new());
        }
        if first && !word.contains('/') {
            let mut choices: Vec<String> = words.iter().filter(|c| c.starts_with(word)).map(|c| String::from(*c)).collect();
            choices.sort();
            choices.dedup();
            let out = if choices.len() == 1 { Some(format!("{}{} ", &line[..start], choices[0])) } else { None };
            return (out, choices);
        }
        // a path: the directory typed and the start of a name in it
        let unq: String = {
            let mut s = String::new();
            let mut e = false;
            for c in word.chars() {
                if e {
                    s.push(c);
                    e = false;
                } else if c == '\\' {
                    e = true;
                } else if c != '\'' && c != '"' {
                    s.push(c);
                }
            }
            s
        };
        let (dir_typed, base) = match unq.rfind('/') {
            Some(i) => (&unq[..=i], &unq[i + 1..]),
            None => ("", unq.as_str()),
        };
        let dir_abs = if dir_typed.is_empty() {
            self.sh(ni).map_or(String::from("/root"), |s| s.cwd.clone())
        } else if let Some(r) = dir_typed.strip_prefix('~') {
            format!("{}{}", self.sh(ni).map_or("/root", |s| s.home()), r)
        } else {
            self.sh_abs(ni, dir_typed)
        };
        let Some(canon) = self.store(ni).and_then(|s| s.norm(&dir_abs).ok()) else { return (None, Vec::new()) };
        let mut hits: Vec<(String, bool)> = self.sh_entries(ni, &canon).into_iter().filter(|(n, _)| n.starts_with(base) && (base.starts_with('.') || !n.starts_with('.'))).collect();
        hits.sort();
        if hits.is_empty() {
            return (None, Vec::new());
        }
        let esc_name = |n: &str| -> String {
            let mut s = String::new();
            for c in n.chars() {
                if c.is_whitespace() || matches!(c, '\'' | '"' | '\\' | '|' | '&' | ';' | '<' | '>' | '$' | '*' | '?' | '[') {
                    s.push('\\');
                }
                s.push(c);
            }
            s
        };
        let head = format!("{}{}", &line[..start], esc_name(dir_typed).replace("\\~", "~"));
        if hits.len() == 1 {
            let (n, dir) = &hits[0];
            return (Some(format!("{}{}{}", head, esc_name(n), if *dir { "/" } else { " " })), alloc::vec![if *dir { format!("{}/", n) } else { n.clone() }]);
        }
        let choices: Vec<String> = hits.iter().map(|(n, d)| if *d { format!("{}/", n) } else { n.clone() }).collect();
        let mut common: String = hits[0].0.clone();
        for (n, _) in &hits[1..] {
            let l = common.chars().zip(n.chars()).take_while(|(a, b)| a == b).count();
            common = common.chars().take(l).collect();
        }
        if common.len() > base.len() {
            return (Some(format!("{}{}", head, esc_name(&common))), choices);
        }
        (None, choices)
    }
}

/// a duration of sleep: "1", "0.5", "2s", "3m", "1h" (None: not one)
pub fn sleep_ns(s: &str) -> Option<Ns> {
    let (num, mul) = match s.as_bytes().last()? {
        b's' => (&s[..s.len() - 1], 1u64),
        b'm' => (&s[..s.len() - 1], 60),
        b'h' => (&s[..s.len() - 1], 3600),
        b'd' => (&s[..s.len() - 1], 86_400),
        _ => (s, 1),
    };
    super::linux::secs(num)?.checked_mul(mul)
}

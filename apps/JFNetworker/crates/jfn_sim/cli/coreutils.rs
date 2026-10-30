// coreutils -- the classic tools of the shell, as their GNU / busybox counterparts behave where
// one can see it (the options people use, exit codes, the form of the messages; help texts and
// man pages are our own German words: man.rs)
//
//   cd, pwd, echo, printf, cat, mv, head, tail, grep, find, wc, sort, uniq, cut, tree, du, df,
//   less, more, file, stat, basename, dirname, date, seq, tee, test / [, xargs, env, export,
//   unset, history, which, type, help, man
//
// A tool reads its FILE arguments (relative to the working directory, the made files of /etc
// too), or what a pipe or < gives it (Engine::sh_take_stdin); a tool without either reads
// nothing (the console is not a terminal a program could read).  Output goes through out(),
// errors through err_out() (2>).  grep uses base::pattern (basic and extended expressions
// without back-references), find -name and the shell's globs its wildcards.  Sizes: a file
// takes whole 4 KiB blocks, a directory one (as ls -l counts them); df gives the store's
// capacity.  date reads the device's clock (virtual time).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::pattern::{Regex, Syntax};

use super::sh::{self, Var};
use super::tools::{echo_bytes, printf_bytes};
use crate::kernel::Engine;
use crate::os::EPOCH_UNIX_NS;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// "-n5", "-n 5", "-5", "--lines=5": the value of a count option and how many words it took
fn count_opt(a: &str, next: Option<&str>, short: char, long: &str) -> Option<(String, usize)> {
    if let Some(v) = a.strip_prefix(&format!("--{}=", long)) {
        return Some((String::from(v), 1));
    }
    let rest = a.strip_prefix('-')?.strip_prefix(short)?;
    if rest.is_empty() {
        return next.map(|n| (String::from(n), 2));
    }
    Some((String::from(rest), 1))
}

/// "1.5K" (du -h, df -h: powers of 1024, rounded up, one decimal below 10)
fn human(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{}", bytes);
    }
    let units = ['K', 'M', 'G', 'T', 'P'];
    let mut v = bytes as u128;
    let mut div = 1024u128;
    let mut k = 0;
    while v >= div * 1024 && k + 1 < units.len() {
        div *= 1024;
        k += 1;
    }
    // tenths, rounded up
    let tenths = (v * 10).div_ceil(div);
    if tenths < 100 {
        return format!("{}.{}{}", tenths / 10, tenths % 10, units[k]);
    }
    v = v.div_ceil(div);
    format!("{}{}", v, units[k])
}

/// the blocks of 1 KiB a file of `size` bytes takes (whole 4 KiB blocks)
fn kib_of(size: u64) -> u64 {
    size.div_ceil(4096) * 4
}

/// a line split into its lines (the last one without its line end)
fn lines_of(data: &[u8]) -> Vec<String> {
    let t = String::from_utf8_lossy(data);
    let mut v: Vec<String> = t.split('\n').map(String::from).collect();
    if v.last().is_some_and(|l| l.is_empty()) {
        v.pop();
    }
    v
}

/// the GNU numeric value at the start of a line (sort -n): leading blanks, a sign, digits and
/// a fraction; none is 0
fn num_key(s: &str) -> f64 {
    let t = s.trim_start();
    let mut end = 0;
    let b = t.as_bytes();
    if end < b.len() && (b[end] == b'-' || b[end] == b'+') {
        end += 1;
    }
    while end < b.len() && b[end].is_ascii_digit() {
        end += 1;
    }
    if end < b.len() && b[end] == b'.' {
        end += 1;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
    }
    t[..end].parse::<f64>().unwrap_or(0.0)
}

impl Engine {
    /// a usage error of a tool: the message, "Try 'CMD --help' ..." and status 1 (GNU)
    fn tool_usage(&mut self, ni: usize, cmd: &str, msg: &str) -> Result<(), String> {
        self.sh_err(ni, &format!("{}: {}\nTry '{} --help' for more information.\n", cmd, msg, cmd))
    }

    /// --help of a tool
    fn tool_help(&mut self, ni: usize, cmd: &str) -> Result<(), String> {
        let u = super::man::usage(cmd);
        self.out(ni, &u);
        Ok(())
    }

    /// the inputs of a filter: each FILE ("-": the pipe), else the pipe; an input that cannot
    /// be read is reported ("CMD: FILE: why") and left out; the bool: one failed
    fn sh_inputs(&mut self, ni: usize, cmd: &str, files: &[String]) -> (Vec<(String, Vec<u8>)>, bool) {
        let mut v = Vec::new();
        let mut failed = false;
        if files.is_empty() {
            let d = self.sh_take_stdin(ni).unwrap_or_default();
            v.push((String::from("-"), d));
            return (v, false);
        }
        for f in files {
            if f == "-" {
                let d = self.sh_take_stdin(ni).unwrap_or_default();
                v.push((String::from("-"), d));
                continue;
            }
            let abs = self.sh_abs(ni, f);
            match self.sh_read(ni, &abs) {
                Ok(d) => v.push((f.clone(), d)),
                Err(e) => {
                    self.err_out(ni, &format!("{}: {}: {}\n", cmd, f, e));
                    failed = true;
                }
            }
        }
        (v, failed)
    }

    /// the canonical path of a path of the session (None: not even a path)
    fn sh_canon(&self, ni: usize, p: &str) -> Option<String> {
        self.store(ni)?.norm(&self.sh_abs(ni, p)).ok()
    }

    /// what is at a path of the session: Some(true) a directory, Some(false) a file
    fn sh_kind(&self, ni: usize, p: &str) -> Option<bool> {
        let c = self.sh_canon(ni, p)?;
        if self.cat_etc(ni, &c).is_some() {
            return Some(false);
        }
        self.store(ni)?.stat(&c).map(|s| s.dir)
    }

    // ------------------------------------------------------------ cd, pwd, variables

    pub(crate) fn sh_cd(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let ops: Vec<&str> = args.iter().copied().filter(|a| *a != "-L" && *a != "-P").collect();
        if ops.len() > 1 {
            return self.sh_err(ni, "jfn-sh: cd: too many arguments\n");
        }
        let mut show = false;
        let target = match ops.first() {
            None => match self.sh(ni).map(|s| String::from(s.var("HOME"))) {
                Some(h) if !h.is_empty() => h,
                _ => return self.sh_err(ni, "jfn-sh: cd: HOME not set\n"),
            },
            Some(&"-") => match self.sh(ni).map(|s| String::from(s.var("OLDPWD"))) {
                Some(o) if !o.is_empty() => {
                    show = true;
                    o
                }
                _ => return self.sh_err(ni, "jfn-sh: cd: OLDPWD not set\n"),
            },
            Some(&"") => return Ok(()),
            Some(p) => String::from(*p),
        };
        let Some(c) = self.sh_canon(ni, &target) else {
            return self.sh_err(ni, &format!("jfn-sh: cd: {}: No such file or directory\n", target));
        };
        match self.store(ni).and_then(|s| s.stat(&c)) {
            Some(s) if s.dir => {}
            Some(_) => return self.sh_err(ni, &format!("jfn-sh: cd: {}: Not a directory\n", target)),
            None if self.cat_etc(ni, &c).is_some() => return self.sh_err(ni, &format!("jfn-sh: cd: {}: Not a directory\n", target)),
            None => return self.sh_err(ni, &format!("jfn-sh: cd: {}: No such file or directory\n", target)),
        }
        if let Some(s) = self.sh_mut(ni) {
            let old = core::mem::replace(&mut s.cwd, c.clone());
            s.vars.insert(String::from("OLDPWD"), Var { value: old, export: true });
            s.vars.insert(String::from("PWD"), Var { value: c.clone(), export: true });
        }
        if show {
            self.out(ni, &format!("{}\n", c));
        }
        Ok(())
    }

    pub(crate) fn sh_pwd(&mut self, ni: usize) -> Result<(), String> {
        let c = self.sh(ni).map(|s| s.cwd.clone()).unwrap_or_default();
        self.out(ni, &format!("{}\n", c));
        Ok(())
    }

    pub(crate) fn sh_echo(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let a: Vec<String> = args.iter().map(|s| String::from(*s)).collect();
        let b = echo_bytes(&a);
        self.out(ni, &String::from_utf8_lossy(&b));
        Ok(())
    }

    pub(crate) fn sh_printf(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let a: Vec<String> = args.iter().map(|s| String::from(*s)).collect();
        match printf_bytes(&a) {
            Ok(d) => {
                self.out(ni, &String::from_utf8_lossy(&d));
                Ok(())
            }
            Err(e) => self.sh_err(ni, &e),
        }
    }

    pub(crate) fn sh_env(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        // env NAME=VALUE ... COMMAND: the command with these variables
        let mut k = 0;
        let mut set: Vec<(String, String)> = Vec::new();
        while let Some(a) = args.get(k) {
            match sh::assignment(a) {
                Some((n, v)) => set.push((String::from(n), String::from(v))),
                None => break,
            }
            k += 1;
        }
        if k < args.len() {
            let mut undo = Vec::new();
            if let Some(s) = self.sh_mut(ni) {
                for (n, v) in set {
                    undo.push((n.clone(), s.vars.get(&n).cloned()));
                    s.vars.insert(n, Var { value: v, export: true });
                }
            }
            let argv: Vec<String> = args[k..].iter().map(|s| String::from(*s)).collect();
            let r = self.linux_cmd(ni, &argv);
            if let Some(s) = self.sh_mut(ni) {
                for (n, old) in undo.into_iter().rev() {
                    match old {
                        Some(v) => {
                            s.vars.insert(n, v);
                        }
                        None => {
                            s.vars.remove(&n);
                        }
                    }
                }
            }
            return r;
        }
        let mut out = String::new();
        if let Some(s) = self.sh(ni) {
            let host = self.host(ni);
            let mut all: Vec<(String, String)> = s.vars.iter().filter(|(_, v)| v.export).map(|(k, v)| (k.clone(), v.value.clone())).collect();
            if !s.vars.contains_key("HOSTNAME") {
                all.push((String::from("HOSTNAME"), host));
            }
            all.extend(set);
            all.sort();
            for (k, v) in all {
                out.push_str(&format!("{}={}\n", k, v));
            }
        }
        self.out(ni, &out);
        Ok(())
    }

    pub(crate) fn sh_export(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let names: Vec<&str> = args.iter().copied().filter(|a| *a != "-p").collect();
        if names.is_empty() {
            let mut out = String::new();
            if let Some(s) = self.sh(ni) {
                for (k, v) in s.vars.iter().filter(|(_, v)| v.export) {
                    out.push_str(&format!("declare -x {}=\"{}\"\n", k, v.value.replace('\\', "\\\\").replace('"', "\\\"")));
                }
            }
            self.out(ni, &out);
            return Ok(());
        }
        let mut err = None;
        for a in names {
            let (n, v) = match a.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (a, None),
            };
            if !sh::is_name(n) {
                let m = format!("jfn-sh: export: `{}': not a valid identifier\n", a);
                self.err_out(ni, &m);
                err.get_or_insert(m);
                continue;
            }
            if let Some(s) = self.sh_mut(ni) {
                match v {
                    Some(v) => {
                        s.vars.insert(String::from(n), Var { value: String::from(v), export: true });
                    }
                    None => {
                        if let Some(x) = s.vars.get_mut(n) {
                            x.export = true;
                        }
                    }
                }
            }
        }
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_unset(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        if let Some(s) = self.sh_mut(ni) {
            for a in args.iter().filter(|a| !a.starts_with('-')) {
                s.vars.remove(*a);
            }
        }
        Ok(())
    }

    pub(crate) fn sh_history(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        if args.first() == Some(&"-c") {
            if let Some(s) = self.sh_mut(ni) {
                s.history.clear();
            }
            return Ok(());
        }
        let n = match args.first() {
            None => usize::MAX,
            Some(a) => match a.parse::<usize>() {
                Ok(n) => n,
                Err(_) => return self.sh_err(ni, &format!("jfn-sh: history: {}: numeric argument required\n", a)),
            },
        };
        let mut out = String::new();
        if let Some(s) = self.sh(ni) {
            let from = s.history.len().saturating_sub(n);
            for (i, h) in s.history.iter().enumerate().skip(from) {
                out.push_str(&format!("{:>5}  {}\n", i + 1, h));
            }
        }
        self.out(ni, &out);
        Ok(())
    }

    /// which NAME ...: where a program lives (shell builtins are none)
    pub(crate) fn sh_which(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut out = String::new();
        let mut missing = false;
        for a in args.iter().filter(|a| !a.starts_with('-')) {
            match super::man::page_of(a).filter(|p| !p.path.is_empty()) {
                Some(p) => {
                    let path = if p.name == *a { String::from(p.path) } else { format!("{}/{}", p.path.rsplit_once('/').map_or("", |x| x.0), a) };
                    out.push_str(&format!("{}\n", path));
                }
                None => missing = true,
            }
        }
        self.out(ni, &out);
        if missing { Err(String::new()) } else { Ok(()) }
    }

    /// type NAME ...: a builtin of the shell or where the program lives
    pub(crate) fn sh_type(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut err = None;
        for a in args {
            match super::man::page_of(a) {
                Some(p) if p.path.is_empty() => self.out(ni, &format!("{} is a shell builtin\n", a)),
                Some(p) => {
                    let path = if p.name == *a { String::from(p.path) } else { format!("{}/{}", p.path.rsplit_once('/').map_or("", |x| x.0), a) };
                    self.out(ni, &format!("{} is {}\n", a, path));
                }
                None => {
                    let m = format!("jfn-sh: type: {}: not found\n", a);
                    self.err_out(ni, &m);
                    err.get_or_insert(m);
                }
            }
        }
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_help(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        match args.first() {
            None => {
                let h = super::man::help();
                self.out(ni, &h);
                Ok(())
            }
            Some(a) => match super::man::page_of(a) {
                Some(_) => {
                    let u = super::man::usage(a);
                    self.out(ni, &u);
                    Ok(())
                }
                None => self.sh_err(ni, &format!("jfn-sh: help: no help topics match `{}'.\n", a)),
            },
        }
    }

    pub(crate) fn sh_man(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let Some(a) = args.iter().find(|a| !a.starts_with('-')) else {
            return self.sh_err(ni, "man: welche Seite? Aufruf: man BEFEHL\n");
        };
        match super::man::man(a) {
            Some(p) => {
                self.out(ni, &p);
                self.sh_pager(ni);
                Ok(())
            }
            None => {
                self.sh_set_code(ni, 16);
                self.sh_err(ni, &format!("man: zu {} gibt es keine Seite\n", a))
            }
        }
    }

    /// the terminal pages the line's output (less, more, man) when it goes there
    fn sh_pager(&mut self, ni: usize) {
        let term = self.sh_stdout_term(ni) && self.sh_bg_ctx(ni).is_none();
        if term && let Some(s) = self.sh_mut(ni) {
            s.pager = true;
        }
    }

    // ------------------------------------------------------------ files

    pub(crate) fn sh_cat(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut number = false;
        let mut files: Vec<String> = Vec::new();
        for a in args {
            match *a {
                "-n" => number = true,
                "--help" => return self.tool_help(ni, "cat"),
                s if s.starts_with('-') && s.len() > 1 => return self.tool_usage(ni, "cat", &format!("invalid option -- '{}'", &s[1..2])),
                s => files.push(String::from(s)),
            }
        }
        if files.is_empty() {
            files.push(String::from("-"));
        }
        let mut err = None;
        let mut n = 0;
        for f in files {
            let data = if f == "-" {
                Ok(self.sh_take_stdin(ni).unwrap_or_default())
            } else {
                let abs = self.sh_abs(ni, &f);
                self.sh_read(ni, &abs)
            };
            match data {
                Ok(d) => {
                    let mut t = String::from_utf8_lossy(&d).into_owned();
                    if number {
                        let mut s = String::new();
                        for l in t.split_inclusive('\n') {
                            n += 1;
                            s.push_str(&format!("{:>6}\t{}", n, l));
                        }
                        t = s;
                    }
                    self.out(ni, &t);
                }
                Err(e) => {
                    let m = format!("cat: {}: {}\n", f, e);
                    self.err_out(ni, &m);
                    err.get_or_insert(m);
                }
            }
        }
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_mv(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut noclobber = false;
        let mut ops: Vec<&str> = Vec::new();
        for a in args {
            match *a {
                "-f" | "-v" | "-i" => {}
                "-n" => noclobber = true,
                "--help" => return self.tool_help(ni, "mv"),
                s if s.starts_with('-') && s.len() > 1 => return self.tool_usage(ni, "mv", &format!("invalid option -- '{}'", &s[1..2])),
                s => ops.push(s),
            }
        }
        match ops.len() {
            0 => return self.tool_usage(ni, "mv", "missing file operand"),
            1 => return self.tool_usage(ni, "mv", &format!("missing destination file operand after '{}'", ops[0])),
            _ => {}
        }
        let dst = ops[ops.len() - 1];
        let srcs = &ops[..ops.len() - 1];
        let dst_dir = self.sh_kind(ni, dst) == Some(true);
        if srcs.len() > 1 && !dst_dir {
            return self.sh_err(ni, &format!("mv: target '{}' is not a directory\n", dst));
        }
        let mut err = None;
        for src in srcs {
            let Some(from) = self.sh_canon(ni, src).filter(|c| self.store(ni).is_some_and(|s| s.exists(c))) else {
                let m = format!("mv: cannot stat '{}': No such file or directory\n", src);
                self.err_out(ni, &m);
                err.get_or_insert(m);
                continue;
            };
            let to = if dst_dir {
                let name = self.store(ni).map(|s| String::from(s.name_of(&from))).unwrap_or_default();
                format!("{}/{}", self.sh_canon(ni, dst).unwrap_or_default().trim_end_matches('/'), name)
            } else {
                self.sh_abs(ni, dst)
            };
            let to_c = self.store(ni).and_then(|s| s.norm(&to).ok()).unwrap_or_default();
            if noclobber && self.store(ni).is_some_and(|s| s.exists(&to_c)) {
                continue;
            }
            let shown = if dst_dir { format!("{}/{}", dst.trim_end_matches('/'), self.store(ni).map(|s| String::from(s.name_of(&from))).unwrap_or_default()) } else { String::from(dst) };
            let r = self.file_rename(ni, &from, &to_c);
            let m = match r {
                Ok(()) => continue,
                Err(jfn_model::files::FsErr::BadName) => format!("mv: cannot move '{}' to a subdirectory of itself, '{}'\n", src, shown),
                Err(jfn_model::files::FsErr::IsDir) => format!("mv: cannot overwrite directory '{}' with non-directory\n", shown),
                Err(jfn_model::files::FsErr::NotDir) if self.store(ni).is_some_and(|s| s.is_dir(&from)) => format!("mv: cannot overwrite non-directory '{}' with directory '{}'\n", shown, src),
                Err(jfn_model::files::FsErr::ReadOnly) => format!("mv: cannot move '{}' to '{}': Permission denied\n", src, shown),
                Err(e) => format!("mv: cannot move '{}' to '{}': {}\n", src, shown, e.errno_text()),
            };
            self.err_out(ni, &m);
            err.get_or_insert(m);
        }
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    /// head and tail
    pub(crate) fn sh_head_tail(&mut self, ni: usize, tail: bool, args: &[&str]) -> Result<(), String> {
        let cmd = if tail { "tail" } else { "head" };
        let (mut lines, mut bytes): (Option<String>, Option<String>) = (None, None);
        let (mut quiet, mut verbose) = (false, false);
        let mut files: Vec<String> = Vec::new();
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            if a == "--help" {
                return self.tool_help(ni, cmd);
            }
            if let Some((v, n)) = count_opt(a, args.get(k + 1).copied(), 'n', "lines") {
                lines = Some(v);
                k += n;
                continue;
            }
            if let Some((v, n)) = count_opt(a, args.get(k + 1).copied(), 'c', "bytes") {
                bytes = Some(v);
                k += n;
                continue;
            }
            match a {
                "-q" => quiet = true,
                "-v" => verbose = true,
                "-f" | "-F" => return self.tool_usage(ni, cmd, "-f is not supported here: the file is read once"),
                s if s.len() > 1 && s.starts_with('-') && s[1..].bytes().all(|b| b.is_ascii_digit()) => lines = Some(String::from(&s[1..])),
                s if s.starts_with('-') && s.len() > 1 => return self.tool_usage(ni, cmd, &format!("invalid option -- '{}'", &s[1..2])),
                s => files.push(String::from(s)),
            }
            k += 1;
        }
        // the count: N, -N (head: all but the last N), +N (tail: from N on)
        let spec = bytes.clone().or(lines).unwrap_or_else(|| String::from("10"));
        let (plus, minus) = (spec.starts_with('+'), spec.starts_with('-'));
        let Ok(n) = spec.trim_start_matches(['+', '-']).parse::<usize>() else {
            let what = if bytes.is_some() { "bytes" } else { "lines" };
            return self.sh_err(ni, &format!("{}: invalid number of {}: '{}'\n", cmd, what, spec));
        };
        let several = files.len() > 1;
        let (inputs, failed) = self.sh_inputs(ni, cmd, &files);
        let mut out = String::new();
        for (i, (name, data)) in inputs.iter().enumerate() {
            if (several && !quiet) || verbose {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(&format!("==> {} <==\n", if name == "-" { "standard input" } else { name }));
            }
            if bytes.is_some() {
                let len = data.len();
                let part: &[u8] = match (tail, plus, minus) {
                    (false, _, true) => &data[..len.saturating_sub(n)],
                    (false, _, _) => &data[..n.min(len)],
                    (true, true, _) => &data[n.saturating_sub(1).min(len)..],
                    (true, _, _) => &data[len.saturating_sub(n)..],
                };
                out.push_str(&String::from_utf8_lossy(part));
                continue;
            }
            let t = String::from_utf8_lossy(data);
            let ls: Vec<&str> = t.split_inclusive('\n').collect();
            let len = ls.len();
            let part: &[&str] = match (tail, plus, minus) {
                (false, _, true) => &ls[..len.saturating_sub(n)],
                (false, _, _) => &ls[..n.min(len)],
                (true, true, _) => &ls[n.saturating_sub(1).min(len)..],
                (true, _, _) => &ls[len.saturating_sub(n)..],
            };
            for l in part {
                out.push_str(l);
            }
        }
        self.out(ni, &out);
        if failed { Err(String::new()) } else { Ok(()) }
    }

    pub(crate) fn sh_grep(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut icase, mut invert, mut number, mut count, mut list, mut rec, mut fixed, mut word, mut whole, mut only, mut quiet) = (false, false, false, false, false, false, false, false, false, false, false);
        let mut syntax = Syntax::Basic;
        let mut names: Option<bool> = None;
        let mut pats: Vec<String> = Vec::new();
        let mut rest: Vec<String> = Vec::new();
        let mut k = 0;
        let mut opts_done = false;
        while k < args.len() {
            let a = args[k];
            if opts_done || !a.starts_with('-') || a == "-" {
                rest.push(String::from(a));
                k += 1;
                continue;
            }
            match a {
                "--" => opts_done = true,
                "--help" => return self.tool_help(ni, "grep"),
                "-e" => {
                    match args.get(k + 1) {
                        Some(p) => pats.push(String::from(*p)),
                        None => {
                            self.sh_set_code(ni, 2);
                            return self.tool_usage(ni, "grep", "option requires an argument -- 'e'");
                        }
                    }
                    k += 1;
                }
                s if s.starts_with("--color") => {}
                s => {
                    for c in s[1..].chars() {
                        match c {
                            'i' | 'y' => icase = true,
                            'v' => invert = true,
                            'n' => number = true,
                            'c' => count = true,
                            'l' => list = true,
                            'r' | 'R' => rec = true,
                            'E' => syntax = Syntax::Extended,
                            'G' => syntax = Syntax::Basic,
                            'F' => fixed = true,
                            'w' => word = true,
                            'x' => whole = true,
                            'o' => only = true,
                            'q' | 's' => quiet = quiet || c == 'q',
                            'H' => names = Some(true),
                            'h' => names = Some(false),
                            o => {
                                self.sh_set_code(ni, 2);
                                return self.tool_usage(ni, "grep", &format!("invalid option -- '{}'", o));
                            }
                        }
                    }
                }
            }
            k += 1;
        }
        if pats.is_empty() {
            if rest.is_empty() {
                self.sh_set_code(ni, 2);
                return self.sh_err(ni, "Usage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information.\n");
            }
            pats.push(rest.remove(0));
        }
        // the expressions (-F: the text with every special character escaped)
        let mut res: Vec<Regex> = Vec::new();
        for p in &pats {
            let p = if fixed {
                let mut e = String::new();
                for c in p.chars() {
                    if matches!(c, '.' | '[' | ']' | '*' | '^' | '$' | '\\') {
                        e.push('\\');
                    }
                    e.push(c);
                }
                e
            } else {
                p.clone()
            };
            match Regex::new(&p, if fixed { Syntax::Basic } else { syntax }, icase) {
                Ok(r) => res.push(r),
                Err(e) => {
                    self.sh_set_code(ni, 2);
                    return self.sh_err(ni, &format!("grep: {}\n", e.text()));
                }
            }
        }
        // the files (-r: everything below a directory, in name order)
        let mut files: Vec<String> = Vec::new();
        let mut failed = false;
        if rec && rest.is_empty() {
            rest.push(String::from("."));
        }
        for f in &rest {
            if rec && self.sh_kind(ni, f) == Some(true) {
                let mut found = Vec::new();
                self.sh_walk(ni, f, 0, usize::MAX, &mut found);
                files.extend(found.into_iter().filter(|(_, d, _)| !*d).map(|(p, _, _)| p));
            } else if self.sh_kind(ni, f) == Some(true) {
                self.err_out(ni, &format!("grep: {}: Is a directory\n", f));
                failed = true;
            } else {
                files.push(f.clone());
            }
        }
        let show_names = names.unwrap_or(files.len() > 1 || rec);
        let (inputs, bad) = if files.is_empty() && !rest.is_empty() { (Vec::new(), false) } else { self.sh_inputs(ni, "grep", &files) };
        failed |= bad;
        let matches = |line: &str| -> Vec<(usize, usize)> {
            let mut v = Vec::new();
            for r in &res {
                let mut at = 0;
                while at <= line.len() {
                    let Some((s, e)) = r.find_at(line, at) else { break };
                    let ok_word = !word || {
                        let before = line[..s].chars().next_back();
                        let after = line[e..].chars().next();
                        let w = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                        !w(before) && !w(after) && e > s
                    };
                    let ok_whole = !whole || (s == 0 && e == line.len());
                    if ok_word && ok_whole {
                        v.push((s, e));
                        if !only {
                            return v;
                        }
                    }
                    at = if e > s { e } else { s + line[s..].chars().next().map_or(1, |c| c.len_utf8()) };
                }
            }
            v.sort();
            v
        };
        let mut out = String::new();
        let mut any = false;
        for (name, data) in &inputs {
            let shown = if name == "-" { "(standard input)" } else { name.as_str() };
            let mut n = 0usize;
            for (i, line) in lines_of(data).iter().enumerate() {
                let m = matches(line);
                let hit = m.is_empty() == invert;
                if !hit {
                    continue;
                }
                n += 1;
                any = true;
                if quiet || list || count {
                    continue;
                }
                let prefix = format!("{}{}", if show_names { format!("{}:", shown) } else { String::new() }, if number { format!("{}:", i + 1) } else { String::new() });
                if only && !invert {
                    for (s, e) in m.iter().filter(|(s, e)| e > s) {
                        out.push_str(&format!("{}{}\n", prefix, &line[*s..*e]));
                    }
                } else {
                    out.push_str(&format!("{}{}\n", prefix, line));
                }
            }
            if quiet && any {
                break;
            }
            if count {
                if show_names {
                    out.push_str(&format!("{}:{}\n", shown, n));
                } else {
                    out.push_str(&format!("{}\n", n));
                }
            } else if list && n > 0 {
                out.push_str(&format!("{}\n", shown));
            }
        }
        if !quiet {
            self.out(ni, &out);
        }
        let code = if quiet && any {
            0
        } else if failed {
            2
        } else if any {
            0
        } else {
            1
        };
        self.sh_set_code(ni, code);
        if code == 0 { Ok(()) } else { Err(String::new()) }
    }

    /// the paths below `start` (as typed), depth first in name order: (path, directory, depth)
    pub(crate) fn sh_walk(&self, ni: usize, start: &str, depth: usize, max: usize, out: &mut Vec<(String, bool, usize)>) {
        let Some(c) = self.sh_canon(ni, start) else { return };
        if depth >= max {
            return;
        }
        for (name, dir) in self.sh_entries(ni, &c) {
            let p = if start.ends_with('/') { format!("{}{}", start, name) } else { format!("{}/{}", start, name) };
            out.push((p.clone(), dir, depth + 1));
            if dir {
                self.sh_walk(ni, &p, depth + 1, max, out);
            }
        }
    }

    pub(crate) fn sh_find(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut starts: Vec<&str> = Vec::new();
        let mut k = 0;
        while k < args.len() && !args[k].starts_with('-') && args[k] != "!" {
            starts.push(args[k]);
            k += 1;
        }
        if starts.is_empty() {
            starts.push(".");
        }
        let (mut name, mut iname, mut kind, mut maxd, mut mind): (Option<String>, Option<String>, Option<bool>, usize, usize) = (None, None, None, usize::MAX, 0);
        while k < args.len() {
            let a = args[k];
            let v = args.get(k + 1).copied();
            let need = |e: &mut Engine| e.sh_err(ni, &format!("find: missing argument to `{}'\n", a));
            match a {
                "-name" | "-iname" | "-type" | "-maxdepth" | "-mindepth" => {
                    let Some(v) = v else { return need(self) };
                    match a {
                        "-name" => name = Some(String::from(v)),
                        "-iname" => iname = Some(v.to_lowercase()),
                        "-type" => match v {
                            "f" => kind = Some(false),
                            "d" => kind = Some(true),
                            _ => return self.sh_err(ni, &format!("find: Unknown argument to -type: {}\n", v)),
                        },
                        _ => match v.parse::<usize>() {
                            Ok(n) if a == "-maxdepth" => maxd = n,
                            Ok(n) => mind = n,
                            Err(_) => return self.sh_err(ni, &format!("find: Expected a positive decimal integer argument to {}, but got `{}'\n", a, v)),
                        },
                    }
                    k += 2;
                }
                "-print" => k += 1,
                "--help" => return self.tool_help(ni, "find"),
                o => return self.sh_err(ni, &format!("find: unknown predicate `{}'\n", o)),
            }
        }
        let mut out = String::new();
        let mut err = None;
        for s in starts {
            let Some(top) = self.sh_kind(ni, s) else {
                let m = format!("find: '{}': No such file or directory\n", s);
                self.err_out(ni, &m);
                err.get_or_insert(m);
                continue;
            };
            let mut all = alloc::vec![(String::from(s), top, 0usize)];
            if top {
                self.sh_walk(ni, s, 0, maxd, &mut all);
            }
            for (p, dir, depth) in all {
                if depth < mind || depth > maxd {
                    continue;
                }
                let base = p.trim_end_matches('/').rsplit('/').next().unwrap_or(&p);
                let base = if base.is_empty() { p.as_str() } else { base };
                if name.as_ref().is_some_and(|n| !base::pattern::glob(n, base)) || iname.as_ref().is_some_and(|n| !base::pattern::glob(n, &base.to_lowercase())) || kind.is_some_and(|d| d != dir) {
                    continue;
                }
                out.push_str(&p);
                out.push('\n');
            }
        }
        self.out(ni, &out);
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_wc(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut l, mut w, mut c, mut m) = (false, false, false, false);
        let mut files: Vec<String> = Vec::new();
        for a in args {
            match *a {
                "--help" => return self.tool_help(ni, "wc"),
                s if s.starts_with('-') && s.len() > 1 => {
                    for ch in s[1..].chars() {
                        match ch {
                            'l' => l = true,
                            'w' => w = true,
                            'c' => c = true,
                            'm' => m = true,
                            o => return self.tool_usage(ni, "wc", &format!("invalid option -- '{}'", o)),
                        }
                    }
                }
                s => files.push(String::from(s)),
            }
        }
        if !(l || w || c || m) {
            (l, w, c) = (true, true, true);
        }
        let n_counts = [l, w, m, c].iter().filter(|x| **x).count();
        let named = !files.is_empty();
        let (inputs, failed) = self.sh_inputs(ni, "wc", &files);
        let rows: Vec<(String, [u64; 4])> = inputs
            .iter()
            .map(|(name, d)| {
                let t = String::from_utf8_lossy(d);
                let lines = d.iter().filter(|&&b| b == b'\n').count() as u64;
                let words = t.split_whitespace().count() as u64;
                (name.clone(), [lines, words, t.chars().count() as u64, d.len() as u64])
            })
            .collect();
        // GNU's widths: one input with one count is not padded; else the digits of the total
        // size of the files, at least 7 for a pipe
        let width = if rows.len() == 1 && n_counts == 1 {
            1
        } else {
            let total: u64 = rows.iter().filter(|r| r.0 != "-").map(|r| r.1[3]).sum();
            let mut wd = format!("{}", total).len();
            if rows.iter().any(|r| r.0 == "-") {
                wd = wd.max(7);
            }
            wd
        };
        let fmt = |v: &[u64; 4], name: &str| -> String {
            let mut parts = Vec::new();
            for (on, x) in [(l, v[0]), (w, v[1]), (m, v[2]), (c, v[3])] {
                if on {
                    parts.push(format!("{:>width$}", x, width = width));
                }
            }
            let mut s = parts.join(" ");
            if !name.is_empty() {
                s.push(' ');
                s.push_str(name);
            }
            s.push('\n');
            s
        };
        let mut out = String::new();
        let mut sum = [0u64; 4];
        for (name, v) in &rows {
            for i in 0..4 {
                sum[i] += v[i];
            }
            out.push_str(&fmt(v, if named && name != "-" { name } else if named { "-" } else { "" }));
        }
        if rows.len() > 1 {
            out.push_str(&fmt(&sum, "total"));
        }
        self.out(ni, &out);
        if failed { Err(String::new()) } else { Ok(()) }
    }

    pub(crate) fn sh_sort(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut rev, mut numeric, mut uniq, mut fold) = (false, false, false, false);
        let mut key: Option<usize> = None;
        let mut sep: Option<char> = None;
        let mut files: Vec<String> = Vec::new();
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            match a {
                "--help" => return self.tool_help(ni, "sort"),
                "-k" | "-t" => {
                    let Some(v) = args.get(k + 1) else { return self.tool_usage(ni, "sort", &format!("option requires an argument -- '{}'", &a[1..])) };
                    if a == "-k" {
                        let f = v.split([',', '.']).next().unwrap_or("").trim_end_matches(|c: char| c.is_ascii_alphabetic());
                        match f.parse::<usize>() {
                            Ok(n) if n > 0 => key = Some(n),
                            _ => return self.sh_err(ni, &format!("sort: invalid number at field start: invalid count at start of '{}'\n", v)),
                        }
                    } else {
                        let mut ch = v.chars();
                        match (ch.next(), ch.next()) {
                            (Some(c), None) => sep = Some(c),
                            _ => return self.sh_err(ni, "sort: multi-character tab\n"),
                        }
                    }
                    k += 2;
                    continue;
                }
                s if s.starts_with('-') && s.len() > 1 => {
                    for c in s[1..].chars() {
                        match c {
                            'r' => rev = true,
                            'n' => numeric = true,
                            'u' => uniq = true,
                            'f' => fold = true,
                            'b' | 's' => {}
                            o => return self.tool_usage(ni, "sort", &format!("invalid option -- '{}'", o)),
                        }
                    }
                }
                s => files.push(String::from(s)),
            }
            k += 1;
        }
        let (inputs, failed) = self.sh_inputs(ni, "sort", &files);
        let mut lines: Vec<String> = Vec::new();
        for (_, d) in &inputs {
            lines.extend(lines_of(d));
        }
        let key_of = |l: &str| -> String {
            let Some(n) = key else { return String::from(l) };
            match sep {
                Some(c) => l.split(c).skip(n - 1).collect::<Vec<_>>().join(&String::from(c)),
                None => {
                    // blank-separated fields: the field starts with its leading blanks
                    let mut idx = 0;
                    let b: Vec<char> = l.chars().collect();
                    let mut field = 1;
                    while field < n && idx < b.len() {
                        while idx < b.len() && b[idx].is_whitespace() {
                            idx += 1;
                        }
                        while idx < b.len() && !b[idx].is_whitespace() {
                            idx += 1;
                        }
                        field += 1;
                    }
                    b[idx.min(b.len())..].iter().collect()
                }
            }
        };
        let cmp = |a: &String, b: &String| -> core::cmp::Ordering {
            let (ka, kb) = (key_of(a), key_of(b));
            let o = if numeric {
                num_key(&ka).partial_cmp(&num_key(&kb)).unwrap_or(core::cmp::Ordering::Equal)
            } else if fold {
                ka.to_uppercase().cmp(&kb.to_uppercase())
            } else {
                ka.cmp(&kb)
            };
            o
        };
        lines.sort_by(|a, b| {
            let o = cmp(a, b);
            // the last resort of GNU sort: the whole line by bytes (not with -u)
            let o = if o == core::cmp::Ordering::Equal && !uniq { a.cmp(b) } else { o };
            if rev { o.reverse() } else { o }
        });
        if uniq {
            lines.dedup_by(|a, b| cmp(a, b) == core::cmp::Ordering::Equal);
        }
        let mut out = String::new();
        for l in lines {
            out.push_str(&l);
            out.push('\n');
        }
        self.out(ni, &out);
        if failed {
            self.sh_set_code(ni, 2);
            Err(String::new())
        } else {
            Ok(())
        }
    }

    pub(crate) fn sh_uniq(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut count, mut dups, mut uniques, mut icase) = (false, false, false, false);
        let mut files: Vec<String> = Vec::new();
        for a in args {
            match *a {
                "--help" => return self.tool_help(ni, "uniq"),
                s if s.starts_with('-') && s.len() > 1 => {
                    for c in s[1..].chars() {
                        match c {
                            'c' => count = true,
                            'd' => dups = true,
                            'u' => uniques = true,
                            'i' => icase = true,
                            o => return self.tool_usage(ni, "uniq", &format!("invalid option -- '{}'", o)),
                        }
                    }
                }
                s => files.push(String::from(s)),
            }
        }
        // uniq [INPUT [OUTPUT]]: only the input here
        files.truncate(1);
        let (inputs, failed) = self.sh_inputs(ni, "uniq", &files);
        let lines: Vec<String> = inputs.iter().flat_map(|(_, d)| lines_of(d)).collect();
        let mut groups: Vec<(String, usize)> = Vec::new();
        for l in lines {
            match groups.last_mut() {
                Some((g, n)) if (if icase { g.to_lowercase() == l.to_lowercase() } else { *g == l }) => *n += 1,
                _ => groups.push((l, 1)),
            }
        }
        let mut out = String::new();
        for (l, n) in groups {
            if (dups && n < 2) || (uniques && n > 1) {
                continue;
            }
            if count {
                out.push_str(&format!("{:>7} {}\n", n, l));
            } else {
                out.push_str(&format!("{}\n", l));
            }
        }
        self.out(ni, &out);
        if failed { Err(String::new()) } else { Ok(()) }
    }

    pub(crate) fn sh_cut(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut delim = '\t';
        let mut fields: Option<String> = None;
        let mut chars: Option<String> = None;
        let mut only = false;
        let mut files: Vec<String> = Vec::new();
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            let take = |k: &mut usize, a: &str, flag: char| -> Option<String> {
                let rest = a.strip_prefix('-')?.strip_prefix(flag)?;
                if rest.is_empty() {
                    *k += 1;
                    args.get(*k).map(|s| String::from(*s))
                } else {
                    Some(String::from(rest))
                }
            };
            if a == "--help" {
                return self.tool_help(ni, "cut");
            }
            if let Some(v) = take(&mut k, a, 'd') {
                let mut c = v.chars();
                match (c.next(), c.next()) {
                    (Some(x), None) => delim = x,
                    _ => return self.tool_usage(ni, "cut", "the delimiter must be a single character"),
                }
            } else if let Some(v) = take(&mut k, a, 'f') {
                fields = Some(v);
            } else if let Some(v) = take(&mut k, a, 'c').or_else(|| take(&mut k, a, 'b')) {
                chars = Some(v);
            } else if a == "-s" {
                only = true;
            } else if a.starts_with('-') && a.len() > 1 {
                return self.tool_usage(ni, "cut", &format!("invalid option -- '{}'", &a[1..2]));
            } else {
                files.push(String::from(a));
            }
            k += 1;
        }
        let Some(list) = fields.clone().or(chars.clone()) else {
            return self.tool_usage(ni, "cut", "you must specify a list of bytes, characters, or fields");
        };
        // N, N-M, N-, -M
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for part in list.split(',') {
            let (a, b) = match part.split_once('-') {
                Some((a, b)) => (if a.is_empty() { Some(1) } else { a.parse().ok() }, if b.is_empty() { Some(usize::MAX) } else { b.parse().ok() }),
                None => (part.parse().ok(), part.parse().ok()),
            };
            match (a, b) {
                (Some(a), Some(b)) if a >= 1 && a <= b => ranges.push((a, b)),
                _ => return self.tool_usage(ni, "cut", &format!("invalid field range '{}'", part)),
            }
        }
        let inr = |i: usize| ranges.iter().any(|r| r.0 <= i && i <= r.1);
        let (inputs, failed) = self.sh_inputs(ni, "cut", &files);
        let mut out = String::new();
        for (_, d) in &inputs {
            for l in lines_of(d) {
                if fields.is_some() {
                    if !l.contains(delim) {
                        if !only {
                            out.push_str(&l);
                            out.push('\n');
                        }
                        continue;
                    }
                    let picked: Vec<&str> = l.split(delim).enumerate().filter(|(i, _)| inr(i + 1)).map(|x| x.1).collect();
                    out.push_str(&picked.join(&String::from(delim)));
                } else {
                    let picked: String = l.chars().enumerate().filter(|(i, _)| inr(i + 1)).map(|x| x.1).collect();
                    out.push_str(&picked);
                }
                out.push('\n');
            }
        }
        self.out(ni, &out);
        if failed { Err(String::new()) } else { Ok(()) }
    }

    pub(crate) fn sh_tree(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut all, mut dirs_only, mut level) = (false, false, usize::MAX);
        let mut roots: Vec<&str> = Vec::new();
        let mut k = 0;
        while k < args.len() {
            match args[k] {
                "-a" => all = true,
                "-d" => dirs_only = true,
                "-L" => {
                    match args.get(k + 1).and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0) {
                        Some(n) => level = n,
                        None => return self.sh_err(ni, "tree: Invalid level, must be greater than 0.\n"),
                    }
                    k += 1;
                }
                "--help" => return self.tool_help(ni, "tree"),
                s if s.starts_with('-') && s.len() > 1 => return self.sh_err(ni, &format!("tree: Invalid argument -`{}'.\n", &s[1..])),
                s => roots.push(s),
            }
            k += 1;
        }
        if roots.is_empty() {
            roots.push(".");
        }
        let mut out = String::new();
        let (mut nd, mut nf) = (0usize, 0usize);
        for r in roots {
            if self.sh_kind(ni, r) != Some(true) {
                out.push_str(&format!("{} [error opening dir]\n", r));
                continue;
            }
            out.push_str(r);
            out.push('\n');
            let Some(c) = self.sh_canon(ni, r) else { continue };
            self.tree_level(ni, &c, String::new(), 1, level, all, dirs_only, &mut out, &mut nd, &mut nf);
        }
        let plural = |n: usize, one: &str, many: &str| format!("{} {}", n, if n == 1 { one } else { many });
        if dirs_only {
            out.push_str(&format!("\n{}\n", plural(nd, "directory", "directories")));
        } else {
            out.push_str(&format!("\n{}, {}\n", plural(nd, "directory", "directories"), plural(nf, "file", "files")));
        }
        self.out(ni, &out);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_level(&self, ni: usize, dir: &str, pre: String, depth: usize, max: usize, all: bool, dirs_only: bool, out: &mut String, nd: &mut usize, nf: &mut usize) {
        let list: Vec<(String, bool)> = self.sh_entries(ni, dir).into_iter().filter(|(n, d)| (all || !n.starts_with('.')) && (*d || !dirs_only)).collect();
        for (i, (name, d)) in list.iter().enumerate() {
            let last = i + 1 == list.len();
            out.push_str(&format!("{}{}{}\n", pre, if last { "└── " } else { "├── " }, name));
            if *d {
                *nd += 1;
                if depth < max {
                    let sub = format!("{}/{}", dir.trim_end_matches('/'), name);
                    self.tree_level(ni, &sub, format!("{}{}", pre, if last { "    " } else { "│   " }), depth + 1, max, all, dirs_only, out, nd, nf);
                }
            } else {
                *nf += 1;
            }
        }
    }

    /// the KiB a path takes, its directories listed on the way (post order) into `rows`
    fn du_of(&self, ni: usize, shown: &str, canon: &str, files: bool, rows: &mut Vec<(u64, String)>) -> u64 {
        let Some(st) = self.store(ni) else { return 0 };
        let s = st.stat(canon).map(|s| (s.dir, s.size)).or_else(|| self.cat_etc(ni, canon).map(|t| (false, t.len() as u64)));
        match s {
            Some((true, _)) => {
                let mut sum = 4;
                for (name, _) in self.sh_entries(ni, canon) {
                    let sub_s = if shown.ends_with('/') { format!("{}{}", shown, name) } else { format!("{}/{}", shown, name) };
                    let sub_c = format!("{}/{}", canon.trim_end_matches('/'), name);
                    sum += self.du_of(ni, &sub_s, &sub_c, files, rows);
                }
                rows.push((sum, String::from(shown)));
                sum
            }
            Some((false, size)) => {
                let k = kib_of(size);
                if files {
                    rows.push((k, String::from(shown)));
                }
                k
            }
            None => 0,
        }
    }

    pub(crate) fn sh_du(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut summary, mut human_on, mut all, mut total) = (false, false, false, false);
        let mut paths: Vec<&str> = Vec::new();
        for a in args {
            match *a {
                "--help" => return self.tool_help(ni, "du"),
                s if s.starts_with('-') && s.len() > 1 => {
                    for c in s[1..].chars() {
                        match c {
                            's' => summary = true,
                            'h' => human_on = true,
                            'a' => all = true,
                            'c' => total = true,
                            'k' => {}
                            o => return self.tool_usage(ni, "du", &format!("invalid option -- '{}'", o)),
                        }
                    }
                }
                s => paths.push(s),
            }
        }
        if paths.is_empty() {
            paths.push(".");
        }
        let mut out = String::new();
        let mut err = None;
        let mut grand = 0u64;
        for p in paths {
            let Some(c) = self.sh_canon(ni, p).filter(|_| self.sh_kind(ni, p).is_some()) else {
                let m = format!("du: cannot access '{}': No such file or directory\n", p);
                self.err_out(ni, &m);
                err.get_or_insert(m);
                continue;
            };
            let mut rows = Vec::new();
            let sum = self.du_of(ni, p, &c, all, &mut rows);
            grand += sum;
            if summary {
                rows = alloc::vec![(sum, String::from(p))];
            } else if self.sh_kind(ni, p) == Some(false) && rows.is_empty() {
                rows.push((sum, String::from(p)));
            }
            for (k, name) in rows {
                out.push_str(&format!("{}\t{}\n", if human_on { human(k * 1024) } else { format!("{}", k) }, name));
            }
        }
        if total {
            out.push_str(&format!("{}\ttotal\n", if human_on { human(grand * 1024) } else { format!("{}", grand) }));
        }
        self.out(ni, &out);
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_df(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut human_on = false;
        for a in args {
            match *a {
                "-h" | "-H" => human_on = true,
                "-k" | "-T" | "-a" => {}
                "--help" => return self.tool_help(ni, "df"),
                s if s.starts_with('-') => return self.tool_usage(ni, "df", &format!("invalid option -- '{}'", &s[1..s.len().min(2)])),
                s => {
                    if self.sh_kind(ni, s).is_none() {
                        return self.sh_err(ni, &format!("df: {}: No such file or directory\n", s));
                    }
                }
            }
        }
        let Some(st) = self.store(ni) else { return self.sh_err(ni, "df: no file systems processed\n") };
        let (cap, used) = (st.capacity, st.used());
        let pct = if cap == 0 { String::from("-") } else { format!("{}%", (used as u128 * 100).div_ceil(cap as u128)) };
        let row: [String; 6] = if human_on {
            [String::from("/dev/sda1"), human(cap), human(used.div_ceil(1024) * 1024), human(cap.saturating_sub(used)), pct, String::from("/")]
        } else {
            [String::from("/dev/sda1"), format!("{}", cap / 1024), format!("{}", used.div_ceil(1024)), format!("{}", cap.saturating_sub(used) / 1024), pct, String::from("/")]
        };
        let head: [&str; 6] = if human_on { ["Filesystem", "Size", "Used", "Avail", "Use%", "Mounted on"] } else { ["Filesystem", "1K-blocks", "Used", "Available", "Use%", "Mounted on"] };
        let w: Vec<usize> = (0..6).map(|i| head[i].len().max(row[i].len())).collect();
        let line = |c: [&str; 6]| -> String { format!("{:<w0$} {:>w1$} {:>w2$} {:>w3$} {:>w4$} {}\n", c[0], c[1], c[2], c[3], c[4], c[5], w0 = w[0].max(14), w1 = w[1], w2 = w[2], w3 = w[3], w4 = w[4]) };
        let mut out = line(head);
        out.push_str(&line([&row[0], &row[1], &row[2], &row[3], &row[4], &row[5]]));
        self.out(ni, &out);
        Ok(())
    }

    /// less and more: the input or the files, paged by the terminal when it goes there
    pub(crate) fn sh_less(&mut self, ni: usize, cmd: &str, args: &[&str]) -> Result<(), String> {
        let files: Vec<String> = args.iter().filter(|a| !a.starts_with('-')).map(|s| String::from(*s)).collect();
        if args.contains(&"--help") {
            return self.tool_help(ni, cmd);
        }
        let mut err = None;
        let mut text = String::new();
        if files.is_empty() {
            let d = self.sh_take_stdin(ni).unwrap_or_default();
            text.push_str(&String::from_utf8_lossy(&d));
        }
        for f in &files {
            match self.sh_kind(ni, f) {
                Some(true) => {
                    let m = format!("{} is a directory\n", f);
                    self.err_out(ni, &m);
                    err.get_or_insert(m);
                }
                _ => {
                    let abs = self.sh_abs(ni, f);
                    match self.sh_read(ni, &abs) {
                        Ok(d) => text.push_str(&String::from_utf8_lossy(&d)),
                        Err(e) => {
                            let m = format!("{}: {}: {}\n", cmd, f, e);
                            self.err_out(ni, &m);
                            err.get_or_insert(m);
                        }
                    }
                }
            }
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        self.out(ni, &text);
        self.sh_pager(ni);
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_file(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let files: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
        if files.is_empty() {
            return self.sh_err(ni, "Usage: file FILE ...\n");
        }
        let mut out = String::new();
        for f in files {
            let what = match self.sh_kind(ni, f) {
                None => format!("cannot open `{}' (No such file or directory)", f),
                Some(true) => String::from("directory"),
                Some(false) => {
                    let abs = self.sh_abs(ni, f);
                    match self.sh_read(ni, &abs) {
                        Ok(d) if d.is_empty() => String::from("empty"),
                        Ok(d) => match core::str::from_utf8(&d) {
                            Ok(t) if jfn_model::files::is_text(&d) => {
                                let kind = if t.is_ascii() { "ASCII text" } else { "Unicode text, UTF-8 text" };
                                let crlf = if t.contains("\r\n") { ", with CRLF line terminators" } else { "" };
                                let noeol = if !t.ends_with('\n') && !crlf.is_empty() { "" } else if !t.ends_with('\n') { ", with no line terminators" } else { "" };
                                format!("{}{}{}", kind, crlf, noeol)
                            }
                            _ => String::from("data"),
                        },
                        Err(_) => String::from("data"),
                    }
                }
            };
            out.push_str(&format!("{}: {}\n", f, what));
        }
        self.out(ni, &out);
        Ok(())
    }

    pub(crate) fn sh_stat(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let files: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
        if files.is_empty() {
            return self.tool_usage(ni, "stat", "missing operand");
        }
        let mut out = String::new();
        let mut err = None;
        for f in files {
            let c = self.sh_canon(ni, f).unwrap_or_default();
            let st = self.store(ni).and_then(|s| s.stat(&c)).or_else(|| self.cat_etc(ni, &c).map(|t| jfn_model::files::Stat { size: t.len() as u64, mtime: 0, dir: false, builtin: true }));
            let Some(s) = st else {
                let m = format!("stat: cannot statx '{}': No such file or directory\n", f);
                self.err_out(ni, &m);
                err.get_or_insert(m);
                continue;
            };
            let (size, kind, mode) = if s.dir { (4096, "directory", "0755/drwxr-xr-x") } else { (s.size, if s.size == 0 { "regular empty file" } else { "regular file" }, "0644/-rw-r--r--") };
            let t = if s.mtime == 0 { EPOCH_UNIX_NS / 1_000_000_000 } else { s.mtime };
            let d = datetime::Civil::from_unix(t);
            let when = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.000000000 +0000", d.year, d.month, d.day, d.hour, d.minute, d.second);
            out.push_str(&format!("  File: {}\n  Size: {:<10}\tBlocks: {:<10} IO Block: 4096   {}\n", f, size, kib_of(size) * 2, kind));
            out.push_str(&format!("Access: ({})  Uid: (    0/    root)   Gid: (    0/    root)\nModify: {}\n", mode, when));
        }
        self.out(ni, &out);
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_basename(&mut self, ni: usize, dir: bool, args: &[&str]) -> Result<(), String> {
        let cmd = if dir { "dirname" } else { "basename" };
        let ops: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-') || *a == "-").collect();
        if args.contains(&"--help") {
            return self.tool_help(ni, cmd);
        }
        let Some(p) = ops.first() else { return self.tool_usage(ni, cmd, "missing operand") };
        let t = p.trim_end_matches('/');
        let r = if dir {
            if p.starts_with('/') && t.is_empty() {
                String::from("/")
            } else {
                match t.rfind('/') {
                    Some(0) => String::from("/"),
                    Some(i) => String::from(t[..i].trim_end_matches('/')),
                    None => String::from("."),
                }
            }
        } else if t.is_empty() {
            String::from(if p.starts_with('/') { "/" } else { "" })
        } else {
            let b = t.rsplit('/').next().unwrap_or(t);
            match ops.get(1) {
                Some(suf) if b.len() > suf.len() && b.ends_with(*suf) => String::from(&b[..b.len() - suf.len()]),
                _ => String::from(b),
            }
        };
        let r = if r.is_empty() && dir { String::from("/") } else { r };
        self.out(ni, &format!("{}\n", r));
        Ok(())
    }

    pub(crate) fn sh_date(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut fmt: Option<&str> = None;
        for a in args {
            match *a {
                "-u" | "--utc" | "-R" => {}
                "--help" => return self.tool_help(ni, "date"),
                s if s.starts_with('+') => fmt = Some(&s[1..]),
                s if s.starts_with('-') => return self.tool_usage(ni, "date", &format!("invalid option -- '{}'", &s[1..s.len().min(2)])),
                s => return self.sh_err(ni, &format!("date: cannot set the date of a simulated device: '{}'\n", s)),
            }
        }
        let dev = self.dev_of(ni);
        let ns = self.clock(dev).unwrap_or(EPOCH_UNIX_NS + self.now as i64);
        let secs = ns.div_euclid(1_000_000_000);
        let c = datetime::Civil::from_unix(secs);
        let wd = DAYS[(c.weekday() as usize).min(6)];
        let mon = MONTHS[(c.month.clamp(1, 12) - 1) as usize];
        let text = match fmt {
            None => format!("{} {} {:>2} {:02}:{:02}:{:02} UTC {}\n", wd, mon, c.day, c.hour, c.minute, c.second, c.year),
            Some(f) => {
                let mut s = String::new();
                let mut it = f.chars();
                while let Some(ch) = it.next() {
                    if ch != '%' {
                        s.push(ch);
                        continue;
                    }
                    let yday = datetime::days_from_civil(c.year, c.month, c.day) - datetime::days_from_civil(c.year, 1, 1) + 1;
                    match it.next() {
                        Some('Y') => s.push_str(&format!("{}", c.year)),
                        Some('m') => s.push_str(&format!("{:02}", c.month)),
                        Some('d') => s.push_str(&format!("{:02}", c.day)),
                        Some('e') => s.push_str(&format!("{:>2}", c.day)),
                        Some('H') => s.push_str(&format!("{:02}", c.hour)),
                        Some('M') => s.push_str(&format!("{:02}", c.minute)),
                        Some('S') => s.push_str(&format!("{:02}", c.second)),
                        Some('F') => s.push_str(&format!("{}-{:02}-{:02}", c.year, c.month, c.day)),
                        Some('T') => s.push_str(&format!("{:02}:{:02}:{:02}", c.hour, c.minute, c.second)),
                        Some('s') => s.push_str(&format!("{}", secs)),
                        Some('N') => s.push_str(&format!("{:09}", ns.rem_euclid(1_000_000_000))),
                        Some('a') => s.push_str(wd),
                        Some('b') | Some('h') => s.push_str(mon),
                        Some('j') => s.push_str(&format!("{:03}", yday)),
                        Some('Z') => s.push_str("UTC"),
                        Some('z') => s.push_str("+0000"),
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('%') => s.push('%'),
                        Some(o) => {
                            s.push('%');
                            s.push(o);
                        }
                        None => s.push('%'),
                    }
                }
                s.push('\n');
                s
            }
        };
        self.out(ni, &text);
        Ok(())
    }

    pub(crate) fn sh_seq(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut sep = String::from("\n");
        let mut wide = false;
        let mut nums: Vec<&str> = Vec::new();
        let mut k = 0;
        while k < args.len() {
            match args[k] {
                "-s" => {
                    sep = String::from(args.get(k + 1).copied().unwrap_or(""));
                    k += 1;
                }
                "-w" => wide = true,
                "--help" => return self.tool_help(ni, "seq"),
                s => nums.push(s),
            }
            k += 1;
        }
        let mut v: Vec<i64> = Vec::new();
        for n in &nums {
            match n.parse::<i64>() {
                Ok(x) => v.push(x),
                Err(_) => return self.tool_usage(ni, "seq", &format!("invalid floating point argument: '{}'", n)),
            }
        }
        let (first, step, last) = match v.as_slice() {
            [l] => (1, 1, *l),
            [f, l] => (*f, 1, *l),
            [f, s, l] => (*f, *s, *l),
            [] => return self.tool_usage(ni, "seq", "missing operand"),
            _ => return self.tool_usage(ni, "seq", &format!("extra operand '{}'", nums[3])),
        };
        if step == 0 {
            return self.tool_usage(ni, "seq", "invalid Zero increment value: '0'");
        }
        let width = if wide { format!("{}", first).len().max(format!("{}", last).len()) } else { 0 };
        let mut parts: Vec<String> = Vec::new();
        let mut x = first;
        while (step > 0 && x <= last) || (step < 0 && x >= last) {
            parts.push(format!("{:0w$}", x, w = width));
            if parts.len() > 1_000_000 {
                break;
            }
            x = x.saturating_add(step);
        }
        let mut out = parts.join(&sep);
        if !parts.is_empty() {
            out.push('\n');
        }
        self.out(ni, &out);
        Ok(())
    }

    pub(crate) fn sh_tee(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let append = args.contains(&"-a");
        let files: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
        let data = self.sh_take_stdin(ni).unwrap_or_default();
        let mut err = None;
        for f in files {
            let abs = self.sh_abs(ni, f);
            if let Err(e) = self.sh_writable(ni, &abs).map_err(String::from).and_then(|_| self.file_put(ni, &abs, &data, append).map(|_| ()).map_err(|e| String::from(e.errno_text()))) {
                let m = format!("tee: {}: {}\n", f, e);
                self.err_out(ni, &m);
                err.get_or_insert(m);
            }
        }
        self.out(ni, &String::from_utf8_lossy(&data));
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    /// test EXPR and [ EXPR ]: 0 true, 1 false, 2 an error
    pub(crate) fn sh_test(&mut self, ni: usize, bracket: bool, args: &[&str]) -> Result<(), String> {
        let mut a: Vec<&str> = args.to_vec();
        if bracket {
            if a.last() != Some(&"]") {
                self.sh_set_code(ni, 2);
                return self.sh_err(ni, "jfn-sh: [: missing `]'\n");
            }
            a.pop();
        }
        let mut p = 0usize;
        let r = self.test_or(ni, &a, &mut p);
        let r = match r {
            Ok(_) if p < a.len() => Err(format!("jfn-sh: test: {}: unexpected argument\n", a[p])),
            x => x,
        };
        match r {
            Ok(true) => Ok(()),
            Ok(false) => Err(String::new()),
            Err(m) => {
                self.sh_set_code(ni, 2);
                self.sh_err(ni, &m)
            }
        }
    }

    fn test_or(&self, ni: usize, a: &[&str], p: &mut usize) -> Result<bool, String> {
        let mut v = self.test_and(ni, a, p)?;
        while a.get(*p) == Some(&"-o") {
            *p += 1;
            let w = self.test_and(ni, a, p)?;
            v = v || w;
        }
        Ok(v)
    }

    fn test_and(&self, ni: usize, a: &[&str], p: &mut usize) -> Result<bool, String> {
        let mut v = self.test_not(ni, a, p)?;
        while a.get(*p) == Some(&"-a") {
            *p += 1;
            let w = self.test_not(ni, a, p)?;
            v = v && w;
        }
        Ok(v)
    }

    fn test_not(&self, ni: usize, a: &[&str], p: &mut usize) -> Result<bool, String> {
        if a.get(*p) == Some(&"!") && a.len() > *p + 1 {
            *p += 1;
            return Ok(!self.test_not(ni, a, p)?);
        }
        self.test_prim(ni, a, p)
    }

    fn test_prim(&self, ni: usize, a: &[&str], p: &mut usize) -> Result<bool, String> {
        let Some(x) = a.get(*p).copied() else { return Ok(false) };
        // binary operators
        if let (Some(op), Some(y)) = (a.get(*p + 1).copied(), a.get(*p + 2).copied()) {
            let int = |s: &str| -> Result<i64, String> { s.trim().parse::<i64>().map_err(|_| format!("jfn-sh: test: {}: integer expression expected\n", s)) };
            let r = match op {
                "=" | "==" => Some(Ok(x == y)),
                "!=" => Some(Ok(x != y)),
                "<" => Some(Ok(x < y)),
                ">" => Some(Ok(x > y)),
                "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => Some((|| {
                    let (l, r) = (int(x)?, int(y)?);
                    Ok(match op {
                        "-eq" => l == r,
                        "-ne" => l != r,
                        "-lt" => l < r,
                        "-le" => l <= r,
                        "-gt" => l > r,
                        _ => l >= r,
                    })
                })()),
                _ => None,
            };
            if let Some(r) = r {
                *p += 3;
                return r;
            }
        }
        // unary operators
        if x.len() == 2 && x.starts_with('-') && a.len() > *p + 1 {
            let y = a[*p + 1];
            let kind = self.sh_kind(ni, y);
            let r = match &x[1..] {
                "e" | "r" | "w" => kind.is_some(),
                "f" => kind == Some(false),
                "d" => kind == Some(true),
                "x" => kind == Some(true),
                "s" => kind == Some(true) || (kind == Some(false) && self.sh_read(ni, &self.sh_abs(ni, y)).is_ok_and(|d| !d.is_empty())),
                "z" => y.is_empty(),
                "n" => !y.is_empty(),
                "L" | "h" => false,
                _ => return Err(format!("jfn-sh: test: {}: unary operator expected\n", x)),
            };
            *p += 2;
            return Ok(r);
        }
        *p += 1;
        Ok(!x.is_empty())
    }

    /// xargs [CMD [ARGS]]: CMD (else echo) once with the words of the input added
    pub(crate) fn sh_xargs(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let data = self.sh_take_stdin(ni).unwrap_or_default();
        let words: Vec<String> = String::from_utf8_lossy(&data).split_whitespace().map(String::from).collect();
        // the options of xargs itself are taken and left aside (one run with every word)
        let mut k = 0;
        while k < args.len() && args[k].starts_with('-') {
            if matches!(args[k], "-n" | "-I" | "-d" | "-L" | "-P") {
                k += 1;
            }
            k += 1;
        }
        let mut argv: Vec<String> = args[k.min(args.len())..].iter().map(|s| String::from(*s)).collect();
        if argv.is_empty() {
            argv.push(String::from("echo"));
        }
        if words.is_empty() && argv[0] != "echo" {
            return Ok(());
        }
        argv.extend(words);
        self.linux_cmd(ni, &argv)
    }
}

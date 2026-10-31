// shelltest -- checks of the shell with folders, the classic tools and processes (group shell
// of --selftest: jfn_sim::cli::sh, shrun, coreutils, man, the flash: directories of
// cli/files.rs and jfn_model::files): the working directory and cd, the grammar (lists, &&,
// ||, !, quoting, variables, $?, history), redirections, pipes (also after a ping), globs,
// every tool with its exact output, Tab completion of paths, processes (jfs scripts that print,
// wait, fail and are ended with Ctrl+C, background jobs with [N] PID and their done lines,
// jobs, fg, wait, kill, ps, sleep, exit codes), IOS-like flash: directories, a project with
// folders saved and loaded, power off, determinism
//
// The project is tests/fixtures/shell_tour.jfnet: PC1 (10.0.0.1), SRV1 (10.0.0.10, a web
// server "web"), SW1, R1 (JF-R4, 10.0.0.254) as their gateway.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::files::{FsErr, Store};
use jfn_model::project::Project;
use jfn_sim::{Engine, Report, MS, SEC};

use crate::selftest::T;

const FIX: &str = include_str!("../tests/fixtures/shell_tour.jfnet");
const BUDGET: usize = 4_000_000;
const PC1: u32 = 1;
const SRV1: u32 = 2;
const R1: u32 = 4;

fn s(x: &str) -> String {
    String::from(x)
}

fn project() -> Project {
    Project::from_text(FIX).unwrap_or_else(|_| Project::new("shell", 13))
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

/// lines typed at a device, then `d` of virtual time: what it printed
fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    run_for(e, d);
    out(e, dev)
}

/// one line on PC1 at once
fn sh(e: &mut Engine, line: &str) -> String {
    cli(e, PC1, &[line], 0)
}

/// a fresh project with PC1's tree: app/ (a.txt "hallo\nwelt\n", lib/, b1.txt, b2.txt, c.log),
/// notes.txt, .hidden
fn tree_engine() -> Engine {
    let mut e = engine(&project());
    let _ = sh(&mut e, "mkdir -p app/lib; printf 'hallo\\nwelt\\n' > app/a.txt; touch app/b1.txt app/b2.txt app/c.log; echo Notiz > notes.txt; echo x > .hidden");
    e
}

pub fn run(t: &mut T) {
    cwd_t(t);
    grammar_t(t);
    redir_t(t);
    pipe_t(t);
    glob_t(t);
    tools_t(t);
    text_t(t);
    tab_t(t);
    jfs_t(t);
    jobs_t(t);
    ios_t(t);
    project_t(t);
    determinism_t(t);
}

// ---------------------------------------------------------------- the working directory

fn cwd_t(t: &mut T) {
    let mut e = tree_engine();
    t.eq("prompt at the start", e.cli_prompt(PC1), s("root@PC1:~# "));
    t.eq("pwd", sh(&mut e, "pwd"), s("/root\n"));
    let _ = sh(&mut e, "cd app");
    t.eq("prompt in ~/app", (e.cli_prompt(PC1), e.cli_cwd(PC1)), (s("root@PC1:~/app# "), s("/root/app")));
    t.eq("relative paths", sh(&mut e, "cat a.txt; ls; ls ..; cat ../notes.txt"), s("hallo\nwelt\na.txt  b1.txt  b2.txt  c.log  lib\napp  notes.txt\nNotiz\n"));
    t.eq("cd lib, .., -", sh(&mut e, "cd lib; pwd; cd ..; pwd; cd -; cd -"), s("/root/app/lib\n/root/app\n/root/app/lib\n/root/app\n"));
    t.eq("OLDPWD and PWD", sh(&mut e, "echo $PWD $OLDPWD"), s("/root/app /root/app/lib\n"));
    let _ = sh(&mut e, "cd /srv/ftp");
    t.eq("prompt outside ~", e.cli_prompt(PC1), s("root@PC1:/srv/ftp# "));
    let _ = sh(&mut e, "cd /");
    t.eq("prompt at /", e.cli_prompt(PC1), s("root@PC1:/# "));
    t.eq("relative from /", sh(&mut e, "ls root/app/lib; cat root/notes.txt"), s("Notiz\n"));
    let _ = sh(&mut e, "cd");
    t.eq("cd alone goes home", e.cli_prompt(PC1), s("root@PC1:~# "));
    let _ = sh(&mut e, "cd /tmp; cd ~/app/./lib/..");
    t.eq("~ and . and ..", e.cli_cwd(PC1), s("/root/app"));
    let _ = sh(&mut e, "cd ~root");
    t.eq("~root", e.cli_cwd(PC1), s("/root"));
    t.eq("no such directory", sh(&mut e, "cd nope; echo $?"), s("jfn-sh: cd: nope: No such file or directory\n1\n"));
    t.eq("not a directory", sh(&mut e, "cd notes.txt"), s("jfn-sh: cd: notes.txt: Not a directory\n"));
    t.eq("a made file of /etc", sh(&mut e, "cd /etc/hosts"), s("jfn-sh: cd: /etc/hosts: Not a directory\n"));
    t.eq("too many arguments", sh(&mut e, "cd a b"), s("jfn-sh: cd: too many arguments\n"));
    t.eq("cwd kept after errors", e.cli_cwd(PC1), s("/root"));
    let mut e2 = engine(&project());
    t.eq("OLDPWD not set", sh(&mut e2, "cd -"), s("jfn-sh: cd: OLDPWD not set\n"));
    let _ = sh(&mut e2, "unset HOME");
    t.eq("HOME not set", sh(&mut e2, "cd"), s("jfn-sh: cd: HOME not set\n"));
    // relative paths in every command: mkdir, touch, cp, mv, rm, rmdir, ls -l, tftp checks
    let _ = sh(&mut e, "cd app");
    let o = sh(&mut e, "mkdir neu; touch neu/x; cp a.txt neu/; mv neu/x neu/y; ls neu; rm neu/y neu/a.txt; rmdir neu; ls");
    t.eq("file commands take relative paths", o, s("a.txt  y\na.txt  b1.txt  b2.txt  c.log  lib\n"));
    t.eq("their messages name the path as typed", sh(&mut e, "rm nope; mkdir lib; rmdir nope"), s("rm: cannot remove 'nope': No such file or directory\nmkdir: cannot create directory 'lib': File exists\nrmdir: failed to remove 'nope': No such file or directory\n"));
    t.check("the store has the absolute paths", e.file_store(PC1).is_some_and(|s| s.exists("/root/app/a.txt") && !s.exists("/root/neu")));
}

// ---------------------------------------------------------------- the grammar

fn grammar_t(t: &mut T) {
    let mut e = tree_engine();
    t.eq("; runs one after the other", sh(&mut e, "echo a; echo b;"), s("a\nb\n"));
    t.eq("&& and ||", sh(&mut e, "false && echo nein; echo $?; false || echo ja; true && echo gut || echo schlecht"), s("1\nja\ngut\n"));
    t.eq("|| after a failure, && after success", sh(&mut e, "false || false && echo nie; echo $?; true || false && echo dann"), s("1\ndann\n"));
    t.eq("! turns the status", sh(&mut e, "! true; echo $?; ! false; echo $?"), s("1\n0\n"));
    t.eq("$? of a missing command", sh(&mut e, "nope; echo $?"), s("nope: command not found\n127\n"));
    t.eq("quotes", sh(&mut e, "echo 'a  b' \"c  $HOME\" \\$x a\\ b"), s("a  b c  /root $x a b\n"));
    t.eq("backslash in double quotes", sh(&mut e, "echo \"a\\\"b\\$c\\\\d\\e\""), s("a\"b$c\\d\\e\n"));
    t.eq("an empty argument", sh(&mut e, "printf '[%s]' '' x \"\"; echo"), s("[][x][]\n"));
    t.eq("variables", sh(&mut e, "X=5; echo ${X}0 $X $UNSET. \"$X\""), s("50 5 . 5\n"));
    t.eq("splitting of unquoted expansions", sh(&mut e, "A='1  2'; printf '[%s]' $A; echo; printf '[%s]' \"$A\"; echo"), s("[1][2]\n[1  2]\n"));
    t.eq("$$ $0 $#", sh(&mut e, "echo $$ $0 $#"), s("100 jfn-sh 0\n"));
    t.eq("an expansion to nothing is no argument", sh(&mut e, "printf '[%s]' $NOTHING; echo"), s("[]\n"));
    t.eq("a comment", sh(&mut e, "echo a # b; echo c"), s("a\n"));
    t.eq("# inside a word is none", sh(&mut e, "echo a#b"), s("a#b\n"));
    t.eq("NAME=value for one command", sh(&mut e, "Z=1 env | grep ^Z=; echo [$Z]"), s("Z=1\n[]\n"));
    t.eq("export", sh(&mut e, "export Y=2; env | grep ^Y=; unset Y; env | grep -c ^Y="), s("Y=2\n0\n"));
    t.eq("export a shell variable", sh(&mut e, "W=3; env | grep -c ^W=; export W; env | grep ^W="), s("0\nW=3\n"));
    t.eq("export with a bad name", sh(&mut e, "export 1x=2; echo $?"), s("jfn-sh: export: `1x=2': not a valid identifier\n1\n"));
    t.eq("syntax: a lone ;", sh(&mut e, "; ls"), s("jfn-sh: syntax error near unexpected token `;'\n"));
    t.eq("syntax: && at the end", sh(&mut e, "ls &&"), s("jfn-sh: syntax error: unexpected end of file\n"));
    t.eq("syntax: > without a file", sh(&mut e, "echo hi >"), s("jfn-sh: syntax error near unexpected token `newline'\n"));
    t.eq("syntax: || ||", sh(&mut e, "true || || true"), s("jfn-sh: syntax error near unexpected token `||'\n"));
    t.eq("syntax: an open quote", sh(&mut e, "echo \"abc"), s("jfn-sh: unexpected EOF while looking for matching `\"'\n"));
    t.eq("a syntax error runs nothing, $? 2", sh(&mut e, "echo vorher; echo hi >; echo $?"), s("jfn-sh: syntax error near unexpected token `;'\n"));
    t.eq("status 2", sh(&mut e, "echo $?"), s("2\n"));
    // history and !!
    let mut h = engine(&project());
    let _ = sh(&mut h, "echo eins");
    let _ = sh(&mut h, "echo zwei");
    t.eq("history", sh(&mut h, "history"), s("    1  echo eins\n    2  echo zwei\n    3  history\n"));
    t.eq("!!", sh(&mut h, "!!"), s("history\n    1  echo eins\n    2  echo zwei\n    3  history\n    4  history\n"));
    t.eq("!N and !-N", sh(&mut h, "!1; !-3"), s("echo eins; echo zwei\neins\nzwei\n"));
    t.eq("an event that is not there", sh(&mut h, "!99"), s("jfn-sh: !99: event not found\n"));
    t.eq("! in single quotes", sh(&mut h, "echo '!!' a!"), s("!! a!\n"));
    t.eq("history N, -c", sh(&mut h, "history 2; history -c; history"), s("    6  echo '!!' a!\n    7  history 2; history -c; history\n"));
    t.eq("ls hides dot files, -a and -A show them", sh(&mut h, "touch .x y; ls; ls -A; ls -a"), s("y\n.x  y\n.  ..  .x  y\n"));
}

// ---------------------------------------------------------------- redirections

fn redir_t(t: &mut T) {
    let mut e = tree_engine();
    t.eq("> >> <", sh(&mut e, "echo eins > f; echo zwei >> f; cat < f"), s("eins\nzwei\n"));
    t.eq("> replaces", sh(&mut e, "echo drei > f; cat f"), s("drei\n"));
    t.eq("2>", sh(&mut e, "cat nope 2> err; echo $?; cat err"), s("1\ncat: nope: No such file or directory\n"));
    t.eq("> f 2>&1", sh(&mut e, "cat f nope > both 2>&1; cat both"), s("drei\ncat: nope: No such file or directory\n"));
    t.eq("2>&1 > f: stderr stays on the terminal", sh(&mut e, "cat f nope 2>&1 > only; cat only"), s("cat: nope: No such file or directory\ndrei\n"));
    t.eq("&>", sh(&mut e, "cat f nope &> all; cat all"), s("drei\ncat: nope: No such file or directory\n"));
    t.eq("/dev/null", sh(&mut e, "cat nope 2>/dev/null; echo $?; echo weg > /dev/null; cat < /dev/null"), s("1\n"));
    t.eq("1>&2 into 2>", sh(&mut e, "echo fehler 1>&2 2>/dev/null; echo fehler 2>/dev/null 1>&2"), s("fehler\n"));
    t.eq("a missing directory", sh(&mut e, "echo x > /nowhere/y; echo $?"), s("jfn-sh: /nowhere/y: No such file or directory\n1\n"));
    t.eq("a directory", sh(&mut e, "echo x > app"), s("jfn-sh: app: Is a directory\n"));
    t.eq("< of a missing file runs nothing", sh(&mut e, "cat < nope; echo $?"), s("jfn-sh: nope: No such file or directory\n1\n"));
    t.eq("an ambiguous redirect", sh(&mut e, "echo x > app/b*.txt"), s("jfn-sh: app/b*.txt: ambiguous redirect\n"));
    t.eq("> alone makes the file", sh(&mut e, "> leer; cat leer; ls leer"), s("leer\n"));
    t.eq("the made files of /etc read", sh(&mut e, "grep PC1 < /etc/hostname"), s("PC1\n"));
    t.eq("relative to the working directory", sh(&mut e, "cd app; echo hier > h.txt; cd; cat app/h.txt"), s("hier\n"));
    // /etc/resolv.conf goes to the resolver as before
    let _ = sh(&mut e, "echo nameserver 10.0.0.10 > /etc/resolv.conf");
    t.check("resolv.conf", sh(&mut e, "cat /etc/resolv.conf").contains("nameserver 10.0.0.10"));
    t.eq("resolv.conf refuses", sh(&mut e, "echo nameserver nope > /etc/resolv.conf"), s("jfn-sh: /etc/resolv.conf: nope is no address\n"));
    // builtins and long-running commands too
    t.eq("pwd into a file", sh(&mut e, "pwd > p; cat p"), s("/root\n"));
    let o = cli(&mut e, PC1, &["ping -c 2 10.0.0.10 > ping.txt"], 3 * SEC);
    t.eq("a ping into a file prints nothing", o, s(""));
    let o = sh(&mut e, "head -2 ping.txt; tail -1 ping.txt");
    t.check("the file has its lines", o.starts_with("PING 10.0.0.10 (10.0.0.10) 56(84) bytes of data.\n64 bytes from 10.0.0.10: icmp_seq=1 ttl=64 time=") && o.contains("\nrtt min/avg/max/mdev = "));
    t.check("the file is a step of the project", e.files(PC1).iter().any(|f| f.path == "/root/ping.txt"));
}

// ---------------------------------------------------------------- pipes

fn pipe_t(t: &mut T) {
    let mut e = tree_engine();
    t.eq("sort | uniq -c", sh(&mut e, "printf 'b\\na\\nb\\n' | sort | uniq -c"), s("      1 a\n      2 b\n"));
    t.eq("ls into a pipe: a name a line", sh(&mut e, "ls /etc | wc -l; ls app | head -2"), s("3\na.txt\nb1.txt\n"));
    t.eq("2>&1 into a pipe", sh(&mut e, "cat nope 2>&1 | wc -l; cat nope | wc -l"), s("1\ncat: nope: No such file or directory\n0\n"));
    t.eq("status of the last command", sh(&mut e, "false | true; echo $?; true | false; echo $?"), s("0\n1\n"));
    t.eq("echo | cat", sh(&mut e, "echo hallo | cat"), s("hallo\n"));
    t.eq("cat - | cat -n", sh(&mut e, "printf 'x\\ny\\n' | cat - | cat -n"), s("     1\tx\n     2\ty\n"));
    // a ping on the left: its output comes when it is done
    let o = cli(&mut e, PC1, &["ping -c 3 10.0.0.10 | grep -c ttl"], 4 * SEC);
    t.eq("ping | grep -c", o, s("3\n"));
    let o = cli(&mut e, PC1, &["ping -c 1 10.0.0.10 | tail -1 | cut -d ' ' -f 1"], 2 * SEC);
    t.eq("ping | tail | cut", o, s("rtt\n"));
    // printf into nc: the request reaches the web server (the pipe of tools.rs before)
    let o = cli(&mut e, PC1, &["printf 'GET / HTTP/1.0\\r\\n\\r\\n' | nc 10.0.0.10 80 | head -1"], 2 * SEC);
    t.eq("printf | nc | head", o, s("HTTP/1.1 200 OK\r\n"));
    let o = cli(&mut e, PC1, &["curl -s http://10.0.0.10/ | wc -c"], 2 * SEC);
    t.eq("curl | wc", o, s("15\n"));
    // Ctrl+C: the rest of the pipeline filters what was collected, $? 130
    let _ = cli(&mut e, PC1, &["ping 10.0.0.10 | grep ttl"], 2500 * MS);
    t.check("the pipeline holds the console", e.cli_busy(PC1));
    let _ = e.take_reports();
    let _ = e.cli_interrupt(PC1);
    let o = out(&mut e, PC1);
    t.eq("Ctrl+C on a pipeline", o.lines().filter(|l| l.contains("ttl=64")).count(), 3);
    t.check("a line end first, then grep's lines", o.starts_with("\n64 bytes from 10.0.0.10: icmp_seq=1 ttl=64 "));
    t.eq("$? 130 and the rest goes", (e.cli_busy(PC1), sh(&mut e, "echo $?")), (false, s("130\n")));
    let _ = cli(&mut e, PC1, &["sleep 5; echo nie"], SEC);
    let _ = e.cli_interrupt(PC1);
    run_for(&mut e, 5 * SEC);
    t.eq("Ctrl+C ends the list", (out(&mut e, PC1), sh(&mut e, "echo $?")), (s("\n"), s("130\n")));
}

// ---------------------------------------------------------------- globs

fn glob_t(t: &mut T) {
    let mut e = tree_engine();
    t.eq("*", sh(&mut e, "echo app/*.txt"), s("app/a.txt app/b1.txt app/b2.txt\n"));
    t.eq("?", sh(&mut e, "echo app/b?.txt"), s("app/b1.txt app/b2.txt\n"));
    t.eq("[...]", sh(&mut e, "echo app/[bc]* app/[!ab]*"), s("app/b1.txt app/b2.txt app/c.log app/c.log app/lib\n"));
    t.eq("no match stays", sh(&mut e, "echo app/*.none; echo $?"), s("app/*.none\n0\n"));
    t.eq("quoted is no glob", sh(&mut e, "echo 'app/*.txt' \"app/*\" app/\\*"), s("app/*.txt app/* app/*\n"));
    t.eq("dot files only with a dot", sh(&mut e, "echo *; echo .h*"), s("app notes.txt\n.hidden\n"));
    t.eq("sorted by bytes", sh(&mut e, "touch app/B.txt app/_x; echo app/*"), s("app/B.txt app/_x app/a.txt app/b1.txt app/b2.txt app/c.log app/lib\n"));
    t.eq("directories in the middle", sh(&mut e, "echo */*.log /e*/h*"), s("app/c.log /etc/hostname /etc/hosts\n"));
    t.eq("a trailing /", sh(&mut e, "echo a*/"), s("app/\n"));
    t.eq("relative to the working directory", sh(&mut e, "cd app; echo *.log ../n*"), s("c.log ../notes.txt\n"));
    t.eq("for rm", sh(&mut e, "rm *.log; ls"), s("B.txt  _x  a.txt  b1.txt  b2.txt  lib\n"));
}

// ---------------------------------------------------------------- the tools

fn tools_t(t: &mut T) {
    let mut e = tree_engine();
    // mv
    t.eq("mv renames", sh(&mut e, "mv notes.txt n.txt; ls; cat n.txt"), s("app  n.txt\nNotiz\n"));
    t.eq("mv into a directory", sh(&mut e, "mv n.txt app; ls app"), s("a.txt  b1.txt  b2.txt  c.log  lib  n.txt\n"));
    t.eq("mv several", sh(&mut e, "mv app/b1.txt app/b2.txt app/lib; ls app/lib"), s("b1.txt  b2.txt\n"));
    t.eq("mv a directory", sh(&mut e, "mv app/lib app/lib2; ls app/lib2; ls app/lib"), s("b1.txt  b2.txt\nls: cannot access 'app/lib': No such file or directory\n"));
    t.eq("mv errors", sh(&mut e, "mv nope x; mv app app/lib2; mv a; mv /srv /x"), s("mv: cannot stat 'nope': No such file or directory\nmv: cannot move 'app' to a subdirectory of itself, 'app/lib2/app'\nmv: missing destination file operand after 'a'\nTry 'mv --help' for more information.\nmv: cannot move '/srv' to '/x': Permission denied\n"));
    t.eq("mv -n keeps", sh(&mut e, "echo neu > z; mv -n z app/n.txt; cat app/n.txt"), s("Notiz\n"));
    t.eq("mv several needs a directory", sh(&mut e, "mv z app/a.txt app/c.log"), s("mv: target 'app/c.log' is not a directory\n"));
    // head, tail
    t.eq("head", sh(&mut e, "seq 12 | head; seq 12 | head -n 3; seq 5 | head -2"), s("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n1\n2\n3\n1\n2\n"));
    t.eq("head -n -N, -c", sh(&mut e, "seq 5 | head -n -3; printf abcdef | head -c 3; echo"), s("1\n2\nabc\n"));
    t.eq("tail", sh(&mut e, "seq 12 | tail -n 2; seq 12 | tail -3; seq 5 | tail -n +4; printf abcdef | tail -c 2; echo"), s("11\n12\n10\n11\n12\n4\n5\nef\n"));
    t.eq("head of two files", sh(&mut e, "head -n 1 app/a.txt app/n.txt"), s("==> app/a.txt <==\nhallo\n\n==> app/n.txt <==\nNotiz\n"));
    t.eq("head errors", sh(&mut e, "head -n x app/a.txt; head nope; tail -f x"), s("head: invalid number of lines: 'x'\nhead: nope: No such file or directory\ntail: -f is not supported here: the file is read once\nTry 'tail --help' for more information.\n"));
    // wc
    t.eq("wc", sh(&mut e, "wc app/a.txt; wc -l app/a.txt; wc -w -c app/a.txt"), s(" 2  2 11 app/a.txt\n2 app/a.txt\n 2 11 app/a.txt\n"));
    t.eq("wc of a pipe", sh(&mut e, "echo hi | wc; echo hi | wc -l; printf 'ä' | wc -c -m"), s("      1       1       3\n1\n      1       2\n"));
    t.eq("wc of two files", sh(&mut e, "wc -l app/a.txt app/n.txt"), s(" 2 app/a.txt\n 1 app/n.txt\n 3 total\n"));
    // sort, uniq, cut
    t.eq("sort", sh(&mut e, "printf '10\\n9\\n100\\nB\\na\\n' | sort"), s("10\n100\n9\nB\na\n"));
    t.eq("sort -n -r -u -f", sh(&mut e, "printf '10\\n9\\n100\\n9\\n' | sort -n; printf '10\\n9\\n100\\n' | sort -rn; printf 'b\\na\\nb\\n' | sort -u; printf 'b\\nA\\na\\n' | sort -f"), s("9\n9\n10\n100\n100\n10\n9\na\nb\nA\na\nb\n"));
    t.eq("sort -k -t", sh(&mut e, "printf 'x 3\\ny 1\\nz 2\\n' | sort -k 2; printf 'a:2\\nb:1\\n' | sort -t : -k 2"), s("y 1\nz 2\nx 3\nb:1\na:2\n"));
    t.eq("uniq", sh(&mut e, "printf 'a\\na\\nb\\na\\n' | uniq; printf 'a\\na\\nb\\n' | uniq -d; printf 'a\\na\\nb\\n' | uniq -u; printf 'a\\nA\\n' | uniq -ci"), s("a\nb\na\na\nb\n      2 a\n"));
    t.eq("cut", sh(&mut e, "echo a:b:c | cut -d : -f 2; echo a:b:c | cut -d : -f 1,3; echo a:b:c | cut -d : -f 2-; echo abcdef | cut -c 2-4; echo ohne | cut -d : -f 2; echo ohne | cut -s -d : -f 2"), s("b\na:c\nb:c\nbcd\nohne\n"));
    t.eq("cut errors", sh(&mut e, "cut app/a.txt; cut -d ab -f 1 x"), s("cut: you must specify a list of bytes, characters, or fields\nTry 'cut --help' for more information.\ncut: the delimiter must be a single character\nTry 'cut --help' for more information.\n"));
    // tree, du, df
    let mut f = tree_engine();
    t.eq("tree", sh(&mut f, "tree"), s(".\n├── app\n│   ├── a.txt\n│   ├── b1.txt\n│   ├── b2.txt\n│   ├── c.log\n│   └── lib\n└── notes.txt\n\n2 directories, 5 files\n"));
    t.eq("tree -d -L", sh(&mut f, "mkdir app/lib/x; tree -d app; tree -L 1"), s("app\n└── lib\n    └── x\n\n2 directories\n.\n├── app\n└── notes.txt\n\n1 directory, 1 file\n"));
    t.eq("tree -a", sh(&mut f, "tree -a -L 1 | tail -1"), s("1 directory, 2 files\n"));
    t.eq("tree of nothing", sh(&mut f, "tree nope"), s("nope [error opening dir]\n\n0 directories, 0 files\n"));
    // (an empty file takes no block)
    t.eq("du", sh(&mut f, "du app; du -s .; du -sh app; du -a app/lib"), s("4\tapp/lib/x\n8\tapp/lib\n16\tapp\n28\t.\n16K\tapp\n4\tapp/lib/x\n8\tapp/lib\n"));
    t.eq("du of a file, -c, missing", sh(&mut f, "du app/a.txt; du -c app/a.txt notes.txt | tail -1; du nope"), s("4\tapp/a.txt\n8\ttotal\ndu: cannot access 'nope': No such file or directory\n"));
    let used = f.file_store(PC1).map(|s| s.used()).unwrap_or(0);
    t.eq("df", sh(&mut f, "df"), format!("Filesystem     1K-blocks Used Available Use% Mounted on\n/dev/sda1      536870912 {:>4} {} {:>4} /\n", used.div_ceil(1024), (536870912u64 * 1024 - used) / 1024, "1%"));
    t.eq("df -h", sh(&mut f, "df -h"), s("Filesystem     Size Used Avail Use% Mounted on\n/dev/sda1      512G 1.0K  512G   1% /\n"));
    // file, stat, basename, dirname
    t.eq("file", sh(&mut f, "printf 'a\\r\\nb' > crlf; printf '\\x01\\x02' > bin; file app app/a.txt app/b1.txt crlf bin nope"), s("app: directory\napp/a.txt: ASCII text\napp/b1.txt: empty\ncrlf: ASCII text, with CRLF line terminators\nbin: data\nnope: cannot open `nope' (No such file or directory)\n"));
    let o = sh(&mut f, "stat app/a.txt");
    t.check("stat", o.starts_with("  File: app/a.txt\n  Size: 11        \tBlocks: 8          IO Block: 4096   regular file\nAccess: (0644/-rw-r--r--)  Uid: (    0/    root)   Gid: (    0/    root)\nModify: 20") && o.ends_with(".000000000 +0000\n"));
    t.eq("stat of a directory, a missing file", sh(&mut f, "stat app | head -2 | tail -1; stat nope"), s("  Size: 4096      \tBlocks: 8          IO Block: 4096   directory\nstat: cannot statx 'nope': No such file or directory\n"));
    t.eq("basename", sh(&mut f, "basename /a/b/c.txt; basename /a/b/c.txt .txt; basename /a/b/; basename /; basename c"), s("c.txt\nc\nb\n/\nc\n"));
    t.eq("dirname", sh(&mut f, "dirname /a/b/c.txt; dirname c; dirname /a; dirname a/b/; dirname /"), s("/a/b\n.\n/\na\n/\n"));
    t.eq("basename without operand", sh(&mut f, "basename"), s("basename: missing operand\nTry 'basename --help' for more information.\n"));
    // date: the device's clock
    let clock = f.clock(PC1).unwrap_or(0).div_euclid(1_000_000_000);
    t.eq("date +%s", sh(&mut f, "date +%s"), format!("{}\n", clock));
    let c = datetime::Civil::from_unix(clock);
    t.eq("date +%F %T", sh(&mut f, "date '+%F %T %%'"), format!("{}-{:02}-{:02} {:02}:{:02}:{:02} %\n", c.year, c.month, c.day, c.hour, c.minute, c.second));
    t.check("date", sh(&mut f, "date").ends_with(&format!(" UTC {}\n", c.year)));
    t.eq("date cannot be set", sh(&mut f, "date 0101"), s("date: cannot set the date of a simulated device: '0101'\n"));
    // seq, tee, test, xargs
    t.eq("seq", sh(&mut f, "seq 3; seq 2 4; seq 10 -3 4; seq -s , 3; seq -w 8 10"), s("1\n2\n3\n2\n3\n4\n10\n7\n4\n1,2,3\n08\n09\n10\n"));
    t.eq("seq errors", sh(&mut f, "seq; seq x"), s("seq: missing operand\nTry 'seq --help' for more information.\nseq: invalid floating point argument: 'x'\nTry 'seq --help' for more information.\n"));
    t.eq("tee", sh(&mut f, "seq 2 | tee t1 t2 | wc -l; cat t1 t2; echo 3 | tee -a t1 > /dev/null; cat t1"), s("2\n1\n2\n1\n2\n1\n2\n3\n"));
    t.eq("test and [", sh(&mut f, "test -d app && echo d; [ -f app/a.txt ] && echo f; [ -e nope ] || echo kein; [ -s app/b1.txt ] || echo leer; [ -z \"\" ] && echo z; [ -n x ] && echo n"), s("d\nf\nkein\nleer\nz\nn\n"));
    t.eq("test numbers and strings", sh(&mut f, "[ 1 -lt 2 ]; echo $?; [ 3 -le 2 ]; echo $?; [ a = a ]; echo $?; [ a != a ]; echo $?; [ ! a = b ]; echo $?; [ 1 -eq 1 -a 2 -gt 1 ]; echo $?; [ 1 -eq 2 -o x ]; echo $?; [ ]; echo $?"), s("0\n1\n0\n1\n0\n0\n0\n1\n"));
    t.eq("test errors", sh(&mut f, "[ a -lt 2 ]; echo $?; [ 1 = 1; echo $?"), s("jfn-sh: test: a: integer expression expected\n2\njfn-sh: [: missing `]'\n2\n"));
    t.eq("xargs", sh(&mut f, "echo app/a.txt notes.txt | xargs cat; printf 'a\\nb\\n' | xargs; printf 'x y' | xargs echo -n; echo"), s("hallo\nwelt\nNotiz\na b\nx y\n"));
    // which, type, help, man
    t.eq("which", sh(&mut f, "which ls grep egrep cd nope; echo $?"), s("/usr/bin/ls\n/usr/bin/grep\n1\n"));
    t.eq("type", sh(&mut f, "type cd ls ip nope"), s("cd is a shell builtin\nls is /usr/bin/ls\nip is /usr/sbin/ip\njfn-sh: type: nope: not found\n"));
    let h = sh(&mut f, "help");
    t.check("help lists every command in our words", h.starts_with("Die Befehle dieser Shell (man BEFEHL zeigt mehr") && h.contains("\n  grep         Zeilen ausgeben, die zu einem Muster passen\n") && h.contains("\n  jfs          ein Skript als Prozess ausführen\n"));
    t.eq("help CMD", sh(&mut f, "help wc"), s("Aufruf: wc [-l] [-w] [-c] [-m] [DATEI ...]\nZeilen, Wörter und Bytes zählen.\n  -l           Zeilen\n  -w           Wörter\n  -c           Bytes\n  -m           Zeichen\n"));
    t.eq("CMD --help", sh(&mut f, "uniq --help | head -1"), s("Aufruf: uniq [-c] [-d] [-u] [-i] [DATEI]\n"));
    let m = sh(&mut f, "man sort");
    t.check("man", m.starts_with("NAME\n       sort - Zeilen sortieren\n\nAUFRUF\n       sort [-r] [-n] [-u] [-f] [-k N] [-t ZEICHEN] [DATEI ...]\n\nBESCHREIBUNG\n") && m.ends_with("\nWO\n       /usr/bin/sort\n"));
    t.check("man pages through the pager", f.cli_pager(PC1));
    t.check("man sh describes the grammar", sh(&mut f, "man sh").contains("Pipes | geben die Ausgabe des linken Befehls"));
    t.check("a page for every command word", {
        let w = ["cd", "pwd", "mv", "head", "tail", "grep", "find", "wc", "sort", "uniq", "cut", "tree", "du", "df", "less", "more", "history", "env", "export", "unset", "which", "type", "help", "man", "file", "stat", "basename", "dirname", "date", "seq", "tee", "test", "xargs", "sleep", "jfs", "jobs", "fg", "wait", "kill", "ps", "ls", "cat", "ip", "ping", "curl"];
        w.iter().all(|c| jfn_sim::cli::man::man(c).is_some())
    });
    t.eq("man of nothing", sh(&mut f, "man nope; echo $?"), s("man: zu nope gibt es keine Seite\n16\n"));
    t.check("no pager into a pipe", {
        let _ = sh(&mut f, "man ls | head -1");
        !f.cli_pager(PC1)
    });
    t.eq("less and more", sh(&mut f, "less app/a.txt; echo x | more; less app"), s("hallo\nwelt\nx\napp is a directory\n"));
    t.check("less pages", f.cli_pager(PC1));
    // env
    t.eq("env", sh(&mut f, "env"), s("HOME=/root\nHOSTNAME=PC1\nLOGNAME=root\nPATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\nPWD=/root\nSHELL=/bin/sh\nTERM=xterm\nUSER=root\n"));
    t.eq("export alone", sh(&mut f, "export | head -2"), s("declare -x HOME=\"/root\"\ndeclare -x LOGNAME=\"root\"\n"));
}

// grep and find
fn text_t(t: &mut T) {
    let mut e = tree_engine();
    let _ = sh(&mut e, "printf 'GET /a\\npost /b\\nGET /c x\\n# Kommentar\\n' > log.txt");
    t.eq("grep", sh(&mut e, "grep GET log.txt"), s("GET /a\nGET /c x\n"));
    t.eq("grep -i -n -v -c", sh(&mut e, "grep -in post log.txt; grep -v '^#' log.txt | grep -c ."), s("2:post /b\n3\n"));
    t.eq("grep -E and BRE", sh(&mut e, "grep -E '^(GET|post) /[ab]$' log.txt; grep '^\\(GET\\|post\\) /[ab]$' log.txt"), s("GET /a\npost /b\nGET /a\npost /b\n"));
    t.eq("grep -w -x -o", sh(&mut e, "grep -w x log.txt; grep -x 'post /b' log.txt; grep -o '/[a-z]' log.txt"), s("GET /c x\npost /b\n/a\n/b\n/c\n"));
    t.eq("grep -F", sh(&mut e, "grep -F '/c x' log.txt; grep -cF . log.txt"), s("GET /c x\n0\n"));
    t.eq("grep several files, -l, -h, -H", sh(&mut e, "grep -l a app/a.txt log.txt notes.txt; grep -h hallo app/a.txt log.txt; grep -H Notiz notes.txt"), s("app/a.txt\nlog.txt\nhallo\nnotes.txt:Notiz\n"));
    t.eq("grep -r", sh(&mut e, "grep -r welt; grep -rc welt app"), s("./app/a.txt:welt\napp/a.txt:1\napp/b1.txt:0\napp/b2.txt:0\napp/c.log:0\n"));
    t.eq("grep -e twice, -q", sh(&mut e, "grep -e hallo -e welt app/a.txt; grep -q hallo app/a.txt; echo $?"), s("hallo\nwelt\n0\n"));
    t.eq("grep status", sh(&mut e, "grep zz log.txt; echo $?; grep x nope; echo $?; grep '(' log.txt; echo $?"), s("1\ngrep: nope: No such file or directory\n2\n1\n"));
    t.eq("grep errors", sh(&mut e, "grep -E '(' log.txt; echo $?; grep '[a' log.txt; grep 'a\\{2' x; grep; grep -Z x"), s("grep: Unmatched ( or \\(\n2\ngrep: Unmatched [, [^, [:, [., or [=\ngrep: Unmatched \\{\nUsage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information.\ngrep: invalid option -- 'Z'\nTry 'grep --help' for more information.\n"));
    t.eq("grep a directory", sh(&mut e, "grep x app; echo $?"), s("grep: app: Is a directory\n2\n"));
    t.eq("grep from a pipe", sh(&mut e, "cat log.txt | grep -c GET; echo x | grep -H x"), s("2\n(standard input):x\n"));
    t.eq("egrep", sh(&mut e, "egrep 'a$|b$' log.txt"), s("GET /a\npost /b\n"));
    t.eq("grep -i of a class", sh(&mut e, "echo ÄÖ | grep -i 'äö'"), s("ÄÖ\n"));
    // find
    t.eq("find (dot files too)", sh(&mut e, "find"), s(".\n./.hidden\n./app\n./app/a.txt\n./app/b1.txt\n./app/b2.txt\n./app/c.log\n./app/lib\n./log.txt\n./notes.txt\n"));
    t.eq("find -name", sh(&mut e, "find . -name '*.txt'"), s("./app/a.txt\n./app/b1.txt\n./app/b2.txt\n./log.txt\n./notes.txt\n"));
    t.eq("find -type, -maxdepth, -mindepth, -iname", sh(&mut e, "find /root -type d; find app -maxdepth 1 -name 'b*'; find app -mindepth 1 -maxdepth 1 -type d; find . -iname 'NOTES*'"), s("/root\n/root/app\n/root/app/lib\napp/b1.txt\napp/b2.txt\napp/lib\n./notes.txt\n"));
    t.eq("find errors", sh(&mut e, "find nope; find . -foo; find . -type x; find . -maxdepth"), s("find: 'nope': No such file or directory\nfind: unknown predicate `-foo'\nfind: Unknown argument to -type: x\nfind: missing argument to `-maxdepth'\n"));
    t.eq("find / with the made files of /etc", sh(&mut e, "find /etc"), s("/etc\n/etc/hostname\n/etc/hosts\n/etc/resolv.conf\n"));
    t.eq("find | xargs grep", sh(&mut e, "find app -name '*.txt' | xargs grep -l welt"), s("app/a.txt\n"));
}

// ---------------------------------------------------------------- Tab

fn tab_t(t: &mut T) {
    let mut e = tree_engine();
    let _ = sh(&mut e, "mkdir app/local; touch app/server.jfs app/service.jfs 'mit leer'");
    let tab = |e: &Engine, l: &str| e.cli_complete(PC1, l);
    t.eq("a file", tab(&e, "cat no"), (Some(s("cat notes.txt ")), alloc::vec![s("notes.txt")]));
    t.eq("a directory gets /", tab(&e, "cd ap"), (Some(s("cd app/")), alloc::vec![s("app/")]));
    t.eq("in a directory", tab(&e, "cat app/a"), (Some(s("cat app/a.txt ")), alloc::vec![s("a.txt")]));
    t.eq("the common prefix", tab(&e, "cat app/se"), (Some(s("cat app/serv")), alloc::vec![s("server.jfs"), s("service.jfs")]));
    t.eq("several choices", tab(&e, "ls app/l"), (None, alloc::vec![s("lib/"), s("local/")]));
    t.eq("everything in a directory", tab(&e, "ls app/").1, alloc::vec![s("a.txt"), s("b1.txt"), s("b2.txt"), s("c.log"), s("lib/"), s("local/"), s("server.jfs"), s("service.jfs")]);
    t.eq("absolute", tab(&e, "ls /sr").0, Some(s("ls /srv/")));
    t.eq("~", tab(&e, "ls ~/ap").0, Some(s("ls ~/app/")));
    t.eq("..", tab(&e, "cd app/../no").0, Some(s("cd app/../notes.txt ")));
    t.eq("a space is escaped", tab(&e, "cat mi").0, Some(s("cat mit\\ leer ")));
    t.eq("dot files with a dot", (tab(&e, "cat .h").0, tab(&e, "cat ").1.contains(&s(".hidden"))), (Some(s("cat .hidden ")), false));
    t.eq("nothing", tab(&e, "cat zz"), (None, Vec::new()));
    t.eq("the made files of /etc", tab(&e, "cat /etc/res").0, Some(s("cat /etc/resolv.conf ")));
    t.eq("after a pipe: a command", tab(&e, "cat x | gre").0, Some(s("cat x | grep ")));
    t.eq("after > a path", tab(&e, "echo x > no").0, Some(s("echo x > notes.txt ")));
    t.eq("new command words", (tab(&e, "his").0, tab(&e, "xar").0, tab(&e, "jf").0), (Some(s("history ")), Some(s("xargs ")), Some(s("jfs "))));
    t.eq("the objects of ip as before", (tab(&e, "ip ro").0, tab(&e, "ip addr s").1), (Some(s("ip route ")), alloc::vec![s("set"), s("show")]));
    let _ = sh(&mut e, "cd app");
    t.eq("relative to the working directory", tab(&e, "cat ser").0, Some(s("cat serv")));
}

// ---------------------------------------------------------------- processes

fn jfs_t(t: &mut T) {
    let mut e = engine(&project());
    let _ = sh(&mut e, "mkdir app; echo 'print(\"hallo \" + config.args[0]); log(\"gestartet\")' > app/hello.jfs");
    let o = cli(&mut e, PC1, &["jfs app/hello.jfs Welt"], 0);
    t.eq("jfs prints and holds the console for its CPU time", (o, e.cli_busy(PC1)), (s("hallo Welt\n"), true));
    run_for(&mut e, MS);
    t.eq("then it ends", (out(&mut e, PC1), e.cli_busy(PC1)), (s(""), false));
    t.eq("exit code 0", sh(&mut e, "echo $?"), s("0\n"));
    t.check("log goes to the journal", e.log_lines(PC1).iter().any(|l| l.tag == "hello" && l.text == "gestartet"));
    let _ = sh(&mut e, "cd app");
    let o = cli(&mut e, PC1, &["jfs hello.jfs Du; echo $?"], MS);
    t.eq("relative, in a list", o, s("hallo Du\n0\n"));
    let _ = sh(&mut e, "echo 'print(\"erst\"); timer.after(500, fn() { print(\"später\") })' > later.jfs");
    let o = cli(&mut e, PC1, &["jfs later.jfs"], 100 * MS);
    t.eq("a timer keeps it running", (o, e.cli_busy(PC1)), (s("erst\n"), true));
    run_for(&mut e, 500 * MS);
    t.eq("until the timer is done", (out(&mut e, PC1), e.cli_busy(PC1)), (s("später\n"), false));
    let _ = sh(&mut e, "echo 'log(\"ein Eintrag\"); let x = 1 / 0' > bad.jfs");
    let o = cli(&mut e, PC1, &["jfs bad.jfs; echo $?"], 10 * MS);
    t.eq("an error: stderr and 1", o, s("bad.jfs:1:31: Fehler: Division durch 0\n1\n"));
    t.eq("its stderr goes to 2>", cli(&mut e, PC1, &["jfs bad.jfs 2> err.txt; cat err.txt"], 10 * MS), s("bad.jfs:1:31: Fehler: Division durch 0\n"));
    let _ = sh(&mut e, "echo 'let = 3' > syn.jfs");
    t.eq("a compile error", sh(&mut e, "jfs syn.jfs; echo $?"), s("syn.jfs:1:5: Fehler: Nach let fehlt ein Name\n1\n"));
    t.eq("no such file", sh(&mut e, "jfs nope.jfs; echo $?"), s("jfs: nope.jfs: No such file or directory\n1\n"));
    t.eq("no file", sh(&mut e, "jfs; echo $?"), s("jfs: welches Skript? Aufruf: jfs DATEI.jfs [ARGUMENTE ...]\n2\n"));
    // into a pipe and a file
    let o = cli(&mut e, PC1, &["jfs later.jfs | grep -c ."], SEC);
    t.eq("jfs | grep", o, s("2\n"));
    let _ = cli(&mut e, PC1, &["jfs later.jfs > later.log"], SEC);
    t.eq("jfs > FILE", sh(&mut e, "cat later.log"), s("erst\nspäter\n"));
    // a process's relative imports start in its file's directory, not in the working directory
    let _ = sh(&mut e, "mkdir lib; echo 'fn twice(x) { return x * 2 }' > lib/util.jfs");
    let _ = sh(&mut e, "echo 'let u = import(\"lib/util.jfs\"); print(u.twice(21))' > main.jfs; cd ~");
    t.eq("imports beside the script", cli(&mut e, PC1, &["jfs app/main.jfs"], MS), s("42\n"));
    let _ = sh(&mut e, "cd app");
    // the network from a process: a request to the web server
    let _ = sh(&mut e, "echo 'http.request(\"http://10.0.0.10/\", fn(err, r) { print(r.status, r.body) })' > get.jfs");
    t.eq("http from a process", cli(&mut e, PC1, &["jfs get.jfs"], SEC), s("200 Hallo von SRV1\n\n"));
    // Ctrl+C ends a script that never ends by itself
    let _ = sh(&mut e, "echo 'timer.every(100, fn() { print(\"tick\") })' > forever.jfs");
    let o = cli(&mut e, PC1, &["jfs forever.jfs"], 350 * MS);
    t.eq("it runs", (o, e.cli_busy(PC1)), (s("tick\ntick\ntick\n"), true));
    let _ = e.cli_interrupt(PC1);
    t.eq("Ctrl+C", (out(&mut e, PC1), e.cli_busy(PC1), sh(&mut e, "echo $?")), (s("\n"), false, s("130\n")));
    run_for(&mut e, SEC);
    t.eq("it is gone", out(&mut e, PC1), s(""));
    t.check("no service is left", e.services(PC1).is_empty());
}

fn jobs_t(t: &mut T) {
    let mut e = engine(&project());
    // & and its lines
    let o = sh(&mut e, "sleep 2 &");
    t.check("[1] PID", o.starts_with("[1] 1") && o.ends_with('\n') && !e.cli_busy(PC1));
    let pid: u32 = o.trim().rsplit(' ').next().and_then(|p| p.parse().ok()).unwrap_or(0);
    t.eq("$!", sh(&mut e, "echo $!"), format!("{}\n", pid));
    t.eq("jobs", sh(&mut e, "jobs; jobs -l; jobs -p"), format!("[1]+  Running                 sleep 2 &\n[1]+ {} Running                 sleep 2 &\n{}\n", pid, pid));
    run_for(&mut e, 3 * SEC);
    t.eq("the done line comes with the next prompt", out(&mut e, PC1), s(""));
    t.eq("Done", sh(&mut e, ""), s("[1]+  Done                    sleep 2\n"));
    t.eq("it is gone", sh(&mut e, "jobs"), s(""));
    // output of a background job reaches the terminal
    let _ = sh(&mut e, "echo 'print(\"erst\"); timer.after(300, fn() { print(\"später\") })' > later.jfs");
    let o = sh(&mut e, "jfs later.jfs &");
    t.check("[1] then its first output", o.starts_with("[1] ") && o.ends_with("\nerst\n"));
    let o = cli(&mut e, PC1, &[], 500 * MS);
    t.eq("its later output while the console is free", o, s("später\n"));
    t.eq("Done after the next line", sh(&mut e, "echo x"), s("x\n[1]+  Done                    jfs later.jfs\n"));
    // a status, two jobs, + and -
    let _ = sh(&mut e, "false &");
    t.eq("Exit 1", sh(&mut e, ""), s(""));
    let _ = sh(&mut e, "sleep 10 &");
    let _ = sh(&mut e, "sleep 20 &");
    t.eq("two jobs", sh(&mut e, "jobs"), s("[1]-  Running                 sleep 10 &\n[2]+  Running                 sleep 20 &\n"));
    // ps
    let ps = sh(&mut e, "ps");
    let lines: Vec<&str> = ps.lines().collect();
    // X1e added the TTY column (cli/ses.rs: the console is tty1, services have none)
    t.eq("ps head", lines.first().copied(), Some("  PID USER     TTY      TIME COMMAND"));
    t.check("ps: init, the shell, the jobs, ps", lines.get(1) == Some(&"    1 root     ?        0:00 init") && lines.get(2) == Some(&"  100 root     tty1     0:00 -sh") && lines.iter().any(|l| l.ends_with(" root     tty1     0:00 sleep 10")) && lines.iter().any(|l| l.ends_with(" root     tty1     0:00 sleep 20")) && lines.last().is_some_and(|l| l.ends_with(" ps")));
    let srv = cli(&mut e, SRV1, &["ps"], 0);
    t.check("ps of a server: its services", srv.contains("  101 root     ?        0:00 web\n"));
    t.eq("kill a service", cli(&mut e, SRV1, &["kill 101; echo $?"], 0), s("jfn-sh: kill: (101) - Operation not permitted\n1\n"));
    // kill
    t.eq("kill %1", sh(&mut e, "kill %1"), s("[1]-  Terminated              sleep 10\n"));
    t.eq("kill -9 PID", { let p2: u32 = sh(&mut e, "jobs -p").trim().parse().unwrap_or(0); sh(&mut e, &format!("kill -9 {}", p2)) }, s("[2]+  Killed                  sleep 20\n"));
    t.eq("kill errors", sh(&mut e, "kill 9999; kill %5; kill x; kill; echo $?"), s("jfn-sh: kill: (9999) - No such process\njfn-sh: kill: %5: no such job\njfn-sh: kill: x: arguments must be process or job IDs\nkill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]\n2\n"));
    t.eq("kill -l", sh(&mut e, "kill -l"), s(" 1) SIGHUP\n 2) SIGINT\n 9) SIGKILL\n15) SIGTERM\n"));
    t.eq("kill of init and the shell does nothing", sh(&mut e, "kill 1 100; echo $?"), s("0\n"));
    // fg and wait
    let _ = sh(&mut e, "sleep 1 && echo fertig &");
    let o = sh(&mut e, "fg");
    t.eq("fg shows the job and waits", (o, e.cli_busy(PC1)), (s("sleep 1 && echo fertig\n"), true));
    run_for(&mut e, 2 * SEC);
    t.eq("its output, no done line", (out(&mut e, PC1), e.cli_busy(PC1), sh(&mut e, "echo $?")), (s("fertig\n"), false, s("0\n")));
    let _ = sh(&mut e, "sleep 5 &");
    let _ = sh(&mut e, "fg %1");
    let _ = e.cli_interrupt(PC1);
    t.eq("Ctrl+C in fg ends the job", (e.cli_busy(PC1), sh(&mut e, "echo $?; jobs")), (false, s("130\n")));
    t.eq("fg without jobs", sh(&mut e, "fg; echo $?"), s("jfn-sh: fg: current: no such job\n1\n"));
    let _ = sh(&mut e, "sleep 1 &");
    let _ = cli(&mut e, PC1, &["wait; echo gewartet"], 2 * SEC);
    t.eq("wait", out(&mut e, PC1), s(""));
    t.eq("wait: no note", sh(&mut e, "echo $?"), s("0\n"));
    // sleep
    let o = cli(&mut e, PC1, &["sleep 1.5; echo wach"], SEC);
    t.eq("sleep holds the console", (o, e.cli_busy(PC1)), (s(""), true));
    run_for(&mut e, SEC);
    t.eq("then the list goes on", out(&mut e, PC1), s("wach\n"));
    t.eq("sleep errors", sh(&mut e, "sleep; sleep x"), s("sleep: missing operand\nTry 'sleep --help' for more information.\nsleep: invalid time interval 'x'\nTry 'sleep --help' for more information.\n"));
    t.eq("sleep 0 and 1m", (sh(&mut e, "sleep 0; echo $?"), jfn_sim::cli::sh::sleep_ns("1m"), jfn_sim::cli::sh::sleep_ns("0.5")), (s("0\n"), Some(60 * SEC), Some(500 * MS)));
    // lines typed while a command runs wait, with their echo
    let o = cli(&mut e, PC1, &["sleep 1", "echo danach"], 2 * SEC);
    t.eq("type-ahead", o, s("root@PC1:~# echo danach\ndanach\n"));
    // exit codes of the network programs
    let o = cli(&mut e, PC1, &["ping -c 1 10.0.0.10 > /dev/null; echo $?"], 2 * SEC);
    t.eq("ping with a reply: 0", o, s("0\n"));
    let o = cli(&mut e, PC1, &["ping -c 1 -W 1 10.0.0.99 > /dev/null; echo $?"], 3 * SEC);
    t.eq("ping without: 1", o, s("1\n"));
    let o = cli(&mut e, PC1, &["ping -c 1 nohost; echo $?"], 2 * SEC);
    t.eq("ping of an unknown name: 2", o, s("ping: nohost: Temporary failure in name resolution\n2\n"));
    let o = cli(&mut e, PC1, &["curl -s http://10.0.0.99:81/ -m 1; echo $?"], 3 * SEC);
    t.eq("curl's exit code", o, s("28\n"));
    // a background ping into a file
    let _ = sh(&mut e, "ping -c 1 10.0.0.10 > p.txt &");
    run_for(&mut e, SEC);
    t.check("a background job's redirection", sh(&mut e, "cat p.txt").starts_with("PING 10.0.0.10"));
    // power off: the processes go, the shell starts afresh
    let _ = sh(&mut e, "cd /tmp; sleep 100 &");
    let _ = cli(&mut e, PC1, &["echo 'timer.every(100, fn() { })' > f.jfs; jfs f.jfs; sleep 50"], 0);
    t.check("a command in the foreground before", e.cli_busy(PC1));
    let _ = e.apply(jfn_sim::Action::Power { dev: PC1, on: false });
    let _ = e.apply(jfn_sim::Action::Power { dev: PC1, on: true });
    run_for(&mut e, SEC);
    let _ = e.take_reports();
    t.eq("power off: a fresh shell, the console free", (e.cli_busy(PC1), sh(&mut e, "jobs; pwd; history"), e.cli_prompt(PC1)), (false, s("/root\n    1  jobs; pwd; history\n"), s("root@PC1:~# ")));
    t.check("the process's service went", e.service_ids(PC1).is_empty());
}

// ---------------------------------------------------------------- IOS-like flash: directories

fn ios_t(t: &mut T) {
    let mut e = engine(&project());
    let r = |e: &mut Engine, l: &str| cli(e, R1, &[l], 0);
    let _ = r(&mut e, "enable");
    t.eq("pwd", r(&mut e, "pwd"), s("flash:/\n"));
    t.eq("mkdir", r(&mut e, "mkdir backups"), s("Directory flash:/backups made.\n"));
    let _ = r(&mut e, "cd backups");
    t.eq("cd and pwd", (r(&mut e, "pwd"), e.cli_cwd(R1)), (s("flash:/backups/\n"), s("flash:/backups")));
    let run = e.running_config(R1);
    t.eq("copy into the working directory", r(&mut e, "copy running-config r1.cfg"), format!("{} bytes copied to flash:/backups/r1.cfg\n", run.len()));
    let o = r(&mut e, "dir");
    t.check("dir of the working directory", o.starts_with("Directory of flash:/backups/\n\n    1  -rw- ") && o.contains("  r1.cfg\n"));
    t.eq("more of a relative name", r(&mut e, "more r1.cfg"), run.clone());
    let _ = r(&mut e, "cd ..");
    let o = r(&mut e, "dir");
    t.check("dir shows the directory", o.starts_with("Directory of flash:/\n\n    1  -rw-    42262528  ") && o.contains("\n    2  drw-           0  ") && o.contains("  backups\n"));
    t.check("dir DIR", r(&mut e, "dir backups").starts_with("Directory of flash:/backups/\n"));
    t.eq("rmdir of a full directory", r(&mut e, "rmdir backups"), s("%Error removing dir flash:/backups (Directory not empty)\n"));
    t.eq("rename", r(&mut e, "rename backups/r1.cfg old.cfg"), s(""));
    t.check("rename moved it", e.file_store(R1).is_some_and(|s| s.exists("flash:/old.cfg") && !s.exists("flash:/backups/r1.cfg")));
    t.eq("rename into a directory", r(&mut e, "rename old.cfg backups"), s(""));
    t.check("there", e.file_store(R1).is_some_and(|s| s.exists("flash:/backups/old.cfg")));
    t.eq("delete with a path", r(&mut e, "delete /force flash:backups/old.cfg"), s(""));
    t.eq("rmdir", r(&mut e, "rmdir backups"), s("Directory flash:/backups removed.\n"));
    t.eq("cd errors", (r(&mut e, "cd nope"), r(&mut e, "cd jf-r4.bin"), r(&mut e, "cd nvram:")), (s("%Error changing directory to flash:/nope (No such file or directory)\n"), s("%Error changing directory to flash:/jf-r4.bin (Not a directory)\n"), s("%Error: nvram:/ has no directories to go to\n")));
    t.eq("mkdir errors", (r(&mut e, "mkdir a"), r(&mut e, "mkdir a"), r(&mut e, "mkdir nvram:x")), (s("Directory flash:/a made.\n"), s("%Error creating dir flash:/a (File exists)\n"), s("%Error creating dir nvram:/x (Permission denied)\n")));
    t.eq("rename errors", (r(&mut e, "rename nope x"), r(&mut e, "rename jf-r4.bin x"), r(&mut e, "rename a a/b")), (s("%Error renaming flash:/nope (No such file or directory)\n"), s("%Error renaming flash:/jf-r4.bin (Permission denied)\n"), s("%Error renaming flash:/a (Invalid argument)\n")));
    let _ = r(&mut e, "cd a");
    let o = r(&mut e, "copy running-config flash:");
    t.eq("copy asks with the working directory", (o, e.cli_prompt(R1)), (s(""), s("Name in flash: [running-config]? ")));
    let o = r(&mut e, "x.cfg");
    t.check("the answer is in the working directory", o.contains("copied to flash:/a/x.cfg") && e.file_store(R1).is_some_and(|s| s.exists("flash:/a/x.cfg")));
    let h = e.cli_help(R1, "");
    t.check("help shows the new commands", h.contains("\n  cd ") && h.contains("\n  mkdir ") && h.contains("\n  pwd ") && h.contains("\n  rename ") && h.contains("\n  rmdir "));
    let _ = r(&mut e, "disable");
    let _ = r(&mut e, "mkdir b");
    t.eq("user level: cd, pwd, no mkdir", (r(&mut e, "pwd"), e.file_store(R1).is_some_and(|s| s.exists("flash:/a/b"))), (s("flash:/a/\n"), false));
    // the store: paths with directories
    let mut ios = Store::new(true, 1 << 20, None);
    t.eq("flash: paths with directories", [ios.norm("flash:a/b"), ios.norm("flash:/a/../b"), ios.norm("a/./b/")], [Ok(s("flash:/a/b")), Ok(s("flash:/b")), Ok(s("flash:/a/b"))]);
    t.eq("parent and name", (ios.parent("flash:/a/b"), ios.parent("flash:/a"), ios.name_of("flash:/a/b")), (s("flash:/a"), s("flash:/"), "b"));
    t.eq("mkdir, write, list", (ios.mkdir("flash:a", 0, false), ios.write("flash:a/f", b"x", 0), ios.list("flash:a").map(|v| v.len())), (Ok(()), Ok(()), Ok(1)));
    t.eq("no file in a missing directory", ios.write("flash:z/f", b"x", 0), Err(FsErr::NotFound));
    t.eq("rename keeps what is inside", (ios.rename("flash:a", "flash:c"), ios.exists("flash:/c/f"), ios.exists("flash:/a")), (Ok(()), true, false));
    t.eq("not into itself, not between file systems", (ios.rename("flash:c", "flash:c/d"), ios.rename("flash:c", "nvram:c")), (Err(FsErr::BadName), Err(FsErr::ReadOnly)));
}

// ---------------------------------------------------------------- a project with folders

fn project_t(t: &mut T) {
    let mut e = tree_engine();
    let _ = cli(&mut e, R1, &["enable", "mkdir conf", "copy running-config conf/r1.cfg"], 0);
    let mut p = project();
    for dev in [PC1, R1] {
        let files = e.files(dev);
        if let Some(d) = p.device_mut(dev) {
            d.files = files;
        }
    }
    let text = p.to_text();
    t.check("the project file has the folders", text.contains("\"path\": \"/root/app/lib\"") && text.contains("\"dir\": true") && text.contains("\"path\": \"flash:/conf\"") && text.contains("\"path\": \"flash:/conf/r1.cfg\""));
    let Ok(q) = Project::from_text(&text) else {
        t.check("the project loads", false);
        return;
    };
    let mut f = engine(&q);
    t.eq("Linux tree after loading", sh(&mut f, "find app; cat app/a.txt"), s("app\napp/a.txt\napp/b1.txt\napp/b2.txt\napp/c.log\napp/lib\nhallo\nwelt\n"));
    let _ = cli(&mut f, R1, &["enable"], 0);
    t.check("flash: tree after loading", cli(&mut f, R1, &["dir conf"], 0).contains("  r1.cfg\n"));
    t.eq("the same records", (f.files(PC1), f.files(R1)), (e.files(PC1), e.files(R1)));
}

// ---------------------------------------------------------------- determinism

fn determinism_t(t: &mut T) {
    let go = || {
        let mut e = tree_engine();
        let mut o = String::new();
        o.push_str(&cli(&mut e, PC1, &["echo 'timer.every(50, fn() { print(rand(100)) })' > r.jfs", "jfs r.jfs | head -3 &", "ping -c 2 10.0.0.10 | tail -1 &", "sleep 1; ps; jobs"], 2 * SEC));
        let _ = e.cli_line(PC1, "kill %1");
        run_for(&mut e, SEC);
        o.push_str(&out(&mut e, PC1));
        o.push_str(&sh(&mut e, "find / -name '*.jfs'; history"));
        (o, jfn_sim::keyframe::digest(&e))
    };
    let (a, b) = (go(), go());
    t.check("twice the same output and state", a == b && a.0.contains("[1] ") && a.0.contains("rtt min/avg/max/mdev"));
    // K1: a replay from the keyframes over a shell with jobs, pipes and a process equals the run
    let mut d = jfn_sim::Inline::new(engine(&project()));
    d.rewind_on(jfn_sim::Budget { gap_events: 20, ..jfn_sim::Budget::default() });
    let steps: [(&str, u64); 6] = [("mkdir -p a/b; cd a", 0), ("echo 'timer.every(100, fn() { print(\"t\") })' > t.jfs", 0), ("jfs t.jfs | head -2 &", 0), ("ping -c 2 10.0.0.10 | tail -1 > p &", 300 * MS), ("sleep 1; echo x > b/x", 2 * SEC), ("kill %1; ls; jobs", SEC)];
    for (line, ns) in steps {
        let _ = d.send(jfn_sim::Cmd::Cli { dev: PC1, line: String::from(line) }, 0);
        let _ = d.send(jfn_sim::Cmd::RunFor(ns), 0);
        let _ = d.reports();
    }
    let pos = d.pos();
    let past = d.restore_pos(pos, None).and_then(|mut r| {
        let rw = d.rewind()?;
        r.run(rw, usize::MAX).then(|| r.finish())
    });
    t.check("a replay of the shell equals the run", past.is_some_and(|p| jfn_sim::keyframe::digest(&p.eng) == jfn_sim::keyframe::digest(&d.eng) && p.eng.cli_cwd(PC1) == "/root/a"));
    // the shell's session counts in a keyframe
    let mut e = engine(&project());
    let before = jfn_sim::keyframe::Footprint::of(&e).devices;
    let _ = sh(&mut e, "echo 0123456789012345678901234567890123456789; sleep 100 &");
    t.check("the footprint grows with history and jobs", jfn_sim::keyframe::Footprint::of(&e).devices > before + 64);
}

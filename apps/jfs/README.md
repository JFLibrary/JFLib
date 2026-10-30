# jfs

`jfs` runs scripts of the language of [`lib/script`](../../lib/script/lib.rs) (`.jfs`, the
scripts of JFNetworker) on the command line, the way `python file.py` runs a Python file.
[GUIDE.md §29](../../docs/GUIDE.md#29-scripts-script-compiler-and-vm) describes the language and
its standard library.

```sh
python build_rs.py app jfs                    # build/win/jfs.exe and build/linux/jfs
build/win/jfs.exe hello.jfs Anna Ben          # run a script; Anna and Ben are config.args
build/win/jfs.exe -e "print(1 + 2)"           # a one-liner
build/win/jfs.exe - < hello.jfs               # the script from standard input
build/win/jfs.exe --check hello.jfs           # only the diagnostics (line:column)
build/win/jfs.exe --test tests/*.jfs          # the functions test_* of the files
build/win/jfs.exe --debug app.jfs             # in the console debugger
python build_rs.py test jfs                   # the example cases (Windows and Linux)
```

## Command line

| Option | Meaning |
|---|---|
| `--check` | only the diagnostics, on stdout: `FILE:LINE:COL: Fehler: …` and the warnings; exit code 1 when one is an error |
| `--test` | every argument is a file of tests: their functions `test_*` run, each in a fresh VM (below) |
| `--debug` | the script in the console debugger, its commands on standard input (below) |
| `--virtual-time` | a virtual clock: timers run without waiting, `now()` starts at 0 and stands still while code runs |
| `--until MS` | the event loop ends when the clock reaches MS milliseconds (exit code 0) |
| `--seed N` | `rand()` from a fixed seed (else a random one) |
| `--disk FILE` | `disk.*` keeps its values in FILE, a JSON object with one key per line (else in memory) |
| `-c`, `--config K=V` | sets `config.K`; V is read as JSON (`7000`, `true`, `"x"`, `[1, 2]`) or else taken as text; a JSON object sets several keys |
| `--env NAME` | the environment variable NAME as `config.env.NAME` |
| `--fuel N` | steps per invocation (0: no limit; default 10 000 000) |
| `--heap N` | heap bytes, also `64k`, `16M`, `1G` (default 16M) |
| `--depth N`, `--stack N` | nested calls (500), values on the stack (100 000) |
| `--selftest DIR` | the example cases `DIR/*.test` (below) |

Options come before the script; everything after the script belongs to it (`config.args`).
`JFS_CONFIG` in the environment is a JSON object that the `--config` options build on.
Exit codes: 0 done, 1 an error of the script (compiler, an uncaught exception, a limit), 2 a
wrong command line.

## What a script has

The console host ([`src/host.rs`](src/host.rs)) answers the names of `script::GLOBALS`, so a
script that uses only the standard library, `print`, `log` and `timer` runs the same here and in
a host of JFNetworker:

| Name | Here |
|---|---|
| `print(..)` | stdout |
| `log(..)`, `log.debug / info / warn / error(..)` | a line on stderr (`warn: …`; `log` and `log.info` without a prefix) |
| `now()` | milliseconds since the start (float), real or virtual clock |
| `rand()`, `rand(n)`, `rand(a, b)` | `[0, 1)`, `0 .. n-1`, `a .. b` |
| `config` | the map of `JFS_CONFIG`, `--config`, `env` (the `--env` variables), `args`, `script` |
| `timer.after(ms, f)`, `timer.every(ms, f)`, `timer.cancel(id)` | an event loop; due timers run in the order of their times, then of their creation |
| `disk.get / put / delete / scan` | as in JFNetworker (JSON values, callbacks `fn(err, ..)` after the current code) |
| `net`, `dns`, `http`, `metrics` | JFNetworker's: a catchable error ("… gibt es nur in JFNetworker") |
| `import(path)` | a module: another `.jfs` file's top-level names as a map, its path relative to the importing file |
| `assert(c, msg)`, `assert_eq(a, b, msg)` | the standard library's checks for tests |

Scripts get no other access to programs or the network, and of files only the `.jfs` files they
import (`PERMISSIONS` allows the runner to read them for a script). Names are checked against the
host's before the run (`script::compile_with`): a typing error (`prnt`) is an error with its line
and column, not a warning. An uncaught error ends the program with the message, the position,
the source line and the calls on stderr:

```text
error.jfs:7:14: Fehler: Division durch 0
      return sum / len(list)
                 ^
  in average (7:14)
  in report (11:15)
  im Hauptprogramm (20:1)
```

## Modules

`import("lib/util.jfs")` reads the file relative to the importing file (the script's own
directory first), runs its top-level code once and gives its `fn`s and `let`s as a map; names
that start with `_` stay inside the module. A later import of the same file gives the same map.
An error in a module names the module's file, and a call in another file names its file:

```text
mod/fail.jfs:2:12: Fehler: Division durch 0
      return a / b
               ^
  in ratio (2:12)
  in run (moderr.jfs:4:12)
  im Hauptprogramm (moderr.jfs:6:7)
```

A cycle of imports is an error that names the chain; a missing file, one that is no `.jfs` file
and one that does not compile are errors the script can catch.

## Tests in jfs

```js
// math_test.jfs
let util = import("../lib/util.jfs")
fn test_twice() {
  assert_eq(util.twice(21), 42)
}
fn test_list() {
  assert_eq([1, 2].map(util.twice), [2, 4], "Liste")
}
```

`jfs --test math_test.jfs more_test.jfs` runs every top-level function `test_*` in the order of
its file, each in a fresh VM: the file's top-level code first (fresh globals and modules), then
the test, then its timers and disk callbacks on the virtual clock (at most 60 s, or
`--until`). `assert(c, msg)` and `assert_eq(a, b, msg)` throw; `assert_eq` compares lists and
maps by their content and names both values. The report:

```text
ok      test_twice (math_test.jfs)
FEHLER  test_list (math_test.jfs:6:3): Liste: erwartet [2, 4], ist [2, 5]
        | (what the test printed and logged)
2 Tests: 1 bestanden, 1 fehlgeschlagen
```

Exit code 1 when a test failed, a file did not load or compile, or there was no test at all.

## The debugger

`jfs --debug app.jfs [ARGS ...]` reads commands from standard input; the prompt is `(jfs) `.

| Command | Meaning |
|---|---|
| `break FILE:LINE [if COND]`, `break LINE` (`b`) | a breakpoint (LINE alone: in the script; FILE from the working directory or the script's directory; COND a jfs expression) |
| `delete N` (`d`), `breaks` | remove breakpoint N, list them |
| `run` (`r`) | start the program (before it, only breakpoints) |
| `continue` (`c`) | on to the next stop |
| `step` (`s`), `next` (`n`), `finish` (`f`) | the next statement into calls, over them, back to the caller |
| `print EXPR` (`p`) | a read-only expression in the innermost call (no calls but `len`, `str`, `type`, `int`, `float`) |
| `bt`, `locals` | the calls; the variables of the innermost call |
| `quit` (`q`) | end the program; so does the end of the input |

Every stop shows `Halt in FILE:LINE (Haltepunkt 1 / Schritt / Pause)` and that line. The
debugger is built on the VM's debug interface ([GUIDE §29](../../docs/GUIDE.md)).

## Tests

`python build_rs.py test jfs` starts `jfs --selftest apps/jfs/tests/cases` through
`tests/cases.uis`. Each `tests/cases/NAME.test` holds one or more runs of `jfs` (started anew)
and what they must give ([`src/selftest.rs`](src/selftest.rs)):

```text
# a comment
args: --disk {tmp}/disk.json {dir}/disk_put.jfs
env: JFS_CONFIG={"port": 7000}
exit: 0
--- stdin
(the standard input)
--- stdout
(the expected standard output)
--- stderr
(the expected standard error)
--- file {tmp}/disk.json
(what the file holds afterwards)
===
args: (the next run)
```

`{dir}` is the cases folder, `{tmp}` a scratch folder of the case below `$TEMP/jfs-*`. The cases
cover hello world, arguments and config, timers on the virtual and the real clock, errors with
line, column and calls, `--check`, `-e`, stdin, the limits, JFNetworker's names, the disk file
across two runs, `log` and `rand` with a seed, modules (`modules.test`: relative paths, once
per run, privacy, errors in modules, cycles, compile errors), `--test` (`unittest.test`:
passing and failing tests, timers, no tests) and `--debug` (`debug.test`: breakpoints in the
script and in a module, the calls, locals, expressions, the step commands, quit, an error).

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the command line, `--check`, the error messages |
| `src/host.rs` | the console host (with the module loader), the event loop, a test's run |
| `src/testrun.rs` | `--test`: the test functions and the report |
| `src/debugger.rs` | `--debug`: the console debugger |
| `src/selftest.rs` | `--selftest`: the example cases |
| `tests/cases.uis`, `tests/cases/` | the test `build_rs.py test` runs, the cases and their scripts (modules in `tests/cases/mod/`) |
| `PERMISSIONS` | the files on the command line, `.jfs` files for import; for `--selftest` a scratch folder and starting itself |
| `PACKAGE` | name, version, app id ([PACKAGING.md](../../docs/PACKAGING.md)) |
| `TARGETS` | Windows and Linux |

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
python build_rs.py test jfs                   # the example cases (Windows and Linux)
```

## Command line

| Option | Meaning |
|---|---|
| `--check` | only the diagnostics, on stdout: `FILE:LINE:COL: Fehler: …` and the warnings; exit code 1 when one is an error |
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

Scripts get no other access to files, programs or the network. Names are checked against the
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
across two runs, `log` and `rand` with a seed.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the command line, `--check`, the error messages |
| `src/host.rs` | the console host and the event loop |
| `src/selftest.rs` | `--selftest`: the example cases |
| `tests/cases.uis`, `tests/cases/` | the test `build_rs.py test` runs, the cases and their scripts |
| `PERMISSIONS` | the files on the command line; for `--selftest` a scratch folder and starting itself |
| `PACKAGE` | name, version, app id ([PACKAGING.md](../../docs/PACKAGING.md)) |
| `TARGETS` | Windows and Linux |

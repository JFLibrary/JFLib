# jfguiLib Build Manager

Native, German-language build workspace for Windows and Linux, using RustLib's WinUI style.
The UI is a `no_std` Rust app; the Python standard-library service reuses `build_rs.py`,
the shared app-test runner and `tools/package.py`.

## Build and start

From `RustLib/`:

```text
python build_rs.py app buildManager --target both --out build/manager-check
build/manager-check/win/buildManager.exe
```

On Linux run `build/manager-check/linux/buildManager`. The app searches the current
directory and executable ancestors for the project. `--workspace PATH` selects another
checkout; its own `tools/build_manager.py` is used. Select `build_rs.py` in
**Projekt & Werkzeuge** if automatic discovery fails. Python, Rust targets and LLVM
tools must be installed. A Python executable path and the WSL distribution are editable.
Linux hosts invoke Linux tools directly; Windows hosts use WSL for Linux execution and
Debian packaging. Windows execution/installer previews require a Windows host.

**Entwicklung / Release:** select apps, targets and optional styles, save a named profile,
then choose **Aufträge vorbereiten** to review the actual argument vectors before enqueuing.
No explicit style means the app's `STYLE`, or the `PACKAGE` style list for releases.
Release uses `size` by default; `release` (LTO) and `o2` are also available.
One build job runs at a time, with the normal builder's `-j` parallelism within it.
Started apps have independent supervised processes and do not block the build queue.

**Starten:** edit the selected app's named launch profiles. Extra arguments are entered
one per line, without shell quoting. Choose a completed app in **Ergebnisse** and start it.
The resolved executable and argument array are shown before launch. Development app settings
are isolated in the manager's build directory. **Projekt & Werkzeuge** reports build,
execution and packaging capabilities separately for each target.

**Paketdaten:** edits `apps/APP/PACKAGE`, preserving comments and unknown fields and
rejecting saves after external changes. Linux packages require `app_48.png`, `app_128.png`
and `app_256.png` in the app's `res` directory. A style subset uses its first selected
style when the package's original default is outside that subset.

## Files and release guarantees

- `build-profiles.json`: version 1, repository-owned named `builds` and per-app `starts`.
- User configuration: `jfguiLib/BuildManager.ini` under `plat::config_dir()`;
  `--config FILE` overrides it for tests. Workspace, Python, WSL, window size and theme stay local.
- Default output: `build/manager/cache/<profile>/<style-key>` and
  `build/manager/artifacts/<job-id>`. Custom bases must stay under the project's `build`.
- Queue history and full logs: `build/manager/jobs`. A lock prevents concurrent managers
  for the same project. Interrupted jobs never restart automatically.
- Each job captures its settings/package metadata; source files follow the normal incremental
  builder. The generated candidates are copied before testing. Installers consume those
  same candidates. `request.json`, `tests.json` and `result.json` record settings, results
  and artifact hashes. A failed stage does not publish a successful result.
- Release tests run every existing non-pending `.uis` on each selected native style/target,
  with `--verify-dirty`. The shared runner honors `#!args`; PaintCraft test plugins are built
  from the existing Rust plugin sources. No tests is reported explicitly. Web bundles have
  no native UI-test certification.

Some existing app tests assert fixed WinUI geometry. Selecting a different style can
therefore produce real test failures, which intentionally block that release. The manager
does not silently skip those tests. Non-GUI tools without package metadata/preview support
remain buildable and need suitable packaging inputs before they can be packaged.

## Protocol and native process API

The UI launches `python -u tools/build_manager.py --stdio`. Each UTF-8 JSONL request has
`version: 1`, `id`, `method`, and `params`. Responses echo `id`/`method` and contain either
`result` or `error`. Requests are bounded to 1 MiB. Operations:

| Method | Parameters |
|---|---|
| `catalog` | optional `wsl_distro`; returns apps, styles, host capabilities, profiles/revision, history |
| `preview`, `enqueue` | `profile`: name, apps, targets, styles, mode, optimization, jobs, tests, output |
| `save_profile` | `kind` (`builds` / `starts`), `profile`, expected `revision` |
| `save_package` | `app`, edited `values`, expected `revision` |
| `job`, `cancel`, `retry` | job `id` |
| `run` | successful job `id`, `artifact` index, `argv` array |

Unsolicited events include `job`, `phase`, `log`, `test`, `result`, `failure`, with a
`job_id` for worker events. Job states are `queued`, `running`, `succeeded`, `failed`,
`cancelled`, `interrupted`. stdin EOF shuts down the service and its owned processes.
The internal `--job` entry point is supervised and waits for its parent's start token.

`plat::process::Command` accepts program, argv, cwd and inherited-environment overrides.
`Child` provides nonblocking output reads, bounded stdin writes, `close_stdin`, `try_wait`
and `terminate`. IO belongs on a worker thread. Windows uses an explicit inherited-handle
list and a kill-on-close job; Linux prepares argv/env before fork, execs without allocating
in the child, and owns a process group. WSL work has an input-pipe lifetime guardian inside
Linux, so killing the Windows supervisor also ends the Linux descendants.

`build_rs.run_app_tests(name, target, exe, verify_dirty=False, report=None)` tests an
already-built executable and returns per-script records (`passed`, `failed`, `skipped`).
The optional `report(record)` callback receives each result as it completes. The CLI's
normal test suite uses the same function. `tools.package.make_setup` and `make_deb` take
their application binaries from `work/<style>/<target>/APP`; only the Windows installer
stub is built separately. `tools.host_exec.run_native` and `run_linux` use argument arrays
and return `subprocess.CompletedProcess` with captured UTF-8 output.

## Verification

```text
python -m unittest tools.tests.test_build_manager -v
python apps/buildManager/tests/native.py --out build/manager-check
python apps/buildManager/tests/integration.py
python apps/buildManager/tests/integration.py --release
python apps/buildManager/tests/packages.py
```

The native matrix checks arguments (including empty/quoted/Unicode), cwd/environment,
large stdout/stderr, exit codes, process-tree cancellation, UI scripts with dirty-render
verification and light/dark screenshots at 100/150% on both targets. `--fixture FILE`
loads a deterministic catalog without starting the backend or saving settings. Fixture
UI actions cannot write project files or launch builds.

The real service acceptance runs create artifacts below `build/manager-integration`.
They also save a reusable `Dunkel · 150 %` start profile for the exercised app.
Installer verification must use `ASMINST_SHELL_DIR` and a destination below `build`;
Debian packages are inspected/extracted with `dpkg-deb`, never installed by these tests.
On Windows, set `jfgui_TEST_WSL=1` for the unit suite to include the real WSL descendant
cancellation check. The native unit suite skips only this Windows-specific integration.

The checked local environment can cross-build all three targets on Windows and execute
the Linux app with WSLg. It has no native Linux Rust/LLVM toolchain; a full native Linux
compiler run requires those tools. Native Linux process/UI tests, helper operations and
direct Debian packaging are exercised independently of the compiler.

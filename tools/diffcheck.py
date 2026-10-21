#!/usr/bin/env python3
"""diffcheck.py -- run a kernel check tool against the Rust rebuild AND compare every result
with the assembly reference, byte for byte.

  python tools/diffcheck.py TOOL [TOOL ARGS...]
      TOOL = blend_check | tmask_check | flood_check   (fxtest batches)
           | gradient_check | pathdata_check | dash_check | resample_check | xform_check
                                                        (rstest op lists)
           | any tool built on tools/imgtest_common.py (codectest batches: codec_check,
             png_check, bmp_check, gif_check, jpeg_check, tga_check, tiff_check,
             imgmeta_check, zlib_state_check, ...)
           | fx_check | fx_check_b | fx_check_c   (efftest runs; r10-fx)

The tool runs as usual with --out build (and --no-build where it has it), so its own
checks (numpy models, tolerances, benchmarks) apply to the Rust programs.  In addition every
fxtest batch / rstest op list it sends is run by the assembly reference build/rwref/<target>
too, and
  * fxtest: result code, every buffer and the extra output of every record are compared
    (the u64 time at the end of timed records is masked);
  * rstest: the result lines (without the us= times) and every file the ops wrote into the
    tool's work directory are compared;
  * codectest: result code and output bytes of every batch record are compared (records
    that report times differ by nature: they are listed, look at them);
  * efftest (r10-fx): every stdout line (the result lines without prepare_us= / render_us= /
    mpxs=, the --list output) and every --out file of a run (also of each line of an --ops
    file) are compared.
The reference runs first, so the tool checks the Rust files afterwards.  Exit code 0 = the
tool passed and nothing differed.
"""
import importlib
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
REF = Path(os.environ.get("jfgui_REF") or ROOT / "build" / "rwref")  # jfgui_REF: a shared reference build
RS = ROOT / "build"
FX_TOOLS = {"blend_check", "tmask_check", "flood_check"}
UNINIT_ON_ERROR = {25, 26, 47}  # codectest ops whose asm surface record is stack garbage on errors
RT_TOOLS = {"gradient_check", "pathdata_check", "dash_check", "resample_check", "xform_check"}


class Diff:
    def __init__(self):
        self.compared = 0
        self.diffs = 0

    def bad(self, what):
        self.diffs += 1
        if self.diffs <= 20:
            print(f"  DIFF {what}")


D = Diff()


def first_diff(a, b):
    n = min(len(a), len(b))
    for i in range(n):
        if a[i] != b[i]:
            return i
    return n


def patch_fx():
    import blend_check as bc

    orig = bc.FxRunner.run

    def run(self, recs, timeout=1800):
        rs_exe = self.exe
        self.exe = REF / self.target / rs_exe.name
        ref = orig(self, recs, timeout)
        self.exe = rs_exe
        res = orig(self, recs, timeout)
        for r, a, b in zip(recs, ref, res):
            D.compared += 1
            timed = len(r.params) > 7 and r.params[7] > 1
            ea, eb = (a.extra[:-8], b.extra[:-8]) if timed else (a.extra, b.extra)
            if a.res != b.res:
                D.bad(f"[{self.target}] fxtest {r.name}: result {b.res} (asm {a.res})")
            for k, (x, y) in enumerate(zip(a.bufs, b.bufs)):
                if x != y:
                    D.bad(f"[{self.target}] fxtest {r.name}: buffer {k} differs at byte {first_diff(x, y)}")
            if ea != eb:
                D.bad(f"[{self.target}] fxtest {r.name}: extra output differs at byte {first_diff(ea, eb)} "
                      f"(len {len(eb)}, asm {len(ea)})")
        return res

    bc.FxRunner.run = run


def patch_rt():
    import resample_check as rc

    orig = rc.Runner.run

    def files(work):
        skip = {"ops.txt", "shot.png"}
        return {p.relative_to(work): p.read_bytes() for p in work.rglob("*") if p.is_file() and p.name not in skip}

    def strip(line):
        return line.split(" us=")[0]

    def run(self, extra=()):
        lines = list(self.lines)
        before = files(self.work)
        rs_out = self.out
        self.out = REF
        ref_lines = orig(self, extra)
        ref_files = files(self.work)
        self.out = rs_out
        self.lines = lines
        res = orig(self, extra)
        got = files(self.work)
        for i, (a, b) in enumerate(zip(ref_lines, res)):
            D.compared += 1
            if strip(a) != strip(b):
                D.bad(f"[{self.target}] rstest line {i + 1} '{lines[i][:60]}': '{strip(b)}' (asm '{strip(a)}')")
        for p, data in ref_files.items():
            if before.get(p) == data and p in before:
                continue  # an input, not written by the ops
            D.compared += 1
            if p not in got:
                D.bad(f"[{self.target}] rstest file {p}: missing (asm wrote it)")
            elif got[p] != data:
                D.bad(f"[{self.target}] rstest file {p}: differs at byte {first_diff(data, got[p])} "
                      f"(len {len(got[p])}, asm {len(data)})")
        return res

    rc.Runner.run = run


def patch_ct():
    """both codectest runners: imgtest_common.Runner and codec_check.Runner (gif_check,
    codec_check itself) -- the same _batch_once(recs) -> (results, process) contract"""
    import codec_check as cc
    import imgtest_common as it

    def wrap(cls):
        orig = cls._batch_once

        def run(self, recs):
            rs_exe = self.exe
            self.exe = REF / self.target / rs_exe.name
            ref, _ = orig(self, recs)
            self.exe = rs_exe
            res, p = orig(self, recs)
            if ref is not None and res is not None:
                for r, a, b in zip(recs, ref, res):
                    D.compared += 1
                    if a[0] == b[0] and a[0] != 0 and r.op in UNINIT_ON_ERROR:
                        continue  # the asm records uninitialised stack bytes on errors here
                    if a[0] != b[0]:
                        D.bad(f"[{self.target}] codectest {r.name} (op {r.op}): result {b[0]} (asm {a[0]})")
                    elif a[1] != b[1]:
                        D.bad(f"[{self.target}] codectest {r.name} (op {r.op}): output differs at byte "
                              f"{first_diff(a[1], b[1])} (len {len(b[1])}, asm {len(a[1])})")
            elif ref is not None and res is None:
                D.bad(f"[{self.target}] codectest batch of {len(recs)} records failed in Rust (exit {p.returncode})")
            return res, p

        cls._batch_once = run

    wrap(it.Runner)
    wrap(cc.Runner)


# r10-fx: the efftest runners -- tools/fx_check.py Harness.run_cmd (also used by fx_check_b /
# fx_check_c) and tools/fx_check_b.py Runner.run
EFF_TOOLS = {"fx_check", "fx_check_b", "fx_check_c"}
EFF_TIMING = ("prepare_us=", "render_us=", "mpxs=")


def eff_strip(line):
    return " ".join(t for t in line.split() if not t.startswith(EFF_TIMING))


def eff_path(p):
    """a path of an efftest command line as Windows sees it (ROOT-relative, absolute, WSL)"""
    if p.startswith("/mnt/") and len(p) > 6:
        return Path(p[5] + ":" + p[6:])
    q = Path(p)
    return q if q.is_absolute() else ROOT / q


def eff_outs(args):
    """the --out files of an efftest command (and of every line of its --ops file)"""
    toks = [list(args)]
    if "--ops" in args:
        i = list(args).index("--ops")
        try:
            toks += [ln.split() for ln in eff_path(args[i + 1]).read_text(encoding="utf-8").splitlines()]
        except (OSError, IndexError):
            pass
    outs = []
    for t in toks:
        if "--out" in t and t.index("--out") + 1 < len(t):
            outs.append(eff_path(t[t.index("--out") + 1]))
    return outs


def eff_compare(target, args, ref_out, rs_out, ref_files):
    what = " ".join(str(a) for a in args)[:70]
    a = [ln for ln in ref_out.splitlines() if ln.strip()]
    b = [ln for ln in rs_out.splitlines() if ln.strip()]
    bad = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if eff_strip(x) != eff_strip(y)]
    D.compared += min(len(a), len(b))
    if bad:  # one message per command: the first differing line
        i, x, y = bad[0]
        D.bad(f"[{target}] efftest '{what}' line {i + 1}: '{eff_strip(y)[:150]}' (asm '{eff_strip(x)[:150]}')"
              + (f" (+{len(bad) - 1} more lines)" if len(bad) > 1 else ""))
        D.diffs += len(bad) - 1
    if len(a) != len(b):
        D.bad(f"[{target}] efftest '{what}': {len(b)} output lines (asm {len(a)})")
    for p, data in ref_files.items():
        D.compared += 1
        if not p.exists():
            D.bad(f"[{target}] efftest file {p.name}: missing (asm wrote it)")
        elif p.read_bytes() != data:
            got = p.read_bytes()
            D.bad(f"[{target}] efftest file {p.name}: differs at byte {first_diff(data, got)} "
                  f"(len {len(got)}, asm {len(data)})")


def eff_ref(target, args, timeout):
    """run the reference efftest; -> (stdout, {out file: bytes}) (the files are removed)"""
    import subprocess
    import build
    outs = eff_outs(args)
    exe = REF / target / ("efftest.exe" if target == "win" else "efftest")
    r = subprocess.run(build.run_cmd_for(exe, target, [str(a) for a in args]), cwd=ROOT, capture_output=True,
                       text=True, timeout=timeout)
    files = {}
    for p in outs:
        if p.exists():
            files[p] = p.read_bytes()
            p.unlink()
    return r.stdout, files


def patch_eff():
    sys.path.insert(0, str(ROOT.parent / "legacy"))  # R13: build.py is in legacy/
    sys.path.insert(0, str(ROOT))
    import fx_check as fc
    import fx_check_b as fb

    orig_h = fc.Harness.run_cmd

    def run_cmd(self, args, timeout=3600):
        pre = ["--shot", "-"] + (["--threads", str(self.pool_threads)] if self.pool_threads else [])
        ref_out, ref_files = eff_ref(self.target, pre + list(args), timeout)
        res = orig_h(self, args, timeout)
        eff_compare(self.target, args, ref_out, res.stdout, ref_files)
        return res

    fc.Harness.run_cmd = run_cmd
    orig_r = fb.Runner.run

    def run(self, args, timeout=900):
        ref_out, ref_files = eff_ref(self.target, ["--shot", "-"] + [str(a) for a in args], timeout)
        rc, out = orig_r(self, args, timeout)
        eff_compare(self.target, args, ref_out, out, ref_files)
        return rc, out

    fb.Runner.run = run


def is_ct_tool(tool):
    src = ROOT / "tools" / (tool + ".py")
    if tool == "codec_check":
        return True
    txt = src.read_text(encoding="utf-8", errors="replace") if src.exists() else ""
    return "imgtest_common" in txt or "from codec_check import" in txt


def main():
    if len(sys.argv) < 2 or not (sys.argv[1] in FX_TOOLS | RT_TOOLS | EFF_TOOLS or is_ct_tool(sys.argv[1])):
        print(__doc__)
        return 2
    tool = sys.argv[1]
    args = sys.argv[2:]
    if tool in EFF_TOOLS:  # r10-fx
        patch_eff()
        args = ["--no-build"] + args
    elif tool in FX_TOOLS:
        patch_fx()
        args = ["--no-build"] + args
    elif tool in RT_TOOLS:
        patch_rt()
    else:
        patch_ct()
        args = ["--no-build"] + args
    mod = importlib.import_module(tool)
    sys.argv = [tool + ".py", "--out", str(RS.relative_to(ROOT))] + args
    try:
        rc = (getattr(mod, "main", None) or mod.check_main)() or 0  # r9-pdn: pdn_check has check_main
    except SystemExit as e:  # some tools end main() with sys.exit
        rc = e.code or 0
    print(f"diffcheck {tool}: {D.compared} results compared with the assembly, "
          + ("all identical" if D.diffs == 0 else f"{D.diffs} DIFFERENT"))
    return 1 if (rc or D.diffs) else 0


if __name__ == "__main__":
    sys.exit(main())

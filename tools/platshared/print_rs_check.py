#!/usr/bin/env python3
"""print_rs_check.py -- plat::print of the Rust rebuild against the assembly (agent plat-shared, R4).

  python tools/platshared/print_rs_check.py [--target win|linux|both] [--ref build/rwref] [--rs build]

Never a dialog, never paper (the test variants of docs/modules/PRINT.md):
  layout  plattest --test printlayout (a grid of jobs / pages, f64 bit patterns) replayed with
          print.s' operations in Python: every value bit for bit (both targets)
  linux   jfgui_PRINT_TO=FILE.pdf (and jfgui_PRINT_SMASK=1) for tools/print_check.py's three
          cases: plattest --test printjob against the assembly's PaintCraft with print_check's
          script -- the PDFs byte for byte, and checked like print_check.py
  win     "Microsoft Print to PDF" into a file (jfgui_PRINT_TO + jfgui_PRINT_PRINTER) for the
          same cases, Rust and the assembly's PaintCraft: one page, the MediaBox of paper and
          orientation, the spooler's files byte for byte apart from their dates (skipped when
          the printer is missing)
  self    plattest --test print on both targets (headless guard, row_white, the writer)
The portal path: tools/platshared/portal_bus.py (inside WSL).  Exit code 0 = passed.
"""
import argparse, os, re, struct, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
import print_check as pc  # noqa: E402

WSL = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e"]
PRINTER = "Microsoft Print to PDF"   # the only printer these tests ever use


def bits(h):
    return struct.unpack("<d", struct.pack("<Q", int(h, 16)))[0]


def replay(paper, land, scale, pct, center, margin, dx, dy, pw, ph, L, T, R, B, w=37, h=23):
    """print.s print_layout, operation by operation"""
    dx = dx if dx > 0 else 96.0
    dy = dy if dy > 0 else dx
    iw = float(w) * 72.0 / dx
    ih = float(h) * 72.0 / dy
    m = float(margin * 100) * 0.028346456692913385
    mx = lambda a, b: a if a > b else b
    mn = lambda a, b: a if a < b else b
    ax = mx(L, m)
    aw = mx(pw - mx(R, m) - ax, 1.0)
    ay = mx(T, m)
    ah = mx(ph - mx(B, m) - ay, 1.0)
    if scale != 1:
        s = float(min(max(pct, 1), 10000)) / 100.0 if scale == 2 else mn(aw / iw, ah / ih)
        iw, ih = iw * s, ih * s
    x, y = ax, ay
    if center:
        x = ax + (aw - iw) * 0.5
        y = ay + (ah - ih) * 0.5
    return [x, y, iw, ih, ax, ay, aw, ah]


def run_rs(target, rs, args, env):
    if target == "win":
        e = dict(os.environ)
        for k in ("jfgui_PRINT_TO", "jfgui_PRINT_PRINTER", "jfgui_PRINT_SMASK", "jfgui_PRINT_PORTAL"):
            e.pop(k, None)
        e.update(env)
        cmd = [str(ROOT / rs / "win" / "plattest.exe")] + args
    else:
        e = None
        cmd = WSL + ["env", "-u", "jfgui_PRINT_PORTAL"] + [f"{k}={v}" for k, v in env.items()] + [f"./{rs}/linux/plattest"] + args
    r = subprocess.run(cmd, cwd=ROOT, env=e, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300)
    return r.returncode, r.stdout + r.stderr


def suite_layout(target, rs):
    rc, out = run_rs(target, rs, ["--test", "printlayout"], {})
    lines = [l for l in out.splitlines() if "->" in l]
    bad = 0
    for l in lines:
        a, b = l.split(" -> ")
        f = a.split()
        paper, land, scale, pct, center, margin = (int(v) for v in f[:6])
        dx, dy, pw, ph, L, T, R, B = (bits(v) for v in f[6:14])
        got = [bits(v) for v in b.split()]
        want = replay(paper, land, scale, pct, center, margin, dx, dy, pw, ph, L, T, R, B)
        if [struct.pack("<d", v) for v in got] != [struct.pack("<d", v) for v in want]:
            bad += 1
            if bad <= 3:
                print("   ", l, want)
    pc.check(f"[{target}] layout: {len(lines)} jobs / pages replayed bit for bit", rc == 0 and lines and bad == 0,
             f"{bad} differ")


def asm_case(target, ref, ci, env, wrel):
    name, steps, opt, dpi = pc.CASES[ci]
    png = f"{wrel}/druck_test.png" if dpi is None else f"{wrel}/dpi{int(dpi)}/druck_test.png"
    Path(ROOT / png).parent.mkdir(parents=True, exist_ok=True)
    pc.write_png(ROOT / png, pc.make_pixels(), dpi=dpi)
    paper, land, scale, pct, center, margin = opt
    lay = pc.layout(paper, land, scale, pct, center, margin, dpi=dpi or 96.0)
    scr = f"{wrel}/case{ci}.uis"
    pc.script(ROOT / scr, steps, pc.cm_text(lay))
    rc, log = pc.run_app(target, ref, scr, env, png)
    return rc, log, lay


def suite_linux(ref, rs, work, wrel):
    px = pc.make_pixels()
    for ci in range(len(pc.CASES)):
        for smask in (False, True):
            tag = f"linux case {ci}{' SMask' if smask else ''}"
            rpdf, apdf = f"{wrel}/rs_case{ci}.pdf", f"{wrel}/asm_case{ci}.pdf"
            for p in (rpdf, apdf):
                if (ROOT / p).exists():
                    (ROOT / p).unlink()
            env = {"jfgui_PRINT_TO": rpdf}
            if smask:
                env["jfgui_PRINT_SMASK"] = "1"
            rc, out = run_rs("linux", rs, ["--test", "printjob", "--case", str(ci)], env)
            env["jfgui_PRINT_TO"] = apdf
            arc, alog, lay = asm_case("linux", ref, ci, env, wrel)
            ok = pc.check(f"{tag}: Rust and the assembly wrote their PDFs", rc == 0 and arc == 0 and (ROOT / rpdf).exists()
                          and (ROOT / apdf).exists(), (out[-300:], alog[-300:]))
            if not ok:
                continue
            rb, ab = (ROOT / rpdf).read_bytes(), (ROOT / apdf).read_bytes()
            pc.check(f"{tag}: the PDF byte for byte ({len(rb)} bytes)", rb == ab, (len(rb), len(ab)))
            pc.check_ours(tag, rb, px, lay, smask)


def wait_file(p, timeout=60):
    return pc.wait_file(p, timeout)


def suite_win(ref, rs, work, wrel):
    if not pc.printer_exists():
        print(f"SKIP win: printer '{PRINTER}' not installed")
        return
    for ci in range(len(pc.CASES)):
        tag = f"win case {ci}"
        rpdf, apdf = work / f"win_rs_case{ci}.pdf", work / f"win_asm_case{ci}.pdf"
        for p in (rpdf, apdf):
            if p.exists():
                p.unlink()
        rc, out = run_rs("win", rs, ["--test", "printjob", "--case", str(ci)],
                         {"jfgui_PRINT_TO": str(rpdf), "jfgui_PRINT_PRINTER": PRINTER})
        arc, alog, lay = asm_case("win", ref, ci, {"jfgui_PRINT_TO": str(apdf), "jfgui_PRINT_PRINTER": PRINTER}, wrel)
        pc.check(f"{tag}: Rust printed to the PDF printer ({out.strip().splitlines()[0] if out.strip() else ''})", rc == 0, out[-300:])
        rd, ad = wait_file(rpdf), wait_file(apdf)
        if not pc.check(f"{tag}: the spooler wrote both PDFs", rd.startswith(b"%PDF") and ad.startswith(b"%PDF"), (len(rd), len(ad))):
            continue
        facts = []
        for d in (rd, ad):
            npg = len(re.findall(rb"/Type\s*/Page(?![a-zA-Z])", d))
            mb = re.search(rb"/MediaBox\s*\[\s*([^\]]*)\]", d)
            facts.append((npg, pc.nums(mb.group(1).decode()) if mb else []))
        (n, mb), (an, amb) = facts
        ok = n == 1 and mb and abs(mb[2] - mb[0] - lay["pw"]) < 2 and abs(mb[3] - mb[1] - lay["ph"]) < 2
        pc.check(f"{tag}: 1 page, MediaBox ~ {lay['pw']:.0f} x {lay['ph']:.0f}", ok, facts)
        pc.check(f"{tag}: pages and MediaBox as the assembly's", (n, mb) == (an, amb), facts)
        mask = lambda d: re.sub(rb"/(CreationDate|ModDate) \(D:[^)]*\)", rb"/\1 ()", d)
        pc.check(f"{tag}: the spooler's PDF byte for byte as the assembly's (dates masked)", mask(rd) == mask(ad),
                 (len(rd), len(ad)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--ref", default="build/rwref")
    ap.add_argument("--rs", default="build")
    a = ap.parse_args()
    ref, rs = a.ref.replace("\\", "/").rstrip("/"), a.rs.replace("\\", "/").rstrip("/")
    wrel = f"build/plat-shared/print"
    work = ROOT / wrel
    work.mkdir(parents=True, exist_ok=True)
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    for t in targets:
        rc, out = run_rs(t, rs, ["--test", "print"], {})
        pc.check(f"[{t}] plattest --test print", rc == 0, out[-600:])
        suite_layout(t, rs)
    if "linux" in targets:
        suite_linux(ref, rs, work, wrel)
    if "win" in targets:
        suite_win(ref, rs, work, wrel)
    print("print_rs_check:", "all passed" if pc.fails == 0 else f"{pc.fails} FAILED")
    return 1 if pc.fails else 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Printing (PR1): the PDFs of plat_print, checked with an own parser.

  python tools/print_check.py [--target win|linux|both] [--out build/pr1]

PaintCraft (built into --out for the targets) opens a generated RGBA PNG, a generated script
sets options in the print dialog and presses "Drucken…"; the test variants never open a
dialog and never reach a printer:
  linux  jfgui_PRINT_TO=FILE.pdf: pdfwrite.s output -- header, xref offsets, trailer,
         MediaBox, the image placement (print_layout replayed here), the decompressed pixels
         = composite over white (jfgui_PRINT_SMASK=1: straight colours + /SMask alpha);
         the xdg-desktop-portal path against a fake session bus (tools/print_portal_bus.py,
         run inside WSL); headless without a test variable -> error, no bus contact.
  win    "Microsoft Print to PDF" into a file (jfgui_PRINT_TO; skipped when the printer is
         missing): one page, MediaBox of the paper / orientation chosen in the dialog;
         headless without jfgui_PRINT_TO -> error (no dialog).
pdfinfo / qpdf --check run on the pdfwrite.s files when WSL has them (nothing is installed).
Exit code 0 = passed.
"""
import argparse, os, re, struct, subprocess, sys, time, zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WSL = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e"]
WIN_PRINTER = "Microsoft Print to PDF"
PAPER_PT = [(595.2755905511812, 841.8897637795276), (612.0, 792.0),
            (419.5275590551181, 595.2755905511812), (612.0, 1008.0),
            (841.8897637795276, 1190.551181102362)]
PAPER_LABEL = ["A4 (210 × 297 mm)", "Letter (8,5 × 11 in)", "A5 (148 × 210 mm)",
               "Legal (8,5 × 14 in)", "A3 (297 × 420 mm)"]
IMG_W, IMG_H = 37, 23
MSG_UNSUPPORTED = "Es ist kein Drucker bzw. Druckdienst verfügbar."
MSG_FAIL = "Beim Drucken ist ein Fehler aufgetreten."

fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


# ------------------------------------------------------------------ fixture
def make_pixels(w=IMG_W, h=IMG_H):
    px = []
    for y in range(h):
        for x in range(w):
            r = (x * 255) // (w - 1)
            g = (y * 255) // (h - 1)
            b = (x * 7 + y * 13) & 255
            a = [255, 0, 128, 200, 37][(x + 2 * y) % 5]
            px.append((r, g, b, a))
    return px


def write_png(path, px, w=IMG_W, h=IMG_H, dpi=None):
    raw = b"".join(b"\0" + b"".join(bytes(px[y * w + x]) for x in range(w)) for y in range(h))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
    if dpi:
        ppm = round(dpi / 0.0254)
        data += chunk(b"pHYs", struct.pack(">IIB", ppm, ppm, 1))
    data += chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    Path(path).write_bytes(data)


def over_white(px):
    out = bytearray()
    for r, g, b, a in px:
        for c in (r, g, b):
            out.append((c * a + 255 * (255 - a) + 127) // 255)
    return bytes(out)


# ------------------------------------------------------------------ layout (print.s replayed)
def layout(paper, landscape, scale, pct, center, margin_mm, dpi=96.0, w=IMG_W, h=IMG_H,
           page=None, insets=(0.0, 0.0, 0.0, 0.0)):
    if page is None:
        pw, ph = PAPER_PT[paper]
        if landscape:
            pw, ph = ph, pw
    else:
        pw, ph = page
    iw = float(w) * 72.0 / dpi
    ih = float(h) * 72.0 / dpi
    m = float(margin_mm * 100) * 0.028346456692913385
    L, T, R, B = insets
    ax = max(L, m)
    aw = max(pw - max(R, m) - ax, 1.0)
    ay = max(T, m)
    ah = max(ph - max(B, m) - ay, 1.0)
    if scale == 0:
        s = min(aw / iw, ah / ih)
        iw, ih = iw * s, ih * s
    elif scale == 2:
        s = float(min(max(pct, 1), 10000)) / 100.0
        iw, ih = iw * s, ih * s
    x, y = ax, ay
    if center:
        x = ax + (aw - iw) * 0.5
        y = ay + (ah - ih) * 0.5
    return dict(pw=pw, ph=ph, x=x, y=y, w=iw, h=ih, ax=ax, ay=ay, aw=aw, ah=ah)


def cm_text(lay):
    wc = round(lay["w"] * 3.5277777777777777)
    hc = round(lay["h"] * 3.5277777777777777)
    return f"Druckgröße: {wc // 100},{wc % 100:02d} × {hc // 100},{hc % 100:02d} cm"


# ------------------------------------------------------------------ PDF parser
def parse_pdf(data):
    """-> (objects {n: (dict_text, stream or None)}, trailer text, errors)"""
    errs = []
    if not data.startswith(b"%PDF-1."):
        return {}, "", ["no %PDF header"]
    m = re.search(rb"startxref\s+(\d+)\s+%%EOF\s*$", data)
    if not m:
        return {}, "", ["no startxref / %%EOF at the end"]
    xo = int(m.group(1))
    if data[xo:xo + 4] != b"xref":
        return {}, "", [f"startxref {xo} does not point at 'xref'"]
    p = xo + 4
    while data[p:p + 1] in b"\r\n ":
        p += 1
    hm = re.match(rb"(\d+) (\d+)\r?\n", data[p:])
    first, count = int(hm.group(1)), int(hm.group(2))
    p += hm.end()
    offs = {}
    for i in range(count):
        ent = data[p:p + 20]
        if len(ent) != 20 or not re.match(rb"\d{10} \d{5} [nf](\r\n| \n| \r)", ent):
            errs.append(f"xref entry {first + i} is not 20 bytes: {ent!r}")
        if ent[17:18] == b"n":
            offs[first + i] = int(ent[:10])
        p += 20
    tm = re.search(rb"trailer\s*<<(.*?)>>\s*startxref", data[p:], re.S)
    trailer = tm.group(1).decode("latin-1") if tm else ""
    size = re.search(r"/Size (\d+)", trailer)
    if not size or int(size.group(1)) != first + count:
        errs.append(f"trailer /Size {size and size.group(1)} != {first + count}")
    objs = {}
    for n, o in offs.items():
        head = f"{n} 0 obj".encode()
        if data[o:o + len(head)] != head:
            errs.append(f"xref offset of object {n} ({o}) points at {data[o:o + 12]!r}")
            continue
        end = data.find(b"endobj", o)
        body = data[o + len(head):end]
        si = body.find(b"stream")
        if si < 0:
            objs[n] = (body.decode("latin-1").strip(), None)
            continue
        d = body[:si].decode("latin-1").strip()
        ln = int(re.search(r"/Length (\d+)", d).group(1))
        s0 = o + len(head) + si + len(b"stream")
        s0 += 2 if data[s0:s0 + 2] == b"\r\n" else 1
        stream = data[s0:s0 + ln]
        if not data[s0 + ln:s0 + ln + 20].lstrip(b"\r\n").startswith(b"endstream"):
            errs.append(f"object {n}: /Length {ln} does not end at endstream")
        objs[n] = (d, stream)
    return objs, trailer, errs


def nums(s):
    return [float(v) for v in s.split()]


def check_ours(name, data, px, lay, smask=False):
    objs, trailer, errs = parse_pdf(data)
    check(f"{name}: structure / xref / trailer", not errs, errs)
    if errs:
        return
    root = re.search(r"/Root (\d+) 0 R", trailer)
    cat = objs.get(int(root.group(1)), ("", None))[0] if root else ""
    pages = re.search(r"/Pages (\d+) 0 R", cat)
    pg = objs.get(int(pages.group(1)), ("", None))[0] if pages else ""
    kids = re.search(r"/Kids \[(\d+) 0 R\]", pg)
    check(f"{name}: catalog -> one page", "/Count 1" in pg and kids is not None, (cat, pg))
    page = objs[int(kids.group(1))][0]
    mb = nums(re.search(r"/MediaBox \[([^\]]*)\]", page).group(1))
    check(f"{name}: MediaBox {lay['pw']:.3f} x {lay['ph']:.3f}",
          abs(mb[2] - lay["pw"]) < 0.002 and abs(mb[3] - lay["ph"]) < 0.002 and mb[:2] == [0, 0], mb)
    cref = int(re.search(r"/Contents (\d+) 0 R", page).group(1))
    content = objs[cref][1].decode("latin-1")
    cm = re.search(r"([-\d.]+) ([-\d.]+) ([-\d.]+) ([-\d.]+) ([-\d.]+) ([-\d.]+) cm", content)
    got = [float(v) for v in cm.groups()]
    want = [lay["w"], 0, 0, lay["h"], lay["x"], lay["ph"] - lay["y"] - lay["h"]]
    check(f"{name}: image placement (cm)", all(abs(a - b) < 0.002 for a, b in zip(got, want)),
          (got, want))
    re_ = re.search(r"([-\d.]+) ([-\d.]+) ([-\d.]+) ([-\d.]+) re W n", content)
    clip = [float(v) for v in re_.groups()] if re_ else []
    wclip = [lay["ax"], lay["ph"] - lay["ay"] - lay["ah"], lay["aw"], lay["ah"]]
    check(f"{name}: clip = printable area", clip and all(abs(a - b) < 0.002 for a, b in zip(clip, wclip)),
          (clip, wclip))
    iref = int(re.search(r"/Im1 (\d+) 0 R", page).group(1))
    idict, istream = objs[iref]
    ok = ("/Subtype /Image" in idict and f"/Width {IMG_W}" in idict and f"/Height {IMG_H}" in idict
          and "/ColorSpace /DeviceRGB" in idict and "/BitsPerComponent 8" in idict
          and "/Filter /FlateDecode" in idict)
    check(f"{name}: image XObject {IMG_W} x {IMG_H} RGB Flate", ok, idict)
    rgb = zlib.decompress(istream)
    if smask:
        want_rgb = bytes(c for p in px for c in p[:3])
        check(f"{name}: pixels = straight colours", rgb == want_rgb, len(rgb))
        sm = re.search(r"/SMask (\d+) 0 R", idict)
        sd, ss = objs[int(sm.group(1))] if sm else ("", b"")
        alpha = zlib.decompress(ss) if ss else b""
        check(f"{name}: /SMask DeviceGray = alpha", "/DeviceGray" in sd and alpha == bytes(p[3] for p in px),
              sd)
    else:
        want_rgb = over_white(px)
        bad = [i for i in range(0, len(want_rgb), 3) if rgb[i:i + 3] != want_rgb[i:i + 3]]
        check(f"{name}: pixels = composite over white", len(rgb) == len(want_rgb) and not bad,
              f"{len(bad)} pixels differ, first {bad[:3]}")
        check(f"{name}: no /SMask", "/SMask" not in idict, idict)
    info = re.search(r"/Info (\d+) 0 R", trailer)
    it = objs[int(info.group(1))][0] if info else ""
    title = re.search(r"/Title <FEFF([0-9A-F]*)>", it)
    tstr = bytes.fromhex(title.group(1)).decode("utf-16-be") if title else None
    check(f"{name}: /Title (UTF-16BE) = document name", tstr == "druck_test.png", it)


# ------------------------------------------------------------------ running PaintCraft
def run_app(target, out, script, env_extra, png):
    args = ["--config", f"{out}/cfg", "--open", png, "--script", script]
    if target == "win":
        env = dict(os.environ)
        for k in ("jfgui_PRINT_TO", "jfgui_PRINT_PRINTER", "jfgui_PRINT_SMASK"):
            env.pop(k, None)
        env.update(env_extra)
        cmd = [str(ROOT / out / "win" / "PaintCraft.exe")] + args
    else:
        env = None
        cmd = WSL + ["env", "-u", "jfgui_PRINT_PORTAL"] + [f"{k}={v}" for k, v in env_extra.items()] + \
            [f"./{out}/linux/PaintCraft"] + args
    r = subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True, encoding="utf-8",
                       errors="replace", timeout=300)
    return r.returncode, r.stdout + r.stderr


def script(path, steps, info, error=None):
    lines = ["size 1280 800", "theme light", "key ctrl+p", "expect window 24640 open"] + steps
    if info:
        lines.append(f'expect text:"{info}" exists')
    lines += ['click button:"Drucken…"', "expect window 24640 closed"]
    if error:
        lines.append(f'expect text:"{error}" exists')
    else:
        lines += [f'expect text:"{MSG_FAIL}" missing', f'expect text:"{MSG_UNSUPPORTED}" missing']
    Path(path).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")


CASES = [
    # name, steps, (paper, landscape, scale, pct, center, margin), png dpi
    ("A4 hoch, an Seite anpassen, zentriert", [], (0, 0, 0, 100, 1, 5), None),
    ("Letter quer, 50 %, oben links, Rand 0",
     ["click #6201", 'click listitem:"Letter (8,5 × 11 in)"', "click #6203", "click #6206",
      "click #6207", "key ctrl+a", 'type "50"', "key enter", "click #6208",
      "click #6209", "key ctrl+a", 'type "0"', "key enter"], (1, 1, 2, 50, 0, 0), None),
    ("A5 Originalgröße 300 dpi, Rand 12", ["click #6201", 'click listitem:"A5 (148 × 210 mm)"', "click #6205",
                                          "click #6209", "key ctrl+a", 'type "12"', "key enter"],
     (2, 0, 1, 100, 1, 12), 300.0),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/pr1")
    a = ap.parse_args()
    out = a.out.replace("\\", "/").rstrip("/")
    work = ROOT / out / "print_check"
    work.mkdir(parents=True, exist_ok=True)
    wrel = f"{out}/print_check"
    px = make_pixels()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    for target in targets:
        print(f"== {target}")
        for ci, (name, steps, opt, dpi) in enumerate(CASES):
            png = f"{wrel}/druck_test.png" if dpi is None else f"{wrel}/dpi{int(dpi)}/druck_test.png"
            Path(ROOT / png).parent.mkdir(parents=True, exist_ok=True)
            write_png(ROOT / png, px, dpi=dpi)
            paper, land, scale, pct, center, margin = opt
            lay = layout(paper, land, scale, pct, center, margin, dpi=dpi or 96.0)
            scr = f"{wrel}/case{ci}.uis"
            script(ROOT / scr, steps, cm_text(lay))
            pdf = f"{wrel}/{target}_case{ci}.pdf"
            if (ROOT / pdf).exists():
                (ROOT / pdf).unlink()
            if target == "linux":
                for smask in (False, True):
                    env = {"jfgui_PRINT_TO": pdf}
                    if smask:
                        env["jfgui_PRINT_SMASK"] = "1"
                    rc, log = run_app(target, out, scr, env, png)
                    tag = f"linux case {ci} ({name}{', SMask' if smask else ''})"
                    if not check(f"{tag}: script", rc == 0 and (ROOT / pdf).exists(), log[-600:]):
                        continue
                    check_ours(tag, (ROOT / pdf).read_bytes(), px, lay, smask)
                    tool_check(pdf)
                    (ROOT / pdf).unlink()
            else:
                if not printer_exists():
                    print(f"SKIP win case {ci}: printer '{WIN_PRINTER}' not installed")
                    continue
                rc, log = run_app(target, out, scr, {"jfgui_PRINT_TO": str(ROOT / pdf),
                                                     "jfgui_PRINT_PRINTER": WIN_PRINTER}, png)
                tag = f"win case {ci} ({name})"
                check(f"{tag}: script", rc == 0, log[-600:])
                data = wait_file(ROOT / pdf)
                if check(f"{tag}: PDF written by the spooler", data.startswith(b"%PDF"), len(data)):
                    np = len(re.findall(rb"/Type\s*/Page(?![a-zA-Z])", data))
                    mb = re.search(rb"/MediaBox\s*\[\s*([^\]]*)\]", data)
                    mbv = nums(mb.group(1).decode()) if mb else []
                    ok = mbv and abs(mbv[2] - mbv[0] - lay["pw"]) < 2 and abs(mbv[3] - mbv[1] - lay["ph"]) < 2
                    check(f"{tag}: 1 page, MediaBox ~ {lay['pw']:.0f} x {lay['ph']:.0f}", np == 1 and ok,
                          (np, mbv))
        # headless without a test variable: an error, never a dialog / bus
        scr = f"{wrel}/guard.uis"
        script(ROOT / scr, [], None, MSG_UNSUPPORTED)
        env = {} if target == "win" else {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent/pr1"}
        rc, log = run_app(target, out, scr, env, f"{wrel}/druck_test.png")
        check(f"{target}: headless without jfgui_PRINT_TO -> '{MSG_UNSUPPORTED}'", rc == 0, log[-600:])
        if target == "linux":
            r = subprocess.run(WSL + ["python3", "tools/print_portal_bus.py", f"./{out}/linux/PaintCraft", out],
                               cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
                               errors="replace", timeout=900)
            print(r.stdout.rstrip())
            check("linux: portal fake bus (print_portal_bus.py)", r.returncode == 0, r.stderr[-800:])
    print("print_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


_TOOLS = None


def tool_check(pdf):
    """pdfinfo / qpdf --check inside WSL when installed (nothing is installed)"""
    global _TOOLS
    if _TOOLS is None:
        r = subprocess.run(WSL + ["sh", "-c", "command -v pdfinfo; command -v qpdf"], cwd=ROOT,
                           capture_output=True, text=True)
        _TOOLS = r.stdout.split()
        if not _TOOLS:
            print("SKIP pdfinfo / qpdf: not installed in WSL")
    for t in _TOOLS:
        args = [t, pdf] if t.endswith("pdfinfo") else [t, "--check", pdf]
        r = subprocess.run(WSL + args, cwd=ROOT, capture_output=True, text=True)
        check(f"{os.path.basename(t)} {pdf}", r.returncode == 0, r.stdout[-300:] + r.stderr[-300:])


_PRN = None


def printer_exists():
    global _PRN
    if _PRN is None:
        r = subprocess.run(["powershell", "-NoProfile", "-Command",
                            f"if (Get-Printer -Name '{WIN_PRINTER}' -ErrorAction SilentlyContinue) {{ 'yes' }}"],
                           capture_output=True, text=True)
        _PRN = "yes" in r.stdout
    return _PRN


def wait_file(p, timeout=60):
    """the spooler writes the file after EndDoc: wait until it exists and stops growing"""
    t0, last = time.time(), -1
    while time.time() - t0 < timeout:
        if p.exists():
            n = p.stat().st_size
            if n > 0 and n == last:
                return p.read_bytes()
            last = n
        time.sleep(0.5)
    return p.read_bytes() if p.exists() else b""


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""pn_crash_check.py -- PaintCraft crash reports (apps/PaintCraft/src/crash.s, the library's
plat_crash_hook; apps/PaintCraft/docs/DIAGNOSTICS.md).

For every target PaintCraft runs a script whose `cmd 911` writes to address 0x10 (crash_test),
with `--crash-dir build/<out>/pn_crash_check/<target>` (headless: the protocol is written, no
report window is started).  Checked:

  * the process ends with exit code 3 and the library's stderr line
  * exactly one protocol "Absturz_JJJJ-MM-TT_hh-mm-ss.txt" with the exception (Zugriffsverletzung
    beim Schreiben an Adresse 0x10), RAX = 0x10, the registers, the diagnostics part
  * the call chain resolved with the build's .map file starts crash_test <- a1_diag_cmd <- pn_cmd
    and reaches the entry point (plat_entry / _start)
  * `--crash-report FILE` opens the report window with that protocol (a script checks the probe)

The Rust build (rs/, `--out build`; found by its map: v0-mangled names from lld-link,
demangled Rust paths from ld.lld, or `--mode rust`) has its own call chain: names are
demangled (Rust v0, as LLVM prints them), return addresses are looked up at the call
instruction before them, and the chain must start plat::crash_test <- PaintCraft::diag::
a1_diag_cmd <- PaintCraft::diag::diag_cmd, contain jfgui_main and end in the platform entry
(mainCRTStartup / _start).  The Rust crash handler unwinds exactly (PE unwind data / .eh_frame
CFI), so in Rust mode every return address must also follow a call instruction whose direct
target is the start of the function below it (no stale stack values).

  python tools/pn_crash_check.py [--target win|linux|both] [--out build/sopt] [--mode auto|asm|rust]
  python tools/pn_crash_check.py --resolve PROTOCOL.txt [--out build]
                                 (prints the chain of a protocol with the functions of the
                                 build's .map, demangled; the target from the protocol)

The real report window (a second process started by plat_spawn_self) is not opened here
(no windows on the desktop); plattest --test spawn covers plat_spawn_self.
"""
import argparse
import bisect
import os
import pathlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
WSL = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e"]


def rel(p):
    return str(pathlib.Path(p).resolve().relative_to(ROOT)).replace("\\", "/")


def load_map(path, target, rust=False):
    """(offset, name) sorted; Rust: names demangled, ld.lld's names may contain blanks"""
    syms = []
    for line in open(path, encoding="utf-8", errors="replace"):
        p = line.split()
        if target == "win":
            if len(p) >= 4 and re.match(r"^[0-9a-f]{16}$", p[2]) and not p[1].startswith("."):
                syms.append((int(p[2], 16) - 0x140000000, demangle(p[1]) if rust else p[1]))
        elif rust:
            if (len(p) >= 5 and re.match(r"^[0-9a-f]+$", p[0]) and re.match(r"^[0-9a-f]+$", p[1])
                    and p[3] == "1" and not p[4].startswith(".")):
                syms.append((int(p[0], 16), demangle(" ".join(p[4:]))))
        elif (len(p) >= 5 and re.match(r"^[0-9a-f]+$", p[0]) and re.match(r"^[0-9a-f]+$", p[1])
              and p[3] == "1" and not p[-1].startswith(".")):
            syms.append((int(p[0], 16), p[-1]))
    syms.sort()
    return syms


def map_is_rust(path, target):
    """the .map of the Rust build: v0-mangled names (lld-link) or demangled Rust paths (ld.lld)"""
    for line in open(path, encoding="utf-8", errors="replace"):
        p = line.split()
        if target == "win":
            if len(p) >= 4 and re.match(r"^[0-9a-f]{16}$", p[2]) and p[1].startswith("_R"):
                return True
        elif (len(p) >= 5 and re.match(r"^[0-9a-f]+$", p[0]) and re.match(r"^[0-9a-f]+$", p[1])
              and p[3] == "1" and "::" in " ".join(p[4:])):
            return True
    return False


def resolve(syms, offs, rust):
    """the function (start, name) of every offset of the chain; Rust: a return address (every
    offset but the first, the crash address) is looked up at the call instruction before it"""
    out = []
    for k, o in enumerate(offs):
        q = o - 1 if rust and k > 0 else o
        i = bisect.bisect_right(syms, (q, "~")) - 1
        out.append(syms[i] if i >= 0 else (None, "?"))
    return out


class Image:
    """the bytes of an executable at offsets as the protocol shows them (PE: image offsets,
    ELF: addresses)"""

    def __init__(self, path, target):
        self.data = pathlib.Path(path).read_bytes()
        self.ranges = []  # (offset, size, file offset)
        d = self.data
        if target == "win":
            nt = int.from_bytes(d[0x3C:0x40], "little")
            nsec = int.from_bytes(d[nt + 6:nt + 8], "little")
            sh = nt + 24 + int.from_bytes(d[nt + 20:nt + 22], "little")
            for k in range(nsec):
                e = sh + 40 * k
                va, raw, ptr = (int.from_bytes(d[e + o:e + o + 4], "little") for o in (12, 16, 20))
                self.ranges.append((va, raw, ptr))
        else:
            phoff = int.from_bytes(d[32:40], "little")
            phsz = int.from_bytes(d[54:56], "little")
            for k in range(int.from_bytes(d[56:58], "little")):
                e = phoff + k * phsz
                if int.from_bytes(d[e:e + 4], "little") == 1:  # PT_LOAD
                    off, va, fsz = (int.from_bytes(d[e + o:e + o + 8], "little") for o in (8, 16, 32))
                    self.ranges.append((va, fsz, off))

    def read(self, at, n):
        for va, size, off in self.ranges:
            if va <= at and at + n <= va + size:
                return self.data[off + at - va:off + at - va + n]
        return None


def call_before(img, ret):
    """the call instruction ending at ret: ("direct", target), ("indirect", None) or None (as
    the crash handlers decode it: E8 rel32, FF /2 with every ModRM / SIB form)"""
    b = img.read(ret - 7, 7)
    if b is None:
        return None

    def at(k):  # the byte k before ret
        return b[7 - k]

    if at(5) == 0xE8:
        return "direct", ret + int.from_bytes(b[3:7], "little", signed=True)
    for n in (2, 3, 4, 6, 7):
        m = at(n - 1)
        if at(n) != 0xFF or (m >> 3) & 7 != 2:
            continue
        md, rm = m >> 6, m & 7
        if md == 3 or (md == 0 and rm not in (4, 5)):
            want = 2
        elif md == 0 and rm == 5:
            want = 6
        elif md == 0:
            want = 7 if n >= 3 and at(n - 2) & 7 == 5 else 3
        elif md == 1:
            want = 4 if rm == 4 else 3
        else:
            want = 7 if rm == 4 else 6
        if want == n:
            return "indirect", None
    return None


# ---- Rust v0 symbol names (lld-link writes them mangled into the .map; ld.lld demangles)

B62 = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
BASIC = {"a": "i8", "b": "bool", "c": "char", "d": "f64", "e": "str", "f": "f32", "h": "u8", "i": "isize",
         "j": "usize", "l": "i32", "m": "u32", "n": "i128", "o": "u128", "s": "i16", "t": "u16", "u": "()",
         "v": "...", "x": "i64", "y": "u64", "z": "!", "p": "_"}


class V0:
    """the Rust v0 mangling grammar (rustc symbol-mangling-version=v0), printed like the LLVM
    demangler prints it in ld.lld's map: `<PaintCraft::PnApp as ui::App>::command`"""

    def __init__(self, s):
        self.s = s  # after "_R"
        self.p = 0
        self.bound = 0  # lifetimes bound by the enclosing binders (for<'a, ...>)

    def peek(self):
        return self.s[self.p] if self.p < len(self.s) else ""

    def take(self):
        c = self.peek()
        if not c:
            raise ValueError("end of symbol")
        self.p += 1
        return c

    def eat(self, c):
        if self.peek() == c:
            self.p += 1
            return True
        return False

    def b62(self):
        if self.eat("_"):
            return 0
        n = 0
        while True:
            c = self.take()
            if c == "_":
                return n + 1
            n = n * 62 + B62.index(c)

    def decimal(self):
        if self.eat("0"):
            return 0  # a leading 0 is the whole number
        start = self.p
        while self.peek().isdigit():
            self.p += 1
        if start == self.p:
            raise ValueError("number expected")
        return int(self.s[start:self.p])

    def ident(self):
        dis = self.b62() + 1 if self.eat("s") else 0
        puny = self.eat("u")
        n = self.decimal()
        self.eat("_")
        name = self.s[self.p:self.p + n]
        self.p += n
        if puny:
            try:
                k = name.rfind("_")
                name = (name[:k] + "-" + name[k + 1:] if k >= 0 else "-" + name).encode().decode("punycode")
            except Exception:
                pass
        return dis, name

    def backref(self, fn):
        pos = self.b62()
        save = self.p
        self.p = pos
        try:
            return fn()
        finally:
            self.p = save

    def path(self, in_type=False):
        c = self.take()
        if c == "C":
            return self.ident()[1]
        if c == "N":
            ns = self.take()
            inner = self.path(in_type)
            dis, name = self.ident()
            if ns.isupper():
                kind = {"C": "closure", "S": "shim"}.get(ns, ns)
                return f"{inner}::{{{kind}{':' + name if name else ''}#{dis}}}"
            return f"{inner}::{name}" if name else inner
        if c == "M":
            if self.eat("s"):
                self.b62()
            self.path()  # the impl's parent (not printed)
            return f"<{self.type()}>"
        if c == "X":
            if self.eat("s"):
                self.b62()
            self.path()
            t = self.type()
            return f"<{t} as {self.path(True)}>"
        if c == "Y":
            t = self.type()
            return f"<{t} as {self.path(True)}>"
        if c == "I":
            p = self.path(in_type)
            args = []
            while not self.eat("E"):
                a = self.generic_arg()
                if a is not None:
                    args.append(a)
            return f"{p}{'' if in_type else '::'}<{', '.join(args)}>"
        if c == "B":
            return self.backref(lambda: self.path(in_type))
        raise ValueError(f"path tag {c!r}")

    def lifetime_name(self, i):
        if i == 0:
            return "'_"
        if i - 1 >= self.bound:
            raise ValueError("unbound lifetime")
        d = self.bound - i
        return "'" + (chr(ord("a") + d) if d < 26 else f"z{d - 25}")

    def binder(self):
        """an optional binder G: for<'a, 'b> (the lifetimes stay bound for the caller's scope)"""
        if not self.eat("G"):
            return ""
        n = self.b62() + 1
        names = []
        for _ in range(n):
            self.bound += 1
            names.append(self.lifetime_name(1))
        return f"for<{', '.join(names)}> "

    def generic_arg(self):
        if self.eat("L"):
            return self.lifetime_name(self.b62())
        if self.eat("K"):
            return self.const()
        return self.type()

    def const(self):
        if self.eat("p"):
            return "_"
        if self.eat("B"):
            return self.backref(self.const)
        t = self.take()
        neg = self.eat("n")
        start = self.p
        while self.peek() != "_":
            self.take()
        digits = self.s[start:self.p]
        self.take()
        v = int(digits, 16) if digits else 0
        if t == "b":
            return "true" if v else "false"
        if t == "c":
            return repr(chr(v))
        return str(-v if neg else v)

    def fn_sig(self):
        save = self.bound
        try:
            return self.binder() + self.fn_sig_rest()
        finally:
            self.bound = save

    def fn_sig_rest(self):
        unsafe = self.eat("U")
        abi = ""
        if self.eat("K"):
            abi = '"C"' if self.eat("C") else '"' + self.ident()[1].replace("_", "-") + '"'
        args = []
        while not self.eat("E"):
            args.append(self.type())
        s = ("unsafe " if unsafe else "") + (f"extern {abi} " if abi else "") + f"fn({', '.join(args)})"
        return s if self.eat("u") else f"{s} -> {self.type()}"

    def type(self):
        c = self.peek()
        if c in BASIC:
            self.p += 1
            return BASIC[c]
        self.p += 1
        if c == "A":
            t = self.type()
            return f"[{t}; {self.const()}]"
        if c == "S":
            return f"[{self.type()}]"
        if c == "T":
            ts = []
            while not self.eat("E"):
                ts.append(self.type())
            return f"({ts[0]},)" if len(ts) == 1 else f"({', '.join(ts)})"
        if c in "RQ":
            lt = ""
            if self.eat("L"):
                i = self.b62()
                lt = self.lifetime_name(i) + " " if i else ""
            return "&" + lt + ("mut " if c == "Q" else "") + self.type()
        if c in "PO":
            return ("*const " if c == "P" else "*mut ") + self.type()
        if c == "F":
            return self.fn_sig()
        if c == "D":
            save = self.bound
            try:
                b = self.binder()
                traits = []
                while not self.eat("E"):
                    t = self.path(True)
                    binds = []
                    while self.eat("p"):
                        name = self.ident()[1]
                        binds.append(f"{name} = {self.type()}")
                    if binds:
                        t = t[:-1] + ", " + ", ".join(binds) + ">" if t.endswith(">") else t + "<" + ", ".join(binds) + ">"
                    traits.append(t)
            finally:
                self.bound = save
            s = "dyn " + b + " + ".join(traits)
            if not self.eat("L"):
                raise ValueError("dyn without lifetime")
            i = self.b62()
            return s + (" + " + self.lifetime_name(i) if i else "")
        if c == "B":
            return self.backref(self.type)
        self.p -= 1
        return self.path(True)


def demangle(name):
    """a Rust v0 name (with or without an LLVM suffix such as .llvm.123) demangled; any other
    name as it is (without the suffix)"""
    base = re.sub(r"\.llvm\.\d+$", "", name)
    base = re.sub(r" \(\.llvm\.\d+\)$", "", base)  # ld.lld's demangled form of the suffix
    if not base.startswith("_R"):
        return base
    s = base[2:]
    i = 0
    while i < len(s) and s[i].isdigit():
        i += 1  # an encoding version
    try:
        d = V0(s)
        d.p = i
        return d.path()
    except (ValueError, IndexError):
        return base


class Check:
    def __init__(self, target, out, mode="auto"):
        self.target = target
        self.out = out
        self.mode = mode
        self.fails = 0

    def ok(self, cond, text, detail=""):
        print(f"[{self.target}] {'PASS' if cond else 'FAIL'} {text}" + (f" -- {detail}" if detail and not cond else ""))
        self.fails += 0 if cond else 1
        return cond

    def run(self, args):
        exe = (str(self.out / "win" / "PaintCraft.exe") if self.target == "win"
               else "./" + rel(self.out / "linux" / "PaintCraft"))
        cmd = [exe] + args
        env = dict(os.environ)
        env.pop("PaintCraft_CONFIG_DIR", None)
        if self.target == "linux":
            cmd = WSL + ["env", "-u", "PaintCraft_CONFIG_DIR"] + cmd
        return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
                              errors="replace", env=env, timeout=120)

    def main(self):
        base = self.out / "pn_crash_check" / self.target
        shutil.rmtree(base, ignore_errors=True)
        base.mkdir(parents=True)
        script = base / "crash.uis"
        script.write_text("size 1280 800\ncmd 911\n", encoding="utf-8")
        crash_dir = base / "logs"
        res = self.run(["--crash-dir", rel(crash_dir), "--script", rel(script)])
        self.ok(res.returncode == 3, "the crash ends the process with exit code 3", f"exit {res.returncode}")
        self.ok("jfgui: crash" in res.stderr, "the library prints its stderr line", res.stderr[-200:])
        logs = sorted(crash_dir.glob("Absturz_*.txt")) if crash_dir.exists() else []
        if not self.ok(len(logs) == 1, "one protocol file Absturz_*.txt", str(logs)):
            return
        log = logs[0]
        self.ok(re.match(r"Absturz_\d{4}-\d\d-\d\d_\d\d-\d\d-\d\d\.txt$", log.name) is not None,
                "the file name carries date and time", log.name)
        text = log.read_text(encoding="utf-8")
        for needle in ["PaintCraft ist abgestürzt.", "Zugriffsverletzung (beim Schreiben an Adresse 0x10)",
                       "RAX 0000000000000010", "Register:", "Aufrufkette", "---- Diagnose ----",
                       "Logische Prozessoren:"]:
            self.ok(needle in text, f"protocol contains {needle!r}")
        exe = "PaintCraft.exe" if self.target == "win" else "PaintCraft"
        offs = [int(m, 16) for m in re.findall(re.escape(exe) + r" \+ 0x([0-9a-f]+)\n", text)]
        mp = self.out / self.target / "PaintCraft.map"
        rust = self.mode == "rust" or (self.mode == "auto" and map_is_rust(mp, self.target))
        if rust:
            self.rust_chain(offs, load_map(mp, self.target, True))
        else:
            syms = load_map(mp, self.target)
            names = []
            for o in offs:
                i = bisect.bisect_right(syms, (o, "~")) - 1
                names.append(syms[i][1] if i >= 0 else "?")
            chain = " <- ".join(names)
            self.ok(names[:3] == ["crash_test", "a1_diag_cmd", "pn_cmd"],
                    "the call chain starts crash_test <- a1_diag_cmd <- pn_cmd", chain)
            self.ok(bool(names) and names[-1] in ("plat_entry", "_start"),
                    "the call chain reaches the entry point", chain)
            print(f"[{self.target}]   chain: {chain}")
        # the report window shows this protocol
        rscript = base / "report.uis"
        rscript.write_text(f'size 680 520\nexpect text:"PaintCraft wurde unerwartet beendet" exists\n'
                           f'expect #6720 text "{rel(log)}"\nclick #6701\nexpect #6706 exists\n',
                           encoding="utf-8")
        res = self.run(["--crash-report", rel(log), "--script", rel(rscript)])
        fails = [l for l in res.stdout.splitlines() if l.startswith("FAIL")]
        self.ok(res.returncode == 0 and not fails, "--crash-report shows the protocol",
                "; ".join(fails) or f"exit {res.returncode}")

    def rust_chain(self, offs, syms):
        """the call chain of the Rust build: crash_test <- a1_diag_cmd <- diag_cmd ...
        jfgui_main <- the entry, and exact (every link is a call into the function below)"""
        funcs = resolve(syms, offs, True)
        names = [n for _, n in funcs]
        chain = " <- ".join(names)
        print(f"[{self.target}]   (Rust build: names demangled)")
        # Current build_rs normalizes the Rust crate name to lowercase. Keep the
        # historical spelling for older maps; function order remains exact.
        starts_correctly = any(names[:3] == ["plat::crash_test", f"{crate}::diag::a1_diag_cmd", f"{crate}::diag::diag_cmd"]
                               for crate in ("PaintCraft", "PaintCraft"))
        self.ok(starts_correctly,
                "the call chain starts plat::crash_test <- PaintCraft::diag::a1_diag_cmd <- PaintCraft::diag::diag_cmd",
                chain)
        entry = "mainCRTStartup" if self.target == "win" else "_start"
        self.ok("jfgui_main" in names and names[-1] == entry,
                f"the call chain reaches jfgui_main and the entry point {entry}", chain)
        exe = self.out / self.target / ("PaintCraft.exe" if self.target == "win" else "PaintCraft")
        img = Image(exe, self.target)
        bad = []
        for k in range(1, len(offs)):
            call = call_before(img, offs[k])
            if call is None:
                bad.append(f"{names[k]}: no call before 0x{offs[k]:x}")
            elif call[0] == "direct" and call[1] != funcs[k - 1][0]:
                bad.append(f"{names[k]}: calls 0x{call[1]:x}, not {names[k - 1]}")
        self.ok(len(offs) > 1 and not bad,
                "every return address follows a call into the function below it (an exact chain)",
                "; ".join(bad) or "no frames")
        print(f"[{self.target}]   chain: {chain}")


def resolve_protocol(path, out, mode):
    """--resolve: the chain of a protocol, one line per address, with the functions of the
    build's .map (the target from the protocol's "PaintCraft.exe + 0x..." lines)"""
    text = pathlib.Path(path).read_text(encoding="utf-8", errors="replace")
    target = "win" if "PaintCraft.exe + 0x" in text else "linux"
    exe = "PaintCraft.exe" if target == "win" else "PaintCraft"
    offs = [int(m, 16) for m in re.findall(re.escape(exe) + r" \+ 0x([0-9a-f]+)\n", text)]
    mp = out / target / "PaintCraft.map"
    rust = mode == "rust" or (mode == "auto" and map_is_rust(mp, target))
    for k, ((start, name), o) in enumerate(zip(resolve(load_map(mp, target, rust), offs, rust), offs)):
        where = f"{name} + 0x{o - start:x}" if start is not None else name
        print(f"  {exe} + 0x{o:x}  {where}{'  (crash)' if k == 0 else ''}")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build")
    ap.add_argument("--mode", default="auto", choices=["auto", "asm", "rust"],
                    help="asm or Rust build (auto: from the .map)")
    ap.add_argument("--resolve", metavar="PROTOCOL", help="print the call chain of a crash protocol")
    a = ap.parse_args()
    out = ROOT / a.out
    if a.resolve:
        return resolve_protocol(a.resolve, out, a.mode)
    fails = 0
    for t in (["win", "linux"] if a.target == "both" else [a.target]):
        c = Check(t, out, a.mode)
        c.main()
        fails += c.fails
    print("pn_crash_check:", "all passed" if fails == 0 else f"{fails} failed")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())

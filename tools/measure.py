#!/usr/bin/env python3
"""measure.py -- the R13 measurement report (rewrite/REPORT.md): the Rust rebuild (rs/) against
the frozen assembly reference (legacy/, binaries in build/rwref).

  python tools/measure.py build  [--targets win,linux] [--modes asm,dev,rel] [--runs N]
        clean and incremental build times (one app file / one ui file edited), files in the
        build folder.  Outputs stay in build/r13-measure/{asm,dev,rel}/<target> and are the
        binaries the other steps measure.
  python tools/measure.py size   [--md]
        sizes of every app: asm (build/rwref), Rust dev, Rust release; plugins; sections;
        the biggest symbols / crates from the .map files
  python tools/measure.py sizexp [--variants a,b] [--apps ...] [--targets win,linux] [--no-speed]
        size-oriented release settings (opt-level s / z, LTO variants, codegen units, per-crate
        opt levels, linker flags): size of gallery / Kritzel / PaintCraft and speed checks per variant
  python tools/measure.py speed  [--parts startup,bench,scripts,codec,tools] [--targets ...] [--runs N]
        start-up + first frame, frame build / render times (`bench` script command), every
        apps/*/tests/*.uis, codec conversions (12 MP), the check tools' bench suites;
        asm against Rust release, best of N
  python tools/measure.py code   [--md]
        lines of code without blank / comment lines per area, `unsafe` per crate, tests
  python tools/measure.py report
        every table from the JSON files (build/r13-measure/*.json) as Markdown

Linux binaries run in WSL (Ubuntu-24.04).  For timings they are copied to the WSL file system
first (/tmp/r13m): binaries and inputs on /mnt/c go through 9P, which would dominate start-up
and file I/O.  Raw data: build/r13-measure/*.json + logs.  Machine load is sampled before every
timed step (Windows: Win32_Processor.LoadPercentage, WSL: /proc/loadavg) and stored with it.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import statistics
import struct
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RS = ROOT
M = ROOT / "build" / "r13-measure"
REF = ROOT / "build" / "rwref"          # the asm reference binaries (copied from the main checkout)
LLVM = Path(r"C:\Program Files\LLVM\bin")
WSL = ["wsl.exe", "-d", "Ubuntu-24.04"]
WSL_TMP = "/tmp/r13m"
sys.path.insert(0, str(RS))
sys.path.insert(0, str(ROOT / "tools"))

from project_sources import entrypoint, project_sources
APPS_RS = sorted(p.name for p in (RS / "apps").iterdir() if p.is_dir() and entrypoint(p, "app", False))
RS_NAME = {"paint": "Kritzel"}      # assembly app -> its Rust app where the name changed
APPS_ASM = sorted(RS_NAME.get(p.name, p.name) for p in (ROOT.parent / "legacy" / "apps").iterdir() if (p / "src").is_dir())


def owned_rust_sources():
    """Measure the same sources as the build, excluding books and external reference code."""
    roots = []
    for folder, kind in (("apps", "app"), ("lib", "lib"), ("styles", "lib"), ("plugins", "lib"), ("modules", "lib")):
        for directory in (RS / folder).iterdir():
            if directory.is_dir():
                root = entrypoint(directory, kind, False)
                if root:
                    roots.append(root)
    return sorted({source for root in roots for source in project_sources(root)})


def tool(name):
    p = LLVM / (name + ".exe")
    return str(p) if os.name == "nt" and p.exists() else name


def app_targets(name):
    f = ROOT / "apps" / name / "TARGETS"
    return f.read_text().split() if f.exists() else ["win", "linux"]


def exe_name(name, target):
    return name + (".exe" if target == "win" else "")


def wsl_path(p):
    p = str(Path(p).resolve())
    return "/mnt/" + p[0].lower() + p[2:].replace("\\", "/") if p[1:2] == ":" else p


def save(name, data):
    M.mkdir(parents=True, exist_ok=True)
    (M / f"{name}.json").write_text(json.dumps(data, indent=1), encoding="utf-8")


def load(name):
    f = M / f"{name}.json"
    return json.loads(f.read_text(encoding="utf-8")) if f.exists() else {}


def load_sample():
    """machine load: Windows CPU % (one sample) and the WSL 1-minute load average"""
    out = {}
    try:
        r = subprocess.run(["powershell", "-NoProfile", "-Command",
                            "(Get-CimInstance Win32_Processor).LoadPercentage"],
                           capture_output=True, text=True, timeout=30)
        out["win_cpu_pct"] = int(r.stdout.strip().split()[0])
    except Exception:
        pass
    return out


def wsl_load():
    try:
        r = subprocess.run(WSL + ["-e", "cat", "/proc/loadavg"], capture_output=True, text=True, timeout=30)
        return float(r.stdout.split()[0])
    except Exception:
        return None


def fmt_bytes(n):
    return f"{n:,}".replace(",", ".") if n is not None else "–"


def fmt_kb(n):
    return f"{n / 1024:,.0f}".replace(",", ".") if n else "–"


# ====================================================================== build times
APP_EDIT = {"rs": RS / "apps" / "Kritzel" / "tools.rs", "asm": ROOT.parent / "legacy" / "apps" / "paint" / "src" / "tools.s"}
LIB_EDIT = {"rs": RS / "lib" / "ui" / "basic.rs", "asm": ROOT.parent / "legacy" / "ASMLIB" / "src" / "ui" / "ui_basic.s"}


def mode_dir(mode, target):
    return M / mode / target


def build_cmd(mode, target):
    """the command that builds the library + every app of one target for a mode"""
    if mode == "asm":
        # the apps the rebuild has (legacy/build.py all also links colortest and icongen)
        return [sys.executable, str(Path(__file__)), "_asmbuild", target, str(M / "asm")]
    cmd = [sys.executable, str(RS / "build_rs.py"), "all", "--target", target, "--out", str(M / mode)]
    return cmd + (["--release"] if mode == "rel" else [])


def asm_build(target, out):
    """(internal) legacy/build.py: the library + the apps of the rebuild, one target"""
    sys.path.insert(0, str(ROOT.parent / "legacy"))
    import build as lb
    out = Path(out).resolve()
    lb.OUT_DIR = out
    lb.build_lib(target, out)
    for n in lb.app_names(target):
        rs = RS_NAME.get(n, n)
        if rs in APPS_RS:
            lb.build_app(n, target, out)
            if rs != n:                 # compared under the Rust app's name
                for f in (out / target).glob(n + ".*") if target == "win" else [out / target / n]:
                    if f.exists():
                        f.replace(f.with_name(rs + f.name[len(n):]))
    return 0


def timed_run(cmd, log):
    t0 = time.perf_counter()
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=ROOT)
    dt = time.perf_counter() - t0
    log.write(f"$ {' '.join(map(str, cmd))}\n{r.stdout}{r.stderr}\n-> {r.returncode} in {dt:.2f} s\n")
    log.flush()
    if r.returncode:
        raise SystemExit(f"build failed: {' '.join(map(str, cmd))}\n{r.stdout[-3000:]}{r.stderr[-3000:]}")
    return dt


def edit_file(path, lang):
    """append one comment line (Rust: the hash cache sees a change; asm: a newer mtime) -> the
    original text"""
    text = path.read_bytes()
    path.write_bytes(text + (b"// r13-measure: incremental build probe\n" if lang == "rs" else
                             b"# r13-measure: incremental build probe\n"))
    return text


def folder_stats(d):
    files = [p for p in Path(d).rglob("*") if p.is_file()]
    return len(files), sum(p.stat().st_size for p in files)


def cmd_build(a):
    res = load("build")
    targets = a.targets.split(",")
    modes = a.modes.split(",")
    log = open(M / "build.log", "a", encoding="utf-8")
    for mode in modes:
        lang = "asm" if mode == "asm" else "rs"
        for t in targets:
            key = f"{mode}_{t}"
            out = mode_dir(mode, t)
            r = {"clean": [], "inc_app": [], "inc_lib": [], "load": []}
            for i in range(a.runs):
                r["load"].append(load_sample())
                if out.exists():
                    shutil.rmtree(out)
                if mode == "asm" and t == "win" and (M / "asm" / "win" / "implib").exists():
                    shutil.rmtree(M / "asm" / "win" / "implib")
                dt = timed_run(build_cmd(mode, t), log)
                r["clean"].append(dt)
                print(f"[{key}] clean {dt:.1f} s", flush=True)
            steps = [("inc_app", APP_EDIT[lang])] * a.inc_runs + [("inc_lib", LIB_EDIT[lang])] * a.inc_runs
            for i, (kind, f) in enumerate(steps):
                orig = edit_file(f, lang)
                try:
                    if lang == "asm":
                        time.sleep(1.1)     # mtime resolution
                    dt = timed_run(build_cmd(mode, t), log)
                finally:
                    f.write_bytes(orig)
                r[kind].append(dt)
                print(f"[{key}] {kind} {dt:.1f} s", flush=True)
                if lang == "asm":
                    time.sleep(1.1)
                # back to the unedited state (not timed); not needed before the first ui edit,
                # which rebuilds every app anyway.  Release: never (the probe is a comment at the
                # end of a file, so the binaries are the same; a release rebuild of every app
                # takes many minutes) -- no "nothing changed" timing then
                if mode != "rel" and not (kind == "inc_app" and i + 1 < len(steps) and steps[i + 1][0] == "inc_lib"):
                    timed_run(build_cmd(mode, t), log)
            if mode != "rel":
                r["noop"] = timed_run(build_cmd(mode, t), log)
            r["files"], r["bytes"] = folder_stats(out)
            res[key] = r
            save("build", res)
    log.close()
    print_build(res)
    return 0


def print_build(res):
    print("\n| Build | Target | sauber | App-Datei geändert | ui-Datei geändert | nichts geändert | Dateien | Größe |")
    print("|---|---|---|---|---|---|---|---|")
    names = {"asm": "asm (legacy/build.py)", "dev": "Rust dev", "rel": "Rust release"}
    for key, r in res.items():
        mode, t = key.split("_")
        f = lambda v: f"{min(v):.1f} s" if v else "–"  # noqa: E731
        print(f"| {names[mode]} | {t} | {f(r['clean'])} | {f(r['inc_app'])} | {f(r['inc_lib'])} | "
              f"{f([r['noop']]) if 'noop' in r else '–'} | {r['files']} | {r['bytes'] / 1e6:.0f} MB |")


# ====================================================================== size
def pe_sections(path):
    """PE file -> [(name, virtual size, raw size in the file)]"""
    b = Path(path).read_bytes()
    nt = struct.unpack_from("<I", b, 0x3C)[0]
    nsec, ohsize = struct.unpack_from("<HH", b, nt + 6)[0], struct.unpack_from("<H", b, nt + 20)[0]
    s0 = nt + 24 + ohsize
    out = []
    for k in range(nsec):
        s = s0 + 40 * k
        name = b[s:s + 8].rstrip(b"\0").decode("ascii", "replace")
        vs, va, rs = struct.unpack_from("<III", b, s + 8)
        out.append((name, vs, rs))
    return out


def elf_sections(path):
    """ELF64 file -> [(name, size, occupies the file)] of the allocated sections"""
    b = Path(path).read_bytes()
    shoff = struct.unpack_from("<Q", b, 0x28)[0]
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x3A)
    secs = [struct.unpack_from("<IIQQQQIIQQ", b, shoff + k * shentsize) for k in range(shnum)]
    stro = secs[shstrndx][4]
    out = []
    for nm, typ, flags, addr, off, size, *_ in secs:
        if not flags & 2:           # SHF_ALLOC
            continue
        name = b[stro + nm:b.index(b"\0", stro + nm)].decode()
        out.append((name, size, typ != 8))       # SHT_NOBITS (.bss) is not in the file
    return out


def section_groups(path, target):
    """file size split into code / read-only data / data / relocations + headers + other"""
    total = Path(path).stat().st_size
    g = {"code": 0, "rodata": 0, "data": 0, "reloc": 0, "unwind": 0}
    if target == "win":
        for name, vs, rs in pe_sections(path):
            k = {".text": "code", ".rdata": "rodata", ".rodata": "rodata", ".data": "data", ".reloc": "reloc",
                 ".pdata": "unwind", ".xdata": "unwind"}.get(name, "data")
            g[k] += rs
    else:
        for name, size, infile in elf_sections(path):
            if not infile:
                continue
            k = ("code" if name.startswith(".text") else "unwind" if name.startswith((".eh_frame", ".gcc_except"))
                 else "rodata" if name.startswith((".rodata", ".data.rel.ro")) else
                 "reloc" if name.startswith(".rela") else "data")
            g[k] += size
    g["other"] = total - sum(g.values())
    return g


def demangle(names):
    """Rust / C++ symbol names -> demangled (llvm-cxxfilt); unchanged where it cannot"""
    if not names:
        return []
    r = subprocess.run([tool("llvm-cxxfilt")], input="\n".join(names), capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    out = r.stdout.splitlines()
    return out if len(out) == len(names) else names


MAP_LINE = re.compile(r"^\s*([0-9a-f]+)\s+([0-9a-f]+)\s+([0-9a-f]+)\s+(\d+)\s(.*)$")


def elf_map_sections(mapfile):
    """ld.lld -Map -> [(output section, size, input object, input section name, first symbol)] for
    every input section (symbols as the map prints them: demangled)"""
    out = []
    cur_out = None
    for ln in Path(mapfile).read_text(encoding="utf-8", errors="replace").splitlines():
        m = MAP_LINE.match(ln)
        if not m:
            continue
        size, rest = int(m.group(3), 16), m.group(5)
        ind = len(rest) - len(rest.lstrip(" "))
        rest = rest.strip()
        if ind == 0:
            cur_out = rest
        elif ind <= 8:
            mm = re.match(r"(.*):\((.*)\)$", rest)
            obj, sec = (mm.group(1), mm.group(2)) if mm else (rest, "")
            out.append([cur_out, size, obj, sec, None])
        elif out and out[-1][4] is None:            # the first symbol of the input section
            out[-1][4] = rest
    return out


RS_CRATES = None


def rs_crate_names():
    global RS_CRATES
    if RS_CRATES is None:
        RS_CRATES = {p.name for p in (RS / "lib").iterdir() if p.is_dir()} | \
                    {f"style_{p.name}" for p in (RS / "styles").iterdir() if p.is_dir()} | set(APPS_RS)
    return RS_CRATES


def sym_crate(sym):
    """demangled Rust symbol -> the crate its code belongs to (generic code of core / alloc
    instantiated for our types counts as core / alloc)"""
    s = sym.lstrip("<&*[(").replace("dyn ", "").replace("mut ", "").replace("const ", "")
    m = re.match(r"([A-Za-z_][A-Za-z0-9_]*)::", s)
    return m.group(1) if m else ("compiler_builtins/mem" if s in ("memcpy", "memset", "memmove", "memcmp", "bcmp")
                                 else "?")


def rs_breakdown(mapfile, binary=None):
    """a Rust ELF map -> bytes per crate (code / data), per category, the biggest items"""
    secs = elf_map_sections(mapfile)
    crates = {}
    cats = {"core::fmt": 0, "Panic-/Fehlerpfade": 0, "SIMD AVX2-Instanzen (run_avx2)": 0,
            "SIMD dispatch (SSE2-Pfad inline)": 0, "generische Instanzen (::<..>)": 0, "core/alloc": 0,
            "anonyme RO-Daten": 0, ".eh_frame / unwind": 0}
    biggest = []
    for out, size, obj, sec, sym in secs:
        if not size or out in (".bss", ".tbss"):
            continue
        kind = "code" if out.startswith(".text") else "data"
        if out.startswith((".eh_frame", ".gcc_except")):
            cats[".eh_frame / unwind"] += size
        mobj = re.search(r"lib([A-Za-z0-9_]+)\.rlib\(", obj)
        crate = mobj.group(1) if mobj else None
        if sym and not sym.startswith("."):
            c2 = sym_crate(sym)
            crate = c2 if c2 != "?" else (crate or "?")
        elif crate is None:
            mm = re.search(r"[\\/]([A-Za-z0-9_]+)\.[a-z0-9]+\.[a-z0-9]+\.rcgu\.o", obj)
            crate = ("anon (LTO)" if mm else ("<internal>" if obj.startswith("<internal>") else Path(obj).name))
        if kind == "data" and (not sym or sym.startswith(".")) and ".Lanon" in sec:
            cats["anonyme RO-Daten"] += size
        e = crates.setdefault(crate, {"code": 0, "data": 0})
        e[kind] += size
        if sym:
            if "core::fmt" in sym:
                cats["core::fmt"] += size
            if re.search(r"panic|unwrap_failed|expect_failed|_fail\b|index_order_fail|len_mismatch_fail", sym):
                cats["Panic-/Fehlerpfade"] += size
            if "run_avx2" in sym or "Avx2>" in sym:
                cats["SIMD AVX2-Instanzen (run_avx2)"] += size
            elif sym.startswith("simd::dispatch::<"):
                cats["SIMD dispatch (SSE2-Pfad inline)"] += size
            if "::<" in sym:
                cats["generische Instanzen (::<..>)"] += size
            if crate in ("core", "alloc"):
                cats["core/alloc"] += size
        biggest.append((size, kind, crate, (sym or sec)[:140]))
    biggest.sort(reverse=True)
    return crates, cats, biggest[:40]


def panic_locations(binary):
    """panic locations in a Rust binary: the source path strings (rs/..., /rustc/.../library/...)
    and every (pointer, length) pair that refers to one (each is the file of a
    core::panic::Location, 24 bytes) -> (path string bytes, locations)"""
    b = Path(binary).read_bytes()
    if b[:4] == b"\x7fELF":
        phoff, phnum = struct.unpack_from("<Q", b, 0x20)[0], struct.unpack_from("<H", b, 0x38)[0]
        segs = []
        for k in range(phnum):
            typ, fl, off, va, pa, fsz, msz, al = struct.unpack_from("<IIQQQQQQ", b, phoff + 56 * k)
            if typ == 1:
                segs.append((off, va, fsz))

        def va_of(o):
            for off, va, fsz in segs:
                if off <= o < off + fsz:
                    return va + o - off
            return None
    else:
        nt = struct.unpack_from("<I", b, 0x3C)[0]
        base = struct.unpack_from("<Q", b, nt + 24 + 24)[0]
        secs = []
        nsec, ohsize = struct.unpack_from("<H", b, nt + 6)[0], struct.unpack_from("<H", b, nt + 20)[0]
        for k in range(nsec):
            sh = nt + 24 + ohsize + 40 * k
            vs, rva, rs, rp = struct.unpack_from("<IIII", b, sh + 8)
            secs.append((rp, rva, rs))

        def va_of(o):
            for rp, rva, rs in secs:
                if rp <= o < rp + rs:
                    return base + rva + o - rp
            return None
    # (--remap-path-prefix on a Windows host: rs\lib\...; the strings end with a NUL)
    pat = re.compile(rb"(?:rs[\\/](?:lib|apps|styles|plugins)[\\/][A-Za-z0-9_\\/]+\.rs|"
                     rb"/rustc/[0-9a-f]{40}/library/[A-Za-z0-9_/]+\.rs)")
    strs = {}
    for m in pat.finditer(b):
        strs[m.start()] = len(m.group(0))
    nloc = 0
    for o, ln in strs.items():
        va = va_of(o)
        if va is None:
            continue
        nloc += b.count(struct.pack("<QQ", va, ln))
    return sum(strs.values()), nloc


def assets_in(binary):
    """bytes of the include_bytes! files found verbatim in a binary"""
    b = Path(binary).read_bytes()
    tot = 0
    for f, n in embedded_assets().items():
        data = (ROOT / f).read_bytes()
        if len(data) > 64 and data in b:
            tot += n
    return tot


def asm_breakdown(mapfile):
    """an asm ELF map -> bytes per library area / app (by object file)"""
    areas = {}
    for out, size, obj, sec, sym in elf_map_sections(mapfile):
        if not size or out in (".bss", ".tbss"):
            continue
        kind = "code" if out.startswith(".text") else "data"
        m = re.search(r"libjfgui\.a\(([A-Za-z0-9_]+)\.o\)", obj)
        if m:
            mod = m.group(1)
            area = ASM_MOD_AREA.get(mod) or asm_mod_area(mod)
        elif "libstyle_" in obj:
            area = "styles"
        elif "obj" in obj and "apps" in obj:
            area = "app"
        else:
            area = "<internal>"
        e = areas.setdefault(area, {"code": 0, "data": 0})
        e[kind] += size
    return areas


ASM_MOD_AREA = {}


def asm_mod_area(mod):
    """module name -> area of legacy/ASMLIB/src (looked up once)"""
    if not ASM_MOD_AREA:
        for p in (ROOT.parent / "legacy" / "ASMLIB" / "src").rglob("*.s"):
            sub = p.relative_to(ROOT.parent / "legacy" / "ASMLIB" / "src").parts[0]
            ASM_MOD_AREA[p.stem] = {"core": "rt_plat", "plat": "rt_plat", "img": "codecs"}.get(sub, sub)
    return ASM_MOD_AREA.get(mod, "?")


RS_CRATE_AREA = {"rt": "rt_plat", "simd": "rt_plat", "plat": "rt_plat", "base": "rt_plat", "gfx": "gfx",
                 "text": "text", "ui": "ui", "fx": "fx"}


def rs_area_of(crate):
    if crate in RS_CRATE_AREA:
        return RS_CRATE_AREA[crate]
    if crate.startswith("img_"):
        return "codecs"
    if crate.startswith("style_"):
        return "styles"
    if crate in APPS_RS:
        return "app"
    if crate in ("core", "alloc", "compiler_builtins", "compiler_builtins/mem"):
        return "core/alloc"
    return "other"


def plugin_sizes():
    """Rust plugins (build_rs.py plugin, release) against the asm plugins (build/rwref)"""
    out = {}
    for t in ("win", "linux"):
        ext = ".dll" if t == "win" else ".so"
        for p in sorted((REF / t / "plugins").glob("*" + ext)):
            out.setdefault(p.stem, {})[f"asm_{t}"] = p.stat().st_size
        for p in sorted((M / "plugins" / t / "plugins").glob("*" + ext)):
            out.setdefault(p.stem, {})[f"rs_{t}"] = p.stat().st_size
    return out


def same_as_ref(a, b):
    """two builds of an exe equal apart from the PE time stamp?"""
    x, y = bytearray(Path(a).read_bytes()), bytearray(Path(b).read_bytes())
    if len(x) != len(y):
        return False
    if x[:2] == b"MZ":
        nt = struct.unpack_from("<I", x, 0x3C)[0]
        x[nt + 8:nt + 12] = y[nt + 8:nt + 12]
        # the debug directory / map time stamps are not in the file (no /DEBUG)
    return x == y


def cmd_size(a):
    res = {"apps": {}, "groups": {}, "ref_equal": {}, "breakdown": {}}
    for name in sorted(set(APPS_RS) | set(APPS_ASM), key=str.lower):
        row = {}
        for t in ("win", "linux"):
            if t not in app_targets(name):
                continue
            for mode, d in (("asm", M / "asm" / t), ("ref", REF / t), ("dev", M / "dev" / t), ("rel", M / "rel" / t)):
                f = d / exe_name(name, t)
                if f.exists():
                    row[f"{mode}_{t}"] = f.stat().st_size
                    if mode in ("asm", "dev", "rel"):
                        res["groups"][f"{name}_{mode}_{t}"] = section_groups(f, t)
            fa, fr = M / "asm" / t / exe_name(name, t), REF / t / exe_name(name, t)
            if fa.exists() and fr.exists():
                res["ref_equal"][f"{name}_{t}"] = same_as_ref(fa, fr)
        res["apps"][name] = row
    # where the bytes are: the Linux maps (exact input-section sizes)
    for name in ("template", "gallery", "Kritzel", "PaintCraft", "codectest"):
        for mode in ("dev", "rel"):
            mp = M / mode / "linux" / (name + ".map")
            if mp.exists():
                crates, cats, big = rs_breakdown(mp)
                e = {"crates": crates, "cats": cats, "biggest": big}
                for t in ("win", "linux"):
                    exe = M / mode / t / exe_name(name, t)
                    if exe.exists():
                        e[f"panic_loc_{t}"] = panic_locations(exe)
                        e[f"assets_{t}"] = assets_in(exe)
                res["breakdown"][f"{name}_{mode}"] = e
        mp = M / "asm" / "linux" / (name + ".map")
        if mp.exists():
            res["breakdown"][f"{name}_asm"] = {"areas": asm_breakdown(mp)}
    res["plugins"] = plugin_sizes()
    res["assets"] = embedded_assets()
    save("size", res)
    print_size(res)
    return 0


def embedded_assets():
    """files the Rust code embeds with include_bytes! -> {file: bytes}"""
    out = {}
    for p in owned_rust_sources():
        for m in re.finditer(r'include_bytes!\(\s*"([^"]+)"\s*\)', p.read_text(encoding="utf-8")):
            f = (p.parent / m.group(1)).resolve()
            if f.exists():
                out[f.relative_to(ROOT).as_posix()] = f.stat().st_size
    return out


def print_size(res):
    print("\n| App | asm win | Rust dev win | Rust release win | rel/asm | asm linux | Rust dev linux | Rust release linux | rel/asm |")
    print("|---|---|---|---|---|---|---|---|---|")
    for name, r in res["apps"].items():
        cells = []
        for t in ("win", "linux"):
            asm, dev, rel = r.get(f"asm_{t}"), r.get(f"dev_{t}"), r.get(f"rel_{t}")
            cells += [fmt_kb(asm), fmt_kb(dev), fmt_kb(rel), f"{rel / asm:.2f}" if asm and rel else "–"]
        print(f"| {name} | " + " | ".join(cells) + " |")
    print("\n(KB = 1024 Bytes)")
    neq = [k for k, v in res["ref_equal"].items() if not v]
    print(f"asm-Neubau gleich build/rwref (ohne PE-Zeitstempel): {sum(res['ref_equal'].values())} von "
          f"{len(res['ref_equal'])}" + (f"; anders: {', '.join(neq)}" if neq else ""))
    print("\n| App / Build | Code | RO-Daten | Daten | Relocs | Unwind | Rest |")
    print("|---|---|---|---|---|---|---|")
    for key, g in res["groups"].items():
        if key.split("_")[0] in ("gallery", "Kritzel", "PaintCraft", "template", "codectest"):
            print(f"| {key} | {fmt_kb(g['code'])} | {fmt_kb(g['rodata'])} | {fmt_kb(g['data'])} | "
                  f"{fmt_kb(g['reloc'])} | {fmt_kb(g['unwind'])} | {fmt_kb(g['other'])} |")
    for key, b in res["breakdown"].items():
        if "crates" in b:
            areas = {}
            for c, e in b["crates"].items():
                ar = rs_area_of(c)
                x = areas.setdefault(ar, {"code": 0, "data": 0})
                x["code"] += e["code"]
                x["data"] += e["data"]
            print(f"\n{key} (Linux-Map): " + ", ".join(f"{ar} {fmt_kb(e['code'])}+{fmt_kb(e['data'])}"
                                                    for ar, e in sorted(areas.items(), key=lambda kv: -sum(kv[1].values()))))
            print("   Kategorien: " + ", ".join(f"{k} {fmt_kb(v)}" for k, v in b["cats"].items()))
            for t in ("win", "linux"):
                if f"panic_loc_{t}" in b:
                    sb, nl = b[f"panic_loc_{t}"]
                    print(f"   {t}: Panic-Orte {nl} × 24 B = {fmt_kb(nl * 24)} KB + Pfade {fmt_kb(sb)} KB; "
                          f"eingebettete Dateien {fmt_kb(b[f'assets_{t}'])} KB")
            print("   größte: " + "; ".join(f"{s // 1024} KB {c} {n[:70]}" for s, k, c, n in b["biggest"][:12]))
        else:
            print(f"\n{key} (Linux-Map): " + ", ".join(f"{ar} {fmt_kb(e['code'])}+{fmt_kb(e['data'])}"
                                                    for ar, e in sorted(b["areas"].items(), key=lambda kv: -sum(kv[1].values()))))
    print("\n| Plugin | asm win | Rust win | asm linux | Rust linux |")
    print("|---|---|---|---|---|")
    for n, r in res["plugins"].items():
        print(f"| {n} | {fmt_bytes(r.get('asm_win'))} | {fmt_bytes(r.get('rs_win'))} | "
              f"{fmt_bytes(r.get('asm_linux'))} | {fmt_bytes(r.get('rs_linux'))} |")
    print(f"\nEingebettete Dateien (include_bytes!): {len(res['assets'])}, {fmt_kb(sum(res['assets'].values()))} KB")


# ====================================================================== speed: process runner
# Linux jobs run through one wsl.exe call per batch: this harness (python3 in WSL) copies the
# binary (and the job's input files) to /tmp/r13m and times every repetition there.
WSL_HARNESS = r'''
import hashlib, json, os, shutil, subprocess, sys, time
TMP = "/tmp/r13m"
def stage(src, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    s = os.stat(src)
    if not os.path.exists(dst) or os.stat(dst).st_size != s.st_size or os.stat(dst).st_mtime < s.st_mtime:
        shutil.copy2(src, dst)
    return dst
jobs = json.load(sys.stdin)
res = []
for j in jobs:
    exe = j["exe"]
    if j.get("stage_exe", True):
        exe = stage(exe, os.path.join(TMP, "bin", j["impl"], os.path.basename(exe)))
        os.chmod(exe, 0o755)
    work = os.path.join(TMP, "work", j.get("work", "default"))
    os.makedirs(work, exist_ok=True)
    for src, name in j.get("stage", []):
        stage(src, os.path.join(work, name))
    cwd = work if j.get("cwd") == "tmp" else j["cwd"]
    env = dict(os.environ)
    env.update(j.get("env", {}))
    times, p = [], None
    for k in range(j["reps"]):
        t0 = time.perf_counter()
        try:
            p = subprocess.run([exe] + j["args"], cwd=cwd, capture_output=True, timeout=j.get("timeout", 1800), env=env)
        except subprocess.TimeoutExpired:
            p = None
            times.append(None)
            break
        times.append(time.perf_counter() - t0)
    r = {"rc": p.returncode if p else -999, "times": times,
         "out": p.stdout.decode("utf-8", "replace")[-j.get("keep_out", 6000):] if p else "",
         "err": p.stderr.decode("utf-8", "replace")[-3000:] if p else "timeout"}
    h = j.get("hash")
    if h:
        f = os.path.join(cwd, h)
        r["hash"] = hashlib.sha1(open(f, "rb").read()).hexdigest() if os.path.exists(f) else None
        r["size"] = os.path.getsize(f) if os.path.exists(f) else None
    res.append(r)
json.dump(res, sys.stdout)
'''


def run_jobs(target, jobs):
    """jobs: dicts {impl, exe (Windows path), args, reps, cwd: ROOT | "tmp", work, stage:
    [(file, name)], hash: output file name, env, timeout} -> results {rc, times, out, err, hash}"""
    if not jobs:
        return []
    if target == "linux":
        hp = M / "wslrun.py"
        hp.write_text(WSL_HARNESS, encoding="utf-8")
        wj = []
        for j in jobs:
            k = dict(j)
            k["exe"] = wsl_path(j["exe"])
            k["cwd"] = "tmp" if j.get("cwd") == "tmp" else wsl_path(j.get("cwd", ROOT))
            k["stage"] = [(wsl_path(f), n) for f, n in j.get("stage", [])]
            wj.append(k)
        r = subprocess.run(WSL + ["-e", "python3", wsl_path(hp)], input=json.dumps(wj), capture_output=True,
                           text=True, encoding="utf-8", errors="replace")
        try:
            return json.loads(r.stdout)
        except ValueError:
            raise SystemExit(f"WSL harness failed: {r.stdout[-2000:]} {r.stderr[-2000:]}")
    res = []
    for j in jobs:
        work = M / "work" / j.get("work", "default")
        work.mkdir(parents=True, exist_ok=True)
        for f, n in j.get("stage", []):
            dst = work / n
            if not dst.exists() or dst.stat().st_size != Path(f).stat().st_size:
                shutil.copy2(f, dst)
        cwd = work if j.get("cwd") == "tmp" else Path(j.get("cwd", ROOT))
        env = dict(os.environ)
        env.update(j.get("env", {}))
        times, p = [], None
        for k in range(j["reps"]):
            t0 = time.perf_counter()
            try:
                p = subprocess.run([str(j["exe"])] + j["args"], cwd=cwd, capture_output=True,
                                   timeout=j.get("timeout", 1800), env=env)
            except subprocess.TimeoutExpired:
                p = None
                times.append(None)
                break
            times.append(time.perf_counter() - t0)
        r = {"rc": p.returncode if p else -999, "times": times,
             "out": p.stdout.decode("utf-8", "replace")[-j.get("keep_out", 6000):] if p else "",
             "err": p.stderr.decode("utf-8", "replace")[-3000:] if p else "timeout"}
        if j.get("hash"):
            f = cwd / j["hash"]
            r["hash"] = hashlib.sha1(f.read_bytes()).hexdigest() if f.exists() else None
            r["size"] = f.stat().st_size if f.exists() else None
        res.append(r)
    return res


def best(times):
    t = [x for x in times if x is not None]
    return min(t) if t else None


IMPL_DIRS = {"asm": M / "asm", "rs": M / "rel", "dev": M / "dev"}


def impl_exe(impl, name, target):
    return IMPL_DIRS[impl] / target / exe_name(name, target)


# ---------------------------------------------------------------- start-up, frame times
GUI_APPS = ["template", "gallery", "Kritzel", "PaintCraft"]
BENCH_RE = re.compile(r"bench: frame build (\d+) us, full render (\d+) us, unchanged frame (\d+) us \((\d+)x(\d+) px\)")


def speed_startup(targets, impls, reps):
    """start-up + first frame: a script that only quits; --shot (start-up, frame, PNG);
    `bench 300` (frame build, full render, unchanged frame) at the default size and 1920x1080"""
    inp = M / "inputs"
    inp.mkdir(parents=True, exist_ok=True)
    (inp / "quit.uis").write_bytes(b"quit\n")
    (inp / "frame1.uis").write_bytes(b"wait 16\nquit\n")
    (inp / "bench.uis").write_bytes(b"bench 300\nquit\n")
    out = {}
    for t in targets:
        jobs, keys = [], []
        for name in GUI_APPS:
            for impl in impls:
                exe = impl_exe(impl, name, t)
                if not exe.exists():
                    continue
                st = [(inp / "quit.uis", "quit.uis"), (inp / "frame1.uis", "frame1.uis"), (inp / "bench.uis", "bench.uis")]
                base = dict(impl=impl, exe=exe, cwd="tmp", work="startup_" + impl, stage=st)
                jobs.append(dict(base, args=["--script", "quit.uis"], reps=reps))
                keys.append((name, impl, "start"))
                jobs.append(dict(base, args=["--script", "frame1.uis"], reps=reps))
                keys.append((name, impl, "frame1"))
                jobs.append(dict(base, args=["--shot", f"shot_{name}.png"], reps=reps))
                keys.append((name, impl, "shot"))
                for size in (None, "1920x1080"):
                    jobs.append(dict(base, args=(["--size", size] if size else []) + ["--script", "bench.uis"], reps=3))
                    keys.append((name, impl, "bench" + ("_" + size if size else "")))
        load = load_sample()
        res = run_jobs(t, jobs)
        for (name, impl, what), r in zip(keys, res):
            e = out.setdefault(t, {}).setdefault(name, {}).setdefault(impl, {})
            if what.startswith("bench"):
                vals = [tuple(map(int, m.groups())) for m in BENCH_RE.finditer(r["out"])]
                e[what] = list(map(min, zip(*vals))) if vals else None
                if not vals:
                    e[what + "_err"] = (r["out"] + r["err"])[-500:]
            else:
                e[what] = best(r["times"])
                if r["rc"] != 0:
                    e[what + "_rc"] = r["rc"]
        out.setdefault(t, {})["_load"] = load
    return out


# ---------------------------------------------------------------- the .uis scripts
def speed_scripts(targets, impls, reps, apps=None):
    """every apps/NAME/tests/*.uis (as build_rs.py test runs them): wall time, best of reps"""
    from build_rs import plugin_test_dir, script_args
    out = {}
    for t in targets:
        jobs, keys = [], []
        for impl in impls:
            plugin_test_dir(IMPL_DIRS[impl], t, REF)          # PaintCraft's --plugins=@ptest
        for name in sorted(APPS_RS, key=str.lower):
            if apps and name not in apps:
                continue
            for uis in sorted((ROOT / "apps" / name / "tests").glob("*.uis")):
                text = uis.read_text(encoding="utf-8")
                for d in {ln.split()[1] for ln in text.splitlines() if ln.startswith(("shot ", "dump ")) and len(ln.split()) > 1}:
                    (ROOT / d).parent.mkdir(parents=True, exist_ok=True)
                for impl in impls:
                    exe = impl_exe(impl, name, t)
                    if not exe.exists():
                        continue
                    args = script_args(uis) + ["--script", uis.relative_to(ROOT).as_posix()]
                    # @ptest is looked up next to the exe: run the staged copy with the plugins
                    # of its own build folder -> do not stage (Linux reads it over /mnt/c)
                    stage_exe = not any("@ptest" in x for x in args)
                    jobs.append(dict(impl=impl, exe=exe, args=args, reps=reps, cwd=ROOT, stage_exe=stage_exe))
                    keys.append((name, uis.name, impl))
        load = load_sample()
        res = run_jobs(t, jobs)
        for (name, uis, impl), r in zip(keys, res):
            e = out.setdefault(t, {}).setdefault(name, {}).setdefault(uis, {})
            e[impl] = best(r["times"])
            if r["rc"] != 0:
                e[impl + "_rc"] = r["rc"]
        out.setdefault(t, {})["_load"] = load
    return out


# ---------------------------------------------------------------- codecs (12 MP)
CODEC_FMTS = ["png", "jpg", "bmp", "gif", "tif", "tga", "dds", "webp", "avif", "heic", "jxl", "jxr", "pdn"]


def make_photo(w, h, seed=1):
    """a natural-looking 24-bit test image (smooth regions, edges, texture, noise) -> BMP bytes"""
    import numpy as np
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    img = np.zeros((h, w, 3), np.float32)
    for c in range(3):
        img[..., c] = 128 + 60 * np.sin(x / (w / (2 + c)) + c) * np.cos(y / (h / (3 + c)))
    # low-frequency noise (upsampled), shapes with hard edges, fine texture
    small = rng.normal(0, 30, (h // 50 + 2, w // 50 + 2, 3)).astype(np.float32)
    iy, ix = (y / 50).astype(int), (x / 50).astype(int)
    img += small[iy, ix] * 0.6
    for k in range(40):
        cx, cy, r = rng.integers(0, w), rng.integers(0, h), rng.integers(20, h // 6)
        col = rng.integers(0, 256, 3)
        m = (x - cx) ** 2 + (y - cy) ** 2 < r * r
        img[m] = img[m] * 0.3 + col * 0.7
    img += rng.normal(0, 6, (h, w, 3)).astype(np.float32)
    img = np.clip(img, 0, 255).astype(np.uint8)
    row = (w * 3 + 3) & ~3
    pix = np.zeros((h, row), np.uint8)
    pix[:, :w * 3] = img[::-1, :, ::-1].reshape(h, w * 3)
    hdr = struct.pack("<2sIHHI", b"BM", 54 + row * h, 0, 0, 54) + struct.pack("<IiiHHIIiiII", 40, w, h, 1, 24, 0, row * h, 2835, 2835, 0, 0)
    return hdr + pix.tobytes()


def codec_inputs(size):
    """build/r13-measure/inputs/photo_<W>x<H>.bmp and one file per format, written by the asm
    codectest (--convert, default options) -> {fmt: path}"""
    w, h = size
    d = M / "inputs"
    d.mkdir(parents=True, exist_ok=True)
    bmp = d / f"photo_{w}x{h}.bmp"
    if not bmp.exists():
        bmp.write_bytes(make_photo(w, h))
    files = {"bmp": bmp}
    exe = impl_exe("asm", "codectest", "win")
    for f in CODEC_FMTS:
        if f == "bmp":
            continue
        p = d / f"photo_{w}x{h}.{f}"
        if not p.exists():
            r = subprocess.run([str(exe), "--convert-ex", str(bmp), "--to", str(p)], capture_output=True, text=True)
            if r.returncode or not p.exists():
                print(f"note: no {f} input ({r.stdout.strip()} {r.stderr.strip()})")
                continue
        files[f] = p
    return files


def speed_codec(targets, impls, reps, size=(4000, 3000), fmts=None):
    """decode (FILE.fmt -> .bmp) and encode (.bmp -> FILE.fmt) with codectest --convert-ex (img_load_ex / img_save_opt, default options); the
    output files of asm and Rust are compared (sha1)"""
    files = codec_inputs(size)
    w, h = size
    out = {}
    for t in targets:
        jobs, keys = [], []
        for f, p in files.items():
            if fmts and f not in fmts:
                continue
            for impl in impls:
                exe = impl_exe(impl, "codectest", t)
                if not exe.exists():
                    continue
                base = dict(impl=impl, exe=exe, reps=reps, cwd="tmp", work=f"codec_{impl}", stage=[(p, p.name)])
                if f != "bmp":
                    jobs.append(dict(base, args=["--convert-ex", p.name, "--to", f"dec_{f}.bmp"], hash=f"dec_{f}.bmp"))
                    keys.append((f, "dec", impl))
                bmp = files["bmp"]
                jobs.append(dict(base, stage=[(bmp, bmp.name)], args=["--convert-ex", bmp.name, "--to", f"enc.{f}"],
                                 hash=f"enc.{f}"))
                keys.append((f, "enc", impl))
        load = load_sample()
        # rounds: every job once per round (asm and Rust interleaved), so that a stall of the
        # machine (WSL in particular) does not hit all repetitions of one job
        res = None
        for k in range(reps):
            rr = run_jobs(t, [dict(j, reps=1) for j in jobs])
            if res is None:
                res = rr
            else:
                for x, y in zip(res, rr):
                    x["times"] += y["times"]
        for (f, what, impl), r in zip(keys, res):
            e = out.setdefault(t, {}).setdefault(f, {}).setdefault(what, {})
            e[impl] = best(r["times"])
            e[impl + "_hash"] = r.get("hash")
            e[impl + "_size"] = r.get("size")
            if r["rc"] != 0:
                e[impl + "_rc"] = r["rc"]
                e[impl + "_err"] = (r["out"] + r["err"])[-300:]
        out.setdefault(t, {})["_load"] = load
        out[t]["_size"] = f"{w}x{h}"
    return out


# ---------------------------------------------------------------- effects (efftest, 12 MP)
FX_RE = re.compile(r"^ok (\S+) .*prepare_us=(\d+) render_us=(\d+)")


def speed_fx(targets, impls, size=(4000, 3000), bench=2, only=None):
    """every registered effect with its default properties on the 12 MP photo, the pool's
    threads, tiles of 256: efftest --bench (fastest prepare / render of N)"""
    files = codec_inputs(size)
    lst = subprocess.run([str(impl_exe("asm", "efftest", "win")), "--shot", "-", "--list"], capture_output=True,
                         text=True).stdout
    names = [ln.split()[1] for ln in lst.splitlines() if ln.startswith("effect ")]
    if only:
        names = [n for n in names if n in only]
    bmp = files["bmp"]
    ops = M / "inputs" / "fx_ops.txt"
    ops.write_text("".join(f"--fx {n} --in {bmp.name} --bench {bench}\n" for n in names), encoding="utf-8")
    out = {}
    for t in targets:
        have = [impl for impl in impls if impl_exe(impl, "efftest", t).exists()]
        jobs = [dict(impl=impl, exe=impl_exe(impl, "efftest", t), reps=1, cwd="tmp", work=f"fx_{impl}",
                     stage=[(bmp, bmp.name), (ops, ops.name)], args=["--shot", "-", "--ops", ops.name], timeout=7200,
                     keep_out=1 << 20)
                for impl in have]
        load = load_sample()
        res = run_jobs(t, jobs)
        for impl, r in zip(have, res):
            got = {m.group(1): (int(m.group(2)), int(m.group(3))) for m in
                   (FX_RE.match(ln) for ln in r["out"].splitlines()) if m}
            out.setdefault(t, {})[impl] = got
        out[t]["_load"] = load
        out[t]["_names"] = names
    return out


# ---------------------------------------------------------------- the check tools' benchmarks
# (tool, arguments): each runs once against the asm build and once against the Rust release
# build (--out <dir> --no-build); the bench lines of both are paired by their text skeleton
TOOL_BENCH = [
    ("codec_check", ["--only", "bench"]),
    ("jpeg_check", ["--only", "bench"]),
    ("webp_check", ["--only", "perf"]),
    ("webp_enc_check", ["--only", "perf"]),
    ("avif_check", ["--only", "perf"]),
    ("heic_check", ["--only", "perf"]),
    ("jxr_check", ["--only", "perf"]),
    ("tiff_check", ["--only", "perf"]),
    ("gif_check", ["--only", "bench"]),
    ("dds_check", ["--only", "perf"]),
    ("dds_write_check", ["--only", "perf"]),
    ("bc_check", ["--only", "perf"]),
    ("pdn_check", ["--only", "perf"]),
    ("zlib_state_check", ["--only", "bench"]),
    ("blend_check", ["--only", "bench"]),
    ("flood_check", []),
    ("gradient_check", ["--bench"]),
    ("resample_check", ["--bench"]),
    ("xform_check", ["--bench"]),
    ("dash_check", ["--bench"]),
    ("fx_check_c", ["--bench", "--only", "none"]),
]
NUM_RE = re.compile(r"(?<![A-Za-z_])-?\d+(?:\.\d+)?")


def skeleton(line):
    return NUM_RE.sub("#", line)


def speed_tools(targets, tools=None):
    """run each tool's benchmark on asm and Rust release; store the raw output"""
    out = {}
    for t in targets:
        for tool_name, args in TOOL_BENCH:
            if tools and tool_name not in tools:
                continue
            for impl in ("asm", "rs"):
                d = IMPL_DIRS[impl].relative_to(ROOT).as_posix()
                # the rstest tools have no --no-build (they never build)
                nb = [] if tool_name in ("gradient_check", "resample_check", "xform_check", "dash_check") else ["--no-build"]
                cmd = [sys.executable, str(ROOT / "tools" / f"{tool_name}.py"), "--target", t, "--out", d] + nb + args
                t0 = time.perf_counter()
                r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace",
                                   timeout=7200)
                dt = time.perf_counter() - t0
                text = r.stdout + r.stderr
                (M / "tools").mkdir(exist_ok=True)
                (M / "tools" / f"{tool_name}_{t}_{impl}.txt").write_text(text, encoding="utf-8")
                out.setdefault(t, {}).setdefault(tool_name, {})[impl] = {"rc": r.returncode, "secs": dt,
                                                                          "lines": text.splitlines()[-200:]}
                print(f"[{t}] {tool_name} {impl}: rc {r.returncode}, {dt:.0f} s", flush=True)
    return out


def pair_bench_lines(a_lines, b_lines):
    """lines of two runs with the same skeleton (numbers replaced) and different numbers ->
    [(skeleton, numbers asm, numbers rs)]"""
    pairs = []
    bmap = {}
    for ln in b_lines:
        bmap.setdefault(skeleton(ln), []).append(ln)
    for ln in a_lines:
        sk = skeleton(ln)
        if sk in bmap and bmap[sk]:
            other = bmap[sk].pop(0)
            na, nb = NUM_RE.findall(ln), NUM_RE.findall(other)
            if na != nb and any(c in sk for c in ("ms", "MB/s", "Mpx", "us", "µs", " s ", "s)", "x ")):
                pairs.append((sk.strip(), na, nb))
    return pairs


def cmd_speed(a):
    res = load("speed")
    targets = a.targets.split(",")
    impls = a.impls.split(",")
    parts = a.parts.split(",")
    if "startup" in parts:
        res.setdefault("startup", {}).update(speed_startup(targets, impls, a.runs))
        save("speed", res)
    if "codec" in parts:
        res.setdefault("codec", {}).update(speed_codec(targets, impls, max(2, a.runs // 3), fmts=a.fmts.split(",") if a.fmts else None))
        save("speed", res)
    if "fx" in parts:
        res.setdefault("fx", {}).update(speed_fx(targets, impls))
        save("speed", res)
    if "scripts" in parts:
        res.setdefault("scripts", {}).update(speed_scripts(targets, impls, max(2, a.runs // 3), apps=a.apps.split(",") if a.apps else None))
        save("speed", res)
    if "tools" in parts:
        tr = speed_tools(targets, a.tools.split(",") if a.tools else None)
        cur = res.setdefault("tools", {})
        for t, d in tr.items():
            cur.setdefault(t, {}).update(d)
        save("speed", res)
    print_speed(res)
    return 0


def ratio(a, b):
    return f"{b / a:.2f}" if a and b else "–"


def print_speed(res):
    st = res.get("startup", {})
    if st:
        print("\n| App | Target | Start (Skript `quit`) asm / Rust | erster Frame (`wait 16`) asm / Rust | `--shot` asm / Rust | "
              "Frame aufbauen asm / Rust (µs) | volles Rendern asm / Rust (µs) | unveränderter Frame asm / Rust (µs) | Größe |")
        print("|---|---|---|---|---|---|---|---|---|")
        for t, apps in st.items():
            for name, e in apps.items():
                if name.startswith("_"):
                    continue
                x, y = e.get("asm", {}), e.get("rs", {})
                for bk in ("bench", "bench_1920x1080"):
                    bx, by = x.get(bk), y.get(bk)
                    if not bx or not by:
                        continue
                    first = bk == "bench"
                    def ms2(k):
                        return f"{x[k] * 1000:.1f} / {y[k] * 1000:.1f} ms" if first and x.get(k) and y.get(k) else ""
                    print(f"| {name} | {t} | {ms2('start')} | {ms2('frame1')} | {ms2('shot')} | {bx[0]} / {by[0]} | "
                          f"{bx[1]} / {by[1]} | {bx[2]} / {by[2]} | {bx[3]}×{bx[4]} |")
    sc = res.get("scripts", {})
    if sc:
        print("\n| App | Target | Skripte | asm gesamt | Rust gesamt | Rust / asm | langsamstes Skript (Rust / asm) |")
        print("|---|---|---|---|---|---|---|")
        for t, apps in sc.items():
            for name, scripts in apps.items():
                if name.startswith("_"):
                    continue
                sa = sum(e.get("asm") or 0 for e in scripts.values())
                sr = sum(e.get("rs") or 0 for e in scripts.values())
                worst = max(scripts.items(), key=lambda kv: (kv[1].get("rs") or 0) / max(kv[1].get("asm") or 1e-9, 1e-9))
                fails = [f"{u}:{k}" for u, e in scripts.items() for k in e if k.endswith("_rc")]
                print(f"| {name} | {t} | {len(scripts)} | {sa:.2f} s | {sr:.2f} s | {ratio(sa, sr)} | {worst[0]} "
                      f"{(worst[1].get('rs') or 0) * 1000:.0f} / {(worst[1].get('asm') or 0) * 1000:.0f} ms |"
                      + (f" (Exit≠0: {', '.join(fails)})" if fails else ""))
    cd = res.get("codec", {})
    if cd:
        print("\n(netto: minus BMP -> BMP desselben Programms = Prozessstart + BMP lesen / schreiben)")
        print("\n| Format | Target | Dekodieren asm / Rust (ms) | Rust / asm | Kodieren asm / Rust (ms) | Rust / asm | Ausgabe gleich | Dateigröße asm / Rust |")
        print("|---|---|---|---|---|---|---|---|")
        for t, fm in cd.items():
            base = fm.get("bmp", {}).get("enc", {})
            for f, e in fm.items():
                if f.startswith("_"):
                    continue
                d0, en0 = e.get("dec", {}), e.get("enc", {})
                net = lambda x: {k: (max(0.0005, v - base[k]) if f != "bmp" and v and base.get(k) else v)  # noqa: E731
                                 for k, v in x.items() if k in ("asm", "rs")} | {k: v for k, v in x.items() if k not in ("asm", "rs")}
                d, en = net(d0), net(en0)
                ms = lambda v: f"{v * 1000:.0f}" if v else "–"  # noqa: E731
                same = []
                for k, x in (("dec", d), ("enc", en)):
                    if x.get("asm_hash") and x.get("rs_hash"):
                        same.append(f"{k} {'ja' if x['asm_hash'] == x['rs_hash'] else 'nein'}")
                print(f"| {f} | {t} | {ms(d.get('asm'))} / {ms(d.get('rs'))} | {ratio(d.get('asm'), d.get('rs'))} | "
                      f"{ms(en.get('asm'))} / {ms(en.get('rs'))} | {ratio(en.get('asm'), en.get('rs'))} | {', '.join(same)} | "
                      f"{fmt_kb(en.get('asm_size'))} / {fmt_kb(en.get('rs_size'))} KB |")
    fx = res.get("fx", {})
    if fx:
        print("\n| Target | Effekte | asm gesamt (Rendern) | Rust gesamt | Rust / asm | Median je Effekt | schneller / gleich / langsamer (±10 %) |")
        print("|---|---|---|---|---|---|---|")
        for t, e in fx.items():
            a, b = e.get("asm", {}), e.get("rs", {})
            common = [n for n in e.get("_names", []) if n in a and n in b]
            if not common:
                continue
            sa = sum(a[n][1] for n in common)
            sb = sum(b[n][1] for n in common)
            rat = sorted(b[n][1] / max(a[n][1], 1) for n in common)
            fast = sum(1 for r in rat if r < 0.9)
            slow = sum(1 for r in rat if r > 1.1)
            print(f"| {t} | {len(common)} | {sa / 1000:.0f} ms | {sb / 1000:.0f} ms | {sb / sa:.2f} | "
                  f"{statistics.median(rat):.2f} | {fast} / {len(rat) - fast - slow} / {slow} |")
            worst = sorted(common, key=lambda n: -b[n][1] / max(a[n][1], 1))[:6]
            bestn = sorted(common, key=lambda n: b[n][1] / max(a[n][1], 1))[:4]
            print("   langsamste (Rust/asm): " + ", ".join(f"{n} {b[n][1] / max(a[n][1], 1):.2f} ({b[n][1] // 1000}/{a[n][1] // 1000} ms)" for n in worst))
            print("   schnellste (Rust/asm): " + ", ".join(f"{n} {b[n][1] / max(a[n][1], 1):.2f} ({b[n][1] // 1000}/{a[n][1] // 1000} ms)" for n in bestn))
    tl = res.get("tools", {})
    for t, tools in tl.items():
        for name, e in tools.items():
            if "asm" not in e or "rs" not in e:
                continue
            pairs = pair_bench_lines(e["asm"]["lines"], e["rs"]["lines"])
            print(f"\n[{t}] {name}: asm rc {e['asm']['rc']}, Rust rc {e['rs']['rc']}")
            for sk, na, nb in pairs[:40]:
                print(f"   {sk[:110]}\n      asm {' '.join(na[:12])}\n      rs  {' '.join(nb[:12])}")


# ====================================================================== size experiment
# opt: library crates that are not compute crates (rt, plat, base, ui, styles) | copt: the compute
# crates of build_rs.compute_crate (gfx, text, simd, fx, img_*) | aopt: the app (bin) crate, which
# also sets the pipeline of the LTO step | lto: fat | thin | off | link: extra linker arguments
# seed: a variant with the same library flags whose rlibs are copied first (only the apps are
# linked again)
VARIANTS = {
    "s": dict(opt="s", copt="s", aopt="s", lto="fat"),
    "z": dict(opt="z", copt="z", aopt="z", lto="fat"),
    # opt-level z everywhere fails build_rs.py's check (slice helpers over __m128i / __m256i stay
    # out of line in img_jxl): the same without the check, to see what z costs and saves
    "z_nochk": dict(opt="z", copt="z", aopt="z", lto="fat", nocheck=True, extra=True),
    "2": dict(opt="2", copt="2", aopt="2", lto="fat"),
    "s_c3": dict(opt="s", copt="3", aopt="s", lto="fat"),
    "z_c3": dict(opt="z", copt="3", aopt="z", lto="fat"),
    "z_c3_a3": dict(opt="z", copt="3", aopt="3", lto="fat", seed="z_c3"),
    "z_cs": dict(opt="z", copt="s", aopt="z", lto="fat", extra=True),
    "z_c2": dict(opt="z", copt="2", aopt="z", lto="fat", extra=True),
    # second round (after the speed checks: an app at z / s slows down rendering, because the
    # app crate also sets the LTO pipeline for every crate)
    "2_nolto": dict(opt="2", copt="2", aopt="2", lto="off", seed="2", extra=True),
    "z_c2_a2": dict(opt="z", copt="2", aopt="2", lto="fat", extra=True),
    "z_c2_a2_nolto": dict(opt="z", copt="2", aopt="2", lto="off", seed="z_c2_a2", extra=True),
    "z_c3_nolto": dict(opt="z", copt="3", aopt="z", lto="off", seed="z_c3"),
    "thin": dict(opt="3", copt="3", aopt="3", lto="thin", seed="rel"),
    "nolto": dict(opt="3", copt="3", aopt="3", lto="off", seed="rel"),
    "cgu16": dict(opt="3", copt="3", aopt="3", lto="fat", cgu=16, extra=True),
    "icf": dict(opt="3", copt="3", aopt="3", lto="fat", link_win=["/OPT:REF,ICF"], link_linux=["--icf=all"], seed="rel",
                link_only=True),
    "icf_safe": dict(opt="3", copt="3", aopt="3", lto="fat", link_linux=["--icf=safe"], seed="rel", link_only=True,
                     targets=["linux"], extra=True),
    "fixed": dict(opt="3", copt="3", aopt="3", lto="fat", link_win=["/FIXED", "/DYNAMICBASE:NO"], seed="rel",
                  link_only=True, targets=["win"]),
    # Linux only: no .eh_frame (the crash report's call chain needs it -- measurement only)
    "nouwt": dict(opt="3", copt="3", aopt="3", lto="fat", rustc_linux=["-C", "force-unwind-tables=no"], targets=["linux"],
                  link_only=True),
}


def seed_variant(name, seed, targets):
    """copy the rlibs and library stamps of the seed variant (same library flags) -> only the
    apps are built again (their stamps are not copied)"""
    for t in targets:
        src, dst = variant_dir(seed) / t, variant_dir(name) / t
        if not src.exists():
            continue
        (dst / ".stamp").mkdir(parents=True, exist_ok=True)
        for f in src.glob("*.rlib"):
            if not (dst / f.name).exists():
                shutil.copy2(f, dst / f.name)
        for f in (src / ".stamp").iterdir():
            if not f.name.startswith("app_"):
                shutil.copy2(f, dst / ".stamp" / f.name)
        for f in (dst / ".stamp").glob("app_*"):
            f.unlink()
XP_APPS = ["gallery", "Kritzel", "PaintCraft", "codectest", "efftest"]


def variant_dir(name):
    return M / "rel" if name == "rel" else M / "var" / name


def build_variant(name, v, targets, apps):
    """build the apps with the settings of variant v (build_rs.Builder with other flags) -> seconds"""
    import build_rs as br

    class VB(br.Builder):
        def flags(self, crate=""):
            f = ["--edition", "2024", "--target", br.TRIPLE[self.target], "-C", "panic=abort", "-C", "debuginfo=0",
                 "-D", "warnings", "-L", f"dependency={self.dir}", f"--remap-path-prefix={br.ROOT}{os.sep}="]
            opt = v["aopt"] if crate == "" else v["copt"] if br.compute_crate(crate) else v["opt"]
            return f + ["-C", f"opt-level={opt}", "-C", f"codegen-units={v.get('cgu', 1)}"] + v.get("rustc", []) + \
                v.get("rustc_" + self.target, [])

    orig = br.run_cmd

    def run_cmd(cmd, quiet=False):
        cmd = [str(c) for c in cmd]
        if "bin" in cmd and "--crate-type" in cmd:
            k = cmd.index("lto") if "lto" in cmd else -1
            if k > 0 and cmd[k - 1] == "-C":
                cmd[k] = {"fat": "lto=fat", "thin": "lto=thin", "off": "lto=off"}[v["lto"]]
            tgt = "win" if "x86_64-pc-windows-msvc" in cmd else "linux"
            for la in v.get("link_" + tgt, []):
                cmd[-3:-3] = ["-C", f"link-arg={la}"]
        return orig(cmd, quiet)

    br.run_cmd = run_cmd
    orig_check = br.check_intrinsics
    if v.get("nocheck"):
        br.check_intrinsics = lambda *x: None
    t0 = time.perf_counter()
    try:
        for t in targets:
            b = VB(t, variant_dir(name), True, 8)
            for app in (apps[t] if isinstance(apps, dict) else apps):
                b.build_app(app, br.app_style(app))
    finally:
        br.run_cmd = orig
        br.check_intrinsics = orig_check
    return time.perf_counter() - t0


XP_CODEC = ["jpg", "png", "webp", "avif", "heic", "jxl"]
XP_FX = ["gaussian_blur", "median", "surface_blur", "oil_painting", "hue_saturation", "levels", "curves",
         "motion_blur", "zoom_blur", "twist", "clouds", "quantize", "crystalize", "unfocus", "bokeh_blur"]


def cmd_sizexp(a):
    res = load("sizexp")
    targets = a.targets.split(",")
    names = a.variants.split(",") if a.variants else ["rel"] + [n for n, v in VARIANTS.items() if not v.get("extra")]
    gui = ["gallery", "Kritzel", "PaintCraft"]
    stg = a.speed_targets.split(",")
    for name in names:
        r = res.setdefault(name, {})
        v = VARIANTS.get(name, {})
        # the compute apps (codectest, efftest) only where speed is measured and only when the
        # variant changes compiled code (not for linker-only variants)
        apps = {t: (a.apps.split(",") if a.apps else
                    gui + (["codectest", "efftest"] if t in stg and not v.get("link_only") else []))
                for t in targets}
        if name != "rel" and not a.no_build:
            try:
                if VARIANTS[name].get("seed"):
                    seed_variant(name, VARIANTS[name]["seed"], VARIANTS[name].get("targets", targets))
                r["build_s"] = build_variant(name, VARIANTS[name], VARIANTS[name].get("targets", targets), apps)
                r["seeded"] = VARIANTS[name].get("seed")
                r.pop("error", None)
            except Exception as e:                     # e.g. intrinsics out of line at -Os / -Oz
                r["error"] = str(e)[:2000]
                print(f"variant {name}: build failed: {str(e)[:500]}")
                save("sizexp", res)
                continue
        r["sizes"] = {}
        for t in targets:
            for app in XP_APPS:
                f = variant_dir(name) / t / exe_name(app, t)
                if f.exists():
                    r["sizes"][f"{app}_{t}"] = f.stat().st_size
                    r.setdefault("groups", {})[f"{app}_{t}"] = section_groups(f, t)
        save("sizexp", res)
        print(f"variant {name}: " + ", ".join(f"{k} {fmt_kb(v)} KB" for k, v in r["sizes"].items()), flush=True)
    if not a.no_speed:
        speed_variants(res, names, a.speed_targets.split(","), a.runs)
        save("sizexp", res)
    print_sizexp(res)
    return 0


def speed_variants(res, names, targets, runs):
    """the speed checks of each variant: start-up, frame times, the Kritzel / PaintCraft scripts,
    codec conversions (12 MP), effects; with asm and the default release as reference"""
    impls = ["asm"]
    for name in ["rel"] + [n for n in names if n != "rel"]:
        if "error" in res.get(name, {}):
            continue
        IMPL_DIRS["v_" + name] = variant_dir(name)
        impls.append("v_" + name)
    impls = list(dict.fromkeys(impls))
    sp = {}
    sp["startup"] = speed_startup(targets, impls, runs)
    sp["scripts"] = speed_scripts(targets, impls, 2, apps=["Kritzel", "PaintCraft"])
    sp["codec"] = speed_codec(targets, impls, 2, fmts=XP_CODEC)
    sp["fx"] = speed_fx(targets, impls, only=XP_FX)
    res["_speed"] = sp


def print_sizexp(res):
    names = [n for n in res if not n.startswith("_")]
    keys = sorted({k for n in names for k in res[n].get("sizes", {})}, key=lambda k: (k.split("_")[-1], XP_APPS.index(k.rsplit("_", 1)[0])))
    print("\n| Variante | Einstellungen | " + " | ".join(keys) + " | Build |")
    print("|---|---|" + "---|" * len(keys) + "---|")
    base = res.get("rel", {}).get("sizes", {})
    for n in names:
        r = res[n]
        v = VARIANTS.get(n, dict(opt="3", copt="3", aopt="3", lto="fat"))
        desc = f"opt {v['opt']}/{v['copt']}/{v['aopt']}, LTO {v['lto']}" + (f", cgu {v['cgu']}" if v.get("cgu") else "") + \
            ("".join(f", {x}" for x in v.get("link_win", []) + v.get("link_linux", [])))
        if "error" in r:
            print(f"| {n} | {desc} | Build-Fehler: {r['error'][:150].replace('|', '/')} |")
            continue
        cells = []
        for k in keys:
            s = r.get("sizes", {}).get(k)
            b = base.get(k)
            cells.append(f"{fmt_kb(s)}" + (f" ({(s / b - 1) * 100:+.0f} %)" if s and b and n != "rel" else ""))
        print(f"| {n} | {desc} | " + " | ".join(cells) + f" | {r.get('build_s', 0):.0f} s |")
    sp = res.get("_speed")
    if not sp:
        return
    impls = [k for k in next(iter(sp["startup"].values()))["gallery"].keys()]
    print("\n| Variante | Target | Start gallery/Kritzel/PaintCraft (ms) | Frame+Render 1080p gallery/Kritzel/PaintCraft (µs) | Skripte Kritzel + PaintCraft (s) | Codecs dekodieren+kodieren (s) | Effekte (s) |")
    print("|---|---|---|---|---|---|---|")
    for t in sp["startup"]:
        for impl in impls:
            st = sp["startup"][t]
            starts = "/".join(f"{(st[a].get(impl, {}).get('start') or 0) * 1000:.0f}" for a in ("gallery", "Kritzel", "PaintCraft"))
            frames = "/".join(f"{(st[a].get(impl, {}).get('bench_1920x1080') or [0, 0])[0]}+"
                              f"{(st[a].get(impl, {}).get('bench_1920x1080') or [0, 0])[1]}" for a in ("gallery", "Kritzel", "PaintCraft"))
            scr = sum((e.get(impl) or 0) for app in ("Kritzel", "PaintCraft") for e in sp["scripts"].get(t, {}).get(app, {}).values())
            cod = sum((e.get(w, {}).get(impl) or 0) for f, e in sp["codec"].get(t, {}).items() if not f.startswith("_")
                      for w in ("dec", "enc"))
            fxe = sp["fx"].get(t, {}).get(impl, {})
            fxs = sum(x[1] for x in fxe.values()) / 1e6 if fxe else 0
            print(f"| {impl.replace('v_', '')} | {t} | {starts} | {frames} | {scr:.1f} | {cod:.2f} | {fxs:.2f} |")


# ====================================================================== code
def rust_clean(src):
    """Rust source -> the same text with comments replaced by blanks and the contents of string /
    char literals by 'x' (line structure kept): code lines and `unsafe` tokens without false hits"""
    out = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append("".join("\n" if ch == "\n" else " " for ch in src[i:j]))
            i = j
        elif c == '"' or (c in "br" and re.match(r'(b?r#*"|b")', src[i:i + 8]) and
                          (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_"))):
            m = re.match(r'(b?r)(#*)"', src[i:i + 16])
            if m:                                    # raw string r#"..."#
                end = '"' + m.group(2)
                j = src.find(end, i + len(m.group(0)))
                j = n if j < 0 else j + len(end)
            else:
                j = i + (2 if src[i] == "b" else 1)
                while j < n and src[j] != '"':
                    j += 2 if src[j] == "\\" else 1
                j += 1
            out.append("".join("\n" if ch == "\n" else "x" for ch in src[i:j]))
            i = j
        elif c == "'":
            m = re.match(r"'(\\.[^']*|[^\\'])'", src[i:i + 12])
            if m:                                    # char literal (not a lifetime)
                out.append("x" * len(m.group(0)))
                i += len(m.group(0))
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def asm_clean(src):
    """assembly -> lines without '#' comments (outside "strings")"""
    lines = []
    for ln in src.splitlines():
        q, cut = False, len(ln)
        for k, ch in enumerate(ln):
            if ch == '"' and (k == 0 or ln[k - 1] != "\\"):
                q = not q
            elif ch == "#" and not q:
                cut = k
                break
        lines.append(ln[:cut])
    return "\n".join(lines)


def code_lines(text):
    return sum(1 for ln in text.splitlines() if ln.strip())


def is_generated(path):
    head = path.read_text(encoding="utf-8", errors="replace")[:600].lower()
    return "generated" in head and ("do not edit" in head or "generated by" in head or "gen_" in head)


def rust_unsafe(clean):
    """(unsafe blocks, lines inside them, unsafe fn, unsafe impl / trait, unsafe extern blocks,
    #[unsafe(..)] attributes)"""
    blocks = lines = 0
    for m in re.finditer(r"\bunsafe\s*\{", clean):
        depth, k = 0, m.end() - 1
        start = k
        while k < len(clean):
            if clean[k] == "{":
                depth += 1
            elif clean[k] == "}":
                depth -= 1
                if depth == 0:
                    break
            k += 1
        blocks += 1
        body = clean[start:k + 1]
        lines += max(1, sum(1 for ln in body.splitlines() if ln.strip()))
    # (string literals are x'ed out by rust_clean: extern "C" reads extern xxx)
    fns = len(re.findall(r"\bunsafe\s+(?:extern\s+x+\s+)?fn\b", clean))
    impls = len(re.findall(r"\bunsafe\s+(?:impl|trait)\b", clean))
    ext = len(re.findall(r"\bunsafe\s+extern\s+x+\s*\{", clean))
    attrs = len(re.findall(r"#\[unsafe\(", clean))
    return blocks, lines, fns, impls, ext, attrs


INC_AREA = {"av1": "codecs", "hevc": "codecs", "img": "codecs", "isobmff": "codecs", "jxl": "codecs",
            "jxr": "codecs", "yuv": "codecs", "icc": "codecs", "font": "text", "fx": "fx", "gfx2": "gfx",
            "gfx3": "gfx", "ui": "ui", "style": "ui", "keys": "ui", "icons": "ui", "thread": "rt_plat",
            "print": "rt_plat", "plugin": "plugins", "jfgui": "rt_plat"}
AREAS = ["rt_plat", "gfx", "text", "ui", "styles", "codecs", "fx", "plugins", "apps"]
AREA_NAMES = {"rt_plat": "Laufzeit + Plattform (rt, simd, plat, base / core, plat)", "gfx": "gfx", "text": "text",
              "ui": "ui", "styles": "Styles (4)", "codecs": "Codecs (img_* / img)", "fx": "Effekte (fx)",
              "plugins": "Plugins (Crate plugin + Beispiele / asm-Plugins)", "apps": "Apps"}


def asm_area(p):
    rel = p.relative_to(ROOT.parent / "legacy").as_posix().split("/")
    if rel[0] == "apps":
        return ("plugins", None) if rel[2] == "plugins" else ("apps", rel[1])
    if rel[1] == "styles":
        return "styles", None
    if rel[1] == "inc":
        stem = re.split(r"[_.]", p.name)[0]
        return INC_AREA.get(stem, "rt_plat"), None
    sub = rel[2]
    return {"core": "rt_plat", "plat": "rt_plat", "img": "codecs"}.get(sub, sub), None


def rs_area(p):
    rel = p.relative_to(RS).as_posix().split("/")
    if rel[0] == "apps":
        return "apps", rel[1]
    if rel[0] == "plugins":
        return "plugins", None
    if rel[0] == "styles":
        return "styles", None
    c = rel[1]
    if c in ("rt", "simd", "plat", "base"):
        return "rt_plat", None
    if c == "plugin":
        return "plugins", None
    if c.startswith("img_"):
        return "codecs", None
    return c, None


def rs_crate(p):
    rel = p.relative_to(RS).as_posix().split("/")
    return {"lib": "", "styles": "style_", "apps": "app ", "plugins": "plugin "}[rel[0]] + rel[1]


def cmd_code(a):
    res = {"asm": {}, "rs": {}, "asm_apps": {}, "rs_apps": {}, "asm_gen": 0, "rs_gen": 0, "unsafe": {},
           "crate_lines": {}}
    asm_files = [p for d in ("ASMLIB", "apps") for p in (ROOT.parent / "legacy" / d).rglob("*")
                 if p.suffix in (".s", ".inc") and p.is_file()]
    for p in asm_files:
        n = code_lines(asm_clean(p.read_text(encoding="utf-8", errors="replace")))
        area, app = asm_area(p)
        if is_generated(p):
            res["asm_gen"] += n
        res["asm"][area] = res["asm"].get(area, 0) + n
        if app:
            res["asm_apps"][app] = res["asm_apps"].get(app, 0) + n
    for p in owned_rust_sources():
        if "tools" in p.relative_to(RS).parts[:1]:
            continue
        raw = p.read_text(encoding="utf-8")
        clean = rust_clean(raw)
        n = code_lines(clean)
        if "SAFETY:" in raw:
            res.setdefault("safety", {})
            res["safety"][rs_crate(p)] = res["safety"].get(rs_crate(p), 0) + raw.count("SAFETY:")
        area, app = rs_area(p)
        if is_generated(p):
            res["rs_gen"] += n
        res["rs"][area] = res["rs"].get(area, 0) + n
        if app:
            res["rs_apps"][app] = res["rs_apps"].get(app, 0) + n
        crate = rs_crate(p)
        res["crate_lines"][crate] = res["crate_lines"].get(crate, 0) + n
        u = rust_unsafe(clean)
        if any(u):
            cur = res["unsafe"].setdefault(crate, [0] * 6)
            res["unsafe"][crate] = [x + y for x, y in zip(cur, u)]
    # tests
    res["uis_shared"] = len(list((ROOT / "apps").glob("*/tests/*.uis")))
    res["uis_rs"] = len(list(RS.glob("apps/*/tests/*.uis"))) + len(list(RS.glob("plugins/tests/*.uis")))
    res["uis_per_app"] = {d.parent.name: len(list(d.glob("*.uis"))) for d in (ROOT / "apps").glob("*/tests")}
    res["check_tools"] = sorted(p.name for p in (ROOT / "tools").glob("*_check*.py"))
    save("code", res)
    print_code(res)
    return 0


def print_code(res):
    print("\n| Bereich | asm | Rust | Rust / asm |")
    print("|---|---|---|---|")
    ta = tr = 0
    for ar in AREAS:
        x, y = res["asm"].get(ar, 0), res["rs"].get(ar, 0)
        ta, tr = ta + x, tr + y
        print(f"| {AREA_NAMES[ar]} | {fmt_bytes(x)} | {fmt_bytes(y)} | {y / x if x else 0:.2f} |")
    print(f"| **gesamt** | **{fmt_bytes(ta)}** | **{fmt_bytes(tr)}** | **{tr / ta:.2f}** |")
    print(f"\n(davon erzeugte Tabellen-Dateien: asm {fmt_bytes(res['asm_gen'])}, Rust {fmt_bytes(res['rs_gen'])})")
    print("\n| App | asm | Rust | Rust / asm |")
    print("|---|---|---|---|")
    for app in sorted(set(res["asm_apps"]) | set(res["rs_apps"]), key=str.lower):
        x, y = res["asm_apps"].get(app, 0), res["rs_apps"].get(app, 0)
        print(f"| {app} | {fmt_bytes(x) if x else '–'} | {fmt_bytes(y) if y else '–'} | "
              f"{f'{y / x:.2f}' if x and y else '–'} |")
    print("\n| Crate | Codezeilen | unsafe-Blöcke | Zeilen darin | unsafe fn | unsafe impl/trait | unsafe extern | #[unsafe(..)] | SAFETY-Kommentare | Anteil |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    tot = sum(res["crate_lines"].values())
    ub = ul = 0
    sf = res.get("safety", {})
    for crate, u in sorted(res["unsafe"].items(), key=lambda kv: -kv[1][1]):
        n = res["crate_lines"][crate]
        ub, ul = ub + u[0], ul + u[1]
        print(f"| {crate} | {fmt_bytes(n)} | {u[0]} | {u[1]} | {u[2]} | {u[3]} | {u[4]} | {u[5]} | {sf.get(crate, 0)} | "
              f"{u[1] / n * 100:.1f} % |")
    print(f"| **alle {len(res['crate_lines'])} Crates** | **{fmt_bytes(tot)}** | **{ub}** | **{ul}** | | | | | "
          f"{sum(sf.values())} | **{ul / tot * 100:.2f} %** |")
    four = [c for c in ("rt", "simd", "plat", "plugin")]
    fl = sum(res["crate_lines"].get(c, 0) for c in four)
    print(f"\nDie vier unsafe-Crates rt/simd/plat/plugin: {fmt_bytes(fl)} Zeilen ({fl / tot * 100:.1f} % des Codes); "
          f"außerhalb: {', '.join(c for c in res['unsafe'] if c not in four) or 'nichts'}")
    print(f"\n.uis-Skripte: {res['uis_shared']} geteilte (apps/*/tests) + {res['uis_rs']} eigene des Rust-Baus; "
          f"{len(res['check_tools'])} Prüf-Tools tools/*_check*.py")


# ====================================================================== main
def main():
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if len(sys.argv) > 1 and sys.argv[1] == "_asmbuild":
        return asm_build(sys.argv[2], sys.argv[3])
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--targets", default="win,linux")
    b.add_argument("--modes", default="asm,dev,rel")
    b.add_argument("--runs", type=int, default=1, help="clean builds per mode / target")
    b.add_argument("--inc-runs", type=int, default=2, help="incremental builds per edit")
    sub.add_parser("code")
    sub.add_parser("size")
    s = sub.add_parser("speed")
    s.add_argument("--parts", default="startup,codec,scripts,tools")
    s.add_argument("--targets", default="win,linux")
    s.add_argument("--impls", default="asm,rs")
    s.add_argument("--runs", type=int, default=6, help="start-up runs (codecs: half, scripts: a third)")
    s.add_argument("--apps", default="", help="scripts: only these apps")
    s.add_argument("--tools", default="", help="tools: only these check tools")
    s.add_argument("--fmts", default="", help="codec: only these formats")
    x = sub.add_parser("sizexp")
    x.add_argument("--variants", default="", help=f"comma list of {', '.join(VARIANTS)} (rel = the default release)")
    x.add_argument("--apps", default="")
    x.add_argument("--targets", default="win,linux")
    x.add_argument("--speed-targets", default="win")
    x.add_argument("--no-build", action="store_true")
    x.add_argument("--no-speed", action="store_true")
    x.add_argument("--runs", type=int, default=6)
    sub.add_parser("report")
    a = ap.parse_args()
    M.mkdir(parents=True, exist_ok=True)
    if a.cmd == "report":
        for name, fn in (("build", print_build), ("size", print_size), ("speed", print_speed), ("code", print_code),
                         ("sizexp", print_sizexp)):
            d = load(name)
            if d:
                print(f"\n## {name}")
                fn(d)
        return 0
    return {"build": cmd_build, "code": cmd_code, "size": cmd_size, "speed": cmd_speed,
            "sizexp": cmd_sizexp}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())

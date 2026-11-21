"""ct_check.py -- lib/crypto's secret-dependent operations take the same way for every secret.
    python tools/ct_check.py [--opt release,dev,o3,s,z] [--only T,..] [--seeds N] [--crypto DIR] [--step]
Builds tools/ct_check/probe.rs against lib/crypto (or the crate in DIR) at each optimization level
below and lets it run every operation of its list (X25519, Ed25519, P-256, EAP-PWD, the RSA / DH
exponentiation, the AEADs, AES, MACs, KDFs, Argon2id, Base64 and hex, and with this
repository's crypto the key files of netio: PEM / PKCS #8 and OpenSSH) with SEEDS different secrets under
ptrace.  An operation fails when two secrets take different branches or jump-table entries; the
report names the function and the line where the paths part (addr2line).
  release  opt-level 2, fat LTO, one codegen unit (JFLib's release, Cargo.toml)
  dev      opt-level 2, 16 codegen units, overflow checks (JFLib's development builds)
  o3       opt-level 3, 16 codegen units, no LTO (Cargo's default release: an app from crates.io)
  s, z     the size levels with fat LTO
The source says nothing about this: LLVM may turn a masked select back into a branch or a table
lookup (it did, docs: crypto's CHANGELOG), so the machine code is what is checked, for the
compiler that runs here (cargo; RUSTUP_TOOLCHAIN picks another).  The probe's target
selftest_leak branches on its secret on purpose: it has to fail, else the tracer sees nothing and
the run fails as a whole.  Not seen: memory addresses that depend on a secret without a branch,
instructions whose time depends on their operands, other processors (aarch64, wasm).
Linux on x86-64 (from Windows: runs itself again in WSL); needs cargo, binutils (addr2line, objdump)
and util-linux' setarch (runs without address randomization; without it results may vary).
Exit code 1 on any difference."""
import argparse
import concurrent.futures
import os
import platform
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROBE = ROOT / "tools" / "ct_check" / "probe.rs"
OUT = ROOT / "build" / "ct_check"

# [profile.release] of the probe per level
LEVELS = {
    "release": {"opt-level": "2", "lto": '"fat"', "codegen-units": "1"},
    "dev": {"opt-level": "2", "lto": "false", "codegen-units": "16", "overflow-checks": "true"},
    "o3": {"opt-level": "3", "lto": "false", "codegen-units": "16"},
    "s": {"opt-level": '"s"', "lto": '"fat"', "codegen-units": "1"},
    "z": {"opt-level": '"z"', "lto": '"fat"', "codegen-units": "1"},
}
SELFTEST = "selftest_leak"
# operations whose way may depend on the secret by design, from this fraction of it on
KNOWN = {
    "argon2id": (0.5, "Argon2id addresses its memory by the password after the first half of the "
                      "first pass (RFC 9106 3.4), and memcpy's path follows the addresses"),
}
# folders cut from the paths in reports (the checked crate's parent is added by main)
PREFIXES = [str(ROOT) + "/"]
SETARCH = subprocess.run(["sh", "-c", "command -v setarch"], capture_output=True).returncode == 0


def package_name(crate_dir):
    text = (crate_dir / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^\[package\][^\[]*?^name\s*=\s*"([^"]+)"', text, re.M | re.S)
    if not m:
        sys.exit(f"ct_check: no package name in {crate_dir / 'Cargo.toml'}")
    return m.group(1)


def build(level, crate_dir, label):
    """the probe at one level -> path of the binary; with this repository's crypto also the
    targets of netio and netpkt (key files), which link that crypto"""
    d = OUT / f"{label}-{level}"
    d.mkdir(parents=True, exist_ok=True)
    prof = "\n".join(f"{k} = {v}" for k, v in LEVELS[level].items())
    lib = ROOT / "lib"
    own = label == "lib"
    extra = f"""rt = {{ package = "jflib-rt", path = "{(lib / 'rt').as_posix()}", features = ["std"], optional = true }}
plat = {{ package = "jflib-plat", path = "{(lib / 'plat').as_posix()}", features = ["std"], optional = true }}
netio = {{ package = "jflib-netio", path = "{(lib / 'netio').as_posix()}", optional = true }}
netpkt = {{ package = "jflib-netpkt", path = "{(lib / 'netpkt').as_posix()}", optional = true }}

[features]
default = [{'"netio"' if own else ''}]
netio = ["dep:rt", "dep:plat", "dep:netio", "dep:netpkt"]
"""
    (d / "Cargo.toml").write_text(f"""# written by tools/ct_check.py
[package]
name = "ct-probe"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[[bin]]
name = "probe"
path = "{PROBE.as_posix()}"

[dependencies]
crypto = {{ package = "{package_name(crate_dir)}", path = "{crate_dir.as_posix()}" }}
{extra}
[profile.release]
{prof}
panic = "abort"
debug = "line-tables-only"
""", encoding="utf-8")
    env = dict(os.environ, CARGO_TARGET_DIR=str(d / "target"))
    env.pop("RUSTFLAGS", None)
    p = subprocess.run(["cargo", "build", "--release", "--quiet", "--manifest-path", str(d / "Cargo.toml")],
                       env=env, capture_output=True, text=True)
    if p.returncode != 0:
        sys.stderr.write(p.stdout + p.stderr)
        sys.exit(f"ct_check: the probe did not build at level {level}")
    return d / "target" / "release" / "probe"


def frames(exe, off):
    """addr2line's inlined frames at an offset of the probe: [(function, file:line)], innermost
    first.  Asked twice with -a, the second answer counts: binutils' addr2line gives the first
    address of a compilation unit no line"""
    p = subprocess.run(["addr2line", "-a", "-f", "-C", "-i", "-e", str(exe), hex(off), hex(off)], capture_output=True, text=True)
    blocks = re.split(r"^0x[0-9a-f]+$", p.stdout, flags=re.M)
    lines = blocks[-1].strip("\n").split("\n") if len(blocks) > 2 else []
    out = []
    for fn, at in zip(lines[0::2], lines[1::2]):
        for pre in PREFIXES:
            at = at.replace(pre, "")
        at = re.sub(r"^/rustc/[0-9a-f]+/library/", "", at)
        out.append((fn, at))
    return out


def has_line(fr):
    return bool(fr) and not fr[0][1].endswith(":?") and not fr[0][1].startswith("??")


def label(exe, off):
    """objdump's symbol label at an offset of the probe ('bcmp@plt', 'crypto::p256::mul+0x1c')"""
    p = subprocess.run(["objdump", "-d", "-C", "--no-show-raw-insn", f"--start-address={off:#x}", f"--stop-address={off + 1:#x}", str(exe)],
                       capture_output=True, text=True)
    m = re.findall(r"<([^>]+)>:", p.stdout)
    return m[-1] if m else None


def where(exe, loc):
    """exe+0x.. -> 'function (file:line) < caller (file:line)' (up to three inlined frames; without
    line information there, that of the instructions just before); /lib+0x.. -> 'lib: offset'"""
    if loc.startswith("/"):
        path, _, off = loc.rpartition("+")
        return f"{Path(path).name}+{off}"
    if not loc.startswith("exe+"):
        return loc
    off = int(loc[4:], 16)
    fr = frames(exe, off)
    near = ""
    if not has_line(fr):
        for k in range(1, 48):
            f2 = frames(exe, off - k)
            if has_line(f2):
                fr, near = f2, f"{k} bytes before: "
                break
    if not fr:
        return loc
    # the frames in this repository (lib/...), innermost first; the standard library's frame
    # before them in brackets ([core/src/array/equality.rs:157]: an array comparison)
    own = [(fn, at) for fn, at in fr if not at.startswith(("core/", "alloc/", "std/")) and not at.endswith(":?")]
    head = f"[{fr[0][1]}] " if own and fr[0] not in own else ""
    # the first frame carries the name of the function it was inlined into, not its own
    shown = [at if (fn, at) == fr[0] and len(fr) > 1 else f"{fn} ({at})" for fn, at in (own or fr)[:3]]
    return near + head + " < ".join(shown)


def from_where(exe, callers):
    """the call that led into a library: its Rust frames and the PLT stub's name"""
    offs = [int(c[4:], 16) for c in callers.split(",") if c.startswith("exe+")]
    stub = next((lb for lb in (label(exe, o) for o in offs) if lb and "@plt" in lb), None)
    site = next((o for o in offs if has_line(frames(exe, o)) or not (label(exe, o) or "").count("@plt")), None)
    text = where(exe, f"exe+{site:#x}") if site is not None else "-"
    return (f"{stub.split('@')[0]} called from " if stub else "") + text


def trace(exe, target, seeds, step):
    env = dict(os.environ)
    if step:
        env["CT_STEP"] = "step"
    # without address randomization: the paths of memcpy and the allocator depend on addresses,
    # which then are the same in every run
    cmd = (["setarch", "-R"] if SETARCH else []) + [str(exe), target, str(seeds)]
    p = subprocess.run(cmd, capture_output=True, text=True, env=env)
    return target, p.stdout.strip() or p.stderr.strip() or f"exit {p.returncode}"


def check(level, exe, targets, seeds, step):
    """-> number of problems at this level"""
    jobs = max(1, (os.cpu_count() or 2) - 1)
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        results = dict(pool.map(lambda t: trace(exe, t, seeds, step), targets))
    bad, same, mode, seen = 0, 0, "?", False
    lines = []
    for t in targets:
        r = results[t].split()
        if r and r[0] == "same":
            mode = r[2] if len(r) > 2 else mode
            if t == SELFTEST:
                bad += 1
                lines.append(f"  {t:<24} FAIL: the deliberate leak was not seen (is the tracer working?)")
            else:
                same += 1
        elif r and r[0] == "diff" and len(r) >= 9:
            mode = r[8]
            if t == SELFTEST:
                seen = True
                continue
            i, na, nb, branch, a, b, caller = r[1:8]
            unit = "branches" if mode == "block" else "instructions"
            known = KNOWN.get(t)
            if known and int(i) >= known[0] * min(int(na), int(nb)):
                same += 1
                lines.append(f"  {t:<24} differs after {i} of {na} {unit}, as expected: {known[1]}")
                continue
            bad += 1
            lines.append(f"  {t:<24} DIFF after {i} of {na} / {nb} {unit}")
            lines.append(f"      branch  {where(exe, branch)}")
            if not branch.startswith("exe+") and caller != "-":
                lines.append(f"      in      {from_where(exe, caller)}")
            lines.append(f"      then    {where(exe, a)}")
            lines.append(f"      or      {where(exe, b)}")
        else:
            bad += 1
            lines.append(f"  {t:<24} ERROR {results[t]}")
    checked = len([t for t in targets if t != SELFTEST])
    print(f"[{level}] {same} of {checked} operations take one way for {seeds} secrets ({mode})" + (", the self-test's leak seen" if seen else ""))
    for ln in lines:
        print(ln)
    return bad


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        rest = " ".join("'" + a.replace("'", "'\\''") + "'" for a in sys.argv[1:])
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/ct_check.py {rest}"]).returncode
    if sys.platform != "linux" or platform.machine() not in ("x86_64", "AMD64"):
        sys.exit("ct_check: Linux on x86-64 only (ptrace of rip)")
    ap = argparse.ArgumentParser(description="secret-independent control flow of lib/crypto")
    ap.add_argument("--opt", default=",".join(LEVELS), help="levels, comma-separated: " + ", ".join(LEVELS))
    ap.add_argument("--only", default="", help="targets, comma-separated (default: all of probe --list)")
    ap.add_argument("--seeds", type=int, default=3, help="secrets per target (at least 2)")
    ap.add_argument("--crypto", default=str(ROOT / "lib" / "crypto"), help="the crate to check (another version of it)")
    ap.add_argument("--step", action="store_true", help="every instruction instead of every taken branch")
    a = ap.parse_args()
    levels = [x for x in a.opt.split(",") if x]
    for lv in levels:
        if lv not in LEVELS:
            sys.exit(f"ct_check: unknown level {lv} ({', '.join(LEVELS)})")
    crate_dir = Path(a.crypto).resolve()
    label = "lib" if crate_dir == (ROOT / "lib" / "crypto").resolve() else "other"
    PREFIXES.insert(0, str(crate_dir.parent) + "/")
    bad = 0
    for lv in levels:
        exe = build(lv, crate_dir, label)
        listed = subprocess.run([str(exe), "--list"], capture_output=True, text=True).stdout.split()
        targets = [t for t in listed if not a.only or t in a.only.split(",") or t == SELFTEST]
        unknown = [t for t in a.only.split(",") if t and t not in listed]
        if unknown:
            sys.exit(f"ct_check: unknown targets {', '.join(unknown)}")
        bad += check(lv, exe, targets, max(2, a.seeds), a.step)
    print("ct_check: " + ("no difference" if bad == 0 else f"{bad} problem(s)"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

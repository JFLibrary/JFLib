"""ct_check.py -- lib/crypto's secret-dependent operations take the same way for every secret.
    python tools/ct_check.py [--opt release,dev,o3,s,z] [--only T,..] [--seeds N] [--crypto DIR] [--step] [--wasm]
Builds tools/ct_check/probe.rs against lib/crypto (or the crate in DIR) at each optimization level
below and lets it run every operation of its list (X25519, Ed25519, P-256, P-384, ECDSA signing,
ML-KEM, EAP-PWD, the RSA / DH exponentiation, the AEADs, AES, MACs, SHA-3 and SHAKE, KDFs,
Argon2id, Base64 and hex, and with this
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
the run fails as a whole.
With valgrind installed (--no-memcheck turns it off) every operation also runs once under
memcheck with its secret bytes marked undefined, as BoringSSL's constant-time check does:
memcheck reports a branch on anything made from them and an address made from them (a table
lookup at a secret index, which the trace cannot see; selftest_lookup must be reported).  Places
where a secret becomes public on purpose (an AEAD's verdict, Base64's padding, Argon2id's
data-dependent addressing) are in DECLASSIFIED, each with its reason, matched by the text of the
source line.  Not seen by either: instructions whose time depends on their operands (division,
some multipliers).
--wasm checks the probe compiled for wasm32-unknown-unknown instead, at the same levels: WABT's
wasm-interp runs each operation with --trace, and between two calls of ct_mark the sequence of the
executed wasm instructions and the addresses of every load, store, memory.copy and memory.fill
(not the values) must be the same for every secret; one comparison sees what the trace and
memcheck see on a processor, for the wasm the compiler made.  After a parting KNOWN allows in
named functions the comparison goes on from where they return, and selftest_resync (such a
parting, then a leak) must be reported.  A browser's JIT turns it into
machine code of its own, which no tool here sees.  Needs wasm-interp and wasm-objdump (WABT) and
the target (rustup target add wasm32-unknown-unknown); runs anywhere those do.
Linux on x86-64 or ARM64 (from Windows: runs itself again in WSL); needs cargo and binutils
(addr2line, objdump).  The probe runs without address randomization (personality(2), as
setarch -R) and starts every secret's run anew in that layout, so the ways of memcpy and the
allocator, which follow addresses, are the same.  ARM64 has no branch trap for ptrace, so there
the trace follows every instruction (as --step does on x86-64): some minutes per level instead of
seconds.  The workflow
.github/workflows/ct-check.yml runs it on both, on x86-64 and on an ARM64 runner.
Exit code 1 on any difference."""
import argparse
import concurrent.futures
import functools
import hashlib
import os
import platform
import re
import shutil
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
# a deliberate table lookup at a secret index: the same branches every time (the trace does not see
# it), memcheck must report it
SELFTEST_LOOKUP = "selftest_lookup"
# in wasm: a parting KNOWN allows, then a leak after it, which the check must still see
SELFTEST_RESYNC = "selftest_resync"
SELFTESTS = (SELFTEST, SELFTEST_LOOKUP, SELFTEST_RESYNC)
VALGRIND = shutil.which("valgrind")

# Where a secret becomes public on purpose, memcheck reports the branch on it as well: (file,
# a pattern of that source line, why the value is public).  Matched by the text of the line, so
# edits elsewhere in a file do not move the entries.
DECLASSIFIED = [
    # verdicts: whether a tag, an unwrapped key's check value or a shared secret passes is the
    # public answer of the operation
    ("crypto/aead.rs", r"ct_eq\(&want, t\)|\.then_some\(buf\)", "the AEAD's verdict (authentic or not) is its public answer"),
    ("crypto/gcm.rs", r"ct_eq\(&self\.tag", "the AEAD's verdict (authentic or not) is its public answer"),
    ("crypto/keywrap.rs", r"ct_eq\(&a, &IV\)", "the unwrap's verdict (the check value matches or not) is its public answer"),
    ("crypto/x25519.rs", r"ct_eq\(&s, &\[0u8; 32\]\)", "an all-zero shared secret is refused: that the peer sent a low-order point is public"),
    ("crypto/b64.rs", r"if good == 0", "the decoder's verdict (valid Base64 or not) is its public answer"),
    ("crypto/lib.rs", r"if good == 0", "the decoder's verdict (valid hex or not) is its public answer"),
    ("crypto/p256.rs", r"ct_mask\(br & nonzero\) == 1", "secret_from: whether random bytes make a scalar is public (they are drawn again)"),
    ("crypto/p256.rs", r"p\.z == \[0; 4\]", "the point at infinity ends the operation (None): reaching it is public"),
    ("crypto/p256.rs", r"\(found != 0\)\.then_some", "y_for_x: the answer whether there is a point (EAP-PWD calls the branch-free core)"),
    # ARM64 makes secret_from's answer without a branch (the Option's tag from the mask), and
    # public_key's assert branches on it: the same answer, where it is used
    ("crypto/p256.rs", r"assert!\(secret_from\(secret\)\.is_some\(\)", "public_key's assert takes secret_from's answer: whether the scalar is in [1, n - 1] is public"),
    ("crypto/p384.rs", r"assert!\(secret_from\(secret\)\.is_some\(\)|\.then_some\(\*random\)", "public_key's assert takes secret_from's answer: whether the scalar is in [1, n - 1] is public"),
    ("crypto/p384.rs", r"if finite == 0", "the point at infinity ends the operation (None): reaching it is public"),
    # ECDSA signing: refused keys and nonces, and the signature itself
    ("crypto/ecdsa.rs", r"in_range_mask\(&d\) == 0|if !secret_ok\(curve, secret\)", "a secret key outside [1, n - 1] is refused: the API's public answer"),
    # a key file: the public key stored with an ECDSA secret is compared with the one it gives
    ("x509/key.rs", r"ct_eq\(p, public\)", "the public key stored in the file is public: whether it is the secret's own is the reader's public answer"),
    ("crypto/ecdsa.rs", r"in_range_mask\(&kk\) != 0|if let Some\(sig\) = found",
     "a nonce outside [1, n - 1] is drawn again (RFC 6979 3.2 h.3, about 2^-32 on P-256): that one was refused shows nothing of the one taken"),
    ("crypto/ecdsa.rs", r"r\.iter\(\)\.all\(\|&w\| w == 0\) \|\| s\.iter", "r and s are the signature, which is published"),
    # ML-KEM: the matrix comes from rho, which the encapsulation key carries
    ("crypto/mlkem.rs", r"if d1 < Q && n < N|if d2 < Q && n < N|while n < N|f\[n\] = d",
     "SampleNTT's rejection sampling (FIPS 203 algorithm 7) reads rho, which the encapsulation key publishes"),
    # EAP-PWD: rand and mask are drawn again when they make no valid scalar; Scalar is sent
    ("crypto/eappwd.rs", r"scalar_ok\(rand\)|scalar_ok\(&scalar\)", "rand / mask that make no valid scalar are drawn again; Scalar itself is sent to the peer"),
    ("crypto/eappwd.rs", r"rounds < ROUNDS as u32 \|\| found == 0", "after the 40 rounds RFC 5931 asks for, a password without a point (2^-40) goes on"),
    # Argon2id addresses its memory by the password after the first half of the first pass
    ("crypto/argon2.rs", r"same|ref_index|ref_lane|mem\[", "Argon2id's data-dependent addressing (RFC 9106 3.4), by design"),
    # text formats: the positions of padding, delimiters and line ends are the public layout,
    # and Base64 or hex characters are never one of them; std's UTF-8 handling looks at the high
    # bit of characters that are ASCII by construction
    ("crypto/b64.rs", r"s\.push\(char_of", "the characters are ASCII by construction (std's String::push looks at the high bit)"),
    ("crypto/lib.rs", r"s\.push\(\(b'0' \+ n", "the characters are ASCII by construction (std's String::push looks at the high bit)"),
    ("x509/lib.rs", r"\(seed\.len\(\) == 32\)\.then", "the PKCS #8 structure around the key is public"),
    ("ct_check/probe.rs", r"\.map\(\|kp\| kp\.public\)", "the probe takes the loader's answer (Some or None is its public verdict)"),
    ("ct_check/probe.rs", r"pem::decode\(.*\.expect\(\"pem\"\)", "the probe takes the PEM decoder's answer (Some or None is its public verdict)"),
    ("crypto/b64.rs", r"trim_end_matches|let bytes = c\.len\(\) - 1|pad > 2|b\.len\(\) % 4|b\.chunks\(4\)|out\.push|with_capacity|c\.iter\(\)\.enumerate|for i in 0\.\.bytes|if bytes < 3", "padding and length are the public layout of the text (memcheck sees the length come from comparing the last characters with '=')"),
    ("x509/der.rs", r".", "DER's tags and lengths are the public structure of a key file"),
    ("x509/lib.rs", r"fn ed25519_from_pkcs8|d\.u64\(\)\? != 0|is_ed25519_alg|read\(der::OCTET_STRING\)", "the PKCS #8 structure around the key is public"),
    ("crypto/lib.rs", r"for b in buf\.iter_mut\(\)|\*b = 0", "zeroize's length is the buffer's, which is public"),
    # the Base64 decoder's answer where pem.rs takes it (ARM64 makes the Option's tag without a
    # branch), and the body's tag: Option<String> keeps None in the capacity, which grows with
    # the characters pushed, and their UTF-8 length is 1 (Base64 is ASCII by construction); and
    # split's answer, which the loop tests on its own line where split's next is a function of
    # its own (o3, 16 codegen units, Rust 1.98)
    ("netio/pem.rs", r"if let Some\(d\) = crypto::b64::decode\(b\)", "the Base64 decoder's verdict (valid or not) is its public answer"),
    ("netio/pem.rs", r"for line in text\.split\('\\n'\)", "whether another line follows is where the line ends are, PEM's public layout; no Base64 character is one (memcheck sees the answer come from comparing every character with '\\n')"),
    ("netio/pem.rs", r"match body\.as_mut\(\)", "the body's tag lives in its capacity, which grows by the characters' UTF-8 length: 1, ASCII by construction"),
    ("netio/pem.rs", r".", "PEM's layout (markers, line ends) is public; Base64 characters are none of them"),
    ("netio/ssh.rs", r".", "the OpenSSH key file's layout and fields other than the key are public"),
]
# own frames whose declassification holds only for errors raised inside std's text handling
TEXT_FILES = ("crypto/b64.rs", "netio/pem.rs", "netio/ssh.rs")
# operations whose way may depend on the secret by design, from this fraction of it on
# (a share of the first secret's trace: a parting of any secret before it is a difference), and
# with a third entry only where the paths part there: in one of these functions (a frame at the
# branch; in wasm, a call; inlined code is named after the function it went into) or, natively,
# on a source line of this file that matches the pattern.  With a third entry the wasm traces
# are compared again from where they return from those functions; natively the probe reports
# the first parting only, and memcheck, which lets through no more than DECLASSIFIED's lines,
# checks the rest
KNOWN = {
    "argon2id": (0.5, "Argon2id addresses its memory by the password after the first half of the "
                      "first pass (RFC 9106 3.4), and memcpy's path follows the addresses"),
    "mlkem768_keygen": (0.15, "ML-KEM's keygen samples s and e first and the matrix after them, from "
                              "rho, which the encapsulation key publishes: its rejection sampling "
                              "(FIPS 203 algorithm 7) follows rho",
                        {"functions": ("mlkem::sample_ntt", "mlkem::matrix"),
                         "line": ("crypto/mlkem.rs", r"if d1 < Q && n < N|if d2 < Q && n < N|while n < N|r\.read\(&mut c\)")}),
    SELFTEST_RESYNC: (0.0, "the self-test's allowed part", {"functions": ("resync_public",), "line": ("ct_check/probe.rs", r"$^")}),
}


def source_line(at):
    """the text of file:line as a report gives it (relative to one of PREFIXES), or None"""
    path, _, line = at.rpartition(":")
    if not line.isdigit():
        return None
    for pre in [""] + PREFIXES + [str(ROOT / "lib") + "/"]:
        f = Path(pre + path)
        if f.is_file():
            try:
                return f.read_text(encoding="utf-8").splitlines()[int(line) - 1]
            except (OSError, IndexError):
                return None
    return None


def known_parting(t, pos, total, functions, locations=()):
    """the reason when target t may part at pos of total, else None; functions: the names of the
    frames (or calls) at the parting, locations: their file:line, innermost first"""
    k = KNOWN.get(t)
    if not k or pos < k[0] * total:
        return None
    if len(k) > 2:
        spec = k[2]
        by_name = any(want in fn for want in spec["functions"] for fn in functions)
        file, pattern = spec["line"]
        text = source_line(locations[0]) if locations and locations[0].startswith(file) else None
        if not by_name and not (text is not None and re.search(pattern, text)):
            return None
    return k[1]
# folders cut from the paths in reports (the checked crate's parent is added by main)
PREFIXES = [str(ROOT) + "/"]


def package_name(crate_dir):
    text = (crate_dir / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^\[package\][^\[]*?^name\s*=\s*"([^"]+)"', text, re.M | re.S)
    if not m:
        sys.exit(f"ct_check: no package name in {crate_dir / 'Cargo.toml'}")
    return m.group(1)


# where the probe's image begins (build): above every range the debug information of dropped
# functions gives from address 0
IMAGE_BASE = 0x10000000


@functools.lru_cache(maxsize=None)
def image_base(exe):
    """the lowest address of the probe's loaded segments (ELF64, little endian): an offset in
    its mapping plus this is an address of the file, the one addr2line and objdump take"""
    import struct
    b = Path(exe).read_bytes()[:65536]
    phoff, = struct.unpack_from("<Q", b, 0x20)
    phentsize, phnum = struct.unpack_from("<HH", b, 0x36)
    loads = [struct.unpack_from("<Q", b, phoff + i * phentsize + 16)[0] for i in range(phnum)
             if struct.unpack_from("<I", b, phoff + i * phentsize)[0] == 1]
    return min(loads, default=0) & ~0xfff


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
    # DWARF 4 line tables: valgrind 3.26 puts lines of a binary of many codegen units in the wrong
    # files when they are DWARF 5 (memcheck's reports need the right line); code is the same.
    # The image from IMAGE_BASE up: without LTO (o3, dev) the dependencies' unused functions are
    # compiled and then dropped by the linker, and their debug information stays, from address 0
    # on.  One as long as netpkt's decode::layers covered the first 96 KiB of ARM64's .text (at
    # 0x6080: crypto's code there was decode.rs to valgrind and addr2line alike; x86-64's .text
    # began after most of it, by luck).  Above 256 MiB no range from 0 reaches the code; the
    # machine code of each function is the same (keeping the dropped functions instead does not
    # link: they name what only an app defines, plat's jfgui_main).  --image-base: lld and GNU ld
    # 2.38 on; -Ttext-segment: older GNU ld
    for base_flag in (f"--image-base={IMAGE_BASE:#x}", f"-Ttext-segment={IMAGE_BASE:#x}"):
        env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(["-Cdwarf-version=4", f"-Clink-arg=-Wl,{base_flag}"])
        p = subprocess.run(["cargo", "build", "--release", "--quiet", "--manifest-path", str(d / "Cargo.toml")],
                           env=env, capture_output=True, text=True)
        if p.returncode == 0 or base_flag.split("=")[0].lstrip("-") not in p.stdout + p.stderr:
            break
    if p.returncode != 0:
        sys.stderr.write(p.stdout + p.stderr)
        sys.exit(f"ct_check: the probe did not build at level {level}")
    return d / "target" / "release" / "probe"


def frames(exe, off):
    """addr2line's inlined frames at an offset of the probe: [(function, file:line)], innermost
    first.  Asked twice with -a, the second answer counts: binutils' addr2line gives the first
    address of a compilation unit no line"""
    at = hex(off + image_base(exe))
    p = subprocess.run(["addr2line", "-a", "-f", "-C", "-i", "-e", str(exe), at, at], capture_output=True, text=True)
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
    off += image_base(exe)
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
    # the probe turns address randomization off itself (as setarch -R) and starts every secret's
    # run anew in that layout: the ways of memcpy and the allocator depend on addresses
    p = subprocess.run([str(exe), target, str(seeds)], capture_output=True, text=True, env=env)
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
            # one threshold for every secret, of the first secret's trace (the reference the
            # parting is counted in): so the earliest parting the probe reports decides for all
            fr = []
            if branch.startswith("exe+"):
                # where the branch's own line is missing, the instructions just before it (as where)
                off = int(branch[4:], 16)
                fr = frames(exe, off)
                k = 1
                while not has_line(fr) and k < 48:
                    fr, k = frames(exe, off - k), k + 1
            why = known_parting(t, int(i), int(na), [fn for fn, _ in fr], [at for _, at in fr])
            if why:
                same += 1
                lines.append(f"  {t:<24} differs after {i} of {na} {unit}, as expected: {why}")
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


def vg_errors(log):
    """memcheck's errors of a log: [(kind, [(function, file, line)])], innermost frame first"""
    out = []
    cur = None
    for ln in log.splitlines():
        m = re.match(r"==\d+== (Conditional jump or move depends on uninitialised|Use of uninitialised value|Syscall param .* uninitialised|Invalid read|Invalid write)", ln)
        if m:
            kind = "branch" if m.group(1).startswith("Conditional") else ("address" if m.group(1).startswith("Use of") else "other")
            cur = (kind, [])
            out.append(cur)
            continue
        f = re.match(r"==\d+==    (at|by) 0x([0-9A-F]+): (.*?) \((.*)\)$", ln)
        if f and cur is not None:
            where = re.match(r"(.*):(\d+)$", f.group(4))
            path, line = (where.group(1), int(where.group(2))) if where else (f.group(4).removeprefix("in "), 0)
            # (function, file, line, address, return address?)
            cur[1].append((f.group(3), path, line, int(f.group(2), 16), f.group(1) == "by"))
        elif re.match(r"==\d+== $", ln):
            cur = None
    return out


def resymbolize(exe, errors, span=None):
    """valgrind 3.26 puts lines of a binary of many codegen units into the wrong files; its
    function names are right.  The frames inside the probe get their (inlined) functions and
    lines from addr2line instead, at the load address under which addr2line names the same
    functions as valgrind.  `span` is where the probe says it was loaded (valgrind's choice,
    which differs between architectures and linkers); without it, x86-64's of a probe from
    address 0.  Every frame comes back as (function, file, line)"""
    lo, hi = span if span else (0x1000000, 1 << 64)
    addrs = sorted({fr[3] - (1 if fr[4] else 0) for _, frames in errors for fr in frames if lo <= fr[3] < hi})
    if not addrs:
        return [(kind, [fr[:3] for fr in frames]) for kind, frames in errors]

    def lookup(base, some):
        # each address asked twice: binutils' addr2line answers the first one of a unit without a line
        args = [hex(a - base + img) for a in some for _ in (0, 1)]
        out = subprocess.run(["addr2line", "-a", "-f", "-C", "-i", "-e", str(exe)] + args, capture_output=True, text=True).stdout
        blocks = re.split(r"^0x([0-9a-f]+)$", out, flags=re.M)[1:]
        res = {}
        for i in range(0, len(blocks), 4):
            off = int(blocks[i + 2], 16) if i + 2 < len(blocks) else None
            lines = blocks[i + 3].strip("\n").split("\n") if i + 3 < len(blocks) else []
            if off is None:
                continue
            fr = []
            for fn, at in zip(lines[0::2], lines[1::2]):
                m = re.match(r"(.*):(\d+)", at)
                fr.append((fn, m.group(1) if m else at, int(m.group(2)) if m else 0))
            res[off - img + base] = fr
        return res

    img = image_base(exe)
    names = {fr[3] - (1 if fr[4] else 0): fr[0] for _, frames in errors for fr in frames}
    best, base = -1, None
    for b in dict.fromkeys(([span[0]] if span else []) + [0x4000000, 0x108000, 0x400000, 0]):
        got = lookup(b, addrs[:20])
        score = sum(1 for a, fr in got.items() if fr and any(f[0].split("::")[-1][:20] in names.get(a, "") or names.get(a, "").split("::")[-1][:20] in f[0] for f in fr))
        if score > best:
            best, base = score, b
    table = lookup(base, addrs)
    out = []
    for kind, frames in errors:
        new = []
        for fr in frames:
            chain = table.get(fr[3] - (1 if fr[4] else 0))
            if chain and any(c[2] > 0 for c in chain):
                new.extend(chain)
            else:
                new.append(fr[:3])
        out.append((kind, new))
    return out


def is_own(fr):
    """whether a frame is in this repository's sources, with a line"""
    return fr[2] > 0 and (fr[1].startswith(tuple(PREFIXES)) or "/lib/" in fr[1])


def own_frame(frames):
    """the innermost frame in this repository's sources with a line (or the innermost one)"""
    for fr in frames:
        if is_own(fr):
            return fr
    return frames[0] if frames else ("?", "?", 0)


# std's allocation code, which branches on a buffer's size alone
ALLOC_SOURCES = ("/alloc/src/raw_vec/", "/alloc/src/vec/", "/alloc/src/alloc.rs", "/core/src/alloc/layout",
                 "/core/src/intrinsics/", "/core/src/num/uint_macros")


def declassified(fr, frames):
    fn, path, line = fr
    try:
        text = Path(path).read_text(encoding="utf-8").splitlines()[line - 1]
    except (OSError, IndexError):
        return None
    for file, pattern, why in DECLASSIFIED:
        if not path.endswith(file) or not re.search(pattern, text):
            continue
        # a text file's catch-all covers std's text handling under it, not its own lines (those
        # need an entry that names them); netio's parsers of the decoded structure are public
        # layout too
        if pattern == "." and file in TEXT_FILES and file != "crypto/b64.rs" and frames and frames[0][1] == path and "ssh.rs" not in file:
            continue
        return why
    return None


def memcheck_one(exe, target):
    log = exe.parent.parent.parent / f"memcheck-{target}.log"
    # ARM64: smaller translation blocks.  VEX's ARM64 back end has 4 KiB of spill slots, and a
    # long run of straight-line code (AES, with --track-origins' shadows) needed more
    # ("genSpill_ARM64: Assertion `offsetB < 4096' failed"); what memcheck sees is the same
    small = ["--vex-guest-max-insns=32", "--vex-guest-chase=no"] if platform.machine() in ("aarch64", "arm64") else []
    cmd = [VALGRIND, "--tool=memcheck", "-q", "--error-exitcode=99", "--track-origins=yes", "--num-callers=40", *small,
           "--fullpath-after=", f"--log-file={log}", str(exe), "--memcheck", target, "1"]
    p = subprocess.run(cmd, capture_output=True, text=True)
    text = log.read_text(encoding="utf-8", errors="replace") if log.exists() else ""
    m = re.search(r"^exe 0x([0-9a-f]+) 0x([0-9a-f]+)$", p.stdout, re.M)
    span = (int(m.group(1), 16), int(m.group(2), 16)) if m else None
    # when valgrind itself ends the run (neither 0 nor memcheck's 99), its log says why
    tail = (p.stdout + p.stderr + ("" if p.returncode in (0, 99) else "\n".join(text.splitlines()[-8:])))[-800:]
    return target, p.returncode, resymbolize(exe, vg_errors(text), span), tail


def memcheck(level, exe, targets):
    """valgrind's memcheck with the secret marked undefined -> number of problems at this level"""
    jobs = max(1, (os.cpu_count() or 2) - 1)
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        results = list(pool.map(lambda t: memcheck_one(exe, t), targets))
    bad, clean, seen_branch, seen_lookup = 0, 0, False, False
    lines = []
    for target, rc, errors, tail in results:
        if rc not in (0, 99):
            bad += 1
            lines.append(f"  {target:<24} ERROR exit {rc}: {' | '.join(tail.strip().splitlines())[-600:]}")
            continue
        found = {}
        for kind, frames in errors:
            fr = own_frame(frames)
            # invalid reads and writes (and a system call given the secret) are never excused:
            # the exceptions below are for branches and addresses that follow public values
            if kind == "other":
                found.setdefault((kind, fr), frames)
                continue
            if declassified(fr, frames):
                continue
            # allocation sizes come from lengths, which are public (std's capacity checks and
            # frees of a buffer whose length memcheck follows from a text's padding): the branch
            # is in allocation code, so the frames inside the first own one, up to the three
            # innermost, are all allocation code.  Up to three: how many there are depends on
            # the inlining (at opt-level z on ARM64 a Vec's free is two frames of raw_vec, or
            # Global::deallocate's three of alloc.rs), and std code may stand between them and
            # the own frame (String::push growing its Vec at opt-level z on x86-64).  A stack
            # without an own frame is not excused
            own_at = next((i for i, f in enumerate(frames) if is_own(f)), 0)
            if own_at and all(any(s in f[1] for s in ALLOC_SOURCES) for f in frames[:min(own_at, 3)]):
                continue
            # String::push of a Base64 or hex character (ASCII by construction) where the line
            # of the caller is lost
            if any(f[1].endswith("alloc/src/string.rs") for f in frames[:6]) and any(f[1].split(":")[0].endswith(("crypto/b64.rs", "crypto/lib.rs")) for f in frames[:8]):
                continue
            # the probe dropping the operation's answer: Some or None is its public verdict
            if any(f[0].startswith(("drop_glue", "core::ptr::drop_in_place")) for f in frames[:8]) and fr[1].endswith("probe.rs"):
                continue
            found.setdefault((kind, fr), frames)
        if target == SELFTEST:
            seen_branch = any(k == "branch" and f[1].endswith("probe.rs") for k, f in found)
            continue
        if target == SELFTEST_LOOKUP:
            seen_lookup = any(k == "address" and f[1].endswith("probe.rs") for k, f in found)
            continue
        if not found:
            clean += 1
            continue
        bad += 1
        lines.append(f"  {target:<24} {len(found)} place(s) that depend on the secret")
        for (kind, fr), frames in list(found.items())[:6]:
            chain = " < ".join(f"{f[0].split('<')[0].split('(')[0]} ({f[1].split('/lib/')[-1]}:{f[2]})" for f in frames[:3])
            lines.append(f"      {kind:<8} {chain}")
    checked = len([t for t in targets if t not in (SELFTEST, SELFTEST_LOOKUP)])
    for ok, what in ((seen_branch, "the branch of " + SELFTEST), (seen_lookup, "the table lookup of " + SELFTEST_LOOKUP)):
        if not ok:
            bad += 1
            lines.append(f"  FAIL: memcheck did not see {what} (is the secret marked?)")
    print(f"[{level}] memcheck: {clean} of {checked} operations neither branch on the secret nor use it as an address"
          + (", both self-tests seen" if seen_branch and seen_lookup else ""))
    for ln in lines:
        print(ln)
    return bad



# ---------------------------------------------------------------- wasm (--wasm)
WASM_TARGET = "wasm32-unknown-unknown"


def build_wasm(level, crate_dir, label):
    """the probe as a wasm cdylib at one level -> its .wasm (crypto alone: netio is not built for
    the browser, so the key-file targets are left out)"""
    d = OUT / f"wasm-{label}-{level}"
    d.mkdir(parents=True, exist_ok=True)
    prof = "\n".join(f"{k} = {v}" for k, v in LEVELS[level].items())
    (d / "Cargo.toml").write_text(f"""# written by tools/ct_check.py --wasm
[package]
name = "ct-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[lib]
crate-type = ["cdylib"]
path = "{PROBE.as_posix()}"

[dependencies]
crypto = {{ package = "{package_name(crate_dir)}", path = "{crate_dir.as_posix()}" }}

[features]
netio = []

[profile.release]
{prof}
panic = "abort"
strip = false
""", encoding="utf-8")
    env = dict(os.environ, CARGO_TARGET_DIR=str(d / "target"))
    env.pop("RUSTFLAGS", None)
    p = subprocess.run(["cargo", "build", "--release", "--quiet", "--target", WASM_TARGET, "--manifest-path", str(d / "Cargo.toml")],
                       env=env, capture_output=True, text=True)
    if p.returncode != 0:
        sys.stderr.write(p.stdout + p.stderr)
        sys.exit(f"ct_check: the wasm probe did not build at level {level} (rustup target add {WASM_TARGET})")
    return d / "target" / WASM_TARGET / "release" / "ct_probe.wasm"


def wasm_functions(wasm):
    """{function index: name} from the name section (demangled with c++filt when it is there)"""
    out = subprocess.run(["wasm-objdump", "-x", "-j", "Function", str(wasm)], capture_output=True, text=True).stdout
    names = {int(m.group(1)): m.group(2) for m in re.finditer(r"func\[(\d+)\] sig=\d+ <(.+?)>", out)}
    if shutil.which("c++filt") and names:
        keys = list(names)
        dem = subprocess.run(["c++filt"], input="\n".join(names[k] for k in keys), capture_output=True, text=True).stdout.split("\n")
        if len(dem) >= len(keys):
            names = {k: re.sub(r"::h[0-9a-f]{16}$", "", dem[i]) for i, k in enumerate(keys)}
    return names


def wasm_key(line):
    """what of a trace line must not depend on the secret: its place (frame depth, instruction,
    value stack depth) and, for memory operations, the addresses and lengths, not the values"""
    i = line.find(b"|")
    left, rest = line[:i], line[i + 2:].rstrip()
    sp = rest.find(b" ")
    op = rest[:sp] if sp > 0 else rest
    if b".load" in op or b".store" in op:
        return left + op + rest[sp:].split(b",", 1)[0]
    if op == b"memory.copy":
        return left + op + b",".join(rest[sp:].split(b",")[2:5])
    if op == b"memory.fill":
        a = rest[sp:].split(b",")
        return left + op + a[1] + b"," + a[-1]
    return left


def wasm_cmd(wasm, index, seed):
    return ["wasm-interp", str(wasm), "-r", "ct_run", "-a", f"i32:{index}", "-a", f"i64:{seed}", "--trace"]


def wasm_digest(wasm, mark, index, seed):
    """-> (instructions between the marks, a digest of their keys, marks seen)"""
    h = hashlib.blake2b(digest_size=16)
    n = marks = 0
    with subprocess.Popen(wasm_cmd(wasm, index, seed), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL) as p:
        for line in p.stdout:
            if line.endswith(mark):
                marks += 1
                if marks == 2:
                    break
            elif marks == 1:
                h.update(wasm_key(line) + b"\n")
                n += 1
        p.kill()
    return n, h.hexdigest(), marks


def wasm_depth(line):
    return int(line[1:line.find(b".")])


def wasm_parting(wasm, mark, names, index, seed, target, total):
    """both traces side by side from the first mark to the second -> (the first parting the
    design does not allow, the first it allows, with KNOWN's reason), each None or (position in
    seed 1's trace of `total`, line of seed 1, line of the other, the functions there, outermost
    first).  A parting KNOWN allows in its functions is passed over: each trace goes on to where
    it has returned from the outermost of them, and from there both are compared again, so a
    parting after it (in the code that follows the public part) is still a difference.  One
    KNOWN allows without functions (Argon2id's) ends the comparison"""
    k = KNOWN.get(target)
    resync = k[2]["functions"] if k and len(k) > 2 else ()
    a = subprocess.Popen(wasm_cmd(wasm, index, 1), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    b = subprocess.Popen(wasm_cmd(wasm, index, seed), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    ia, ib = iter(a.stdout), iter(b.stdout)
    stack = {}
    n, bad, allowed = 0, None, None
    try:
        for it in (ia, ib):
            if not any(ln.endswith(mark) for ln in it):
                return None, None
        la, lb = next(ia, None), next(ib, None)
        while la is not None and lb is not None:
            ma, mb = la.endswith(mark), lb.endswith(mark)
            if ma and mb:
                break
            depth = wasm_depth(la)
            if ma or mb or wasm_key(la) != wasm_key(lb):
                chain = [names.get(stack.get(d), "?") for d in range(1, depth + 1)]
                where = (n, la.decode(errors="replace").strip(), lb.decode(errors="replace").strip(), chain)
                why = known_parting(target, n, total, chain)
                if not why:
                    bad = where
                    break
                if allowed is None:
                    allowed = where + (why,)
                outer = next((d for d in range(1, depth + 1) if any(f in chain[d - 1] for f in resync)), None)
                if outer is None or ma or mb:
                    break
                # each trace on to its first line outside that function: the call that entered it
                # was the same in both, so the frames below it are too
                while la is not None and not la.endswith(mark) and wasm_depth(la) >= outer:
                    la, n = next(ia, None), n + 1
                while lb is not None and not lb.endswith(mark) and wasm_depth(lb) >= outer:
                    lb = next(ib, None)
                continue
            rest = la[la.find(b"|") + 2:]
            if rest.startswith(b"call $"):
                stack[depth + 1] = int(rest[6:].strip())
            elif rest.startswith(b"call_indirect"):
                stack[depth + 1] = None
            n += 1
            la, lb = next(ia, None), next(ib, None)
    finally:
        for p in (a, b):
            p.kill()
            p.wait()
    return bad, allowed


def wasm_diff(wasm, mark, names, index, digests, target):
    """the digests of one target's secrets (1, 2, ...) -> None, ("diff", where, count A, count B),
    ("known", where and reason, count A) or ("error", text)"""
    first = digests[0]
    # every secret's run reached both marks (one that ended early would be a difference or,
    # with the same digest, the same as a part)
    for s, d in enumerate(digests, start=1):
        if d[2] < 2:
            return ("error", f"secret {s}: the marks were not reached ({d[2]}): did wasm-interp run it?")
    # every differing secret's partings: the earliest one the design does not allow counts (an
    # expected late one, Argon2id's, must not hide an early one of another secret)
    parts = [(wasm_parting(wasm, mark, names, index, s, target, first[0]), other[0])
             for s, other in enumerate(digests[1:], start=2) if other[1] != first[1]]
    if not parts:
        return None
    bad = [(p[0], count) for p, count in parts if p[0]]
    if bad:
        where, count = min(bad, key=lambda p: p[0][0])
        return ("diff", where, first[0], count)
    known = [p[1] for p, _ in parts if p[1]]
    if len(known) == len(parts):
        return ("known", min(known, key=lambda w: w[0]), first[0])
    # the digests differ, but side by side the traces did not part: say so
    return ("diff", None, first[0], next(count for p, count in parts if not p[1]))


def check_wasm(level, wasm, targets, seeds):
    """-> number of problems at this level (the targets in the probe's order, their index is ct_run's)"""
    names = wasm_functions(wasm)
    marks = [k for k, v in names.items() if v == "ct_mark"]
    if not marks:
        sys.exit("ct_check: ct_mark is not in the wasm probe's name section")
    mark = f"| call ${marks[0]}\n".encode()
    jobs = max(1, (os.cpu_count() or 2) - 1)
    # every (target, secret) on its own, in processes (the lines are read in Python, one GIL
    # would make them one core): the long ones (EAP-PWD, RSA) side by side
    runs = [(i, t, s) for i, t in targets for s in range(1, seeds + 1)]
    with concurrent.futures.ProcessPoolExecutor(jobs) as pool:
        digests = dict(zip([(t, s) for _, t, s in runs], pool.map(wasm_digest, *zip(*[(wasm, mark, i, s) for i, _, s in runs]))))
        results = dict(zip([t for _, t in targets], pool.map(
            wasm_diff, *zip(*[(wasm, mark, names, i, [digests[(t, s)] for s in range(1, seeds + 1)], t) for i, t in targets]))))
    bad, same, seen = 0, 0, set()
    lines = []
    for _, t in targets:
        r = results[t]
        if r is None or (t == SELFTEST_RESYNC and r[0] == "known"):
            if t in SELFTESTS:
                bad += 1
                after = " after the parting it allows" if t == SELFTEST_RESYNC else ""
                lines.append(f"  {t:<24} FAIL: the self-test's leak{after} was not seen (is the trace working?)")
            else:
                same += 1
            continue
        if r[0] == "error":
            bad += 1
            lines.append(f"  {t:<24} ERROR {r[1]}")
            continue
        if t in SELFTESTS:
            seen.add(t)
            continue
        if r[0] == "known":
            # one threshold for every secret, of the first secret's run (the reference the
            # parting is counted in): wasm_parting judged each parting with it
            _, where, na = r
            again = " (compared again after it: the same)" if len(KNOWN[t]) > 2 else ""
            same += 1
            lines.append(f"  {t:<24} differs after {where[0]} of {na} wasm instructions, as expected{again}: {where[4]}")
            continue
        _, where, na, nb = r
        pos = where[0] if where else 0
        bad += 1
        lines.append(f"  {t:<24} DIFF after {pos} of {na} / {nb} wasm instructions")
        if where:
            lines.append(f"      in      {' < '.join(reversed(where[3])) or '?'}")
            lines.append(f"      secret 1  {where[1]}")
            lines.append(f"      another   {where[2]}")
    checked = len([1 for _, t in targets if t not in SELFTESTS])
    every = all(t in seen for t in SELFTESTS)
    print(f"[wasm {level}] {same} of {checked} operations take one way with the same addresses for {seeds} secrets"
          + (", the three self-tests seen" if every else ""))
    for ln in lines:
        print(ln)
    return bad


def main_wasm(a, levels, crate_dir, label):
    for tool in ("wasm-interp", "wasm-objdump"):
        if shutil.which(tool) is None:
            sys.exit(f"ct_check: --wasm needs {tool} (WABT: apt install wabt, brew install wabt)")
    # the probe's list in wasm: TARGETS without the key files (each behind #[cfg(feature =
    # "netio")]), in the same order (the index is what ct_run takes)
    order, key_files, gated = [], [], False
    for ln in PROBE.read_text(encoding="utf-8").split("const TARGETS: &[&str] = &[", 1)[1].split("];", 1)[0].splitlines():
        ln = ln.strip()
        if ln == '#[cfg(feature = "netio")]':
            gated = True
        elif ln.startswith('"'):
            (key_files if gated else order).append(ln.strip('",'))
            gated = False
    wasm_targets = order
    asked = [t for t in a.only.split(",") if t]
    unknown = [t for t in asked if t not in wasm_targets]
    if unknown:
        why = " (netio's key files are not built for wasm)" if any(t in key_files for t in unknown) else ""
        sys.exit(f"ct_check: unknown targets for --wasm {', '.join(unknown)}{why}")
    targets = [(i, t) for i, t in enumerate(wasm_targets) if not asked or t in asked or t in SELFTESTS]
    bad = 0
    for lv in levels:
        wasm = build_wasm(lv, crate_dir, label)
        bad += check_wasm(lv, wasm, targets, max(2, a.seeds))
    print("ct_check: " + ("no difference" if bad == 0 else f"{bad} problem(s)"))
    return 1 if bad else 0


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        rest = " ".join("'" + a.replace("'", "'\\''") + "'" for a in sys.argv[1:])
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/ct_check.py {rest}"]).returncode
    ap = argparse.ArgumentParser(description="secret-independent control flow of lib/crypto")
    ap.add_argument("--opt", default=",".join(LEVELS), help="levels, comma-separated: " + ", ".join(LEVELS))
    ap.add_argument("--only", default="", help="targets, comma-separated (default: all of probe --list)")
    ap.add_argument("--seeds", type=int, default=3, help="secrets per target (at least 2)")
    ap.add_argument("--crypto", default=str(ROOT / "lib" / "crypto"), help="the crate to check (another version of it)")
    ap.add_argument("--step", action="store_true", help="every instruction instead of every taken branch")
    ap.add_argument("--no-memcheck", action="store_true", help="only the trace, not valgrind's memcheck")
    ap.add_argument("--wasm", action="store_true", help="the probe for wasm32 in WABT's wasm-interp instead")
    a = ap.parse_args()
    levels = [x for x in a.opt.split(",") if x]
    for lv in levels:
        if lv not in LEVELS:
            sys.exit(f"ct_check: unknown level {lv} ({', '.join(LEVELS)})")
    crate_dir = Path(a.crypto).resolve()
    label = "lib" if crate_dir == (ROOT / "lib" / "crypto").resolve() else "other"
    if a.wasm:
        return main_wasm(a, levels, crate_dir, label)
    if sys.platform != "linux" or platform.machine() not in ("x86_64", "AMD64", "aarch64", "arm64"):
        sys.exit("ct_check: Linux on x86-64 or ARM64 only (ptrace of the program counter; --wasm runs elsewhere)")
    PREFIXES.insert(0, str(crate_dir.parent) + "/")
    bad = 0
    for lv in levels:
        exe = build(lv, crate_dir, label)
        listed = subprocess.run([str(exe), "--list"], capture_output=True, text=True).stdout.split()
        targets = [t for t in listed if not a.only or t in a.only.split(",") or t in (SELFTEST, SELFTEST_LOOKUP)]
        unknown = [t for t in a.only.split(",") if t and t not in listed]
        if unknown:
            sys.exit(f"ct_check: unknown targets {', '.join(unknown)}")
        bad += check(lv, exe, [t for t in targets if t != SELFTEST_LOOKUP], max(2, a.seeds), a.step)
        # overflow checks put a never-taken branch on every addition of secrets, which memcheck
        # cannot tell from a leak (the trace shows they never change the way): not under dev
        if VALGRIND and not a.no_memcheck and LEVELS[lv].get("overflow-checks") != "true":
            bad += memcheck(lv, exe, targets)
    print("ct_check: " + ("no difference" if bad == 0 else f"{bad} problem(s)"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

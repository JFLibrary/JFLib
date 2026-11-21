"""net_fuzz.py -- mutation fuzzing of the parsers that read untrusted bytes in compress, netio and netpkt.
    python tools/net_fuzz.py [--seconds N] [--only T,..]
Builds tools/net_fuzz/fuzz.rs (with panic = "unwind", overflow checks and debug assertions) into
build/net_fuzz and runs every target on its own thread for N seconds (default 60):
  zstd, lz4                     decompress with an output limit
  x509, x509_netpkt             the certificate parser (the crate jflib-x509) with names,
                                fingerprint and verify_chain; netpkt's check_chain over it;
                                seeds: the system's CA bundle
  dns, dns_netpkt               netio's answer parser and netpkt's message parser
  http_netio, http_netpkt       netio's request reader over a stream in pieces, netpkt's heads
  tls_server                    netio's TLS server fed a mutated ClientHello
  tls_client, tls_webclient     netio's clients (own CA, web PKI) fed a mutated server flight
  ipv4_reassembly, ipv6_reassembly   netpkt's fragment reassembly fed mutated, reordered,
                                duplicated and overlapping fragments
  netpkt_decode, netpkt_app     the dissector, and SNMP, RADIUS, BGP, mail, SSH KEXINIT parsers
The seeds are built by the program itself, plus the system's certificates and frames of the
reference `zstd` (levels, --long, --ultra) when that tool is there.  No coverage feedback: this
finds panics, hangs and slow inputs near the seeds, not deep paths (cargo-fuzz would).
A panic (a crash under panic = "abort", the library's setting), a hang (a call over 10 s) or a call
over --slow seconds fails the run, and so does a run in which the target selftest_panic (a
deliberate panic) found nothing; the inputs are in build/net_fuzz/out.  Linux (from Windows:
runs itself again in WSL); needs cargo.  Exit code 1 on any finding."""
import argparse
import os
import random
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "tools" / "net_fuzz" / "fuzz.rs"
OUT = ROOT / "build" / "net_fuzz"
SELFTEST = "selftest_panic"
CA_BUNDLES = ["/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt", "/etc/ssl/cert.pem"]


def build():
    lib = ROOT / "lib"
    dep = lambda name, crate, feats="": f'{name} = {{ package = "{crate}", path = "{(lib / name).as_posix()}"{feats} }}'  # noqa: E731
    (OUT / "Cargo.toml").write_text("\n".join([
        "# written by tools/net_fuzz.py",
        "[package]", 'name = "net-fuzz"', 'version = "0.0.0"', 'edition = "2024"', "publish = false", "",
        "[workspace]", "",
        "[[bin]]", 'name = "fuzz"', f'path = "{SRC.as_posix()}"', "",
        "[dependencies]",
        dep("rt", "jflib-rt", ', features = ["std", "unwind"]'),
        dep("plat", "jflib-plat", ', features = ["std", "unwind"]'),
        dep("crypto", "jflib-crypto"),
        dep("compress", "jflib-compress"),
        dep("netpkt", "jflib-netpkt"),
        dep("netio", "jflib-netio"), "",
        "[profile.release]", "opt-level = 3", "debug = 1", "debug-assertions = true", "overflow-checks = true",
        'panic = "unwind"', "",
    ]), encoding="utf-8")
    env = dict(os.environ, CARGO_TARGET_DIR=str(OUT / "target"))
    env.pop("RUSTFLAGS", None)
    p = subprocess.run(["cargo", "build", "--release", "--quiet", "--manifest-path", str(OUT / "Cargo.toml")], env=env,
                       capture_output=True, text=True)
    if p.returncode != 0:
        sys.stderr.write(p.stdout + p.stderr)
        sys.exit("net_fuzz: the fuzzer did not build")
    return OUT / "target" / "release" / "fuzz"


def seeds():
    """build/net_fuzz/seeds: the system's certificates and the reference zstd's frames"""
    d = OUT / "seeds"
    d.mkdir(parents=True, exist_ok=True)
    for b in CA_BUNDLES:
        if Path(b).is_file():
            shutil.copyfile(b, d / "ca.pem")
            break
    else:
        print("net_fuzz: no CA bundle found; x509 starts from the fuzzer's own certificates only")
    if shutil.which("zstd") is None:
        print("net_fuzz: no zstd tool; zstd starts from the library's own frames only")
        return d
    r = random.Random(3)
    text = b"Restore-Points werden dedupliziert, komprimiert und verschluesselt. " * 3000
    rnd = r.randbytes(20000)
    mixed = text[:30000] + rnd[:5000] + b"\0" * 8000 + text[:5000]
    i = 0
    for data in (text, rnd, mixed, b"hello hello hello"):
        for args in (["-1"], ["-3"], ["-19"], ["--fast=3"], ["--ultra", "-22"], ["-3", "--no-check"], ["-9", "--long=20"]):
            p = subprocess.run(["zstd", "-q", "-c"] + args, input=data, capture_output=True)
            if p.returncode == 0:
                (d / f"ref{i}.zst").write_bytes(p.stdout)
                i += 1
    return d


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        rest = " ".join("'" + a.replace("'", "'\\''") + "'" for a in sys.argv[1:])
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/net_fuzz.py {rest}"]).returncode
    ap = argparse.ArgumentParser(description="mutation fuzzing of compress, netio and netpkt")
    ap.add_argument("--seconds", type=int, default=60, help="per target (all run at the same time)")
    ap.add_argument("--only", default="", help="targets, comma-separated")
    ap.add_argument("--slow", type=float, default=2.0, help="a single call slower than this (s) is a finding")
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    exe = build()
    out = OUT / "out"
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir()
    only = [t for t in a.only.split(",") if t]
    cmd = [str(exe), str(a.seconds), str(seeds()), str(out)] + (only + [SELFTEST] if only else [])
    p = subprocess.run(cmd, capture_output=True, text=True)
    bad = 0
    runs = 0
    for ln in p.stdout.splitlines():
        f = ln.split(" ", 6)
        if f[0] == "target" and len(f) >= 7:
            name, n, nseeds, slow_us, slow_len, sites = f[1], int(f[2]), f[3], int(f[4]), f[5], int(f[6])
            flag = ""
            if name == SELFTEST:
                seen = sites > 0
                print(f"  {name:<16} {n:>12,} runs  " + ("its deliberate panic found" if seen else "FAIL: its deliberate panic was not found"))
                bad += 0 if seen else 1
                continue
            runs += n
            if sites:
                flag = f"  {sites} panic site(s)"
                bad += sites
            if slow_us > a.slow * 1e6:
                flag += f"  SLOW: {slow_us / 1e6:.1f} s for {slow_len} bytes ({name}-slowest.bin)"
                bad += 1
            print(f"  {name:<16} {n:>12,} runs  {nseeds:>4} seeds  slowest {slow_us / 1000:7.1f} ms{flag}")
        elif f[0] == "panic" and ln.split(" ")[1] != SELFTEST:
            print(f"      panic: {ln.split(' ', 3)[3]}  ({ln.split(' ')[2]}x)")
        elif f[0] == "worker-failed":
            print(f"  WORKER FAILED {ln.split(' ', 1)[1]} (a fault of the fuzzer, or a target without seeds)")
            bad += 1
        elif f[0] == "hang":
            print(f"  HANG {ln}")
            bad += 1
    if p.returncode not in (0, 2) or (p.returncode == 0 and runs == 0):
        sys.stderr.write(p.stdout + p.stderr)
        bad += 1
    if p.returncode == 2 and not any(ln.startswith("hang") for ln in p.stdout.splitlines()):
        bad += 1
    print(f"net_fuzz: {runs:,} runs, " + ("no finding" if not bad else f"{bad} finding(s), inputs in {out.relative_to(ROOT)}"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

"""cov_fuzz.py -- coverage-guided fuzzing (libFuzzer through cargo-fuzz) of the parsers of untrusted bytes.
    python tools/cov_fuzz.py [--seconds N] [--only T,..] [--jobs J]
The targets are tools/net_fuzz/targets.rs, the same as tools/net_fuzz.py's: zstd, LZ4, DEFLATE,
zlib, gzip, Brotli, JSON, XML, the charsets, X.509 (with netio's verify_chain and netpkt's
check_chain), both DNS and HTTP parsers, netio's TLS server and clients, IPv4 / IPv6 reassembly,
netpkt's dissector and application-layer parsers, netchan's packets, handshake cores and sealing,
plat's decoders of other programs' data (clipboard, drops; on Linux D-Bus and Wayland).
Writes a cargo-fuzz project into build/cov_fuzz (one fuzz target
per name, each including targets.rs), starts every target's corpus from the seeds of net_fuzz
(build/cov_fuzz/corpus/NAME, kept between runs so coverage adds up) and runs J targets at a time
for N seconds each (default 300) with debug assertions and overflow checks.
A crash, a timeout (10 s per input) or a memory blow-up (2 GiB) is a finding; libFuzzer leaves the
input in build/cov_fuzz/artifacts/NAME.  Needs the nightly toolchain and cargo-fuzz
(rustup toolchain install nightly; cargo install cargo-fuzz) and a C++ compiler; Linux or macOS.
Exit code 1 on any finding."""
import argparse
import concurrent.futures
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGETS_RS = ROOT / "tools" / "net_fuzz" / "targets.rs"
OUT = ROOT / "build" / "cov_fuzz"
sys.path.insert(0, str(ROOT / "tools"))
import net_fuzz  # noqa: E402

NAMES = ["zstd", "lz4", "deflate", "zlib", "gzip", "brotli", "json", "xml", "charsets", "x509", "x509_netpkt", "ocsp", "csr", "dns",
         "dns_netpkt", "http_netio", "http_netpkt", "tls_server", "tls_server_psk", "tls_client", "tls_webclient", "ipv4_reassembly",
         "ipv6_reassembly", "netpkt_decode", "netpkt_app", "netchan_packet", "netchan_server", "netchan_client",
         "netchan_open", "plat_clipboard"] + (["plat_dbus", "plat_wayland"] if sys.platform.startswith("linux") else [])


def project():
    lib = ROOT / "lib"
    dep = lambda name, crate, feats="": f'{name} = {{ package = "{crate}", path = "{(lib / name).as_posix()}"{feats} }}'  # noqa: E731
    bins = "\n".join(f'[[bin]]\nname = "{n}"\npath = "fuzz_targets/{n}.rs"\ntest = false\ndoc = false\nbench = false\n' for n in NAMES)
    (OUT / "Cargo.toml").write_text("\n".join([
        "# written by tools/cov_fuzz.py",
        "[package]", 'name = "cov-fuzz"', 'version = "0.0.0"', 'edition = "2024"', "publish = false", "",
        "[package.metadata]", "cargo-fuzz = true", "",
        "[workspace]", "",
        "[dependencies]",
        'libfuzzer-sys = "0.4"',
        dep("rt", "jflib-rt", ', features = ["std"]'),
        dep("plat", "jflib-plat", ', features = ["std"]'),
        dep("crypto", "jflib-crypto"),
        dep("compress", "jflib-compress"),
        dep("netpkt", "jflib-netpkt"),
        dep("netio", "jflib-netio"),
        dep("formats", "jflib-formats"),
        dep("netchan", "jflib-netchan"), "",
        bins,
    ]), encoding="utf-8")
    (OUT / "fuzz_targets").mkdir(parents=True, exist_ok=True)
    for n in NAMES:
        (OUT / "fuzz_targets" / f"{n}.rs").write_text(
            "// written by tools/cov_fuzz.py\n#![no_main]\n"
            f'#[path = "{TARGETS_RS.as_posix()}"]\nmod targets;\n'
            f'libfuzzer_sys::fuzz_target!(|d: &[u8]| (targets::get("{n}"))(d));\n', encoding="utf-8")


def run_one(name, seconds, seeds_dir):
    corpus = OUT / "corpus" / name
    art = OUT / "artifacts" / name
    art.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, NET_FUZZ_SEEDS=str(seeds_dir))
    cmd = ["cargo", "+nightly", "fuzz", "run", "--fuzz-dir", str(OUT), "--debug-assertions", name, str(corpus), "--",
           f"-max_total_time={seconds}", "-timeout=10", "-rss_limit_mb=2048", f"-artifact_prefix={art}/", "-print_final_stats=1"]
    p = subprocess.run(cmd, capture_output=True, text=True, env=env, cwd=ROOT)
    text = p.stdout + p.stderr
    cov = re.findall(r"cov: (\d+) ft: (\d+) corp: (\d+)", text)
    execs = re.findall(r"stat::number_of_executed_units:\s*(\d+)", text)
    found = sorted(f.name for f in art.iterdir() if f.name.startswith(("crash-", "timeout-", "oom-", "leak-", "slow-unit-")))
    return name, p.returncode, cov[-1] if cov else None, int(execs[-1]) if execs else 0, found, text[-3000:]


def main():
    ap = argparse.ArgumentParser(description="coverage-guided fuzzing with cargo-fuzz")
    ap.add_argument("--seconds", type=int, default=300, help="per target")
    ap.add_argument("--only", default="", help="targets, comma-separated: " + ", ".join(NAMES))
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) - 1), help="targets at a time")
    a = ap.parse_args()
    names = [n for n in a.only.split(",") if n] or NAMES
    for n in names:
        if n not in NAMES:
            sys.exit(f"cov_fuzz: unknown target {n}")
    if shutil.which("cargo-fuzz") is None or subprocess.run(["cargo", "+nightly", "--version"], capture_output=True).returncode:
        sys.exit("cov_fuzz: needs the nightly toolchain and cargo-fuzz (rustup toolchain install nightly; cargo install cargo-fuzz)")
    OUT.mkdir(parents=True, exist_ok=True)
    project()
    # the seeds of net_fuzz as the first corpus (only for a target whose corpus is still empty)
    (net_fuzz.OUT).mkdir(parents=True, exist_ok=True)
    exe = net_fuzz.build()
    seeds_dir = net_fuzz.seeds()
    dump = OUT / "seeds"
    shutil.rmtree(dump, ignore_errors=True)
    subprocess.run([str(exe), "--dump-seeds", str(seeds_dir), str(dump)], check=True)
    for n in names:
        corpus = OUT / "corpus" / n
        if not corpus.is_dir() or not any(corpus.iterdir()):
            shutil.copytree(dump / n, corpus, dirs_exist_ok=True)
    env = dict(os.environ, NET_FUZZ_SEEDS=str(seeds_dir))
    # all targets in one build (cargo fuzz build takes one name or none)
    p = subprocess.run(["cargo", "+nightly", "fuzz", "build", "--fuzz-dir", str(OUT), "--debug-assertions"], cwd=ROOT,
                       capture_output=True, text=True, env=env)
    if p.returncode:
        sys.stderr.write(p.stdout[-4000:] + p.stderr[-4000:])
        sys.exit("cov_fuzz: the fuzz targets did not build")
    bad = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for name, rc, cov, execs, found, tail in pool.map(lambda n: run_one(n, a.seconds, seeds_dir), names):
            files = len(list((OUT / "corpus" / name).iterdir()))
            line = f"  {name:<16} {execs:>12,} runs  corpus {files:>5}  " + (f"cov {cov[0]} ft {cov[1]}" if cov else "no coverage line")
            if found or rc not in (0,):
                bad += 1
                line += "  FINDING: " + (", ".join(found) if found else f"exit {rc}")
                if not found:
                    sys.stderr.write(tail)
            print(line, flush=True)
    print("cov_fuzz: " + ("no finding" if not bad else f"{bad} target(s) with findings, inputs in {(OUT / 'artifacts').relative_to(ROOT)}"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

"""net_fuzz.py -- mutation fuzzing of the parsers that read untrusted bytes in compress, formats, netchan,
netio, netpkt, plat and the image decoders img_jp2 and img_jbig2.
    python tools/net_fuzz.py [--seconds N] [--only T,..]
Builds tools/net_fuzz/fuzz.rs (with panic = "unwind", overflow checks and debug assertions) into
build/net_fuzz and runs every target on its own thread for N seconds (default 60):
  zstd, lz4                     decompress with an output limit
  deflate, zlib, gzip, brotli   decompress with an output limit (DEFLATE also into a small buffer,
                                and its InflateState after any input decodes a known stream as a
                                fresh one does; gzip with several members); Brotli's seeds are
                                the reference encoder's streams of coretest
  stream_deflate, stream_zlib, stream_gzip, stream_brotli, stream_zstd, stream_lz4
                                the stream decoders (compress::stream) fed in pieces of 1 byte to
                                5 KB with output buffers of 1 byte to 4 KB, in two schedules: the
                                same output as the one-shot decoder (or both refuse), and the
                                rules of a Step (read, written, Status) kept
  roundtrip                     compress() and the stream encoders (pieces, Sync flushes) of each
                                format and level: the one-shot decoder gives the data back, and
                                without Sync the encoder writes compress()'s bytes
  json, xml, charsets           formats: JSON with default and tight limits, the XML tree, the
                                strict Reader and the Lenient tokenizer with its entity decoding
                                (every XML reader of the library is one of these), the charsets
  netchan_packet                netchan's packet parser and writer
  netchan_server, netchan_client   netchan's handshake cores fed datagrams (seeds: a real UDP
                                handshake with fixed secrets, its sealed packets included)
  netchan_open                  ChaCha20-Poly1305 and packet-number reconstruction of a connection
  plat_clipboard, plat_dbus, plat_wayland   plat's decoders of other programs' data: clipboard
                                and drops (Windows' and Linux' formats), D-Bus messages, Wayland
                                events (docs/PLAT_INPUTS.md; plat::fuzz)
  x509, x509_netpkt             the certificate parser (the crate jflib-x509) with names,
                                fingerprint and verify_chain; netpkt's check_chain over it;
                                seeds: the system's CA bundle
  ocsp                          OCSP responses checked against a certificate and its CA (the
                                issuer's, a delegated responder's), requests read as a responder
  csr                           certificate requests (PKCS #10) of each key kind
  dns, dns_netpkt               netio's answer parser and netpkt's message parser
  http_netio, http_netpkt       netio's request reader over a stream in pieces, netpkt's heads
  http_client                   netio's client on a response stream: status lines, interim
                                answers, framing, Content-Encoding decoded, Set-Cookie into a jar
  url                           Url::parse (its text parses to the same URL), join, proxy URLs
                                and NO_PROXY lists
  cookie                        netpkt's cookie jar and the public suffix list
  tls_server                    netio's TLS server fed a mutated ClientHello
  tls_server_psk                ... with session tickets and 0-RTT, the ClientHello offering a
                                ticket (pre_shared_key, binders, early data behind it)
  tls_client, tls_webclient     netio's clients (own CA, web PKI) fed a mutated server flight
  ipv4_reassembly, ipv6_reassembly   netpkt's fragment reassembly fed mutated, reordered,
                                duplicated and overlapping fragments
  netpkt_decode, netpkt_app     the dissector, and SNMP, RADIUS, BGP, mail, SSH KEXINIT parsers
  img_jp2                       the JPEG 2000 decoder: img_all's codec (into ARGB) and a lower
                                resolution with the components as coded, under a cap of 2^20
                                pixels; seeds: pdftest's images, JP2 headers made around them
                                (palette, ICC, channel definitions, sYCC, CIELab, 64-bit boxes)
                                and small files of Pillow (OpenJPEG) when it is there
  img_jbig2                     the JBIG2 decoder: files (every page, and img_all's codec) and
                                PDF's embedded form with its globals (the input: their length in
                                two bytes, them, the page's segments), under a cap of 2^22 pixels;
                                seeds: every kind of segment, made in targets.rs
The seeds are built by the program itself, plus the system's certificates, frames of the
reference `zstd` (levels, --long, --ultra) when that tool is there and JPEG 2000 files of Pillow
(tiles, precincts, the progressions, layers, the 9/7 wavelet, 16 bits, PLT) when it is there.
No coverage feedback: this finds panics, hangs and slow inputs near the seeds, not deep paths
(cargo-fuzz would).
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
        dep("netio", "jflib-netio"),
        dep("formats", "jflib-formats"),
        dep("netchan", "jflib-netchan"),
        dep("img_core", "jflib-img-core"),
        dep("img_jp2", "jflib-img-jp2"),
        dep("img_jbig2", "jflib-img-jbig2"), "",
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


# small JPEG 2000 files of Pillow (OpenJPEG) for the seeds of img_jp2: (name, mode, size, save options),
# the options of tools/jp2_check.py's made suite at small sizes (OpenJPEG asserts on some others)
JP2_SEEDS = [
    ("tiles", "RGB", (40, 30), {"tile_size": (16, 16), "num_resolutions": 3}),
    ("lossy", "RGB", (48, 40), {"irreversible": True, "quality_layers": [30, 10], "quality_mode": "rates"}),
] + [("prog-" + p, "RGB", (33, 29), {"progression": p, "tile_size": (16, 24), "num_resolutions": 3, "precinct_size": (32, 32), "codeblock_size": (8, 8)})
     for p in ("LRCP", "RLCP", "RPCL", "PCRL", "CPRL")] + [
    ("gray16", "I;16", (24, 20), {}),
    ("la-97", "LA", (20, 20), {"irreversible": True}),
    ("rgba-cblk4", "RGBA", (40, 36), {"codeblock_size": (4, 4)}),
    ("offset", "RGB", (30, 26), {"offset": (5, 3), "tile_offset": (2, 1), "tile_size": (12, 10)}),
    ("plt", "L", (32, 32), {"plt": True}),
    ("layers-53", "RGB", (32, 24), {"quality_layers": [40, 10, 0], "quality_mode": "rates"}),
    ("no-mct", "RGB", (24, 24), {"mct": 0}),
    ("res6", "L", (64, 64), {"num_resolutions": 6}),
    ("tiny", "L", (1, 1), {"num_resolutions": 1}),
    ("raw", "L", (16, 16), {"no_jp2": True, "num_resolutions": 2}),
]


def jp2_seeds(d):
    """the files of JP2_SEEDS into d (pil-NAME.jp2, .j2k for a raw codestream), when Pillow is there"""
    try:
        from PIL import Image
    except ImportError:
        print("net_fuzz: no Pillow; img_jp2 starts from the library's own images only")
        return
    r = random.Random(11)
    for name, mode, (w, h), opt in JP2_SEEDS:
        top = 65535 if mode == "I;16" else 255
        nch = len(Image.new(mode, (1, 1)).getbands())
        # a gradient with some noise, shifted per band: code-blocks with data in every band
        px = []
        for y in range(h):
            for x in range(w):
                v = ((x * 7 + y * 5) * top // (w * 7 + h * 5) + r.randrange(0, top // 16)) % (top + 1)
                px.append(v if nch == 1 else tuple((v + 60 * c) % (top + 1) for c in range(nch)))
        im = Image.new(mode, (w, h))
        im.putdata(px)
        try:
            im.save(d / f"pil-{name}.{'j2k' if opt.get('no_jp2') else 'jp2'}", format="JPEG2000", **opt)
        except (OSError, ValueError) as e:
            print(f"net_fuzz: Pillow did not write the JPEG 2000 seed {name} ({e})")


def seeds():
    """build/net_fuzz/seeds: the system's certificates, the reference zstd's frames, (when fetched)
    google/brotli's test streams and (with Pillow) JPEG 2000 files"""
    d = OUT / "seeds"
    d.mkdir(parents=True, exist_ok=True)
    for b in CA_BUNDLES:
        if Path(b).is_file():
            shutil.copyfile(b, d / "ca.pem")
            break
    else:
        print("net_fuzz: no CA bundle found; x509 starts from the fuzzer's own certificates only")
    # google/brotli's test streams, where tools/brotli_check.py --vectors fetched them
    for f in sorted(ROOT.glob("build/vectors/brotli-*/tests/testdata/*.compressed*")):
        shutil.copyfile(f, d / f"{f.name}.br")
    jp2_seeds(d)
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

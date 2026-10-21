#!/usr/bin/env python3
"""pencheck.py -- (PN1) the Linux pen paths end to end, without a pen and without touching
the system's input: a protocol proxy inside WSL sits between a real app and WSLg's display
server and plays a pen tablet (docs/modules/PEN_INPUT.md).

  python tools/pencheck.py wayland [--exe build/pn1/linux/template]
      the proxy announces a zwp_tablet_manager_v2 global (WSLg's Weston has none), answers
      get_tablet_seat itself (the real server gets a harmless wl_display.sync for every id
      the client allocates for a fake object; their done / delete_id are filtered), adds a
      tablet, a pen (pressure + tilt) and an eraser tool and, once the window has drawn,
      a pen stroke (proximity, motion, pressure, tilt, down, barrel button, up, out) and an
      eraser touch on the app's first wl_surface.  The app runs with --log-events: every
      E_MOVE / E_DOWN / E_UP of the tool must carry the expected pen data (evq_pen_data).
  python tools/pencheck.py x11 [--exe ...]
      the proxy answers "Abs Pressure" / "Abs Tilt X|Y" atoms if the server has none,
      appends a pen and an eraser slave device to the XIQueryDevice reply, rewrites the
      app's XISelectEvents for the fake devices to a real slave pointer (the server checks
      the request), then sends XI_Motion / XI_ButtonPress / XI_ButtonRelease GenericEvents,
      each followed by the core event of the same time on the app's window.
  python tools/pencheck.py all
Exit code 0 = all checks passed.  Logs: build/pn1/pencheck_*.log
"""
import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DISTRO = os.environ.get("jfgui_WSL_DISTRO", "Ubuntu-24.04")
sys.path.insert(0, os.path.join(ROOT, "tools"))
import testenv  # noqa: E402,F401  (keeps PaintCraft away from the user's settings)

FAILS = []
PRESSURE_ONE = 65535


def check(ok, name, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (("  (" + detail + ")") if detail else ""))
    if not ok:
        FAILS.append(name)
    return ok


def pen_data(p, typ, tx=0, ty=0):
    """evq_pen_data: p = -1 none, else clamped 1..PRESSURE_ONE"""
    d = 0 if p < 0 else max(1, min(p, PRESSURE_ONE))
    tx = max(-63, min(63, tx)) & 127
    ty = max(-63, min(63, ty)) & 127
    return d | typ << 16 | tx << 18 | ty << 25


# ---------------------------------------------------------------- Wayland proxy (in WSL)
WL_PROXY = r'''
import array, os, select, socket, struct, sys, time
real, listen, logf, timeout = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
log = open(logf, "w")
def L(*a): print(*a, file=log, flush=True)
FAKE_NAME = 0x7FFF0001
TAB, PEN, ERA = 0xFF100000, 0xFF100001, 0xFF100002
def u32(x): return struct.pack("<I", x & 0xFFFFFFFF)
def fx(v): return struct.pack("<i", int(round(v * 256)))
def sstr(s):
    b = s.encode() + b"\0"
    return u32(len(b)) + b + b"\0" * ((4 - len(b) % 4) % 4)
def msg(oid, op, pl=b""): return struct.pack("<II", oid, (8 + len(pl)) << 16 | op) + pl
try: os.unlink(listen)
except FileNotFoundError: pass
lsock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
lsock.bind(listen); lsock.listen(1)
print("READY", flush=True)
lsock.settimeout(timeout)
try: c, _ = lsock.accept()
except socket.timeout: raise SystemExit(0)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect(real)
st = dict(reg=0, comp=0, mgr=0, tseat=0, surf=0, commits=0, stroke_at=0, sent=False)
subst = set(); fake = {TAB, PEN, ERA}
cbuf = b""; sbuf = b""; cfds = []; sfds = []
def rx(sock):
    fds = array.array("i")
    data, anc, flags, _ = sock.recvmsg(65536, socket.CMSG_SPACE(64 * 4))
    for lvl, typ, d in anc:
        if lvl == socket.SOL_SOCKET and typ == socket.SCM_RIGHTS:
            fds.frombytes(d[:len(d) - len(d) % 4])
    return data, list(fds)
def tx(sock, data, fds):
    anc = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", fds))] if fds else []
    sock.sendmsg([data], anc)
    for f in fds: os.close(f)
def to_client(b): tx(c, b, [])
def tool_setup(seat):
    out = msg(seat, 0, u32(TAB)) + msg(TAB, 0, sstr("pencheck tablet")) + msg(TAB, 3)
    out += msg(seat, 1, u32(PEN)) + msg(PEN, 0, u32(0x140)) + msg(PEN, 3, u32(2)) + msg(PEN, 3, u32(1)) + msg(PEN, 4)
    out += msg(seat, 1, u32(ERA)) + msg(ERA, 0, u32(0x141)) + msg(ERA, 3, u32(2)) + msg(ERA, 4)
    to_client(out); L("tools announced on seat", seat)
def stroke():
    surf, t = st["surf"], 1000
    def fr(tool):
        nonlocal t; t += 8; return msg(tool, 18, u32(t))
    ev = []
    ev.append(msg(PEN, 6, u32(50) + u32(TAB) + u32(surf)) + msg(PEN, 10, fx(100) + fx(60)) + msg(PEN, 11, u32(0)) + msg(PEN, 13, fx(10) + fx(-5)) + fr(PEN))
    ev.append(msg(PEN, 8, u32(51)) + msg(PEN, 11, u32(16384)) + fr(PEN))
    for i, p in enumerate((24576, 32768, 49152, 65535)):
        ev.append(msg(PEN, 10, fx(110 + 10 * i) + fx(60)) + msg(PEN, 11, u32(p)) + fr(PEN))
    ev.append(msg(PEN, 9) + fr(PEN))
    ev.append(msg(PEN, 17, u32(52) + u32(0x14B) + u32(1)) + msg(PEN, 8, u32(53)) + msg(PEN, 11, u32(40000)) + fr(PEN))
    ev.append(msg(PEN, 9) + msg(PEN, 17, u32(54) + u32(0x14B) + u32(0)) + fr(PEN))
    ev.append(msg(PEN, 7) + fr(PEN))
    ev.append(msg(ERA, 6, u32(55) + u32(TAB) + u32(surf)) + msg(ERA, 10, fx(80) + fx(40)) + msg(ERA, 11, u32(32768)) + fr(ERA))
    ev.append(msg(ERA, 8, u32(56)) + fr(ERA))
    ev.append(msg(ERA, 9) + fr(ERA))
    ev.append(msg(ERA, 7) + fr(ERA))
    for e in ev:
        to_client(e); time.sleep(0.05)
    L("stroke sent on surface", surf)
def client_msgs():
    global cbuf, cfds
    out = b""
    while len(cbuf) >= 8:
        oid, w = struct.unpack("<II", cbuf[:8]); size, op = w >> 16, w & 0xFFFF
        if size < 8 or len(cbuf) < size: break
        m, cbuf = cbuf[:size], cbuf[size:]
        a = m[8:]
        if oid == 1 and op == 1:
            st["reg"] = struct.unpack("<I", a[:4])[0]
            out += m
            tx(s, out, cfds); cfds = []; out = b""
            to_client(msg(st["reg"], 0, u32(FAKE_NAME) + sstr("zwp_tablet_manager_v2") + u32(1)))
            L("global announced on registry", st["reg"]); continue
        if oid == st["reg"] and op == 0:
            name, n = struct.unpack("<II", a[:8]); iface = a[8:8 + n - 1].decode()
            k = 8 + n + (4 - n % 4) % 4; ver, nid = struct.unpack("<II", a[k:k + 8])
            if name == FAKE_NAME:
                st["mgr"] = nid; subst.add(nid); out += msg(1, 0, u32(nid))
                L("bind", iface, "version", ver, "id", nid); continue
            if iface == "wl_compositor": st["comp"] = nid
        elif oid == st["comp"] and op == 0 and st["comp"]:
            nid = struct.unpack("<I", a[:4])[0]
            if not st["surf"]: st["surf"] = nid
        elif oid == st["mgr"] and st["mgr"]:
            if op == 0:
                nid = struct.unpack("<I", a[:4])[0]; st["tseat"] = nid; subst.add(nid)
                out += msg(1, 0, u32(nid)); tx(s, out, cfds); cfds = []; out = b""
                L("get_tablet_seat", nid, "seat", struct.unpack("<I", a[4:8])[0]); tool_setup(nid)
            else: L("manager request", op)
            continue
        elif oid == st["tseat"] or oid in fake:
            L("request to fake object", hex(oid), "op", op); continue
        elif oid == st["surf"] and op == 6:
            st["commits"] += 1
            if st["commits"] == 2: st["stroke_at"] = time.time() + 1.0
        out += m
    if out or cfds:
        tx(s, out, cfds); cfds = []
def server_msgs():
    global sbuf, sfds
    out = b""
    while len(sbuf) >= 8:
        oid, w = struct.unpack("<II", sbuf[:8]); size, op = w >> 16, w & 0xFFFF
        if size < 8 or len(sbuf) < size: break
        m, sbuf = sbuf[:size], sbuf[size:]
        if oid in subst: continue                                   # callback.done
        if oid == 1 and op == 1 and struct.unpack("<I", m[8:12])[0] in subst: continue
        out += m
    if out or sfds:
        tx(c, out, sfds); sfds = []
end = time.time() + timeout
while time.time() < end:
    r, _, _ = select.select([c, s], [], [], 0.05)
    for a in r:
        data, fds = rx(a)
        if not data and not fds: L("closed"); raise SystemExit(0)
        if a is c: cbuf += data; cfds += fds; client_msgs()
        else: sbuf += data; sfds += fds; server_msgs()
    if st["stroke_at"] and not st["sent"] and time.time() >= st["stroke_at"]:
        st["sent"] = True; stroke()
'''

# ---------------------------------------------------------------- X11 proxy (in WSL)
X11_PROXY = r'''
import os, select, socket, struct, sys, time
listen, logf, timeout = sys.argv[1], sys.argv[2], float(sys.argv[3])
log = open(logf, "w")
def L(*a): print(*a, file=log, flush=True)
PEN_ID, ERA_ID = 99, 98
FAKE_ATOMS = {b"Abs Pressure": 0x1F000001, b"Abs Tilt X": 0x1F000002, b"Abs Tilt Y": 0x1F000003}
def pad4(n): return (n + 3) & ~3
def recvn(sock, n):
    b = b""
    while len(b) < n:
        x = sock.recv(n - len(b))
        if not x: raise SystemExit(0)
        b += x
    return b
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind("\0" + listen); srv.listen(1)
print("READY", flush=True)
srv.settimeout(timeout)
try: c, _ = srv.accept()
except socket.timeout: raise SystemExit(0)
hdr = recvn(c, 12)
n, d = struct.unpack("<HH", hdr[6:10]); recvn(c, pad4(n) + pad4(d))
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect("/tmp/.X11-unix/X0")
s.sendall(hdr[:6] + b"\0\0\0\0" + hdr[10:12])
sh = recvn(s, 8); body = recvn(s, struct.unpack("<H", sh[6:8])[0] * 4); c.sendall(sh + body)
root = 0                                    # not needed: the app reads only the event window
st = dict(seq=0, xi=0, win=0, slaves=[], inject_at=0, done=False, atoms={})
pend = {}
cbuf = b""; sbuf = b""
def valuator(src, num, label, lo, hi):
    return struct.pack("<HHHHI", 2, 11, src, num, label) + struct.pack("<iIiIiIIB3x", lo, 0, hi, 0, 0, 0, 0, 0)
def device(did, name, labels):
    nm = name.encode(); classes = b"".join(valuator(did, i, lab, lo, hi) for i, (lab, lo, hi) in enumerate(labels))
    return struct.pack("<HHHHHBB", did, 3, 2, len(labels), len(nm), 1, 0) + nm + b"\0" * (pad4(len(nm)) - len(nm)) + classes
def fix_reply(p):
    seq = struct.unpack("<H", p[2:4])[0]; what = pend.pop(seq, None)
    if what is None: return p
    if what == "xiext":
        if p[8]: st["xi"] = p[9]; L("XInputExtension op", p[9])
    elif isinstance(what, tuple) and what[0] == "atom":
        a = struct.unpack("<I", p[8:12])[0]
        if a == 0:
            a = FAKE_ATOMS[what[1]]; p = p[:8] + struct.pack("<I", a) + p[12:]
        st["atoms"][what[1]] = a; L("atom", what[1], hex(a))
    elif what == "qdev":
        num = struct.unpack("<H", p[8:10])[0]; off = 32
        for i in range(num):
            did, use, att, ncl, nl = struct.unpack("<HHHHH", p[off:off + 10]); q = off + 12 + pad4(nl)
            for k in range(ncl): q += struct.unpack("<H", p[q + 2:q + 4])[0] * 4
            if use == 3: st["slaves"].append(did)
            off = q
        A = st["atoms"]
        extra = device(PEN_ID, "pencheck stylus", [(A[b"Abs Pressure"], 0, 1023), (A[b"Abs Tilt X"], -64, 63), (A[b"Abs Tilt Y"], -64, 63)])
        extra += device(ERA_ID, "pencheck eraser", [(A[b"Abs Pressure"], 0, 1023)])
        body = p[32:off] + extra + p[off:]       # after the last device (the server may pad)
        p = p[:4] + struct.pack("<IH", len(body) // 4, num + 2) + p[10:32] + body
        L("XIQueryDevice: %u devices, real slave pointers %s, 2 fake pens added" % (num, st["slaves"]))
        if os.environ.get("PENCHECK_DUMP"): L("REPLY " + p.hex())
    return p
def client_reqs():
    global cbuf
    out = b""
    while len(cbuf) >= 4:
        op, data, ln = struct.unpack("<BBH", cbuf[:4]); size = ln * 4
        if ln == 0:
            if len(cbuf) < 8: break
            size = struct.unpack("<I", cbuf[4:8])[0] * 4
        if size < 4 or len(cbuf) < size: break
        r, cbuf = cbuf[:size], cbuf[size:]
        st["seq"] = (st["seq"] + 1) & 0xFFFF
        if op == 98:
            nl = struct.unpack("<H", r[4:6])[0]
            if r[8:8 + nl] == b"XInputExtension": pend[st["seq"]] = "xiext"
        elif op == 16:
            nl = struct.unpack("<H", r[4:6])[0]; nm = r[8:8 + nl]
            if nm in FAKE_ATOMS: pend[st["seq"]] = ("atom", nm)
        elif st["xi"] and op == st["xi"] and data == 48:
            pend[st["seq"]] = "qdev"
        elif st["xi"] and op == st["xi"] and data == 46:
            win, nm = struct.unpack("<IH", r[4:10]); k = 12; b = bytearray(r)
            for i in range(nm):
                did, ml = struct.unpack("<HH", r[k:k + 4])
                if did in (PEN_ID, ERA_ID) and st["slaves"]:
                    real = st["slaves"][min(i, len(st["slaves"]) - 1)]
                    struct.pack_into("<H", b, k, real)
                    if win != root and not st["win"]:
                        st["win"] = win; st["inject_at"] = time.time() + 1.5
                    L("XISelectEvents window %x: device %u -> real slave %u, mask %x" % (win, did, real, struct.unpack("<I", r[k + 4:k + 8])[0]))
                k += 4 + ml * 4
            r = bytes(b)
        out += r
    if out: s.sendall(out)
def server_pkts():
    global sbuf
    out = b""
    while len(sbuf) >= 32:
        t = sbuf[0]; size = 32
        if t == 1 or (t & 0x7F) == 35: size = 32 + struct.unpack("<I", sbuf[4:8])[0] * 4
        if len(sbuf) < size: break
        p, sbuf = sbuf[:size], sbuf[size:]
        if t == 1: p = fix_reply(p)
        out += p
    if out: c.sendall(out)
def xi_ev(evtype, dev, tm, detail, vals):
    mask = 0; v = b""
    for num, val in sorted(vals.items()):
        mask |= 1 << num; v += struct.pack("<iI", val, 0)
    body = struct.pack("<HHIIIII", evtype, dev, tm, detail, root, st["win"], 0) + struct.pack("<iiii", 0, 0, 0, 0)
    body += struct.pack("<HHHHI", 0, 1, dev, 0, 0) + b"\0" * 20 + struct.pack("<I", mask) + v
    return struct.pack("<BBHI", 35, st["xi"], st["seq"], (len(body) + 8 - 32) // 4) + body
def core(typ, detail, tm, x, y, state):
    return struct.pack("<BBHIIIIhhhhHBx", typ, detail, st["seq"], tm, root, st["win"], 0, x, y, x, y, state, 1)
def inject():
    tm = 500000; ev = []
    def pair(xt, ct, det, x, y, state, dev, vals):
        nonlocal tm; tm += 10
        ev.append(xi_ev(xt, dev, tm, det, vals) + core(ct, det, tm, x, y, state))
    pair(6, 6, 0, 30, 40, 0, PEN_ID, {0: 0, 1: 10, 2: -5})
    pair(4, 4, 1, 30, 40, 0, PEN_ID, {0: 256})
    for i, v in enumerate((384, 512, 768, 1023)):
        pair(6, 6, 0, 40 + 10 * i, 40, 0x100, PEN_ID, {0: v})
    pair(5, 5, 1, 70, 40, 0x100, PEN_ID, {0: 0})
    tm += 10; ev.append(core(6, 0, tm, 71, 41, 0))            # a mouse motion in between
    pair(4, 4, 1, 20, 20, 0, ERA_ID, {0: 512})
    pair(5, 5, 1, 20, 20, 0x100, ERA_ID, {})
    for e in ev:
        c.sendall(e); time.sleep(0.05)
    L("events sent to window %x" % st["win"])
end = time.time() + timeout
while time.time() < end:
    r, _, _ = select.select([c, s], [], [], 0.05)
    for a in r:
        b = a.recv(65536)
        if not b: L("closed"); raise SystemExit(0)
        if a is c: cbuf += b; client_reqs()
        else: sbuf += b; server_pkts()
    if st["inject_at"] and not st["done"] and time.time() >= st["inject_at"]:
        st["done"] = True; inject()
'''


def events(text):
    """--log-events lines -> [(type, btn, x, y, data)]"""
    out = []
    for ln in text.splitlines():
        if not ln.startswith("event type="):
            continue
        f = dict(kv.split("=", 1) for kv in ln.split()[1:])
        out.append((int(f["type"]), int(f["btn"]), int(f["x"]), int(f["y"]), int(f["data"], 16)))
    return out


def run(kind, exe, outdir):
    os.makedirs(os.path.join(ROOT, outdir), exist_ok=True)
    pfile = f"{outdir}/pencheck_{kind}_proxy.py"
    with open(os.path.join(ROOT, pfile), "w", encoding="utf-8", newline="\n") as f:
        f.write(WL_PROXY if kind == "wayland" else X11_PROXY)
    plog = f"{outdir}/pencheck_{kind}_proxy.log"
    alog = os.path.join(ROOT, outdir, f"pencheck_{kind}_app.log")
    if kind == "wayland":
        sock = "/run/user/1000/wayland-pencheck"
        pargs = ["/mnt/wslg/runtime-dir/wayland-0", sock, plog, "20"]
        env = "jfgui_BACKEND=wayland WAYLAND_DISPLAY=wayland-pencheck"
    else:
        pargs = ["/tmp/.X11-unix/X61", plog, "20"]
        env = "jfgui_BACKEND=x11 jfgui_X11DEBUG=1 DISPLAY=:61"
    proxy = subprocess.Popen(["wsl.exe", "-d", DISTRO, "--cd", ROOT, "-e", "python3", pfile] + pargs,
                             stdout=subprocess.PIPE, text=True)
    proxy.stdout.readline()
    exe = exe.replace("\\", "/")
    r = subprocess.run(["wsl.exe", "-d", DISTRO, "--cd", ROOT, "-e", "sh", "-c",
                        f"exec env {env} ./{exe} --config {outdir}/cfg --log-events --auto-close 6000 2>&1"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
    open(alog, "w", encoding="utf-8").write(r.stdout)
    try:
        proxy.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proxy.kill()
    try:
        plog_text = open(os.path.join(ROOT, plog), encoding="utf-8").read()
    except OSError:
        plog_text = ""
    print(f"--- {kind}: proxy log\n" + plog_text.rstrip())
    check(r.returncode == 0, f"{kind}: app ran and closed (exit 0)", f"exit {r.returncode}")
    return [e for e in events(r.stdout) if e[0] in (1, 2, 3, 12)], plog_text


def expect_seq(kind, got, want, scale_from=None):
    """want = [(type, btn, xs, ys, data)]; positions are compared after scaling by the first
    pen event's x (the window scale is not known here)"""
    pen = got
    ok = len(pen) >= len(want)
    k = 1.0
    if pen and want and want[0][2]:
        k = pen[0][2] / want[0][2]
    detail = ""
    for i, w in enumerate(want):
        if i >= len(pen):
            ok = False
            detail = f"missing event {i} {w}"
            break
        g = pen[i]
        if (g[0], g[1], g[4]) != (w[0], w[1], w[4]) or abs(g[2] - round(w[2] * k)) > 1 or abs(g[3] - round(w[3] * k)) > 1:
            ok = False
            detail = f"event {i}: got {g[:4]} data {g[4]:x}, want {w[:4]} data {w[4]:x}"
            break
    return check(ok, f"{kind}: pen events through the real protocol path ({len(want)})", detail)


def check_wayland(a):
    got, plog = run("wayland", a.exe, a.out_dir)
    check("bind zwp_tablet_manager_v2 version 1" in plog, "wayland: the app binds the tablet manager")
    check("get_tablet_seat" in plog, "wayland: get_tablet_seat on our seat")
    check("stroke sent" in plog, "wayland: the window drew, stroke sent")
    P, E = 1, 2
    want = [(1, 0, 100, 60, pen_data(0, P, 10, -5)),
            (2, 1, 100, 60, pen_data(16384, P, 10, -5))]
    for i, p in enumerate((24576, 32768, 49152, 65535)):
        want.append((1, 0, 110 + 10 * i, 60, pen_data(p, P, 10, -5)))
    want.append((3, 1, 140, 60, pen_data(65535, P, 10, -5)))
    want.append((2, 2, 140, 60, pen_data(40000, P, 10, -5)))
    want.append((3, 2, 140, 60, pen_data(40000, P, 10, -5)))
    want.append((12, 0, 0, 0, 0))
    want.append((1, 0, 80, 40, pen_data(32768, E)))
    want.append((2, 1, 80, 40, pen_data(32768, E)))
    want.append((3, 1, 80, 40, pen_data(32768, E)))
    # E_LEAVE has no data: keep it in the comparison through the type filter
    got = [g for g in got if g[4] != 0 or g[0] in (3, 12) or (g[0] == 2 and g[4] != 0)]
    expect_seq("wayland", got, want)


def check_x11(a):
    got, plog = run("x11", a.exe, a.out_dir)
    check("2 fake pens added" in plog, "x11: XIQueryDevice answered (2 fake pens added)")
    check("XISelectEvents window" in plog, "x11: the app selects the pens' events on its window")
    check("events sent" in plog, "x11: events sent")
    P, E = 1, 2
    v = lambda x: x * PRESSURE_ONE // 1023
    want = [(1, 0, 30, 40, pen_data(v(0), P, 10, -5)),
            (2, 1, 30, 40, pen_data(v(256), P, 10, -5))]
    for i, x in enumerate((384, 512, 768, 1023)):
        want.append((1, 0, 40 + 10 * i, 40, pen_data(v(x), P, 10, -5)))
    want.append((3, 1, 70, 40, pen_data(v(0), P, 10, -5)))
    want.append((2, 1, 20, 20, pen_data(v(512), E)))
    want.append((3, 1, 20, 20, pen_data(v(512), E)))
    mouse = [g for g in got if g[0] == 1 and (g[2], g[3]) == (71, 41)]
    check(len(mouse) == 1 and mouse[0][4] == 0, "x11: a core motion without an XI event stays a mouse event")
    got = [g for g in got if g[0] in (1, 2, 3) and g[4] != 0 or g[0] == 3]
    expect_seq("x11", got, want)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("what", choices=["wayland", "x11", "all"])
    ap.add_argument("--exe", default="build/pn1/linux/template")
    ap.add_argument("--out-dir", default="build/pn1")
    a = ap.parse_args()
    if a.what in ("wayland", "all"):
        check_wayland(a)
    if a.what in ("x11", "all"):
        check_x11(a)
    print("pencheck:", "all passed" if not FAILS else f"{len(FAILS)} failed")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()

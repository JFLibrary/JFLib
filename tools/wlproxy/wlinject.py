#!/usr/bin/env python3
# wlinject.py -- a Wayland protocol proxy (runs inside WSL) between one client and WSLg's
# compositor that plays the input devices and the other clipboard clients, so the same
# synthetic session can be replayed against the assembly and the Rust build:
#
#   python3 wlinject.py REAL_SOCKET LISTEN_SOCKET LOG TIMEOUT SCENARIO FRAME_OUT
#
# Real pointer / keyboard / data-device events for the client are dropped (the user's mouse
# and clipboard never reach the app); the client's set_selection never reaches the
# compositor (the system clipboard stays untouched).  Every commit of the client's first
# surface snapshots the attached shm buffer; the last one is written to FRAME_OUT (u32 w, u32
# h, then w * h * 4 bytes BGRX).
# Scenarios: input (pointer + keyboard), clip (paste a fake offer, then read our selection
# as another client), drop (text/uri-list drag and drop).
import array, mmap, os, select, socket, struct, sys, time

real, listen, logf, timeout, scen, frame_out = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4]), sys.argv[5], sys.argv[6]
log = open(logf, "w")


def L(*a):
    print(*a, file=log, flush=True)


def u32(x): return struct.pack("<I", x & 0xFFFFFFFF)
def i32(x): return struct.pack("<i", x)
def fx(v): return struct.pack("<i", int(round(v * 256)))


def sstr(s):
    b = (s.encode() if isinstance(s, str) else s) + b"\0"
    return u32(len(b)) + b + b"\0" * ((4 - len(b) % 4) % 4)


def arr(b):
    return u32(len(b)) + b + b"\0" * ((4 - len(b) % 4) % 4)


def msg(oid, op, pl=b""): return struct.pack("<II", oid, (8 + len(pl)) << 16 | op) + pl


def rstr(a, o):
    n = struct.unpack_from("<I", a, o)[0]
    s = a[o + 4:o + 4 + n - 1] if n else None
    return s, o + 4 + ((n + 3) & ~3)


try:
    os.unlink(listen)
except FileNotFoundError:
    pass
lsock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
lsock.bind(listen)
lsock.listen(1)
print("READY", flush=True)
lsock.settimeout(timeout)
try:
    c, _ = lsock.accept()
except socket.timeout:
    raise SystemExit(0)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(real)

st = dict(reg=0, comp=0, seat=0, shm=0, ddm=0, ptr=0, kb=0, dd=0, surf=0, commits=0, go_at=0, done=False,
          src=0, attached=0)
names = {}       # registry name -> interface
pools = {}       # pool id -> mmap
bufs = {}        # buffer id -> (pool, offset, w, h, stride)
fake_offers = {} # fake offer id -> {mime: data}
drop_ids = set() # real server objects whose events are dropped (data offers)
xs2surf = {}     # xdg_surface -> wl_surface
src_mimes = {}   # client data source -> offered MIME types
recvs = {}       # fake offer -> receive requests served
PNG = bytes([0x89]) + b"PNG fake image data for the clipboard test"
BMP = bytes([66, 77, 70, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0,
             16, 0, 0, 0, 19, 11, 0, 0, 19, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 20, 30, 0, 40, 50, 60, 0, 70, 80, 90, 0,
             100, 110, 120, 0])


def fnv(d):
    h = 0xCBF29CE484222325
    for b in d:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h
last = None      # last committed frame (w, h, bytes)
cbuf = b""
sbuf = b""
cfds = []
sfds = []
FAKE = [0xFF200000]
serial = [5000]
tms = [100000]


def ser():
    serial[0] += 1
    return serial[0]


def tm():
    tms[0] += 7
    return tms[0]


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
    for f in fds:
        os.close(f)


def to_client(b, fds=()):
    tx(c, b, list(fds))


def snapshot():
    global last
    b = bufs.get(st["attached"])
    if not b:
        return
    pool, off, w, h, stride = b
    m = pools.get(pool)
    if m is None:
        return
    rows = []
    for y in range(h):
        rows.append(m[off + y * stride: off + y * stride + w * 4])
    last = (w, h, b"".join(rows))


def new_offer(mimes):
    oid = FAKE[0]
    FAKE[0] += 1
    fake_offers[oid] = mimes
    out = msg(st["dd"], 0, u32(oid))
    for mt in mimes:
        out += msg(oid, 0, sstr(mt))
    return oid, out


def scenario():
    surf, p, k, dd = st["surf"], st["ptr"], st["kb"], st["dd"]
    L("scenario", scen, "surface", surf, "pointer", p, "keyboard", k, "data device", dd)
    if scen == "input":
        seq = []
        seq.append(msg(p, 0, u32(ser()) + u32(surf) + fx(100.5) + fx(60)) + msg(p, 5))
        seq.append(msg(p, 2, u32(tm()) + fx(120) + fx(70.75)) + msg(p, 5))
        seq.append(msg(p, 3, u32(ser()) + u32(tm()) + u32(0x110) + u32(1)) + msg(p, 5))
        seq.append(msg(p, 2, u32(tm()) + fx(130) + fx(-5)) + msg(p, 5))
        seq.append(msg(p, 3, u32(ser()) + u32(tm()) + u32(0x110) + u32(0)) + msg(p, 5))
        seq.append(msg(p, 4, u32(tm()) + u32(0) + fx(10)) + msg(p, 8, u32(0) + i32(1)) + msg(p, 5))
        seq.append(msg(p, 4, u32(tm()) + u32(1) + fx(-7.5)) + msg(p, 5))
        seq.append(msg(p, 3, u32(ser()) + u32(tm()) + u32(0x111) + u32(1)) + msg(p, 5))
        seq.append(msg(p, 3, u32(ser()) + u32(tm()) + u32(0x116) + u32(1)) + msg(p, 5))
        seq.append(msg(p, 1, u32(ser()) + u32(surf)) + msg(p, 5))
        seq.append(msg(k, 1, u32(ser()) + u32(surf) + arr(b"")))
        seq.append(msg(k, 4, u32(ser()) + u32(0) + u32(0) + u32(0) + u32(0)))
        for key, dep in ((16, 0), (16, 1), (18, 0x80), (40, 0), (40, 1), (8, 0x80), (12, 0), (21, 0), (28, 0),
                         (105, 0), (71, 0), (2, 0x04)):
            seq.append(msg(k, 4, u32(ser()) + u32(dep) + u32(0) + u32(0x10 if key == 71 else 0) + u32(0)))
            seq.append(msg(k, 3, u32(ser()) + u32(tm()) + u32(key) + u32(1)))
            seq.append(msg(k, 3, u32(ser()) + u32(tm()) + u32(key) + u32(0)))
        seq.append(msg(k, 4, u32(ser()) + u32(0) + u32(0) + u32(0) + u32(0)))
        seq.append(msg(k, 3, u32(ser()) + u32(tm()) + u32(42) + u32(1)))   # Shift key itself
        seq.append(msg(k, 4, u32(ser()) + u32(1) + u32(0) + u32(0) + u32(0)))
        seq.append(msg(k, 3, u32(ser()) + u32(tm()) + u32(42) + u32(0)))
        seq.append(msg(k, 4, u32(ser()) + u32(0) + u32(0) + u32(0) + u32(0)))
        seq.append(msg(k, 2, u32(ser()) + u32(surf)))
        for e in seq:
            to_client(e)
            time.sleep(0.03)
    elif scen == "grab":
        seq = [msg(p, 0, u32(ser()) + u32(surf) + fx(50) + fx(60)) + msg(p, 5)]
        seq.append(msg(p, 3, u32(ser()) + u32(tm()) + u32(0x110) + u32(1)) + msg(p, 5))
        seq.append(msg(p, 3, u32(ser()) + u32(tm()) + u32(0x110) + u32(0)) + msg(p, 5))
        seq.append(msg(p, 2, u32(tm()) + fx(500) + fx(300)) + msg(p, 5))
        seq.append(msg(p, 3, u32(ser()) + u32(tm()) + u32(0x110) + u32(1)) + msg(p, 5))
        seq.append(msg(p, 3, u32(ser()) + u32(tm()) + u32(0x111) + u32(1)) + msg(p, 5))
        seq.append(msg(p, 1, u32(ser()) + u32(surf)) + msg(p, 5))
        for e in seq:
            to_client(e)
            time.sleep(0.05)
    elif scen == "bad1":
        to_client(struct.pack("<II", 1, (6 << 16) | 0) + b"\x00\x00\x00\x00")
    elif scen == "bad2":
        to_client(msg(k, 0, u32(1) + u32(100)))                     # keymap without its fd
    elif scen == "bad3":
        to_client(msg(st["seat"], 1, u32(4) + b"abcd"))             # name: no NUL
    elif scen == "bad4":
        to_client(msg(999, 0, u32(1)) + msg(p, 0, u32(ser()) + u32(surf) + fx(3) + fx(4)) + msg(p, 5))
    elif scen == "bad5":
        to_client(msg(st["seat"], 1, u32(40) + b"abcd"))            # string longer than the message
    elif scen == "clipimg":
        oid, out = new_offer({"image/png": PNG, "image/bmp": BMP, "application/x-other": b"?"})
        st["offerA"] = oid
        out += msg(dd, 5, u32(oid))
        out += msg(k, 1, u32(ser()) + u32(surf) + arr(b"")) + msg(k, 4, u32(ser()) + u32(0) + u32(0) + u32(0) + u32(0))
        to_client(out)
        L("fake image offer", hex(oid))
    elif scen == "clip":
        oid, out = new_offer({"text/plain;charset=utf-8": "Fake offer: Grüße € ✓ from the proxy".encode(),
                              "UTF8_STRING": b"wrong mime", "TEXT": b"wrong mime"})
        out += msg(dd, 5, u32(oid))
        out += msg(k, 1, u32(ser()) + u32(surf) + arr(b"")) + msg(k, 4, u32(ser()) + u32(0) + u32(0) + u32(0) + u32(0))
        to_client(out)
        L("fake selection offer", hex(oid))
    elif scen == "drop":
        oid, out = new_offer({"text/uri-list": b"# a comment\r\nfile:///tmp/a%20b.txt\r\nfile://localhost/etc/hostname\r\n"
                                                 b"http://example.com/x\r\nfile:///home/u/%C3%BC.png"})
        out += msg(dd, 1, u32(ser()) + u32(surf) + fx(50.5) + fx(40) + u32(oid))
        to_client(out)
        time.sleep(0.1)
        to_client(msg(oid, 1, u32(1)) + msg(oid, 2, u32(1)))     # source_actions, action copy
        time.sleep(0.05)
        to_client(msg(dd, 3, u32(tm()) + fx(60) + fx(45.25)))
        time.sleep(0.05)
        to_client(msg(dd, 4))
        time.sleep(0.3)
        to_client(msg(dd, 2))
        L("drop sent")


def selection_read(src):
    # another client pastes: wl_data_source.send(mime, fd) with a pipe, every offered type
    for mt in src_mimes.get(src, []):
        r, w = os.pipe()
        to_client(msg(src, 1, sstr(mt)), [w])
        data = b""
        end = time.time() + 3
        while time.time() < end:
            rr, _, _ = select.select([r, c, s], [], [], 0.05)
            if r in rr:
                d = os.read(r, 65536)
                if not d:
                    break
                data += d
            pump(rr)
        os.close(r)
        if mt.startswith("image/"):
            L("SOURCE DATA", mt, len(data), "bytes, fnv", hex(fnv(data)), "png" if data == PNG else "bmp" if data == BMP else "?")
        else:
            L("SOURCE DATA", mt, repr(data.decode("utf-8", "replace")))


def client_msgs():
    global cbuf, cfds
    out = b""
    ofds = []
    while len(cbuf) >= 8:
        oid, w = struct.unpack("<II", cbuf[:8])
        size, op = w >> 16, w & 0xFFFF
        if size < 8 or len(cbuf) < size:
            break
        m, cbuf = cbuf[:size], cbuf[size:]
        a = m[8:]
        if oid in fake_offers:
            if op == 1:   # receive(mime, fd)
                mt, _ = rstr(a, 0)
                fd = cfds.pop(0)
                data = fake_offers[oid].get(mt.decode(), b"")
                os.write(fd, data)
                os.close(fd)
                L("offer receive", mt.decode(), len(data), "bytes")
                recvs[oid] = recvs.get(oid, 0) + 1
                if scen == "clipimg" and oid == st.get("offerA") and recvs[oid] == 2:
                    ob, out2 = new_offer({"image/bmp": BMP})
                    to_client(out2 + msg(st["dd"], 5, u32(ob)))
                    L("fake bmp offer", hex(ob))
            else:
                L("offer request", hex(oid), "op", op, a.hex())
            continue
        if oid == 1 and op == 1:
            st["reg"] = struct.unpack("<I", a[:4])[0]
        elif oid == st["reg"] and op == 0 and st["reg"]:
            name = struct.unpack("<I", a[:4])[0]
            iface, o = rstr(a, 4)
            ver, nid = struct.unpack_from("<II", a, o)
            iface = iface.decode()
            if iface == "wl_compositor":
                st["comp"] = nid
            elif iface == "wl_seat":
                st["seat"] = nid
            elif iface == "wl_shm":
                st["shm"] = nid
            elif iface == "wl_data_device_manager":
                st["ddm"] = nid
            elif iface == "xdg_wm_base":
                st["wm"] = nid
        elif oid == st["comp"] and op == 0 and st["comp"]:
            nid = struct.unpack("<I", a[:4])[0]
            if not st["surf"]:
                st["surf"] = nid
        elif oid == st["seat"] and st["seat"]:
            nid = struct.unpack("<I", a[:4])[0]
            if op == 0:
                st["ptr"] = nid
            elif op == 1:
                st["kb"] = nid
        elif oid == st.get("wm") and op == 2:
            xs, sf = struct.unpack_from("<II", a, 0)
            xs2surf[xs] = sf
            if not st.get("xs"):
                st["xs"] = xs
        elif oid in xs2surf and op == 1:
            tp = struct.unpack("<I", a[:4])[0]
            if not st.get("top"):
                st["top"] = tp
            elif scen == "popup" and not st.get("toolsurf"):
                st["toolsurf"] = xs2surf[oid]
                st["tool_at"] = time.time() + 0.5
                L("tool window surface", xs2surf[oid])
        elif oid in xs2surf and op == 2 and scen == "popup":
            st["popsurf"] = xs2surf[oid]
            st["pop_at"] = time.time() + 0.3
            L("popup surface", xs2surf[oid])
        elif oid == st.get("top") and op in (5, 6):    # move / resize: never to the compositor
            L("toplevel", "move" if op == 5 else "resize", a.hex())
            continue
        elif oid == st["ddm"] and st["ddm"] and op == 0:
            src_mimes[struct.unpack("<I", a[:4])[0]] = []
        elif oid in src_mimes and op == 0:
            mt, _ = rstr(a, 0)
            src_mimes[oid].append(mt.decode())
        elif oid == st["ddm"] and st["ddm"] and op == 1:
            st["dd"] = struct.unpack("<I", a[:4])[0]
        elif oid == st["shm"] and st["shm"] and op == 0:
            nid, size = struct.unpack_from("<Ii", a, 0)
            fd = cfds.pop(0)
            pools[nid] = mmap.mmap(os.dup(fd), size, mmap.MAP_SHARED, mmap.PROT_READ)
            ofds.append(fd)
        elif oid in pools and op == 0:
            nid, off, w, h, stride, fmt = struct.unpack_from("<IiiiiI", a, 0)
            bufs[nid] = (oid, off, w, h, stride)
        elif oid == st["dd"] and st["dd"] and op == 1:   # set_selection: never to the compositor
            src, sr = struct.unpack_from("<II", a, 0)
            L("set_selection source", src)
            if src:
                st["src"] = src
            continue
        elif oid == st["surf"] and st["surf"]:
            if op == 1:
                st["attached"] = struct.unpack("<I", a[:4])[0]
            elif op == 6:
                st["commits"] += 1
                snapshot()
                if st["commits"] == 2 and not st["go_at"]:
                    st["go_at"] = time.time() + 1.0
        out += m
    if out or ofds:
        tx(s, out, ofds)


def server_msgs():
    global sbuf, sfds
    out = b""
    ofds = []
    while len(sbuf) >= 8:
        oid, w = struct.unpack("<II", sbuf[:8])
        size, op = w >> 16, w & 0xFFFF
        if size < 8 or len(sbuf) < size:
            break
        m, sbuf = sbuf[:size], sbuf[size:]
        nfd = 1 if (oid == st["kb"] and op == 0) else 0   # keymap carries the only fd we pass
        fds = [sfds.pop(0) for _ in range(nfd)] if nfd else []
        if oid == st["ptr"] and st["ptr"]:
            continue                                     # the user's pointer stays out
        if oid == st["kb"] and st["kb"] and op not in (0, 5):
            continue                                     # keymap / repeat_info only
        if oid == st["dd"] and st["dd"]:
            if op == 0:
                drop_ids.add(struct.unpack("<I", m[8:12])[0])
            continue                                     # the system clipboard stays out
        if oid in drop_ids:
            continue
        out += m
        ofds += fds
    if out or ofds:
        tx(c, out, ofds)


def pump(ready):
    global cbuf, cfds, sbuf, sfds
    for a in ready:
        if a is c or a is s:
            data, fds = rx(a)
            if not data and not fds:
                L("closed")
                finish()
            if a is c:
                cbuf += data
                cfds += fds
                client_msgs()
            else:
                sbuf += data
                sfds += fds
                server_msgs()


def finish():
    if last:
        with open(frame_out, "wb") as f:
            f.write(struct.pack("<II", last[0], last[1]) + last[2])
        L("frame", last[0], last[1], "commits", st["commits"])
    raise SystemExit(0)


end = time.time() + timeout
while time.time() < end:
    r, _, _ = select.select([c, s], [], [], 0.05)
    try:
        pump(r)
    except (BrokenPipeError, ConnectionResetError):
        L("closed")
        finish()
    if st["go_at"] and not st["done"] and time.time() >= st["go_at"]:
        st["done"] = True
        scenario()
    for key in ("pop", "tool"):
        at = st.get(key + "_at")
        if at and time.time() >= at:
            st[key + "_at"] = 0
            sf, p = st[key + "surf"], st["ptr"]
            for e in (msg(p, 0, u32(ser()) + u32(sf) + fx(5.5) + fx(6)) + msg(p, 5),
                      msg(p, 3, u32(ser()) + u32(tm()) + u32(0x110) + u32(1)) + msg(p, 5),
                      msg(p, 2, u32(tm()) + fx(20) + fx(30)) + msg(p, 5),
                      msg(p, 3, u32(ser()) + u32(tm()) + u32(0x110) + u32(0)) + msg(p, 5),
                      msg(p, 1, u32(ser()) + u32(sf)) + msg(p, 5)):
                to_client(e)
                time.sleep(0.03)
            L("pointer on", key, sf)
    if scen in ("clip", "clipimg") and st["src"]:
        src, st["src"] = st["src"], 0
        selection_read(src)
finish()

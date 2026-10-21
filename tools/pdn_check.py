#!/usr/bin/env python3
"""Independent paint.net (.pdn) reader / writer checks and the MS-NRBF dumper (agent CP).

Standalone reader (stdlib + numpy):

  python tools/pdn_check.py --dump FILE.pdn            object graph, layers, deferred blocks
  python tools/pdn_check.py --png OUTDIR FILE.pdn ...  every layer + the flattened composite as PNG
  python tools/pdn_check.py --compare FILE.pdn IMG.png max |diff| of the composite vs a PNG

Library checks (runs apps/codectest ops 220..239 and 240 / 252 through tools/imgtest_common.py):

  python tools/pdn_check.py --target both --out build/cp            all suites
  python tools/pdn_check.py --target win --out build/cp --only samples,fuzz --no-build

Suites: samples (our decoder vs this reader on apps/PaintCraft/tests/fixtures/pdn/*.pdn: every layer
bit-exact, all properties and metadata, flattened composite, img_decode_ex / img_probe),
roundtrip (paint.net's files re-encoded: NRBF byte-identical to paint.net's, payload layout;
synthetic documents: names, hidden layers, opacity, all 14 blend modes, metadata, ICC, DPI
units, PARGB / XRGB / A8 layers, deflate levels; our files decoded again), flat (img_encode /
img_decode_ex, cancel, pixel limit), legacy (gzip-wrapped variants), v3 (synthetic paint.net
3.x shapes: NameValueCollection metadata, blendOp classes, inline MemoryBlock chunks, raw
payload), nrbf (structural MS-NRBF mutants: limits, references, cycles, nesting 10 000,
1M + 1 objects, record types), fuzz (truncation, bit flips, NRBF i32 fields, chunk headers),
dotnet (Microsoft's BinaryFormatter reads the NRBF part of the samples and of our files;
Windows PowerShell 5.1), user (writes the files for the paint.net acceptance test to
OUT/for_user), perf (d_big decode, 4000x3000x10 encode; best of 3 / 5 runs).

The format (confirmed on paint.net 5.1.12 files):
  "PDN3" | u24 LE n | n bytes UTF-8 XML header | 00 01 | NRBF stream | deferred MemoryBlocks
  (legacy: 1F 8B after the header = gzip of NRBF + blocks; no "PDN3" = whole file gzip)
  deferred block: u8 format (0 gzip, 1 raw) | u32 BE chunk size | chunks in any order:
  u32 BE index | u32 BE size | data (a gzip member or raw bytes)
"""

import base64
import gzip
import io
import os
import re
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
SAMPLES = ROOT / "apps" / "PaintCraft" / "tests" / "fixtures" / "pdn"


class PdnError(Exception):
    pass


# ================================================================== MS-NRBF reader

PRIM_NAMES = {1: "Boolean", 2: "Byte", 3: "Char", 5: "Decimal", 6: "Double", 7: "Int16", 8: "Int32",
              9: "Int64", 10: "SByte", 11: "Single", 12: "TimeSpan", 13: "DateTime", 14: "UInt16",
              15: "UInt32", 16: "UInt64", 17: "Null", 18: "String"}
PRIM_FMT = {1: "<?", 2: "<B", 6: "<d", 7: "<h", 8: "<i", 9: "<q", 10: "<b", 11: "<f", 12: "<q",
            13: "<Q", 14: "<H", 15: "<I", 16: "<Q"}
BT_PRIM, BT_STRING, BT_OBJECT, BT_SYSCLASS, BT_CLASS, BT_OBJARR, BT_STRARR, BT_PRIMARR = range(8)
MAX_DEPTH = 64
MAX_OBJECTS = 1 << 20


class Ref:
    __slots__ = ("id",)

    def __init__(self, i):
        self.id = i

    def __repr__(self):
        return f"#ref{self.id}"


class NClass:
    """Class metadata: name, member names, member types [(binarytype, info)], library id."""
    __slots__ = ("oid", "name", "members", "types", "lib", "system", "rec")

    def __init__(self, oid, name, members, types, lib, system, rec):
        self.oid, self.name, self.members, self.types = oid, name, members, types
        self.lib, self.system, self.rec = lib, system, rec


class NObj:
    __slots__ = ("oid", "cls", "values")

    def __init__(self, oid, cls, values):
        self.oid, self.cls, self.values = oid, cls, values

    def get(self, *names, default=None):
        for n in names:
            if n in self.cls.members:
                return self.values[self.cls.members.index(n)]
        return default

    def has(self, name):
        return name in self.cls.members


class NArr:
    __slots__ = ("oid", "kind", "etype", "items", "rank", "lengths")

    def __init__(self, oid, kind, etype, items, rank=1, lengths=None):
        self.oid, self.kind, self.etype, self.items = oid, kind, etype, items
        self.rank = rank
        self.lengths = lengths if lengths is not None else [len(items) if items is not None else 0]


class NStr(str):
    oid = 0


class NrbfReader:
    """Parses one MS-NRBF stream (records 0..17; 21 / 22 rejected).  Objects are kept in
    `objs` (id -> NObj / NArr / NStr); member values are Python values, NStr, nested objects
    or Ref(id).  `order` lists the object ids in stream order, `libs` the libraries."""

    def __init__(self, data, pos=0):
        self.d = data
        self.p = pos
        self.objs = {}
        self.order = []
        self.classes = {}           # metadata id -> NClass
        self.libs = {}
        self.root = None
        self.header = None
        self.depth = 0
        self.nrec = 0
        self.nested = set()
        self.spans = {}             # class record oid -> (rt, body start, body end, [lib id positions])
        self.arr_tails = {}         # BinaryArray oid -> (start, end) of its element type info
        self._libpos = []

    # ---------------------------------------------------------------- primitives
    def need(self, n):
        if n < 0 or self.p + n > len(self.d):
            raise PdnError(f"truncated at {self.p} (+{n})")

    def u8(self):
        self.need(1)
        v = self.d[self.p]
        self.p += 1
        return v

    def i32(self):
        self.need(4)
        v = struct.unpack_from("<i", self.d, self.p)[0]
        self.p += 4
        return v

    def lps(self):
        n, shift = 0, 0
        for i in range(5):
            b = self.u8()
            n |= (b & 0x7F) << shift
            shift += 7
            if not b & 0x80:
                break
            if i == 4:
                raise PdnError("bad 7-bit length")
        if n > 0x7FFFFFFF:
            raise PdnError("string length")
        self.need(n)
        s = self.d[self.p:self.p + n].decode("utf-8")
        self.p += n
        return s

    def prim(self, t):
        if t == 18 or t == 5:
            return self.lps()
        if t == 17:
            return None
        if t == 3:                  # Char: one UTF-8 encoded character
            b0 = self.u8()
            n = 0 if b0 < 0x80 else 1 if b0 < 0xE0 else 2 if b0 < 0xF0 else 3
            self.need(n)
            s = bytes([b0]) + self.d[self.p:self.p + n]
            self.p += n
            return s.decode("utf-8")
        f = PRIM_FMT.get(t)
        if f is None:
            raise PdnError(f"bad primitive type {t}")
        n = struct.calcsize(f)
        self.need(n)
        v = struct.unpack_from(f, self.d, self.p)[0]
        self.p += n
        return v

    # ---------------------------------------------------------------- records
    def add(self, oid, obj):
        if oid in self.objs:
            raise PdnError(f"duplicate object id {oid}")
        if len(self.objs) >= MAX_OBJECTS:
            raise PdnError("too many objects")
        self.objs[oid] = obj
        self.order.append(oid)
        if isinstance(obj, NStr):
            obj.oid = oid

    def class_info(self):
        oid = self.i32()
        name = self.lps()
        n = self.i32()
        if n < 0 or n > 100000:
            raise PdnError("member count")
        members = [self.lps() for _ in range(n)]
        return oid, name, members

    def type_info(self, n):
        bts = [self.u8() for _ in range(n)]
        types = []
        for bt in bts:
            if bt in (BT_PRIM, BT_PRIMARR):
                types.append((bt, self.u8()))
            elif bt == BT_SYSCLASS:
                types.append((bt, self.lps()))
            elif bt == BT_CLASS:
                nm = self.lps()
                self._libpos.append(self.p)
                types.append((bt, (nm, self.i32())))
            elif bt in (BT_STRING, BT_OBJECT, BT_OBJARR, BT_STRARR):
                types.append((bt, None))
            else:
                raise PdnError(f"bad binary type {bt}")
        return types

    def values(self, cls):
        vals = []
        for i in range(len(cls.members)):
            t = cls.types[i] if cls.types is not None else None
            if t is not None and t[0] == BT_PRIM:
                vals.append(self.prim(t[1]))
            else:
                v = self.record(value=True)
                if isinstance(v, (NObj, NArr, NStr)):
                    self.nested.add(v.oid)
                vals.append(v)
        return vals

    def record(self, value=False):
        """Read one record; returns its value (object / Ref / None / primitive) or a
        marker for non-value records."""
        self.depth += 1
        self.nrec += 1
        if self.depth > MAX_DEPTH:
            raise PdnError("nesting too deep")
        try:
            return self._record(value)
        finally:
            self.depth -= 1

    def _record(self, value):
        start = self.p
        rt = self.u8()
        if rt == 0:
            self.header = struct.unpack_from("<iiii", self.d, self.p) if self.p + 16 <= len(self.d) else None
            self.need(16)
            self.p += 16
            self.root = self.header[0]
            return "HEADER"
        if rt == 12:
            lid = self.i32()
            if lid in self.libs:
                raise PdnError("duplicate library id")
            self.libs[lid] = self.lps()
            return self.record(value) if value else "LIB"
        if rt == 11:
            return "END"
        if rt in (2, 3, 4, 5):
            body0 = self.p + 4
            self._libpos = []
            oid, name, members = self.class_info()
            types = self.type_info(len(members)) if rt in (4, 5) else None
            if rt in (3, 5):
                self._libpos.append(self.p)
            lib = self.i32() if rt in (3, 5) else None
            self.spans[oid] = (rt, body0, self.p, list(self._libpos))
            if lib is not None and lib not in self.libs:
                raise PdnError(f"unknown library {lib}")
            cls = NClass(oid, name, members, types, lib, rt in (2, 4), rt)
            if oid in self.classes:
                raise PdnError("duplicate metadata id")
            self.classes[oid] = cls
            obj = NObj(oid, cls, None)
            self.add(oid, obj)
            obj.values = self.values(cls)
            return obj
        if rt == 1:
            oid = self.i32()
            mid = self.i32()
            cls = self.classes.get(mid)
            if cls is None:
                raise PdnError(f"ClassWithId: unknown metadata {mid}")
            obj = NObj(oid, cls, None)
            self.add(oid, obj)
            obj.values = self.values(cls)
            return obj
        if rt == 6:
            oid = self.i32()
            s = NStr(self.lps())
            self.add(oid, s)
            return s
        if rt == 8:
            return self.prim(self.u8())
        if rt == 9:
            return Ref(self.i32())
        if rt == 10:
            return None
        if rt in (13, 14):
            n = self.u8() if rt == 13 else self.i32()
            if n < 0:
                raise PdnError("null count")
            return ("NULLS", n)
        if rt == 15:
            oid = self.i32()
            n = self.i32()
            t = self.u8()
            if n < 0:
                raise PdnError("array length")
            if t in PRIM_FMT and t != 1:
                sz = struct.calcsize(PRIM_FMT[t])
                self.need(n * sz)
                if t == 2:
                    items = bytes(self.d[self.p:self.p + n])
                else:
                    items = list(struct.unpack_from("<%d%s" % (n, PRIM_FMT[t][1]), self.d, self.p))
                self.p += n * sz
            else:
                items = [self.prim(t) for _ in range(n)]
            arr = NArr(oid, "prim", t, items)
            self.add(oid, arr)
            return arr
        if rt in (16, 17):
            oid = self.i32()
            n = self.i32()
            if n < 0:
                raise PdnError("array length")
            arr = NArr(oid, "object" if rt == 16 else "string", None, None, 1, [n])
            self.add(oid, arr)
            arr.items = self.elements(n, None)
            return arr
        if rt == 7:
            oid = self.i32()
            kind = self.u8()
            rank = self.i32()
            if rank < 1 or rank > 32:
                raise PdnError("array rank")
            lengths = [self.i32() for _ in range(rank)]
            if kind in (3, 4, 5):
                [self.i32() for _ in range(rank)]
            tail0 = self.p
            bt = self.u8()
            info = None
            if bt in (BT_PRIM, BT_PRIMARR):
                info = self.u8()
            elif bt == BT_SYSCLASS:
                info = self.lps()
            elif bt == BT_CLASS:
                info = (self.lps(), self.i32())
            elif bt not in (BT_STRING, BT_OBJECT, BT_OBJARR, BT_STRARR):
                raise PdnError(f"bad binary type {bt}")
            self.arr_tails[oid] = (tail0, self.p)
            total = 1
            for ln in lengths:
                if ln < 0:
                    raise PdnError("array length")
                total *= ln
            if total > 1 << 28:
                raise PdnError("array too large")
            arr = NArr(oid, "binary%d" % kind, (bt, info), None, rank, lengths)
            self.add(oid, arr)
            if bt == BT_PRIM:
                arr.items = [self.prim(info) for _ in range(total)]
            else:
                arr.items = self.elements(total, None)
            return arr
        raise PdnError(f"unsupported record type {rt} at {start}")

    def elements(self, n, _t):
        items = []
        while len(items) < n:
            v = self.record(value=True)
            if isinstance(v, tuple) and v and v[0] == "NULLS":
                if len(items) + v[1] > n:
                    raise PdnError("null run past the array end")
                items.extend([None] * v[1])
            elif isinstance(v, str) and not isinstance(v, NStr) and v in ("END", "HEADER", "LIB"):
                raise PdnError("unexpected record in array")
            else:
                if isinstance(v, (NObj, NArr, NStr)):
                    self.nested.add(v.oid)
                items.append(v)
        return items

    def parse(self):
        v = self.record()
        if v != "HEADER":
            raise PdnError("no serialization header")
        while True:
            v = self.record()
            if v == "END":
                break
            if isinstance(v, tuple):
                raise PdnError("null record at top level")
        if self.root not in self.objs:
            raise PdnError("root object missing")
        return self

    def deref(self, v):
        seen = 0
        while isinstance(v, Ref):
            if v.id not in self.objs:
                raise PdnError(f"dangling reference {v.id}")
            v = self.objs[v.id]
            seen += 1
            if seen > 4:
                raise PdnError("reference chain")
        return v


# ================================================================== object graph dump

def type_str(t):
    bt, info = t
    if bt == BT_PRIM:
        return PRIM_NAMES.get(info, str(info))
    if bt == BT_PRIMARR:
        return PRIM_NAMES.get(info, str(info)) + "[]"
    if bt == BT_CLASS:
        return f"{info[0]} (lib {info[1]})"
    if bt == BT_SYSCLASS:
        return info
    return ["", "String", "Object", "", "", "Object[]", "String[]"][bt]


def short(v, rd):
    if isinstance(v, Ref):
        o = rd.objs.get(v.id)
        cn = o.cls.name if isinstance(o, NObj) else type(o).__name__
        return f"-> #{v.id} ({cn})"
    if isinstance(v, NStr):
        return repr(str(v)[:80]) + f" (#{v.oid})"
    if isinstance(v, NObj):
        return f"inline #{v.oid} {v.cls.name}"
    if isinstance(v, NArr):
        return f"inline array #{v.oid}"
    if isinstance(v, bytes):
        return f"bytes[{len(v)}]"
    return repr(v)


def dump_graph(rd, out=sys.stdout):
    print(f"  header root={rd.header[0]} headerId={rd.header[1]} version={rd.header[2]}.{rd.header[3]}",
          file=out)
    for lid, name in rd.libs.items():
        print(f"  library {lid}: {name}", file=out)

    def obj(o, ind):
        if isinstance(o, NObj):
            c = o.cls
            rec = {1: "ClassWithId", 2: "SystemClassWithMembers", 3: "ClassWithMembers",
                   4: "SystemClassWithMembersAndTypes", 5: "ClassWithMembersAndTypes"}[c.rec]
            meta = "" if c.oid == o.oid else f" (metadata of #{c.oid})"
            print(f"{ind}#{o.oid} {c.name} [{rec}{meta}{'' if c.lib is None else ', lib %d' % c.lib}]",
                  file=out)
            for i, m in enumerate(c.members):
                t = type_str(c.types[i]) if c.types else "?"
                v = o.values[i]
                print(f"{ind}    {m}: {t} = {short(v, rd)}", file=out)
                if isinstance(v, (NObj, NArr)):
                    obj(v, ind + "        ")
        elif isinstance(o, NArr):
            et = type_str(o.etype) if isinstance(o.etype, tuple) else PRIM_NAMES.get(o.etype, o.etype)
            print(f"{ind}#{o.oid} array {o.kind} of {et} lengths {o.lengths}", file=out)
            if isinstance(o.items, bytes):
                print(f"{ind}    bytes[{len(o.items)}]", file=out)
                return
            for i, v in enumerate(o.items[:64]):
                print(f"{ind}    [{i}] {short(v, rd)}", file=out)
                if isinstance(v, (NObj, NArr)):
                    obj(v, ind + "        ")
            if len(o.items) > 64:
                print(f"{ind}    ... {len(o.items)} items", file=out)
        elif isinstance(o, NStr):
            print(f"{ind}#{o.oid} string {str(o)[:100]!r}", file=out)

    for oid in rd.order:
        o = rd.objs[oid]
        # print only top-level records (nested ones are printed with their parent)
        if oid in rd.nested:
            continue
        obj(o, "  ")


# ================================================================== paint.net document

# LayerBlendMode (paint.net 4.x / 5.x) -> index into BM_NAMES (library BM_* order).
# Confirmed values: Normal 0, Multiply 1, Overlay 7 (c_layers.pdn); the rest follows the
# enum order which equals the UserBlendOps order of paint.net 3.x.
PDN_BLEND_NAMES = ["Normal", "Multiply", "Additive", "ColorBurn", "ColorDodge", "Reflect", "Glow",
                   "Overlay", "Difference", "Negation", "Lighten", "Darken", "Screen", "Xor"]
BLENDOP_CLASS = {"PaintDotNet.UserBlendOps+%sBlendOp" % n: i for i, n in enumerate(PDN_BLEND_NAMES)}


class Layer:
    def __init__(self):
        self.name = ""
        self.visible = True
        self.isbg = False
        self.opacity = 255
        self.blend = 0
        self.blendop = None          # 3.x-style UserBlendOp class name
        self.px = None               # (h, w, 4) uint8 straight BGRA
        self.meta = []               # [(key, value)]
        self.w = self.h = 0


class Doc:
    def __init__(self):
        self.w = self.h = 0
        self.xml = ""
        self.layers = []
        self.meta = []
        self.saved_with = None
        self.nrbf = None
        self.nrbf_span = (0, 0)
        self.blocks = []            # [(fmt, chunksize, [(index, size, offset)])]
        self.variant = ""


def kv_items(rd, v):
    """KeyValuePair<string,string>[] or NameValueCollection -> [(k, v)]."""
    v = rd.deref(v)
    out = []
    if v is None:
        return out
    if isinstance(v, NArr):
        for it in v.items:
            it = rd.deref(it)
            if isinstance(it, NObj):
                out.append((str(rd.deref(it.get("key"))), str(rd.deref(it.get("value")))))
        return out
    if isinstance(v, NObj):            # NameValueCollection (3.x): Keys / Values arrays
        ks = rd.deref(v.get("Keys"))
        vs = rd.deref(v.get("Values"))
        if isinstance(ks, NArr) and isinstance(vs, NArr):
            for k, val in zip(ks.items, vs.items):
                val = rd.deref(val)
                if isinstance(val, NObj):           # ArrayList: its first element
                    arr = rd.deref(val.get("_items"))
                    val = rd.deref(arr.items[0]) if isinstance(arr, NArr) and arr.items else ""
                if val is None:
                    val = ""
                out.append((str(rd.deref(k)), str(val)))
    return out


def read_blocks(data, pos, blocks, strict=True):
    """Read the deferred MemoryBlock payloads following the NRBF stream.
    blocks = [length]; returns ([bytes], [(fmt, chunk, [(idx, size)])], end)."""
    res, info = [], []
    for length in blocks:
        if pos + 5 > len(data):
            raise PdnError("deferred data truncated")
        fmt = data[pos]
        chunk = struct.unpack_from(">I", data, pos + 1)[0]
        pos += 5
        if fmt not in (0, 1):
            raise PdnError(f"block format {fmt}")
        if chunk == 0:
            raise PdnError("chunk size 0")
        n = (length + chunk - 1) // chunk
        buf = bytearray(length)
        seen = [False] * n
        chunks = []
        for _ in range(n):
            if pos + 8 > len(data):
                raise PdnError("chunk header truncated")
            idx, size = struct.unpack_from(">II", data, pos)
            pos += 8
            if idx >= n or seen[idx]:
                raise PdnError(f"bad chunk index {idx}")
            seen[idx] = True
            if pos + size > len(data):
                raise PdnError("chunk data truncated")
            raw = data[pos:pos + size]
            exp = min(chunk, length - idx * chunk)
            if fmt == 0:
                dec = gzip.decompress(raw)
            else:
                dec = raw
            if len(dec) != exp:
                raise PdnError(f"chunk {idx}: {len(dec)} bytes, expected {exp}")
            buf[idx * chunk: idx * chunk + exp] = dec
            chunks.append((idx, size, pos))
            pos += size
        res.append(bytes(buf))
        info.append((fmt, chunk, chunks))
    return res, info, pos


def read_pdn(data):
    doc = Doc()
    data = bytes(data)
    if data[:2] == b"\x1f\x8b":
        doc.variant = "legacy-gzip"
        data = gzip.decompress(data)
        nrbf_pos = 0
    else:
        if data[:4] != b"PDN3":
            raise PdnError("no PDN3 magic")
        if len(data) < 7:
            raise PdnError("truncated header")
        n = data[4] | data[5] << 8 | data[6] << 16
        if 7 + n + 2 > len(data):
            raise PdnError("truncated XML header")
        doc.xml = data[7:7 + n].decode("utf-8")
        p = 7 + n
        if data[p:p + 2] == b"\x00\x01":
            nrbf_pos = p + 2
            doc.variant = "pdn3"
        elif data[p:p + 2] == b"\x1f\x8b":
            doc.variant = "pdn3-gzip"
            data = gzip.decompress(data[p:])
            nrbf_pos = 0
        else:
            raise PdnError("unknown stream marker")
    rd = NrbfReader(data, nrbf_pos).parse()
    doc.nrbf = rd
    doc.nrbf_span = (nrbf_pos, rd.p)
    root = rd.objs[rd.root]
    if not isinstance(root, NObj) or not root.cls.name.startswith("PaintDotNet.Document"):
        raise PdnError(f"root is {getattr(getattr(root, 'cls', None), 'name', root)}")
    doc.w = root.get("width")
    doc.h = root.get("height")
    sw = rd.deref(root.get("savedWith"))
    if isinstance(sw, NObj):
        doc.saved_with = tuple(sw.get(k) for k in ("_Major", "_Minor", "_Build", "_Revision"))
    doc.meta = kv_items(rd, root.get("userMetadataItems", "userMetaData", "metadata"))
    ll = rd.deref(root.get("layers"))
    if not isinstance(ll, NObj):
        raise PdnError("no layer list")
    items = rd.deref(ll.get("ArrayList+_items", "_items"))
    size = ll.get("ArrayList+_size", "_size")
    if not isinstance(items, NArr) or not isinstance(size, int) or size < 0 or size > len(items.items):
        raise PdnError("bad layer list")
    # MemoryBlocks in stream order (= the order of the deferred payloads)
    mblocks = [oid for oid in rd.order if isinstance(rd.objs[oid], NObj)
               and rd.objs[oid].cls.name == "PaintDotNet.MemoryBlock"]
    deferred = [oid for oid in mblocks if rd.objs[oid].get("deferred")]
    lens = [rd.objs[oid].get("length64") for oid in deferred]
    payload, doc.blocks, end = read_blocks(data, rd.p, lens)
    doc.trailing = len(data) - end
    blockdata = dict(zip(deferred, payload))
    for li in items.items[:size]:
        lo = rd.deref(li)
        if not isinstance(lo, NObj):
            raise PdnError("layer is not an object")
        L = Layer()
        lp = rd.deref(lo.get("Layer+properties", "properties"))
        if isinstance(lp, NObj) and lp.cls.name.endswith("LayerProperties") and not lp.cls.name.endswith(
                "BitmapLayerProperties"):
            L.name = str(rd.deref(lp.get("name")) or "")
            L.visible = bool(lp.get("visible", default=True))
            L.isbg = bool(lp.get("isBackground", default=False))
            L.opacity = lp.get("opacity", default=255)
            L.meta = kv_items(rd, lp.get("userMetadataItems", "userMetaData"))
            bm = rd.deref(lp.get("blendMode"))
            if isinstance(bm, NObj):
                L.blend = bm.get("value__")
            elif isinstance(bm, int):
                L.blend = bm
        bp = rd.deref(lo.get("properties"))
        if isinstance(bp, NObj) and bp.cls.name.endswith("BitmapLayerProperties"):
            op = rd.deref(bp.get("blendOp"))
            if isinstance(op, NObj):
                L.blendop = op.cls.name
                if lp is None or not isinstance(rd.deref(lp.get("blendMode")), (NObj, int)):
                    L.blend = BLENDOP_CLASS.get(op.cls.name, 0)
        sf = rd.deref(lo.get("surface"))
        if not isinstance(sf, NObj):
            raise PdnError("no surface")
        w, h, stride = sf.get("width"), sf.get("height"), sf.get("stride")
        mb = sf.get("scan0")
        mbo = rd.deref(mb)
        if not isinstance(mbo, NObj):
            raise PdnError("no scan0")
        if mbo.get("deferred"):
            raw = blockdata[mbo.oid]
        else:
            parts = []
            i = 0
            while mbo.has("chunk%d" % i):
                parts.append(rd.deref(mbo.get("chunk%d" % i)).items)
                i += 1
            if not parts and mbo.has("pointerData"):
                parts.append(rd.deref(mbo.get("pointerData")).items)
            raw = b"".join(parts)
        if w != doc.w or h != doc.h or stride < w * 4 or len(raw) < stride * h:
            raise PdnError(f"surface {w}x{h} stride {stride} len {len(raw)}")
        a = np.frombuffer(raw, np.uint8, count=stride * h).reshape(h, stride)[:, :w * 4]
        L.px = a.reshape(h, w, 4).copy()
        L.w, L.h = w, h
        doc.layers.append(L)
    return doc


# ================================================================== composite (paint.net UserBlendOps)

def int_scale(a, b):
    t = a * b + 0x80
    return ((t >> 8) + t) >> 8


def blend_f(mode, l, r):
    if mode == 1:
        return int_scale(l, r)
    if mode == 2:
        return np.minimum(255, l + r)
    if mode == 3:
        q = ((255 - l) * 255) // np.maximum(r, 1)
        return np.where(r == 0, 0, np.maximum(0, 255 - q))
    if mode == 4:
        q = (l * 255) // np.maximum(255 - r, 1)
        return np.where(r == 255, 255, np.minimum(255, q))
    if mode == 5:
        q = (l * l) // np.maximum(255 - r, 1)
        return np.where(r == 255, 255, np.minimum(255, q))
    if mode == 6:
        q = (r * r) // np.maximum(255 - l, 1)
        return np.where(l == 255, 255, np.minimum(255, q))
    if mode == 7:
        return np.where(l < 128, int_scale(2 * l, r), 255 - int_scale(2 * (255 - l), 255 - r))
    if mode == 8:
        return np.abs(l - r)
    if mode == 9:
        return 255 - np.abs(255 - l - r)
    if mode == 10:
        return np.maximum(l, r)
    if mode == 11:
        return np.minimum(l, r)
    if mode == 12:
        return r + l - int_scale(r, l)
    if mode == 13:
        return l ^ r
    return r


def blend_layer(lhs, rhs, mode, opacity):
    """rhs over lhs, straight BGRA (h, w, 4) uint8 -> uint8 (tools/blend_check.py ref_blend)."""
    L = lhs.astype(np.int64)
    R = rhs.astype(np.int64)
    la, ra = L[..., 3], int_scale(R[..., 3], opacity)
    y = int_scale(la, 255 - ra)
    tot = y + ra
    x = int_scale(la, ra)
    z = ra - x
    d = np.maximum(tot, 1)
    out = np.zeros_like(L)
    for c in range(3):
        lv, rv = L[..., c], R[..., c]
        out[..., c] = (lv * y + rv * z + blend_f(mode, lv, rv) * x) // d
    out[..., 3] = tot
    out[tot == 0] = 0
    return out.astype(np.uint8)


def composite(doc):
    acc = np.zeros((doc.h, doc.w, 4), np.uint8)
    for L in doc.layers:
        if not L.visible:
            continue
        acc = blend_layer(acc, L.px, L.blend if 0 <= L.blend < 14 else 0, L.opacity)
    return acc


# ================================================================== PNG I/O

def png_bytes(bgra):
    h, w = bgra.shape[:2]
    rgba = bgra[..., [2, 1, 0, 3]]
    raw = b"".join(b"\x00" + rgba[y].tobytes() for y in range(h))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def png_read(data):
    """Minimal PNG reader (non-interlaced; grey / RGB / palette / grey+alpha / RGBA, 8-bit and
    palette / grey bit depths 1..8) -> straight BGRA (h, w, 4)."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise PdnError("not a PNG")
    p, idat, hdr, plte, trns = 8, b"", None, None, None
    while p < len(data):
        n, t = struct.unpack_from(">I4s", data, p)
        body = data[p + 8:p + 8 + n]
        if t == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif t == b"IDAT":
            idat += body
        elif t == b"PLTE":
            plte = body
        elif t == b"tRNS":
            trns = body
        p += 12 + n
    w, h, bd, ct, _, _, il = hdr
    if il or (bd != 8 and ct not in (0, 3)):
        raise PdnError("unsupported PNG")
    ch = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ct]
    raw = zlib.decompress(idat)
    stride = (w * ch * bd + 7) // 8
    bpp = max(1, ch * bd // 8)
    out = np.zeros((h, stride), np.int64)
    prev = np.zeros(stride, np.int64)
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = np.frombuffer(raw, np.uint8, stride, y * (stride + 1) + 1).astype(np.int64)
        cur = np.zeros(stride, np.int64)
        if f == 0:
            cur = line
        elif f == 2:
            cur = (line + prev) & 255
        else:
            for i in range(stride):
                a = cur[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                if f == 1:
                    pr = a
                elif f == 3:
                    pr = (a + b) >> 1
                else:
                    pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                    pr = a if pa <= pb and pa <= pc else b if pb <= pc else c
                cur[i] = (line[i] + pr) & 255
        out[y] = cur
        prev = cur
    if bd < 8:
        bits = np.unpackbits(out.astype(np.uint8), axis=1)
        vals = np.zeros((h, w), np.int64)
        for k in range(bd):
            vals = (vals << 1) | bits[:, k:w * bd:bd][:, :w]
        samples = vals.reshape(h, w, 1)
        if ct == 0:
            samples = samples * 255 // ((1 << bd) - 1)
        px = samples.astype(np.uint8)
    else:
        px = out.reshape(h, w, ch).astype(np.uint8)
    if ct == 3:
        pal = np.frombuffer(plte, np.uint8).reshape(-1, 3)
        al = np.full(len(pal), 255, np.uint8)
        if trns:
            al[:len(trns)] = np.frombuffer(trns, np.uint8)
        idx = px[..., 0]
        rgb = pal[idx]
        return np.concatenate([rgb[..., ::-1], al[idx][..., None]], -1)
    if ch == 4:
        return px[..., [2, 1, 0, 3]].copy()
    if ch == 3:
        a = np.full((h, w, 1), 255, np.uint8)
        return np.concatenate([px[..., [2, 1, 0]], a], -1)
    g = px[..., 0:1]
    a = px[..., 1:2] if ch == 2 else np.full((h, w, 1), 255, np.uint8)
    return np.concatenate([g, g, g, a], -1)


# ================================================================== CLI: dump / png / compare

def describe(doc, path, graph=False, out=sys.stdout):
    print(f"{path}: variant {doc.variant}, {doc.w}x{doc.h}, {len(doc.layers)} layer(s), "
          f"savedWith {doc.saved_with}", file=out)
    x = re.sub(r'png="[^"]*"', lambda m: f'png="<{len(m.group(0)) - 6} base64 chars>"', doc.xml)
    print(f"  xml: {x}", file=out)
    rd = doc.nrbf
    print(f"  nrbf: {doc.nrbf_span[1] - doc.nrbf_span[0]} bytes at {doc.nrbf_span[0]}, "
          f"{len(rd.objs)} objects, {len(rd.classes)} class records", file=out)
    for k, v in doc.meta:
        print(f"  doc meta {k!r} = {v[:120]!r}", file=out)
    for i, L in enumerate(doc.layers):
        bn = PDN_BLEND_NAMES[L.blend] if 0 <= L.blend < 14 else f"?{L.blend}"
        print(f"  layer {i}: name {L.name!r} visible {L.visible} bg {L.isbg} opacity {L.opacity} "
              f"blendMode {L.blend} ({bn}) blendOp {L.blendop} meta {L.meta}", file=out)
    for i, (fmt, chunk, chunks) in enumerate(doc.blocks):
        order = [c[0] for c in chunks]
        sizes = [c[1] for c in chunks]
        print(f"  block {i}: format {fmt} chunk {chunk} ({len(chunks)} chunks, order "
              f"{'ascending' if order == sorted(order) else order[:16]}, "
              f"sizes {sizes[:6]}{'...' if len(sizes) > 6 else ''})", file=out)
        if chunks and fmt == 0:
            off = chunks[0][2]
            hdr = doc_data_cache.get(id(doc), b"")[off:off + 10]
            if hdr:
                print(f"           first gzip header {hdr.hex()}", file=out)
    print(f"  trailing bytes after the blocks: {doc.trailing}", file=out)
    if graph:
        dump_graph(rd, out)


doc_data_cache = {}


def cli_main(argv):
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--dump", action="store_true")
    ap.add_argument("--graph", action="store_true", help="with --dump: print the full object graph")
    ap.add_argument("--png", default=None)
    ap.add_argument("--compare", action="store_true")
    ap.add_argument("files", nargs="+")
    a = ap.parse_args(argv)
    if a.compare:
        doc = read_pdn(Path(a.files[0]).read_bytes())
        ref = png_read(Path(a.files[1]).read_bytes())
        c = composite(doc)
        if c.shape != ref.shape:
            print(f"size differs: {c.shape} vs {ref.shape}")
            return 1
        d = np.abs(c.astype(int) - ref.astype(int))
        print(f"max |diff| {d.max()}, pixels differing {(d.max(-1) > 0).sum()} of {doc.w * doc.h}, "
              f"per channel BGRA max {[int(d[..., i].max()) for i in range(4)]}")
        return 0
    for f in a.files:
        data = Path(f).read_bytes()
        doc = read_pdn(data)
        doc_data_cache[id(doc)] = data
        if a.dump or a.graph:
            describe(doc, f, a.graph)
        if a.png:
            od = Path(a.png)
            od.mkdir(parents=True, exist_ok=True)
            stem = Path(f).stem
            for i, L in enumerate(doc.layers):
                (od / f"{stem}_layer{i}.png").write_bytes(png_bytes(L.px))
            (od / f"{stem}_flat.png").write_bytes(png_bytes(composite(doc)))
    return 0




# ================================================================== library checks (codectest)

OP_DEC, OP_ENC, OP_RT, OP_NRBF, OP_FLAT, OP_BDEC, OP_BENC = range(220, 227)
PDND_PROBE = 1
PDNE_BLENDOP = 1


class DocRec:
    """Doc record of codectest ops 220 / 221 (see apps/codectest/src/pdn_ops.s)."""

    def __init__(self):
        self.w = self.h = 0
        self.unit = 0
        self.dpux = self.dpuy = 0.0
        self.nvc = b""
        self.xml = b""
        self.icc = b""
        self.layers = []            # dicts: name, opacity, visible, blend, isbg, meta, fmt, px


def nvc_blob(items):
    out = bytearray()
    for k, v in items:
        kb, vb = k.encode("utf-8"), v.encode("utf-8")
        out += struct.pack("<I", len(kb)) + kb + struct.pack("<I", len(vb)) + vb
    return bytes(out)


def nvc_items(blob):
    out, p = [], 0
    while p + 4 <= len(blob):
        n = struct.unpack_from("<I", blob, p)[0]
        k = blob[p + 4:p + 4 + n].decode("utf-8")
        p += 4 + n
        n = struct.unpack_from("<I", blob, p)[0]
        v = blob[p + 4:p + 4 + n].decode("utf-8")
        p += 4 + n
        out.append((k, v))
    return out


def parse_docrec(b):
    d = DocRec()
    d.w, d.h, n, d.unit, d.dpux, d.dpuy, nl, xl, il = struct.unpack_from("<IIIIddIII", b, 0)
    p = 64
    d.nvc = bytes(b[p:p + nl]); p += nl
    d.xml = bytes(b[p:p + xl]); p += xl
    d.icc = bytes(b[p:p + il]); p += il
    for _ in range(n):
        namelen, op, vis, bl, bg, ml, w, h, fmt = struct.unpack_from("<IBBBBIIII", b, p)
        p += 32
        name = bytes(b[p:p + namelen]).decode("utf-8"); p += namelen
        meta = bytes(b[p:p + ml]); p += ml
        bpp = 1 if fmt == 2 else 4
        px = np.frombuffer(b, np.uint8, w * h * bpp, p).reshape((h, w, bpp) if bpp == 4 else (h, w)).copy()
        p += w * h * bpp
        d.layers.append(dict(name=name, opacity=op, visible=vis, blend=bl, isbg=bg, meta=meta, fmt=fmt, px=px))
    if p != len(b):
        raise PdnError(f"doc record: {len(b)} bytes, parsed {p}")
    return d


def make_docrec(w, h, layers, unit=0, dpux=0.0, dpuy=0.0, nvc=b"", icc=b""):
    out = bytearray(struct.pack("<IIIIddIII", w, h, len(layers), unit, dpux, dpuy, len(nvc), 0, len(icc)))
    out += bytes(64 - len(out))
    out += nvc + icc
    for L in layers:
        name = L.get("name", "").encode("utf-8")
        meta = L.get("meta", b"")
        px = np.ascontiguousarray(L["px"], np.uint8)
        fmt = L.get("fmt", 3)
        lh, lw = px.shape[:2]
        out += struct.pack("<IBBBBIIII", len(name), L.get("opacity", 255), L.get("visible", 1),
                           L.get("blend", 0), L.get("isbg", 0), len(meta), lw, lh, fmt) + bytes(8)
        out += name + meta + px.tobytes()
    return bytes(out)


def exif_item(tag, typ, data):
    return f'<exif id="{tag}" len="{len(data)}" type="{typ}" value="{base64.b64encode(data).decode()}" />'


def nrbf_bytes(data):
    """The NRBF stream of a PDN3 file (header .. MessageEnd)."""
    doc = read_pdn(data)
    a, b = doc.nrbf_span
    return data[a:b]


def expect_doc_equal(ref, got, what, check_xml=True):
    """ref: Doc (Python reader); got: DocRec (library) -> None / message."""
    if (ref.w, ref.h, len(ref.layers)) != (got.w, got.h, len(got.layers)):
        return f"{what}: size / layers {got.w}x{got.h}x{len(got.layers)} != {ref.w}x{ref.h}x{len(ref.layers)}"
    if check_xml and ref.xml.encode("utf-8") != got.xml:
        return f"{what}: XML header differs"
    if nvc_items(got.nvc) != ref.meta:
        return f"{what}: metadata {nvc_items(got.nvc)[:3]} != {ref.meta[:3]}"
    for i, (R, G) in enumerate(zip(ref.layers, got.layers)):
        props = (R.name, R.opacity, int(R.visible), R.blend, int(R.isbg))
        gp = (G["name"], G["opacity"], G["visible"], G["blend"], G["isbg"])
        if props != gp:
            return f"{what}: layer {i} properties {gp} != {props}"
        if nvc_items(G["meta"]) != R.meta:
            return f"{what}: layer {i} metadata differs"
        if G["fmt"] != 3:
            return f"{what}: layer {i} format {G['fmt']}"
        if not np.array_equal(G["px"], R.px):
            d = np.argwhere(G["px"] != R.px)[0]
            return f"{what}: layer {i} pixels differ at (x {d[1]}, y {d[0]})"
    return None


def resolution_of(meta):
    unit, dx, dy = 2, 96.0, 96.0
    for k, v in meta:
        if not k.startswith("$exif.tag"):
            continue
        m = re.search(r'id="(\d+)"', v)
        m2 = re.search(r'value="([^"]*)"', v)
        if not m or not m2:
            continue
        raw = base64.b64decode(m2.group(1))
        tag = int(m.group(1))
        if tag == 296 and len(raw) >= 2 and 1 <= raw[0] <= 3:
            unit = raw[0]
        elif tag in (282, 283) and len(raw) >= 8:
            n, dd = struct.unpack_from("<II", raw)
            if dd:
                if tag == 282:
                    dx = n / dd
                else:
                    dy = n / dd
    return unit, dx, dy


def imgopt(fmt, effort=0, flags=0):
    return struct.pack("<IIIIIIIIIIIIQQ", fmt, 0, flags, 0, 0, 0, 0, 0, effort, 0, 0, 0, 0, 0)


# ---------------------------------------------------------------- suites

def suite_samples(it, runner, target):
    s = it.Suite("samples", target)
    recs = []
    for f in sorted(SAMPLES.glob("*.pdn")):
        data = f.read_bytes()
        ref = read_pdn(data)
        comp = composite(ref)

        def chk_doc(res, out, ref=ref, name=f.name):
            if res != 0:
                return f"error {it.err_name(res)}"
            got = parse_docrec(out)
            m = expect_doc_equal(ref, got, name)
            if m:
                return m
            unit, dx, dy = resolution_of(ref.meta)
            if (got.unit, got.dpux, got.dpuy) != (unit, dx, dy):
                return f"resolution {got.unit} {got.dpux} {got.dpuy} != {unit} {dx} {dy}"
            return None
        recs.append(it.Rec(OP_DEC, 0, data, chk_doc, f"decode {f.name}"))

        def chk_probe(res, out, ref=ref):
            if res != 0:
                return f"error {it.err_name(res)}"
            got = parse_docrec(out)
            if (got.w, got.h, len(got.layers)) != (ref.w, ref.h, 0):
                return f"probe gave {got.w}x{got.h} {len(got.layers)} layers"
            return None
        recs.append(it.Rec(OP_DEC, PDND_PROBE, data, chk_probe, f"probe {f.name}"))

        def chk_flat(res, out, comp=comp):
            if res != 0:
                return f"error {it.err_name(res)}"
            w, h, fmt, fl, px = it.parse_surface(out)
            if fmt != it.C.FMT_ARGB or not np.array_equal(px, comp):
                return f"composite differs (max {it.max_abs(px, comp) if px.shape == comp.shape else 'size'})"
            return None
        recs.append(it.Rec(OP_FLAT, 0, data, chk_flat, f"flatten {f.name}"))
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=it.C.IMGD_STRAIGHT),
                           it.expect_decex(comp, fmt=it.C.FMT_ARGB,
                                           meta=dict(fmt=it.C.IMGF_PDN, w=ref.w, h=ref.h, dpix=96 << 16,
                                                     dpiy=96 << 16, resunit=1, bits=8)),
                           f"img_decode_ex straight {f.name}"))
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data), it.expect_decex(it.premul(comp)),
                           f"img_decode_ex premultiplied {f.name}"))
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=it.C.IMGD_PROBE),
                           lambda r, o, ref=ref: None if r == 0 and it.parse_decex(o)["meta"]["w"] == ref.w
                           else f"probe result {r}", f"img_decode_ex probe {f.name}"))
        recs.append(it.Rec(it.OP_PROBE, 0, data,
                           lambda r, o, ref=ref: None if r == it.C.IMGF_PDN and it.parse_meta_rec(o)["w"] == ref.w
                           and it.parse_meta_rec(o)["h"] == ref.h else f"img_probe {r} {it.parse_meta_rec(o)}",
                           f"img_probe {f.name}"))
    it.run_suite(runner, s, recs)
    # thumbnail check vs Unbenannt1.png: which sample (if any) it is the composite of
    ref_png = SAMPLES / "Unbenannt1.png"
    if ref_png.exists():
        png = png_read(ref_png.read_bytes())
        notes = []
        for f in sorted(SAMPLES.glob("*.pdn")):
            d = read_pdn(f.read_bytes())
            if (d.h, d.w) == png.shape[:2]:
                notes.append(f"{f.name}: max |diff| {it.max_abs(composite(d), png)}")
        s.notes.append("Unbenannt1.png vs composites: " + ("; ".join(notes) or "no sample of that size"))
    return s


def suite_roundtrip(it, runner, target):
    s = it.Suite("roundtrip", target)
    recs = []
    for f in sorted(SAMPLES.glob("*.pdn")):
        data = f.read_bytes()
        ref = read_pdn(data)
        ref_nrbf = nrbf_bytes(data)

        def chk(res, out, ref=ref, ref_nrbf=ref_nrbf, name=f.name, data=data):
            if res != 0:
                return f"error {it.err_name(res)}"
            ours = read_pdn(bytes(out))
            if ours.variant != "pdn3":
                return "not a PDN3 file"
            for i, (a, b) in enumerate(zip(ref.layers, ours.layers)):
                if not np.array_equal(a.px, b.px):
                    return f"layer {i} pixels differ"
            if [(L.name, L.opacity, L.visible, L.blend, L.isbg) for L in ours.layers] != \
                    [(L.name, L.opacity, L.visible, L.blend, L.isbg) for L in ref.layers]:
                return "layer properties differ"
            if ours.meta != ref.meta:
                return f"metadata {ours.meta} != {ref.meta}"
            got = nrbf_bytes(bytes(out))
            if got != ref_nrbf:
                n = min(len(got), len(ref_nrbf))
                k = next((i for i in range(n) if got[i] != ref_nrbf[i]), n)
                return f"NRBF not byte-identical to paint.net's ({len(got)} vs {len(ref_nrbf)}, first diff at {k})"
            for (fmt, ch, chunks) in ours.blocks:
                if fmt != 0 or ch != 262144 or [c[0] for c in chunks] != list(range(len(chunks))):
                    return "payload layout differs from paint.net's"
            th = re.search(r'png="([^"]*)"', ours.xml)
            if not th:
                return "no thumbnail"
            tpng = png_read(base64.b64decode(th.group(1)))
            rth = png_read(base64.b64decode(re.search(r'png="([^"]*)"', ref.xml).group(1)))
            if tpng.shape != rth.shape:
                return f"thumbnail size {tpng.shape} != paint.net's {rth.shape}"
            return None
        recs.append(it.Rec(OP_RT, 1 << 8, data, chk, f"re-encode {f.name}"))
    # synthetic documents
    rng = np.random.default_rng(7)
    cases = []

    def rnd(h, w, alpha="mixed"):
        px = rng.integers(0, 256, (h, w, 4), np.uint8)
        if alpha == "opaque":
            px[..., 3] = 255
        elif alpha == "sparse":
            px[..., 3] = np.where(rng.random((h, w)) < 0.2, px[..., 3], 0)
        return px
    names = ["Hintergrund", "Ebene Ä", "öüß – 日本語 🎨", "", "x" * 300]
    layers = [dict(name=names[i % len(names)], px=rnd(37, 53, "opaque" if i == 0 else "mixed"),
                   opacity=[255, 128, 0, 1, 200][i], visible=[1, 0, 1, 1, 1][i], blend=[0, 1, 7, 13, 12][i],
                   isbg=int(i == 0)) for i in range(5)]
    cases.append(("5 layers 53x37", 53, 37, layers, {}))
    cases.append(("1x1", 1, 1, [dict(name="a", px=rnd(1, 1))], {}))
    cases.append(("300x2 wide", 300, 2, [dict(name="w", px=rnd(2, 300))], {}))
    cases.append(("2x300 tall", 2, 300, [dict(name="t", px=rnd(300, 2))], {}))
    cases.append(("600x520 chunks", 600, 520, [dict(name="c", px=rnd(520, 600, "sparse")),
                                              dict(name="d", px=rnd(520, 600))], {}))
    all_modes = [dict(name=f"mode {m}", px=rnd(20, 16), blend=m) for m in range(14)]
    cases.append(("14 blend modes", 16, 20, all_modes, {}))
    cases.append(("14 blend modes + blendOp classes", 16, 20, all_modes, dict(flags=PDNE_BLENDOP)))
    cases.append(("cm 118.1102", 17, 9, [dict(name="cm", px=rnd(9, 17))], dict(unit=3, dpux=118.1102, dpuy=37.795)))
    cases.append(("pixel unit", 17, 9, [dict(name="px", px=rnd(9, 17))], dict(unit=1, dpux=72.0, dpuy=72.0)))
    meta = [("$exif.tag3[0]", exif_item(305, 2, b"Paint.NET 5.1.12\0")), ("custom", "Wert äöü"),
            ("$exif.tag9[0]", exif_item(282, 5, struct.pack("<II", 1, 1)))]
    cases.append(("metadata + ICC", 8, 8, [dict(name="m", px=rnd(8, 8), meta=nvc_blob([("k", "v"), ("k2", "")]))],
                  dict(nvc=nvc_blob(meta), icc=bytes(range(256)) * 3, unit=2, dpux=300.0, dpuy=300.0)))
    cases.append(("PARGB layer", 9, 7, [dict(name="p", px=it.premul(rnd(7, 9)), fmt=1)], {}))
    cases.append(("XRGB layer", 9, 7, [dict(name="x", px=rnd(7, 9), fmt=0)], {}))
    cases.append(("A8 layer", 9, 7, [dict(name="g", px=rng.integers(0, 256, (7, 9), np.uint8), fmt=2)], {}))
    for lv in (1, 3, 6, 9):
        cases.append((f"level {lv}", 64, 33, [dict(name="l", px=rnd(33, 64))], dict(level=lv)))
    for name, w, h, layers, kw in cases:
        rec = make_docrec(w, h, layers, unit=kw.get("unit", 0), dpux=kw.get("dpux", 0.0),
                          dpuy=kw.get("dpuy", 0.0), nvc=kw.get("nvc", b""), icc=kw.get("icc", b""))
        arg = kw.get("flags", 0) | kw.get("level", 1) << 8

        def chk(res, out, layers=layers, kw=kw, name=name, w=w, h=h):
            if res != 0:
                return f"error {it.err_name(res)}"
            try:
                d = read_pdn(bytes(out))
            except Exception as e:  # noqa: BLE001
                return f"our file does not parse: {e}"
            if (d.w, d.h, len(d.layers)) != (w, h, len(layers)):
                return "size / layer count"
            for i, (L, R) in enumerate(zip(layers, d.layers)):
                fmt = L.get("fmt", 3)
                src = L["px"]
                if fmt == 1:
                    exp = it.unpremul(src)
                elif fmt == 0:
                    exp = src.copy()
                    exp[..., 3] = 255
                elif fmt == 2:
                    exp = np.stack([src, src, src, np.full_like(src, 255)], -1)
                else:
                    exp = src
                if not np.array_equal(R.px, exp):
                    return f"layer {i} pixels differ"
                if (R.name, R.opacity, int(R.visible), R.blend, int(R.isbg)) != \
                        (L.get("name", ""), L.get("opacity", 255), L.get("visible", 1), L.get("blend", 0),
                         L.get("isbg", 0)):
                    return f"layer {i} properties {(R.name[:20], R.opacity, R.visible, R.blend, R.isbg)}"
                want_op = PDN_BLEND_NAMES[L.get("blend", 0)] if kw.get("flags", 0) & PDNE_BLENDOP else "Normal"
                if R.blendop != f"PaintDotNet.UserBlendOps+{want_op}BlendOp":
                    return f"layer {i} blendOp {R.blendop}"
                if R.meta != nvc_items(L.get("meta", b"")):
                    return f"layer {i} metadata {R.meta}"
            unit, dx, dy = resolution_of(d.meta)
            eu = kw.get("unit", 0) or 2
            ex = kw.get("dpux", 0.0) or 96.0
            ey = kw.get("dpuy", 0.0) or 96.0
            if unit != eu or abs(dx - ex) > 1e-4 or abs(dy - ey) > 1e-4:
                return f"resolution {unit} {dx} {dy} != {eu} {ex} {ey}"
            if kw.get("icc"):
                icc = [v for k, v in d.meta if 'id="34675"' in v]
                if len(icc) != 1 or base64.b64decode(re.search(r'value="([^"]*)"', icc[0]).group(1)) != kw["icc"]:
                    return "ICC item missing / wrong"
            if kw.get("nvc"):
                kept = [(k, v) for k, v in nvc_items(kw["nvc"]) if not re.search(r'id="(296|282|283|34675)"', v)]
                if [x for x in d.meta if x in kept] != kept:
                    return "metadata items lost"
                keys = [k for k, _ in d.meta]
                if len(set(keys)) != len(keys):
                    return f"duplicate metadata keys {keys}"
            return None
        recs.append(it.Rec(OP_ENC, arg, rec, chk, f"encode {name}"))

        # decode our own file again: exact round trip through the library
        def chk2(res, out, rec=rec, name=name, arg=arg):
            return None if res == 0 else f"error {it.err_name(res)}"
        recs.append(it.Rec(OP_ENC, arg, rec, chk2, f"encode2 {name}"))
    results = it.run_suite(runner, s, recs)
    # library decode of the library's own files
    recs2 = []
    for r, res in zip(recs, results):
        if r.op == OP_ENC and r.name.startswith("encode2") and res and res[0] == 0:
            src = parse_docrec_input(r.data)

            def chk3(rr, out, src=src):
                if rr != 0:
                    return f"error {it.err_name(rr)}"
                got = parse_docrec(out)
                for i, (a, b) in enumerate(zip(src.layers, got.layers)):
                    if (a["name"], a["opacity"], a["visible"], a["blend"], a["isbg"]) != \
                            (b["name"], b["opacity"], b["visible"], b["blend"], b["isbg"]):
                        return f"layer {i} properties differ"
                    if a["fmt"] == 3 and not np.array_equal(a["px"], b["px"]):
                        return f"layer {i} pixels differ"
                    if a["meta"] != b["meta"]:
                        return f"layer {i} metadata differs"
                if src.icc and got.icc != src.icc:
                    return "ICC profile differs"
                return None
            recs2.append(it.Rec(OP_DEC, 0, res[1], chk3, "decode ours: " + r.name[8:]))
    it.run_suite(runner, s, recs2)
    return s


def parse_docrec_input(b):
    """The input doc record (make_docrec) as a DocRec (pixels in their stored format)."""
    return parse_docrec(b)


def suite_flat(it, runner, target):
    s = it.Suite("flat", target)
    rng = np.random.default_rng(3)
    recs = []
    px = rng.integers(0, 256, (45, 67, 4), np.uint8)

    def chk(res, out, px=px):
        if res != 0:
            return f"error {it.err_name(res)}"
        d = read_pdn(bytes(out))
        if len(d.layers) != 1 or not np.array_equal(d.layers[0].px, px):
            return "layer differs"
        if d.layers[0].name != "Background" or not d.layers[0].isbg:
            return f"layer {d.layers[0].name!r}"
        return None
    recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(it.C.IMGF_PDN) + it.surface_input(px, it.C.FMT_ARGB), chk,
                       "img_encode ARGB"))
    recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(it.C.IMGF_PDN, effort=9) + it.surface_input(it.premul(px), it.C.FMT_PARGB),
                       lambda r, o: None if r == 0 and np.array_equal(read_pdn(bytes(o)).layers[0].px,
                                                                    it.unpremul(it.premul(px))) else f"r {r}",
                       "img_encode PARGB effort 9"))
    c = (SAMPLES / "c_layers.pdn").read_bytes()
    recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(c, cancel=1), it.expect_err(it.C.ERR_CANCEL), "cancel"))
    recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(c, maxpx=800 * 600 - 1), it.expect_err(it.C.ERR_LIMIT),
                       "maxpx"))
    recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(c, maxpx=800 * 600, flags=it.C.IMGD_STRAIGHT),
                       it.expect_decex(composite(read_pdn(c))), "maxpx exact"))
    it.run_suite(runner, s, recs)
    return s


# ---------------------------------------------------------------- NRBF structural mutants

def lps(b):
    if isinstance(b, str):
        b = b.encode("utf-8")
    out = bytearray()
    n = len(b)
    while True:
        c = n & 0x7F
        n >>= 7
        out.append(c | (0x80 if n else 0))
        if not n:
            break
    return bytes(out) + b


def n_hdr(root=1):
    return b"\x00" + struct.pack("<iiii", root, -1, 1, 0)


def n_sysclass(oid, name, members, types, values):
    """SystemClassWithMembersAndTypes: types = [(bt, info)] with info bytes."""
    out = b"\x04" + struct.pack("<i", oid) + lps(name) + struct.pack("<i", len(members))
    out += b"".join(lps(m) for m in members)
    out += bytes(t[0] for t in types) + b"".join(t[1] for t in types)
    return out + values


def n_str(oid, s):
    return b"\x06" + struct.pack("<i", oid) + lps(s)


END = b"\x0b"


def nrbf_mutants():
    """(name, NRBF bytes, expected error codes or None = any result but no crash)."""
    E = None
    out = []
    I32 = (0, b"\x08")
    OBJ = (2, b"")
    ok_min = n_hdr() + n_sysclass(1, "X", [], [], b"") + END
    out.append(("minimal", ok_min, [0]))
    out.append(("empty", b"", ["TRUNC"]))
    out.append(("header only", n_hdr(), ["TRUNC"]))
    out.append(("no header", n_sysclass(1, "X", [], [], b"") + END, ["HDR"]))
    out.append(("two headers", n_hdr() + n_hdr() + END, ["HDR"]))
    out.append(("root missing", n_hdr(5) + n_sysclass(1, "X", [], [], b"") + END, ["HDR"]))
    out.append(("int member", n_hdr() + n_sysclass(1, "X", ["a"], [I32], struct.pack("<i", 7)) + END, [0]))
    out.append(("7-bit length 6 bytes", n_hdr() + b"\x06\x01\x00\x00\x00\x80\x80\x80\x80\x80\x01" + END, ["TRUNC"]))
    out.append(("string 2^31-1", n_hdr() + b"\x06\x01\x00\x00\x00\xff\xff\xff\xff\x07" + END, ["TRUNC"]))
    out.append(("string 2^31", n_hdr() + b"\x06\x01\x00\x00\x00\x80\x80\x80\x80\x08" + END, ["TRUNC"]))
    out.append(("member count 2^31-1", n_hdr() + b"\x04\x01\x00\x00\x00" + lps("X") + struct.pack("<i", 0x7FFFFFFF) + END,
                ["HDR", "TRUNC"]))
    out.append(("member count -1", n_hdr() + b"\x04\x01\x00\x00\x00" + lps("X") + struct.pack("<i", -1) + END, ["HDR"]))
    out.append(("array 2^31-1", n_hdr() + b"\x10" + struct.pack("<ii", 1, 0x7FFFFFFF) + END, ["LIMIT"]))
    out.append(("array -1", n_hdr() + b"\x10" + struct.pack("<ii", 1, -1) + END, ["HDR"]))
    out.append(("prim array 2^31-1", n_hdr() + b"\x0f" + struct.pack("<ii", 1, 0x7FFFFFFF) + b"\x08" + END, ["TRUNC"]))
    out.append(("prim array of Null", n_hdr() + b"\x0f" + struct.pack("<ii", 1, 1000000) + b"\x11" + END, ["HDR"]))
    out.append(("prim array bad type", n_hdr() + b"\x0f" + struct.pack("<ii", 1, 1) + b"\x04" + END, ["HDR"]))
    out.append(("nulls past the end", n_hdr() + b"\x10" + struct.pack("<ii", 1, 3) + b"\x0d\x05" + END, ["HDR"]))
    out.append(("nulls 0", n_hdr() + b"\x10" + struct.pack("<ii", 1, 3) + b"\x0d\x00" + END, ["HDR"]))
    out.append(("nulls ok", n_hdr() + b"\x10" + struct.pack("<ii", 1, 300) + b"\x0e" + struct.pack("<i", 300) + END, [0]))
    out.append(("nulls in member", n_hdr() + n_sysclass(1, "X", ["a"], [OBJ], b"\x0d\x02") + END, ["HDR"]))
    out.append(("binary array rank 0", n_hdr() + b"\x07" + struct.pack("<iBi", 1, 0, 0) + END, ["HDR"]))
    out.append(("binary array rank 99", n_hdr() + b"\x07" + struct.pack("<iBi", 1, 0, 99) + END, ["HDR"]))
    out.append(("binary array product", n_hdr() + b"\x07" + struct.pack("<iBiii", 1, 2, 2, 100000, 100000) + b"\x02" + END,
                ["LIMIT"]))
    out.append(("binary array offset kind", n_hdr() + b"\x07" + struct.pack("<iBiiii", 1, 3, 2, 2, 1, 0) + struct.pack("<i", 0)
                + b"\x00\x08" + struct.pack("<ii", 1, 2) + END, ["TRUNC", "HDR", 0]))
    out.append(("binary array 2x2 int", n_hdr() + b"\x07" + struct.pack("<iBiii", 1, 2, 2, 2, 2) + b"\x00\x08"
                + struct.pack("<iiii", 1, 2, 3, 4) + END, [0]))
    out.append(("dup id", n_hdr() + n_str(1, "a") + n_str(1, "b") + END, ["HDR"]))
    out.append(("dup id value types", n_hdr() + n_sysclass(1, "X", ["a", "b"], [OBJ, OBJ], n_str(-2, "a") + n_str(-2, "b")) + END,
                ["HDR"]))
    out.append(("extreme ids", n_hdr(-0x80000000) + n_str(-0x80000000, "a") + n_str(0x7FFFFFFF, "b") + n_str(0, "c") + END, [0]))
    out.append(("dangling ref", n_hdr() + n_sysclass(1, "X", ["a"], [OBJ], b"\x09" + struct.pack("<i", 99)) + END, ["HDR"]))
    out.append(("self ref", n_hdr() + n_sysclass(1, "X", ["a"], [OBJ], b"\x09" + struct.pack("<i", 1)) + END, [0]))
    cyc = (n_hdr() + n_sysclass(1, "A", ["b"], [OBJ], b"\x09" + struct.pack("<i", 2))
           + n_sysclass(2, "B", ["a"], [OBJ], b"\x09" + struct.pack("<i", 1)) + END)
    out.append(("reference cycle", cyc, [0]))
    out.append(("ClassWithId unknown", n_hdr() + b"\x01" + struct.pack("<ii", 1, 77) + END, ["HDR"]))
    out.append(("ClassWithId of string", n_hdr() + n_str(2, "s") + b"\x01" + struct.pack("<ii", 1, 2) + END, ["HDR"]))
    out.append(("ClassWithId ok", n_hdr() + n_sysclass(1, "X", ["a"], [I32], struct.pack("<i", 1))
                + b"\x01" + struct.pack("<ii", 2, 1) + struct.pack("<i", 5) + END, [0]))
    out.append(("unknown library", n_hdr() + b"\x05" + struct.pack("<i", 1) + lps("X") + struct.pack("<i", 0)
                + struct.pack("<i", 42) + END, ["HDR"]))
    out.append(("library then class", n_hdr() + b"\x0c" + struct.pack("<i", 42) + lps("Lib")
                + b"\x05" + struct.pack("<i", 1) + lps("X") + struct.pack("<i", 0) + struct.pack("<i", 42) + END, [0]))
    out.append(("dup library", n_hdr() + b"\x0c" + struct.pack("<i", 4) + lps("L") + b"\x0c" + struct.pack("<i", 4) + lps("M")
                + n_str(1, "x") + END, ["HDR"]))
    out.append(("MethodCall", n_hdr() + b"\x15" + END, ["UNSUPPORTED"]))
    out.append(("MethodReturn", n_hdr() + b"\x16" + END, ["UNSUPPORTED"]))
    for rt in (18, 19, 20, 23, 99, 255):
        out.append((f"record type {rt}", n_hdr() + bytes([rt]) + END, ["HDR"]))
    out.append(("end in member", n_hdr() + n_sysclass(1, "X", ["a"], [OBJ], END) + END, ["HDR"]))
    out.append(("bad binary type", n_hdr() + n_sysclass(1, "X", ["a"], [(9, b"")], b"") + END, ["HDR"]))
    out.append(("bad prim type", n_hdr() + n_sysclass(1, "X", ["a"], [(0, b"\x04")], b"") + END, ["HDR"]))
    out.append(("char members", n_hdr() + n_sysclass(1, "X", ["a", "b", "c"], [(0, b"\x03")] * 3, "aé€".encode()) + END, [0]))
    out.append(("char truncated", n_hdr() + n_sysclass(1, "X", ["a"], [(0, b"\x03")], b"\xf0\x9f"), ["TRUNC"]))
    out.append(("decimal member", n_hdr() + n_sysclass(1, "X", ["a"], [(0, b"\x05")], lps("1.5")) + END, [0]))
    out.append(("primtyped top level", n_hdr() + b"\x08\x08" + struct.pack("<i", 1) + END, ["HDR"]))
    # deep nesting
    for depth in (60, 64, 65, 10000):
        body = b"\x0a"
        for k in range(depth - 1, 0, -1):
            body = n_sysclass(k + 1, "N", ["m"], [OBJ], body)
        body = n_sysclass(1, "N", ["m"], [OBJ], body)
        out.append((f"nesting {depth}", n_hdr() + body + END, [0] if depth < 64 else ["LIMIT"]))
    # object count limit
    many = n_hdr() + b"".join(b"\x06" + struct.pack("<i", i + 1) + b"\x00" for i in range((1 << 20) + 1)) + END
    out.append(("1M+1 objects", many, ["LIMIT"]))
    return out


def suite_nrbf(it, runner, target):
    s = it.Suite("nrbf", target)
    recs = []
    for name, data, exp in nrbf_mutants():
        codes = [0 if e == 0 else getattr(it.C, "ERR_" + e) for e in exp]

        def chk(res, out, codes=codes):
            return None if res in codes else f"result {it.err_name(res)}, expected {'/'.join(map(it.err_name, codes))}"
        recs.append(it.Rec(OP_NRBF, 0, data, chk, name))
    # table budget
    recs.append(it.Rec(OP_NRBF, 4096, n_hdr() + b"".join(n_str(i + 1, "abc") for i in range(2000)) + END,
                       it.expect_err(it.C.ERR_LIMIT), "maxmem 4096"))
    # cross-check the object counts of the samples with the Python reader
    for f in sorted(SAMPLES.glob("*.pdn")):
        data = f.read_bytes()
        d = read_pdn(data)
        nb = nrbf_bytes(data)

        def chk(res, out, d=d, nb=nb):
            if res != 0:
                return f"error {it.err_name(res)}"
            n, root, end = struct.unpack("<IiQ", out)
            if n != len(d.nrbf.objs) or end != len(nb) or root != d.nrbf.order.index(d.nrbf.root):
                return f"objects {n} root {root} end {end} vs {len(d.nrbf.objs)} {len(nb)}"
            return None
        recs.append(it.Rec(OP_NRBF, 0, nb, chk, f"sample {f.name}"))
    it.run_suite(runner, s, recs)
    return s


def pdn_wrap(nrbf, payload=b"", xml=b'<pdnImage width="1" height="1" layers="1" />'):
    return b"PDN3" + struct.pack("<I", len(xml))[:3] + xml + b"\x00\x01" + nrbf + payload


def suite_fuzz(it, runner, target, n, seed):
    import random
    s = it.Suite("fuzz", target)
    rng = random.Random(seed)
    recs = []
    base = [(SAMPLES / "a_1x1.pdn").read_bytes(), (SAMPLES / "c_layers.pdn").read_bytes()]
    small = base[0]
    # every truncation of the small file (header, NRBF, payload)
    for cut in range(0, len(small), 7):
        recs.append(it.Rec(OP_DEC, 0, small[:cut], None, f"trunc {cut}"))
    # truncations of the payload area of c_layers
    c = base[1]
    for k in range(40):
        recs.append(it.Rec(OP_DEC, 0, c[:rng.randrange(len(c) - 200000 if len(c) > 200000 else 1, len(c))], None,
                           f"trunc c {k}"))
    # random mutations
    for k in range(n):
        src = base[k % 2] if k % 5 else small
        data = it.mutate(rng, src)
        recs.append(it.Rec(OP_DEC, 0, data, None, f"mutate {k}"))
    # NRBF-aware: overwrite i32 fields inside the NRBF part with extreme values
    d0 = read_pdn(small)
    a, b = d0.nrbf_span
    for k in range(n // 2):
        data = bytearray(small if k % 3 else c)
        dd = d0 if k % 3 else read_pdn(c)
        a2, b2 = dd.nrbf_span
        for _ in range(rng.randrange(1, 4)):
            p = rng.randrange(a2, b2 - 4)
            v = rng.choice([0, 1, -1, 0x7FFFFFFF, -0x80000000, 0x10000, 255, 256, 65536, rng.randrange(1 << 32) - (1 << 31)])
            data[p:p + 4] = struct.pack("<i", v)
        recs.append(it.Rec(OP_DEC, 0, bytes(data), None, f"nrbf i32 {k}"))
    # payload chunk mutants: index / size / chunk size fields
    for k in range(200):
        data = bytearray(c)
        dd = read_pdn(c)
        blk = dd.blocks[rng.randrange(len(dd.blocks))]
        fmt, ch, chunks = blk
        idx, size, off = chunks[rng.randrange(len(chunks))]
        field = rng.randrange(3)
        v = rng.choice([0, 1, 7, 0xFFFFFFFF, 0x7FFFFFFF, 262144, 262143, size + 1, size - 1, rng.randrange(1 << 32)])
        if field == 0:
            data[off - 8:off - 4] = struct.pack(">I", v)
        elif field == 1:
            data[off - 4:off] = struct.pack(">I", v)
        else:
            data[off + rng.randrange(size)] ^= 1 << rng.randrange(8)
        recs.append(it.Rec(OP_DEC, 0, bytes(data), None, f"chunk {k}"))
    # legacy gzip wrappers
    import gzip as _gz
    nb = nrbf_bytes(small)
    recs.append(it.Rec(OP_DEC, 0, _gz.compress(nb + small[b:]), None, "bare gzip"))
    recs.append(it.Rec(OP_DEC, 0, small[:a - 2] + _gz.compress(small[a:]), None, "PDN3 + gzip"))
    recs.append(it.Rec(OP_DEC, 0, b"\x1f\x8b\x08\x00" + bytes(20), None, "gzip garbage"))
    # the flat path too
    for k in range(100):
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(it.mutate(rng, small), flags=it.C.IMGD_STRAIGHT), None,
                           f"decex mutate {k}"))
    t0 = time.time()
    it.run_suite(runner, s, recs)
    s.notes.append(f"{len(recs)} cases in {time.time() - t0:.1f}s")
    return s


def suite_legacy(it, runner, target):
    """gzip-wrapped variants built from a sample must decode like the sample."""
    import gzip as _gz
    s = it.Suite("legacy", target)
    small = (SAMPLES / "c_layers.pdn").read_bytes()
    ref = read_pdn(small)
    a, b = ref.nrbf_span

    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        got = parse_docrec(out)
        return expect_doc_equal(ref, got, "legacy", check_xml=False)
    recs = [it.Rec(OP_DEC, 0, small[:a - 2] + _gz.compress(small[a:]), chk, "PDN3 + 1F 8B"),
            it.Rec(OP_DEC, 0, _gz.compress(small[a:]), chk, "bare gzip (<= 2.0)")]
    it.run_suite(runner, s, recs)
    return s


# ---------------------------------------------------------------- synthetic paint.net 3.x style file

def n_class(oid, name, members, types, lib, values):
    """ClassWithMembersAndTypes (record 5)."""
    out = b"\x05" + struct.pack("<i", oid) + lps(name) + struct.pack("<i", len(members))
    out += b"".join(lps(m) for m in members)
    out += bytes(t[0] for t in types) + b"".join(t[1] for t in types)
    return out + struct.pack("<i", lib) + values


def n_ref(i):
    return b"\x09" + struct.pack("<i", i)


def t_cls(name, lib):
    return (4, lps(name) + struct.pack("<i", lib))


def t_sys(name):
    return (3, lps(name))


def make_v3_pdn(px, chunks=1, blend="Multiply", deferred=False):
    """A .pdn in the paint.net 3.x shape: NameValueCollection metadata (ArrayList and plain
    string values), no LayerProperties.blendMode (the mode is the blendOp class), a
    MemoryBlock with inline chunk0..chunkN byte arrays (or one deferred raw payload)."""
    h, w = px.shape[:2]
    raw = px.tobytes()
    I32, BOOL, I64, U8 = (0, b"\x08"), (0, b"\x01"), (0, b"\x09"), (0, b"\x02")
    OBJ, STR, OBJARR, STRARR = (2, b""), (1, b""), (5, b""), (6, b"")
    lib = b"\x0c" + struct.pack("<i", 90) + lps("PaintDotNet.Data, Version=3.36.3158.38064, Culture=neutral, "
                                                  "PublicKeyToken=null")
    s = n_hdr(1) + lib
    s += n_class(1, "PaintDotNet.Document", ["layers", "width", "height", "userMetaData", "savedWith"],
                 [t_cls("PaintDotNet.LayerList", 90), I32, I32,
                  t_sys("System.Collections.Specialized.NameValueCollection"), t_sys("System.Version")], 90,
                 n_ref(2) + struct.pack("<ii", w, h) + n_ref(3) + n_ref(4))
    s += n_class(2, "PaintDotNet.LayerList", ["parent", "ArrayList+_items", "ArrayList+_size", "ArrayList+_version"],
                 [t_cls("PaintDotNet.Document", 90), OBJARR, I32, I32], 90,
                 n_ref(1) + n_ref(5) + struct.pack("<ii", 1, 1))
    s += n_sysclass(3, "System.Collections.Specialized.NameValueCollection",
                    ["ReadOnly", "HashProvider", "Comparer", "Count", "Keys", "Values", "Version"],
                    [BOOL, OBJ, OBJ, I32, STRARR, OBJARR, I32],
                    b"\x00" + b"\x0a" + b"\x0a" + struct.pack("<i", 3) + n_ref(6) + n_ref(7) + struct.pack("<i", 3))
    s += n_sysclass(4, "System.Version", ["_Major", "_Minor", "_Build", "_Revision"], [I32] * 4,
                    struct.pack("<iiii", 3, 36, 3158, 38064))
    s += b"\x10" + struct.pack("<ii", 5, 4) + n_ref(8) + b"\x0d\x03"
    s += b"\x11" + struct.pack("<ii", 6, 3) + n_str(9, "$exif.tag0") + n_str(10, "$exif.tag1") + n_str(20, "custom")
    s += b"\x10" + struct.pack("<ii", 7, 3) + n_ref(11) + n_str(21, exif_item(282, 5, struct.pack("<II", 50, 1))) \
        + n_str(22, "Wert ö")
    s += n_class(8, "PaintDotNet.BitmapLayer", ["properties", "surface", "Layer+isDisposed", "Layer+width",
                                               "Layer+height", "Layer+properties"],
                 [t_cls("PaintDotNet.BitmapLayer+BitmapLayerProperties", 90), t_cls("PaintDotNet.Surface", 90), BOOL,
                  I32, I32, t_cls("PaintDotNet.Layer+LayerProperties", 90)], 90,
                 n_ref(12) + n_ref(13) + b"\x00" + struct.pack("<ii", w, h) + n_ref(14))
    s += n_sysclass(11, "System.Collections.ArrayList", ["_items", "_size", "_version"], [OBJARR, I32, I32],
                    n_ref(15) + struct.pack("<ii", 1, 1))
    s += n_class(12, "PaintDotNet.BitmapLayer+BitmapLayerProperties", ["blendOp"],
                 [t_cls(f"PaintDotNet.UserBlendOps+{blend}BlendOp", 90)], 90, n_ref(16))
    s += n_class(13, "PaintDotNet.Surface", ["width", "height", "stride", "scan0"],
                 [I32, I32, I32, t_cls("PaintDotNet.MemoryBlock", 90)], 90,
                 struct.pack("<iii", w, h, w * 4) + n_ref(17))
    s += n_class(14, "PaintDotNet.Layer+LayerProperties", ["name", "userMetaData", "visible", "isBackground",
                                                          "opacity"],
                 [STR, t_sys("System.Collections.Specialized.NameValueCollection"), BOOL, BOOL, U8], 90,
                 n_str(18, "Hintergrund 3.x") + b"\x0a" + b"\x01\x01" + bytes([200]))
    s += b"\x10" + struct.pack("<ii", 15, 1) + n_str(19, exif_item(296, 3, struct.pack("<H", 3)))
    s += n_class(16, f"PaintDotNet.UserBlendOps+{blend}BlendOp", [], [], 90, b"")
    if deferred:
        s += n_class(17, "PaintDotNet.MemoryBlock", ["length64", "hasParent", "deferred"], [I64, BOOL, BOOL], 90,
                     struct.pack("<q", len(raw)) + b"\x00\x01")
        payload = b"\x01" + struct.pack(">I", len(raw)) + struct.pack(">II", 0, len(raw)) + raw
    else:
        step = (len(raw) + chunks - 1) // chunks
        names = [f"chunk{i}" for i in range(chunks)]
        s += n_class(17, "PaintDotNet.MemoryBlock", ["length64", "hasParent", "deferred"] + names,
                     [I64, BOOL, BOOL] + [(7, b"\x02")] * chunks, 90,
                     struct.pack("<q", len(raw)) + b"\x00\x00" + b"".join(n_ref(30 + i) for i in range(chunks)))
        for i in range(chunks):
            part = raw[i * step:(i + 1) * step]
            s += b"\x0f" + struct.pack("<ii", 30 + i, len(part)) + b"\x02" + part
        payload = b""
    s += END
    return pdn_wrap(s, payload, xml=f'<pdnImage width="{w}" height="{h}" layers="1" />'.encode())


def suite_v3(it, runner, target):
    s = it.Suite("v3", target)
    rng = np.random.default_rng(11)
    recs = []
    for name, kw in (("inline 1 chunk", dict(chunks=1)), ("inline 3 chunks", dict(chunks=3)),
                     ("deferred raw", dict(deferred=True)), ("Overlay", dict(blend="Overlay")),
                     ("Xor", dict(blend="Xor"))):
        px = rng.integers(0, 256, (13, 21, 4), np.uint8)
        data = make_v3_pdn(px, **kw)
        ref = read_pdn(data)
        mode = PDN_BLEND_NAMES.index(kw.get("blend", "Multiply"))

        def chk(res, out, px=px, mode=mode):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = parse_docrec(out)
            L = d.layers[0]
            if not np.array_equal(L["px"], px):
                return "pixels differ"
            if (L["name"], L["blend"], L["opacity"], L["isbg"]) != ("Hintergrund 3.x", mode, 200, 1):
                return f"layer {L['name']} blend {L['blend']} opacity {L['opacity']}"
            items = nvc_items(d.nvc)
            if [k for k, _ in items] != ["$exif.tag0", "$exif.tag1", "custom"] or items[2][1] != "Wert ö":
                return f"metadata {items}"
            if (d.unit, d.dpux) != (3, 50.0):
                return f"resolution {d.unit} {d.dpux}"
            return None
        recs.append(it.Rec(OP_DEC, 0, data, chk, f"3.x {name}"))
        if ref.layers[0].blend != mode:
            s.fail(f"python reader: 3.x {name} blend {ref.layers[0].blend}")
    it.run_suite(runner, s, recs)
    return s


# ---------------------------------------------------------------- .NET BinaryFormatter cross-check

DOTNET_CS = r"""
using System;
using System.IO;
using System.Collections.Generic;
using System.Runtime.Serialization;
using System.Runtime.Serialization.Formatters.Binary;

[Serializable]
public class PdnDump : ISerializable {
    public string TypeName;
    public Dictionary<string, object> M = new Dictionary<string, object>();
    public PdnDump(SerializationInfo info, StreamingContext ctx) {
        TypeName = info.FullTypeName;
        foreach (SerializationEntry e in info) M[e.Name] = e.Value;
    }
    public void GetObjectData(SerializationInfo info, StreamingContext ctx) { }
}
public enum PdnEnum { }
public class PdnBinder : SerializationBinder {
    public override Type BindToType(string asm, string type) {
        if (type == "PaintDotNet.LayerBlendMode") return typeof(PdnEnum);
        if (type.StartsWith("PaintDotNet.")) return typeof(PdnDump);
        return null;
    }
}
public static class PdnCheck {
    public static string Run(byte[] data) {
        var bf = new BinaryFormatter();
        bf.Binder = new PdnBinder();
        var ms = new MemoryStream(data);
        object root = bf.Deserialize(ms);
        var doc = (PdnDump)root;
        var sb = new System.Text.StringBuilder();
        sb.Append(doc.TypeName).Append(' ').Append(doc.M["width"]).Append('x').Append(doc.M["height"]);
        sb.Append(" consumed ").Append(ms.Position).Append('/').Append(data.Length);
        var ll = (PdnDump)doc.M["layers"];
        object[] items = (object[])ll.M["ArrayList+_items"];
        int n = (int)ll.M["ArrayList+_size"];
        sb.Append(" layers ").Append(n);
        for (int i = 0; i < n; i++) {
            var bl = (PdnDump)items[i];
            var lp = (PdnDump)bl.M["Layer+properties"];
            var blp = (PdnDump)bl.M["properties"];
            var op = (PdnDump)blp.M["blendOp"];
            sb.Append(" |").Append(lp.M["name"]).Append(',').Append(lp.M["visible"]).Append(',');
            sb.Append(lp.M["opacity"]).Append(',').Append((int)(PdnEnum)lp.M["blendMode"]).Append(',');
            sb.Append(op.TypeName.Substring(op.TypeName.IndexOf('+') + 1));
            var sf = (PdnDump)bl.M["surface"];
            var mb = (PdnDump)sf.M["scan0"];
            sb.Append(',').Append(mb.M["length64"]);
        }
        return sb.ToString();
    }
}
"""


def dotnet_check(files):
    """Deserialize the NRBF part of every file with .NET's BinaryFormatter (Windows
    PowerShell 5.1) -> {path: summary string or 'ERROR ...'}."""
    import json
    import tempfile
    td = Path(tempfile.mkdtemp(prefix="pdn_dotnet_"))
    listing = []
    for i, f in enumerate(files):
        nb = nrbf_bytes(Path(f).read_bytes())
        p = td / f"s{i}.bin"
        p.write_bytes(nb)
        listing.append((str(f), str(p)))
    (td / "check.cs").write_text(DOTNET_CS, encoding="utf-8")
    ps = td / "check.ps1"
    lines = ["$ErrorActionPreference = 'Stop'",
             f"Add-Type -TypeDefinition ([IO.File]::ReadAllText('{td / 'check.cs'}'))",
             "$out = @{}"]
    for orig, p in listing:
        lines.append(f"try {{ $out['{orig}'] = [PdnCheck]::Run([IO.File]::ReadAllBytes('{p}')) }} "
                     f"catch {{ $out['{orig}'] = 'ERROR ' + $_.Exception.ToString() }}")
    lines.append("$out | ConvertTo-Json -Compress | Out-File -Encoding utf8 '" + str(td / "out.json") + "'")
    ps.write_text("\n".join(lines), encoding="utf-8-sig")
    p = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                        "-File", str(ps)], capture_output=True, text=True, timeout=600)
    outp = td / "out.json"
    if not outp.exists():
        raise RuntimeError("PowerShell failed: " + (p.stdout + p.stderr)[-2000:])
    return json.loads(outp.read_text(encoding="utf-8-sig"))


def suite_dotnet(it, runner, target, outdir):
    s = it.Suite("dotnet", target)
    ours = []
    rt_recs = [it.Rec(OP_RT, 1 << 8, f.read_bytes(), None, f.name) for f in sorted(SAMPLES.glob("*.pdn"))]
    res = runner.batch(rt_recs)
    od = Path(outdir) / "dotnet"
    od.mkdir(parents=True, exist_ok=True)
    for r, x in zip(rt_recs, res):
        if x and x[0] == 0:
            p = od / ("ours_" + r.name)
            p.write_bytes(bytes(x[1]))
            ours.append(p)
    ud = Path(outdir) / "for_user"
    user = sorted(ud.glob("*.pdn")) if ud.exists() else []
    files = sorted(SAMPLES.glob("*.pdn")) + ours + user
    try:
        out = dotnet_check(files)
    except Exception as e:  # noqa: BLE001
        s.notes.append(f"skipped: {e}")
        return s
    for f in files:
        v = out.get(str(f), "missing")
        s.check(not v.startswith("ERROR") and v != "missing", f"{Path(f).name}: {v[:300]}")
        s.notes.append(f"{Path(f).name}: {v[:200]}")
    # our re-encoded samples must look exactly like the originals to .NET
    for f in sorted(SAMPLES.glob("*.pdn")):
        o = od / ("ours_" + f.name)
        if o.exists():
            s.check(out.get(str(f)) == out.get(str(o)), f"{f.name}: .NET sees a different graph in our file")
    return s


# ---------------------------------------------------------------- files for the user

def user_files(it, runner, outdir):
    od = Path(outdir) / "for_user"
    od.mkdir(parents=True, exist_ok=True)
    W, H = 400, 300
    yy, xx = np.mgrid[0:H, 0:W]
    bg = np.zeros((H, W, 4), np.uint8)
    bg[...] = (255, 255, 255, 255)
    bg[(xx // 20 + yy // 20) % 2 == 0] = (230, 230, 230, 255)
    red = np.zeros((H, W, 4), np.uint8)
    m = (xx - 150) ** 2 + (yy - 150) ** 2 < 90 ** 2
    red[m] = (40, 40, 220, 255)
    grad = np.zeros((H, W, 4), np.uint8)
    grad[..., 0] = 255
    grad[..., 1] = (xx * 255 // W).astype(np.uint8)
    grad[..., 3] = np.where((yy > 100) & (yy < 250), 255, 0)
    semi = np.zeros((H, W, 4), np.uint8)
    semi[60:240, 220:380] = (0, 160, 0, 128)
    files = {}
    files["cp_3layers.pdn"] = (make_docrec(W, H, [
        dict(name="Hintergrund", px=bg, isbg=1),
        dict(name="Ebene Ä (versteckt)", px=red, visible=0),
        dict(name="Multiplizieren 50%", px=grad, opacity=128, blend=1),
        dict(name="Halbtransparent ß", px=semi)], unit=2, dpux=300.0, dpuy=300.0), 1 << 8)
    names = ["Normal", "Multiplizieren", "Additiv", "Farbig nachbelichten", "Farbig abwedeln", "Reflektieren",
             "Leuchten", "Überlagern", "Differenz", "Negation", "Aufhellen", "Abdunkeln", "Negativ multiplizieren",
             "Xor"]
    lay = [dict(name="Hintergrund", px=bg, isbg=1)]
    for mnum in range(14):
        p = np.zeros((H, W, 4), np.uint8)
        y0 = 10 + mnum * 20
        p[y0:y0 + 16, 20:380] = (60 + mnum * 10, 120, 200 - mnum * 10, 255)
        lay.append(dict(name=f"{mnum} {PDN_BLEND_NAMES[mnum]} = {names[mnum]}", px=p, blend=mnum))
    files["cp_blendmodes.pdn"] = (make_docrec(W, H, lay), 1 << 8)
    files["cp_blendmodes_blendop.pdn"] = (make_docrec(W, H, lay), 1 << 8 | PDNE_BLENDOP)
    odd = np.zeros((777, 1001, 4), np.uint8)
    y2, x2 = np.mgrid[0:777, 0:1001]
    odd[..., 2] = (x2 * 255 // 1000).astype(np.uint8)
    odd[..., 1] = (y2 * 255 // 776).astype(np.uint8)
    odd[..., 3] = ((x2 + y2) * 255 // 1776).astype(np.uint8)
    files["cp_1001x777_alpha.pdn"] = (make_docrec(1001, 777, [dict(name="Verlauf mit Alpha", px=odd)],
                                                  unit=3, dpux=118.1102, dpuy=118.1102), 1 << 8)
    files["cp_1x1.pdn"] = (make_docrec(1, 1, [dict(name="Hintergrund", px=np.array([[[0, 0, 255, 255]]], np.uint8),
                                                    isbg=1)]), 1 << 8)
    recs = [it.Rec(OP_ENC, arg, rec, None, name) for name, (rec, arg) in files.items()]
    recs.append(it.Rec(OP_RT, 1 << 8, (SAMPLES / "c_layers.pdn").read_bytes(), None, "cp_c_layers_reencoded.pdn"))
    recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(it.C.IMGF_PDN) + it.surface_input(odd, it.C.FMT_ARGB), None,
                       "cp_img_encode_flat.pdn"))
    written = []
    for r, x in zip(recs, runner.batch(recs)):
        if x and x[0] == 0:
            (od / r.name).write_bytes(bytes(x[1]))
            written.append(r.name)
            read_pdn(bytes(x[1]))           # must parse
    return written


# ---------------------------------------------------------------- performance

def suite_perf(it, runner, target):
    s = it.Suite("perf", target)
    big = (SAMPLES / "d_big.pdn").read_bytes()
    runs = [runner.one(OP_BDEC, 1, big) for _ in range(3)]
    r = min(runs, key=lambda x: struct.unpack_from("<Q", x[1])[0] if x[0] == 0 else 1 << 62)
    if r[0] == 0:
        us = struct.unpack_from("<Q", r[1])[0]
        s.check(us < 1_500_000, f"d_big decode {us / 1e3:.0f} ms >= 1500 ms")
        s.notes.append(f"d_big.pdn decode: {us / 1e3:.0f} ms")
    else:
        s.fail(f"d_big bench error {it.err_name(r[0])}")
    # the machine may be shared: best of five runs (the first also verifies the file)
    r = runner.one(OP_BENC, 1 | 256, struct.pack("<IIII", 4000, 3000, 10, 1))
    for _ in range(4):
        r2 = runner.one(OP_BENC, 1, struct.pack("<IIII", 4000, 3000, 10, 1))
        if r[0] == 0 and r2[0] == 0 and struct.unpack_from("<Q", r2[1])[0] < struct.unpack_from("<Q", r[1])[0]:
            r = (0, r2[1][:16] + r[1][16:24])
    if r[0] == 0:
        us, size, dus = struct.unpack_from("<QQQ", r[1])
        s.check(us < 4_000_000, f"encode 4000x3000x10 {us / 1e3:.0f} ms >= 4000 ms")
        s.notes.append(f"4000x3000x10 encode (level 1): {us / 1e3:.0f} ms, {size / 1e6:.1f} MB; "
                       f"decode + verify: {dus / 1e3:.0f} ms (exact)")
    else:
        s.fail(f"encode bench error {it.err_name(r[0])}")
    return s


def check_main():
    import imgtest_common as it
    ap = it.argparser(__doc__, "samples,roundtrip,flat,nrbf,legacy,v3,fuzz,dotnet,user,perf")
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    if not a.no_build:
        it.build_codectest(targets, out)
    only = set(x for x in a.only.split(",") if x)
    suites = []
    for t in targets:
        runner = it.Runner(t, out, work_name="pdncheck")

        def want(n):
            return not only or n in only
        if want("samples"):
            suites.append(suite_samples(it, runner, t))
        if want("roundtrip"):
            suites.append(suite_roundtrip(it, runner, t))
        if want("flat"):
            suites.append(suite_flat(it, runner, t))
        if want("legacy"):
            suites.append(suite_legacy(it, runner, t))
        if want("v3"):
            suites.append(suite_v3(it, runner, t))
        if want("nrbf"):
            suites.append(suite_nrbf(it, runner, t))
        if want("fuzz"):
            suites.append(suite_fuzz(it, runner, t, a.fuzz, a.seed))
        if t == "win" and want("user"):
            w = user_files(it, runner, out / "cp" if out.name != "cp" else out)
            su = it.Suite("user", t)
            su.check(len(w) == 7, f"only {len(w)} of 7 files written")
            su.notes.append("files: " + ", ".join(w))
            suites.append(su)
        if t == "win" and want("dotnet") and not a.no_ext:
            suites.append(suite_dotnet(it, runner, t, out if out.name == "cp" else out / "cp"))
        if want("perf"):
            suites.append(suite_perf(it, runner, t))
    return 0 if it.report(suites, "pdn_check") else 1


if __name__ == "__main__":
    if any(x in sys.argv for x in ("--dump", "--graph", "--png", "--compare")):
        sys.exit(cli_main(sys.argv[1:]))
    sys.exit(check_main())

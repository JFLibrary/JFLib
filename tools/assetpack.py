"""assetpack.py -- an app's assets folder as one bundle (JFA1) for app:// (lib/plat/vfs.rs)

    python tools/assetpack.py SRC_DIR OUT.jfa      pack a folder (build_rs.py does this for
                                                  apps/NAME/assets when it builds the app)
    python tools/assetpack.py --list FILE.jfa      the entries of a bundle

Format (little endian):
    "JFA1", u32 count, u32 names_len
    count entries of 20 bytes, sorted by name (bytewise):
        u32 name_off, u16 name_len, u8 codec (0 stored, 1 LZ4 block), u8 0,
        u32 data_off (from the start of the bundle), u32 stored_len, u32 size
    the names (UTF-8, '/' separated, relative), then the data
Names starting with '.' (files and folders) are left out.  A file is stored LZ4-compressed when
that saves at least a tenth.  The LZ4 compressor is the one of lib/compress/lz4.rs (greedy,
16K-entry hash table); every block is decoded again before it is written."""
import struct
import sys
from pathlib import Path

MAGIC = b"JFA1"
CODEC_STORED, CODEC_LZ4 = 0, 1
ENTRY = struct.Struct("<IHBBIII")
MAX_FILE = 0xFFFF_FFFF

MIN_MATCH, LAST_LITERALS, MF_LIMIT, HASH_LOG, MAX_DIST = 4, 5, 12, 14, 65535


def _put_len(out, n):
    while n >= 255:
        out.append(255)
        n -= 255
    out.append(n)


def _hash(v):
    return ((v * 2654435761) & 0xFFFFFFFF) >> (32 - HASH_LOG)


def lz4_compress(src):
    """one LZ4 block (the rules of lib/compress/lz4.rs: last 5 bytes literals, no match in the
    last 12 bytes)"""
    out = bytearray()
    n = len(src)

    def last(lits):
        ll = len(lits)
        out.append(min(ll, 15) << 4)
        if ll >= 15:
            _put_len(out, ll - 15)
        out.extend(lits)

    if n <= MF_LIMIT:
        last(src)
        return bytes(out)
    rd32 = lambda i: int.from_bytes(src[i:i + 4], "little")
    table = [0] * (1 << HASH_LOG)
    mflimit, match_end = n - MF_LIMIT, n - LAST_LITERALS
    anchor, ip, misses = 0, 1, 0
    while ip <= mflimit:
        seq = rd32(ip)
        h = _hash(seq)
        cand = table[h]
        table[h] = ip
        if cand < ip and ip - cand <= MAX_DIST and rd32(cand) == seq:
            s, c = ip, cand
            while s > anchor and c > 0 and src[s - 1] == src[c - 1]:
                s -= 1
                c -= 1
            length = MIN_MATCH + (ip - s)
            while s + length < match_end and src[c + length] == src[s + length]:
                length += 1
            lits = src[anchor:s]
            ll, ml = len(lits), length - MIN_MATCH
            out.append((min(ll, 15) << 4) | min(ml, 15))
            if ll >= 15:
                _put_len(out, ll - 15)
            out.extend(lits)
            out.extend((s - c).to_bytes(2, "little"))
            if ml >= 15:
                _put_len(out, ml - 15)
            ip = s + length
            anchor = ip
            misses = 0
            if ip >= 2 and ip - 2 <= mflimit:
                table[_hash(rd32(ip - 2))] = ip - 2
        else:
            misses += 1
            ip += 1 + (misses >> 6)
    last(src[anchor:])
    return bytes(out)


def lz4_decompress(src, size):
    """decode one block of exactly size bytes (ValueError on corrupt input)"""
    out = bytearray()
    i = 0

    def get_len(n):
        nonlocal i
        while True:
            b = src[i]
            i += 1
            n += b
            if b != 255:
                return n

    while True:
        token = src[i]
        i += 1
        ll = token >> 4
        if ll == 15:
            ll = get_len(ll)
        out += src[i:i + ll]
        i += ll
        if i == len(src):
            break
        off = int.from_bytes(src[i:i + 2], "little")
        i += 2
        if off == 0 or off > len(out):
            raise ValueError("bad offset")
        ml = token & 15
        if ml == 15:
            ml = get_len(ml)
        for _ in range(ml + MIN_MATCH):
            out.append(out[-off])
    if len(out) != size:
        raise ValueError("bad size")
    return bytes(out)


def collect(src_dir):
    """{name: path} of every file below src_dir, without names starting with '.'"""
    root = Path(src_dir)
    files = {}
    for p in sorted(root.rglob("*")):
        rel = p.relative_to(root)
        if any(part.startswith(".") for part in rel.parts) or not p.is_file():
            continue
        name = rel.as_posix()
        if p.stat().st_size > MAX_FILE:
            raise ValueError(f"{p}: larger than 4 GiB")
        files[name] = p
    return files


def pack(src_dir):
    """the bundle of a folder (bytes)"""
    return pack_files({name: p.read_bytes() for name, p in collect(src_dir).items()})


def pack_files(files, compress=True):
    """the bundle of {name: bytes} ('/' separated relative names; compress=False stores every
    file, which is much faster for large files)"""
    names = sorted(files, key=lambda n: n.encode("utf-8"))
    pool = bytearray()
    blobs = []
    for name in names:
        data = files[name]
        if len(data) > MAX_FILE:
            raise ValueError(f"{name}: larger than 4 GiB")
        packed = lz4_compress(data) if data and compress else b""
        if data and compress and len(packed) * 10 <= len(data) * 9:
            if lz4_decompress(packed, len(data)) != data:
                raise ValueError(f"{name}: LZ4 round trip failed")
            blobs.append((CODEC_LZ4, packed, len(data)))
        else:
            blobs.append((CODEC_STORED, data, len(data)))
        pool += name.encode("utf-8")
    head = 12 + ENTRY.size * len(names)
    data_off = head + len(pool)
    table = bytearray()
    off_name = 0
    for name, (codec, blob, size) in zip(names, blobs):
        n = len(name.encode("utf-8"))
        table += ENTRY.pack(off_name, n, codec, 0, data_off, len(blob), size)
        off_name += n
        data_off += len(blob)
    out = bytearray(MAGIC + struct.pack("<II", len(names), len(pool)))
    out += table + pool
    for _, blob, _ in blobs:
        out += blob
    if len(out) > MAX_FILE:
        raise ValueError("bundle larger than 4 GiB")
    return bytes(out)


def entries(bundle):
    """[(name, codec, stored_len, size)] of a bundle"""
    if bundle[:4] != MAGIC:
        raise ValueError("not a JFA1 bundle")
    count, pool_len = struct.unpack_from("<II", bundle, 4)
    pool = 12 + ENTRY.size * count
    out = []
    for k in range(count):
        no, nl, codec, _, _, stored, size = ENTRY.unpack_from(bundle, 12 + ENTRY.size * k)
        out.append((bundle[pool + no:pool + no + nl].decode("utf-8"), codec, stored, size))
    return out


def main(argv):
    if len(argv) == 2 and argv[0] == "--list":
        for name, codec, stored, size in entries(Path(argv[1]).read_bytes()):
            print(f"{size:>10} {stored:>10} {'lz4' if codec else '-  '} {name}")
        return 0
    if len(argv) != 2:
        print(__doc__)
        return 2
    data = pack(argv[0])
    Path(argv[1]).parent.mkdir(parents=True, exist_ok=True)
    Path(argv[1]).write_bytes(data)
    print(f"wrote {argv[1]}: {len(entries(data))} files, {len(data)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

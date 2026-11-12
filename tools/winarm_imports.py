"""The imports of Windows programs against the system's DLLs (ARM5, docs/ARM64.md).  Windows refuses to start a
program whose DLL or function is missing (0xC0000135 / 0xC0000139) without a word of its own, so this names
them first.  Every import comes from raw-dylib: the program states DLL and function itself.

    python tools/winarm_imports.py [--libs DIR] PROGRAM_OR_DIR...

Without --libs the DLLs of this machine's System32 answer (on windows-11-arm: the ARM64 ones, see
.github/workflows/arm64-windows.yml); --libs DIR takes the import libraries of an SDK instead, e.g.
"C:/Program Files (x86)/Windows Kits/10/Lib/10.0.26100.0/um/arm64" on an x64 PC (only the members for the
program's own processor count; a DLL without an import library there, such as combase.dll, is looked up in
System32 after all).  API sets (api-ms-win-*) are resolved by the loader, not by a file: listed,
not checked.  Exit code 1 when something is missing."""
import argparse
import os
import struct
import sys
from pathlib import Path

MACHINES = {0x8664: "x64", 0xAA64: "ARM64", 0xA641: "ARM64EC", 0x14C: "x86"}


class Pe:
    """the headers, sections and data directories of a PE image read from a file"""

    def __init__(self, path):
        self.data = d = Path(path).read_bytes()
        if d[:2] != b"MZ":
            raise ValueError(f"{path}: not a PE image")
        nt = struct.unpack_from("<I", d, 0x3C)[0]
        if d[nt:nt + 4] != b"PE\0\0":
            raise ValueError(f"{path}: not a PE image")
        self.machine = struct.unpack_from("<H", d, nt + 4)[0]
        nsec, = struct.unpack_from("<H", d, nt + 6)
        opt = nt + 24
        self.pe32plus = struct.unpack_from("<H", d, opt)[0] == 0x20B
        self.dirs = opt + (112 if self.pe32plus else 96)
        sh = opt + struct.unpack_from("<H", d, nt + 20)[0]
        self.secs = [struct.unpack_from("<IIII", d, sh + 40 * i + 8) for i in range(nsec)]   # vsize, va, rsize, ptr

    def off(self, rva):
        for vsize, va, rsize, ptr in self.secs:
            if va <= rva < va + max(vsize, rsize):
                return rva - va + ptr
        raise ValueError(f"RVA 0x{rva:x} outside the sections")

    def dir(self, i):
        return struct.unpack_from("<II", self.data, self.dirs + 8 * i)

    def cstr(self, o):
        return self.data[o:self.data.index(b"\0", o)].decode("ascii", "replace")

    def imports(self):
        """{dll (lower case): {function name or "#ordinal"}}"""
        rva, _ = self.dir(1)
        res = {}
        o = self.off(rva) if rva else None
        while o is not None:
            ilt, _, _, name, iat = struct.unpack_from("<IIIII", self.data, o)
            if not name:
                break
            funcs = res.setdefault(self.cstr(self.off(name)).lower(), set())
            t, w = self.off(ilt or iat), 8 if self.pe32plus else 4
            while True:
                v = struct.unpack_from("<Q" if w == 8 else "<I", self.data, t)[0]
                if not v:
                    break
                if v >> (w * 8 - 1):
                    funcs.add(f"#{v & 0xFFFF}")
                else:
                    funcs.add(self.cstr(self.off(v & 0x7FFF_FFFF) + 2))
                t += w
            o += 20
        return res

    def exports(self):
        """the exported names and "#ordinal"s"""
        rva, _ = self.dir(0)
        if not rva:
            return set()
        o = self.off(rva)
        base, nfun, nnames, _, anames = struct.unpack_from("<IIIII", self.data, o + 16)
        names = {self.cstr(self.off(struct.unpack_from("<I", self.data, self.off(anames) + 4 * i)[0])) for i in range(nnames)}
        return names | {f"#{base + i}" for i in range(nfun)}


def lib_exports(libdir, machine):
    """{dll (lower case): {names}} from the short import members of the import libraries in libdir"""
    res = {}
    for lib in Path(libdir).glob("*.[lL]ib"):
        d = lib.read_bytes()
        if d[:8] != b"!<arch>\n":
            continue
        o = 8
        while o + 60 <= len(d):
            size = int(d[o + 48:o + 58].decode("ascii", "replace").strip() or 0)
            m = d[o + 60:o + 60 + size]
            if m[:4] == b"\0\0\xff\xff" and struct.unpack_from("<H", m, 6)[0] == machine:
                ordinal, typ = struct.unpack_from("<HH", m, 16)
                sym, dll = m[20:].split(b"\0")[:2]
                names = res.setdefault(dll.decode("ascii", "replace").lower(), set())
                names.add(sym.decode("ascii", "replace"))
                if typ & 3 == 0 and (typ >> 2) & 7 == 0:     # IMPORT_CODE by ordinal
                    names.add(f"#{ordinal}")
            o += 60 + size + (size & 1)
    return res


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--libs", help="import libraries instead of System32's DLLs")
    ap.add_argument("programs", nargs="+")
    a = ap.parse_args()
    exes = []
    for p in map(Path, a.programs):
        exes += sorted(p.glob("*.exe")) + sorted(p.glob("*.dll")) if p.is_dir() else [p]
    sysdir = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32"
    libs, dlls, bad, fallback = {}, {}, 0, set()
    for exe in exes:
        pe = Pe(exe)
        imp = pe.imports()
        missing = []
        for dll, funcs in sorted(imp.items()):
            if dll.startswith(("api-ms-win-", "ext-ms-win-")):
                continue
            have = None
            if a.libs:
                if pe.machine not in libs:
                    libs[pe.machine] = lib_exports(a.libs, pe.machine)
                have = libs[pe.machine].get(dll)
                if have is None:
                    fallback.add(dll)
            if have is None:
                if dll not in dlls:
                    f = sysdir / dll
                    dlls[dll] = Pe(f).exports() if f.is_file() else None
                have = dlls[dll]
            if have is None:
                missing.append(f"{dll} (the DLL)")
            else:
                missing += [f"{dll}!{f}" for f in sorted(funcs - have)]
        sets = sorted(d for d in imp if d.startswith(("api-ms-win-", "ext-ms-win-")))
        n = sum(len(f) for f in imp.values())
        print(f"{'FAIL' if missing else 'PASS'} {exe.name} ({MACHINES.get(pe.machine, hex(pe.machine))}): {n} imports from "
              f"{len(imp)} DLLs" + (f", API sets not checked: {', '.join(sets)}" if sets else ""))
        for m in missing:
            print(f"   missing {m}")
        bad += bool(missing)
    if fallback:
        print(f"note: no import library for {', '.join(sorted(fallback))}: checked against {sysdir} instead")
    print(f"winarm_imports: {len(exes) - bad} of {len(exes)} programs find every import"
          + (f" ({'import libraries ' + a.libs if a.libs else sysdir})"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

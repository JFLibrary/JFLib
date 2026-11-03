#!/usr/bin/env python3
"""lib/rt/kernel.rs against the kernel's headers: every number, open flag and struct stat field of
both tables, by the C compiler of the Android NDK (its sysroot has the kernel's UAPI headers of both
architectures).  Nothing here runs: a table entry that differs from the header stops the compile.

  python tools/kernel_table_check.py [--ndk DIR]

A wrong number compiles in Rust and calls something else, so run this when the table changes.  The
order of clone's arguments is in no header (kernel/fork.c, CONFIG_CLONE_BACKWARDS; clone(2) says ARM64
has it); plattest --test kernel checks it against the running kernel.
Exit code 0 = the table agrees with the headers.
"""
import argparse, os, re, subprocess, sys, tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TABLE = ROOT / "lib" / "rt" / "kernel.rs"
ARCHES = {"x86_64": "x86_64-linux-android", "aarch64": "aarch64-linux-android"}
SIZES = {"u32": 4, "i32": 4, "u64": 8, "i64": 8}


def find_ndk(given):
    """the newest NDK under the usual places"""
    if given:
        return Path(given)
    for base in (os.environ.get("ANDROID_NDK_HOME"), os.environ.get("ANDROID_NDK_ROOT")):
        if base and Path(base).is_dir():
            return Path(base)
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT") or Path(os.environ.get("LOCALAPPDATA", "")) / "Android" / "Sdk"
    versions = sorted((Path(sdk) / "ndk").glob("*")) if (Path(sdk) / "ndk").is_dir() else []
    return versions[-1] if versions else None


def tables():
    """{arch: table text}"""
    parts = re.split(r'#\[cfg\(target_arch = "(\w+)"\)\]\s*mod arch', TABLE.read_text(encoding="utf-8"))[1:]
    return dict(zip(parts[::2], parts[1::2]))


def checks(text):
    """the C assertions of one table -> (lines, number of entries)"""
    lines, n = [], 0
    for name, value in re.findall(r"pub const (SYS_\w+|O_\w+): usize = ([^;]+);", text):
        header = "__NR_" + name[4:].lower() if name.startswith("SYS_") else name
        value = re.sub(r"(?<=[0-9a-f])_(?=[0-9a-f])", "", value)     # Rust's digit groups, not O_DIRECTORY
        lines.append(f"_Static_assert({header} == ({value}), \"{name}\");")
        n += 1
    off = 0
    body = text[text.index("pub struct Stat {"):]
    for field, ty in re.findall(r"^\s*(?:pub )?(\w+): (\[\w+; \d+\]|\w+),$", body[:body.index("\n    }")], flags=re.M):
        m = re.fullmatch(r"\[(\w+); (\d+)\]", ty)
        size = SIZES[m.group(1)] * int(m.group(2)) if m else SIZES[ty]
        align = SIZES[m.group(1)] if m else size
        off = (off + align - 1) // align * align
        if not field.startswith("__"):      # padding: the header's names differ, the total size below covers it
            lines.append(f"_Static_assert(offsetof(struct stat, {field}) == {off}, \"Stat.{field} offset\");")
            lines.append(f"_Static_assert(sizeof(((struct stat *)0)->{field}) == {size}, \"Stat.{field} size\");")
            n += 1
        off += size
    lines.append(f"_Static_assert(sizeof(struct stat) == {off}, \"Stat size\");")
    # what the table does not list because both architectures agree, and the code relies on it
    lines.append('_Static_assert(sizeof(struct statfs) == 120, "struct statfs");')
    for field, at in (("f_bsize", 8), ("f_blocks", 16), ("f_bavail", 32), ("f_frsize", 72)):
        lines.append(f'_Static_assert(offsetof(struct statfs, {field}) == {at}, "statfs.{field}");')
    return lines, n


def main():
    ap = argparse.ArgumentParser(description="lib/rt/kernel.rs against the kernel's headers")
    ap.add_argument("--ndk", help="the NDK (default: the newest under the Android SDK)")
    a = ap.parse_args()
    ndk = find_ndk(a.ndk)
    name = "clang.exe" if os.name == "nt" else "clang"
    clang = next(iter(ndk.glob(f"toolchains/llvm/prebuilt/*/bin/{name}")), None) if ndk else None
    if not clang:
        print("no Android NDK found (--ndk DIR): nothing checked")
        return 1
    bad = False
    with tempfile.TemporaryDirectory() as tmp:
        for arch, text in tables().items():
            lines, n = checks(text)
            src = Path(tmp) / f"{arch}.c"
            src.write_text("#include <stddef.h>\n#include <linux/types.h>\n#include <asm/unistd.h>\n#include <asm/stat.h>\n#include <asm/statfs.h>\n"
                           "#include <asm/fcntl.h>\n" + "\n".join(lines) + "\n", encoding="utf-8")
            r = subprocess.run([str(clang), f"--target={ARCHES[arch]}35", "-fsyntax-only", "-ferror-limit=0", str(src)],
                               capture_output=True, text=True, encoding="utf-8", errors="replace")
            if r.returncode:
                bad = True
                print(f"FAIL {arch}:")
                for ln in r.stderr.splitlines():
                    if "error:" in ln:
                        print("   " + ln.split("error:", 1)[1].strip())
            else:
                print(f"PASS {arch}: {n} entries agree with {ndk.name}'s kernel headers")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

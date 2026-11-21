"""suite_fetch.py -- external test suites (Wycheproof, x509-limbo) into build/vectors, pinned.

    from suite_fetch import fetch
    folder = fetch("C2SP/wycheproof", COMMIT, ["testvectors_v1/x25519_test.json", ...])

Each file comes once from raw.githubusercontent.com at the pinned commit and stays in
build/vectors/<repo name>-<commit[:12]>/ (build/ is ignored by git: the suites are not part of
the repository).  The files are data: the check tools only read them as JSON."""
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def folder(repo, commit):
    return ROOT / "build" / "vectors" / f"{repo.split('/')[-1]}-{commit[:12]}"


def fetch(repo, commit, paths, quiet=False):
    """the local folder holding `paths` of `repo` at `commit` (missing ones are downloaded)"""
    dest = folder(repo, commit)
    for p in paths:
        out = dest / p
        if out.exists():
            continue
        out.parent.mkdir(parents=True, exist_ok=True)
        url = f"https://raw.githubusercontent.com/{repo}/{commit}/{p}"
        if not quiet:
            print(f"suite_fetch: {url}", file=sys.stderr)
        with urllib.request.urlopen(url, timeout=120) as r:
            data = r.read()
        tmp = out.with_suffix(out.suffix + ".part")
        tmp.write_bytes(data)
        tmp.replace(out)
    return dest

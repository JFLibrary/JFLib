#!/usr/bin/env python3
"""publish.py -- the crates to crates.io, one by one, waiting out its rate limit (docs/CRATES.md, CR7)

    python tools/publish.py                  the plan: every published crate in the order of its
                                             dependencies and what crates.io has of it; uploads nothing
    python tools/publish.py --publish        uploads what crates.io lacks, in that order
    python tools/publish.py --publish --no-verify
                                             ... without building each package first (the workspace's
                                             `cargo publish --workspace --dry-run` did that)

crates.io takes 5 new crates at once, then one every 10 minutes; new versions of crates it has 30,
then one a minute (its rate_limiter.rs). `cargo publish --workspace` stops at the first refusal, and
started again it refuses to begin while one of its crates exists at that version already. This
script runs `cargo publish -p NAME` for each crate that crates.io lacks at the workspace's version,
in the order of their dependencies, and waits until crates.io's index shows it before the next one.
On "429 Too Many Requests" it waits until the time crates.io names and tries the same crate again;
from then on it paces itself (one new crate per 10 minutes) so that cargo does not build a package
for a refusal. Stopped (Ctrl+C, a failure), it goes on where it stopped when started again.

Before the first upload:
  - `cargo login` with a token of crates.io: cargo uses it, this script never sees it;
  - the commit to publish checked out, without changes (cargo refuses a package with uncommitted
    files), its release cut (`python build_rs.py release X.Y.Z`: CHANGELOG.md has [X.Y.Z]);
  - the facade's copies of docs/GUIDE.md and docs/API.md current, every crate's README.md telling
    what is new in X.Y.Z and its CHANGELOG.md cut (the script checks both).
For the first release every name must be free: while `jflib` is not on crates.io, a name that is
taken stops the script before anything is uploaded. After the last crate: `git tag vX.Y.Z`.
Everything it prints also goes to build/publish.log.
"""
import argparse
import datetime
import email.utils
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOG = ROOT / "build" / "publish.log"
INDEX = "https://index.crates.io/"
AGENT = "jflib-publish (https://github.com/JFLibrary/JFLib)"
FACADE = "jflib"
# crates.io's limits (src/rate_limiter.rs): seconds between two uploads once the burst is used up
PACE = {"new": 10 * 60 + 20, "update": 60 + 5}


class PublishError(Exception):
    pass


def say(text):
    line = f"{datetime.datetime.now():%H:%M:%S} {text}"
    print(line, flush=True)
    LOG.parent.mkdir(parents=True, exist_ok=True)
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(line + "\n")


# ---------------------------------------------------------------- the workspace
def workspace_packages():
    """the packages that go to crates.io -> {name: {"version", "deps"}} (deps: the workspace's
    published packages it needs, of every kind but dev; Cargo drops dev-dependencies without a version)"""
    r = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked", "--offline"],
                       cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
    if r.returncode:
        raise PublishError("cargo metadata failed:\n" + r.stderr.strip())
    meta = json.loads(r.stdout)
    published = {p["name"] for p in meta["packages"] if p.get("publish") is None}
    return {p["name"]: {"version": p["version"],
                        "deps": sorted({d["name"] for d in p["dependencies"] if d["name"] in published and d["kind"] != "dev"})}
            for p in meta["packages"] if p["name"] in published}


def order(packages):
    """the names in an order where each comes after the packages it needs (by name where free)"""
    done, out, left = set(), [], dict(packages)
    while left:
        ready = sorted(n for n, p in left.items() if set(p["deps"]) <= done)
        if not ready:
            raise PublishError(f"the dependencies form a cycle: {', '.join(sorted(left))}")
        for n in ready:
            out.append(n)
            done.add(n)
            del left[n]
    return out


# ---------------------------------------------------------------- crates.io
def index_path(name):
    """the file of a crate in the sparse index (doc.rust-lang.org/cargo/reference/registry-index.html)"""
    n = name.lower()
    if len(n) <= 2:
        return f"{len(n)}/{n}"
    if len(n) == 3:
        return f"3/{n[0]}/{n}"
    return f"{n[:2]}/{n[2:4]}/{n}"


def index_versions(name):
    """the versions crates.io has of a crate (yanked ones too) -> a set, or None when it has no such crate"""
    req = urllib.request.Request(INDEX + index_path(name), headers={"User-Agent": AGENT, "Cache-Control": "no-cache"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            text = r.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        if e.code in (404, 410, 451):
            return None
        raise PublishError(f"index.crates.io: {name}: HTTP {e.code}")
    return {json.loads(line)["vers"] for line in text.splitlines() if line.strip()}


def retry_after(text):
    """crates.io's "Please try again after Mon, 09 Nov 2026 12:34:56 GMT" -> an aware datetime, or None"""
    m = re.search(r"try again after ([A-Z][a-z]{2}, \d{1,2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2} GMT)", text)
    if not m:
        return None
    try:
        return email.utils.parsedate_to_datetime(m.group(1))
    except (TypeError, ValueError):
        return None


def plan(packages, names, versions_of=index_versions):
    """-> [(name, version, state)]: state "there" (crates.io has that version), "new" (no such crate),
    "update" (the crate without that version)"""
    out = []
    for n in names:
        have = versions_of(n)
        v = packages[n]["version"]
        out.append((n, v, "new" if have is None else "there" if v in have else "update"))
    return out


# ---------------------------------------------------------------- before the upload
def preflight(version):
    """what must hold before anything is uploaded -> problems (empty: go)"""
    bad = []
    sys.path.insert(0, str(ROOT))
    from tools import crates_accept
    bad += crates_accept.template_problems()        # the facade's template/ and docs/ (build_rs.py check)
    log = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    m = re.search(rf"^## \[{re.escape(version)}\] - (\d{{4}}-\d{{2}}-\d{{2}})", log, re.M)
    if not m:
        bad.append(f"CHANGELOG.md has no [{version}]: python build_rs.py release {version}")
    elif m.group(1) != datetime.date.today().isoformat():
        say(f"note: CHANGELOG.md dates [{version}] {m.group(1)}, today is {datetime.date.today()}: fix the date if it matters")
    from tools import changelog                     # every crate's README.md tells what is new in it, its CHANGELOG.md has it
    bad += changelog.crate_doc_problems(release=version, cut=True)
    return bad


def wait_until(when, why):
    """sleep until an aware datetime, saying so once"""
    now = datetime.datetime.now(datetime.timezone.utc)
    if when > now:
        say(f"waiting until {when.astimezone():%H:%M:%S} ({why})")
        time.sleep((when - now).total_seconds())


def wait_in_index(name, version, limit=15 * 60):
    """until crates.io's index shows the version (the next crate's check reads the index)"""
    end = time.monotonic() + limit
    while time.monotonic() < end:
        have = index_versions(name)
        if have and version in have:
            return
        time.sleep(10)
    raise PublishError(f"{name} {version} uploaded, but the index does not show it after {limit // 60} minutes: start again later")


def cargo_publish(name, no_verify):
    """`cargo publish -p NAME`, its output shown and kept, in the log too -> (exit code, output)"""
    args = ["cargo", "publish", "-p", name, "--locked"] + (["--no-verify"] if no_verify else [])
    say("$ " + " ".join(args))
    p = subprocess.Popen(args, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    lines = []
    with open(LOG, "a", encoding="utf-8") as log:
        for line in p.stdout:
            print("    " + line.rstrip(), flush=True)
            log.write("    " + line.rstrip() + "\n")
            lines.append(line)
    return p.wait(), "".join(lines)


def publish(packages, rows, no_verify):
    """upload every crate crates.io lacks, in order; waits out the rate limit"""
    last = {}                     # kind -> monotonic time of the last upload of that kind
    paced = set()                 # the kinds crates.io has refused once: from then on one per PACE
    todo = [(n, v, s) for n, v, s in rows if s != "there"]
    for i, (name, version, kind) in enumerate(todo, 1):
        while True:
            if kind in paced and kind in last:
                gap = PACE[kind] - (time.monotonic() - last[kind])
                if gap > 0:
                    wait_until(datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=gap),
                               f"crates.io's limit, {'a new crate' if kind == 'new' else 'a new version'} every {PACE[kind] // 60} min")
            say(f"[{i}/{len(todo)}] {name} {version} ({kind})")
            code, out = cargo_publish(name, no_verify)
            if code == 0:
                last[kind] = time.monotonic()
                wait_in_index(name, version)
                say(f"[{i}/{len(todo)}] {name} {version} is on crates.io")
                break
            if "already exists" in out or "already uploaded" in out:
                say(f"[{i}/{len(todo)}] {name} {version} was there already")
                break
            if "429" in out or "Too Many Requests" in out:
                paced.add(kind)
                when = retry_after(out) or datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=PACE[kind])
                wait_until(when + datetime.timedelta(seconds=15), "crates.io refused: too many uploads (429)")
                continue
            raise PublishError(f"cargo publish -p {name} failed (exit {code}); start again to go on from here")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--publish", action="store_true", help="upload; without it only the plan")
    ap.add_argument("--no-verify", action="store_true", help="cargo publish --no-verify")
    a = ap.parse_args(argv)
    try:
        packages = workspace_packages()
        names = order(packages)
        versions = {packages[n]["version"] for n in names}
        if len(versions) != 1:
            raise PublishError(f"the crates have different versions: {sorted(versions)}")
        version = versions.pop()
        rows = plan(packages, names)
        for i, (n, v, s) in enumerate(rows, 1):
            print(f"{i:3}  {n:28} {v:10} {'on crates.io' if s == 'there' else 'to upload (' + s + ')'}")
        new = sum(s == "new" for _, _, s in rows)
        update = sum(s == "update" for _, _, s in rows)
        first = next(s for n, _, s in rows if n == FACADE) == "new"
        taken = [n for n, _, s in rows if s != "new"] if first else []
        if taken:
            raise PublishError(f"{FACADE} is not on crates.io yet, but these names are taken: {', '.join(taken)}")
        hours = max(0, new - 5) * PACE["new"] / 3600 + max(0, update - 30) * PACE["update"] / 3600
        print(f"{new} new crates, {update} new versions to upload, {len(rows) - new - update} there; "
              f"crates.io's limit makes that about {hours:.1f} h" if new + update else "crates.io has every crate at " + version)
        if not a.publish:
            if new + update:
                print("nothing uploaded (python tools/publish.py --publish uploads)")
            return 0
        if not new + update:
            return 0
        bad = preflight(version)
        if bad:
            raise PublishError("before the upload:\n  " + "\n  ".join(bad))
        head = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        say(f"publishing {version} from {head}: {new} new crates, {update} new versions")
        publish(packages, rows, a.no_verify)
        say(f"every crate is on crates.io at {version}; next: git tag v{version} {head} and push the tag")
        return 0
    except PublishError as e:
        say(f"error: {e}")
        return 1
    except KeyboardInterrupt:
        say("stopped; python tools/publish.py --publish goes on from here")
        return 130


if __name__ == "__main__":
    sys.exit(main())

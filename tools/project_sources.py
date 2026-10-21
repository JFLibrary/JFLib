"""One source definition for direct-rustc projects, tooling and API documentation.

Existing flat projects and new src/ projects coexist. Reference material is never
treated as source; explicitly referenced shared modules and embedded assets are
included in fingerprints, including references outside the project directory.
"""
import os
import re
from pathlib import Path


class SourceError(ValueError):
    """An ambiguous or missing project entry point."""


EXCLUDED = frozenset({"context", "books", "build", "target", "tests", "fixtures",
                      ".git", "node_modules", "__pycache__"})
REFERENCES = re.compile(r'(?:#\[path\s*=\s*|include_(?:bytes|str)!\(\s*|include!\(\s*)"([^"]+)"')


def entrypoint(project, kind, required=True):
    """Resolve main.rs/lib.rs; never silently prefer one of two entry points."""
    project = Path(project)
    filename = "main.rs" if kind == "app" else "lib.rs"
    found = [p for p in (project / "src" / filename, project / filename) if p.is_file()]
    if len(found) > 1:
        raise SourceError(f"{project}: competing entry points: src/{filename} and {filename}")
    if not found and required:
        raise SourceError(f"{project}: missing src/{filename} or {filename}")
    return found[0] if found else None


def resolve_root(root):
    """Resolve a registered legacy or src entry using the same ambiguity check."""
    root = Path(root)
    project = root.parent.parent if root.parent.name == "src" else root.parent
    return entrypoint(project, "app" if root.name == "main.rs" else "lib")


def project_sources(root, assets=False):
    """Sorted source set (or fingerprint set with embedded resources included)."""
    root = resolve_root(root).resolve()
    files = set()
    for directory, children, names in os.walk(root.parent, followlinks=False):
        children[:] = sorted(c for c in children if c not in EXCLUDED
                             and not (Path(directory) / c).is_symlink())
        files.update((Path(directory) / n).resolve() for n in names if n.endswith(".rs"))
    pending = list(files)
    seen = set(files)
    while pending:
        source = pending.pop()
        for relative in REFERENCES.findall(source.read_text(encoding="utf-8")):
            dep = (source.parent / relative).resolve()
            if dep in seen:
                continue
            seen.add(dep)
            if not dep.is_file():
                raise SourceError(f"{source}: referenced source/asset does not exist: {relative}")
            if dep.suffix == ".rs":
                files.add(dep)
                pending.append(dep)
            elif assets:
                files.add(dep)
    return sorted(files)

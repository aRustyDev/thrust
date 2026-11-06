# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Layout tolerance: resolve hard-coded repo-relative paths across Plane tree layouts, loudly.

Why this exists. Upstream flattened `apps/web/core/**` and `apps/web/ce/**` into `apps/web/**` (seen at the oracle pin
ba85d4d3d3) and moved or removed some packages. A script that hard-codes `apps/web/core/store/...` then dies with a bare
FileNotFoundError, or - worse - returns an empty result because a helper swallowed the error.

Contract:
  * every hard-coded path goes through `Layout.resolve()` (or `.resolve_dir()`, `.optional()`);
  * `resolve()` tries the path as written, then the layouts in MOVE_RULES (both directions), returns the path that
    EXISTS, and records which variant it used (`moved: true` when it is not the one written);
  * if no variant exists it raises `LayoutError` naming the path and every variant tried: no silent fallback;
  * `optional()` is only for things that may legitimately be absent; the absence is RECORDED, never hidden;
  * `canon()` maps any layout of a file path onto one comparison form, so hand-curated tables keyed by file path
    (client_overrides.py, p0_slice.py) keep working when files move.
Nothing here reads file contents or runs a process: stat-level only.
"""

from __future__ import annotations

from pathlib import Path

# (path prefix in the layout the hand-curated tables were written in, prefix after the move). First existing candidate wins.
MOVE_RULES = [
    ("apps/web/core/", "apps/web/"),
    ("apps/web/ce/", "apps/web/"),
    ("packages/editor/src/core/", "packages/editor/src/"),
    ("packages/editor/src/ce/", "packages/editor/src/"),
]


class LayoutError(FileNotFoundError):
    pass


def canon(rel: str) -> str:
    """One comparison form for a repo-relative path across layouts (forward-maps every known move)."""
    for old, new in MOVE_RULES:
        if rel.startswith(old):
            return new + rel[len(old):]
    return rel


class Layout:
    def __init__(self, root: Path):
        self.root = root
        self.resolutions: dict[str, dict] = {}
        self.absent_optional: list[str] = []
        self.anchor_checks: list[dict] = []

    # ------------------------------------------------------------------ candidates
    @staticmethod
    def candidates(rel: str) -> list[str]:
        out = [rel]
        for old, new in MOVE_RULES:
            if rel.startswith(old):
                out.append(new + rel[len(old):])
            elif rel.startswith(new):
                out.append(old + rel[len(new):])
        seen, uniq = set(), []
        for c in out:
            if c not in seen:
                seen.add(c)
                uniq.append(c)
        return uniq

    # ------------------------------------------------------------------ resolution
    def resolve(self, rel: str, kind: str = "file") -> str:
        """Return the repo-relative path (as it exists in THIS tree) for `rel`; raise LayoutError if no variant exists."""
        tried = self.candidates(rel)
        for c in tried:
            p = self.root / c
            if (p.is_file() if kind == "file" else p.is_dir()):
                self.resolutions[rel] = {"used": c, "moved": c != rel, "kind": kind}
                return c
        raise LayoutError(f"{kind} not found in tree {self.root}: {rel} (tried: {', '.join(tried)})")

    def resolve_dir(self, rel: str) -> str:
        return self.resolve(rel, kind="dir")

    def path(self, rel: str, kind: str = "file") -> Path:
        return self.root / self.resolve(rel, kind)

    def optional(self, rel: str, kind: str = "file") -> str | None:
        """Only for paths that may legitimately be absent. The absence is recorded in layout-report.json."""
        try:
            return self.resolve(rel, kind)
        except LayoutError:
            self.absent_optional.append(rel)
            return None

    # ------------------------------------------------------------------ report
    def report(self) -> dict:
        moved = {k: v["used"] for k, v in sorted(self.resolutions.items()) if v["moved"]}
        return {
            "paths_resolved": len(self.resolutions),
            "paths_moved_vs_hand_curated_layout": moved,
            "optional_paths_absent": sorted(set(self.absent_optional)),
        }


_CACHE: dict[Path, Layout] = {}


def for_root(root: Path) -> Layout:
    root = Path(root).resolve()
    if root not in _CACHE:
        _CACHE[root] = Layout(root)
    return _CACHE[root]


def reset():
    _CACHE.clear()

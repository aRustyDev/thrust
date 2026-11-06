# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Precision / recall instruments for the client scan (step 2).

  recall    : an INDEPENDENT raw-text regex finds every quoted `/api/...` or `/auth/...` literal in the scanned sources
              (comments dropped, `${...}` -> `{}`); a literal counts as recalled if its normalised path equals a path the
              scanner extracted (client-calls.json paths + client-refs.json paths), segment-wise, where `{}` matches
              anything. Missing literals are listed with file:line.
  precision : `handcheck-sample.csv` is a seeded random sample of N extracted call sites with the source lines next to
              the extracted fields, for a human to verify. Verdicts go into handcheck-verdicts.csv (written by hand);
              this script reports precision from that file when it exists.

Usage:  uv run precision_recall.py --root <tree> --out <dir> [--sample 30 --seed 9]
"""

from __future__ import annotations

import argparse
import csv
import json
import random
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from client_calls import DEFAULT_CLIENTS, iter_sources  # noqa: E402

RAW = re.compile(r"""["'`](/(?:api|auth)/[^"'`\s]*)""")


def norm(p: str) -> str:
    p = re.sub(r"\$\{[^}]*\}", "{}", p)
    p = re.sub(r"\{[^{}]*\}", "{}", p)
    p = p.partition("?")[0]
    return p.rstrip("/")


def seg_match(a: str, b: str) -> bool:
    A, B = a.strip("/").split("/"), b.strip("/").split("/")
    return len(A) == len(B) and all(x == y or x == "{}" or y == "{}" or ("{}" in x and "{}" in y) for x, y in zip(A, B))


def recall(root: Path, out: Path) -> dict:
    calls = json.loads((out / "client-calls.json").read_text())
    refs = json.loads((out / "client-refs.json").read_text())
    got = set()
    for c in calls:
        for p in c["paths"]:
            got.add(norm(p))
    for r in refs:
        for p in r["paths"]:
            got.add(norm(p))
    seen_files = set()
    raw_hits = []
    for rel, label, _ in DEFAULT_CLIENTS:
        for f in iter_sources(root, rel):
            if f in seen_files:
                continue
            seen_files.add(f)
            for i, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
                s = line.strip()
                if s.startswith("//") or s.startswith("*") or s.startswith("/*"):
                    continue
                for m in RAW.finditer(line):
                    raw_hits.append((str(f.relative_to(root)), i, norm(m.group(1))))
    distinct = {}
    for f, i, p in raw_hits:
        distinct.setdefault(p, (f, i))
    missing = []
    for p, (f, i) in sorted(distinct.items()):
        if p in got:
            continue
        if any(seg_match(p, g) for g in got):
            continue
        missing.append({"path": p, "first_seen": f"{f}:{i}"})
    return {
        "raw_literal_occurrences": len(raw_hits),
        "raw_distinct_normalised_paths": len(distinct),
        "recalled": len(distinct) - len(missing),
        "recall": round((len(distinct) - len(missing)) / max(1, len(distinct)), 4),
        "missing": missing,
    }


def sample(root: Path, out: Path, n: int, seed: int):
    calls = json.loads((out / "client-calls.json").read_text())
    pool = sorted((c for c in calls if c["resolution"] in ("literal", "derived")), key=lambda c: (c["file"], c["line"]))
    rng = random.Random(seed)
    pick = rng.sample(pool, min(n, len(pool)))
    rows = []
    for c in sorted(pick, key=lambda c: (c["file"], c["line"])):
        lines = (root / c["file"]).read_text(encoding="utf-8").splitlines()
        start = c["line"] - 1
        # method signature + body of the call: from the nearest preceding `async`/member line to ~8 lines after the call
        lo = max(0, start - 6)
        snippet = "\n".join(f"{j + 1}: {lines[j]}" for j in range(lo, min(len(lines), start + 9)))
        rows.append({
            "n": len(rows) + 1, "client": c["client"], "site": f"{c['file']}:{c['line']}", "member": f"{c['cls']}.{c['member']}",
            "verb": c["verb"], "paths": ";".join(c["paths"]), "query_keys": c["params_keys"], "params_type": c["params_type"],
            "body_type": c["body_type"], "response_type": c["response_type"], "then_cast": c["then_cast"], "catch_throws": c["catch_throws"],
            "options": c["options"], "snippet": snippet,
        })
    with open(out / "handcheck-sample.csv", "w", newline="") as fh:
        wr = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        wr.writeheader()
        wr.writerows(rows)
    return rows


def precision(out: Path, verdicts: Path | None = None) -> dict | None:
    """Verdicts live NEXT TO THE SCRIPTS (not in out/): they are human work, valid only for the sample they were made on.
    One verdicts file per tree (the pre-pin one is handcheck-verdicts.csv; pass `verdicts` for another tree)."""
    p = verdicts or (Path(__file__).resolve().parent / "handcheck-verdicts.csv")
    if not p.exists():
        return None
    rows = list(csv.DictReader(open(p)))
    cur = {(r["n"], r["site"], r["member"]) for r in csv.DictReader(open(out / "handcheck-sample.csv"))}
    stale = [r["n"] for r in rows if (r["n"], r["site"], r["member"]) not in cur]
    if stale:
        return {"stale": True, "note": f"{len(stale)} verdict rows do not match the current sample (tree changed): re-do the hand check", "stale_rows": stale}
    fields = ("verb", "path", "query", "body", "response", "catch")
    tot = {f: [0, 0] for f in fields}
    for r in rows:
        for f in fields:
            v = (r.get(f) or "").strip().lower()
            if v in ("ok", "wrong"):
                tot[f][1] += 1
                tot[f][0] += v == "ok"
    return {"n_sites": len(rows), "per_field_ok_over_checked": {f: f"{a}/{b}" for f, (a, b) in tot.items()},
            "sites_fully_correct": sum(1 for r in rows if all((r.get(f) or "").strip().lower() in ("ok", "n/a") for f in fields))}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--sample", type=int, default=30)
    ap.add_argument("--seed", type=int, default=9)
    a = ap.parse_args()
    root, out = a.root.resolve(), a.out.resolve()
    res = {"recall": recall(root, out)}
    sample(root, out, a.sample, a.seed)
    pr = precision(out)
    if pr:
        res["precision_handcheck"] = pr
    (out / "precision-recall.json").write_text(json.dumps(res, indent=1, sort_keys=True) + "\n")
    print(json.dumps({"recall": {k: v for k, v in res["recall"].items() if k != "missing"}, "missing": res["recall"]["missing"][:20],
                      "precision_handcheck": pr}, indent=1))


if __name__ == "__main__":
    main()

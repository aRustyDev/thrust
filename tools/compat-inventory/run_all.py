# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Spike 0009 (desk half) - run the whole static inventory against a Plane tree.

    uv run run_all.py --root <plane tree root> --out <output dir> [--vcs files|git] [--label TEXT]

Steps (each is also runnable alone, see its docstring):
  1 server_routes.py   Django URL confs via ast            -> server-*.json|csv
  2 client_calls.py    React/TS call sites                 -> client-*.json|csv
  3 join_tag.py        join + FEATURES tags + effort       -> endpoint-inventory.*, join-*.json|csv
  4 shapes_probe.py    anchors for the shape catalogue     -> shapes-anchors.json
  5 effort_report.py   class totals, P0 slice, foundations -> effort-summary.json
Also writes RUN.json (tree commit + counts) and layout-report.json (which hard-coded paths had to be resolved to a different
location in this tree, optional paths that are absent, unscanned dirs that contain API literals, anchor drift, slice-trigger
line drift). Everything else in <out> is deterministic for a given tree, so after a re-run on a new commit:
    diff -ru <old out> <new out> -x RUN.json -x layout-report.json      or      compare_runs.py --old <old out> --new <out>

Layout tolerance (layout.py): hard-coded paths are resolved through a helper that tries the pre-pin and the flattened layouts
and FAILS LOUDLY, naming the missing path, if neither exists. Exit status 2 = unmapped routes or stale slice evidence;
a LayoutError aborts with the path.

--vcs files (default) reads the commit from the worktree's .git pointer with plain file reads and never executes git, which is
what you want for a tree you must not touch; --vcs git additionally records `git describe` and dirty-file counts (use it on a
tree you own: the pre-pin run used it). Stdlib only; run through `uv run`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))


def git(root: Path, *args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=True).stdout.strip()
    except Exception as e:  # noqa: BLE001
        return f"unknown ({type(e).__name__})"


def read_head(root: Path) -> str:
    """HEAD commit of a worktree/checkout using plain file reads only (no git process)."""
    g = root / ".git"
    try:
        if g.is_file():
            gitdir = Path(g.read_text().split("gitdir:", 1)[1].strip())
        elif g.is_dir():
            gitdir = g
        else:
            return "unknown (no .git)"
        head = (gitdir / "HEAD").read_text().strip()
        if not head.startswith("ref:"):
            return head
        ref = head.split(None, 1)[1]
        common = gitdir
        if (gitdir / "commondir").is_file():
            common = (gitdir / (gitdir / "commondir").read_text().strip()).resolve()
        for base in (gitdir, common):
            if (base / ref).is_file():
                return (base / ref).read_text().strip()
        packed = common / "packed-refs"
        if packed.is_file():
            for line in packed.read_text().splitlines():
                if line.endswith(" " + ref):
                    return line.split()[0]
        return f"unknown (ref {ref} not found)"
    except Exception as e:  # noqa: BLE001
        return f"unknown ({type(e).__name__})"


def read_pin(root: Path) -> dict | None:
    """oracle/PIN (KEY=VALUE) if the tree carries one (spike 0008)."""
    f = root / "oracle" / "PIN"
    if not f.is_file():
        return None
    out = {}
    for line in f.read_text().splitlines():
        if line.strip() and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            out[k.strip()] = v.strip()
    keep = ("pin_tag", "pinned_commit", "upstream_sha", "upstream_describe", "upstream_version", "fork_base_commit", "date")
    return {k: out[k] for k in keep if k in out}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", required=True, type=Path, help="Plane tree root (contains apps/ and packages/)")
    ap.add_argument("--out", type=Path, default=HERE / "out")
    ap.add_argument("--features", type=Path, default=HERE.parents[1] / "FEATURES.md")
    ap.add_argument("--vcs", choices=("files", "git"), default="files", help="how to stamp the tree commit (default: plain file reads, no git)")
    ap.add_argument("--verdicts", type=Path, default=None, help="hand-check verdicts CSV for this tree (default: handcheck-verdicts.csv next to the scripts)")
    ap.add_argument("--label", default=None, help="free-text label recorded as tree_label (e.g. the pin tag)")
    a = ap.parse_args()
    root, out = a.root.resolve(), a.out.resolve()
    out.mkdir(parents=True, exist_ok=True)

    import client_calls
    import effort_report
    import join_tag
    import layout
    import server_routes
    import shapes_probe

    L = layout.for_root(root)

    res = server_routes.run(root)
    server_routes.write_outputs(res, out)
    ssum = server_routes.summarise(res)

    cres = client_calls.scan(root)
    cstats = client_calls.write_outputs(cres, out)

    jres = join_tag.run(out, a.features)
    join_tag.write(out, jres)
    jrep = join_tag.report(jres)
    (out / "join-report.json").write_text(json.dumps(jrep, indent=1, sort_keys=True) + "\n")

    shapes = shapes_probe.run(root)
    (out / "shapes-anchors.json").write_text(json.dumps(shapes, indent=1, sort_keys=True) + "\n")

    import precision_recall

    pr = {"recall": precision_recall.recall(root, out)}
    precision_recall.sample(root, out, 30, 9)
    prec = precision_recall.precision(out, a.verdicts)
    if prec:
        pr["precision_handcheck"] = prec
    (out / "precision-recall.json").write_text(json.dumps(pr, indent=1, sort_keys=True) + "\n")

    eff = effort_report.run(out, root)
    trig = eff.pop("_layout_triggers", None)
    (out / "effort-summary.json").write_text(json.dumps(eff, indent=1, sort_keys=True) + "\n")

    # ---- layout report (new in the post-pin tooling; deliberately a separate file so older outputs stay byte-comparable)
    anchor_bad = [c for c in L.anchor_checks if not c["ok"]]
    unlisted = client_calls.unlisted_dirs(root)
    lrep = {
        "tree_root_name": root.name,
        **L.report(),
        "anchor_checks_total": len(L.anchor_checks),
        "anchor_checks_violated": anchor_bad,
        "unlisted_dirs_not_scanned_as_clients": unlisted,
        "slice_triggers": trig,
    }
    (out / "layout-report.json").write_text(json.dumps(lrep, indent=1, sort_keys=True) + "\n")

    run = {
        "tree_root": str(root),
    }
    if a.vcs == "git":
        dirty = git(root, "status", "--porcelain", "--", "apps", "packages")
        run.update({
            "tree_commit": git(root, "rev-parse", "HEAD"),
            "tree_describe": git(root, "describe", "--tags", "--always"),
            "tree_dirty_files_under_apps_packages": 0 if (not dirty or dirty.startswith("unknown")) else len(dirty.splitlines()),
        })
    else:
        run.update({
            "tree_commit": read_head(root),
            "tree_describe": "n/a (--vcs files: git is never executed against this tree)",
            "tree_dirty_files_under_apps_packages": None,
        })
    pin = read_pin(root)
    if pin:
        run["tree_pin_file"] = pin
    if a.label:
        run["tree_label"] = a.label
    run.update({
        "server_routes": {s: v["route_entries"] for s, v in ssum.items() if not s.startswith("_")},
        "server_rows": {s: v["endpoint_rows_method_x_path"] for s, v in ssum.items() if not s.startswith("_")},
        "client_calls": cstats["calls_extracted"],
        "client_recall_check": cstats["recall_check_extracted_plus_nonservice_over_textual"],
        "client_literal_recall": pr["recall"]["recall"],
        "client_handcheck": pr.get("precision_handcheck", "no verdicts file"),
        "join_unmatched_by_category": jrep["client_unmatched_by_category"],
        "unmapped_server_routes": jrep["unmapped_server_routes"],
        "slice_validation_errors": eff["p0_slice"]["validation_errors"],
    })
    (out / "RUN.json").write_text(json.dumps(run, indent=1, sort_keys=True) + "\n")

    import contextlib
    import io

    import render_tables

    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        render_tables.render(out)  # reads RUN.json for the commit stamp
    (out / "tables.md").write_text(buf.getvalue())
    print(json.dumps(run, indent=1, sort_keys=True))

    # ---- loud notes (stderr): nothing about layout drift is silent
    moved = lrep["paths_moved_vs_hand_curated_layout"]
    if moved:
        print(f"NOTE layout: {len(moved)} hand-curated path(s) resolved to a moved location (see layout-report.json)", file=sys.stderr)
    if lrep["optional_paths_absent"]:
        print(f"NOTE layout: optional path(s) absent in this tree: {lrep['optional_paths_absent']}", file=sys.stderr)
    for u in unlisted:
        if u["api_literal_hits"]:
            print(f"WARNING layout: unscanned dir {u['dir']} contains {u['api_literal_hits']} /api|/auth literal(s): {u['first_hits']}", file=sys.stderr)
    if anchor_bad:
        print(f"WARNING anchors: {len(anchor_bad)} shape anchor(s) violated their expectation (drift): see layout-report.json", file=sys.stderr)
    if trig and trig["drift"]:
        print(f"NOTE slice: {len(trig['drift'])} trigger line(s) drifted (see layout-report.json slice_triggers.drift)", file=sys.stderr)
    bad = run["unmapped_server_routes"] or run["slice_validation_errors"]
    if bad:
        print("ERROR: unmapped routes or slice validation errors - see RUN.json", file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()

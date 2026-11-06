# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Step 6 - effort classes, person-day totals, and validation of the hand-traced P0 slice.

Reads out/endpoint-inventory.json (join_tag.py) and p0_slice.py; applies effort_model.py.
Writes out/effort-summary.json. Exit status 2 from run_all.py if any slice entry fails validation.

Usage:  uv run effort_report.py --out <dir>
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import effort_model as EM  # noqa: E402
import p0_slice  # noqa: E402

COMPAT_SURFACES = ("internal", "space", "instance", "auth")


def pd_range(cls: str, likely: float):
    lo, hi = EM.RANGE[cls]
    return round(likely * lo, 2), round(likely, 2), round(likely * hi, 2)


def scope_summary(rows: list[dict], foundations: dict[str, float]) -> dict:
    by_cls: dict[str, dict] = {}
    for c in EM.CLASSES:
        rs = [r for r in rows if r["effort_class"] == c]
        likely = sum(r["days_marginal"] for r in rs)
        lo, mid, hi = pd_range(c, likely)
        by_cls[c] = {
            "rows": len(rs),
            "paths": len({r["norm_path"] for r in rs}),
            "complexity": dict(Counter(r["complexity"] for r in rs)),
            "marginal_days_low": lo, "marginal_days_likely": mid, "marginal_days_high": hi,
        }
    marg_lo = sum(v["marginal_days_low"] for v in by_cls.values())
    marg_mid = sum(v["marginal_days_likely"] for v in by_cls.values())
    marg_hi = sum(v["marginal_days_high"] for v in by_cls.values())
    f = sum(foundations.values())
    return {
        "rows": len(rows), "paths": len({r["norm_path"] for r in rows}), "by_class": by_cls,
        "foundations_days": round(f, 2), "foundations_detail": {k: round(v, 2) for k, v in foundations.items()},
        "total_days_low": round(marg_lo + 0.7 * f, 1), "total_days_likely": round(marg_mid + f, 1), "total_days_high": round(marg_hi + 1.6 * f, 1),
    }


def check_triggers(root: Path) -> dict:
    """Verify each slice entry's trigger `file:line` against the tree being scanned (layout-tolerant, see layout.py).

    -> {"errors": [...], "drift": [...], "resolved": {id: {"file", "line"}}}
    errors: the trigger file is missing in every layout, or its token is nowhere in the file (evidence is stale: the run must fail);
    drift:  the token is in the file but not within +-5 lines of the cited line (line numbers moved; new lines are suggested)."""
    import layout

    L = layout.for_root(root)
    errors, drift, resolved = [], [], {}
    for e in p0_slice.SLICE:
        file_part, _, line_part = e["trigger"].partition(":")
        cited = int(line_part) if line_part.isdigit() else None
        try:
            used = L.resolve(file_part)
        except layout.LayoutError as ex:
            errors.append(f"{e['id']}: trigger file missing: {ex}")
            continue
        token = p0_slice.TRIGGER_TOKENS.get(e["id"])
        if not token:
            errors.append(f"{e['id']}: no TRIGGER_TOKENS entry (add one in p0_slice.py)")
            continue
        lines = (root / used).read_text(encoding="utf-8").splitlines()
        hits = [i for i, ln in enumerate(lines, 1) if token in ln]
        if not hits:
            errors.append(f"{e['id']}: token {token!r} not found in {used}: the trigger evidence is stale")
            continue
        near = [h for h in hits if cited is not None and abs(h - cited) <= 5]
        if near:
            resolved[e["id"]] = {"file": used, "line": cited, "moved_file": used != file_part}
        else:
            best = min(hits, key=lambda h: abs(h - cited) if cited else h)
            resolved[e["id"]] = {"file": used, "line": best, "moved_file": used != file_part, "drift_from": cited}
            drift.append(f"{e['id']}: cited {file_part}:{cited}, token {token!r} now at {used}:{hits[:6]}")
    return {"errors": errors, "drift": drift, "resolved": resolved}


def run(out: Path, root: Path | None = None) -> dict:
    rows = json.loads((out / "endpoint-inventory.json").read_text())
    by_key = {(r["surface"], r["method"], r["norm_path"]): r for r in rows}
    by_mp = defaultdict(list)
    for r in rows:
        by_mp[(r["method"], r["norm_path"])].append(r)
    calls = json.loads((out / "client-calls.json").read_text())
    call_index = {(c["client"], f"{c['cls']}.{c['member']}") for c in calls}
    external = {(c["client"], f"{c['cls']}.{c['member']}") for c in calls if str(c.get("note", "")).startswith("EXTERNAL")}

    full_found = {k: v[0] for k, v in EM.FOUNDATIONS.items()}
    compat = [r for r in rows if r["surface"] in COMPAT_SURFACES]

    # ---- slice validation
    errors, slice_rows, ids_seen = [], [], set()
    for e in p0_slice.SLICE:
        if e["id"] in ids_seen:
            errors.append(f"{e['id']}: duplicate id")
        ids_seen.add(e["id"])
        cands = by_mp.get((e["method"], e["path"]), [])
        cands = [r for r in cands if r["surface"] in COMPAT_SURFACES]
        if not cands:
            errors.append(f"{e['id']}: no server route {e['method']} {e['path']}")
            continue
        r = cands[0]
        kind, _, ref = e["call"].partition(":")
        if kind == "ref":
            ok = any(k.startswith("ref:" + ref) for k in r["ref_kinds"])
            if not ok:
                errors.append(f"{e['id']}: no `{e['call']}` reference reaches {e['method']} {e['path']} (ref kinds: {r['ref_kinds']})")
        else:
            label = ref
            if not any(label == s for s in r["services"]):
                errors.append(f"{e['id']}: call {e['call']} does not reach {e['method']} {e['path']} (services: {r['services']})")
        slice_rows.append({**e, "row": r})
    minimal = [x for x in slice_rows if x["kind"] == "min"]
    ext = [x for x in slice_rows if x["kind"] == "ext"]
    # a slice entry may use only part of its handler (e.g. S47: the flat list branch, not the group_by branches)
    min_rows = [dict(x["row"], days_marginal=round(x["row"]["days_marginal"] * x.get("scale", 1.0), 3)) for x in minimal]
    sf = {k: EM.FOUNDATIONS[k][0] * frac for k, (frac, _) in p0_slice.SLICE_FOUNDATIONS.items()}
    slice_sum = scope_summary(min_rows, sf)

    # ---- scopes
    used = [r for r in compat if r["disposition"] in ("P0", "P1", "P2")]
    p0 = [r for r in compat if r["disposition"] == "P0"]
    p0_no_slice = [r for r in p0 if (r["surface"], r["method"], r["norm_path"]) not in {(x["surface"], x["method"], x["norm_path"]) for x in min_rows}]
    scopes = {
        "all_compat_rows(internal+space+instance+auth)": scope_summary(compat, full_found),
        "used_by_react_not_dropped(P0+P1+P2)": scope_summary(used, full_found),
        "P0_used": scope_summary(p0, full_found),
        "P0_used_excl_minimum_slice": scope_summary(p0_no_slice, {k: v - sf.get(k, 0) for k, v in full_found.items()}),
        "minimum_slice": slice_sum,
    }
    # ---- by-class listing for P0 (what the findings table shows)
    p0_by_class_feature = defaultdict(lambda: Counter())
    for r in p0:
        p0_by_class_feature[r["effort_class"]][",".join(r["features"])] += 1
    # ---- permission decorator combos (input for the permission seam)
    srv = json.loads((out / "server-routes.json").read_text())
    perm = Counter()
    import ast

    for r in srv:
        if r["surface"] in COMPAT_SURFACES:
            key = None
            for deco in (r["decorators"] or "").split(" | "):
                if not deco.startswith("allow_permission("):
                    continue
                call = ast.parse(deco, mode="eval").body
                roles = []
                level = "PROJECT"  # decorator default (app/permissions/base.py)
                extra = ""
                if call.args and isinstance(call.args[0], ast.List):
                    roles = [ast.unparse(e).replace("ROLE.", "") for e in call.args[0].elts]
                for kw in call.keywords:
                    if kw.arg == "allowed_roles" and isinstance(kw.value, ast.List):
                        roles = [ast.unparse(e).replace("ROLE.", "") for e in kw.value.elts]
                    elif kw.arg == "level" and isinstance(kw.value, ast.Constant):
                        level = kw.value.value
                    elif kw.arg == "creator":
                        extra = " +creator-of:" + next((ast.unparse(k.value) for k in call.keywords if k.arg == "model"), "?")
                key = (",".join(roles) + extra, level)
            if key is None:
                key = ("(class permission_classes: " + (r["permission_classes"] or "default") + ")", "-")
            perm[key] += 1
    trig = check_triggers(root) if root is not None else None
    if trig:
        errors = errors + trig["errors"]  # stale trigger evidence fails the run exactly like a stale route
    return {
        "_layout_triggers": trig,  # popped by the caller: kept out of effort-summary.json so older runs stay byte-comparable
        "model": {"per_row_days": EM.PER_ROW, "range_multipliers": EM.RANGE,
                  "foundations": {k: {"days": v[0], "what": v[1], "where": v[2]} for k, v in EM.FOUNDATIONS.items()},
                  "note": "unit costs are judgement [inferred]; classification and counts are computed"},
        "scopes": scopes,
        "p0_slice": {
            "entries_min": len(minimal), "entries_ext": len(ext),
            "stages": dict(Counter(x["stage"] for x in minimal)),
            "distinct_server_routes_min": len({x["row"]["norm_path"] for x in minimal}),
            "by_class_min": dict(Counter(x["row"]["effort_class"] for x in minimal)),
            "validation_errors": errors,
            "external_calls_not_in_server_table": sorted(f"{c}:{m}" for c, m in external),
            "entries": [{"id": x["id"], "stage": x["stage"], "kind": x["kind"], "method": x["method"], "path": x["path"], "call": x["call"],
                         "trigger": x["trigger"], "class": x["row"]["effort_class"], "complexity": x["row"]["complexity"],
                         "days": round(x["row"]["days_marginal"] * x.get("scale", 1.0), 3), "scale": x.get("scale", 1.0),
                         "features": x["row"]["features"], "handler": x["row"]["handler_loc"], "why": x["why"]}
                        for x in slice_rows],
            "foundations_used": {k: {"fraction": frac, "why": why, "days": round(EM.FOUNDATIONS[k][0] * frac, 2)}
                                 for k, (frac, why) in p0_slice.SLICE_FOUNDATIONS.items()},
            "excluded_foundations": sorted(set(EM.FOUNDATIONS) - set(p0_slice.SLICE_FOUNDATIONS)),
        },
        "p0_class_by_feature": {c: dict(v) for c, v in p0_by_class_feature.items()},
        "facts": {
            "p0_rows_with_bg_tasks": sum(1 for r in p0 if (r.get("bg_tasks") or 0) > 0),
            "slice_entries_whose_handler_dispatches_a_task": [x["id"] for x in slice_rows if (x["row"].get("bg_tasks") or 0) > 0],
            "grouped_listing_rows": sorted(f"{r['surface']} {r['method']} {r['path']} [{r['disposition']}]" for r in compat if r["effort_class"] == "grouped-listing"),
            "paginated_listing_rows": sum(1 for r in compat if r["effort_class"] == "paginated-listing"),
            "slice_methods": dict(Counter(x["method"] for x in minimal)),
            "slice_tiers": dict(Counter(x["row"]["disposition"] for x in minimal)),
            "slice_non_p0_rows": sorted(f"{x['id']} {x['method']} {x['path']} [{x['row']['disposition']}]" for x in minimal if x["row"]["disposition"] != "P0"),
        },
        "permission_decorator_combos": {f"{k[0]} @ {k[1]}": n for k, n in perm.most_common()},
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, type=Path)
    a = ap.parse_args()
    res = run(a.out)
    res.pop("_layout_triggers", None)
    (a.out / "effort-summary.json").write_text(json.dumps(res, indent=1, sort_keys=True) + "\n")
    print(json.dumps({k: res["scopes"][k]["total_days_likely"] for k in res["scopes"]}, indent=1))
    print("validation errors:", res["p0_slice"]["validation_errors"])


if __name__ == "__main__":
    main()

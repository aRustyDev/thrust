# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Static-vs-static diff of two inventory runs (e.g. pre-pin vs pinned Plane commit).

    uv run compare_runs.py --old <out dir A> --new <out dir B> [--json diff.json]

Everything is keyed by things that survive file moves and line-number drift (surface/method/normalised path, Class.member,
verb, client path). Sections:
  headline                  RUN.json counts side by side, plus disposition and effort totals
  server_rows_added/removed (surface, method, normalised path)
  server_contract_changed   same route, but its view class / kind / action / serializers / permission classes / decorators /
                            pagination or group_by use changed (the things a facade must reproduce)
  serializer_shape_changed  same route, serializer declared-field or Meta.fields count changed (response-shape drift)
  server_handler_loc_changed  count only: handler body length moved (refactors move lines; contract-relevant ones are above)
  server_handler_body_changed handler source text changed (blank/comment lines ignored), listed for the compat surfaces and counted per
                            surface; needs both scanned trees on disk (RUN.json tree_root, or --old-root/--new-root). It sees queryset
                            filters, new error branches and dropped Celery calls; it does NOT see the view class's get_queryset/mixins
  client_calls_added/removed, client_refs_added/removed
  live_unmatched_*          call sites that look live but have no route, old vs new
  inventory_changes         disposition / tier / effort class / client status / complexity moved for the same route
  shapes_changed            shapes-anchors.json with every file:line stripped (envelope, filters, error codes, cookies, ...)
  p0_slice                  per slice entry: days, class, handler LOC, serializers, resolved trigger line; totals old vs new
Plain `diff -ru A B -x RUN.json -x layout-report.json` also works; this tool is for a readable summary.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


def load(d: Path, name: str):
    return json.loads((d / name).read_text())


def strip_locs(o):
    if isinstance(o, dict):
        return {k: strip_locs(v) for k, v in o.items() if k not in ("loc", "line", "first_seen", "cursor_str_loc", "paginate_loc", "auth_error_codes_loc", "sites", "file")}
    if isinstance(o, list):
        return [strip_locs(x) for x in o]
    return o


def tree_diff(a, b, path=""):
    out = []
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a:
                out.append(f"+ {path}/{k}: {json.dumps(b[k])[:200]}")
            elif k not in b:
                out.append(f"- {path}/{k}: {json.dumps(a[k])[:200]}")
            else:
                out += tree_diff(a[k], b[k], f"{path}/{k}")
    elif a != b:
        out.append(f"~ {path}: {json.dumps(a)[:240]}  ->  {json.dumps(b)[:240]}")
    return out


CONTRACT_FIELDS = ("view_class", "view_kind", "action", "serializers", "permission_classes", "authentication_classes", "decorators",
                   "paginates", "mentions_group_by", "mentions_bulk", "mentions_storage", "mentions_redirect", "drf_default")


def body_hashes(rows: dict, root: Path | None) -> dict | None:
    """sha1 of each handler's source text (blank and comment-only lines dropped), keyed like `rows`; None when the tree is not on disk.

    The AST contract fields above cannot see a changed queryset filter, a new 403 branch or a dropped Celery call; the handler text can.
    Rows with no handler location at all are skipped. A DRF-default row (`file:line (get_queryset)`) is compared on its get_queryset body.
    For ordinary rows the view class's own `get_queryset` / mixins are NOT part of the span.
    """
    import hashlib

    if root is None or not root.is_dir():
        return None
    cache: dict[str, list[str]] = {}
    out = {}
    for k, r in rows.items():
        loc, n = r.get("handler_loc"), r.get("handler_loc_n")
        if not loc or not n:
            continue
        m = re.fullmatch(r"(.+?):(\d+)(?: \(.*\))?", loc)  # `file:line` or `file:line (get_queryset)` for DRF-default rows
        if not m:
            raise ValueError(f"unparseable handler_loc {loc!r} for {k}")
        f, ln = m.group(1), m.group(2)
        if f not in cache:
            p = root / f
            cache[f] = p.read_text(encoding="utf-8").splitlines() if p.is_file() else []
        lines = cache[f][int(ln) - 1:int(ln) - 1 + int(n)]
        norm = "\n".join(s.rstrip() for s in lines if s.strip() and not s.strip().startswith("#"))
        out[k] = hashlib.sha1(norm.encode()).hexdigest()
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--old", required=True, type=Path)
    ap.add_argument("--new", required=True, type=Path)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--old-root", type=Path, help="tree the OLD run scanned (default: tree_root from its RUN.json)")
    ap.add_argument("--new-root", type=Path, help="tree the NEW run scanned (default: tree_root from its RUN.json)")
    a = ap.parse_args()

    res: dict = {}
    ro, rn = load(a.old, "RUN.json"), load(a.new, "RUN.json")
    res["commits"] = {"old": ro["tree_commit"], "new": rn["tree_commit"]}
    res["headline"] = {k: {"old": ro.get(k), "new": rn.get(k)} for k in ("server_routes", "server_rows", "client_calls", "join_unmatched_by_category")}

    # ---- server rows
    so = {(r["surface"], r["method"], r["norm_path"]): r for r in load(a.old, "server-routes.json")}
    sn = {(r["surface"], r["method"], r["norm_path"]): r for r in load(a.new, "server-routes.json")}
    res["server_rows_added"] = sorted(" ".join(k) for k in sn.keys() - so.keys())
    res["server_rows_removed"] = sorted(" ".join(k) for k in so.keys() - sn.keys())
    both = so.keys() & sn.keys()
    contract, shape, loc_changed = [], [], 0
    for k in sorted(both):
        d = {f: (so[k].get(f), sn[k].get(f)) for f in CONTRACT_FIELDS if so[k].get(f) != sn[k].get(f)}
        if d:
            contract.append(f"{' '.join(k)}: " + "; ".join(f"{f} {x!r}->{y!r}" for f, (x, y) in d.items()))
        if so[k].get("serializer_fields") != sn[k].get("serializer_fields"):
            shape.append(f"{' '.join(k)}: {so[k].get('serializer_fields')!r} -> {sn[k].get('serializer_fields')!r}")
        if so[k].get("handler_loc_n") != sn[k].get("handler_loc_n"):
            loc_changed += 1
    res["server_contract_changed"] = contract
    res["serializer_shape_changed"] = shape
    res["server_handler_loc_changed (count)"] = loc_changed

    # ---- handler source text (catches body changes the AST contract fields and the LOC count cannot see)
    ho = body_hashes(so, a.old_root or (Path(ro["tree_root"]) if ro.get("tree_root") else None))
    hn = body_hashes(sn, a.new_root or (Path(rn["tree_root"]) if rn.get("tree_root") else None))
    body_changed_keys: set = set()
    if ho is None or hn is None:
        res["server_handler_body_changed"] = "SKIPPED: a scanned tree root is not on disk (pass --old-root / --new-root); this section is not evidence of no change"
    else:
        body_changed_keys = {k for k in both if k in ho and k in hn and ho[k] != hn[k]}
        by_surface: dict = {}
        for k in body_changed_keys:
            by_surface[k[0]] = by_surface.get(k[0], 0) + 1
        res["server_handler_body_changed_by_surface"] = dict(sorted(by_surface.items()))
        res["server_handler_body_changed_rows_without_project_handler (not compared)"] = {s: sum(1 for k, r in sn.items() if k[0] == s and not r.get("handler_loc_n")) for s in sorted({k[0] for k in sn})}
        res["server_handler_body_changed"] = sorted(f"{' '.join(k)} @ {sn[k]['handler_loc']}" for k in body_changed_keys if k[0] != "v1")

    # ---- client calls / refs
    def ckey(c):
        return (c["client"], f"{c['cls']}.{c['member']}", c["verb"], tuple(c["paths"]))

    co = {ckey(c) for c in load(a.old, "client-calls.json")}
    cn = {ckey(c) for c in load(a.new, "client-calls.json")}
    res["client_calls_added"] = sorted(" | ".join(map(str, k)) for k in cn - co)
    res["client_calls_removed"] = sorted(" | ".join(map(str, k)) for k in co - cn)

    def rkey(r):
        return (r["client"], r["kind"], tuple(r["paths"]))

    ro_, rn_ = {rkey(r) for r in load(a.old, "client-refs.json")}, {rkey(r) for r in load(a.new, "client-refs.json")}
    res["client_refs_added"] = sorted(" | ".join(map(str, k)) for k in rn_ - ro_)
    res["client_refs_removed"] = sorted(" | ".join(map(str, k)) for k in ro_ - rn_)

    # ---- join: live-looking unmatched call sites
    def live(d):
        rep = load(d, "join-report.json")
        return {(u["call"], u["verb"], u["status"][0]) for u in rep["client_call_sites_unmatched_list"] if not u["dead"]}, rep

    lo, jo = live(a.old)
    ln, jn = live(a.new)
    res["live_unmatched_old_count"], res["live_unmatched_new_count"] = len(lo), len(ln)
    res["live_unmatched_added"] = sorted(" | ".join(x) for x in ln - lo)
    res["live_unmatched_removed"] = sorted(" | ".join(x) for x in lo - ln)
    res["join_counts"] = {k: {"old": jo[k], "new": jn[k]} for k in (
        "client_call_sites_total", "client_call_sites_matched", "client_call_sites_unmatched", "client_call_sites_unmatched_dead_vs_live",
        "client_alternatives_total", "client_alternatives_matched", "client_unmatched_by_category", "not_used_by_surface",
        "not_used_internal_by_method", "disposition_by_surface")}
    res["join_counts"]["matched_without_trailing_slash"] = {"old": len(jo["client_matched_without_trailing_slash"]), "new": len(jn["client_matched_without_trailing_slash"])}
    res["join_counts"]["multi_client_routes"] = {"old": len(jo["multi_client_routes"]), "new": len(jn["multi_client_routes"])}
    res["join_counts"]["routes_called_from_shared_packages_services"] = {"old": len(jo["routes_called_from_shared_packages_services"]), "new": len(jn["routes_called_from_shared_packages_services"])}

    # ---- inventory
    io = {(r["surface"], r["method"], r["norm_path"]): r for r in load(a.old, "endpoint-inventory.json")}
    inn = {(r["surface"], r["method"], r["norm_path"]): r for r in load(a.new, "endpoint-inventory.json")}
    ch = []
    for k in sorted(io.keys() & inn.keys()):
        d = {f: (io[k][f], inn[k][f]) for f in ("disposition", "tier", "effort_class", "client_status", "complexity") if io[k][f] != inn[k][f]}
        if d:
            ch.append(f"{' '.join(k)}: " + ", ".join(f"{f} {x}->{y}" for f, (x, y) in d.items()))
    res["inventory_changes"] = ch

    # ---- shapes
    res["shapes_changed"] = tree_diff(strip_locs(load(a.old, "shapes-anchors.json")), strip_locs(load(a.new, "shapes-anchors.json")))

    # ---- slice + effort
    eo, en = load(a.old, "effort-summary.json"), load(a.new, "effort-summary.json")
    trig = None
    if (a.new / "layout-report.json").exists():
        trig = (load(a.new, "layout-report.json").get("slice_triggers") or {}).get("resolved")
    old_e = {e["id"]: e for e in eo["p0_slice"]["entries"]}
    sl = []
    for e in en["p0_slice"]["entries"]:
        o = old_e.get(e["id"])
        row_o = so.get(("internal", e["method"], e["path"])) or next((v for k, v in so.items() if k[1] == e["method"] and k[2] == e["path"] and k[0] != "v1"), None)
        row_n = sn.get(("internal", e["method"], e["path"])) or next((v for k, v in sn.items() if k[1] == e["method"] and k[2] == e["path"] and k[0] != "v1"), None)
        d = {}
        if o is None:
            d["entry"] = "new"
        else:
            for f in ("days", "class", "complexity"):
                if o[f] != e[f]:
                    d[f] = (o[f], e[f])
        if row_o and row_n:
            if row_o.get("handler_loc_n") != row_n.get("handler_loc_n"):
                d["handler_loc_n"] = (row_o.get("handler_loc_n"), row_n.get("handler_loc_n"))
            if row_o.get("serializer_fields") != row_n.get("serializer_fields"):
                d["serializer_fields"] = (row_o.get("serializer_fields"), row_n.get("serializer_fields"))
        kn = next((k for k, v in sn.items() if v is row_n), None)
        if kn is not None and kn in body_changed_keys:
            d["handler_body"] = "changed"
        if trig and e["id"] in trig and (trig[e["id"]].get("drift_from") or trig[e["id"]].get("moved_file")):
            d["trigger"] = f"{trig[e['id']]['file']}:{trig[e['id']]['line']}"
        if d:
            sl.append(f"{e['id']} {e['method']} {e['path']}: " + "; ".join(f"{k} {v}" for k, v in d.items()))
    res["p0_slice_entry_changes"] = sl
    res["p0_slice"] = {
        "validation_errors_new": en["p0_slice"]["validation_errors"],
        "minimum_slice_days_likely": {"old": eo["scopes"]["minimum_slice"]["total_days_likely"], "new": en["scopes"]["minimum_slice"]["total_days_likely"]},
        "minimum_slice_days_low_high": {"old": [eo["scopes"]["minimum_slice"]["total_days_low"], eo["scopes"]["minimum_slice"]["total_days_high"]],
                                        "new": [en["scopes"]["minimum_slice"]["total_days_low"], en["scopes"]["minimum_slice"]["total_days_high"]]},
        "p0_used_days_likely": {"old": eo["scopes"]["P0_used"]["total_days_likely"], "new": en["scopes"]["P0_used"]["total_days_likely"]},
        "p0_used_days_low_high": {"old": [eo["scopes"]["P0_used"]["total_days_low"], eo["scopes"]["P0_used"]["total_days_high"]],
                                  "new": [en["scopes"]["P0_used"]["total_days_low"], en["scopes"]["P0_used"]["total_days_high"]]},
        "scopes_rows_and_days": {k: {"old": [eo["scopes"][k]["rows"], eo["scopes"][k]["total_days_likely"]], "new": [en["scopes"][k]["rows"], en["scopes"][k]["total_days_likely"]]}
                                 for k in eo["scopes"]},
        "p0_class_rows_old_new": {c: [eo["scopes"]["P0_used"]["by_class"][c]["rows"], en["scopes"]["P0_used"]["by_class"][c]["rows"]] for c in eo["scopes"]["P0_used"]["by_class"]},
    }
    for k, v in res.items():
        if isinstance(v, list):
            print(f"== {k} ({len(v)})")
            for x in v[:300]:
                print("  ", x)
        else:
            print(f"== {k}\n  {json.dumps(v, sort_keys=True)}")
    if a.json:
        a.json.write_text(json.dumps(res, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()

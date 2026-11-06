# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Steps 3, 4 and 6 - join server routes to client call sites, tag with FEATURES.md ids/tiers, size by effort class.

Inputs : out/server-routes.json, out/client-calls.json, out/client-refs.json (from the two scanners),
         FEATURES.md (tiers are parsed from it), feature_rules.py (route -> feature ids), effort_model.py,
         p0_slice.py (hand-curated minimum slice, validated against the join).
Outputs: out/endpoint-inventory.csv|json  (one row per server method x path)
         out/join-matches.csv             (one row per client path alternative -> server route)
         out/join-report.json             (unused routes, unmatched calls, multi-client routes, counts)
         out/effort-summary.json          (class counts and person-days; P0 slice; foundations)

Usage:  uv run join_tag.py --out <dir> [--features <FEATURES.md>]
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import effort_model as EM  # noqa: E402
import feature_rules as FR  # noqa: E402

TIER_RANK = {"P0": 0, "P1": 1, "P2": 2, "unassigned": 3, "drop": 4}
EXTRA_TIERS = {"F-A5": "P0/P1 (recommended default, FEATURES A5)"}
REQUESTED_APPS = ("web", "space")  # the apps the brief names; admin/live are extra clients


# ------------------------------------------------------------------ FEATURES.md
def parse_features(path: Path) -> dict[str, str]:
    tiers: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if not cells or not re.fullmatch(r"F-[A-Z]\d+[a-z]?", cells[0]):
            continue
        last = cells[-1]
        m = re.search(r"\bP[012]\b|drop|—", last)
        tiers[cells[0]] = {"—": "unassigned"}.get(m.group(0), m.group(0)) if m else "unassigned"
    return tiers


def best_tier(feats: list[str], tiers: dict[str, str], override: str | None) -> tuple[str, str]:
    if override:
        return override, "rule-override"
    got = []
    for f in feats:
        t = tiers.get(f)
        if t is None and f in EXTRA_TIERS:
            return "P0/P1", "FEATURES A5 recommended default"
        got.append(t or "unassigned")
    return (min(got, key=lambda t: TIER_RANK[t]) if got else "unassigned"), "FEATURES.md"


# ------------------------------------------------------------------ path matching
def norm_client(p: str) -> str:
    p = p.partition("?")[0]
    p = re.sub(r"\{[^{}]*\}", "{}", p)
    return p.rstrip("/")


def split(p: str) -> list[str]:
    return [s for s in p.strip("/").split("/")]


def seg_cmp(c: str, s: str):
    """-> 'exact' | 'param' | None  (client segment c vs server segment s; both already placeholder-normalised)."""
    if c == s:
        return "exact"
    if s == "{}":
        return "param"
    if "{}" in s:  # mixed server segment like {}-{}
        rx = "^" + re.escape(s).replace(re.escape("{}"), "[^/]+") + "$"
        if re.match(rx, c.replace("{}", "x")):
            return "param"
    return None


class ServerTable:
    def __init__(self, rows: list[dict]):
        by_path: dict[str, dict] = {}
        for r in rows:
            k = r["norm_path"].rstrip("/") or "/"
            e = by_path.setdefault(k, {"norm_path": k, "segs": split(k), "methods": {}, "order": len(by_path)})
            e["methods"][r["method"]] = r
        self.paths = list(by_path.values())

    def match(self, cpath: str, verb: str):
        """Best server path for a client path. Returns (status, entry|None, quality, ties)."""
        cs = split(cpath)
        best = []
        for e in self.paths:
            ss = e["segs"]
            if len(ss) != len(cs):
                continue
            exact, ok = 0, True
            for a, b in zip(cs, ss):
                r = seg_cmp(a, b)
                if r is None:
                    ok = False
                    break
                exact += r == "exact"
            if ok:
                best.append((exact, -e["order"], e))
        if not best:
            return "no-route", None, None, []
        best.sort(key=lambda x: (x[0], x[1]), reverse=True)
        with_verb = [b for b in best if verb.upper() in b[2]["methods"] or "ANY" in b[2]["methods"]]
        if not with_verb:
            return "method-mismatch", best[0][2], best[0][0], [b[2]["norm_path"] for b in best[:3]]
        top = with_verb[0]
        ties = [b[2]["norm_path"] for b in with_verb if b[0] == top[0]]
        return "ok", top[2], top[0], ties


# ------------------------------------------------------------------ main
def load(out: Path):
    srv = json.loads((out / "server-routes.json").read_text())
    calls = json.loads((out / "client-calls.json").read_text())
    refs = json.loads((out / "client-refs.json").read_text())
    return srv, calls, refs


def ref_method(kind: str) -> str:
    if kind.startswith("form-action"):
        return "POST"
    return "GET"


def run(out: Path, features_md: Path) -> dict:
    tiers = parse_features(features_md)
    srv, calls, refs = load(out)
    table = ServerTable([r for r in srv if r["method"] != "ANY"] + [r for r in srv if r["method"] == "ANY"])
    # ---- client items -> alternatives
    items = []
    for c in calls:
        uses = c["method_uses_by_app"]
        dead = c["method_uses_elsewhere"] == 0
        total = sum(uses.values())
        if c["client"] in ("web", "space", "admin", "live", "pkg-other"):
            # an app's own service file can only be consumed by that app: names found in other apps are collisions
            uses = {c["client"]: total} if total else {}
        elif c["client"] == "pkg-services":
            named = ("web", "space", "admin", "live")
            strict_apps = {a: n for a, n in uses.items() if a in named} if c["method_use_evidence"] == "strict" else {}
            if strict_apps:  # member name found in a file that mentions the class: attribute to those apps
                uses = strict_apps
            elif total:  # only by-name evidence elsewhere: fall back to the apps that import the class at all
                class_named = {a: n for a, n in c.get("class_consumer_apps", {}).items() if a in named}
                uses = class_named or {"?": total}
            else:
                uses = {}
        if c["client"] == "live":
            # apps/live reaches its services through a factory (getPageService), so by-name liveness is unreliable
            uses, dead = (uses or {"live": 1}), False
        items.append({"src": "call", "client": c["client"], "file": c["file"], "line": c["line"], "label": f"{c['cls']}.{c['member']}",
                      "verb": c["verb"].upper(), "paths": c["paths"], "resolution": c["resolution"], "note": c.get("note", ""),
                      "external": str(c.get("note", "")).startswith("EXTERNAL"), "uses": uses,
                      "dead": dead, "raw": c})
    for r in refs:
        items.append({"src": "ref:" + r["kind"], "client": r["client"], "file": r["file"], "line": r["line"], "label": r["kind"],
                      "verb": ref_method(r["kind"]), "paths": r["paths"], "resolution": "literal", "note": "", "external": False,
                      "uses": {(r["client"] if r["client"] in ("web", "space", "admin", "live") else "?"): 1},
                      "dead": False, "raw": r})
    matches, unmatched = [], []
    hits: dict[tuple, list] = defaultdict(list)  # (method, norm_path) -> list of item dicts
    for it in items:
        if it["external"] or not it["paths"]:
            continue
        results = []
        for p in it["paths"]:
            if not (p.startswith("/api/") or p.startswith("/auth/")):
                results.append((p, "non-api-path", None, None, []))
                continue
            st, e, q, ties = table.match(norm_client(p), it["verb"])
            results.append((p, st, e, q, ties))
        any_ok = any(st == "ok" for _, st, *_ in results)
        for p, st, e, q, ties in results:
            m = {"client": it["client"], "file": it["file"], "line": it["line"], "label": it["label"], "src": it["src"],
                 "verb": it["verb"], "client_path": p, "status": st,
                 "client_trailing_slash": p.partition("?")[0].endswith("/"),
                 "server_path": e["norm_path"] if e else "", "match_exact_segments": q, "ambiguous": len(ties) > 1,
                 "ties": ";".join(ties[1:]) if len(ties) > 1 else ""}
            matches.append(m)
            if st == "ok":
                hits[(it["verb"], e["norm_path"])].append(it)
            else:
                variant = any_ok and len(results) > 1
                unmatched.append({**m, "category": ("variant-fanout" if variant else st), "dead": it["dead"], "uses": it["uses"],
                                  "resolution": it["resolution"]})
    # ---- server rows
    rows = []
    for r in srv:
        key = (r["method"], r["norm_path"].rstrip("/") or "/")
        its = hits.get(key, [])
        apps: Counter = Counter()
        sites = []
        for it in its:
            if it["src"] == "call":
                for app, n in it["uses"].items():
                    if n:
                        apps[app] += n
            else:
                apps[it["client"]] += 1
            sites.append(f"{it['file']}:{it['line']}")
        # live-dead: a call whose service method is never referenced is not counted as use
        live_calls = [it for it in its if it["src"] != "call" or not it["dead"]]
        tg = FR.tag(r["surface"], r["norm_path"], r["method"])
        feats = tg["features"] if tg else []
        tier, tier_src = best_tier(feats, tiers, tg["tier_override"] if tg else None)
        rows.append({
            "surface": r["surface"], "method": r["method"], "path": r["full_path"], "norm_path": r["norm_path"],
            "view_class": r["view_class"], "action": r["action"], "handler_loc": r["handler_loc"],
            "route_entry_loc": r["route_entry_loc"], "permission_classes": r["permission_classes"],
            "features": feats, "tier": tier, "tier_source": tier_src, "tag_note": tg["note"] if tg else "UNMAPPED",
            "client_calls": len(its), "live_client_calls": len(live_calls),
            "used_by_apps": dict(apps), "call_sites": sites,
            "services": sorted({f"{it['label']}" for it in its if it["src"] == "call"}),
            "ref_kinds": sorted({it["src"] for it in its if it["src"] != "call"}),
            "response_types": sorted({it["raw"].get("response_type", "") for it in its if it["src"] == "call" and it["raw"].get("response_type")}),
            "query_params": sorted({k for it in its if it["src"] == "call" for k in (it["raw"].get("params_keys") or "").split(",") if k}),
            "params_types": sorted({it["raw"].get("params_type") for it in its if it["src"] == "call" and it["raw"].get("params_type")}),
            "body_types": sorted({it["raw"].get("body_type") for it in its if it["src"] == "call" and it["raw"].get("body_type")}),
            "catch_throws": sorted({it["raw"].get("catch_throws") for it in its if it["src"] == "call" and it["raw"].get("catch_throws")}),
            "effort_class": EM.classify(r), "complexity": EM.bucket(r), "days_marginal": EM.row_days(r),
            "handler_loc_n": r.get("handler_loc_n"), "orm_calls": r.get("orm_calls"), "agg_calls": r.get("agg_calls"), "bg_tasks": r.get("bg_tasks"),
            "serializers": r.get("serializers"), "serializer_fields": r.get("serializer_fields"),
            "mentions_group_by": r.get("mentions_group_by"), "paginates": r.get("paginates"),
            "drf_default": r.get("drf_default"),
        })
    # ---- client status + disposition
    for row in rows:
        apps = row["used_by_apps"]
        surf = row["surface"]
        in_req = any(a in apps for a in REQUESTED_APPS) or "?" in apps  # "?" = loose by-name evidence only
        if surf == "v1":
            status, disp = "external-contract", "v1-out-of-scope"
        elif surf == "schema":
            status, disp = "n/a", "drop"
        elif surf == "web":
            status, disp = "infra", "infra"
        elif in_req:
            status, disp = "used", row["tier"]
        elif apps and set(apps) <= {"admin"}:
            status, disp = "admin-only", "admin-only"
        elif apps and set(apps) <= {"live"}:
            status, disp = "live-only", "live-only"
        elif apps:
            status, disp = "used-other", row["tier"]
        elif row["client_calls"] and not row["live_client_calls"]:
            status, disp = "dead-client-method", "not-used"
        else:
            status, disp = "no-client-call", "not-used"
        row["client_status"], row["disposition"] = status, disp
    # auth callbacks / redirect targets are reached by the identity provider's redirect, not by a client call
    by_key = {(r["surface"], r["method"], r["norm_path"]): r for r in rows}
    for r in rows:
        if r["surface"] == "auth" and r["disposition"] == "not-used" and re.search(r"/callback/$", r["norm_path"]):
            init = by_key.get(("auth", "GET", r["norm_path"].replace("callback/", "")))
            if init and init["client_status"] == "used":
                r["client_status"], r["disposition"] = "used-via-redirect [inferred]", r["tier"]
    return {"rows": rows, "matches": matches, "unmatched": unmatched, "items": items, "tiers": tiers, "hits": hits}


def write(out: Path, res: dict):
    rows, matches, unmatched = res["rows"], res["matches"], res["unmatched"]
    (out / "endpoint-inventory.json").write_text(json.dumps(rows, indent=1, sort_keys=True) + "\n")
    cols = ["surface", "method", "path", "view_class", "action", "features", "tier", "tier_source", "client_status", "disposition",
            "client_calls", "live_client_calls", "used_by_apps", "services", "ref_kinds", "response_types", "query_params", "body_types",
            "effort_class", "complexity", "days_marginal", "handler_loc", "route_entry_loc", "call_sites", "tag_note"]
    with open(out / "endpoint-inventory.csv", "w", newline="") as fh:
        wr = csv.writer(fh)
        wr.writerow(cols)
        def cell(v):
            s = ";".join(map(str, v)) if isinstance(v, list) else (json.dumps(v, sort_keys=True) if isinstance(v, dict) else v)
            return re.sub(r"\s+", " ", s) if isinstance(s, str) else s  # one physical line per row

        for r in sorted(rows, key=lambda r: (r["surface"], r["norm_path"], r["method"])):
            wr.writerow([cell(r[c]) for c in cols])
    with open(out / "join-matches.csv", "w", newline="") as fh:
        mc = ["client", "file", "line", "label", "src", "verb", "client_path", "client_trailing_slash", "status", "server_path", "match_exact_segments", "ambiguous", "ties"]
        wr = csv.DictWriter(fh, fieldnames=mc)
        wr.writeheader()
        for m in sorted(matches, key=lambda m: (m["client"], m["file"], m["line"])):
            wr.writerow(m)
    (out / "join-unmatched.json").write_text(json.dumps(unmatched, indent=1, sort_keys=True) + "\n")


def report(res: dict) -> dict:
    rows, matches, unmatched, items = res["rows"], res["matches"], res["unmatched"], res["items"]
    internal = [r for r in rows if r["surface"] in ("internal", "space", "instance", "auth")]
    rep: dict = {}
    rep["server_rows_by_surface"] = Counter(r["surface"] for r in rows)
    rep["server_rows_by_client_status"] = {s: Counter(r["client_status"] for r in rows if r["surface"] == s) for s in sorted({r["surface"] for r in rows})}
    rep["disposition_internal_space_instance_auth"] = Counter(r["disposition"] for r in internal)
    rep["disposition_by_surface"] = {s: Counter(r["disposition"] for r in rows if r["surface"] == s) for s in sorted({r["surface"] for r in rows})}
    rep["server_routes_no_client_use"] = [
        f"{r['surface']} {r['method']} {r['path']}  [{r['client_status']}] {','.join(r['features'])}"
        for r in sorted(internal, key=lambda r: (r["surface"], r["path"], r["method"])) if r["disposition"] == "not-used"
    ]
    named = ("web", "space", "admin", "live")
    nu = [r for r in internal if r["disposition"] == "not-used"]
    rep["not_used_by_surface"] = Counter(r["surface"] for r in nu)
    rep["not_used_internal_by_method"] = Counter(r["method"] for r in nu if r["surface"] == "internal")
    rep["not_used_internal_by_primary_feature"] = Counter((r["features"] or ["?"])[0] for r in nu if r["surface"] == "internal")
    rep["not_used_internal_by_status"] = Counter(r["client_status"] for r in nu if r["surface"] == "internal")
    rep["put_rows_internal"] = {"total": sum(1 for r in rows if r["surface"] == "internal" and r["method"] == "PUT"),
                                "not_used": sum(1 for r in nu if r["surface"] == "internal" and r["method"] == "PUT")}
    rep["multi_client_routes"] = [
        f"{r['method']} {r['path']}  {{{', '.join(f'{a}:{n}' for a, n in sorted(r['used_by_apps'].items()) if a in named)}}}"
        for r in rows if len([a for a in r["used_by_apps"] if a in named]) >= 2
    ]
    rep["routes_called_from_shared_packages_services"] = [
        f"{r['method']} {r['path']}" for r in rows
        if any(s.startswith("packages/services/") for s in r["call_sites"]) and r["surface"] != "v1"
    ]
    cats = Counter(u["category"] for u in unmatched)
    rep["client_alternatives_total"] = len(matches)
    rep["client_alternatives_matched"] = sum(1 for m in matches if m["status"] == "ok")
    rep["client_unmatched_by_category"] = cats
    rep["client_unmatched"] = [
        {k: u[k] for k in ("category", "client", "file", "line", "label", "verb", "client_path", "server_path", "dead", "resolution")}
        for u in sorted(unmatched, key=lambda u: (u["category"], u["file"], u["line"]))
    ]
    # per CALL SITE (not per path alternative): matched if at least one alternative matched a server route + verb
    per_site: dict[tuple, list] = defaultdict(list)
    for m in matches:
        per_site[(m["client"], m["file"], m["line"], m["label"], m["verb"], m["src"])].append(m["status"])
    dead_by_site = {(it["client"], it["file"], it["line"], it["label"], it["verb"], it["src"]): it["dead"] for it in items}
    external_sites = [it for it in items if it["external"] or not it["paths"]]
    rep["client_call_sites_total"] = len(per_site) + len(external_sites)
    rep["client_call_sites_external_not_plane_api"] = len(external_sites)
    rep["client_call_sites_matched"] = sum(1 for v in per_site.values() if "ok" in v)
    unm_sites = {k: v for k, v in per_site.items() if "ok" not in v}
    rep["client_call_sites_unmatched"] = len(unm_sites)
    rep["client_call_sites_unmatched_dead_vs_live"] = {
        "dead_service_method(no reference elsewhere)": sum(1 for k in unm_sites if dead_by_site.get(k)),
        "live": sum(1 for k in unm_sites if not dead_by_site.get(k)),
    }
    rep["client_call_sites_unmatched_list"] = [
        {"site": f"{k[1]}:{k[2]}", "client": k[0], "call": k[3], "verb": k[4], "status": sorted(set(v)), "dead": bool(dead_by_site.get(k))}
        for k, v in sorted(unm_sites.items(), key=lambda kv: (kv[0][1], kv[0][2]))
    ]
    rep["client_matched_without_trailing_slash"] = [
        f"{m['verb']} {m['client_path']}  {m['file']}:{m['line']}"
        for m in matches if m["status"] == "ok" and m["src"] == "call" and not m["client_trailing_slash"]
    ]
    rep["ambiguous_matches"] =[{k: m[k] for k in ("file", "line", "client_path", "server_path", "ties")} for m in matches if m["ambiguous"]]
    rep["unmapped_server_routes"] = [f"{r['surface']} {r['method']} {r['path']}" for r in rows if r["tag_note"] == "UNMAPPED"]
    # server rows matched by more than one distinct client item
    return json.loads(json.dumps(rep, default=dict))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--features", type=Path, default=Path(__file__).resolve().parents[2] / "FEATURES.md")
    a = ap.parse_args()
    res = run(a.out, a.features)
    write(a.out, res)
    rep = report(res)
    (a.out / "join-report.json").write_text(json.dumps(rep, indent=1, sort_keys=True) + "\n")
    print(json.dumps({k: rep[k] for k in ("server_rows_by_surface", "disposition_internal_space_instance_auth", "client_alternatives_total",
                                           "client_alternatives_matched", "client_unmatched_by_category", "unmapped_server_routes")}, indent=1, sort_keys=True))


if __name__ == "__main__":
    main()

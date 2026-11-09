#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Q-0122.1: does the drf-spectacular schema cover every /api/v1 operation in the URL conf?

Inputs
  --urlconf  JSON from oracle/scripts/urlconf_ops.py (what the running Django actually routes)
  --schema   the OpenAPI JSON served at /api/schema/?format=json
Outputs
  JSON report (--out) and a short text summary on stdout.

An "operation" is (METHOD, path-template). Path parameter *names* are ignored for matching
(`{slug}` == `{workspace_slug}`) and compared separately, because a renamed parameter is a
documentation difference, not a missing route.

Gaps are CLASSIFIED, never just counted:
  put-filtered     the project's own preprocessing hook drops every PUT (settings/openapi.py ->
                   utils/openapi/hooks.py::preprocess_filter_api_v1_paths)
  server-filtered  the same hook drops any path containing "server"
  not-in-schema    anything else: routed but absent from the schema for another reason
                   (extend_schema(exclude=True), or a view drf-spectacular could not introspect)

Not counted as operations (reported under excluded_from_operation_count, never dropped silently):
  phantom          routed, but the handler's signature cannot accept the route's kwargs, so a
                   call raises TypeError (a 500). Typical cause: one view class mounted on both a
                   list route and a detail route.
  router-format    DRF router `.format`-suffix twins of real routes.
  router-root      DRF router API-root views (framework, not Plane API surface).

--self-test is the control that proves the comparison can see a gap: it deletes one known
operation from a copy of the schema and requires the report to list exactly that operation.
Exit status: 0 if the report was produced (and the self-test, when asked, passed); 2 if the
self-test failed (the comparator is then void); 1 with --fail-on-gap when any gap exists.
"""

import argparse
import copy
import json
import re
import sys

PARAM = re.compile(r"\{[^}]+\}")
METHODS = ("get", "post", "put", "patch", "delete")


def shape(path):
    """Path template with parameter names erased: /a/{x}/b -> /a/{}/b."""
    return PARAM.sub("{}", path)


def schema_ops(schema):
    ops = {}
    for path, item in schema.get("paths", {}).items():
        if not path.startswith("/api/v1/"):
            continue
        for m in METHODS:
            if m in item:
                ops[(m.upper(), shape(path))] = {
                    "path": path,
                    "method": m.upper(),
                    "operationId": item[m].get("operationId"),
                    "tags": item[m].get("tags", []),
                }
    return ops


def is_real(o):
    """A real, callable v1 operation: a normal or router route whose handler accepts the route."""
    return o.get("kind", "path") in ("path", "router") and o.get("signature_ok", True)


def urlconf_ops(urlconf):
    ops = {}
    for o in urlconf:
        if is_real(o):
            ops[(o["method"], shape(o["path"]))] = o
    return ops


def excluded(urlconf):
    """Routed (route, method) pairs that are NOT counted as operations, with the reason."""
    out = {"phantom": [], "router-format": [], "router-root": []}
    for o in urlconf:
        if o.get("kind") == "router-format":
            out["router-format"].append(o)
        elif o.get("kind") == "router-root":
            out["router-root"].append(o)
        elif not o.get("signature_ok", True):
            out["phantom"].append(o)
    return out


def classify(op):
    if op["method"] == "PUT":
        return "put-filtered"
    if "server" in op["path"].lower():
        return "server-filtered"
    return "not-in-schema"


def compare(urlconf, schema):
    u = urlconf_ops(urlconf)
    s = schema_ops(schema)
    missing = sorted(set(u) - set(s))
    extra = sorted(set(s) - set(u))
    gaps = []
    for key in missing:
        o = u[key]
        gaps.append({"method": o["method"], "path": o["path"], "view": o["view"], "class": classify(o)})
    by_class = {}
    for g in gaps:
        by_class[g["class"]] = by_class.get(g["class"], 0) + 1
    renamed = []
    for key in sorted(set(u) & set(s)):
        up = PARAM.findall(u[key]["path"])
        sp = PARAM.findall(s[key]["path"])
        if up != sp:
            renamed.append({"method": key[0], "urlconf": u[key]["path"], "schema": s[key]["path"]})
    ex = excluded(urlconf)
    return {
        "urlconf_operations": len(u),
        "schema_operations": len(s),
        "covered": len(set(u) & set(s)),
        "gap_total": len(gaps),
        "gap_by_class": by_class,
        "gaps": gaps,
        "in_schema_not_in_urlconf": [{"method": k[0], "path": s[k]["path"]} for k in extra],
        "param_name_differences": renamed,
        "excluded_from_operation_count": {
            k: [
                {"method": o["method"], "path": o["path"], "view": o["view"].split(".")[-1],
                 "why": (f"missing={o['sig_missing']} unexpected={o['sig_unexpected']}"
                         if k == "phantom" else k)}
                for o in v
            ]
            for k, v in ex.items()
        },
    }


def self_test(urlconf, schema):
    """Remove one operation the schema DOES cover; the comparator must report exactly it."""
    s = schema_ops(schema)
    u = urlconf_ops(urlconf)
    victims = [k for k in sorted(set(u) & set(s)) if k[0] == "GET"]
    if not victims:
        return False, "no GET operation is covered, nothing to remove"
    method, shp = victims[0]
    path = s[(method, shp)]["path"]
    mutated = copy.deepcopy(schema)
    del mutated["paths"][path][method.lower()]
    base = compare(urlconf, schema)
    after = compare(urlconf, mutated)
    new = [g for g in after["gaps"] if (g["method"], shape(g["path"])) == (method, shp)]
    ok = after["gap_total"] == base["gap_total"] + 1 and len(new) == 1
    return ok, f"removed {method} {path}: gaps {base['gap_total']} -> {after['gap_total']}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--urlconf", required=True)
    ap.add_argument("--schema", required=True)
    ap.add_argument("--out")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--fail-on-gap", action="store_true")
    a = ap.parse_args()

    urlconf = json.load(open(a.urlconf))
    schema = json.load(open(a.schema))

    if a.self_test:
        ok, msg = self_test(urlconf, schema)
        print(f"self-test (control must detect a removed operation): {'PASS' if ok else 'FAIL'} -- {msg}")
        if not ok:
            print("COMPARATOR IS VOID: it did not notice a deleted operation.", file=sys.stderr)
            return 2

    rep = compare(urlconf, schema)
    if a.out:
        json.dump(rep, open(a.out, "w"), indent=1, sort_keys=True)
        open(a.out, "a").write("\n")
    print(f"urlconf operations : {rep['urlconf_operations']}")
    print(f"schema operations  : {rep['schema_operations']}")
    print(f"covered            : {rep['covered']}")
    print(f"gap                : {rep['gap_total']}  {rep['gap_by_class']}")
    print(f"schema-only (not routed): {len(rep['in_schema_not_in_urlconf'])}")
    print(f"param-name differences : {len(rep['param_name_differences'])}")
    exc = rep["excluded_from_operation_count"]
    print("excluded from the count: " + ", ".join(f"{k}={len(v)}" for k, v in exc.items()))
    return 1 if (a.fail_on_gap and rep["gap_total"]) else 0


if __name__ == "__main__":
    sys.exit(main())

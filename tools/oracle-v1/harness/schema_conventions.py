#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Q-0032 input: what naming/structure conventions does the pinned v1 schema actually use?

ADR-0032's hybrid decision stands (v1 is code-first in Rust, constrained by a breaking-change diff
against this frozen schema). This script does not decide anything; it records the facts a Rust
OpenAPI generator (utoipa-style) must be configured to reproduce so that the diff stays quiet:
operationId shape, tags, component-schema names, parameter names, security scheme, status codes.

Usage: schema_conventions.py <openapi.json> [--out results/schema-conventions.json]
"""

import argparse
import collections
import json
import re
import sys

METHODS = ("get", "post", "put", "patch", "delete")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("schema")
    ap.add_argument("--out")
    a = ap.parse_args()
    s = json.load(open(a.schema))

    ops = []
    for path, item in s["paths"].items():
        for m in METHODS:
            if m in item:
                o = item[m]
                ops.append((m.upper(), path, o))

    op_ids = [o.get("operationId") for _, _, o in ops]
    dup = [k for k, n in collections.Counter(op_ids).items() if n > 1]
    missing_id = [(m, p) for m, p, o in ops if not o.get("operationId")]
    snake = re.compile(r"^[a-z][a-z0-9]*(_[a-z0-9]+)*$")
    non_snake = [i for i in op_ids if i and not snake.match(i)]

    verbs = collections.Counter(i.split("_")[0] for i in op_ids if i)
    tags = collections.Counter(t for _, _, o in ops for t in o.get("tags", []))
    untagged = [(m, p) for m, p, o in ops if not o.get("tags")]
    declared_tags = [t["name"] for t in s.get("tags", [])]

    comps = s.get("components", {})
    schemas = sorted(comps.get("schemas", {}))
    suffixes = collections.Counter()
    for n in schemas:
        for suf in ("Request", "Response", "PatchedRequest", "Create", "Update", "List", "Detail", "Enum"):
            if n.endswith(suf):
                suffixes[suf] += 1
    prefix_paginated = [n for n in schemas if n.startswith("Paginated")]

    status = collections.Counter()
    for _, _, o in ops:
        for code in o.get("responses", {}):
            status[code] += 1

    params = collections.Counter()
    for path, item in s["paths"].items():
        for m in METHODS:
            if m in item:
                for p in item[m].get("parameters", []):
                    params[(p.get("in"), p.get("name"))] += 1

    out = {
        "openapi": s.get("openapi"),
        "info": {k: s["info"].get(k) for k in ("title", "version")},
        "servers": s.get("servers"),
        "operation_count": len(ops),
        "operationId": {
            "all_present": not missing_id,
            "missing": missing_id,
            "unique": not dup,
            "duplicates": dup,
            "all_snake_case": not non_snake,
            "non_snake_case": non_snake,
            "leading_verb_counts": dict(verbs.most_common()),
            "examples": sorted(op_ids)[:12],
        },
        "tags": {
            "declared": declared_tags,
            "used_counts": dict(tags.most_common()),
            "untagged_operations": untagged,
            "declared_but_unused": sorted(set(declared_tags) - set(tags)),
            "used_but_undeclared": sorted(set(tags) - set(declared_tags)),
        },
        "components": {
            "schema_count": len(schemas),
            "suffix_counts": dict(suffixes),
            "paginated_wrappers": prefix_paginated,
            "names": schemas,
            "other_kinds": {k: sorted(v) for k, v in comps.items() if k != "schemas"},
        },
        "security": {"schemes": comps.get("securitySchemes"), "global": s.get("security")},
        "response_status_counts": dict(sorted(status.items())),
        "parameters_by_use": [
            {"in": k[0], "name": k[1], "uses": n} for k, n in params.most_common()
        ],
    }
    txt = json.dumps(out, indent=1, sort_keys=True)
    if a.out:
        open(a.out, "w").write(txt + "\n")
    print(f"openapi {out['openapi']}; {out['operation_count']} operations; {len(schemas)} component schemas")
    print(f"operationId: present={out['operationId']['all_present']} unique={out['operationId']['unique']} "
          f"snake_case={out['operationId']['all_snake_case']}")
    print(f"tags used: {len(tags)}; untagged ops: {len(untagged)}; declared-unused: {out['tags']['declared_but_unused']}")
    print(f"statuses: {out['response_status_counts']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

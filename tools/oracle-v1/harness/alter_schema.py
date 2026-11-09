#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Make deliberately altered copies of the pinned schema, to prove the schema-diff job can see them.

    alter_schema.py <base.json> --breaking out_breaking.json --benign out_benign.json [--manifest m.json]

breaking  six DIFFERENT kinds of break in one file (so a diff tool that only notices one kind
          cannot pass): removed operation, response property type change, removed response
          property, request property made required, request enum member removed, new required
          query parameter.
benign    purely additive changes (new optional response property, new endpoint). A correct gate
          must NOT fail on these; otherwise it would reject every legitimate extension.

The mutation list is written next to the output (<out>.mutations.json) so the checker can compare
what was injected with what the diff tool reported.
"""

import argparse
import copy
import json
import sys


def breaking(s):
    s = copy.deepcopy(s)
    C = s["components"]["schemas"]
    muts = []

    # B1: remove a whole operation
    del s["paths"]["/api/v1/users/me/"]["get"]
    if not any(m in s["paths"]["/api/v1/users/me/"] for m in ("get", "post", "patch", "delete", "put")):
        del s["paths"]["/api/v1/users/me/"]
    muts.append({"id": "B1-remove-operation", "what": "removed GET /api/v1/users/me/"})

    # B2: response property type string -> integer
    C["State"]["properties"]["name"] = {"type": "integer"}
    muts.append({"id": "B2-response-type-change", "what": "State.name: string -> integer"})

    # B3: remove a required response property
    del C["State"]["properties"]["color"]
    muts.append({"id": "B3-response-property-removed", "what": "State.color removed (it is required)"})

    # B4: request property becomes required
    req = C["StateRequest"]
    req["required"] = sorted(set(req.get("required", [])) | {"description"})
    muts.append({"id": "B4-request-property-required", "what": "StateRequest.description became required"})

    # B5: request enum member removed (GroupEnum is referenced by StateRequest.group)
    C["GroupEnum"]["enum"] = [v for v in C["GroupEnum"]["enum"] if v != "triage"]
    muts.append({"id": "B5-request-enum-member-removed", "what": "GroupEnum lost 'triage'"})

    # B6: new REQUIRED query parameter on an existing operation
    op = s["paths"]["/api/v1/workspaces/{slug}/projects/{project_id}/labels/"]["get"]
    op.setdefault("parameters", []).append(
        {"in": "query", "name": "oracle_required_param", "required": True, "schema": {"type": "string"}}
    )
    muts.append({"id": "B6-new-required-parameter", "what": "GET labels list requires query param oracle_required_param"})
    return s, muts


def benign(s):
    s = copy.deepcopy(s)
    muts = []
    s["components"]["schemas"]["State"]["properties"]["oracle_new_optional"] = {"type": "string"}
    muts.append({"id": "N1-new-optional-response-property", "what": "State.oracle_new_optional (optional)"})
    s["paths"]["/api/v1/workspaces/{slug}/oracle-benign/"] = {
        "get": {
            "operationId": "oracle_benign_endpoint",
            "tags": ["Users"],
            "parameters": [{"in": "path", "name": "slug", "required": True, "schema": {"type": "string"}}],
            "responses": {"200": {"description": "ok"}},
        }
    }
    muts.append({"id": "N2-new-endpoint", "what": "GET /api/v1/workspaces/{slug}/oracle-benign/"})
    return s, muts


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base")
    ap.add_argument("--breaking", required=True)
    ap.add_argument("--benign", required=True)
    a = ap.parse_args()
    base = json.load(open(a.base))
    for fn, out in ((breaking, a.breaking), (benign, a.benign)):
        doc, muts = fn(base)
        json.dump(doc, open(out, "w"))
        json.dump(muts, open(out + ".mutations.json", "w"), indent=1)
        print(f"{out}: {len(muts)} mutation(s): " + ", ".join(m["id"] for m in muts))
    return 0


if __name__ == "__main__":
    sys.exit(main())

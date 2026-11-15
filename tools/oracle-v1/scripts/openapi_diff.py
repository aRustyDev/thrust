#!/usr/bin/env python3
"""The parts of `scripts/check-openapi.sh` that read JSON: scoping the pinned schema, judging what `oasdiff` found, and breaking a
document on purpose for the controls. Standard library only (it runs under `python3 -I`, with no `uv` and no PyYAML).

  openapi_diff.py scope <pin.json> <operations> <out.json>     the pin cut to the listed operations (and the schemas they reach)
  openapi_diff.py same-operations <doc.json> <operations>      exit 1 unless the document has exactly the listed operations
  openapi_diff.py classify <oasdiff.json> <accepted.json> <operations>
                                                               exit 0 only if every finding is an accepted difference and every
                                                               accepted difference is found (on each operation it names)
  openapi_diff.py mutate <doc.json> <kind> <out.json>          break a document on purpose: drop-property, retype, require
  openapi_diff.py self-test                                    the cases that show each of the above can fail

Exit codes: 0 clean, 1 a finding that is not accepted or an accepted difference that is not found (stale), 2 the check cannot vouch
for itself (nothing to compare, a name that is not there, a file that does not parse).

`<operations>` is a file of operationIds, one per line, `#` for comments: the part of the pinned schema that this crate's document
describes so far (issue #58: it grows a resource at a time, to the 32 operations of the scope `m1`).

`<accepted.json>` is the register of differences that were decided, one entry per (oasdiff check, text) with the operations it holds
on and the reason. oasdiff's own `--err-ignore` has no notion of an entry that stopped being true; this one fails on it, as the
oracle gate's register does ("a register entry that matches nothing fails as stale", `tools/oracle-v1/exceptions.yaml`).
"""

import json
import sys

METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")
REF = "#/components/schemas/"


class Cannot(Exception):
    """The check cannot vouch for itself (exit 2)."""


def load(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError) as e:
        raise Cannot(f"{path}: {e}")


def read_operations(path):
    try:
        with open(path) as f:
            lines = [l.strip() for l in f]
    except OSError as e:
        raise Cannot(f"{path}: {e}")
    ops = [l for l in lines if l and not l.startswith("#")]
    if not ops:
        raise Cannot(f"{path}: no operations listed")
    dup = sorted({o for o in ops if ops.count(o) > 1})
    if dup:
        raise Cannot(f"{path}: listed twice: {dup}")
    return ops


def operations_of(doc):
    """operationId -> (METHOD, path); an operation with no operationId, or a repeated one, cannot be compared."""
    out = {}
    for path, item in doc.get("paths", {}).items():
        for m in METHODS:
            if m in item:
                oid = item[m].get("operationId")
                if not oid:
                    raise Cannot(f"{m.upper()} {path}: no operationId")
                if oid in out:
                    raise Cannot(f"operationId {oid} is on two operations")
                out[oid] = (m.upper(), path)
    return out


def same_operations(doc, allow):
    """(operations the document has that the list lacks, operations the list has that the document lacks)."""
    have, want = set(operations_of(doc)), set(allow)
    return sorted(have - want), sorted(want - have)


def scope(pin, allow):
    """The pin with only the allowed operations, and only the component schemas they reach."""
    paths, found = {}, set()
    for path, item in pin["paths"].items():
        keep = {k: v for k, v in item.items() if k not in METHODS}
        for m in METHODS:
            if m in item and item[m].get("operationId") in allow:
                keep[m] = item[m]
                found.add(item[m]["operationId"])
        if any(m in keep for m in METHODS):
            paths[path] = keep
    missing = sorted(set(allow) - found)
    if missing:
        raise Cannot(f"not operations of the pin: {missing}")
    schemas = pin.get("components", {}).get("schemas", {})
    need = set()

    def walk(o):
        if isinstance(o, dict):
            r = o.get("$ref")
            if isinstance(r, str) and r.startswith(REF):
                name = r[len(REF):]
                if name not in schemas:
                    raise Cannot(f"$ref to a schema the pin does not have: {name}")
                if name not in need:
                    need.add(name)
                    walk(schemas[name])
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for v in o:
                walk(v)

    walk(paths)
    out = {k: v for k, v in pin.items() if k not in ("paths", "components")}
    out["paths"] = paths
    comps = {k: v for k, v in pin.get("components", {}).items() if k != "schemas"}
    comps["schemas"] = {k: schemas[k] for k in sorted(need)}
    out["components"] = comps
    return out


def validate_accepted(accepted):
    entries = accepted.get("accepted")
    if accepted.get("format_version") != 1 or not isinstance(entries, list):
        raise Cannot("accepted: format_version 1 and a list `accepted` are required")
    ids = set()
    for e in entries:
        for k in ("id", "ref", "check", "text_contains", "operations", "reason"):
            if not e.get(k):
                raise Cannot(f"accepted {e.get('id')}: `{k}` is required")
        if e["id"] in ids:
            raise Cannot(f"accepted: {e['id']} twice")
        ids.add(e["id"])
    return entries


def classify(findings, accepted, allow):
    """(unaccepted findings, stale (entry, operation) pairs, ambiguous findings)."""
    entries = validate_accepted(accepted)
    allowed = set(allow)
    for e in entries:
        outside = sorted(set(e["operations"]) - allowed)
        if outside:
            raise Cannot(f"accepted {e['id']} names operations that are not in the scope: {outside}")
    hits = {(e["id"], op): 0 for e in entries for op in e["operations"]}
    unaccepted, ambiguous = [], []
    for f in findings:
        matched = [
            e
            for e in entries
            if f["id"] == e["check"] and f["operationId"] in e["operations"] and e["text_contains"] in f["text"]
        ]
        if not matched:
            unaccepted.append(f)
            continue
        if len(matched) > 1:
            ambiguous.append(f)
        for e in matched:
            hits[(e["id"], f["operationId"])] += 1
    stale = sorted(k for k, n in hits.items() if n == 0)
    return unaccepted, stale, ambiguous


def mutate(doc, kind):
    """Break the document in a way a schema diff must see. A name that is not in the document is `Cannot`: a control that mutates
    nothing would pass for the wrong reason."""
    schemas = doc.get("components", {}).get("schemas", {})
    if kind == "drop-property":
        # a key the routes send, gone from what they say they send
        target = next((n for n in sorted(schemas) if n.endswith("Dto") and "name" in schemas[n].get("properties", {})), None)
        if target is None:
            raise Cannot("drop-property: no response schema with a `name` property")
        del schemas[target]["properties"]["name"]
        if "name" in schemas[target].get("required", []):
            schemas[target]["required"].remove("name")
    elif kind == "retype":
        target = next(
            (n for n in sorted(schemas) if n.endswith("Dto") and "is_member" in schemas[n].get("properties", {})), None
        )
        if target is None:
            raise Cannot("retype: no response schema with an `is_member` property")
        schemas[target]["properties"]["is_member"] = {"type": "string"}
    elif kind == "require":
        target = next((n for n in sorted(schemas) if n.endswith("Request") and n.startswith("Project")), None)
        if target is None or "description" not in schemas[target].get("properties", {}):
            raise Cannot("require: no project request schema with a `description` property")
        req = schemas[target].setdefault("required", [])
        if "description" in req:
            raise Cannot("require: `description` is already required")
        req.append("description")
    else:
        raise Cannot(f"unknown mutation {kind}")
    return doc


# --- the cases that show the above can fail -----------------------------------------------------------------------------------


def _finding(check, op, text):
    return {"id": check, "operationId": op, "text": text, "level": 3}


def _entry(eid, check, contains, ops):
    return {"id": eid, "ref": "X-1", "check": check, "text_contains": contains, "operations": ops, "reason": "r"}


def self_test():
    failed = []

    def expect(label, got, want):
        ok = got == want
        print(f"{'ok  ' if ok else 'FAIL'} {label}" + ("" if ok else f": got {got!r}, want {want!r}"))
        if not ok:
            failed.append(label)

    def raises(label, fn):
        try:
            fn()
        except Cannot:
            expect(label, "cannot", "cannot")
        else:
            expect(label, "returned", "cannot")

    f1 = _finding("response-property-became-nullable", "op_a", "the response property `x` became nullable")
    f2 = _finding("request-body-media-type-removed", "op_b", "removed the media type `multipart/form-data`")
    e1 = _entry("SD-1", "response-property-became-nullable", "`x`", ["op_a"])
    e2 = _entry("SD-2", "request-body-media-type-removed", "multipart", ["op_b"])
    allow = ["op_a", "op_b"]

    def run(findings, entries):
        u, s, a = classify(findings, {"format_version": 1, "accepted": entries}, allow)
        return (len(u), len(s), len(a))

    expect("every finding accepted, every entry found", run([f1, f2], [e1, e2]), (0, 0, 0))
    expect("a finding no entry covers is unaccepted", run([f1, f2], [e1]), (1, 0, 0))
    expect("an entry that finds nothing is stale", run([f1], [e1, e2]), (0, 1, 0))
    expect("nothing found: every entry is stale", run([], [e1, e2]), (0, 2, 0))
    # the same difference on ANOTHER operation is not accepted (one unaccepted), and e1 then finds nothing on its own (one stale);
    # e2 finds its finding on op_b, so it is not stale
    expect("an entry holds on its operations only", run([_finding(f1["id"], "op_b", f1["text"]), f2], [e1, e2]), (1, 1, 0))
    expect(
        "an entry for two operations is stale until both have the finding",
        run([f1], [_entry("SD-1", f1["id"], "`x`", ["op_a", "op_b"])]),
        (0, 1, 0),
    )
    expect(
        "a finding two entries cover is ambiguous",
        run([f1], [e1, _entry("SD-3", f1["id"], "nullable", ["op_a"])]),
        (0, 0, 1),
    )
    raises("an entry with no reason cannot be read", lambda: validate_accepted(
        {"format_version": 1, "accepted": [{**e1, "reason": ""}]}))
    raises("an entry outside the scope cannot be judged", lambda: classify(
        [f1], {"format_version": 1, "accepted": [_entry("SD-9", f1["id"], "`x`", ["op_z"])]}, allow))

    pin = {
        "openapi": "3.0.3",
        "paths": {
            "/a/": {"get": {"operationId": "op_a", "responses": {"200": {"content": {"application/json": {"schema": {"$ref": REF + "A"}}}}}}},
            "/b/": {"get": {"operationId": "op_b"}, "post": {"operationId": "op_c"}},
        },
        "components": {"schemas": {"A": {"properties": {"n": {"$ref": REF + "B"}}}, "B": {"type": "string"}, "C": {"type": "string"}}},
    }
    expect("the document has the listed operations and no others", same_operations(pin, ["op_a", "op_b", "op_c"]), ([], []))
    expect("an operation the list lacks is reported", same_operations(pin, ["op_a", "op_b"]), (["op_c"], []))
    expect("an operation the document lacks is reported", same_operations(pin, ["op_a", "op_b", "op_c", "op_d"]), ([], ["op_d"]))
    cut = scope(pin, ["op_a", "op_c"])
    expect("scope keeps the listed operations", sorted(operations_of(cut)), ["op_a", "op_c"])
    expect("scope keeps the schemas they reach, and only those", sorted(cut["components"]["schemas"]), ["A", "B"])
    raises("scope refuses an operation the pin does not have", lambda: scope(pin, ["op_a", "op_zz"]))
    raises("a document with an operation that has no operationId cannot be compared", lambda: operations_of(
        {"paths": {"/x/": {"get": {}}}}))

    doc = {
        "components": {
            "schemas": {
                "ThingDto": {"properties": {"name": {"type": "string"}, "is_member": {"type": "boolean"}}, "required": ["name"]},
                "ProjectXRequest": {"properties": {"description": {"type": "string"}}},
            }
        }
    }

    def mutated(kind):
        return mutate(json.loads(json.dumps(doc)), kind)["components"]["schemas"]

    expect("drop-property removes `name` and its `required`", mutated("drop-property")["ThingDto"],
           {"properties": {"is_member": {"type": "boolean"}}, "required": []})
    expect("retype changes `is_member`", mutated("retype")["ThingDto"]["properties"]["is_member"], {"type": "string"})
    expect("require makes `description` required", mutated("require")["ProjectXRequest"]["required"], ["description"])
    raises("a mutation with nothing to mutate cannot pass", lambda: mutate({"components": {"schemas": {}}}, "drop-property"))
    raises("an unknown mutation cannot pass", lambda: mutate(json.loads(json.dumps(doc)), "nope"))

    if failed:
        print(f"{len(failed)} case(s) failed")
        return 1
    return 0


def main(argv):
    try:
        cmd = argv[1] if len(argv) > 1 else ""
        if cmd == "self-test":
            return self_test()
        if cmd == "scope" and len(argv) == 5:
            out = scope(load(argv[2]), read_operations(argv[3]))
            with open(argv[4], "w") as f:
                json.dump(out, f, indent=1, sort_keys=True)
            print(f"{len(operations_of(out))} operations, {len(out['components']['schemas'])} schemas")
            return 0
        if cmd == "same-operations" and len(argv) == 4:
            doc, allow = load(argv[2]), read_operations(argv[3])
            extra, missing = same_operations(doc, allow)
            if extra or missing:
                print(f"the document has {extra} that the list does not, and lacks {missing}")
                return 1
            print(f"{len(allow)} operations, the listed ones")
            return 0
        if cmd == "classify" and len(argv) == 5:
            findings = load(argv[2])
            if not isinstance(findings, list):
                raise Cannot(f"{argv[2]}: not a list of findings")
            allow = read_operations(argv[4])
            unaccepted, stale, ambiguous = classify(findings, load(argv[3]), allow)
            for f in unaccepted:
                print(f"UNACCEPTED {f['operationId']}: [{f['id']}] {f['text']}")
            for eid, op in stale:
                print(f"STALE {eid} on {op}: it is accepted and no longer found")
            for f in ambiguous:
                print(f"AMBIGUOUS {f['operationId']}: [{f['id']}] {f['text']} (two entries cover it)")
            print(f"{len(findings)} findings, {len(unaccepted)} unaccepted, {len(stale)} stale, {len(ambiguous)} ambiguous")
            return 1 if unaccepted or stale or ambiguous else 0
        if cmd == "mutate" and len(argv) == 5:
            with open(argv[4], "w") as f:
                json.dump(mutate(load(argv[2]), argv[3]), f, indent=1, sort_keys=True)
            return 0
        print(__doc__, file=sys.stderr)
        return 2
    except Cannot as e:
        print(f"cannot vouch: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))

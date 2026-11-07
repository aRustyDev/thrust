#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Judge the three oasdiff runs made by schema_diff.sh. Exit 0 only if the diff job is TRUSTWORTHY.

  self     must exit 0 and report nothing          (a diff that flags identical schemas is noise)
  altered  must exit non-zero AND report EACH injected mutation by its own oasdiff rule id
           (catching one kind of break but missing another would still be a blind diff job)
  benign   must exit 0 and report no ERR-level change (else every additive extension is rejected)
"""

import argparse
import json
import sys

# mutation id -> oasdiff rule id(s) that must appear at ERR level
EXPECT = {
    "B1-remove-operation": {"api-path-removed-without-deprecation", "api-removed-without-deprecation"},
    "B2-response-type-change": {"response-property-type-changed"},
    "B3-response-property-removed": {"response-required-property-removed"},
    "B4-request-property-required": {"request-property-became-required"},
    "B5-request-enum-member-removed": {"request-property-enum-value-removed"},
    "B6-new-required-parameter": {"new-required-request-parameter"},
}


def load(spec):
    path, rc = spec.rsplit(":", 1)
    txt = open(path).read().strip()
    data = json.loads(txt) if txt and txt != "null" else []
    return data, int(rc)


def main():
    ap = argparse.ArgumentParser()
    for k in ("self", "breaking", "benign"):
        ap.add_argument(f"--{k}", required=True)
    ap.add_argument("--mutations", required=True)
    ap.add_argument("--oasdiff-image", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    self_d, self_rc = load(a.self)
    brk_d, brk_rc = load(a.breaking)
    ben_d, ben_rc = load(a.benign)
    muts = json.load(open(a.mutations))
    err = lambda d: [x for x in d if int(x.get("level", 0)) >= 3]  # noqa: E731

    seen = {x["id"] for x in err(brk_d)}
    per_mutation = {m["id"]: sorted(EXPECT[m["id"]] & seen) for m in muts}
    checks = {
        "self: exit 0": self_rc == 0,
        "self: reports nothing": not self_d,
        "altered: exit non-zero": brk_rc != 0,
        "altered: every injected mutation detected": all(per_mutation.values()),
        "benign: exit 0": ben_rc == 0,
        "benign: no ERR-level change": not err(ben_d),
    }
    result = {
        "oasdiff": a.oasdiff_image,
        "self": {"exit": self_rc, "changes": len(self_d)},
        "altered": {"exit": brk_rc, "err_changes": len(err(brk_d)), "rule_ids_seen": sorted(seen),
                    "mutation_detected_by": per_mutation},
        "benign": {"exit": ben_rc, "changes": len(ben_d), "err_changes": len(err(ben_d))},
        "checks": checks,
        "verdict": "PASS" if all(checks.values()) else "FAIL",
        "not_done": ["utoipa code-first generation check (needs the Rust v1 slice; wave 2)"],
    }
    json.dump(result, open(a.out, "w"), indent=1, sort_keys=True)
    for k, ok in checks.items():
        print(("PASS " if ok else "FAIL ") + k)
    for mid, rules in per_mutation.items():
        print(f"  {mid:36} -> {rules or 'NOT DETECTED'}")
    print(f"altered: {len(err(brk_d))} ERR-level changes from {len(muts)} injected mutations; verdict {result['verdict']}")
    return 0 if result["verdict"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())

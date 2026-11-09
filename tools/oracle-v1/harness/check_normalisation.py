#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Control for the corpus: normalisation must be doing real work, and must leave no volatile value behind.

    check_normalisation.py <raw_dir> <normalised_dir>

A byte-identical re-recording only proves something if the RAW recording would NOT have been
identical. This counts the volatile values in the raw files (timestamps, server-generated uuids,
presigned signatures) and requires:
  * raw has MANY of them            (else the check is vacuous: the corpus had nothing volatile)
  * normalised has NONE of them     (else a volatile value leaked and the corpus will flap)
It deliberately counts INSIDE response/request bodies only, not the harness' own `elapsed_s`.
"""

import json
import re
import sys
from pathlib import Path

ISO = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?")
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", re.I)
# A presigned-URL signature VALUE is 64 hex chars. The FIELD NAME `x-amz-signature` is deliberately
# kept by the normaliser (only its value becomes <opaque>), so the name must not be counted.
SIG = re.compile(r"\b[0-9a-f]{64}\b")


# Dashless uuid4().hex, as embedded in storage keys (exactly 32 hex chars, not part of a longer run)
HEX32 = re.compile(r"(?<![0-9a-fA-F])[0-9a-fA-F]{32}(?![0-9a-fA-F])")


def volatile_in(text, known_ids):
    iso = len(ISO.findall(text))
    uu = [u for u in UUID.findall(text) if u.lower() not in known_ids]
    return iso, len(uu) + len(HEX32.findall(text)), len(SIG.findall(text))


def bodies_only(doc, raw):
    """Concatenate only request/response bodies (and request paths) so harness metadata is ignored."""
    out = []
    for e in doc if raw else doc["exchanges"]:
        if raw:
            out += [json.dumps(e.get("body")), json.dumps(e.get("request_body")), e.get("path", "")]
        else:
            out += [json.dumps(e["response"]["body"]), json.dumps(e["request"]["body"]), e["request"]["path"]]
    return "\n".join(out)


def main():
    raw_dir, norm_dir = Path(sys.argv[1]), Path(sys.argv[2])
    manifest = json.load(open(Path(__file__).resolve().parent.parent / "corpus" / "seed-manifest.json"))
    known = set()

    def walk(o):
        if isinstance(o, dict):
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for v in o:
                walk(v)
        elif isinstance(o, str) and UUID.fullmatch(o):
            known.add(o.lower())

    walk(manifest)
    # ids derived from the seed namespace but not listed in the manifest are still deterministic
    import uuid
    ns = uuid.UUID("5a0a0a0a-0000-4000-8000-0123456789ab")
    for name in ("comment:i3:1", "comment:i3:2", "link:i3:1", "activity:i3:1", "activity:i3:2"):
        known.add(str(uuid.uuid5(ns, name)))

    r_iso = r_uu = r_sig = n_iso = n_uu = n_sig = 0
    for f in sorted(raw_dir.glob("*.raw.json")):
        a, b, c = volatile_in(bodies_only(json.load(open(f)), True), known)
        r_iso, r_uu, r_sig = r_iso + a, r_uu + b, r_sig + c
    for f in sorted(norm_dir.glob("*.json")):
        if f.name in ("INDEX.json", "seed-manifest.json"):
            continue
        a, b, c = volatile_in(bodies_only(json.load(open(f)), False), set())
        n_iso, n_uu, n_sig = n_iso + a, n_uu + b, n_sig + c
    print(f"raw        : {r_iso} timestamps, {r_uu} non-seeded uuids, {r_sig} presigned signatures")
    print(f"normalised : {n_iso} timestamps, {n_uu} raw uuids, {n_sig} presigned signatures")
    ok_raw = r_iso >= 20 and r_uu >= 5 and r_sig >= 1
    ok_norm = n_iso == 0 and n_uu == 0 and n_sig == 0
    print(("PASS" if ok_raw else "FAIL") + " raw recordings contain plenty of volatile values (check is not vacuous)")
    print(("PASS" if ok_norm else "FAIL") + " normalised corpus contains none (nothing volatile leaked)")
    return 0 if ok_raw and ok_norm else 1


if __name__ == "__main__":
    sys.exit(main())

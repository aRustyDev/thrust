#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml"]
# ///
"""Apply oracle/normalise.yaml to recorded HTTP exchanges (Q-0122.2).

A library for the corpus recorder and the future differential runner, plus `--self-test`.

The rules file is authoritative; this code only executes it. See oracle/NORMALISATION.md for the
reasoning. In one line each:
  * uuid        seeded ids -> symbolic names; other ids -> <uuid:N> by first appearance
  * timestamps  datetime/date VALUES in time-like keys -> <ts>/<date>; null is left alone
  * verbatim    sequence_id, ordering fields, cursors and counts are never touched
  * opaque      presigned-upload signatures etc. -> <opaque> (shape kept)
  * headers     whitelist only; counters/epochs tokenised
  * ordering    arrays stay in API order (strict)
"""

import json
import re
import sys
from pathlib import Path

import yaml

ORACLE = Path(__file__).resolve().parent.parent


class Normaliser:
    def __init__(self, rules_path=ORACLE / "normalise.yaml", manifest=None):
        self.r = yaml.safe_load(open(rules_path))
        assert self.r["format_version"] == 1
        self.uuid_re = re.compile(self.r["uuid"]["regex"])
        self.hex32_re = re.compile(self.r["uuid"]["hex32_regex"])
        self.random_paths = [re.compile(x) for x in self.r["random_defaults"]["json_paths_regex"]]
        t = self.r["timestamps"]
        self.dt_key = re.compile(t["datetime_key_regex"])
        self.dt_val = re.compile(t["datetime_value_regex"])
        self.d_key = re.compile(t["date_key_regex"])
        self.d_val = re.compile(t["date_value_regex"])
        self.opaque = [re.compile(x) for x in self.r["opaque"]["json_paths_regex"]]
        self.known = self._symbols(manifest or {})
        self.ordinal = {}

    @staticmethod
    def _symbols(m):
        """uuid -> '<kind:name>' for every id the seed manifest knows."""
        s = {}

        def put(i, label):
            if i:
                s[i.lower()] = f"<{label}>"

        put(m.get("workspace", {}).get("id"), "workspace:oracle")
        for k, v in m.get("users", {}).items():
            put(v["id"], f"user:{k}")
        for k, v in m.get("projects", {}).items():
            put(v["id"], f"project:{k}")
        for k, v in m.get("states", {}).items():
            put(v, f"state:{k}")
        for k, v in m.get("labels", {}).items():
            put(v, f"label:{k}")
        for k, v in m.get("issues", {}).items():
            put(v["id"], f"issue:{k}")
        put(m.get("cycle"), "cycle:1")
        put(m.get("module"), "module:1")
        for k, v in m.get("project_members", {}).items():
            put(v, f"pmember:{k}")
        for k, v in m.get("comments", {}).items():
            put(v, f"comment:{k}")
        for k, v in m.get("links", {}).items():
            put(v, f"link:{k}")
        for k, v in m.get("activities", {}).items():
            put(v, f"activity:{k}")
        return s

    def reset(self):
        """Start a new recording: unknown-uuid ordinals restart at 1."""
        self.ordinal = {}

    # -- strings ------------------------------------------------------------------------------
    def _id(self, u):
        """Symbolic name for a seeded id, else the next ordinal. `u` is lower-case, dashed."""
        if u in self.known:
            return self.known[u]
        if u not in self.ordinal:
            self.ordinal[u] = len(self.ordinal) + 1
        return f"<uuid:{self.ordinal[u]}>"

    @staticmethod
    def _dash(h):
        return f"{h[:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:]}"

    def uuids(self, text):
        # dashed first, so the dashless pattern never looks inside an already-replaced id
        text = self.uuid_re.sub(lambda m: self._id(m.group(0).lower()), text)
        return self.hex32_re.sub(lambda m: self._id(self._dash(m.group(0).lower())), text)

    # -- JSON bodies --------------------------------------------------------------------------
    def body(self, obj, path=""):
        if isinstance(obj, dict):
            out = {}
            for k, v in obj.items():
                nk = self.uuids(k)  # dict keys can be ids (group_by responses)
                p = f"{path}.{k}" if path else k
                out[nk] = self._field(k, v, p)
            return out
        if isinstance(obj, list):
            return [self.body(x, path) for x in obj]
        if isinstance(obj, str):
            return self.uuids(obj)
        return obj

    def _field(self, key, v, path):
        if v is None:
            return None  # null vs set is semantic; never normalised
        if key in self.r["verbatim_keys"]:
            return v
        if any(rx.search(path) for rx in self.opaque):
            return self.r["opaque"]["token"]
        if any(rx.search(path) for rx in self.random_paths):
            return self.r["random_defaults"]["token"]
        if isinstance(v, str):
            if self.dt_key.search(key) and self.dt_val.match(v):
                return self.r["timestamps"]["datetime_token"]
            if self.d_key.search(key) and self.d_val.match(v):
                return self.r["timestamps"]["date_token"]
        return self.body(v, path)

    # -- headers ------------------------------------------------------------------------------
    def headers(self, hdrs, resource=None):
        h = self.r["headers"]
        keep, out = set(h["keep"]), {}
        verbatim = {k for k, res in (h.get("verbatim_in_resources") or {}).items() if resource in res}
        for k, v in hdrs.items():
            lk = k.lower()
            if lk not in keep:
                continue
            if lk in verbatim:
                pass  # per-resource override: this header's value IS the contract here
            elif re.search(h["epoch_regex"], lk):
                v = h["epoch_token"]
            elif re.search(h["int_regex"], lk):
                v = h["int_token"]
            else:
                v = self.uuids(v)
            out[lk] = v
        return dict(sorted(out.items()))

    # -- whole exchange -----------------------------------------------------------------------
    def request_path(self, path):
        return self.uuids(path)


def self_test():
    """Each rule must fire when it should and NOT fire when it should not."""
    m = {"workspace": {"id": "11111111-1111-1111-1111-111111111111"},
         "projects": {"alpha": {"id": "22222222-2222-2222-2222-222222222222"}}}
    n = Normaliser(manifest=m)
    rnd = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    rnd2 = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    doc = {
        "id": rnd, "project": "22222222-2222-2222-2222-222222222222", "other": rnd2, "again": rnd,
        "created_at": "2026-10-18T06:11:45.989341Z", "deleted_at": None, "archived_at": None,
        "start_date": "2026-10-19", "name": "2026-10-19",   # `name` is not a date key: untouched
        "sequence_id": 7, "sort_order": 65535.0, "next_cursor": "20:1:0", "total_count": 3,
        "label": "created_at", "when": "2026-10-18T06:11:45Z",  # `when` is not a time key: untouched
        "results": [{"id": rnd2}, {"id": rnd}],
        "upload_data": {"url": "http://plane-minio:9000/uploads", "method": "POST",
                        "fields": {"key": f"x/{rnd}", "policy": "abc", "Content-Type": "image/png"}},
        "group": {rnd: 1},
    }
    out = n.body(doc)
    checks = {
        "known id -> symbol": out["project"] == "<project:alpha>",
        "unknown id -> ordinal 1": out["id"] == "<uuid:1>",
        "second unknown -> ordinal 2": out["other"] == "<uuid:2>",
        "same id twice -> same ordinal": out["again"] == "<uuid:1>",
        "ordinals stable inside arrays": out["results"] == [{"id": "<uuid:2>"}, {"id": "<uuid:1>"}],
        "ids as dict keys": out["group"] == {"<uuid:1>": 1},
        "datetime -> <ts>": out["created_at"] == "<ts>",
        "null timestamp stays null": out["deleted_at"] is None and out["archived_at"] is None,
        "date -> <date>": out["start_date"] == "<date>",
        "non-date key untouched": out["name"] == "2026-10-19" and out["when"] == "2026-10-18T06:11:45Z",
        "sequence_id verbatim": out["sequence_id"] == 7,
        "sort_order/cursor/count verbatim": (out["sort_order"], out["next_cursor"], out["total_count"])
                                              == (65535.0, "20:1:0", 3),
        "presigned fields opaque": out["upload_data"]["fields"]["policy"] == "<opaque>"
                                   and out["upload_data"]["fields"]["key"] == "<opaque>",
        "Content-Type field kept": out["upload_data"]["fields"]["Content-Type"] == "image/png",
        "method kept": out["upload_data"]["method"] == "POST",
        "upload url opaque": out["upload_data"]["url"] == "<opaque>",
    }
    n.reset()
    again = n.body({"x": rnd2})
    checks["reset restarts ordinals"] = again == {"x": "<uuid:1>"}

    # dashless 32-hex ids (storage keys) and server-chosen random defaults
    n.reset()
    h = "0123456789abcdef0123456789abcdef"
    k = n.body({"asset": f"ws/{h}-file.png", "again": h, "dashed_same": "01234567-89ab-cdef-0123-456789abcdef",
                "short": "0123456789abcdef0123456789abcde", "long": h + "0",
                "known": "22222222222222222222222222222222",
                "logo_props": {"icon": {"name": "sync", "color": "#95999f"}, "in_use": "icon"},
                "name": "sync", "results": [{"logo_props": {"icon": {"name": "x", "color": "#000000"}}}]})
    checks["hex32 -> ordinal inside a storage key"] = k["asset"] == "ws/<uuid:1>-file.png"
    checks["hex32 twice / dashed spelling of same id -> same ordinal"] = k["again"] == "<uuid:1>" == k["dashed_same"]
    checks["31 or 33 hex chars are NOT an id"] = k["short"] == "0123456789abcdef0123456789abcde" and k["long"] == h + "0"
    checks["hex32 of a seeded id -> symbol"] = k["known"] == "<project:alpha>"
    checks["random default logo icon -> <random>"] = k["logo_props"] == {
        "icon": {"name": "<random>", "color": "<random>"}, "in_use": "icon"}
    checks["random-default rule is path-scoped (a plain `name` is untouched)"] = k["name"] == "sync"
    checks["random-default rule matches inside arrays"] = k["results"][0]["logo_props"]["icon"]["name"] == "<random>"
    h = n.headers({"Content-Type": "application/json", "Date": "x", "X-RateLimit-Reset": "1792304337",
                   "X-RateLimit-Remaining": "59", "Retry-After": "41", "Server": "uvicorn"})
    checks["header whitelist + tokens"] = h == {"content-type": "application/json",
                                                "retry-after": "<int>", "x-ratelimit-remaining": "<int>",
                                                "x-ratelimit-reset": "<epoch>"}
    h2 = n.headers({"X-RateLimit-Remaining": "4", "Retry-After": "41"}, resource="rate_limit")
    checks["remaining verbatim ONLY in rate_limit; retry-after still tokenised"] = h2 == {
        "retry-after": "<int>", "x-ratelimit-remaining": "4"}
    bad = [k for k, ok in checks.items() if not ok]
    for k, ok in checks.items():
        print(("PASS " if ok else "FAIL ") + k)
    print(f"{len(checks) - len(bad)}/{len(checks)} normalisation checks passed")
    return 1 if bad else 0


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    print(__doc__)

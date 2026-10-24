"""Normalisation of recorded exchanges, the Python half of a pair.

The Rust half is crates/thrust-compat-plane/tests/common/normalise.rs. Both read
crates/thrust-compat-plane/tests/parity/normalise.json and must produce the same output for the same input; the shared
cases in tests/parity/normalise-cases.json are checked by `record.py --self-test` here and by the
`parity_normaliser_cases` test there. If you change a rule, change both and add a case.
"""

import re

HEX = set("0123456789abcdefABCDEF")


def _is_hex_run(s, start, length):
    return start + length <= len(s) and all(c in HEX for c in s[start : start + length])


def _uuid_at(s, i):
    # 8-4-4-4-12 hex digits with dashes
    if i + 36 > len(s):
        return False
    for pos, ln in ((0, 8), (9, 4), (14, 4), (19, 4), (24, 12)):
        if not _is_hex_run(s, i + pos, ln):
            return False
    return all(s[i + p] == "-" for p in (8, 13, 18, 23))


def _canonical(raw):
    h = raw.replace("-", "").lower()
    return f"{h[0:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:32]}"


class Ids:
    """Ordinal numbering of ids by first appearance within one fixture."""

    def __init__(self, token):
        self.token = token
        self.seen = {}

    def ordinal(self, raw):
        key = _canonical(raw)
        if key not in self.seen:
            self.seen[key] = len(self.seen) + 1
        return self.token.replace("{n}", str(self.seen[key]))

    def scrub(self, s):
        out, i = [], 0
        while i < len(s):
            if _uuid_at(s, i) and not (i > 0 and s[i - 1] in HEX) and not (i + 36 < len(s) and s[i + 36] in HEX):
                out.append(self.ordinal(s[i : i + 36]))
                i += 36
            elif (
                _is_hex_run(s, i, 32)
                and not (i > 0 and s[i - 1] in HEX)
                and not (i + 32 < len(s) and s[i + 32] in HEX)
            ):
                out.append(self.ordinal(s[i : i + 32]))
                i += 32
            else:
                out.append(s[i])
                i += 1
        return "".join(out)


_DATETIME = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}")
_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")


def _key_matches(key, suffixes, exact=()):
    return key in exact or any(key.endswith(s) for s in suffixes)


def value(rules, ids, v, path=""):
    """Normalise one JSON value; `path` is the dotted path of `v` with `[]` for array elements."""
    if isinstance(v, dict):
        out = {}
        for k in sorted(v):
            p = f"{path}.{k}" if path else k
            child = v[k]
            token = rules["opaque_paths"].get(p)
            dt = rules["datetime"]
            dd = rules["date"]
            if token is not None and child is not None:
                out[k] = token
            elif isinstance(child, str) and _key_matches(k, dt["key_suffixes"], dt["key_exact"]) and _DATETIME.match(child):
                out[k] = dt["token"]
            elif isinstance(child, str) and _key_matches(k, dd["key_suffixes"]) and _DATE.match(child):
                out[k] = dd["token"]
            else:
                out[k] = value(rules, ids, child, p)
        return out
    if isinstance(v, list):
        return [value(rules, ids, x, f"{path}[]") for x in v]
    if isinstance(v, str):
        return ids.scrub(v)
    return v


_ORIGIN = re.compile(r"^https?://[^/?#]+")


def headers(rules, hs):
    keep = set(rules["headers_keep"])
    rewrite = rules["header_origin_rewrite"]
    out = {}
    for k, v in sorted(hs.items(), key=lambda kv: kv[0].lower()):
        name = k.lower()
        if name not in keep:
            continue
        out[name] = _ORIGIN.sub(rewrite["token"], v) if name in rewrite["headers"] else v
    return out


def exchange(rules, ids, ex):
    """Normalise one recorded exchange in the fixed traversal order: request path, request body, response body."""
    req, resp = ex["request"], ex["response"]
    out_req = {"method": req["method"], "path": ids.scrub(req["path"])}
    if req.get("body") is not None:
        out_req["body"] = value(rules, ids, req["body"])
    out_resp = {"status": resp["status"], "headers": headers(rules, resp.get("headers", {}))}
    out_resp["body"] = value(rules, ids, resp["body"]) if resp.get("body") is not None else None
    return {"name": ex["name"], "request": out_req, "response": out_resp}

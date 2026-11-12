#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Scrub a Playwright HAR of one journey on the pinned Plane, so that it can be committed as the input to spike 0009's replay harness.

    uv run --script tools/e2e-har/scrub.py --out e2e/har/<journey>.plane.har <recorded>.har
    uv run --script tools/e2e-har/scrub.py --self-test

The oracle is synthetic (throwaway credentials, loopback, a database that is dropped after the run), but a HAR holds every cookie and
token of a signed-in browser, so it is scrubbed rather than trusted: nothing in a committed file is a live credential, and a re-recording
does not differ from the last one by noise.

What is scrubbed: the value of every cookie (the `Cookie` and `Set-Cookie` headers and the `cookies` arrays, keeping names and
attributes); `Authorization`, `X-CSRFToken` and `X-Api-Key`; the fields `csrfmiddlewaretoken`, `csrf_token`, `password`,
`confirm_password`, `new_password` and the token fields, in a form body and in JSON, requests and responses; the signature of a
presigned upload URL (`X-Amz-Signature`, `X-Amz-Security-Token`) wherever the URL is, a JSON body, a `Location` or a query; the `Date` header, and the
`Expires` attribute of a `Set-Cookie` (dropped: `Max-Age` says the same, and no made-up time says "deleted"); start times (replaced by the entry's position, so the order stays), durations and timings; the connection details
(`serverIPAddress`, `connection`, the `_`-prefixed fields Playwright adds). What is kept: every URL, method, status, other header and body.
The origin in the URLs is the one recorded (http://localhost:18088); the replay maps it.

It does not renumber ids. If the replay harness turns out to need stable ids, tools/compat-record/normalise.py does that (a Rust and a
Python half that share rules and cases: import it, do not change a rule in one half only).

The check that the scrub worked is made on its OUTPUT: every secret it collected from the input (cookie values, tokens, passwords) must
be absent from the serialised result, or it exits 1 and writes nothing. And it refuses an input in which it found nothing to scrub (no
cookie, no CSRF token): that is not the HAR of a signed-in journey, and a "clean" result from it would mean nothing. `--self-test` plants
known secrets, scrubs, and requires both the scrub and the verifier to do their parts; the verifier is also run on the UNSCRUBBED
planted HAR and must fail there.
"""

import argparse
import base64
import copy
import json
import re
import sys
import urllib.parse
from datetime import datetime, timedelta, timezone
from pathlib import Path

REDACTED = "<redacted>"
EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)
FIXED_DATE = "Thu, 01 Jan 1970 00:00:00 GMT"
# a field of this name, at any depth of a JSON body or in a form, holds a secret
SECRET_FIELDS = {
    "csrfmiddlewaretoken", "csrf_token", "password", "confirm_password", "new_password", "token", "access_token", "refresh_token",
    "secret", "api_key", "x-amz-signature", "x-amz-security-token", "policy",
}
# A presigned URL carries its signature in the query string, and the URL is a string inside a JSON body (S28's answer), a `Location`
# header or a query. The oracle's store is throwaway and the signature has long expired, but a committed file holds no signature.
SIGNED_QUERY = re.compile(r"((?:X-Amz-Signature|X-Amz-Security-Token)=)([^&\"'\\\s]+)", re.IGNORECASE)
SECRET_HEADERS = {"authorization", "x-csrftoken", "x-api-key", "proxy-authorization"}
# entry keys that are not HAR's and say where and when, or are connection details
DROP_ENTRY_KEYS = {"serverIPAddress", "connection"}


class Found:
    """What the scrub took out, so that its output can be checked for it."""

    def __init__(self):
        self.secrets: set[str] = set()
        self.cookies = 0
        self.tokens = 0

    def add(self, value, kind):
        if isinstance(value, str) and len(value) >= 6 and value != REDACTED:
            self.secrets.add(value)
            if kind == "cookie":
                self.cookies += 1
            else:
                self.tokens += 1


def scrub_cookie_pairs(header_value, found):
    """`a=1; b=2` (a Cookie header) with each value redacted."""
    out = []
    for part in header_value.split(";"):
        name, sep, value = part.strip().partition("=")
        if sep:
            found.add(value, "cookie")
            out.append(f"{name}={REDACTED}")
        elif name:
            out.append(name)
    return "; ".join(out)


def scrub_set_cookie(header_value, found):
    """`sid=abc; Path=/; Expires=...; Max-Age=...; HttpOnly`: the value of the first pair redacted, the attributes kept, `Expires` dropped.

    `Expires` is the wall-clock time of the recording, so it differs on every run, and a made-up replacement (1970, say) would say the cookie
    was deleted. `Max-Age`, which Plane sends beside it and which wins in a browser, carries the lifetime."""
    parts = [p.strip() for p in header_value.split(";")]
    name, sep, value = parts[0].partition("=")
    found.add(value, "cookie")
    out = [f"{name}={REDACTED}" if sep else parts[0]]
    out.extend(attr for attr in parts[1:] if not attr.lower().startswith("expires="))
    return "; ".join(out)


def scrub_json(node, found):
    if isinstance(node, dict):
        out = {}
        for key, value in node.items():
            if key.lower() in SECRET_FIELDS and isinstance(value, str):
                found.add(value, "token")
                out[key] = REDACTED
            else:
                out[key] = scrub_json(value, found)
        return out
    if isinstance(node, list):
        return [scrub_json(v, found) for v in node]
    return node


def scrub_form(text, found):
    pairs = urllib.parse.parse_qsl(text, keep_blank_values=True)
    out = []
    for key, value in pairs:
        if key.lower() in SECRET_FIELDS:
            found.add(value, "token")
            value = REDACTED
        out.append((key, value))
    return urllib.parse.urlencode(out)


def scrub_signed(text, found):
    """Any signature of a presigned URL in a piece of text: `...&X-Amz-Signature=<hex>` becomes `...&X-Amz-Signature=<redacted>`."""

    def one(match):
        if match.group(2) != REDACTED:
            found.add(match.group(2), "token")
        return match.group(1) + REDACTED

    return SIGNED_QUERY.sub(one, text)


def scrub_body_text(text, mime, found):
    """A body as text: JSON and form bodies are scrubbed by field, and every body for the signature of a presigned URL."""
    mime = (mime or "").lower()
    if "json" in mime:
        try:
            text = json.dumps(scrub_json(json.loads(text), found), separators=(",", ":"))
        except ValueError:
            pass
    elif "x-www-form-urlencoded" in mime:
        text = scrub_form(text, found)
    return scrub_signed(text, found)


def scrub_headers(headers, found):
    out = []
    for h in headers:
        name, value = h["name"], h["value"]
        low = name.lower()
        if low == "cookie":
            value = scrub_cookie_pairs(value, found)
        elif low == "set-cookie":
            value = scrub_set_cookie(value, found)
        elif low in SECRET_HEADERS:
            found.add(value, "token")
            value = REDACTED
        elif low == "date":
            value = FIXED_DATE
        else:
            value = scrub_signed(value, found)  # a Location that redirects to a presigned URL
        out.append({**h, "name": name, "value": value})
    return out


def scrub_cookies(cookies, found):
    out = []
    for c in cookies:
        found.add(c.get("value"), "cookie")
        out.append({**c, "value": REDACTED, **({"expires": None} if "expires" in c else {})})
    return out


def scrub_entry(entry, index, found):
    e = copy.deepcopy(entry)
    e["startedDateTime"] = (EPOCH + timedelta(milliseconds=index)).strftime("%Y-%m-%dT%H:%M:%S.") + f"{index % 1000:03d}Z"
    e["time"] = 0
    e["timings"] = {"blocked": -1, "dns": -1, "connect": -1, "ssl": -1, "send": 0, "wait": 0, "receive": 0}
    for key in list(e):
        if key in DROP_ENTRY_KEYS or key.startswith("_"):
            del e[key]

    req = e["request"]
    req["headers"] = scrub_headers(req.get("headers", []), found)
    req["cookies"] = scrub_cookies(req.get("cookies", []), found)
    post = req.get("postData")
    if post:
        mime = post.get("mimeType", "")
        if "text" in post:
            post["text"] = scrub_body_text(post["text"], mime, found)
        for p in post.get("params", []) or []:
            if p.get("name", "").lower() in SECRET_FIELDS:
                found.add(p.get("value"), "token")
                p["value"] = REDACTED
    for q in req.get("queryString", []) or []:
        if q.get("name", "").lower() in SECRET_FIELDS:
            found.add(q.get("value"), "token")
            q["value"] = REDACTED
    # the query string is also in the URL
    req["url"] = scrub_url(req["url"], found)

    resp = e["response"]
    resp["headers"] = scrub_headers(resp.get("headers", []), found)
    resp["cookies"] = scrub_cookies(resp.get("cookies", []), found)
    content = resp.get("content", {})
    if "text" in content:
        text = content["text"]
        if content.get("encoding") == "base64":
            try:
                decoded = base64.b64decode(text).decode("utf-8")
            except (ValueError, UnicodeDecodeError):
                decoded = None
            if decoded is not None and any(k in (content.get("mimeType") or "").lower() for k in ("json", "text", "form")):
                text, content["encoding"] = decoded, None
                del content["encoding"]
        if content.get("encoding") != "base64":
            content["text"] = scrub_body_text(text, content.get("mimeType"), found)
    resp["redirectURL"] = scrub_url(resp.get("redirectURL", ""), found)
    for key in list(resp):
        if key.startswith("_"):
            del resp[key]
    for key in list(req):
        if key.startswith("_"):
            del req[key]
    return e


def scrub_url(url, found):
    url = scrub_signed(url, found)
    if "?" not in url:
        return url
    base, _, query = url.partition("?")
    pairs = urllib.parse.parse_qsl(query, keep_blank_values=True)
    out = []
    for key, value in pairs:
        if key.lower() in SECRET_FIELDS:
            found.add(value, "token")
            value = REDACTED
        out.append((key, value))
    return base + "?" + urllib.parse.urlencode(out, safe="/:")


def scrub_har(har):
    found = Found()
    out = copy.deepcopy(har)
    log = out["log"]
    log["entries"] = [scrub_entry(e, i, found) for i, e in enumerate(log["entries"])]
    for page in log.get("pages", []) or []:
        page["startedDateTime"] = EPOCH.strftime("%Y-%m-%dT%H:%M:%S.000Z")
        page["pageTimings"] = {}
        for key in list(page):
            if key.startswith("_"):
                del page[key]
    return out, found


def verify_clean(serialised, found):
    """Every secret that was collected must be absent from the output, and no signature of a presigned URL may be in it, collected or
    not (that second check does not depend on the scrub having found the signature). Returns what is left."""
    left = [s for s in found.secrets if s in serialised]
    left += [m.group(2) for m in SIGNED_QUERY.finditer(serialised) if m.group(2) != REDACTED]
    return sorted(set(left))


def main_scrub(src, dest):
    har = json.loads(Path(src).read_text())
    if not har.get("log", {}).get("entries"):
        raise SystemExit(f"{src} has no entries: nothing was recorded, so there is nothing to commit")
    out, found = scrub_har(har)
    if found.cookies == 0 or found.tokens == 0:
        raise SystemExit(
            f"{src}: found {found.cookies} cookie value(s) and {found.tokens} token or password value(s) to scrub. A signed-in journey has "
            "both, so this is not one, or its API calls were not recorded (E2E_HAR_BODIES=1 and a urlFilter on /api/ and /auth/)."
        )
    text = json.dumps(out, indent=1, ensure_ascii=False) + "\n"
    leaked = verify_clean(text, found)
    if leaked:
        raise SystemExit(f"the scrub left {len(leaked)} collected secret(s) in its output (first: {leaked[0][:4]}...); nothing was written")
    Path(dest).parent.mkdir(parents=True, exist_ok=True)
    Path(dest).write_text(text)
    print(
        f"scrub: {src} -> {dest}: {len(out['log']['entries'])} entries, {len(text):,} bytes; scrubbed {len(found.secrets)} distinct secret "
        f"value(s) ({found.cookies} cookie, {found.tokens} token or password occurrences), none left in the output"
    )


# ---------------------------------------------------------------------------------------------------------------------
# The self-test: planted secrets, and a verifier that must be able to fail.
# ---------------------------------------------------------------------------------------------------------------------

SESSION, CSRF, PASSWORD, HEADER_TOKEN = "PLANTED-SESSION-1f9c2", "PLANTED-CSRF-77aa31", "PLANTED-PASSWORD-Zq81", "PLANTED-XCSRF-90bb12"
SIGNATURE = "PLANTEDSIG0123456789abcdef"
PRESIGNED = f"http://localhost:18088/uploads/k?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Expires=3600&X-Amz-Signature={SIGNATURE}"


def planted_har():
    def entry(method, url, req_headers, post, status, resp_headers, content, mime):
        return {
            "startedDateTime": "2026-11-12T05:00:00.123Z",
            "time": 41.5,
            "request": {
                "method": method, "url": url, "httpVersion": "HTTP/1.1", "cookies": [{"name": "session-id", "value": SESSION}],
                "headers": req_headers, "queryString": [], "headersSize": -1, "bodySize": 0, **({"postData": post} if post else {}),
            },
            "response": {
                "status": status, "statusText": "", "httpVersion": "HTTP/1.1", "cookies": [], "headers": resp_headers,
                "content": {"size": len(content), "mimeType": mime, "text": content}, "redirectURL": "", "headersSize": -1, "bodySize": -1,
            },
            "cache": {},
            "timings": {"send": 1, "wait": 30, "receive": 2},
            "serverIPAddress": "127.0.0.1",
            "_securityDetails": {},
        }

    return {
        "log": {
            "version": "1.2",
            "creator": {"name": "Playwright", "version": "1.65.0"},
            "pages": [{"startedDateTime": "2026-11-12T05:00:00.000Z", "id": "page@1", "title": "", "pageTimings": {"onLoad": 12}}],
            "entries": [
                entry(
                    "GET", "http://localhost:18088/auth/get-csrf-token/", [{"name": "Cookie", "value": f"session-id={SESSION}; csrftoken={CSRF}"}],
                    None, 200,
                    [{"name": "Set-Cookie", "value": f"csrftoken={CSRF}; expires=Thu, 12 Nov 2026 05:00:00 GMT; Path=/; SameSite=Lax"},
                     {"name": "Date", "value": "Thu, 12 Nov 2026 05:00:00 GMT"}],
                    json.dumps({"csrf_token": CSRF}), "application/json",
                ),
                entry(
                    "POST", "http://localhost:18088/auth/sign-up/", [{"name": "X-CSRFToken", "value": HEADER_TOKEN}],
                    {"mimeType": "application/x-www-form-urlencoded",
                     "text": f"email=journey%40example.test&password={PASSWORD}&csrfmiddlewaretoken={CSRF}"},
                    302, [{"name": "Location", "value": "http://localhost:18088/onboarding/"}], "", "text/html",
                ),
                entry(
                    "POST", "http://localhost:18088/api/users/me/", [], {"mimeType": "application/json", "text": json.dumps({"password": PASSWORD, "first_name": "Journey"})},
                    200, [], json.dumps({"id": "u-1", "email": "journey@example.test"}), "application/json",
                ),
                entry(
                    "POST", "http://localhost:18088/api/assets/v2/workspaces/ws/", [], {"mimeType": "application/json", "text": "{}"},
                    200, [], json.dumps({"upload_data": {"url": PRESIGNED, "method": "PUT"}}), "application/json",
                ),
            ],
        }
    }


def self_test():
    har = planted_har()
    out, found = scrub_har(har)
    text = json.dumps(out, indent=1)
    problems = []

    # the scrub took them out
    for label, secret in (
        ("session", SESSION), ("csrf", CSRF), ("password", PASSWORD), ("x-csrftoken", HEADER_TOKEN), ("presigned signature", SIGNATURE),
    ):
        if secret in text:
            problems.append(f"the {label} secret is still in the scrubbed output")
        if secret not in found.secrets:
            problems.append(f"the {label} secret was never collected, so the output check could not have seen it")
    if verify_clean(text, found):
        problems.append("verify_clean found a leak in a correct scrub")

    # what must survive
    paths = [e["request"]["url"] for e in out["log"]["entries"]]
    if paths != [e["request"]["url"] for e in har["log"]["entries"]]:
        problems.append("the URLs changed")
    if [e["response"]["status"] for e in out["log"]["entries"]] != [200, 302, 200, 200]:
        problems.append("the statuses changed")
    presigned = json.loads(out["log"]["entries"][3]["response"]["content"]["text"])["upload_data"]["url"]
    if presigned != PRESIGNED.replace(SIGNATURE, REDACTED):
        problems.append(f"a presigned URL was not scrubbed to exactly its signature: {presigned!r}")
    if out["log"]["entries"][1]["response"]["headers"][0]["value"] != "http://localhost:18088/onboarding/":
        problems.append("the Location header changed")
    cookie_header = next(h["value"] for h in out["log"]["entries"][0]["response"]["headers"] if h["name"].lower() == "set-cookie")
    if "xpires" in cookie_header:
        problems.append("a Set-Cookie kept its Expires, which is the time of the recording")
    if cookie_header != f"csrftoken={REDACTED}; Path=/; SameSite=Lax":
        problems.append(f"a Set-Cookie lost an attribute that is no secret: {cookie_header!r}")
    if json.loads(out["log"]["entries"][2]["response"]["content"]["text"]) != {"id": "u-1", "email": "journey@example.test"}:
        problems.append("a body with no secret in it changed")
    if "first_name" not in out["log"]["entries"][2]["request"]["postData"]["text"]:
        problems.append("a request body lost a field that is no secret")
    if any("serverIPAddress" in e or "_securityDetails" in e for e in out["log"]["entries"]):
        problems.append("connection details survived")
    if [e["startedDateTime"] for e in out["log"]["entries"]] != sorted(e["startedDateTime"] for e in out["log"]["entries"]):
        problems.append("the order of the entries is not in their start times")

    # the control: the verifier has to be able to fail. Run on the UNSCRUBBED har it must find every planted secret
    raw_leaks = verify_clean(json.dumps(har), found)
    if len(raw_leaks) < 5:
        problems.append(f"the verifier found {len(raw_leaks)} of 5 planted secrets in the unscrubbed input: it cannot fail")
    # ... and the signature check must not depend on the scrub having collected the signature: an empty `found` still sees it
    if SIGNATURE not in verify_clean(json.dumps(har), Found()):
        problems.append("the verifier cannot see a presigned signature that nothing collected")

    # and the vacuity refusal: a HAR with nothing to scrub is refused
    bare = {"log": {"entries": [{"request": {"method": "GET", "url": "http://x/api/", "headers": [], "cookies": []},
                                 "response": {"status": 200, "headers": [], "cookies": [], "content": {"mimeType": "text/plain"}}}]}}
    _, nothing = scrub_har(bare)
    if nothing.cookies or nothing.tokens:
        problems.append("a HAR with no secret was reported to hold some")

    if problems:
        print("scrub self-test FAILED:\n  " + "\n  ".join(problems), file=sys.stderr)
        return 1
    print(f"scrub self-test ok: {len(found.secrets)} planted secrets scrubbed and verified absent; the verifier found {len(raw_leaks)} in the unscrubbed input")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("har", nargs="?", help="a HAR recorded by a journey with E2E_HAR_BODIES=1")
    ap.add_argument("--out", help="where to write the scrubbed HAR (e2e/har/<journey>.plane.har)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if not args.har or not args.out:
        ap.error("a HAR and --out are required (or --self-test)")
    main_scrub(args.har, args.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Record the compat-slice fixtures from the pinned Plane (oracle/v1-pin-20261018, ba85d4d3d3).

The oracle stack is the one from the Plane fork (oracle/compose.oracle.yml, API on 127.0.0.1:18080); this script does
not start it. It drives the journey's requests, normalises each exchange with the rules shared with the Rust harness
(crates/thrust-compat-plane/tests/parity/normalise.json) and writes one fixture per row:

    crates/thrust-compat-plane/tests/parity/fixtures/<row>.json

    uv run tools/compat-record/record.py --rows S01,S02
    uv run tools/compat-record/record.py --self-test

Raw (un-normalised) exchanges go to --raw-dir and are never committed. Fixtures are written, never hand-edited.
"""

import argparse
import http.cookiejar
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import normalise  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
PARITY = REPO / "crates/thrust-compat-plane/tests/parity"
PIN = "ba85d4d3d3"

# A throwaway instance admin. Synthetic: example.test is reserved and the password exists only in the oracle's volume.
ADMIN = {
    "email": "instance-admin@example.test",
    "password": "Oracle-Admin-Passw0rd!",
    "first_name": "Instance",
    "last_name": "Admin",
    "company_name": "Oracle",
    "is_telemetry_enabled": "False",
}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Hand 3xx back as a response: several rows ARE redirects and the oracle's answer is the 302."""

    def redirect_request(self, *a, **k):
        return None


class Client:
    """One browser-like session: a cookie jar, no redirects followed, CSRF on request."""

    def __init__(self, base):
        self.base = base.rstrip("/")
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar), NoRedirect)

    def request(self, method, path, body=None, form=None, headers=None):
        """Returns the exchange as the raw dict `normalise.exchange` takes (name added by the caller)."""
        hs = dict(headers or {})
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            hs["Content-Type"] = "application/json"
        elif form is not None:
            data = urllib.parse.urlencode(form).encode()
            hs["Content-Type"] = "application/x-www-form-urlencoded"
        req = urllib.request.Request(self.base + path, data=data, method=method, headers=hs)
        try:
            resp = self.opener.open(req, timeout=30)
        except urllib.error.HTTPError as e:  # 3xx (not followed), 4xx, 5xx all land here
            resp = e
        raw = resp.read()
        ctype = resp.headers.get("Content-Type", "")
        parsed = None
        if raw:
            if "json" not in ctype:
                raise SystemExit(f"{method} {path}: {resp.status if hasattr(resp, 'status') else resp.code} body is {ctype!r}, not JSON; extend the fixture format before recording it")
            parsed = json.loads(raw)
        request = {"method": method, "path": path}
        if body is not None:
            request["body"] = body
        elif form is not None:
            request["body"] = {"form": form}
        return {
            "request": request,
            "response": {
                "status": getattr(resp, "status", None) or resp.code,
                "headers": {k: v for k, v in resp.headers.items()},
                "body": parsed,
            },
        }

    def csrf(self):
        """GET /auth/get-csrf-token/ (S03), keeping the csrftoken cookie; returns the token for X-CSRFToken."""
        return self.request("GET", "/auth/get-csrf-token/")["response"]["body"]["csrf_token"]


def setup_instance_admin(base):
    """Prerequisite, not a recorded row: a fresh pinned Plane answers /api/instances/ with is_setup_done=false, and the
    stock app sends the visitor to the instance-admin setup instead of the sign-up page. Doing the setup the way the
    admin app does (a form post, answered by a 302) is what flips the flag."""
    c = Client(base)
    token = c.csrf()
    ex = c.request("POST", "/api/instances/admins/sign-up/", form=ADMIN, headers={"X-CSRFToken": token})
    status = ex["response"]["status"]
    if status != 302:
        raise SystemExit(f"instance admin sign-up answered {status}, expected 302 (is the stack fresh? `down -v` first)")
    # A rejected sign-up is also a 302, with ?error_code= in Location.
    location = ex["response"]["headers"].get("Location", "")
    if "error_code" in location:
        raise SystemExit(f"instance admin sign-up was rejected: {location}")


# ---------------------------------------------------------------------------------------------------------------------
# The rows. Each returns (list of named raw exchanges). Row order is the journey's order; a row may depend on the state
# the earlier rows left (none of the rows below do yet).
# ---------------------------------------------------------------------------------------------------------------------


def row_s01(base):
    """GET /api/instances/ : the boot request. Recorded with no session, after the instance admin exists."""
    c = Client(base)
    return [("boot, no session, instance set up", c.request("GET", "/api/instances/"))]


def row_s02(base):
    """GET /api/users/me/ without a session. The pin answers 401 with a DRF `detail` body: a JSON object with no `id`,
    which is how the React app learns it is signed out (research/0003 section 9, row 3). The authenticated answer is
    recorded with the rows that create a user (S05/S06)."""
    c = Client(base)
    return [("no session", c.request("GET", "/api/users/me/"))]


ROWS = {"S01": row_s01, "S02": row_s02}


def record(base, rows, raw_dir):
    rules = json.loads((PARITY / "normalise.json").read_text())
    out_dir = PARITY / "fixtures"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_dir.mkdir(parents=True, exist_ok=True)
    for row in rows:
        named = ROWS[row](base)
        raw = [{"name": n, **ex} for n, ex in named]
        (raw_dir / f"{row}.raw.json").write_text(json.dumps(raw, indent=2, sort_keys=True) + "\n")
        ids = normalise.Ids(rules["uuid_token"])
        fixture = {
            "format_version": 1,
            "recorded_from": PIN,
            "row": row,
            "exchanges": [normalise.exchange(rules, ids, ex) for ex in raw],
        }
        (out_dir / f"{row}.json").write_text(json.dumps(fixture, indent=2, sort_keys=True) + "\n")
        print(f"{row}: {len(raw)} exchange(s) -> fixtures/{row}.json")


def self_test():
    """Run the shared normaliser cases. A case is {name, input, expected}; `input`/`expected` are lists of exchanges."""
    rules = json.loads((PARITY / "normalise.json").read_text())
    cases = json.loads((PARITY / "normalise-cases.json").read_text())["cases"]
    failed = 0
    for case in cases:
        ids = normalise.Ids(rules["uuid_token"])
        got = [normalise.exchange(rules, ids, ex) for ex in case["input"]]
        if got != case["expected"]:
            failed += 1
            print(f"FAIL {case['name']}\n  got      {json.dumps(got, sort_keys=True)}\n  expected {json.dumps(case['expected'], sort_keys=True)}")
    if failed:
        raise SystemExit(f"{failed} of {len(cases)} normaliser case(s) failed")
    print(f"ok: {len(cases)} normaliser case(s)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base-url", default="http://127.0.0.1:18080")
    ap.add_argument("--rows", default=",".join(ROWS), help="comma-separated row ids")
    ap.add_argument("--raw-dir", type=Path, default=REPO / "target/compat-record/raw")
    ap.add_argument("--skip-setup", action="store_true", help="the instance admin already exists")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    rows = [r.strip() for r in args.rows.split(",") if r.strip()]
    unknown = [r for r in rows if r not in ROWS]
    if unknown:
        raise SystemExit(f"no recorder for row(s) {unknown}; known: {sorted(ROWS)}")
    if not args.skip_setup:
        setup_instance_admin(args.base_url)
    record(args.base_url, rows, args.raw_dir)


if __name__ == "__main__":
    main()

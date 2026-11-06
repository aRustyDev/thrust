# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Prove that nothing named spike-0004* is left in the sandbox account/zone. GET requests ONLY.

Inventory: Workers scripts, Workers custom domains, Workers routes, Access applications, Access policies,
Access service tokens, and DNS records containing "spike-0004" in the sandbox zone.

A "zero" is only evidence if the query could have found something, so this script is run TWICE and is
itself paired with a control:
  1. BEFORE teardown:  --expect present   the verifier MUST see the resources the spike created. If it
                       sees fewer, the verifier is blind (wrong permission / endpoint / pagination) and a
                       later "zero" would mean nothing -> exit 3.
  2. AFTER teardown:   --expect absent    every endpoint must answer success AND list zero matches.
                       An endpoint that errors (403 for a missing token permission) is UNVERIFIABLE, never "zero".

Credentials: CLOUDFLARE_API_TOKEN from the environment only (never argv). Output prints names, never ids or secrets.
Env: SPIKE_ACCOUNT_ID, SPIKE_ZONE_ID.

  uv run verify_teardown.py --expect present [--overlap]
  uv run verify_teardown.py --expect absent
  uv run verify_teardown.py --selftest         # fake local API; proves the verifier tells present from absent from blind

Exit: 0 as expected · 1 expectation not met · 3 verifier blind/unverifiable.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

PREFIX = "spike-0004"
API = "https://api.cloudflare.com/client/v4"

# name, path template, field holding the name, extra query, minimum expected BEFORE teardown
CHECKS = [
    ("workers scripts", "/accounts/{a}/workers/scripts", "id", {}, 3),            # edge, b, space
    ("workers custom domains", "/accounts/{a}/workers/domains", "hostname", {}, 2),  # product host + space host
    ("workers routes", "/zones/{z}/workers/routes", "pattern", {}, 0),
    ("access applications", "/accounts/{a}/access/apps", "name", {}, 3),          # ui, bypass, space-bypass (+ overlap)
    ("access policies", "/accounts/{a}/access/policies", "name", {}, 3),
    ("access service tokens", "/accounts/{a}/access/service_tokens", "name", {}, 1),
    ("dns records", "/zones/{z}/dns_records", "name", {"name.contains": PREFIX}, 2),
    # NOT removed by deleting the Custom Domain (docs: workers/configuration/routing/custom-domains, "Certificates")
    ("edge certificate packs", "/zones/{z}/ssl/certificate_packs", "hosts", {"status": "all"}, 1),
]


def fetch_all(base: str, path: str, token: str, extra: dict) -> tuple[bool, list, str]:
    items: list = []
    page = 1
    while True:
        q = urllib.parse.urlencode({"per_page": 100, "page": page, **extra})
        req = urllib.request.Request(f"{base}{path}?{q}", headers={"Authorization": f"Bearer {token}", "Accept": "application/json"}, method="GET")
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                body = json.load(r)
        except urllib.error.HTTPError as e:
            return False, [], f"HTTP {e.code}"
        except (urllib.error.URLError, OSError, ValueError) as e:
            return False, [], f"{type(e).__name__}"
        if not body.get("success", False):
            return False, [], "success=false"
        res = body.get("result") or []
        items.extend(res if isinstance(res, list) else [])
        info = body.get("result_info") or {}
        if page >= int(info.get("total_pages") or 1):
            return True, items, "ok"
        page += 1


def run(base: str, token: str, account: str, zone: str, expect: str, overlap: bool, allow: list[str] | None = None) -> int:
    """allow: substrings of inventory names whose endpoint may answer 403 (missing read permission). They are printed
    UNVERIFIABLE and excluded from the verdict, so the gap is visible instead of being read as 'zero'."""
    allow = [a.strip().lower() for a in (allow or []) if a.strip()]
    blind, unmet, rows, unverifiable = [], [], [], []
    for name, tpl, field, extra, minimum in CHECKS:
        if name == "access applications" and overlap:
            minimum = 4
        ok, items, why = fetch_all(base, tpl.format(a=account, z=zone), token, extra)
        matches = sorted(str(i.get(field, "")) for i in items if PREFIX in str(i.get(field, "")).lower())
        if name == "edge certificate packs":
            matches = [f"{i.get('id', '?')[:8]}.. hosts={i.get('hosts')}" for i in items if any(PREFIX in h.lower() for h in i.get("hosts", []))]
        rows.append((name, ok, why, matches))
        if not ok and any(a in name for a in allow):
            unverifiable.append(f"{name}: {why}")
        elif not ok:
            blind.append(f"{name}: {why}")
        elif expect == "present" and len(matches) < minimum:
            blind.append(f"{name}: found {len(matches)}, the spike should have created >= {minimum}")
        elif expect == "absent" and matches:
            unmet.append(f"{name}: {matches}")
    for name, ok, why, matches in rows:
        print(f"  {'ok ' if ok else 'ERR'} {name:<24} {len(matches):>2} match(es) {matches if matches else ''} {'' if ok else '(' + why + ')'}")
    if blind:
        print("\nVERIFIER BLIND / UNVERIFIABLE: " + "; ".join(blind))
        print("A zero from this run would NOT be evidence. Fix the token permissions/endpoint first.")
        return 3
    if unmet:
        print("\nLEFTOVERS: " + "; ".join(unmet))
        return 1
    if unverifiable:
        print("\nUNVERIFIABLE (allowed, NOT counted as zero): " + "; ".join(unverifiable))
    print("\n" + ("verifier sees every expected resource it can read (control OK): proceed to teardown" if expect == "present"
                  else "ZERO spike-0004 resources in every readable inventory (and every required inventory answered): teardown verified"
                  + (" -- except the UNVERIFIABLE ones above" if unverifiable else "")))
    return 0


# ---------------------------------------------------------------------------------------------------------
def selftest() -> int:
    state = {"present": True, "forbid": ""}
    data = {
        "/accounts/ACC/workers/scripts": [{"id": "spike-0004-edge"}, {"id": "spike-0004-b"}, {"id": "spike-0004-space"}, {"id": "unrelated"}],
        "/accounts/ACC/workers/domains": [{"hostname": "spike-0004.example.test"}, {"hostname": "spike-0004-space.example.test"}],
        "/zones/ZONE/workers/routes": [],
        "/accounts/ACC/access/apps": [{"name": "spike-0004-ui"}, {"name": "spike-0004-bypass"}, {"name": "spike-0004-space-bypass"},
                                      {"name": "spike-0004-overlap-gated-under-api"}, {"name": "other"}],
        "/accounts/ACC/access/policies": [{"name": "spike-0004-a"}, {"name": "spike-0004-b"}, {"name": "spike-0004-c"}],
        "/accounts/ACC/access/service_tokens": [{"name": "spike-0004-test-token"}],
        "/zones/ZONE/dns_records": [{"name": "spike-0004.example.test"}, {"name": "spike-0004-space.example.test"}],
        "/zones/ZONE/ssl/certificate_packs": [{"id": "abcdef0123456789", "hosts": ["spike-0004.example.test"]}, {"id": "universal00", "hosts": ["example.test", "*.example.test"]}],
    }

    class H(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            p = urllib.parse.urlsplit(self.path).path
            if state["forbid"] and state["forbid"] in p:
                self.send_response(403)
                self.end_headers()
                return
            rows = data.get(p, [])
            if not state["present"]:
                rows = [r for r in rows if not any(PREFIX in str(v) for v in r.values())]
            if "certificate_packs" in p:
                rows = [r for r in rows if state["present"] or "universal" in r["id"]]
            body = json.dumps({"success": True, "result": rows, "result_info": {"total_pages": 1}}).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):
            pass

    srv = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_port}"
    cases = [
        ("present, --expect present", dict(present=True, forbid=""), "present", 0),
        ("present, --expect absent  (leftovers must be reported)", dict(present=True, forbid=""), "absent", 1),
        ("absent,  --expect absent", dict(present=False, forbid=""), "absent", 0),
        ("absent,  --expect present (verifier-blind control must trip)", dict(present=False, forbid=""), "present", 3),
        ("a 403 endpoint under --expect absent must be UNVERIFIABLE, not zero", dict(present=False, forbid="access/apps"), "absent", 3),
    ]
    bad = 0
    # (name, state, expect, want exit, allow-unverifiable list)
    cases = [(n, s, e, w, []) for n, s, e, w in cases] + [
        ("an ALLOWED 403 endpoint is printed UNVERIFIABLE but does not block (absent)", dict(present=False, forbid="dns_records"), "absent", 0, ["dns"]),
        ("a 403 endpoint NOT on the allow list still blocks", dict(present=False, forbid="access/apps"), "absent", 3, ["dns"]),
    ]
    for name, st, expect, want, allow in cases:
        state.update(st)
        print(f"== {name}")
        rc = run(base, "tok", "ACC", "ZONE", expect, overlap=True, allow=allow)
        print(f"   -> exit {rc} (want {want}) {'OK' if rc == want else 'WRONG'}\n")
        bad += rc != want
    srv.shutdown()
    print("selftest:", "PASS" if not bad else f"{bad} case(s) WRONG")
    return 1 if bad else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--expect", choices=["present", "absent"])
    ap.add_argument("--overlap", action="store_true", help="the overlap experiment app was created")
    ap.add_argument("--api-base", default=API)
    ap.add_argument("--allow-unverifiable", default="", help="comma list of inventory names allowed to answer 403, e.g. 'dns,certificate,routes'")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if not a.expect:
        ap.error("--expect present|absent required")
    token = os.environ.get("CLOUDFLARE_API_TOKEN")
    acct, zone = os.environ.get("SPIKE_ACCOUNT_ID"), os.environ.get("SPIKE_ZONE_ID")
    if not (token and acct and zone):
        print("need CLOUDFLARE_API_TOKEN, SPIKE_ACCOUNT_ID, SPIKE_ZONE_ID in the environment", file=sys.stderr)
        return 2
    return run(a.api_base, token, acct, zone, a.expect, a.overlap, a.allow_unverifiable.split(","))


if __name__ == "__main__":
    sys.exit(main())

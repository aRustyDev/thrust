# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Delete the edge certificate pack(s) that a Workers Custom Domain left behind (docs: deleting a Custom Domain does
NOT delete its Advanced Certificate). Part of the spike-0004 teardown.

Safety: a pack is eligible ONLY if EVERY host on it contains "spike-0004". Universal SSL and any other certificate on the
zone are never touched. Default is a dry run; pass --yes to delete. DELETE is the only non-GET call in the whole spike harness.

Credentials: CLOUDFLARE_API_TOKEN (needs Zone: SSL and Certificates Edit) and SPIKE_ZONE_ID from the environment only.

  uv run delete_cert_packs.py            # list what would be deleted
  uv run delete_cert_packs.py --yes      # delete
  uv run delete_cert_packs.py --selftest # fake local API: eligible packs deleted, the universal pack untouched
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import threading
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

PREFIX = "spike-0004"
API = "https://api.cloudflare.com/client/v4"


def call(base: str, token: str, method: str, path: str):
    req = urllib.request.Request(f"{base}{path}", headers={"Authorization": f"Bearer {token}", "Accept": "application/json"}, method=method)
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def run(base: str, token: str, zone: str, yes: bool) -> int:
    try:
        body = call(base, token, "GET", f"/zones/{zone}/ssl/certificate_packs?status=all&per_page=100")
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(f"cannot list certificate packs ({type(e).__name__}): nothing deleted")
        return 3
    packs = body.get("result") or []
    eligible = [p for p in packs if p.get("hosts") and all(PREFIX in h.lower() for h in p["hosts"])]
    skipped = [p for p in packs if p not in eligible]
    for p in skipped:
        print(f"  keep    {str(p.get('id'))[:8]}.. hosts={p.get('hosts')}")
    for p in eligible:
        print(f"  {'DELETE ' if yes else 'would delete'} {str(p.get('id'))[:8]}.. hosts={p.get('hosts')}")
    if not eligible:
        print("no spike-0004 certificate packs found")
        return 0
    if not yes:
        print("dry run: pass --yes to delete")
        return 0
    rc = 0
    for p in eligible:
        try:
            call(base, token, "DELETE", f"/zones/{zone}/ssl/certificate_packs/{p['id']}")
        except (urllib.error.URLError, OSError, ValueError) as e:
            print(f"  FAILED to delete {str(p['id'])[:8]}..: {type(e).__name__}")
            rc = 1
    return rc


def selftest() -> int:
    packs = {"abcdef0123456789": ["spike-0004.example.test"], "universal00": ["example.test", "*.example.test"],
             "mixed0000000000": ["spike-0004.example.test", "www.example.test"]}
    deleted: list[str] = []

    class H(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            body = json.dumps({"success": True, "result": [{"id": k, "hosts": v} for k, v in packs.items()]}).encode()
            self.send_response(200)
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_DELETE(self):  # noqa: N802
            deleted.append(self.path.rsplit("/", 1)[-1])
            self.send_response(200)
            self.send_header("content-length", "2")
            self.end_headers()
            self.wfile.write(b"{}")

        def log_message(self, *a):
            pass

    srv = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_port}"
    run(base, "t", "Z", yes=False)
    dry_ok = deleted == []
    run(base, "t", "Z", yes=True)
    srv.shutdown()
    ok = dry_ok and deleted == ["abcdef0123456789"]
    print(f"\nselftest: dry run deleted nothing={dry_ok}; --yes deleted only the all-spike pack={deleted == ['abcdef0123456789']} "
          f"(universal and the mixed-host pack untouched) -> {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--yes", action="store_true")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--api-base", default=API)
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    token, zone = os.environ.get("CLOUDFLARE_API_TOKEN"), os.environ.get("SPIKE_ZONE_ID")
    if not (token and zone):
        print("need CLOUDFLARE_API_TOKEN and SPIKE_ZONE_ID in the environment", file=sys.stderr)
        return 2
    return run(a.api_base, token, zone, a.yes)


if __name__ == "__main__":
    sys.exit(main())

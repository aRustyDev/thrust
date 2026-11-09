#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Does the pinned oracle KEEP the woven OIDC SSO code? Prove it on the RUNNING stack, not the tree.

The fork-only OIDC code is not part of the v1 contract (it lives under /auth/ and /api/instances/,
neither of which has a schema), so the schema cannot tell us whether it survived the upstream merge.
This probes the real server:

  1. GET /api/instances/  -> JSON containing a boolean `is_oidc_enabled` (False until an admin configures it)
  2. GET /auth/oidc/                -> 302 (the OIDC initiate view ran)
  3. GET /auth/spaces/oidc/         -> 302 (the space-portal initiate view ran)
  4. CONTROL: GET /auth/not-a-real-provider/ must NOT behave like 2/3 (it is not mounted), so a
     302 above cannot be a catch-all redirect that would make checks 2/3 vacuous.
Redirects are NOT followed. Exit 0 only if all four hold.
"""

import http.client
import json
import sys
from urllib.parse import urlsplit

base = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:18080"
u = urlsplit(base)


def get(path):
    c = http.client.HTTPConnection(u.hostname, u.port, timeout=30)
    try:
        c.request("GET", path, headers={"Accept": "application/json"})
        r = c.getresponse()
        return r.status, r.getheader("Location"), r.read()
    finally:
        c.close()


results = {}
st, _, body = get("/api/instances/")
try:
    cfg = json.loads(body).get("config", json.loads(body))
except ValueError:
    cfg = {}
results["instances: is_oidc_enabled is a boolean"] = st == 200 and isinstance(cfg.get("is_oidc_enabled"), bool)
results["instances: is_oidc_enabled is False by default"] = cfg.get("is_oidc_enabled") is False

st_app, loc_app, _ = get("/auth/oidc/")
st_spc, loc_spc, _ = get("/auth/spaces/oidc/")
st_ctl, loc_ctl, _ = get("/auth/not-a-real-provider/")
results["GET /auth/oidc/ -> 302 (view ran)"] = st_app == 302
results["GET /auth/spaces/oidc/ -> 302 (view ran)"] = st_spc == 302
results["CONTROL: unmounted provider is NOT a 302 (no catch-all)"] = st_ctl != 302

for k, ok in results.items():
    print(("PASS " if ok else "FAIL ") + k)
print(f"  /auth/oidc/ -> {st_app} {loc_app}")
print(f"  /auth/spaces/oidc/ -> {st_spc} {loc_spc}")
print(f"  /auth/not-a-real-provider/ -> {st_ctl} {loc_ctl}")
sys.exit(0 if all(results.values()) else 1)

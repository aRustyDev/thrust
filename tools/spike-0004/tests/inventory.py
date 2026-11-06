# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Read-only helpers for the real spike-0004 run on a NON-EMPTY account. GET requests only; stdlib only.

  uv run inventory.py verify-token                 GET /accounts/<id>/tokens/verify (account-owned token)
  uv run inventory.py snapshot --out FILE          names (never ids/secrets) of everything that matters, per endpoint
  uv run inventory.py compare --against FILE [--baseline FILE]
                                                   current inventory vs a snapshot, and vs the named baseline
  uv run inventory.py check-plan PLAN.json         `tofu show -json` of the plan: exactly 10 creates, all spike-0004
  uv run inventory.py wait-ready URL [URL ...]     poll until each public host serves; log propagation delays

Credentials: CLOUDFLARE_API_TOKEN from the environment only. Env: SPIKE_ACCOUNT_ID, SPIKE_ZONE_ID.

An endpoint that answers 403 is UNVERIFIABLE (a missing token permission), never "empty". `compare` exits 0 only if every
VERIFIABLE endpoint is identical to the snapshot/baseline and none lists a spike-0004* name; unverifiable endpoints are
printed so the gap is visible.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.cloudflare.com/client/v4"
PREFIX = "spike-0004"


def _get(path: str, token: str, params: dict | None = None):
    q = urllib.parse.urlencode(params or {})
    req = urllib.request.Request(f"{API}{path}{'?' + q if q else ''}",
                                 headers={"Authorization": f"Bearer {token}", "Accept": "application/json"}, method="GET")
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def _names(acct: str, zone: str):
    """endpoint key -> (path template params, name extractor)"""
    return {
        "workers_scripts": (f"/accounts/{acct}/workers/scripts", {}, lambda r: r.get("id")),
        "workers_domains": (f"/accounts/{acct}/workers/domains", {}, lambda r: r.get("hostname")),
        "access_apps": (f"/accounts/{acct}/access/apps", {}, lambda r: r.get("name")),
        "access_service_tokens": (f"/accounts/{acct}/access/service_tokens", {}, lambda r: r.get("name")),
        "access_policies": (f"/accounts/{acct}/access/policies", {}, lambda r: r.get("name")),
        "zone_workers_routes": (f"/zones/{zone}/workers/routes", {}, lambda r: r.get("pattern")),
        "zone_dns_records_spike": (f"/zones/{zone}/dns_records", {"name.contains": PREFIX}, lambda r: r.get("name")),
        "zone_certificate_packs": (f"/zones/{zone}/ssl/certificate_packs", {"status": "all"},
                                   lambda r: ",".join(sorted(r.get("hosts", [])))),
    }


def take_snapshot(token: str, acct: str, zone: str) -> dict:
    out = {}
    for key, (path, extra, namer) in _names(acct, zone).items():
        names, page, ok, why = [], 1, True, "ok"
        while True:
            try:
                body = _get(path, token, {"per_page": 100, "page": page, **extra})
            except urllib.error.HTTPError as e:
                ok, why = False, f"HTTP {e.code}"
                break
            except (urllib.error.URLError, OSError, ValueError) as e:
                ok, why = False, type(e).__name__
                break
            if not body.get("success", False):
                ok, why = False, "success=false"
                break
            res = body.get("result") or []
            names += [str(namer(r)) for r in (res if isinstance(res, list) else [])]
            if page >= int((body.get("result_info") or {}).get("total_pages") or 1):
                break
            page += 1
        out[key] = {"ok": ok, "why": why, "names": sorted(names) if ok else []}
    return out


def need_env() -> tuple[str, str, str]:
    t, a, z = os.environ.get("CLOUDFLARE_API_TOKEN"), os.environ.get("SPIKE_ACCOUNT_ID"), os.environ.get("SPIKE_ZONE_ID")
    if not (t and a):
        print("need CLOUDFLARE_API_TOKEN and SPIKE_ACCOUNT_ID in the environment", file=sys.stderr)
        sys.exit(2)
    return t, a, z or ""


def cmd_verify_token(_a) -> int:
    t, acct, _ = need_env()
    try:
        b = _get(f"/accounts/{acct}/tokens/verify", t)
    except urllib.error.HTTPError as e:
        print(f"tokens/verify -> HTTP {e.code}")
        return 1
    r = b.get("result") or {}
    print(f"tokens/verify success={b.get('success')} status={r.get('status')} expires_on={r.get('expires_on')} not_before={r.get('not_before')}")
    return 0 if r.get("status") == "active" else 1


def cmd_workers_subdomain(_a) -> int:
    """GET /accounts/<id>/workers/subdomain -> prints the account's workers.dev subdomain (a name, not a secret)."""
    t, acct, _ = need_env()
    try:
        b = _get(f"/accounts/{acct}/workers/subdomain", t)
    except urllib.error.HTTPError as e:
        print(f"workers/subdomain -> HTTP {e.code}")
        return 1
    print((b.get("result") or {}).get("subdomain") or "")
    return 0


def cmd_snapshot(a) -> int:
    t, acct, zone = need_env()
    snap = take_snapshot(t, acct, zone)
    for k, v in snap.items():
        print(f"  {'ok ' if v['ok'] else 'UNV'} {k:<24} {len(v['names']):>2} {v['names'] if v['ok'] else '(' + v['why'] + ')'}")
    json.dump(snap, open(a.out, "w"), indent=1)
    return 0


def cmd_compare(a) -> int:
    t, acct, zone = need_env()
    before = json.load(open(a.against))
    now = take_snapshot(t, acct, zone)
    bad, unv = [], []
    base = json.load(open(a.baseline)) if a.baseline else {}
    for k, cur in now.items():
        if not cur["ok"]:
            unv.append(f"{k} ({cur['why']})")
            print(f"  UNV {k:<24} unverifiable: {cur['why']}")
            continue
        leak = [n for n in cur["names"] if PREFIX in n.lower()]
        prev = before.get(k, {})
        diff_note = ""
        if prev.get("ok") and prev["names"] != cur["names"]:
            added = sorted(set(cur["names"]) - set(prev["names"]))
            removed = sorted(set(prev["names"]) - set(cur["names"]))
            diff_note = f" DIFF vs snapshot: +{added} -{removed}"
            bad.append(f"{k}{diff_note}")
        if k in base and sorted(base[k]) != cur["names"]:
            bad.append(f"{k} != named baseline: have {cur['names']}, baseline {sorted(base[k])}")
        if leak:
            bad.append(f"{k} lists spike-0004 names: {leak}")
        print(f"  ok  {k:<24} {len(cur['names']):>2} {cur['names']}{diff_note}")
    if unv:
        print("\nUNVERIFIABLE endpoints (missing read permission): " + "; ".join(unv))
    if bad:
        print("\nNOT BACK AT BASELINE: " + " | ".join(bad))
        return 1
    print("\nevery verifiable endpoint is identical to the snapshot/baseline and lists no spike-0004 name")
    return 0


def cmd_check_plan(a) -> int:
    plan = json.load(open(a.plan))
    rcs = plan.get("resource_changes", [])
    problems, rows = [], []
    for rc in rcs:
        actions = rc["change"]["actions"]
        after = rc["change"].get("after") or {}
        blob = json.dumps(after, default=str).lower()
        rows.append((rc["address"], actions))
        if actions != ["create"]:
            problems.append(f"{rc['address']}: actions {actions}")
        if PREFIX not in blob:
            problems.append(f"{rc['address']}: no spike-0004 name/hostname in planned values")
    for addr, act in rows:
        print(f"  {act} {addr}")
    if len(rcs) != 10:
        problems.append(f"expected exactly 10 resource changes, plan has {len(rcs)}")
    if problems:
        print("PLAN GATE FAILED: " + "; ".join(problems))
        return 1
    print("plan gate ok: 10 creates, every one carries a spike-0004 name, nothing changed or destroyed")
    return 0


def cmd_wait_ready(a) -> int:
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import checks
    from rawhttp import Target, request

    t0 = time.time()
    state: dict[str, dict] = {}
    targets = []
    for url in a.urls:
        tg = Target.parse(url)
        targets.append(tg)
    deadline = t0 + a.timeout
    ready_streak = 0
    last: dict[str, str] = {}
    while time.time() < deadline:
        all_ok = True
        for tg in targets:
            name = tg.host_header
            probes = [("/api/ping", "bypass"), ("/internal/worker-secret", "gated")] if "space" not in name else [("/", "space-open")]
            for path_, label in probes:
                r = request(tg, path_, timeout=10)
                cls = checks.classify(r)
                desc = f"{label}: status={r.status} class={cls} {(r.error or '')[:70]}"
                gated_ok = cls in checks.REFUSED_CLASSES if a.expect_gate else cls in ("gated-content", "worker-internal")
                ok = (label == "bypass" and cls == "worker-json") or (label == "gated" and gated_ok) \
                    or (label == "space-open" and r.status == 200 and "spike-0004-space" in r.text)
                key = f"{name} {label}"
                if last.get(key) != desc:
                    print(f"  t+{time.time() - t0:5.0f}s  {name:<34} {desc}")
                    last[key] = desc
                    state.setdefault(key, {})["changed_at"] = time.time() - t0
                if ok:
                    state.setdefault(key, {}).setdefault("first_ok", time.time() - t0)
                all_ok &= ok
        ready_streak = ready_streak + 1 if all_ok else 0
        if ready_streak >= 2:
            print(f"\nready after {time.time() - t0:.0f}s; first-ok per probe: " +
                  "; ".join(f"{k}={v.get('first_ok', float('nan')):.0f}s" for k, v in state.items()))
            return 0
        time.sleep(5)
    print(f"\nNOT READY after {a.timeout}s; last states: {last}")
    return 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("verify-token").set_defaults(fn=cmd_verify_token)
    sub.add_parser("workers-subdomain").set_defaults(fn=cmd_workers_subdomain)
    s = sub.add_parser("snapshot")
    s.add_argument("--out", required=True)
    s.set_defaults(fn=cmd_snapshot)
    c = sub.add_parser("compare")
    c.add_argument("--against", required=True)
    c.add_argument("--baseline")
    c.set_defaults(fn=cmd_compare)
    p = sub.add_parser("check-plan")
    p.add_argument("plan")
    p.set_defaults(fn=cmd_check_plan)
    w = sub.add_parser("wait-ready")
    w.add_argument("urls", nargs="+")
    w.add_argument("--timeout", type=int, default=900)
    w.add_argument("--no-gate", dest="expect_gate", action="store_false", help="no Access apps exist: the 'gated' probe is ready when the Worker answers")
    w.set_defaults(fn=cmd_wait_ready)
    a = ap.parse_args()
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())

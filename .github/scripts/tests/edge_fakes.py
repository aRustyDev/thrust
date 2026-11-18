"""Stand-ins for wrangler, the smoke and the edge's controls, for test-deploy-edge.sh (ADR-0111, #195 stage E3).

    edge_fakes.py wrangler <wrangler's args>     `deployments list`, `versions list`, `deploy`, `rollback`
    edge_fakes.py smoke <base> <production version>     scripts/smoke-deploy.sh's interface (SMOKE_EXPECT_EDGE, SMOKE_EXPECT_ROUTE)
    edge_fakes.py controls <base> <edge version>        scripts/edge-controls.sh's interface

State: $FAKE_DIR/state.json holds the edge Worker's versions ([{id, tag}]) and the one that takes the traffic (`active`, or null when
nothing is deployed). Every call is a line of $FAKE_DIR/calls.log, and each records the secrets of SENSITIVE that its process could
see (`<tool> SEES <NAME>`), so the test can show each secret reached only the step that needs it.

The deploy reads the --secrets-file as wrangler would and records what it found: the names, whether each value is the one the test
gave (FAKE_WANT_<NAME>, which are not the secrets' own names, so the check does not count as the fake seeing them), and the file's
mode. A deploy without the file, or with a name missing, fails as `secrets.required` makes wrangler fail.

The smoke and the controls are truthful: they pass only when the version taking the traffic is the one asked for, it is not broken,
the route asked for is production and production is the version asked for.

Switches (environment):
  FAKE_LIVE=<tag>|none|split|untagged  what the edge runs before the deploy (default none): one version with that tag (id ver-1); no
                           deployment; two versions sharing the traffic; one version with no tag
  FAKE_LIST=fail           `deployments list` fails with an error that is not "no such Worker"; =flaky: it fails once, then answers
  FAKE_DEPLOY=fail         `wrangler deploy` fails, and nothing changes
  FAKE_ROLLBACK=fail       `wrangler rollback` fails, and nothing changes
  FAKE_BROKEN=<tag>[,<tag>]  the versions with these tags answer, but wrongly: the smoke of them fails
  FAKE_PROD=<version>      what production answers behind the edge (default: the DEPLOY_SHA the test runs with, FAKE_DEPLOY_SHA)
  FAKE_SMOKE_ANY=1         a broken smoke that passes whatever it is asked
  FAKE_CONTROLS=fail       the edge's controls fail
"""

import json
import os
import stat
import sys

D = os.environ.get("FAKE_DIR", "")
SENSITIVE = ["CLOUDFLARE_API_TOKEN", "EDGE_PIN_KEY", "PREVIEW_ACCESS_CLIENT_ID", "PREVIEW_ACCESS_CLIENT_SECRET", "SMOKE_PASSWORD"]
EDGE_SECRETS = ["EDGE_PIN_KEY", "PREVIEW_ACCESS_CLIENT_ID", "PREVIEW_ACCESS_CLIENT_SECRET"]


def log(tool, line):
    with open(os.path.join(D, "calls.log"), "a") as f:
        f.write(f"{tool} {line}\n")
        for s in SENSITIVE:
            if os.environ.get(s):
                f.write(f"{tool} SEES {s}\n")


def initial():
    live = os.environ.get("FAKE_LIVE", "none")
    if live == "none":
        return {"versions": [], "active": None, "split": False}
    if live == "split":
        return {"versions": [{"id": "ver-1", "tag": "a" * 40}, {"id": "ver-2", "tag": "b" * 40}], "active": "ver-2", "split": True}
    if live == "untagged":
        return {"versions": [{"id": "ver-1", "tag": ""}], "active": "ver-1", "split": False}
    return {"versions": [{"id": "ver-1", "tag": live}], "active": "ver-1", "split": False}


def state():
    path = os.path.join(D, "state.json")
    if not os.path.exists(path):
        return initial()
    with open(path) as f:
        return json.load(f)


def save(st):
    with open(os.path.join(D, "state.json"), "w") as f:
        json.dump(st, f, indent=1)


def opt(args, name):
    return args[args.index(name) + 1] if name in args else None


def active_label(st):
    """What the edge taking the traffic says in x-thrust-edge-version: its tag, else its id."""
    for v in st["versions"]:
        if v["id"] == st["active"]:
            return v["tag"] or v["id"]
    return None


def listing(st, what):
    if what == "deployments" and os.environ.get("FAKE_LIST") in ("fail", "flaky"):
        marker = os.path.join(D, "list-failed")
        if os.environ["FAKE_LIST"] == "fail" or not os.path.exists(marker):
            open(marker, "w").close()
            print("X [ERROR] An unknown error has occurred [code: 10013]", file=sys.stderr)
            return 1
    if st["active"] is None:
        # what wrangler answers for a Worker that does not exist yet
        print("X [ERROR] This Worker does not exist on your account. [code: 10007]", file=sys.stderr)
        return 1
    if what == "deployments":
        if st["split"]:
            shares = [{"version_id": "ver-1", "percentage": 50}, {"version_id": "ver-2", "percentage": 50}]
        else:
            shares = [{"version_id": st["active"], "percentage": 100}]
        print(json.dumps([{"id": "dep-1", "source": "wrangler", "strategy": "percentage", "created_on": "2026-11-17T10:00:00Z",
                           "versions": shares}]))
    else:
        print(json.dumps([{"id": v["id"], "number": n + 1, "annotations": {"workers/tag": v["tag"]} if v["tag"] else {}}
                          for n, v in enumerate(st["versions"])]))
    return 0


def deploy(st, args):
    cfg, tag, secrets_file = opt(args, "-c"), opt(args, "--tag"), opt(args, "--secrets-file")
    if not cfg or not os.path.isfile(cfg):
        print("X [ERROR] no configuration", file=sys.stderr)
        return 1
    found, matches, mode = [], [], "none"
    if secrets_file and os.path.isfile(secrets_file):
        mode = oct(stat.S_IMODE(os.stat(secrets_file).st_mode))
        with open(secrets_file) as f:
            given = json.load(f)
        found = sorted(given)
        matches = [n for n in EDGE_SECRETS if n in given and given[n] == os.environ.get("FAKE_WANT_" + n)]
        with open(os.path.join(D, "secrets-file-path"), "w") as f:
            f.write(secrets_file)
    log("wrangler", f"deploy SECRETS {','.join(found) or 'none'} MATCH {','.join(matches) or 'none'} MODE {mode}")
    missing = [n for n in EDGE_SECRETS if n not in found]
    if missing:
        print(f"X [ERROR] Missing required secrets: {', '.join(missing)}", file=sys.stderr)
        return 1
    if os.environ.get("FAKE_DEPLOY") == "fail":
        print("X [ERROR] A request to the Cloudflare API failed. [code: 10000]", file=sys.stderr)
        return 1
    new = f"ver-{len(st['versions']) + 1}"
    st["versions"].append({"id": new, "tag": tag or ""})
    st["active"], st["split"] = new, False
    save(st)
    print(f"Uploaded {os.path.basename(cfg)}\nCurrent Version ID: {new}")
    return 0


def rollback(st, args):
    target = args[1] if len(args) > 1 else ""
    if os.environ.get("FAKE_ROLLBACK") == "fail":
        print("X [ERROR] rollback failed", file=sys.stderr)
        return 1
    if target not in [v["id"] for v in st["versions"]]:
        print(f"X [ERROR] no version {target}", file=sys.stderr)
        return 1
    st["active"], st["split"] = target, False
    save(st)
    print(f"Rolled back to {target}")
    return 0


def wrangler(args):
    log("wrangler", " ".join(args))
    st = state()
    if args[:2] in (["deployments", "list"], ["versions", "list"]):
        return listing(st, args[0])
    if args[:1] == ["deploy"]:
        return deploy(st, args)
    if args[:1] == ["rollback"]:
        return rollback(st, args)
    print(f"edge_fakes: wrangler {' '.join(args)} is not faked", file=sys.stderr)
    return 3


def serving(st, edge):
    """Does the edge taking the traffic answer as `edge`, and is it not broken?"""
    label = active_label(st)
    broken = [t for t in os.environ.get("FAKE_BROKEN", "").split(",") if t]
    return label is not None and label == edge and label not in broken


def smoke(args):
    base, prod = args[0], args[1]
    edge, route = os.environ.get("SMOKE_EXPECT_EDGE", ""), os.environ.get("SMOKE_EXPECT_ROUTE", "")
    log("smoke", f"{base} {prod} EDGE={edge} ROUTE={route} AUTH={os.environ.get('SMOKE_AUTH', 'on')}")
    # is the file of the edge's secrets still there while the smoke runs? (it should live no longer than the deploy)
    recorded = os.path.join(D, "secrets-file-path")
    if os.path.exists(recorded):
        with open(recorded) as f:
            log("smoke", "SECRETS-FILE " + ("present" if os.path.exists(f.read()) else "gone"))
    if os.environ.get("FAKE_SMOKE_ANY") == "1":
        print("smoke: PASS (a broken smoke)")
        return 0
    st = state()
    want_prod = os.environ.get("FAKE_PROD") or os.environ.get("FAKE_DEPLOY_SHA", "")
    if route == "prod" and prod == want_prod and serving(st, edge):
        print(f"smoke: PASS ({prod} at {base}, routed to prod, by the edge {edge})")
        return 0
    broken = " (broken: it answers wrongly)" if active_label(st) in os.environ.get("FAKE_BROKEN", "").split(",") else ""
    print(f"smoke: FAIL: the edge serving is {active_label(st)}{broken}, asked for {edge}; production is {want_prod}, asked for "
          f"{prod}; the route asked for is '{route}'", file=sys.stderr)
    return 1


def controls(args):
    base, edge = args[0], args[1]
    log("controls", f"{base} {edge}")
    if os.environ.get("FAKE_CONTROLS") == "fail":
        print("edge-controls: FAIL: a forged pin was routed to 'v0.3.0', not to production", file=sys.stderr)
        return 1
    if not serving(state(), edge):
        print(f"edge-controls: FAIL: the answer came from the edge '{active_label(state())}', not '{edge}'", file=sys.stderr)
        return 1
    print("edge-controls: PASS")
    return 0


def main():
    tool, args = sys.argv[1], sys.argv[2:]
    if tool == "wrangler":
        return wrangler(args)
    if tool == "smoke":
        return smoke(args)
    if tool == "controls":
        return controls(args)
    print(f"edge_fakes: unknown tool {tool}", file=sys.stderr)
    return 3


sys.exit(main())

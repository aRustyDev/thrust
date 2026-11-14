"""Stand-ins for wrangler, the smoke and the expand step, for test-deploy-worker.sh (issue #49).

    deploy_fakes.py wrangler <args as wrangler's>
    deploy_fakes.py smoke <base-url> <expected-version>
    deploy_fakes.py migrate

They share one piece of state, $FAKE_DIR/state.json, which models the part of Cloudflare that matters: whether the Worker exists, the
versions uploaded, the deployments made, and which version the edge is actually SERVING. The smoke is truthful about it: it passes only
when the version the edge serves is the one it was asked for, so the deploy's own control (a version nobody deployed must be refused)
fails or passes for the right reason, and a rollback verifies only if it really made the previous version answer.

What each fake records in $FAKE_DIR/calls.log, one line per call, for the test to read: what was called with, and which of the
secrets in SENSITIVE the process could see in its environment (`<tool> SEES <NAME>`), so the test can show each secret reached only the
process that needs it.

Behaviour switches (environment variables of the test, read by the fakes):
  FAKE_LIST_ERROR=1      `deployments list` fails with an error that is not "the Worker does not exist"
  FAKE_DEPLOYMENTS_RAW   print this instead of the deployments (a shape the parser does not know)
  FAKE_DEPLOY=fail       `wrangler deploy` fails and changes nothing
  FAKE_SERVE_STICKY=1    the deploy succeeds but the edge keeps serving the old version (the new one never answers)
  FAKE_BAD_TAGS=a,b      versions with these tags answer, but the smoke's real read fails (the app is broken)
  FAKE_SMOKE_ANY=1       a broken smoke that accepts any version
  FAKE_SMOKE_USAGE=1     the smoke exits 2 (called wrongly)
  FAKE_ROLLBACK=fail     `wrangler rollback` fails
  FAKE_ROLLBACK_NOOP=1   `wrangler rollback` succeeds and the edge keeps serving what it served
  FAKE_MIGRATE=fail      the expand step fails
"""

import json
import os
import sys

D = os.environ.get("FAKE_DIR", "")
SENSITIVE = ["CLOUDFLARE_API_TOKEN", "DATABASE_URL", "TUNNEL_SERVICE_TOKEN_ID", "TUNNEL_SERVICE_TOKEN_SECRET", "SMOKE_PASSWORD"]


def state_path():
    return os.path.join(D, "state.json")


def load():
    try:
        with open(state_path()) as f:
            return json.load(f)
    except FileNotFoundError:
        return None


def save(st):
    with open(state_path(), "w") as f:
        json.dump(st, f)


def log(line):
    with open(os.path.join(D, "calls.log"), "a") as f:
        f.write(line + "\n")


def sees(tool):
    for name in SENSITIVE:
        if os.environ.get(name):
            log(f"{tool} SEES {name}")


def flag(argv, name):
    if name in argv:
        i = argv.index(name)
        if i + 1 < len(argv):
            return argv[i + 1]
    return None


def serving_tag(st):
    for v in st["versions"]:
        if v["id"] == st["serving"]:
            return v.get("tag") or v["id"]
    return ""


def wrangler(argv):
    sees("wrangler")
    st = load()
    cmd = argv[0:2] if argv[:1] in (["deployments"], ["versions"]) else argv[0:1]
    if cmd == ["deployments", "list"] or cmd == ["versions", "list"]:
        log("wrangler " + " ".join(cmd))
        if os.environ.get("FAKE_LIST_ERROR"):
            print("Error: the API is not answering [code: 10000]", file=sys.stderr)
            return 1
        if st is None:
            print("This Worker does not exist on your account. [code: 10007]", file=sys.stderr)
            return 1
        if cmd[0] == "deployments":
            raw = os.environ.get("FAKE_DEPLOYMENTS_RAW")
            if raw is not None:
                print(raw)
                return 0
            print(" ⛅️ wrangler 4.151.2 (fake)")
            deployments = [
                {
                    "id": d["id"],
                    "source": "wrangler",
                    "strategy": "percentage",
                    "created_on": d["created_on"],
                    "versions": [{"version_id": d["version"], "percentage": 100}],
                }
                for d in reversed(st["deployments"])
            ]
            print(json.dumps(deployments, indent=2))
        else:
            versions = [
                {"id": v["id"], "number": i + 1, "annotations": ({"workers/tag": v["tag"]} if v.get("tag") else {})}
                for i, v in enumerate(st["versions"])
            ]
            print(json.dumps(list(reversed(versions)), indent=2))
        return 0
    if cmd == ["deploy"]:
        tag = flag(argv, "--tag")
        config = flag(argv, "-c")
        log(f"wrangler deploy tag={tag}")
        if not config or not os.path.isfile(config):
            print("the configuration file is missing", file=sys.stderr)
            return 1
        with open(config) as f:
            text = f.read()
        if "__" in text and any(m in text for m in ("__WORKER_NAME__", "__HYPERDRIVE_ID__")):
            log("wrangler deploy GOT AN UNRENDERED CONFIG")
            return 1
        if os.environ.get("FAKE_DEPLOY") == "fail":
            print("deploy failed", file=sys.stderr)
            return 1
        if st is None:
            st = {"versions": [], "deployments": [], "serving": ""}
        n = len(st["versions"]) + 1
        vid = f"ver-{n}"
        st["versions"].append({"id": vid, "tag": tag})
        st["deployments"].append({"id": f"dep-{n}", "created_on": f"2026-11-14T{10 + n:02d}:00:00Z", "version": vid})
        if not os.environ.get("FAKE_SERVE_STICKY") or not st["serving"]:
            st["serving"] = vid
        save(st)
        return 0
    if cmd == ["rollback"]:
        target = argv[1] if len(argv) > 1 else ""
        log(f"wrangler rollback id={target}")
        if os.environ.get("FAKE_ROLLBACK") == "fail":
            print("rollback failed", file=sys.stderr)
            return 1
        if st is None or not any(v["id"] == target for v in st["versions"]):
            print("no such version", file=sys.stderr)
            return 1
        n = len(st["deployments"]) + 1
        st["deployments"].append({"id": f"dep-{n}", "created_on": f"2026-11-14T{10 + n:02d}:00:00Z", "version": target})
        if not os.environ.get("FAKE_ROLLBACK_NOOP"):
            st["serving"] = target
        save(st)
        return 0
    print("fake wrangler: unexpected: " + " ".join(argv), file=sys.stderr)
    return 99


def smoke(argv):
    sees("smoke")
    base, expected = (argv + ["", ""])[:2]
    auth = os.environ.get("SMOKE_AUTH", "on")
    force = os.environ.get("SMOKE_FORCE_FAIL", "")
    log(f"smoke expected={expected} auth={auth} force={force or '-'}")
    if os.environ.get("FAKE_SMOKE_USAGE"):
        return 2
    if os.environ.get("FAKE_SMOKE_ANY"):
        return 0
    st = load()
    if st is None:
        return 1
    if expected != serving_tag(st):
        print(f"smoke: FAIL: the version that answers is '{serving_tag(st)}', not '{expected}'", file=sys.stderr)
        return 1
    bad = [t for t in os.environ.get("FAKE_BAD_TAGS", "").split(",") if t]
    if auth != "off" and serving_tag(st) in bad:
        print("smoke: FAIL: the real read failed", file=sys.stderr)
        return 1
    if force:
        print("smoke: FAIL: forced", file=sys.stderr)
        return 1
    return 0


def migrate(_argv):
    sees("migrate")
    log("migrate RUN")
    if os.environ.get("FAKE_MIGRATE") == "fail":
        print("migrate: FAIL", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    tool = sys.argv[1] if len(sys.argv) > 1 else ""
    rest = sys.argv[2:]
    sys.exit({"wrangler": wrangler, "smoke": smoke, "migrate": migrate}[tool](rest))

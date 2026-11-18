"""scripts/flagship.py against a fake Flagship (flagship_fake.py) and synthetic policies (ADR-0111, #195).

    test_flagship.py [path to a copy of flagship.py]

The expected flags are built here, from the policy, by a model of their own (expected_*), not by the script's functions; the fake
evaluates rules as the documentation says, independently of both. Generative where the domain is finite:
  validate  the repository's policy passes; every way of breaking a good one is refused.
  put       every target kind, with several names: the flag is the model's, and Flagship serves `allowed` exactly to the
            principal kinds and audiences the policy lists; every way of breaking a value is refused before any write;
            a write that does not stick is caught by the read-back.
  alias     to production and to a live target; an alias the policy lacks, a target that is not live or not a name, refused.
  delete    a live target, an absent one; a bad name refused.
  check/sync  a good app checks clean. Then EVERY flag of it with EVERY applicable change (description, enabled, default,
            variations, rules, metadata) and each foreign flag: check finds it, sync restores exactly the good app and touches
            only that flag. An invalid value and an alias naming no live target: found, left, and sync exits 1. A missing
            routing is created; the policy's routing value decides its default. Every page size finds a change on the last page.
  evaluate  a service that ORs conditions is caught only with --evaluate; propagation lag fails without --retries and passes
            with them; Cloudflare's envelope or the bare result both read.
  environment  each missing variable, a base that is not https, a wrong token: exit 2, no write. The token never appears in
            any output of the whole run.
A path argument points it at a deliberately broken copy, to show this test fails.
"""

import copy
import json
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
SCRIPT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "scripts", "flagship.py")
FAKE = os.path.join(HERE, "flagship_fake.py")
TOKEN = "fake-token-6f1d0c2e9a8b4d7e"
ACCOUNT, APP = "acct-0123456789", "app-0f1e2d3c"
SUFFIX = "-thrust.example-sub.workers.dev"
KINDS = ["ci", "human"]

POLICY = {
    "$comment": "synthetic, for the test",
    "routing": False,
    "audiences": ["dev", "qa", "rc"],
    "aliases": ["prod", "staging"],
    "targets": {
        "rc": [{"kinds": ["ci", "human"], "audiences": ["qa", "rc"]}],
        "dev": [{"kinds": ["ci"], "audiences": ["dev", "qa"]}, {"kinds": ["human"], "audiences": ["dev"]}],
        "release": [{"kinds": ["human"], "audiences": ["rc"]}],
    },
}

passed = failed = 0
outputs = []
work = tempfile.mkdtemp(prefix="flagship-test-")
STATE, PORTFILE, CALLS = (os.path.join(work, n) for n in ("state.json", "port", "calls.log"))
EDGE = os.path.join(work, "edge.toml")
with open(EDGE, "w") as f:
    f.write('[vars]\nSHARED_ORIGIN = "x"\nACCESS_AUD_DEV = "a"\nACCESS_AUD_QA = "b"\nACCESS_AUD_RC = "c"\n')


def check(label, got, want):
    global passed, failed
    if got == want:
        passed += 1
    else:
        failed += 1
        print(f"BAD  {label}: got {got!r}, wanted {want!r}")


def write_policy(policy, name="policy.json"):
    path = os.path.join(work, name)
    with open(path, "w") as f:
        json.dump(policy, f)
    return path


POLICY_FILE = write_policy(POLICY)
ENV = {
    "PATH": os.environ.get("PATH", ""),
    "FLAGSHIP_API_TOKEN": TOKEN,
    "FLAGSHIP_ACCOUNT_ID": ACCOUNT,
    "FLAGSHIP_APP_ID": APP,
    "PREVIEW_HOST_SUFFIX": SUFFIX,
    "FLAGSHIP_RETRY_SECONDS": "0",
}


def run(*args, policy=None, env=None, drop=()):
    e = {k: v for k, v in {**ENV, **(env or {})}.items() if k not in drop}
    argv = [sys.executable, "-I", SCRIPT, "--policy", policy or POLICY_FILE, "--edge-config", EDGE, *args]
    p = subprocess.run(argv, capture_output=True, text=True, env=e)
    outputs.append(p.stdout + p.stderr)
    return p.returncode, p.stdout + p.stderr


# ---------------------------------------------------------------- the fake

with open(STATE, "w") as f:
    json.dump({"token": TOKEN, "account": ACCOUNT, "app": APP, "flags": {}}, f)
open(CALLS, "w").close()
fake = subprocess.Popen([sys.executable, "-I", FAKE, STATE, PORTFILE, CALLS])
for _ in range(100):
    if os.path.exists(PORTFILE):
        break
    time.sleep(0.05)
with open(PORTFILE) as f:
    ENV["FLAGSHIP_API_BASE"] = f"http://127.0.0.1:{f.read().strip()}/client/v4"


def seed(flags, **modes):
    with open(STATE, "w") as f:
        json.dump({"token": TOKEN, "account": ACCOUNT, "app": APP, "flags": copy.deepcopy(flags), "modes": modes}, f)
    open(CALLS, "w").close()


def app():
    with open(STATE) as f:
        return json.load(f)["flags"]


def writes():
    with open(CALLS) as f:
        return [line.strip() for line in f if line.split(" ")[0] in ("POST", "PUT", "DELETE") and "/evaluate" not in line]


# ---------------------------------------------------------------- the model


def key(prefix, name):
    return prefix + "-" + name.replace(".", "_")


def value(kind, label="v0-3-0", **over):
    v = {"url": f"https://{label}{SUFFIX}", "deployment": "0f9e8d7c-1234", "release": "v0.3.0", "kind": kind,
         "data": "shared", "expires": "2026-12-01T00:00:00Z"}
    v.update(over)
    return v


def stored(flag):
    """What the fake keeps for a flag the script wrote."""
    return {**flag, "type": "json" if isinstance(next(iter(flag["variations"].values())), dict) else
            ("boolean" if isinstance(next(iter(flag["variations"].values())), bool) else "string"),
            "updated_at": "2026-01-01T00:00:00Z", "updated_by": "fake"}


def expected_target(policy, name, v):
    rules = []
    for i, r in enumerate(policy["targets"][v["kind"]]):
        rules.append({"priority": i + 1, "serve_variation": "allowed", "conditions": [
            {"attribute": "kind", "operator": "in", "value": r["kinds"]},
            {"attribute": "audience", "operator": "in", "value": r["audiences"]}]})
    return stored({"key": key("target", name), "enabled": True, "default_variation": "denied", "rules": rules,
                   "variations": {"denied": {}, "allowed": v},
                   "description": "A live target. Written by its deploy and removed by its teardown (scripts/flagship.py); do not edit here."})


def expected_alias(alias, target):
    return stored({"key": key("alias", alias), "enabled": True, "default_variation": "target", "rules": [],
                   "variations": {"target": target},
                   "description": "The target an alias names. Written by CI (scripts/flagship.py alias); do not edit here."})


def expected_routing(on):
    return stored({"key": "routing", "enabled": True, "default_variation": "on" if on else "off", "rules": [],
                   "variations": {"off": False, "on": True},
                   "description": "The kill switch for every target. Made by CI from .github/deploy/flagship/policy.json; do not edit here."})


def reaches(policy, target_kind, kind, audience):
    return any(kind in r["kinds"] and audience in r["audiences"] for r in policy["targets"][target_kind])


TARGETS = {"v0.3.0": value("rc"), "dev-login-page": value("dev", label="dev-login-page"), "v0.2.0": value("release", label="v0-2-0")}
GOOD = {"routing": expected_routing(False), key("alias", "staging"): expected_alias("staging", "v0.3.0"),
        key("alias", "prod"): expected_alias("prod", "prod")}
GOOD.update({key("target", n): expected_target(POLICY, n, v) for n, v in TARGETS.items()})

# ---------------------------------------------------------------- validate

p = subprocess.run([sys.executable, "-I", SCRIPT, "validate"], capture_output=True, text=True)
check("the repository's policy, with the edge's real configuration (the defaults), is valid", p.returncode, 0)
rc, out = run("validate")
check("the synthetic policy is valid", rc, 0)


def broken(change):
    p = copy.deepcopy(POLICY)
    change(p)
    return p


BROKEN = {
    "routing is a string": lambda p: p.update(routing="yes"),
    "routing missing": lambda p: p.pop("routing"),
    "an unknown key": lambda p: p.update(owners=["x"]),
    "$comment not a string": lambda p: p.update({"$comment": 3}),
    "an '@' (a person) in the comment": lambda p: p.update({"$comment": "ask someone@example.org"}),
    "an audience in capitals": lambda p: p["audiences"].append("QA2"),
    "an audience twice": lambda p: p["audiences"].append("dev"),
    "an audience the edge lacks": lambda p: p["audiences"].append("ops"),
    "an audience of the edge missing": lambda p: p["audiences"].remove("qa") or p["targets"]["rc"][0]["audiences"].remove("qa") or p["targets"]["dev"][0]["audiences"].remove("qa"),
    "no audiences": lambda p: p.update(audiences=[]),
    "an alias that is not a name": lambda p: p["aliases"].append("Beta_1"),
    "an alias twice": lambda p: p["aliases"].append("staging"),
    "a target kind that is not a word": lambda p: p["targets"].update({"r-c": [{"kinds": ["ci"], "audiences": ["rc"]}]}),
    "a target kind with no rules": lambda p: p["targets"].update(empty=[]),
    "no target kinds": lambda p: p.update(targets={}),
    "a rule with a principal kind the edge never writes": lambda p: p["targets"]["rc"][0]["kinds"].append("robot"),
    "a rule with an audience the policy lacks": lambda p: p["targets"]["rc"][0]["audiences"].append("ops"),
    "a rule with no kinds": lambda p: p["targets"]["dev"][1].update(kinds=[]),
    "a rule with a kind twice": lambda p: p["targets"]["dev"][1].update(kinds=["human", "human"]),
    "a rule with another field": lambda p: p["targets"]["dev"][1].update(people=["x"]),
    "a rule that is a list": lambda p: p["targets"]["dev"].append(["ci"]),
}
for label, change in BROKEN.items():
    rc, out = run("validate", policy=write_policy(broken(change), "broken.json"))
    check(f"validate refuses {label}", rc, 1)
with open(os.path.join(work, "notjson.json"), "w") as f:
    f.write("{")
check("validate on a file that is not JSON: exit 2", run("validate", policy=os.path.join(work, "notjson.json"))[0], 2)
check("check with an invalid policy: exit 2 before any call",
      (seed({}), run("check", policy=write_policy(broken(BROKEN["routing is a string"]), "broken.json"))[0], writes())[1:], (2, []))

# ---------------------------------------------------------------- put


def write_value(v, name="value.json"):
    path = os.path.join(work, name)
    with open(path, "w") as f:
        json.dump(v, f)
    return path


for kind in POLICY["targets"]:
    for name in ("v0.3.0", "dev-login-page", "pr-1234"):
        seed({"routing": expected_routing(False)})
        v = value(kind, label=name.replace(".", "-"))
        rc, out = run("put", name, write_value(v))
        check(f"put {kind} {name}: exit 0", rc, 0)
        check(f"put {kind} {name}: the flag is the model's", app().get(key("target", name)), expected_target(POLICY, name, v))
        check(f"put {kind} {name}: one POST", writes(), ["POST /client/v4/accounts/%s/flagship/apps/%s/flags" % (ACCOUNT, APP)])
        rc, out = run("check", "--evaluate")
        check(f"put {kind} {name}: Flagship serves the policy to every kind and audience (check --evaluate)", rc, 0)

seed({key("target", "v0.3.0"): expected_target(POLICY, "v0.3.0", value("rc"))})
v2 = value("rc", deployment="aaaa-2222")
rc, out = run("put", "v0.3.0", write_value(v2))
check("put over a live target: exit 0", rc, 0)
check("put over a live target: replaced with PUT", [w.split(" ")[0] for w in writes()], ["PUT"])
check("put over a live target: the new value", app()[key("target", "v0.3.0")], expected_target(POLICY, "v0.3.0", v2))

seed({key("target", "v0.3.0"): expected_target(POLICY, "v0.3.0", value("rc"))}, put_noop=True)
rc, out = run("put", "v0.3.0", write_value(v2))
check("put whose write does not stick: exit 1 (the read-back)", rc, 1)

BAD_VALUES = {
    "a url on another host": value("rc", url="https://evil.example.com"),
    "a url with a path": value("rc", url=f"https://v0-3-0{SUFFIX}/x"),
    "a url over http": value("rc", url=f"http://v0-3-0{SUFFIX}"),
    "a url whose label ends in '-'": value("rc", url=f"https://v0-{SUFFIX}"),
    "a url with a dot in the label": value("rc", url=f"https://v0.3{SUFFIX}"),
    "a deployment in capitals": value("rc", deployment="ABC"),
    "a deployment too long": value("rc", deployment="a" * 65),
    "a kind the policy lacks": value("qa"),
    "data neither shared nor isolated": value("rc", data="both"),
    "a release with a space": value("rc", release="v 1"),
    "expires not a UTC time": value("rc", expires="tomorrow"),
    "expires not a date": value("rc", expires="2026-13-45T00:00:00Z"),
    "an extra field": {**value("rc"), "owner": "x"},
    "a field missing": {k: x for k, x in value("rc").items() if k != "expires"},
    "a field that is not a string": value("rc", release=3),
    "not an object": ["rc"],
}
for label, v in BAD_VALUES.items():
    seed({})
    rc, out = run("put", "v0.3.0", write_value(v))
    check(f"put refuses {label}: exit 1", rc, 1)
    check(f"put refuses {label}: nothing written", writes(), [])
seed({})
check("put of a name that is not one: exit 2", run("put", "V0.3.0", write_value(value("rc")))[0], 2)

# ---------------------------------------------------------------- alias

for target in ("prod", "v0.3.0"):
    seed({key("target", "v0.3.0"): expected_target(POLICY, "v0.3.0", value("rc"))})
    rc, out = run("alias", "staging", target)
    check(f"alias staging -> {target}: exit 0", rc, 0)
    check(f"alias staging -> {target}: the flag is the model's", app().get(key("alias", "staging")), expected_alias("staging", target))
for label, args, want in (("an alias the policy lacks", ("beta", "prod"), 2), ("a target that is not live", ("staging", "v9.9.9"), 1),
                          ("a target that is not a name", ("staging", "Bad Name"), 1)):
    seed({key("target", "v0.3.0"): expected_target(POLICY, "v0.3.0", value("rc"))})
    rc, out = run("alias", *args)
    check(f"alias refuses {label}: exit {want}", rc, want)
    check(f"alias refuses {label}: nothing written", writes(), [])

# ---------------------------------------------------------------- delete

seed(GOOD)
check("delete a live target: exit 0", run("delete", "v0.2.0")[0], 0)
check("delete a live target: gone, the rest untouched", app(), {k: f for k, f in GOOD.items() if k != key("target", "v0.2.0")})
rc, out = run("delete", "v9.9.9")
check("delete an absent target: exit 0, says it was absent", (rc, "target-v9_9_9: already absent" in out), (0, True))
check("delete a name that is not one: exit 2", run("delete", "v9_9")[0], 2)

# ---------------------------------------------------------------- the status of a write

# The live API answers a create 201: the first sync on the live app (flagship.yml run 44373055490, 2026-11-18) made `routing`,
# then took its own 201 for a failure. The fake now answers a create 201, so every create above runs into it; the codes of an
# update and a delete are not documented. Every 2xx is a write done (the read-back is the check), anything else stops the run.
for code in (200, 201, 202, 204):
    seed({k: f for k, f in GOOD.items() if k != "routing"}, write_code=code)
    check(f"a create answered {code}: sync exit 0, routing made", (run("sync")[0], app()), (0, GOOD))
    seed({"routing": expected_routing(False)}, write_code=code)
    rc, out = run("put", "v0.3.0", write_value(value("rc")))
    check(f"a create answered {code}: put exit 0, read back", (rc, "written and read back" in out), (0, True))
    seed({key("target", "v0.3.0"): expected_target(POLICY, "v0.3.0", value("rc"))}, write_code=code)
    rc, out = run("put", "v0.3.0", write_value(v2))
    check(f"an update answered {code}: put exit 0, the new value", (rc, app()[key("target", "v0.3.0")]),
          (0, expected_target(POLICY, "v0.3.0", v2)))
    seed(GOOD, write_code=code)
    rc, out = run("delete", "v0.2.0")
    check(f"a delete answered {code}: exit 0, gone, said so", (rc, key("target", "v0.2.0") in app(), "target-v0_2_0: deleted" in out),
          (0, False, True))
for code in (409, 500):
    seed({k: f for k, f in GOOD.items() if k != "routing"}, write_code=code)
    check(f"a create answered {code}: sync exit 2", run("sync")[0], 2)
    seed({key("target", "v0.3.0"): expected_target(POLICY, "v0.3.0", value("rc"))}, write_code=code)
    check(f"an update answered {code}: put exit 2", run("put", "v0.3.0", write_value(v2))[0], 2)
    seed(GOOD, write_code=code)
    check(f"a delete answered {code}: exit 2", run("delete", "v0.2.0")[0], 2)

# ---------------------------------------------------------------- check and sync

seed(GOOD)
check("the good app checks clean", run("check")[0], 0)
check("the good app checks clean, evaluated", run("check", "--evaluate")[0], 0)
check("sync of the good app writes nothing", (run("sync")[0], writes()), (0, []))


def changes(flag):
    """Every change to a flag that the policy decides, as (label, changed flag)."""
    out = []

    def add(label, f):
        g = copy.deepcopy(flag)
        f(g)
        out.append((label, g))

    add("description changed", lambda g: g.update(description="edited in the dashboard"))
    add("description removed", lambda g: g.update(description=None))
    add("disabled", lambda g: g.update(enabled=False))
    add("metadata added", lambda g: g.update(metadata={"owner": "x"}))
    add("a rule added", lambda g: g["rules"].append({"priority": 99, "conditions": [], "serve_variation": g["default_variation"]}))
    if flag["rules"]:
        add("its rules removed", lambda g: g.update(rules=[]))
        add("an audience added to a rule", lambda g: g["rules"][0]["conditions"][1]["value"].append("dev"))
        add("a condition dropped", lambda g: g["rules"][0]["conditions"].pop())
        add("a rule serving the default", lambda g: g["rules"][0].update(serve_variation="denied"))
    if flag["key"] == "routing":
        add("flipped", lambda g: g.update(default_variation="on"))
        add("variations swapped", lambda g: g.update(variations={"off": True, "on": False}))
    if flag["key"].startswith("target-"):
        add("the denied variation not empty", lambda g: g["variations"].update(denied={"url": "x"}))
        add("the default made allowed", lambda g: g.update(default_variation="allowed"))
        add("a variation added", lambda g: g["variations"].update(other={}))
    if flag["key"].startswith("alias-"):
        add("a second variation", lambda g: g["variations"].update(other="v0.2.0"))
    return out


for k in sorted(GOOD):
    for label, changed in changes(GOOD[k]):
        seed({**GOOD, k: changed})
        rc, out = run("check")
        check(f"{k} {label}: check exit 1", rc, 1)
        check(f"{k} {label}: check names it", f"flagship: {k}:" in out, True)
        rc, out = run("sync")
        check(f"{k} {label}: sync exit 0", rc, 0)
        check(f"{k} {label}: sync restores the good app", app(), GOOD)
        check(f"{k} {label}: sync writes only that flag", [w.rsplit("/", 1)[-1] for w in writes()], [k])

for foreign in ("foo", "alias-beta", "target-Bad", "Routing", "target-", "alias-staging-2"):
    seed({**GOOD, foreign: stored({"key": foreign, "enabled": True, "default_variation": "a", "variations": {"a": True}, "rules": []})})
    rc, out = run("check")
    check(f"foreign flag {foreign}: check exit 1, named", (rc, f"flagship: {foreign}: not a flag the policy makes" in out), (1, True))
    rc, out = run("sync")
    check(f"foreign flag {foreign}: sync deletes it and only it", (rc, app(), [w.split(" ")[0] for w in writes()]), (0, GOOD, ["DELETE"]))

for label, v in BAD_VALUES.items():
    if not isinstance(v, dict):
        continue
    bad = copy.deepcopy(GOOD[key("target", "v0.2.0")])
    bad["variations"]["allowed"] = v
    seed({**GOOD, key("target", "v0.2.0"): bad})
    rc, out = run("check")
    check(f"a live target with {label}: check exit 1, invalid", (rc, "target-v0_2_0: the target is invalid" in out), (1, True))
    rc, out = run("sync")
    check(f"a live target with {label}: sync exit 1 and changes nothing", (rc, writes()), (1, []))

dangling = expected_alias("staging", "v9.9.9")
seed({**GOOD, key("alias", "staging"): dangling})
check("an alias naming no live target: check exit 1", run("check")[0], 1)
check("an alias naming no live target: sync exit 1, nothing written", (run("sync")[0], writes()), (1, []))

seed({k: f for k, f in GOOD.items() if k != "routing"})
check("routing missing: check exit 1", run("check")[0], 1)
check("routing missing: sync creates it", (run("sync")[0], app()), (0, GOOD))
on = write_policy({**POLICY, "routing": True}, "on.json")
seed(GOOD)
check("the policy turns routing on: check exit 1", run("check", policy=on)[0], 1)
check("the policy turns routing on: sync sets its default to on",
      (run("sync", policy=on)[0], app()["routing"]), (0, expected_routing(True)))
check("the policy turns routing on: routing evaluates on", run("check", "--evaluate", policy=on)[0], 0)

last = sorted(GOOD)[-1]
for page in (1, 2, 3, 200):
    seed({**GOOD, last: {**GOOD[last], "description": "x"}}, page=page)
    rc, out = run("check")
    check(f"pages of {page}: the change on the last page is found", (rc, f"flagship: {last}:" in out), (1, True))
    check(f"pages of {page}: every flag listed", f"flagship: {len(GOOD)} flags" in out, True)

# ---------------------------------------------------------------- evaluate

seed(GOOD, eval_or=True)
check("conditions ORed by the service: check without --evaluate cannot see it", run("check")[0], 0)
rc, out = run("check", "--evaluate")
check("conditions ORed by the service: check --evaluate exit 1", rc, 1)
check("conditions ORed by the service: names a served variant the policy does not",
      "serves 'allowed' for" in out and "the policy says 'denied'" in out, True)
# The first evaluations are the aliases' and routing's, whose expected answer IS the default: the lag must reach a target's
# `allowed` contexts to be visible, so it covers the first 8.
seed(GOOD, eval_lag=8)
check("propagation lag, no retries: exit 1", run("check", "--evaluate")[0], 1)
seed(GOOD, eval_lag=8)
check("propagation lag, --retries 2: exit 0", run("check", "--evaluate", "--retries", "2")[0], 0)
seed(GOOD, eval_envelope=True)
check("evaluate answers in the envelope: exit 0", run("check", "--evaluate")[0], 0)
disabled = copy.deepcopy(GOOD)
disabled[key("target", "v0.3.0")]["enabled"] = False
seed(disabled)
rc, out = run("check", "--evaluate")
check("a disabled target: evaluated as denied where the policy allows", "target-v0_3_0: serves 'denied'" in out, True)

# ---------------------------------------------------------------- environment

for var in ("FLAGSHIP_API_TOKEN", "FLAGSHIP_ACCOUNT_ID", "FLAGSHIP_APP_ID", "PREVIEW_HOST_SUFFIX"):
    seed(GOOD)
    rc, out = run("sync", drop=(var,))
    check(f"{var} missing: exit 2, no call", (rc, open(CALLS).read()), (2, ""))
for label, env in (("a base over http elsewhere", {"FLAGSHIP_API_BASE": "http://example.com/client/v4"}),
                   ("an app id with a slash", {"FLAGSHIP_APP_ID": "a/../b"}),
                   ("a suffix without its dash", {"PREVIEW_HOST_SUFFIX": "thrust.workers.dev"})):
    seed(GOOD)
    check(f"{label}: exit 2, no call", (run("sync", env=env)[0], open(CALLS).read()), (2, ""))
seed({**GOOD, "routing": {**GOOD["routing"], "description": "x"}})
rc, out = run("sync", env={"FLAGSHIP_API_TOKEN": "not-the-token"})
check("a wrong token: exit 2, nothing written", (rc, app()["routing"]["description"]), (2, "x"))

check("the token appears in no output of the run", any(TOKEN in o for o in outputs), False)

fake.terminate()
fake.wait()
print(f"test_flagship: {passed} passed, {failed} failed")
sys.exit(1 if failed else 0)

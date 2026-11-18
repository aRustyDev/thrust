"""scripts/preview-result.py: reading `wrangler preview --json` for a target, and refusing a Preview that is not the one asked for.

    test_preview_result.py [path to a copy of preview-result.py]

Generated (seeded): 300 `previews` blocks, each a random choice of the bindings preview-config.py writes, and the answer Cloudflare
would give for each (in the shape the spike recorded, research/0016), behind 0 to 3 progress lines. Each must be accepted, with
the URL and the deployment id printed. Then, for EVERY declared binding of each, the answer without it; for every room, its
script_name gone or another; for every variable, other text; and the URL and the deployment id broken every way the edge would
refuse: each must be refused, naming what is wrong. An answer with no JSON object (wrangler failing before it, or an array) is exit 2.
A path argument points it at a deliberately broken copy, to show this test fails.

The thousands of cases call the script's `main()` in this process, with its output captured, and get what a process would give: the exit status, stdout and
stderr. A process for each of them was most of this test's time (about 60 s of 80). So that the in-process call stays an honest stand-in for the command
line, a sample of cases of every kind (accepted, each way of refused, no JSON, not an object, wrongly called) is also run through a real process, as
`python3 -I <script> ...`, and the two must agree on all three.
"""

import contextlib
import copy
import importlib.util
import io
import json
import os
import random
import subprocess
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "..", "..", "scripts", "preview-result.py")
SUFFIX = "-thrust-production.team-4d9.workers.dev"
work = tempfile.mkdtemp(prefix="preview-result-test-")
passed = failed = 0
rng = random.Random(195)


def check(label, got, want):
    global passed, failed
    if got == want:
        passed += 1
    else:
        failed += 1
        print(f"BAD  {label}: got {got!r}, wanted {want!r}")


def load(path):
    """The script as a module. Its name is not `__main__`, so its entry point (`sys.exit(main(sys.argv))`) does not run."""
    spec = importlib.util.spec_from_file_location("preview_result_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PREVIEW_RESULT = load(SCRIPT)


def call(argv):
    """What a process running the script with these arguments would give, without starting one: (exit status, stdout, stderr)."""
    out, err = io.StringIO(), io.StringIO()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = PREVIEW_RESULT.main(argv)
    except SystemExit as e:
        rc = e.code
    except Exception:
        rc = 1  # an uncaught exception ends a process with status 1 and the traceback on stderr
        err.write(traceback.format_exc())
    return (0 if rc is None else rc if isinstance(rc, int) else 1), out.getvalue(), err.getvalue()


def write_inputs(answer_text, previews):
    out_path, cfg_path = os.path.join(work, "out"), os.path.join(work, "cfg.json")
    with open(out_path, "w") as f:
        f.write(answer_text)
    with open(cfg_path, "w") as f:
        json.dump({"name": "thrust-production", "previews": previews}, f)
    return out_path, cfg_path


def run(answer_text, previews, label):
    out_path, cfg_path = write_inputs(answer_text, previews)
    return call([SCRIPT, out_path, cfg_path, label, SUFFIX])


def run_process(answer_text, previews, label):
    """The same, through a real process: the command line, and the `-I` the workflow runs it with."""
    out_path, cfg_path = write_inputs(answer_text, previews)
    p = subprocess.run([sys.executable, "-I", SCRIPT, out_path, cfg_path, label, SUFFIX], capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


def previews_block():
    p = {"version_metadata": {"binding": "CF_VERSION_METADATA"}}
    if rng.random() < 0.8:
        p["vars"] = {f"V{i}": rng.choice(["on", "https://thrust.sow.foo", "a b", ""]) for i in range(rng.randint(1, 4))}
    if rng.random() < 0.7:
        p["hyperdrive"] = [{"binding": "HYPERDRIVE", "id": "ab" * 16}]
    if rng.random() < 0.7:
        p["r2_buckets"] = [{"binding": "BLOBS", "bucket_name": "b"}]
    if rng.random() < 0.7:
        p["ratelimits"] = [{"name": f"RL{i}", "namespace_id": "93002", "simple": {"limit": 10, "period": 60}} for i in range(rng.randint(1, 2))]
    if rng.random() < 0.5:
        p["send_email"] = [{"name": "EMAIL"}]
    if rng.random() < 0.7:
        p["queues"] = {"producers": [{"binding": "EVENTS", "queue": "q"}]}
    if rng.random() < 0.8:
        p["durable_objects"] = {"bindings": [{"name": f"ROOM{i}", "class_name": "RoomDo",
                                              "script_name": rng.choice(["thrust-production", "another"])} for i in range(rng.randint(1, 2))]}
    return p


def applied(p):
    """Cloudflare's report of the bindings, in the spike's shape, with an extra binding it adds (ASSETS)."""
    env = {"ASSETS": {"type": "assets"}}
    for k, v in (p.get("vars") or {}).items():
        env[k] = {"type": "plain_text", "text": v}
    for key, field in (("hyperdrive", "binding"), ("r2_buckets", "binding"), ("ratelimits", "name"), ("send_email", "name")):
        for b in p.get(key) or []:
            env[b[field]] = {"type": key}
    env[p["version_metadata"]["binding"]] = {"type": "version_metadata"}
    for b in (p.get("queues") or {}).get("producers") or []:
        env[b["binding"]] = {"type": "queue"}
    for b in (p.get("durable_objects") or {}).get("bindings") or []:
        env[b["name"]] = {"class_name": b["class_name"], "namespace_id": "ns", "script_name": b["script_name"], "type": "durable_object_namespace"}
    return env


def answer(label, dep, env, urls=None):
    return {"preview": {"slug": label, "urls": urls if urls is not None else
                        [f"https://{label}.custom.example", f"https://{label}{SUFFIX}"]},
            "deployment": {"id": dep, "urls": [f"https://{str(dep)[:8]}{SUFFIX}"], "env": env}}


def text(obj, progress=None):
    lines = progress if progress is not None else ["🌀 Building list of assets...\n", "✨ Read 2 files\n", "🌀 Starting asset upload...\n"][:rng.randint(0, 3)]
    return "".join(lines) + json.dumps(obj, indent=rng.choice([None, 2]))


def names(p):
    out = list((p.get("vars") or {}))
    for key, field in (("hyperdrive", "binding"), ("r2_buckets", "binding"), ("ratelimits", "name"), ("send_email", "name")):
        out += [b[field] for b in p.get(key) or []]
    out.append(p["version_metadata"]["binding"])
    out += [b["binding"] for b in (p.get("queues") or {}).get("producers") or []]
    out += [b["name"] for b in (p.get("durable_objects") or {}).get("bindings") or []]
    return out


counts = {"bindings dropped": 0, "rooms broken": 0, "vars broken": 0}
for i in range(300):
    p = previews_block()
    label = rng.choice(["v0-3-0", "staging-1", "pr-42", "a"])
    dep = rng.choice(["51f4a871-4944-4278-9c2a-785e1bea4125", "abc", "0" * 64])
    env = applied(p)
    rc, out, err = run(text(answer(label, dep, env)), p, label)
    check(f"{i}: the answer for what was asked is accepted", (rc, out), (0, f"url=https://{label}{SUFFIX}\ndeployment={dep}\n"))
    for name in names(p):
        counts["bindings dropped"] += 1
        broken = copy.deepcopy(env)
        del broken[name]
        rc, out, err = run(text(answer(label, dep, broken)), p, label)
        check(f"{i}: {name} missing is refused, named, nothing printed", (rc, f"the binding {name} is not in" in err, out), (1, True, ""))
    for b in (p.get("durable_objects") or {}).get("bindings") or []:
        for wrong in ("absent", "thrust-other"):
            counts["rooms broken"] += 1
            broken = copy.deepcopy(env)
            if wrong == "absent":
                del broken[b["name"]]["script_name"]
            else:
                broken[b["name"]]["script_name"] = wrong
            rc, out, err = run(text(answer(label, dep, broken)), p, label)
            check(f"{i}: room {b['name']} with script_name {wrong} is refused", (rc, f"the binding {b['name']}: script_name is" in err), (1, True))
    for name, value in (p.get("vars") or {}).items():
        counts["vars broken"] += 1
        broken = copy.deepcopy(env)
        broken[name]["text"] = value + "x"
        rc, out, err = run(text(answer(label, dep, broken)), p, label)
        check(f"{i}: var {name} with other text is refused", (rc, f"the binding {name}: text is" in err), (1, True))
for what, n in counts.items():
    check(f"the generator reached cases where {what}", n > 100, True)

# the URL and the deployment id, every way the edge would refuse them
p = previews_block()
env = applied(p)
for label_urls, why in (
    ([], "no URL at all"),
    ([f"https://v0-3-0.custom.example"], "only the custom-domain URL"),
    ([f"https://v0-3-0-another-worker.team-4d9.workers.dev"], "another Worker's URL"),
    ([f"http://v0-3-0{SUFFIX}"], "http"),
    ([f"https://v0-3-0{SUFFIX}/"], "with a path"),
    ([f"https://v0-3-1{SUFFIX}"], "another Preview's URL"),
):
    rc, out, err = run(text(answer("v0-3-0", "dep-1", env, label_urls)), p, "v0-3-0")
    check(f"URL: {why} is refused", (rc, "the Preview has no URL" in err), (1, True))
for dep, why in (("DEP-1", "capitals"), ("a" * 65, "too long"), ("dep_1", "an underscore"), ("", "empty"), (None, "missing"), (7, "a number")):
    rc, out, err = run(text(answer("v0-3-0", dep, env) if dep is not None else {"preview": answer("v0-3-0", "x", env)["preview"],
                                                                                    "deployment": {"env": env}}), p, "v0-3-0")
    check(f"deployment id: {why} is refused", (rc, "is not one the edge accepts" in err), (1, True))
rc, out, err = run("🌀 Building list of assets...\nX [ERROR] the upload failed\n", p, "v0-3-0")
check("an answer with no JSON line: exit 2", rc, 2)
rc, out, err = run(text([1, 2]), p, "v0-3-0")
check("an answer that is not an object (no line starts with '{'): exit 2", rc, 2)


# The in-process call stands in for the command line in every case above, so a sample of every kind of case is run both ways, and the two must agree on
# the exit status, stdout and stderr: the entry point, the arguments and the `-I` the workflow runs it with are the process's, and are covered here.
def agree(why, answer_text, previews, label):
    check(f"in-process and process agree: {why}", run(answer_text, previews, label), run_process(answer_text, previews, label))


kinds = set()
for _ in range(6):
    p = previews_block()
    label = rng.choice(["v0-3-0", "staging-1", "pr-42", "a"])
    env = applied(p)
    dep = "51f4a871-4944-4278-9c2a-785e1bea4125"
    agree("an answer for what was asked", text(answer(label, dep, env)), p, label)
    kinds.add("accepted")
    broken = copy.deepcopy(env)
    del broken[names(p)[0]]
    agree("a binding missing", text(answer(label, dep, broken)), p, label)
    kinds.add("binding")
    for b in (p.get("durable_objects") or {}).get("bindings") or []:
        broken = copy.deepcopy(env)
        broken[b["name"]]["script_name"] = "thrust-other"
        agree("a room on another script", text(answer(label, dep, broken)), p, label)
        kinds.add("room")
        break
    for name, value in (p.get("vars") or {}).items():
        broken = copy.deepcopy(env)
        broken[name]["text"] = value + "x"
        agree("a variable with other text", text(answer(label, dep, broken)), p, label)
        kinds.add("var")
        break
check("the sample reached every kind of case", sorted(kinds), ["accepted", "binding", "room", "var", ])
agree("no URL", text(answer("v0-3-0", "dep-1", env, [])), p, "v0-3-0")
agree("a deployment id the edge refuses", text(answer("v0-3-0", "DEP-1", env)), p, "v0-3-0")
agree("no JSON line (exit 2)", "🌀 Building list of assets...\nX [ERROR] the upload failed\n", p, "v0-3-0")
agree("an answer that is not an object (exit 2)", text([1, 2]), p, "v0-3-0")
bare = subprocess.run([sys.executable, "-I", SCRIPT], capture_output=True, text=True)
check("in-process and process agree: called with no arguments (exit 2)", call([SCRIPT]), (bare.returncode, bare.stdout, bare.stderr))

print(f"test_preview_result: {passed} passed, {failed} failed")
sys.exit(1 if failed else 0)

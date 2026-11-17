"""Stand-ins for wrangler's Previews, the opt-in and the smoke, for test-deploy-target.sh (ADR-0111, #195).

    target_fakes.py wrangler <wrangler's args>     `preview` and `preview delete`
    target_fakes.py <put|delete|...> <args>        scripts/flagship.py itself, after recording which secrets it could see
    target_fakes.py optin <base> <audience> <target> <jar>
    target_fakes.py smoke <base> <expected-version>

State: $FAKE_DIR/state.json holds the Previews ({label: {deployment, env, urls}}); the Flagship app is flagship_fake.py's, read from
$FAKE_FLAGSHIP_STATE. Every call is a line of $FAKE_DIR/calls.log, and each records the secrets of SENSITIVE that its process could
see (`<tool> SEES <NAME>`), so the test can show each secret reached only the step that needs it.

The wrangler fake applies the target configuration's `previews` block as Cloudflare reports it (the shape measured in the spike,
research/0016): a variable as {type: plain_text, text}, a room as {class_name, namespace_id, script_name if one was given, type},
the rest as {type}. Its answer is progress lines, then the JSON. The opt-in is truthful: it pins only when the target's flag
exists. The smoke is truthful: it passes only when the pin names a target whose Preview AND flag carry <expected-version>, and
SMOKE_EXPECT_ROUTE is that target.

Switches (environment):
  FAKE_PREVIEW=fail        the Preview is created, then wrangler fails (so the cleanup must remove a Preview it never saw described)
  FAKE_PREVIEW_URL=other   the Preview's URLs are another Worker's
  FAKE_ENV_DROP=<NAME>     that binding is missing from what Cloudflare applied
  FAKE_ROOM_OWN=1          rooms get the Preview's own namespace (no script_name), as when script_name is not honoured
  FAKE_VAR=<NAME>=<text>   that variable was applied with other text
  FAKE_DEPLOYMENT=<id>     the deployment id wrangler reports
  FAKE_DELETE=fail         `preview delete` fails
  FAKE_OPTIN=fail          the opt-in fails
  FAKE_SMOKE=fail          the smoke fails (its control, which asks for a version nobody deployed, is truthful anyway)
  FAKE_SMOKE_ANY=1         a broken smoke that passes whatever it is asked
"""

import json
import os
import sys

D = os.environ.get("FAKE_DIR", "")
SENSITIVE = ["CLOUDFLARE_API_TOKEN", "FLAGSHIP_API_TOKEN", "EDGE_CI_CLIENT_ID", "EDGE_CI_CLIENT_SECRET", "SMOKE_PASSWORD"]
SUB = os.environ.get("FAKE_SUBDOMAIN", "team-4d9")
REAL_FLAGSHIP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "scripts", "flagship.py")


def log(tool, line):
    with open(os.path.join(D, "calls.log"), "a") as f:
        f.write(f"{tool} {line}\n")
        for s in SENSITIVE:
            if os.environ.get(s):
                f.write(f"{tool} SEES {s}\n")


def state():
    path = os.path.join(D, "state.json")
    if not os.path.exists(path):
        return {"previews": {}, "n": 0}
    with open(path) as f:
        return json.load(f)


def save(st):
    with open(os.path.join(D, "state.json"), "w") as f:
        json.dump(st, f, indent=1)


def opt(args, name):
    return args[args.index(name) + 1] if name in args else None


def applied(previews):
    env = {}
    for k, v in (previews.get("vars") or {}).items():
        env[k] = {"type": "plain_text", "text": v}
    for key, field, kind in (("hyperdrive", "binding", "hyperdrive"), ("r2_buckets", "binding", "r2_bucket"),
                             ("ratelimits", "name", "ratelimit"), ("send_email", "name", "send_email")):
        for b in previews.get(key) or []:
            env[b[field]] = {"type": kind}
    if previews.get("version_metadata"):
        env[previews["version_metadata"]["binding"]] = {"type": "version_metadata"}
    for b in (previews.get("queues") or {}).get("producers") or []:
        env[b["binding"]] = {"type": "queue"}
    for b in (previews.get("durable_objects") or {}).get("bindings") or []:
        room = {"class_name": b["class_name"], "namespace_id": "ns-0123", "type": "durable_object_namespace"}
        if "script_name" in b and os.environ.get("FAKE_ROOM_OWN") != "1":
            room["script_name"] = b["script_name"]
        env[b["name"]] = room
    env["ASSETS"] = {"type": "assets"}
    drop = os.environ.get("FAKE_ENV_DROP")
    if drop:
        env.pop(drop, None)
    if os.environ.get("FAKE_VAR"):
        name, text = os.environ["FAKE_VAR"].split("=", 1)
        env[name] = {"type": "plain_text", "text": text}
    return env


def wrangler(args):
    log("wrangler", " ".join(args))
    st = state()
    if args[:2] == ["preview", "delete"]:
        label = opt(args, "--name")
        if os.environ.get("FAKE_DELETE") == "fail" or label not in st["previews"]:
            print(f"X [ERROR] could not delete the Preview {label}", file=sys.stderr)
            return 1
        del st["previews"][label]
        save(st)
        print(f'Preview "{label}" deleted successfully.')
        return 0
    if args[:1] != ["preview"] or "--json" not in args or "--ignore-base-config" not in args:
        print(f"the fake wrangler does not know: {args}", file=sys.stderr)
        return 2
    label, tag, config = opt(args, "--name"), opt(args, "--tag"), opt(args, "--config")
    with open(config) as f:
        cfg = json.load(f)
    st["n"] += 1
    dep = os.environ.get("FAKE_DEPLOYMENT") or f"{st['n']:08x}-0000-4000-8000-{(tag or '0' * 12)[:12]}"
    worker = "another-worker" if os.environ.get("FAKE_PREVIEW_URL") == "other" else cfg["name"]
    urls = [f"https://{label}-{worker}.{SUB}.workers.dev"]
    st["previews"][label] = {"deployment": dep, "env": applied(cfg["previews"]), "urls": urls, "tag": tag}
    save(st)
    if os.environ.get("FAKE_PREVIEW") == "fail":
        print("X [ERROR] the upload failed half way", file=sys.stderr)
        return 1
    print("🌀 Building list of assets...\n✨ Read 2 files from the assets directory\n🌀 Starting asset upload...")
    print(json.dumps({"preview": {"id": "p-1", "slug": label, "name": label, "urls": urls},
                      "deployment": {"id": dep, "number": st["n"], "urls": [f"https://{dep[:8]}-{worker}.{SUB}.workers.dev"],
                                     "env": st["previews"][label]["env"]}}, indent=2))
    return 0


def flag(name):
    with open(os.environ["FAKE_FLAGSHIP_STATE"]) as f:
        flags = json.load(f)["flags"]
    return flags.get("target-" + name.replace(".", "_"))


def optin(args):
    log("optin", " ".join(args))
    base, audience, target, jar = args
    if os.environ.get("FAKE_OPTIN") == "fail" or flag(target) is None:
        print(f"edge-opt-in: FAIL: no pin for {target}", file=sys.stderr)
        return 1
    with open(jar, "w") as f:
        f.write(f"#HttpOnly_127.0.0.1\tFALSE\t/\tTRUE\t0\t__Host-thrust_edge\tpin-{target}\n")
    return 0


def smoke(args):
    log("smoke", " ".join(args) + f" route={os.environ.get('SMOKE_EXPECT_ROUTE', '')} auth={os.environ.get('SMOKE_AUTH', 'on')}")
    base, expected = args
    if os.environ.get("FAKE_SMOKE_ANY") == "1":
        return 0
    if os.environ.get("FAKE_SMOKE") == "fail" and not expected.startswith("not-deployed-"):
        print("smoke: FAIL: the fake smoke fails", file=sys.stderr)
        return 1
    jar = os.environ.get("SMOKE_COOKIE_FILE", "")
    pinned = open(jar).read().rsplit("pin-", 1)[-1].strip() if jar and os.path.exists(jar) else ""
    preview = state()["previews"].get(pinned.replace(".", "-"))
    f = flag(pinned) if pinned else None
    ok = (preview is not None and preview["deployment"] == expected and f is not None
          and f["variations"]["allowed"]["deployment"] == expected and os.environ.get("SMOKE_EXPECT_ROUTE") == pinned)
    if not ok:
        print(f"smoke: FAIL: {expected} does not answer for the pin {pinned!r}", file=sys.stderr)
        return 1
    print(f"smoke: PASS ({expected})")
    return 0


def main():
    role, args = sys.argv[1], sys.argv[2:]
    if role == "wrangler":
        return wrangler(args)
    if role == "optin":
        return optin(args)
    if role == "smoke":
        return smoke(args)
    log("flagship", " ".join(sys.argv[1:3]))
    os.execv(sys.executable, [sys.executable, "-I", os.path.abspath(REAL_FLAGSHIP)] + sys.argv[1:])


if __name__ == "__main__":
    sys.exit(main())
